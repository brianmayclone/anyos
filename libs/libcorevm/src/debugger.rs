//! Built-in interactive debugger for the interpreter path.
//!
//! Activated by the `host_test` feature flag. Provides breakpoints,
//! watchpoints, single-stepping, memory/register dumps, disassembly,
//! and page-table walks — all controllable from stdin when a break
//! triggers.
//!
//! # Usage from test_vmd
//!
//! ```text
//! scripts/test_vmd --iso ... --plain --debugger
//! ```
//!
//! Then type commands at the prompt when a breakpoint hits, or press
//! Ctrl-C to break into the debugger at any time.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::cpu::{Cpu, Mode};
use crate::memory::{GuestMemory, MemoryBus, Mmu, AccessType};
use crate::registers::SegReg;

// ── Global debugger state ────────────────────────────────────────────

/// Whether the debugger is enabled at all.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// If set, break before the next instruction (single-step or Ctrl-C).
static BREAK_NEXT: AtomicBool = AtomicBool::new(false);
/// If set, break on next exception delivery.
static BREAK_ON_EXCEPTION: AtomicBool = AtomicBool::new(false);
/// If non-zero, break when instruction_count reaches this value.
static BREAK_AT_IC: AtomicU64 = AtomicU64::new(0);

/// Maximum number of breakpoints / watchpoints.
const MAX_BP: usize = 16;
const MAX_WP: usize = 16;
const MAX_EXCEPT_BP: usize = 8;

/// A code breakpoint (by linear address).
struct Breakpoint {
    active: bool,
    /// Linear address (CS.base + RIP) to break on.
    address: u64,
    /// If true, this is a one-shot breakpoint (auto-removed after hit).
    once: bool,
    /// Optional: only trigger when mode matches.
    mode_filter: Option<Mode>,
}

/// A memory watchpoint (by physical address range).
struct Watchpoint {
    active: bool,
    /// Physical address start.
    addr: u64,
    /// Length in bytes.
    len: u64,
    /// Watch writes, reads, or both.
    kind: WatchKind,
}

#[derive(Clone, Copy, PartialEq)]
enum WatchKind {
    Write,
    Read,
    ReadWrite,
}

/// Break on specific exception vector.
struct ExceptionBreakpoint {
    active: bool,
    vector: u8,
    /// Optional: only when CR2 (for #PF) is in this range.
    addr_lo: Option<u64>,
    addr_hi: Option<u64>,
}

/// All mutable debugger state, protected behind a simple global.
/// Only used from the single-threaded interpreter loop.
struct DebuggerState {
    breakpoints: [Breakpoint; MAX_BP],
    watchpoints: [Watchpoint; MAX_WP],
    exception_bps: [ExceptionBreakpoint; MAX_EXCEPT_BP],
    /// Stepping mode: 0 = run, N = step N instructions then break.
    steps_remaining: u64,
    /// Last command for repeat with Enter.
    last_cmd: String,
    /// Suppress output during "continue" (avoid flooding).
    silent: bool,
}

static mut STATE: Option<DebuggerState> = None;

fn state() -> &'static mut DebuggerState {
    unsafe {
        STATE.get_or_insert_with(|| DebuggerState {
            breakpoints: core::array::from_fn(|_| Breakpoint {
                active: false,
                address: 0,
                once: false,
                mode_filter: None,
            }),
            watchpoints: core::array::from_fn(|_| Watchpoint {
                active: false,
                addr: 0,
                len: 0,
                kind: WatchKind::Write,
            }),
            exception_bps: core::array::from_fn(|_| ExceptionBreakpoint {
                active: false,
                vector: 0,
                addr_lo: None,
                addr_hi: None,
            }),
            steps_remaining: 0,
            last_cmd: String::new(),
            silent: false,
        })
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Enable the debugger. Call once at startup.
/// Immediately breaks before the first instruction so the user can
/// set breakpoints and configure the session.
pub fn enable() {
    ENABLED.store(true, Ordering::SeqCst);
    BREAK_NEXT.store(true, Ordering::SeqCst); // break at first instruction
    let _ = state(); // force init
    eprintln!("[dbg] Debugger enabled. Will break at first instruction.");
}

/// Is the debugger enabled?
#[inline(always)]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Request an immediate break (e.g. from Ctrl-C handler).
pub fn request_break() {
    BREAK_NEXT.store(true, Ordering::SeqCst);
}

/// Called before each instruction in the interpreter loop.
/// Returns true if we should stop and enter the interactive prompt.
#[inline(always)]
pub fn should_break(cpu: &Cpu, _phys_addr: u64) -> bool {
    if !ENABLED.load(Ordering::Relaxed) {
        return false;
    }

    // Ctrl-C / explicit break
    if BREAK_NEXT.swap(false, Ordering::SeqCst) {
        return true;
    }

    // Instruction count breakpoint
    let ic_bp = BREAK_AT_IC.load(Ordering::Relaxed);
    if ic_bp != 0 && cpu.instruction_count >= ic_bp {
        BREAK_AT_IC.store(0, Ordering::Relaxed);
        return true;
    }

    let st = state();

    // Single-step
    if st.steps_remaining > 0 {
        st.steps_remaining -= 1;
        if st.steps_remaining == 0 {
            return true;
        }
    }

    // Code breakpoints
    let linear = cpu.regs.seg[SegReg::Cs as usize].base.wrapping_add(cpu.regs.rip);
    for bp in st.breakpoints.iter_mut() {
        if bp.active && bp.address == linear {
            if let Some(mode) = bp.mode_filter {
                if cpu.mode != mode {
                    continue;
                }
            }
            if bp.once {
                bp.active = false;
            }
            return true;
        }
    }

    false
}

/// Called when an exception is about to be injected into the guest.
/// Returns true if we should break.
pub fn on_exception(vector: u8, error_code: Option<u32>, cr2: u64, cpu: &Cpu) -> bool {
    if !ENABLED.load(Ordering::Relaxed) {
        return false;
    }

    let st = state();
    for ebp in st.exception_bps.iter() {
        if ebp.active && ebp.vector == vector {
            if let (Some(lo), Some(hi)) = (ebp.addr_lo, ebp.addr_hi) {
                if cr2 < lo || cr2 > hi {
                    continue;
                }
            }
            eprintln!("[dbg] Exception breakpoint: #{} err={:?} CR2={:#010x} at CS:IP={:04x}:{:#010x}",
                vector, error_code, cr2,
                cpu.regs.seg[SegReg::Cs as usize].selector, cpu.regs.rip);
            return true;
        }
    }
    false
}

/// Called on physical memory writes. Returns true if a watchpoint hit.
#[inline(always)]
pub fn on_mem_write(phys_addr: u64, len: u64) -> bool {
    if !ENABLED.load(Ordering::Relaxed) {
        return false;
    }
    check_watchpoints(phys_addr, len, WatchKind::Write)
}

/// Called on physical memory reads. Returns true if a watchpoint hit.
#[inline(always)]
pub fn on_mem_read(phys_addr: u64, len: u64) -> bool {
    if !ENABLED.load(Ordering::Relaxed) {
        return false;
    }
    check_watchpoints(phys_addr, len, WatchKind::Read)
}

fn check_watchpoints(addr: u64, len: u64, kind: WatchKind) -> bool {
    let st = state();
    for wp in st.watchpoints.iter() {
        if !wp.active {
            continue;
        }
        if wp.kind != kind && wp.kind != WatchKind::ReadWrite {
            if kind == WatchKind::Write && wp.kind == WatchKind::Read {
                continue;
            }
            if kind == WatchKind::Read && wp.kind == WatchKind::Write {
                continue;
            }
        }
        // Check overlap
        if addr < wp.addr + wp.len && addr + len > wp.addr {
            eprintln!("[dbg] Watchpoint hit: {:?} at phys {:#010x} len={}",
                match wp.kind { WatchKind::Write => "write", WatchKind::Read => "read", WatchKind::ReadWrite => "rw" },
                addr, len);
            return true;
        }
    }
    false
}

/// Enter the interactive debugger prompt. Blocks until the user
/// issues a "continue" or "step" command.
pub fn enter_prompt(
    cpu: &Cpu,
    memory: &GuestMemory,
    mmu: &Mmu,
    phys_ip: u64,
    reason: &str,
) {
    let st = state();
    st.silent = false;

    let linear = cpu.regs.seg[SegReg::Cs as usize].base.wrapping_add(cpu.regs.rip);
    eprintln!("\n[dbg] Break: {} (ic={})", reason, cpu.instruction_count);
    print_regs(cpu);
    disasm_at(cpu, memory, mmu, linear, 1);

    loop {
        eprint!("dbg> ");
        let line = match read_line_stdin() {
            Some(l) => l,
            None => {
                // EOF on stdin — continue running
                eprintln!("[dbg] stdin EOF, continuing");
                return;
            }
        };

        let line = line.trim().to_string();
        let cmd = if line.is_empty() {
            st.last_cmd.clone()
        } else {
            st.last_cmd = line.clone();
            line
        };

        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "help" | "h" | "?" => print_help(),

            // ── Execution control ────────────────────────────────
            "continue" | "c" => return,
            "step" | "s" => {
                let n = parts.get(1).and_then(|s| parse_u64(s)).unwrap_or(1);
                st.steps_remaining = n;
                return;
            }
            "stepi" | "si" => {
                // Step one instruction, then break
                st.steps_remaining = 1;
                return;
            }

            // ── Breakpoints ──────────────────────────────────────
            "break" | "b" => {
                if parts.len() < 2 {
                    eprintln!("Usage: break <linear_addr> [mode=real|prot|long]");
                    continue;
                }
                let addr = match parse_u64(parts[1]) {
                    Some(a) => a,
                    None => { eprintln!("Bad address: {}", parts[1]); continue; }
                };
                let mode_filter = parts.get(2).and_then(|m| match *m {
                    "real" => Some(Mode::RealMode),
                    "prot" | "protected" => Some(Mode::ProtectedMode),
                    "long" => Some(Mode::LongMode),
                    _ => None,
                });
                match add_breakpoint(addr, false, mode_filter) {
                    Some(i) => eprintln!("[dbg] Breakpoint #{} at {:#010x}", i, addr),
                    None => eprintln!("[dbg] No free breakpoint slots"),
                }
            }
            "tbreak" | "tb" => {
                if parts.len() < 2 {
                    eprintln!("Usage: tbreak <linear_addr>");
                    continue;
                }
                let addr = match parse_u64(parts[1]) {
                    Some(a) => a,
                    None => { eprintln!("Bad address"); continue; }
                };
                match add_breakpoint(addr, true, None) {
                    Some(i) => eprintln!("[dbg] Temp breakpoint #{} at {:#010x}", i, addr),
                    None => eprintln!("[dbg] No free breakpoint slots"),
                }
            }
            "delete" | "d" => {
                if parts.len() < 2 {
                    eprintln!("Usage: delete <bp_number> | delete all");
                    continue;
                }
                if parts[1] == "all" {
                    for bp in st.breakpoints.iter_mut() { bp.active = false; }
                    for wp in st.watchpoints.iter_mut() { wp.active = false; }
                    for ebp in st.exception_bps.iter_mut() { ebp.active = false; }
                    eprintln!("[dbg] All breakpoints/watchpoints deleted");
                } else if let Some(n) = parse_u64(parts[1]) {
                    let n = n as usize;
                    if n < MAX_BP {
                        st.breakpoints[n].active = false;
                        eprintln!("[dbg] Breakpoint #{} deleted", n);
                    }
                }
            }
            "info" | "i" => {
                if parts.len() < 2 || parts[1] == "break" || parts[1] == "b" {
                    list_breakpoints();
                }
                if parts.len() >= 2 && (parts[1] == "watch" || parts[1] == "w") {
                    list_watchpoints();
                }
            }

            // ── Exception breakpoints ────────────────────────────
            "catch" => {
                if parts.len() < 2 {
                    eprintln!("Usage: catch <vector> [addr_lo addr_hi]");
                    eprintln!("  e.g. catch 14         — break on all #PF");
                    eprintln!("  e.g. catch 14 0xfef00000 0xff000000  — #PF in range");
                    continue;
                }
                let vec = match parse_u64(parts[1]) {
                    Some(v) => v as u8,
                    None => { eprintln!("Bad vector"); continue; }
                };
                let (lo, hi) = if parts.len() >= 4 {
                    (parse_u64(parts[2]), parse_u64(parts[3]))
                } else {
                    (None, None)
                };
                match add_exception_bp(vec, lo, hi) {
                    Some(i) => eprintln!("[dbg] Exception breakpoint #{} on vector {}", i, vec),
                    None => eprintln!("[dbg] No free exception breakpoint slots"),
                }
            }

            // ── Watchpoints ──────────────────────────────────────
            "watch" | "w" => {
                if parts.len() < 2 {
                    eprintln!("Usage: watch <phys_addr> [len] [r|w|rw]");
                    continue;
                }
                let addr = match parse_u64(parts[1]) {
                    Some(a) => a,
                    None => { eprintln!("Bad address"); continue; }
                };
                let len = parts.get(2).and_then(|s| parse_u64(s)).unwrap_or(4);
                let kind = match parts.get(3).copied() {
                    Some("r") => WatchKind::Read,
                    Some("rw") => WatchKind::ReadWrite,
                    _ => WatchKind::Write,
                };
                match add_watchpoint(addr, len, kind) {
                    Some(i) => eprintln!("[dbg] Watchpoint #{} at phys {:#010x} len={}", i, addr, len),
                    None => eprintln!("[dbg] No free watchpoint slots"),
                }
            }

            // ── IC breakpoint ────────────────────────────────────
            "bic" => {
                if parts.len() < 2 {
                    eprintln!("Usage: bic <instruction_count>");
                    continue;
                }
                let ic = match parse_u64(parts[1]) {
                    Some(v) => v,
                    None => { eprintln!("Bad count"); continue; }
                };
                BREAK_AT_IC.store(ic, Ordering::Relaxed);
                eprintln!("[dbg] Will break at ic={}", ic);
            }

            // ── Register display ─────────────────────────────────
            "regs" | "r" => print_regs(cpu),
            "sregs" => print_sregs(cpu),
            "cregs" => print_cregs(cpu),

            // ── Disassembly ──────────────────────────────────────
            "disasm" | "u" => {
                let addr = parts.get(1).and_then(|s| parse_u64(s)).unwrap_or(linear);
                let count = parts.get(2).and_then(|s| parse_u64(s)).unwrap_or(10) as usize;
                disasm_at(cpu, memory, mmu, addr, count);
            }

            // ── Memory dump ──────────────────────────────────────
            "xp" => {
                // Examine physical memory
                if parts.len() < 2 {
                    eprintln!("Usage: xp <phys_addr> [count]");
                    continue;
                }
                let addr = match parse_u64(parts[1]) {
                    Some(a) => a,
                    None => { eprintln!("Bad address"); continue; }
                };
                let count = parts.get(2).and_then(|s| parse_u64(s)).unwrap_or(64) as usize;
                dump_phys(memory, addr, count);
            }
            "xv" | "x" => {
                // Examine virtual/linear memory
                if parts.len() < 2 {
                    eprintln!("Usage: xv <linear_addr> [count]");
                    continue;
                }
                let addr = match parse_u64(parts[1]) {
                    Some(a) => a,
                    None => { eprintln!("Bad address"); continue; }
                };
                let count = parts.get(2).and_then(|s| parse_u64(s)).unwrap_or(64) as usize;
                dump_linear(cpu, memory, mmu, addr, count);
            }

            // ── Page table walk ──────────────────────────────────
            "ptwalk" | "pt" => {
                if parts.len() < 2 {
                    eprintln!("Usage: ptwalk <linear_addr>");
                    continue;
                }
                let addr = match parse_u64(parts[1]) {
                    Some(a) => a,
                    None => { eprintln!("Bad address"); continue; }
                };
                ptwalk(cpu, memory, mmu, addr);
            }
            "ptdump" => {
                // Dump all present PDEs
                ptdump(cpu, memory);
            }

            // ── Stack trace ──────────────────────────────────────
            "bt" | "backtrace" => {
                let depth = parts.get(1).and_then(|s| parse_u64(s)).unwrap_or(20) as usize;
                backtrace(cpu, memory, mmu, depth);
            }

            // ── Misc ─────────────────────────────────────────────
            "mode" => {
                eprintln!("[dbg] CPU mode: {:?}, CPL={}", cpu.mode, cpu.regs.cpl);
            }
            "ic" => {
                eprintln!("[dbg] Instruction count: {}", cpu.instruction_count);
            }

            _ => {
                eprintln!("[dbg] Unknown command: '{}'. Type 'help' for commands.", parts[0]);
            }
        }
    }
}

// ── Breakpoint management ────────────────────────────────────────────

fn add_breakpoint(addr: u64, once: bool, mode_filter: Option<Mode>) -> Option<usize> {
    let st = state();
    for (i, bp) in st.breakpoints.iter_mut().enumerate() {
        if !bp.active {
            bp.active = true;
            bp.address = addr;
            bp.once = once;
            bp.mode_filter = mode_filter;
            return Some(i);
        }
    }
    None
}

fn add_watchpoint(addr: u64, len: u64, kind: WatchKind) -> Option<usize> {
    let st = state();
    for (i, wp) in st.watchpoints.iter_mut().enumerate() {
        if !wp.active {
            wp.active = true;
            wp.addr = addr;
            wp.len = len;
            wp.kind = kind;
            return Some(i);
        }
    }
    None
}

fn add_exception_bp(vector: u8, addr_lo: Option<u64>, addr_hi: Option<u64>) -> Option<usize> {
    let st = state();
    for (i, ebp) in st.exception_bps.iter_mut().enumerate() {
        if !ebp.active {
            ebp.active = true;
            ebp.vector = vector;
            ebp.addr_lo = addr_lo;
            ebp.addr_hi = addr_hi;
            return Some(i);
        }
    }
    None
}

fn list_breakpoints() {
    let st = state();
    eprintln!("[dbg] Code breakpoints:");
    for (i, bp) in st.breakpoints.iter().enumerate() {
        if bp.active {
            eprintln!("  #{}: {:#010x}{}{}", i, bp.address,
                if bp.once { " (once)" } else { "" },
                match bp.mode_filter {
                    Some(Mode::RealMode) => " [real]",
                    Some(Mode::ProtectedMode) => " [prot]",
                    Some(Mode::LongMode) => " [long]",
                    None => "",
                });
        }
    }
    eprintln!("[dbg] Exception breakpoints:");
    for (i, ebp) in st.exception_bps.iter().enumerate() {
        if ebp.active {
            let range = match (ebp.addr_lo, ebp.addr_hi) {
                (Some(lo), Some(hi)) => format!(" CR2 in [{:#x}, {:#x}]", lo, hi),
                _ => String::new(),
            };
            eprintln!("  #{}: vector {}{}", i, ebp.vector, range);
        }
    }
}

fn list_watchpoints() {
    let st = state();
    eprintln!("[dbg] Watchpoints:");
    for (i, wp) in st.watchpoints.iter().enumerate() {
        if wp.active {
            eprintln!("  #{}: phys {:#010x} len={} kind={}", i, wp.addr, wp.len,
                match wp.kind { WatchKind::Write => "w", WatchKind::Read => "r", WatchKind::ReadWrite => "rw" });
        }
    }
}

// ── Display helpers ──────────────────────────────────────────────────

fn print_regs(cpu: &Cpu) {
    let flags = cpu.regs.rflags as u32;
    let flag_str = format!("{}{}{}{}{}{}{}{}{}",
        if flags & (1 << 11) != 0 { "OF " } else { "" },
        if flags & (1 << 10) != 0 { "DF " } else { "" },
        if flags & (1 << 9) != 0 { "IF " } else { "" },
        if flags & (1 << 7) != 0 { "SF " } else { "" },
        if flags & (1 << 6) != 0 { "ZF " } else { "" },
        if flags & (1 << 4) != 0 { "AF " } else { "" },
        if flags & (1 << 2) != 0 { "PF " } else { "" },
        if flags & (1 << 0) != 0 { "CF " } else { "" },
        if flags & (1 << 8) != 0 { "TF " } else { "" },
    );
    match cpu.mode {
        Mode::RealMode | Mode::ProtectedMode => {
            eprintln!("  EAX={:08X}  EBX={:08X}  ECX={:08X}  EDX={:08X}",
                cpu.regs.gpr[0] as u32, cpu.regs.gpr[3] as u32,
                cpu.regs.gpr[1] as u32, cpu.regs.gpr[2] as u32);
            eprintln!("  ESI={:08X}  EDI={:08X}  EBP={:08X}  ESP={:08X}",
                cpu.regs.gpr[6] as u32, cpu.regs.gpr[7] as u32,
                cpu.regs.gpr[5] as u32, cpu.regs.sp() as u32);
            eprintln!("  CS={:04X}  DS={:04X}  ES={:04X}  SS={:04X}  FS={:04X}  GS={:04X}",
                cpu.regs.seg[SegReg::Cs as usize].selector,
                cpu.regs.seg[SegReg::Ds as usize].selector,
                cpu.regs.seg[SegReg::Es as usize].selector,
                cpu.regs.seg[SegReg::Ss as usize].selector,
                cpu.regs.seg[SegReg::Fs as usize].selector,
                cpu.regs.seg[SegReg::Gs as usize].selector);
            eprintln!("  EIP={:08X}  EFLAGS={:08X}  [{}]",
                cpu.regs.rip as u32, flags, flag_str.trim());
            eprintln!("  CR0={:08X}  CR2={:08X}  CR3={:08X}  CR4={:08X}",
                cpu.regs.cr0 as u32, cpu.regs.cr2 as u32,
                cpu.regs.cr3 as u32, cpu.regs.cr4 as u32);
            eprintln!("  Mode={:?}  CPL={}  ic={}", cpu.mode, cpu.regs.cpl, cpu.instruction_count);
        }
        Mode::LongMode => {
            eprintln!("  RAX={:016X}  RBX={:016X}  RCX={:016X}  RDX={:016X}",
                cpu.regs.gpr[0], cpu.regs.gpr[3], cpu.regs.gpr[1], cpu.regs.gpr[2]);
            eprintln!("  RSI={:016X}  RDI={:016X}  RBP={:016X}  RSP={:016X}",
                cpu.regs.gpr[6], cpu.regs.gpr[7], cpu.regs.gpr[5], cpu.regs.sp());
            eprintln!("  R8 ={:016X}  R9 ={:016X}  R10={:016X}  R11={:016X}",
                cpu.regs.gpr[8], cpu.regs.gpr[9], cpu.regs.gpr[10], cpu.regs.gpr[11]);
            eprintln!("  R12={:016X}  R13={:016X}  R14={:016X}  R15={:016X}",
                cpu.regs.gpr[12], cpu.regs.gpr[13], cpu.regs.gpr[14], cpu.regs.gpr[15]);
            eprintln!("  CS={:04X}  RIP={:016X}  RFLAGS={:016X}  [{}]",
                cpu.regs.seg[SegReg::Cs as usize].selector, cpu.regs.rip, cpu.regs.rflags, flag_str.trim());
            eprintln!("  CR0={:016X}  CR3={:016X}  CR4={:016X}  EFER={:016X}",
                cpu.regs.cr0, cpu.regs.cr3, cpu.regs.cr4, cpu.regs.efer);
        }
    }
}

fn print_sregs(cpu: &Cpu) {
    let names = ["ES", "CS", "SS", "DS", "FS", "GS"];
    for (i, name) in names.iter().enumerate() {
        let seg = &cpu.regs.seg[i];
        eprintln!("  {}={:04X}  base={:08X}  limit={:08X}  big={}  dpl={}",
            name, seg.selector, seg.base as u32, seg.limit, seg.big as u8, seg.dpl);
    }
    eprintln!("  GDTR: base={:08X}  limit={:04X}",
        cpu.regs.gdtr.base as u32, cpu.regs.gdtr.limit);
    eprintln!("  IDTR: base={:08X}  limit={:04X}",
        cpu.regs.idtr.base as u32, cpu.regs.idtr.limit);
    eprintln!("  TR={:04X}  LDTR={:04X}", cpu.regs.tr, cpu.regs.ldtr);
}

fn print_cregs(cpu: &Cpu) {
    eprintln!("  CR0={:08X}  CR2={:08X}  CR3={:08X}  CR4={:08X}",
        cpu.regs.cr0 as u32, cpu.regs.cr2 as u32,
        cpu.regs.cr3 as u32, cpu.regs.cr4 as u32);
    eprintln!("  EFER={:016X}", cpu.regs.efer);
    let apic_base = cpu.regs.read_msr(0x1B);
    eprintln!("  IA32_APIC_BASE={:016X}", apic_base);
}

fn disasm_at(cpu: &Cpu, memory: &GuestMemory, mmu: &Mmu, linear: u64, count: usize) {
    let mut addr = linear;
    for _ in 0..count {
        // Translate linear → physical
        let phys = if (cpu.regs.cr0 & 0x8000_0001) == 0x8000_0001 {
            // Paging enabled
            match translate_linear_to_phys(cpu, memory, addr) {
                Some(p) => p,
                None => {
                    eprintln!("  {:#010x}: <unmapped>", addr);
                    break;
                }
            }
        } else {
            addr
        };

        // Read up to 15 bytes
        let mut bytes = [0u8; 15];
        let mut valid = 0;
        for i in 0..15 {
            match memory.read_u8(phys + i as u64) {
                Ok(b) => { bytes[i] = b; valid = i + 1; }
                Err(_) => break,
            }
        }
        if valid == 0 {
            eprintln!("  {:#010x}: <cannot read>", addr);
            break;
        }

        // Try to decode using our decoder
        match cpu.decoder.decode(&*memory, phys) {
            Ok(inst) => {
                let len = inst.length as usize;
                let mut hex = String::new();
                for i in 0..len.min(8) {
                    use core::fmt::Write;
                    let _ = write!(hex, "{:02x} ", bytes[i]);
                }
                // Pad hex to fixed width
                while hex.len() < 24 {
                    hex.push(' ');
                }
                eprintln!("  {:#010x}: {} {}", addr, hex, format_instruction(&inst));
                addr += len as u64;
            }
            Err(_) => {
                eprintln!("  {:#010x}: {:02x} {:02x} {:02x} {:02x}  <decode error>",
                    addr, bytes[0], bytes[1], bytes[2], bytes[3]);
                addr += 1;
            }
        }
    }
}

fn format_instruction(inst: &crate::instruction::DecodedInst) -> String {
    // Minimal instruction formatting — show opcode map + opcode byte
    use crate::instruction::OpcodeMap;
    let map = match inst.opcode_map {
        OpcodeMap::Primary => "",
        OpcodeMap::Secondary => "0F ",
        _ => "?? ",
    };
    let mut s = format!("{}{:02X}", map, inst.opcode as u8);
    // Show first operand if present (Operand doesn't impl PartialEq, check via Debug)
    let op0_dbg = format!("{:?}", inst.operands[0]);
    if op0_dbg != "None" {
        use core::fmt::Write;
        let _ = write!(s, "  ; {}", op0_dbg);
    }
    s
}

fn dump_phys(memory: &GuestMemory, addr: u64, count: usize) {
    let count = count.min(4096);
    let mut offset = 0;
    while offset < count {
        let line_addr = addr + offset as u64;
        let mut hex = String::new();
        let mut ascii = String::new();
        for i in 0..16 {
            if offset + i < count {
                let b = memory.read_u8(line_addr + i as u64).unwrap_or(0xFF);
                use core::fmt::Write;
                let _ = write!(hex, "{:02x} ", b);
                ascii.push(if b >= 0x20 && b < 0x7f { b as char } else { '.' });
            }
        }
        eprintln!("  {:08X}: {}  {}", line_addr as u32, hex, ascii);
        offset += 16;
    }
}

fn dump_linear(cpu: &Cpu, memory: &GuestMemory, mmu: &Mmu, linear: u64, count: usize) {
    let count = count.min(4096);
    let paging = (cpu.regs.cr0 & 0x8000_0001) == 0x8000_0001;
    let mut offset = 0;
    while offset < count {
        let la = linear + offset as u64;
        let phys = if paging {
            match translate_linear_to_phys(cpu, memory, la) {
                Some(p) => p,
                None => {
                    eprintln!("  {:08X}: <unmapped>", la as u32);
                    // Skip to next page
                    let next_page = (la + 0x1000) & !0xFFF;
                    offset = (next_page - linear) as usize;
                    continue;
                }
            }
        } else {
            la
        };

        let mut hex = String::new();
        let mut ascii = String::new();
        for i in 0..16 {
            if offset + i < count {
                let b = memory.read_u8(phys + i as u64).unwrap_or(0xFF);
                use core::fmt::Write;
                let _ = write!(hex, "{:02x} ", b);
                ascii.push(if b >= 0x20 && b < 0x7f { b as char } else { '.' });
            }
        }
        eprintln!("  {:08X}: {}  {}", la as u32, hex, ascii);
        offset += 16;
    }
}

fn ptwalk(cpu: &Cpu, memory: &GuestMemory, _mmu: &Mmu, linear: u64) {
    let cr3 = cpu.regs.cr3;
    let cr4 = cpu.regs.cr4;
    let pae = (cr4 & (1 << 5)) != 0;

    eprintln!("[dbg] Page table walk for VA {:#010x}  CR3={:#010x}  PAE={}", linear, cr3, pae);

    if pae {
        eprintln!("  PAE mode not fully implemented in debugger yet");
        return;
    }

    // 2-level 32-bit paging
    let cr3_base = cr3 & 0xFFFF_F000;
    let pde_idx = (linear >> 22) & 0x3FF;
    let pte_idx = (linear >> 12) & 0x3FF;

    let pde_addr = cr3_base + pde_idx * 4;
    let pde = memory.read_u32(pde_addr).unwrap_or(0);
    eprintln!("  PDE[{}] @ phys {:#010x} = {:#010x}  {}",
        pde_idx, pde_addr, pde, format_pte_flags(pde as u64));

    if pde & 1 == 0 {
        eprintln!("  -> NOT PRESENT");
        return;
    }

    // Check PSE (4MB pages)
    let pse = (cr4 & (1 << 4)) != 0;
    if pse && (pde & (1 << 7)) != 0 {
        let phys = ((pde as u64) & 0xFFC0_0000) | (linear & 0x003F_FFFF);
        eprintln!("  -> 4MB page, phys = {:#010x}", phys);
        return;
    }

    let pt_base = (pde as u64) & 0xFFFF_F000;
    let pte_addr = pt_base + pte_idx * 4;
    let pte = memory.read_u32(pte_addr).unwrap_or(0);
    eprintln!("  PTE[{}] @ phys {:#010x} = {:#010x}  {}",
        pte_idx, pte_addr, pte, format_pte_flags(pte as u64));

    if pte & 1 == 0 {
        eprintln!("  -> NOT PRESENT (PTE value for OS use: {:#010x})", pte);
        return;
    }

    let phys = ((pte as u64) & 0xFFFF_F000) | (linear & 0xFFF);
    eprintln!("  -> phys = {:#010x}", phys);

    // Read first 16 bytes at physical address
    let mut bytes_str = String::new();
    for i in 0..16 {
        let b = memory.read_u8(phys + i).unwrap_or(0xFF);
        use core::fmt::Write;
        let _ = write!(bytes_str, "{:02x} ", b);
    }
    eprintln!("  -> data: {}", bytes_str);
}

fn ptdump(cpu: &Cpu, memory: &GuestMemory) {
    let cr3 = cpu.regs.cr3 & 0xFFFF_F000;
    eprintln!("[dbg] Page directory at CR3={:#010x}:", cr3);
    for i in 0..1024u64 {
        let pde = memory.read_u32(cr3 + i * 4).unwrap_or(0);
        if pde & 1 != 0 {
            let va_base = i << 22;
            eprintln!("  PDE[{:>4}] VA {:#010x}-{:#010x} = {:#010x} {}",
                i, va_base, va_base + 0x3FFFFF, pde, format_pte_flags(pde as u64));
        }
    }
}

fn format_pte_flags(pte: u64) -> String {
    if pte & 1 == 0 {
        return String::from("[NOT PRESENT]");
    }
    let mut s = String::from("[P");
    if pte & 2 != 0 { s.push_str("|RW"); }
    if pte & 4 != 0 { s.push_str("|US"); }
    if pte & 8 != 0 { s.push_str("|PWT"); }
    if pte & 0x10 != 0 { s.push_str("|PCD"); }
    if pte & 0x20 != 0 { s.push_str("|A"); }
    if pte & 0x40 != 0 { s.push_str("|D"); }
    if pte & 0x80 != 0 { s.push_str("|PS"); }
    if pte & 0x100 != 0 { s.push_str("|G"); }
    s.push(']');
    s
}

fn backtrace(cpu: &Cpu, memory: &GuestMemory, mmu: &Mmu, depth: usize) {
    eprintln!("[dbg] Stack backtrace (EBP chain):");
    let paging = (cpu.regs.cr0 & 0x8000_0001) == 0x8000_0001;

    let mut ebp = cpu.regs.gpr[5]; // EBP
    let rip = cpu.regs.rip;
    eprintln!("  #0  EIP={:#010x}  EBP={:#010x}  ESP={:#010x}",
        rip as u32, ebp as u32, cpu.regs.sp() as u32);

    for frame in 1..=depth {
        if ebp == 0 || ebp < 0x1000 {
            break;
        }
        // Read return address at [ebp+4] and saved ebp at [ebp]
        let (saved_ebp, ret_addr) = if paging {
            let p1 = match translate_linear_to_phys(cpu, memory, ebp) {
                Some(p) => p,
                None => { eprintln!("  #{}: EBP={:#010x} <unmapped>", frame, ebp as u32); break; }
            };
            let p2 = match translate_linear_to_phys(cpu, memory, ebp + 4) {
                Some(p) => p,
                None => break,
            };
            (memory.read_u32(p1).unwrap_or(0) as u64, memory.read_u32(p2).unwrap_or(0) as u64)
        } else {
            (memory.read_u32(ebp).unwrap_or(0) as u64, memory.read_u32(ebp + 4).unwrap_or(0) as u64)
        };

        eprintln!("  #{}  EIP={:#010x}  EBP={:#010x}", frame, ret_addr as u32, saved_ebp as u32);

        if saved_ebp <= ebp {
            // Not ascending — corrupted or end of chain
            break;
        }
        ebp = saved_ebp;
    }
}

// ── Page table translation helper ────────────────────────────────────

fn translate_linear_to_phys(cpu: &Cpu, memory: &GuestMemory, linear: u64) -> Option<u64> {
    let cr3 = cpu.regs.cr3 & 0xFFFF_F000;
    let pde_idx = (linear >> 22) & 0x3FF;
    let pde = memory.read_u32(cr3 + pde_idx * 4).ok()?;
    if pde & 1 == 0 { return None; }

    // PSE 4MB check
    if (cpu.regs.cr4 & (1 << 4)) != 0 && (pde & (1 << 7)) != 0 {
        let phys = ((pde as u64) & 0xFFC0_0000) | (linear & 0x003F_FFFF);
        return Some(phys);
    }

    let pt_base = (pde as u64) & 0xFFFF_F000;
    let pte_idx = (linear >> 12) & 0x3FF;
    let pte = memory.read_u32(pt_base + pte_idx * 4).ok()?;
    if pte & 1 == 0 { return None; }

    let phys = ((pte as u64) & 0xFFFF_F000) | (linear & 0xFFF);
    Some(phys)
}

// ── stdin reading ────────────────────────────────────────────────────

fn read_line_stdin() -> Option<String> {
    let mut buf = [0u8; 256];
    let mut line = String::new();
    loop {
        let n = unsafe {
            libc_read(0, buf.as_mut_ptr(), 1)
        };
        if n <= 0 { return if line.is_empty() { None } else { Some(line) }; }
        let ch = buf[0];
        if ch == b'\n' { return Some(line); }
        line.push(ch as char);
    }
}

#[cfg(unix)]
extern "C" {
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
}

#[cfg(unix)]
unsafe fn libc_read(fd: i32, buf: *mut u8, count: usize) -> isize {
    unsafe { read(fd, buf, count) }
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetStdHandle(nStdHandle: u32) -> *mut core::ffi::c_void;
    fn ReadFile(h: *mut core::ffi::c_void, buf: *mut u8, len: u32, read: *mut u32, ovl: *mut core::ffi::c_void) -> i32;
}

#[cfg(windows)]
unsafe fn libc_read(_fd: i32, buf: *mut u8, count: usize) -> isize {
    let h = unsafe { GetStdHandle(0xFFFF_FFF6u32) }; // STD_INPUT_HANDLE
    let mut n: u32 = 0;
    let ok = unsafe { ReadFile(h, buf, count as u32, &mut n, core::ptr::null_mut()) };
    if ok == 0 { -1 } else { n as isize }
}

// ── Number parsing ───────────────────────────────────────────────────

fn parse_u64(s: &str) -> Option<u64> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else if s.starts_with(|c: char| c.is_ascii_digit()) {
        // Try decimal first, then hex
        s.parse::<u64>().ok().or_else(|| u64::from_str_radix(s, 16).ok())
    } else {
        None
    }
}

// ── Help ─────────────────────────────────────────────────────────────

fn print_help() {
    eprintln!("Commands:");
    eprintln!("  c, continue         — Resume execution");
    eprintln!("  s [N], step [N]     — Step N instructions (default 1), show regs after");
    eprintln!("  si, stepi           — Step 1 instruction");
    eprintln!("  b <addr> [mode]     — Set breakpoint at linear address");
    eprintln!("  tb <addr>           — Set one-shot breakpoint");
    eprintln!("  d <N> | d all       — Delete breakpoint/watchpoint");
    eprintln!("  i b                 — List breakpoints");
    eprintln!("  i w                 — List watchpoints");
    eprintln!("  catch <vec> [lo hi] — Break on exception (e.g. catch 14)");
    eprintln!("  w <phys> [len] [rw] — Set watchpoint on physical address");
    eprintln!("  bic <count>         — Break at instruction count");
    eprintln!("  r, regs             — Show registers");
    eprintln!("  sregs               — Show segment registers + GDTR/IDTR");
    eprintln!("  cregs               — Show control registers");
    eprintln!("  u [addr] [count]    — Disassemble (default: at EIP, 10 insns)");
    eprintln!("  x <addr> [count]    — Examine virtual memory (hex dump)");
    eprintln!("  xp <phys> [count]   — Examine physical memory (hex dump)");
    eprintln!("  pt <addr>           — Page table walk for linear address");
    eprintln!("  ptdump              — Dump all present page directory entries");
    eprintln!("  bt [depth]          — Stack backtrace via EBP chain");
    eprintln!("  mode                — Show current CPU mode");
    eprintln!("  ic                  — Show instruction count");
    eprintln!("  help                — This help");
    eprintln!("  (Enter)             — Repeat last command");
}
