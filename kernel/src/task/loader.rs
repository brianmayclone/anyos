//! User program loader: reads ELF or flat binaries from the filesystem, creates an
//! isolated per-process PML4, maps code/stack/DLL pages, and spawns a kernel
//! trampoline thread that transitions to Ring 3 via `iretq`.

use crate::memory::address::VirtAddr;
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;

/// Default load address for flat binaries (128 MiB).
/// ELF binaries use their own vaddr from program headers.
const PROGRAM_LOAD_ADDR: u64 = 0x0800_0000;

/// User stack is allocated below this address (3 GiB).
/// Stack grows downward.  Placed at the top of the user address space
/// (like Linux/FreeBSD) so the heap and mmap regions below can grow freely.
const USER_STACK_TOP: u64 = 0xC000_0000;

/// Fixed virtual address for the per-process signal return trampoline page.
/// This is currently only used by the x86 compat signal-return path.
/// ARM64 keeps the native 64-bit signal path trampoline-free for now.
pub const SIGRETURN_TRAMPOLINE_ADDR: u64 = 0xC000_0000;

/// Number of pages for the user stack (8 MiB = 2048 pages).
/// Matches the Linux default of 8 MiB.
const USER_STACK_PAGES: u64 = 2048;

const PAGE_SIZE: u64 = 4096;
const PAGE_WRITABLE: u64 = 0x02;
const PAGE_USER: u64 = 0x04;

/// ELF program header flag: segment is executable.
const PF_X: u32 = 1;
/// ELF program header flag: segment is writable.
const PF_W: u32 = 2;

/// Maximum random page offset applied to the stack top (ASLR).
/// 256 pages = 1 MiB of entropy. The allocated 8 MiB stack region provides
/// ample headroom, so the bottom of the used region still has plenty of space.
const ASLR_STACK_MAX_PAGES: u32 = 256;

/// Maximum random page offset applied to the mmap base (ASLR).
/// 4096 pages = 16 MiB of entropy within the 1.25 GiB mmap region.
pub const ASLR_MMAP_MAX_PAGES: u32 = 4096;

#[cfg(target_arch = "aarch64")]
unsafe fn sync_user_text_range_for_exec(start: u64, len: usize) {
    if len == 0 {
        return;
    }

    let ctr_el0: u64;
    core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr_el0, options(nomem, nostack));

    let dline = 4usize << ((ctr_el0 >> 16) & 0xF);
    let iline = 4usize << (ctr_el0 & 0xF);
    let dline = dline.max(16);
    let iline = iline.max(16);

    let start = start as usize;
    let end = start.saturating_add(len);
    let dstart = start & !(dline - 1);
    let istart = start & !(iline - 1);

    let mut addr = dstart;
    while addr < end {
        core::arch::asm!("dc cvau, {}", in(reg) addr, options(nostack, preserves_flags));
        addr += dline;
    }
    core::arch::asm!("dsb ish", options(nomem, nostack));

    let mut addr = istart;
    while addr < end {
        core::arch::asm!("ic ivau, {}", in(reg) addr, options(nostack, preserves_flags));
        addr += iline;
    }
    core::arch::asm!("dsb ish", options(nomem, nostack));
    core::arch::asm!("isb", options(nomem, nostack));
}

/// Generate a random page offset in `[0, max_pages)` using hardware RNG
/// with a counter-based fallback.
///
/// Returns a page count (multiply by PAGE_SIZE to get a byte offset).
pub fn random_page_offset(max_pages: u32) -> u32 {
    if max_pages == 0 {
        return 0;
    }
    let entropy: u64 = hw_entropy().unwrap_or_else(counter_entropy);
    (entropy % max_pages as u64) as u32
}

/// Try to get hardware entropy (RDRAND on x86, RNDR on ARM64).
fn hw_entropy() -> Option<u64> {
    #[cfg(target_arch = "x86_64")]
    {
        if !crate::arch::x86::cpuid::features().rdrand {
            return None;
        }
        let raw: u64;
        let ok: u8;
        unsafe {
            core::arch::asm!(
                "rdrand {val}",
                "setc {ok}",
                val = out(reg) raw,
                ok  = out(reg_byte) ok,
                options(nostack, nomem),
            );
        }
        if ok != 0 { Some(raw) } else { None }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if !crate::arch::arm64::cpu_features::HAS_RNG.load(core::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let val: u64;
        unsafe {
            // RNDR: random number; returns 0 on failure (FEAT_RNG required)
            core::arch::asm!("mrs {}, s3_3_c2_c4_0", out(reg) val, options(nomem, nostack));
        }
        if val != 0 { Some(val) } else { None }
    }
}

/// Counter-based entropy fallback (TSC on x86, CNTPCT on ARM64).
fn counter_entropy() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        let lo: u32;
        let hi: u32;
        unsafe {
            core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem));
        }
        let tsc = ((hi as u64) << 32) | (lo as u64);
        tsc ^ (tsc >> 17) ^ (tsc << 31)
    }
    #[cfg(target_arch = "aarch64")]
    {
        let cnt: u64;
        unsafe {
            core::arch::asm!("mrs {}, cntpct_el0", out(reg) cnt, options(nomem, nostack));
        }
        cnt ^ (cnt >> 17) ^ (cnt << 31)
    }
}

/// Max concurrent pending programs (no heap allocation needed).
///
/// This must comfortably exceed bursty thread/process creation patterns such as
/// kstress, browser worker fan-out, and parallel child spawns at boot.
const MAX_PENDING_PROGRAMS: usize = 256;
const MAX_PENDING_FORKS: usize = 64;
const PENDING_LOOKUP_SPIN_LIMIT: usize = 65536;

/// Slot holding the entry point and stack pointer for a newly spawned user thread.
///
/// The trampoline thread looks up its TID in this table to learn where to jump
/// after the context switch into the new address space.
#[derive(Clone, Copy)]
struct PendingSlot {
    tid: u32,
    entry: u64,
    user_stack: u64,
    user_lr: u64,
    used: bool,
}

impl PendingSlot {
    const fn empty() -> Self {
        PendingSlot { tid: 0, entry: 0, user_stack: 0, user_lr: 0, used: false }
    }
}

static PENDING_PROGRAMS: Spinlock<[PendingSlot; MAX_PENDING_PROGRAMS]> =
    Spinlock::new([PendingSlot::empty(); MAX_PENDING_PROGRAMS]);

fn try_store_pending_program(
    tid: u32,
    entry: u64,
    user_stack: u64,
    user_lr: u64,
) -> bool {
    let mut slots = PENDING_PROGRAMS.lock();
    let Some(slot) = slots.iter_mut().find(|s| !s.used) else {
        return false;
    };
    slot.tid = tid;
    slot.entry = entry;
    slot.user_stack = user_stack;
    slot.user_lr = user_lr;
    slot.used = true;
    true
}

fn take_pending_program(
    tid: u32,
    trampoline_name: &str,
) -> (u64, u64, u64) {
    let mut spins = 0usize;
    loop {
        {
            let mut slots = PENDING_PROGRAMS.lock();
            if let Some(slot) = slots.iter_mut().find(|s| s.used && s.tid == tid) {
                let e = slot.entry;
                let s = slot.user_stack;
                let lr = slot.user_lr;
                slot.used = false;
                return (e, s, lr);
            }

            if spins >= PENDING_LOOKUP_SPIN_LIMIT {
                let used = slots.iter().filter(|s| s.used).count();
                crate::serial_println!(
                    "[loader] {}: no pending slot for tid={} after {} spins (used_slots={})",
                    trampoline_name,
                    tid,
                    spins,
                    used,
                );
                for slot in slots.iter().filter(|s| s.used).take(8) {
                    crate::serial_println!(
                        "[loader]   pending tid={} entry={:#x} stack={:#x}",
                        slot.tid,
                        slot.entry,
                        slot.user_stack,
                    );
                }
                drop(slots);
                crate::serial_println!(
                    "[loader] terminating tid={} after missing pending-program slot ({})",
                    tid,
                    trampoline_name,
                );
                crate::task::scheduler::kill_thread(tid);
                loop {
                    crate::task::scheduler::schedule();
                }
            }
        }

        spins += 1;
        if (spins & 0xFF) == 0 {
            crate::task::scheduler::schedule();
        } else {
            core::hint::spin_loop();
        }
    }
}

// =========================================================================
// fork() child state — saved parent registers for child to resume from
// =========================================================================
//
// The fork mechanism uses architecture-specific register state and return
// paths (IRETQ on x86_64, ERET on AArch64). Currently only implemented
// for x86_64.

/// User-mode register state saved by fork() for the child process.
/// The child's trampoline restores these via IRETQ (x86_64) or ERET (AArch64),
/// with the return register set to 0.
#[cfg(target_arch = "x86_64")]
#[repr(C)]
pub struct ForkChildRegs {
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    // IRETQ frame
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// User-mode register state saved by fork() for the child process on AArch64.
#[cfg(target_arch = "aarch64")]
#[repr(C)]
pub struct ForkChildRegs {
    pub x: [u64; 31],
    pub sp_el0: u64,
    pub elr_el1: u64,
    pub spsr_el1: u64,
    pub tpidr_el0: u64,
}

#[cfg(target_arch = "x86_64")]
struct ForkPendingSlot {
    tid: u32,
    used: bool,
    regs: ForkChildRegs,
}

#[cfg(target_arch = "aarch64")]
struct ForkPendingSlot {
    tid: u32,
    used: bool,
    regs: ForkChildRegs,
}

#[cfg(target_arch = "x86_64")]
impl ForkPendingSlot {
    const fn empty() -> Self {
        ForkPendingSlot {
            tid: 0,
            used: false,
            regs: ForkChildRegs {
                rbx: 0, rcx: 0, rdx: 0, rsi: 0, rdi: 0, rbp: 0,
                r8: 0, r9: 0, r10: 0, r11: 0, r12: 0, r13: 0, r14: 0, r15: 0,
                rip: 0, cs: 0, rflags: 0, rsp: 0, ss: 0,
            },
        }
    }
}

#[cfg(target_arch = "aarch64")]
impl ForkPendingSlot {
    const fn empty() -> Self {
        ForkPendingSlot {
            tid: 0,
            used: false,
            regs: ForkChildRegs {
                x: [0; 31],
                sp_el0: 0,
                elr_el1: 0,
                spsr_el1: 0,
                tpidr_el0: 0,
            },
        }
    }
}

#[cfg(target_arch = "x86_64")]
static PENDING_FORKS: Spinlock<[ForkPendingSlot; MAX_PENDING_FORKS]> =
    Spinlock::new([
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
    ]);

#[cfg(target_arch = "aarch64")]
static PENDING_FORKS: Spinlock<[ForkPendingSlot; MAX_PENDING_FORKS]> =
    Spinlock::new([
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
    ]);

/// Store the parent's register state for a fork() child to pick up.
#[cfg(target_arch = "x86_64")]
pub fn store_pending_fork(tid: u32, regs: ForkChildRegs) {
    let mut slots = PENDING_FORKS.lock();
    let slot = slots.iter_mut().find(|s| !s.used)
        .expect("Too many pending forks");
    slot.tid = tid;
    slot.regs = regs;
    slot.used = true;
}

/// Store the parent's register state for a fork() child to pick up.
#[cfg(target_arch = "aarch64")]
pub fn store_pending_fork(tid: u32, regs: ForkChildRegs) {
    let mut slots = PENDING_FORKS.lock();
    let slot = slots.iter_mut().find(|s| !s.used)
        .expect("Too many pending forks");
    slot.tid = tid;
    slot.regs = regs;
    slot.used = true;
}

/// Trampoline for fork() child threads.
/// Wakes up in kernel mode, retrieves saved parent registers, then IRETQ to user
/// mode with RAX=0 (fork child return value).
#[cfg(target_arch = "x86_64")]
pub extern "C" fn fork_child_trampoline() {
    let tid = crate::task::scheduler::current_tid();

    // Retrieve saved register state
    let regs = {
        let mut slots = PENDING_FORKS.lock();
        let slot = slots.iter_mut().find(|s| s.used && s.tid == tid)
            .expect("No pending fork state for child trampoline");
        // Copy regs out and free slot
        let r = ForkChildRegs {
            rbx: slot.regs.rbx, rcx: slot.regs.rcx, rdx: slot.regs.rdx,
            rsi: slot.regs.rsi, rdi: slot.regs.rdi, rbp: slot.regs.rbp,
            r8: slot.regs.r8, r9: slot.regs.r9, r10: slot.regs.r10,
            r11: slot.regs.r11, r12: slot.regs.r12, r13: slot.regs.r13,
            r14: slot.regs.r14, r15: slot.regs.r15,
            rip: slot.regs.rip, cs: slot.regs.cs,
            rflags: slot.regs.rflags, rsp: slot.regs.rsp, ss: slot.regs.ss,
        };
        slot.used = false;
        r
    };

    unsafe { fork_return_to_user(&regs); }
}

/// Trampoline for fork() child threads on AArch64.
#[cfg(target_arch = "aarch64")]
pub extern "C" fn fork_child_trampoline() {
    let tid = crate::task::scheduler::current_tid();

    let regs = {
        let mut slots = PENDING_FORKS.lock();
        let slot = slots.iter_mut().find(|s| s.used && s.tid == tid)
            .expect("No pending fork state for child trampoline");
        let r = ForkChildRegs {
            x: slot.regs.x,
            sp_el0: slot.regs.sp_el0,
            elr_el1: slot.regs.elr_el1,
            spsr_el1: slot.regs.spsr_el1,
            tpidr_el0: slot.regs.tpidr_el0,
        };
        slot.used = false;
        r
    };

    unsafe { arm64_fork_return_to_user(&regs); }
}

/// Restore all user-mode registers from a ForkChildRegs struct and IRETQ.
/// Sets RAX=0 (fork child return value). Never returns.
///
/// CRITICAL: Never hardcode register names (ax, rax, etc.) in asm! blocks when
/// `in(reg)` operands exist — LLVM may allocate the same register, causing
/// silent corruption of the pointer operand.
#[cfg(target_arch = "x86_64")]
unsafe fn fork_return_to_user(regs: *const ForkChildRegs) -> ! {
    crate::arch::x86::syscall_msr::debug_assert_gs_is_kernel();
    core::arch::asm!(
        "cli",
        // Set data segments for user mode — use {seg} operand, NEVER hardcode ax
        "mov ds, {seg:x}",
        "mov es, {seg:x}",
        "mov fs, {seg:x}",
        "mov gs, {seg:x}",
        // Build IRETQ frame from struct (field offsets in ForkChildRegs):
        // rbx=0, rcx=8, rdx=16, rsi=24, rdi=32, rbp=40,
        // r8=48, r9=56, r10=64, r11=72, r12=80, r13=88, r14=96, r15=104,
        // rip=112, cs=120, rflags=128, rsp=136, ss=144
        "push qword ptr [{p} + 144]",   // SS
        "push qword ptr [{p} + 136]",   // RSP
        "push qword ptr [{p} + 128]",   // RFLAGS
        "or qword ptr [rsp], 0x200",    // Ensure IF set (no hardcoded reg)
        "push qword ptr [{p} + 120]",   // CS
        "push qword ptr [{p} + 112]",   // RIP
        // Restore GPRs — {p} is still live, no hardcoded reg writes allowed
        "mov r15, [{p} + 104]",
        "mov r14, [{p} + 96]",
        "mov r13, [{p} + 88]",
        "mov r12, [{p} + 80]",
        "mov r11, [{p} + 72]",
        "mov r10, [{p} + 64]",
        "mov r9,  [{p} + 56]",
        "mov r8,  [{p} + 48]",
        "mov rbp, [{p} + 40]",
        "mov rdi, [{p} + 32]",
        "mov rsi, [{p} + 24]",
        "mov rdx, [{p} + 16]",
        "mov rcx, [{p} + 8]",
        "mov rbx, [{p}]",
        // {p} is now dead — safe to clobber any register.
        // Preserve PERCPU invariant across ring 3 transition.
        crate::prepare_gs_for_ring3_asm!(),
        "xor eax, eax",             // RAX = 0 (fork child return value)
        "iretq",
        p = in(reg) regs,
        seg = in(reg) 0x23u64,
        options(noreturn)
    );
}

#[cfg(target_arch = "aarch64")]
extern "C" {
    fn arm64_fork_return_to_user(regs: *const ForkChildRegs) -> !;
}

// =========================================================================
// ELF structures
// =========================================================================

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const PT_LOAD: u32 = 1;

/// ELF class constants (EI_CLASS byte at offset 4).
const ELFCLASS32: u8 = 1;
const ELFCLASS64: u8 = 2;

/// ELF64 file header layout (64 bytes, packed to match on-disk format).
#[repr(C, packed)]
struct Elf64Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

/// ELF64 program header layout (56 bytes, packed to match on-disk format).
#[repr(C, packed)]
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

/// Result of loading an ELF: entry point and brk address.
struct ElfLoadResult {
    entry: u64,
    brk: u64,
    /// Total user pages mapped (code + data + BSS segments).
    pages_mapped: u32,
}

/// Load an ELF64 binary into a user PML4.
/// Returns the entry point and the brk (end of last segment, page-aligned).
fn load_elf64(data: &[u8], pd_phys: crate::memory::address::PhysAddr) -> Result<ElfLoadResult, &'static str> {
    if data.len() < 64 {
        return Err("ELF64 file too small");
    }

    let hdr = unsafe { &*(data.as_ptr() as *const Elf64Header) };

    let entry = hdr.e_entry;
    let ph_off = hdr.e_phoff as usize;
    let ph_size = hdr.e_phentsize as usize;
    let ph_num = hdr.e_phnum as usize;



    let mut max_vaddr_end: u64 = 0;
    let mut total_pages: u32 = 0;

    // Iterate program headers and load PT_LOAD segments
    for i in 0..ph_num {
        let ph_offset = ph_off + i * ph_size;
        if ph_offset + ph_size > data.len() {
            return Err("ELF64 program header out of bounds");
        }
        let phdr = unsafe { &*(data.as_ptr().add(ph_offset) as *const Elf64Phdr) };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        let vaddr = phdr.p_vaddr;
        let memsz = phdr.p_memsz;
        let _filesz = phdr.p_filesz;

        if memsz == 0 {
            continue;
        }

        // Validate: vaddr must be in user space (lower canonical half)
        if vaddr >= 0x0000_8000_0000_0000 {
            return Err("ELF64 segment in kernel space");
        }

        // Validate: vaddr must be above the kernel identity-mapped region.
        // The kernel identity-maps the first 128 MiB (0x0 - 0x08000000) of
        // physical memory. User programs link at 0x08000000 (stdlib/link.ld).
        // If a broken binary has segments below this boundary, loading it
        // would write to physical addresses used by kernel data structures
        // (via the identity mapping), causing silent memory corruption and
        // spinlock deadlocks. Reject early with a clear error.
        if vaddr < 0x0800_0000 {
            return Err("ELF64 segment below 128 MiB identity-map boundary");
        }

        // Allocate pages for this segment
        let page_start = vaddr & !0xFFF;
        let seg_total = match vaddr.checked_add(memsz).and_then(|v| v.checked_add(PAGE_SIZE - 1)) {
            Some(v) => v,
            None => return Err("ELF64 segment vaddr+memsz overflow"),
        };
        let page_end = seg_total & !0xFFF;
        let num_pages = (page_end - page_start) / PAGE_SIZE;

        // Derive PTE flags from ELF p_flags:
        //   PF_W set              → data/bss: writable + non-executable
        //   PF_X set, no PF_W    → code: executable, read-only
        //   PF_X | PF_W (rare)   → RWX (e.g. JIT buffers): writable, executable
        let seg_flags = phdr.p_flags;
        let is_writable = (seg_flags & PF_W) != 0;
        let is_exec     = (seg_flags & PF_X) != 0;
        let pte_flags: u64 = PAGE_USER
            | if is_writable { PAGE_WRITABLE } else { 0 }
            | if !is_exec { virtual_mem::page_nx_flag() } else { 0 };

        for p in 0..num_pages {
            let page_virt = VirtAddr::new(page_start + p * PAGE_SIZE);
            if !virtual_mem::is_mapped_in_pd(pd_phys, page_virt) {
                let phys = physical::alloc_frame()
                    .ok_or("Failed to allocate frame for ELF64 segment")?;
                if !virtual_mem::map_page_in_pd(pd_phys, page_virt, phys, pte_flags) {
                    physical::free_frame(phys);
                    return Err("Failed to map frame for ELF64 segment");
                }
                total_pages += 1;
            }
        }

        let seg_end = match vaddr.checked_add(memsz) {
            Some(v) => v,
            None => return Err("ELF64 segment vaddr+memsz overflow"),
        };
        if seg_end > max_vaddr_end {
            max_vaddr_end = seg_end;
        }
    }

    // Switch to user PD and copy data (interrupts disabled to prevent
    // timer-driven context switch while the page table points at the target PD).
    // Save/restore interrupt state instead of unconditional cli/sti to avoid
    // re-enabling interrupts when caller already had them disabled.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        let saved_flags: u64;
        #[cfg(target_arch = "x86_64")]
        {
            core::arch::asm!("pushfq; pop {}", out(reg) saved_flags, options(nomem));
            core::arch::asm!("cli", options(nomem, nostack));
        }
        #[cfg(target_arch = "aarch64")]
        let saved_daif: u64;
        #[cfg(target_arch = "aarch64")]
        {
            core::arch::asm!("mrs {}, daif", out(reg) saved_daif, options(nomem, nostack));
            core::arch::asm!("msr daifset, #0xf", options(nomem, nostack));
        }
        let old_pt = virtual_mem::current_cr3();
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());
        #[cfg(target_arch = "aarch64")]
        {
            core::arch::asm!("msr ttbr0_el1, {}", in(reg) pd_phys.as_u64(), options(nomem, nostack));
            core::arch::asm!("isb", options(nomem, nostack));
        }

        for i in 0..ph_num {
            let ph_offset = ph_off + i * ph_size;
            let phdr = &*(data.as_ptr().add(ph_offset) as *const Elf64Phdr);

            if phdr.p_type != PT_LOAD || phdr.p_memsz == 0 {
                continue;
            }

            let vaddr = phdr.p_vaddr;
            let filesz = phdr.p_filesz as usize;
            let memsz = phdr.p_memsz as usize;
            let offset = phdr.p_offset as usize;

            // Zero all allocated pages first
            let page_start = (vaddr & !0xFFF) as usize;
            let page_end = match (vaddr as usize).checked_add(memsz).and_then(|v| v.checked_add(0xFFF)) {
                Some(v) => v & !0xFFF,
                None => continue,
            };
            core::ptr::write_bytes(page_start as *mut u8, 0, page_end - page_start);

            // Copy file data over the zeroed pages
            if filesz > 0 && offset.checked_add(filesz).map_or(false, |end| end <= data.len()) {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(offset),
                    vaddr as *mut u8,
                    filesz,
                );
            }

            #[cfg(target_arch = "aarch64")]
            if (phdr.p_flags & PF_X) != 0 {
                sync_user_text_range_for_exec(page_start as u64, page_end - page_start);
            }
        }

        #[cfg(target_arch = "x86_64")]
        {
            core::arch::asm!("mov cr3, {}", in(reg) old_pt);
            core::arch::asm!("push {}; popfq", in(reg) saved_flags, options(nomem));
        }
        #[cfg(target_arch = "aarch64")]
        {
            core::arch::asm!("msr ttbr0_el1, {}", in(reg) old_pt, options(nomem, nostack));
            core::arch::asm!("isb", options(nomem, nostack));
            core::arch::asm!("msr daif, {}", in(reg) saved_daif, options(nomem, nostack));
        }
    }

    let brk = (max_vaddr_end + PAGE_SIZE - 1) & !0xFFF;

    // Validate entry point: must be above identity-map boundary
    if entry < 0x0800_0000 || entry >= 0x0000_8000_0000_0000 {
        return Err("ELF64 entry point outside valid user address range");
    }

    Ok(ElfLoadResult { entry, brk, pages_mapped: total_pages })
}


/// Check if data starts with ELF magic bytes.
fn is_elf(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == ELF_MAGIC
}

/// Return the ELF class (1=ELF32, 2=ELF64) or 0 if not an ELF.
fn elf_class(data: &[u8]) -> u8 {
    if data.len() >= 5 && data[0..4] == ELF_MAGIC {
        data[4]
    } else {
        0
    }
}

// =========================================================================
// Shared binary loading (used by both spawn and exec)
// =========================================================================

/// Result of loading a binary into a page directory.
pub struct LoadResult {
    pub entry: u64,
    pub brk: u64,
    pub user_pages: u32,
    /// Initial user stack pointer (ASLR-randomized, ABI-aligned: `stack_top % 16 == 8`).
    pub stack_top: u64,
}

/// Load a binary (ELF64/flat) into an already-created page directory.
/// Maps code segments + user stack. Returns entry point, brk, page count.
pub fn load_binary_into_pd(
    data: &[u8],
    pd_phys: crate::memory::address::PhysAddr,
) -> Result<LoadResult, &'static str> {
    if data.is_empty() {
        return Err("Program data is empty");
    }

    let mut total_user_pages: u32 = 0;
    // ASLR: randomize the stack top within the 8 MiB region.
    // The offset is subtracted from USER_STACK_TOP so the stack starts a
    // random number of pages below the fixed top.  The full 8 MiB is still
    // allocated, so the random gap is simply unused guard space above.
    let stack_aslr_offset = random_page_offset(ASLR_STACK_MAX_PAGES) as u64 * PAGE_SIZE;
    let aslr_stack_top = USER_STACK_TOP - stack_aslr_offset;
    let stack_bottom = aslr_stack_top - USER_STACK_PAGES * PAGE_SIZE;
    // Stack is data — writable but never executed.
    let stack_flags = PAGE_WRITABLE | PAGE_USER | virtual_mem::page_nx_flag();

    #[cfg(target_arch = "x86_64")]
    {
        // Map a signal-return trampoline page (USER | EXECUTABLE, no NX).
        // Contains `mov eax, SYS_SIGRETURN; int 0x80; nop` so signal handlers
        // can return without executing code on the NX-protected stack.
        let tramp_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(SIGRETURN_TRAMPOLINE_ADDR),
            1,
            PAGE_USER, // readable + executable (no PAGE_WRITABLE, no NX)
            true,
        )?;
        // Write the trampoline code into the page (switch to new PD temporarily)
        unsafe {
            let saved_flags: u64;
            core::arch::asm!("pushfq; pop {}", out(reg) saved_flags, options(nomem));
            core::arch::asm!("cli", options(nomem, nostack));
            let old_pt = virtual_mem::current_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());

            let tramp = SIGRETURN_TRAMPOLINE_ADDR as *mut u8;
            // mov eax, 246 (SYS_SIGRETURN)
            tramp.offset(0).write_volatile(0xB8);
            tramp.offset(1).write_volatile(246); // SYS_SIGRETURN
            tramp.offset(2).write_volatile(0x00);
            tramp.offset(3).write_volatile(0x00);
            tramp.offset(4).write_volatile(0x00);
            // syscall  (2 bytes — same length as the old `int 0x80` opcode)
            tramp.offset(5).write_volatile(0x0F);
            tramp.offset(6).write_volatile(0x05);
            // nop (padding)
            tramp.offset(7).write_volatile(0x90);

            core::arch::asm!("mov cr3, {}", in(reg) old_pt);
            core::arch::asm!("push {}; popfq", in(reg) saved_flags, options(nomem));
        }
        total_user_pages += tramp_mapped;
    }

    // Guard page: leave the bottom-most page of the stack region UNMAPPED.
    // If user code overflows the stack, it touches this unmapped page and
    // triggers a page fault → the kernel kills the thread with SIGSEGV
    // instead of corrupting memory or crashing the kernel.
    let guard_pages: u64 = 1;
    let stack_guard_bottom = stack_bottom; // unmapped guard page
    let stack_usable_bottom = stack_bottom + guard_pages * PAGE_SIZE;
    let stack_usable_pages = USER_STACK_PAGES - guard_pages;

    let class = elf_class(data);
    if class == ELFCLASS64 {
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_usable_bottom),
            stack_usable_pages,
            stack_flags,
            true,
        )?;
        let elf_result = load_elf64(data, pd_phys)?;
        total_user_pages += elf_result.pages_mapped + stack_mapped;
        Ok(LoadResult {
            entry: elf_result.entry,
            brk: elf_result.brk,
            user_pages: total_user_pages,
            // x86-64 ABI: RSP % 16 == 8 at function entry (simulates `call` push).
            stack_top: aslr_stack_top - 8,
        })
    } else if class == ELFCLASS32 {
        Err("ELF32 binaries are no longer supported (32-bit user space removed)")
    } else if is_elf(data) {
        Err("Unknown ELF class (only ELF64 is supported)")
    } else {
        // Flat binary: no ELF headers so we cannot know which sections are
        // code vs. data.  Map everything RWX for backwards compatibility.
        let code_pages = (data.len() as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
        let code_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(PROGRAM_LOAD_ADDR),
            code_pages,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            stack_flags,
            true,
        )?;

        // Copy binary data into the new address space
        unsafe {
            #[cfg(target_arch = "x86_64")]
            let saved_flags: u64;
            #[cfg(target_arch = "x86_64")]
            {
                core::arch::asm!("pushfq; pop {}", out(reg) saved_flags, options(nomem));
                core::arch::asm!("cli", options(nomem, nostack));
            }
            #[cfg(target_arch = "aarch64")]
            let saved_daif: u64;
            #[cfg(target_arch = "aarch64")]
            {
                core::arch::asm!("mrs {}, daif", out(reg) saved_daif, options(nomem, nostack));
                core::arch::asm!("msr daifset, #0xf", options(nomem, nostack));
            }
            let old_pt = virtual_mem::current_cr3();
            #[cfg(target_arch = "x86_64")]
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());
            #[cfg(target_arch = "aarch64")]
            {
                core::arch::asm!("msr ttbr0_el1, {}", in(reg) pd_phys.as_u64(), options(nomem, nostack));
                core::arch::asm!("isb", options(nomem, nostack));
            }

            let dest = PROGRAM_LOAD_ADDR as *mut u8;
            core::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());

            #[cfg(target_arch = "aarch64")]
            sync_user_text_range_for_exec(PROGRAM_LOAD_ADDR, data.len());

            #[cfg(target_arch = "x86_64")]
            {
                core::arch::asm!("mov cr3, {}", in(reg) old_pt);
                core::arch::asm!("push {}; popfq", in(reg) saved_flags, options(nomem));
            }
            #[cfg(target_arch = "aarch64")]
            {
                core::arch::asm!("msr ttbr0_el1, {}", in(reg) old_pt, options(nomem, nostack));
                core::arch::asm!("isb", options(nomem, nostack));
                core::arch::asm!("msr daif, {}", in(reg) saved_daif, options(nomem, nostack));
            }
        }

        total_user_pages += code_mapped + stack_mapped;
        Ok(LoadResult {
            entry: PROGRAM_LOAD_ADDR,
            brk: PROGRAM_LOAD_ADDR + code_pages * PAGE_SIZE,
            user_pages: total_user_pages,
            stack_top: aslr_stack_top - 8,
        })
    }
}

// =========================================================================
// exec() — replace current process image
// =========================================================================

/// Replace the current process with a new binary loaded from `data`.
/// On success, never returns (jumps to user mode in new address space).
/// On failure, returns an error string and the old process continues.
pub fn exec_current_process(data: &[u8], args: &str) -> &'static str {
    let tid = crate::task::scheduler::current_tid();

    // Get old PD before we replace it
    let old_pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd,
        None => return "exec: no page directory on current thread",
    };

    // Create fresh page directory
    let new_pd = match virtual_mem::create_user_page_directory() {
        Some(pd) => pd,
        None => return "exec: failed to create page directory (OOM)",
    };

    // Load binary into new PD
    let result = match load_binary_into_pd(data, new_pd) {
        Ok(r) => r,
        Err(e) => {
            virtual_mem::destroy_user_page_directory(new_pd);
            return e;
        }
    };

    // Map DLLs into new address space
    crate::task::dll::map_all_dlls_into(new_pd);

    // All user threads now run in 64-bit native mode.
    let arch_mode = crate::task::thread::ArchMode::Native64;

    // Update thread metadata (PD, brk, arch_mode, FPU reset, mmap reset)
    crate::task::scheduler::exec_update_thread(
        tid, new_pd, result.brk as u32, arch_mode, result.user_pages,
    );

    // Set new args (clear old args first)
    crate::task::scheduler::set_thread_args(tid, args);

    // Rekey environment from old PD to new PD (move entries in-place)
    crate::task::env::rekey_env(old_pd.0, new_pd.0);

    // Switch page table to new address space and destroy old one
    unsafe {
        #[cfg(target_arch = "x86_64")]
        {
            core::arch::asm!("cli", options(nomem, nostack));
            core::arch::asm!("mov cr3, {}", in(reg) new_pd.as_u64());
        }
        #[cfg(target_arch = "aarch64")]
        {
            core::arch::asm!("msr daifset, #0xf", options(nomem, nostack));
            core::arch::asm!("msr ttbr0_el1, {}", in(reg) new_pd.as_u64(), options(nomem, nostack));
            core::arch::asm!("isb", options(nomem, nostack));
        }
    }

    // Destroy old PD (safe: we're now running on new PD, kernel pages are shared)
    virtual_mem::destroy_user_page_directory(old_pd);

    // Re-enable interrupts and jump to user mode (never returns).
    // stack_top already includes ABI alignment (-8) and ASLR offset.
    let user_stack = result.stack_top;

    crate::serial_verbose_println!("exec: T{} -> (elf64, {} pages, entry={:#x})",
        tid, result.user_pages, result.entry);

    #[cfg(target_arch = "x86_64")]
    unsafe { jump_to_user_mode(result.entry, user_stack); }
    #[cfg(target_arch = "aarch64")]
    unsafe { jump_to_user_mode(result.entry, user_stack, 0); }
}

/// Load a flat binary from the filesystem and run it in Ring 3.
/// Creates a per-process PML4 with isolated user-space mappings.
/// Returns the TID of the spawned thread.
pub fn load_and_run(path: &str, name: &str) -> Result<u32, &'static str> {
    load_and_run_with_args(path, name, "")
}

/// Load a flat binary or ELF and run it with command-line arguments.
/// If `path` ends with `.app`, it is treated as a bundle directory:
/// the binary is resolved from Info.conf `exec=` field, or derived from the folder name.
/// The exec binary MUST reside directly inside the .app directory (no subdirectories).
pub fn load_and_run_with_args(path: &str, name: &str, args: &str) -> Result<u32, &'static str> {
    // .app bundle resolution
    let resolved_path: alloc::string::String;
    let bundle_cwd: Option<alloc::string::String>;
    let bundle_caps: Option<crate::task::capabilities::CapSet>;
    let bundle_app_id: Option<alloc::string::String>;
    let actual_path = if path.ends_with(".app") {
        // Parse Info.conf for exec field and working_dir
        let config = crate::task::app_config::parse_info_conf(path);

        // Determine binary name: prefer Info.conf exec=, fallback to folder name
        let binary_name: alloc::string::String = if let Some(ref cfg) = config {
            if let Some(ref exec) = cfg.exec {
                // SECURITY: exec must be a plain filename — no '/' or '..' allowed.
                // The binary MUST reside directly inside the .app directory.
                if exec.contains('/') || exec.contains("..") {
                    return Err(".app exec must be a plain filename (no path separators)");
                }
                alloc::string::String::from(exec.as_str())
            } else {
                // Fallback: derive from folder name minus ".app"
                let folder_name = path.rsplit('/').next().unwrap_or(path);
                alloc::string::String::from(&folder_name[..folder_name.len() - 4])
            }
        } else {
            let folder_name = path.rsplit('/').next().unwrap_or(path);
            alloc::string::String::from(&folder_name[..folder_name.len() - 4])
        };

        if binary_name.is_empty() {
            return Err("Invalid .app bundle: empty exec name");
        }

        resolved_path = alloc::format!("{}/{}", path, binary_name);

        // Determine CWD from working_dir field (default: bundle directory)
        bundle_cwd = if let Some(ref cfg) = config {
            match cfg.working_dir.as_deref() {
                Some("home") => Some(alloc::string::String::from("/")),
                Some(explicit) if explicit != "bundle" => Some(alloc::string::String::from(explicit)),
                _ => Some(alloc::string::String::from(path)), // "bundle" or unset
            }
        } else {
            Some(alloc::string::String::from(path))
        };

        // Extract capabilities from Info.conf
        bundle_caps = if let Some(ref cfg) = config {
            if let Some(ref cap_str) = cfg.capabilities {
                Some(crate::task::capabilities::parse_capabilities(cap_str))
            } else {
                Some(crate::task::capabilities::CAP_DEFAULT)
            }
        } else {
            Some(crate::task::capabilities::CAP_DEFAULT)
        };

        // Extract app_id for permission lookup (id field from Info.conf, or folder name)
        bundle_app_id = if let Some(ref cfg) = config {
            if let Some(ref id) = cfg.id {
                Some(id.clone())
            } else {
                Some(alloc::string::String::from(name))
            }
        } else {
            Some(alloc::string::String::from(name))
        };

        resolved_path.as_str()
    } else {
        bundle_cwd = None;
        bundle_caps = None;
        bundle_app_id = None;
        path
    };

    // Permission check: caller must have read permission on the binary
    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(actual_path) {
        if !crate::fs::permissions::check_permission(uid, gid, mode, crate::fs::permissions::PERM_READ) {
            return Err("Permission denied");
        }
    }

    // Read the binary from the filesystem
    let data = match crate::fs::vfs::read_file_to_vec(actual_path) {
        Ok(d) => d,
        Err(e) => {
            crate::serial_verbose_println!("  load_and_run: read_file_to_vec('{}') failed: {:?}", actual_path, e);
            return Err("Failed to read program file");
        }
    };

    if data.is_empty() {
        return Err("Program file is empty");
    }

    // Create per-process PML4 (clones kernel mappings, empty user space)
    let pd_phys = virtual_mem::create_user_page_directory()
        .ok_or("Failed to create user page directory")?;

    let (entry_point, brk);
    let mut total_user_pages: u32 = 0;

    // ASLR: randomize the stack top within the 8 MiB region.
    let stack_aslr_offset = random_page_offset(ASLR_STACK_MAX_PAGES) as u64 * PAGE_SIZE;
    let aslr_stack_top = USER_STACK_TOP - stack_aslr_offset;
    // Stack is data — writable but never executed.
    let stack_flags = PAGE_WRITABLE | PAGE_USER | virtual_mem::page_nx_flag();

    #[cfg(target_arch = "x86_64")]
    {
        // Map signal-return trampoline page (same as load_binary_into_pd).
        let tramp_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(SIGRETURN_TRAMPOLINE_ADDR),
            1,
            PAGE_USER,
            true,
        )?;
        unsafe {
            let saved_flags_t: u64;
            core::arch::asm!("pushfq; pop {}", out(reg) saved_flags_t, options(nomem));
            core::arch::asm!("cli", options(nomem, nostack));
            let old_pt_t = virtual_mem::current_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());
            let tramp = SIGRETURN_TRAMPOLINE_ADDR as *mut u8;
            // mov eax, 246 (SYS_SIGRETURN)
            tramp.offset(0).write_volatile(0xB8);
            tramp.offset(1).write_volatile(246);
            tramp.offset(2).write_volatile(0x00);
            tramp.offset(3).write_volatile(0x00);
            tramp.offset(4).write_volatile(0x00);
            // syscall
            tramp.offset(5).write_volatile(0x0F);
            tramp.offset(6).write_volatile(0x05);
            // nop (padding)
            tramp.offset(7).write_volatile(0x90);
            core::arch::asm!("mov cr3, {}", in(reg) old_pt_t);
            core::arch::asm!("push {}; popfq", in(reg) saved_flags_t, options(nomem));
        }
        total_user_pages += tramp_mapped;
    }

    let class = elf_class(&data);
    if class == ELFCLASS64 {
        // ---- ELF64 binary path ----

        // Allocate, map, and zero stack pages (single CR3 switch)
        let stack_bottom = aslr_stack_top - USER_STACK_PAGES * PAGE_SIZE;
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            stack_flags,
            true,
        )?;

        // Pre-map DLIB shared RO pages into the new address space.
        // Without this, every first DLIB access triggers a page fault inside
        // the kernel's demand-page handler (which holds LOADED_DLLS + ALLOCATOR
        // locks). Pre-mapping avoids that fragile nested-lock path for RO code.
        // Per-process .data/.bss pages are still demand-paged on first access.
        crate::task::dll::map_all_dlls_into(pd_phys);

        // Load ELF64 segments
        let elf_result = load_elf64(&data, pd_phys)?;
        entry_point = elf_result.entry;
        brk = elf_result.brk;
        total_user_pages += elf_result.pages_mapped + stack_mapped;

    } else if class == ELFCLASS32 {
        return Err("ELF32 binaries are no longer supported (32-bit user space removed)");
    } else if is_elf(&data) {
        return Err("Unknown ELF class (only ELF64 is supported)");
    } else {
        // ---- Flat binary path (no ELF headers → map code RWX for compat) ----
        let code_pages = (data.len() as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
        let code_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(PROGRAM_LOAD_ADDR),
            code_pages,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;

        let stack_bottom = aslr_stack_top - USER_STACK_PAGES * PAGE_SIZE;
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            stack_flags,
            true,
        )?;

        // Pre-map DLIB shared RO pages (same as ELF64/ELF32 paths above)
        crate::task::dll::map_all_dlls_into(pd_phys);

        // Copy binary data (pages already zeroed by map_pages_range_in_pd)
        unsafe {
            #[cfg(target_arch = "x86_64")]
            let saved_flags: u64;
            #[cfg(target_arch = "x86_64")]
            {
                core::arch::asm!("pushfq; pop {}", out(reg) saved_flags, options(nomem));
                core::arch::asm!("cli", options(nomem, nostack));
            }
            #[cfg(target_arch = "aarch64")]
            let saved_daif: u64;
            #[cfg(target_arch = "aarch64")]
            {
                core::arch::asm!("mrs {}, daif", out(reg) saved_daif, options(nomem, nostack));
                core::arch::asm!("msr daifset, #0xf", options(nomem, nostack));
            }
            let old_pt = virtual_mem::current_cr3();
            #[cfg(target_arch = "x86_64")]
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());
            #[cfg(target_arch = "aarch64")]
            {
                core::arch::asm!("msr ttbr0_el1, {}", in(reg) pd_phys.as_u64(), options(nomem, nostack));
                core::arch::asm!("isb", options(nomem, nostack));
            }

            let dest = PROGRAM_LOAD_ADDR as *mut u8;
            core::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());

            #[cfg(target_arch = "aarch64")]
            sync_user_text_range_for_exec(PROGRAM_LOAD_ADDR, data.len());

            #[cfg(target_arch = "x86_64")]
            {
                core::arch::asm!("mov cr3, {}", in(reg) old_pt);
                core::arch::asm!("push {}; popfq", in(reg) saved_flags, options(nomem));
            }
            #[cfg(target_arch = "aarch64")]
            {
                core::arch::asm!("msr ttbr0_el1, {}", in(reg) old_pt, options(nomem, nostack));
                core::arch::asm!("isb", options(nomem, nostack));
                core::arch::asm!("msr daif, {}", in(reg) saved_daif, options(nomem, nostack));
            }
        }

        entry_point = PROGRAM_LOAD_ADDR;
        brk = PROGRAM_LOAD_ADDR + code_pages * PAGE_SIZE;
        total_user_pages += code_mapped + stack_mapped;

    }

    // Spawn in Blocked state — the thread cannot be picked up by any CPU
    // (including APs) until we explicitly wake it.  This prevents the SMP race
    // where an AP runs the trampoline before we store pending-program data.
    let tid = crate::task::scheduler::spawn_blocked(user_thread_trampoline, 100, name);
    // ASLR: randomize mmap base for each new process so mmap allocations
    // land at a different address than the previous run.
    let mmap_rand = random_page_offset(ASLR_MMAP_MAX_PAGES);
    let mmap_start = 0x7000_0000u32.wrapping_add(mmap_rand * 4096);
    crate::task::scheduler::set_thread_mmap_next(tid, mmap_start);
    // Initialize VMA table for this process (gap-finding allocator).
    crate::memory::vma::init_process(pd_phys, mmap_start);
    crate::task::scheduler::set_thread_user_info(tid, pd_phys, brk as u32);
    if total_user_pages > 0 {
        crate::task::scheduler::adjust_thread_user_pages(tid, total_user_pages as i32);
    }

    // Store pending program info keyed by TID (after spawn so we know the TID).
    // x86_64 enters userspace via `iretq` into a call-like ABI state and
    // therefore wants RSP % 16 == 8. AArch64 enters EL0 via `eret` and
    // requires SP to remain 16-byte aligned.
    #[cfg(target_arch = "x86_64")]
    let pending_user_stack = aslr_stack_top - 8;
    #[cfg(target_arch = "aarch64")]
    let pending_user_stack = aslr_stack_top;

    if !try_store_pending_program(tid, entry_point, pending_user_stack, 0) {
        crate::serial_println!(
            "load_and_run: pending-program table full for '{}' (tid={})",
            path,
            tid
        );
        crate::task::scheduler::kill_thread(tid);
        return Err("Too many pending programs");
    }
    if !args.is_empty() {
        crate::task::scheduler::set_thread_args(tid, args);
    }

    // Set CWD for .app bundle processes
    if let Some(ref cwd) = bundle_cwd {
        crate::task::scheduler::set_thread_cwd(tid, cwd);
    }

    // Set capabilities: .app bundles use Info.conf intersected with stored permissions,
    // non-.app binaries inherit parent's caps.
    // The permission boundary is at the .app bundle level — CLI tools and system services
    // inherit whatever their parent has (compositor children get CAP_ALL, etc.).
    let caps = if let Some(declared) = bundle_caps {
        use crate::task::capabilities::*;
        if declared == CAP_ALL {
            // System app (capabilities=all) — full access, no permission restriction
            CAP_ALL
        } else if crate::fs::vfs::root_is_iso9660() {
            // Live-CD boot: root is read-only ISO 9660, so permission files
            // cannot be stored.  Grant all declared capabilities directly.
            declared
        } else if crate::task::scheduler::current_thread_capabilities()
                    == crate::task::capabilities::CAP_ALL {
            // Parent is fully privileged (compositor, sessionhost, etc.) —
            // grant all declared caps directly without requiring stored
            // permissions.  The trust boundary is the parent process.
            declared
        } else {
            // Intersect declared caps with stored user permissions:
            // - auto-granted caps (DLL, THREAD, SHM, EVENT, PIPE) always apply
            // - sensitive caps only if the user granted them
            let auto = CAP_AUTO_GRANTED;
            let uid = crate::task::scheduler::current_thread_uid();
            let app_id = bundle_app_id.as_deref().unwrap_or(name);
            let granted_sensitive = crate::task::permissions::read_stored_perms(uid, app_id)
                .unwrap_or(0);
            auto | (declared & granted_sensitive)
        }
    } else {
        let parent_caps = crate::task::scheduler::current_thread_capabilities();
        if parent_caps == 0 {
            // Kernel thread spawning user process (e.g. compositor at boot) — full access
            crate::task::capabilities::CAP_ALL
        } else if actual_path == "/System/permdialog" {
            // Kernel allowlist: PermissionDialog needs MANAGE_PERMS + FILESYSTEM
            // regardless of parent's caps, so it can write permission files.
            parent_caps | crate::task::capabilities::CAP_MANAGE_PERMS
                        | crate::task::capabilities::CAP_FILESYSTEM
        } else {
            // Non-.app binary: inherit parent's full capabilities
            parent_caps
        }
    };
    crate::task::scheduler::set_thread_capabilities(tid, caps);

    // Inherit uid/gid from parent thread (processes start with same identity)
    let (parent_uid, parent_gid) = {
        let uid = crate::task::scheduler::current_thread_uid();
        let gid = crate::task::scheduler::current_thread_gid();
        (uid, gid)
    };
    crate::task::scheduler::set_thread_identity(tid, parent_uid, parent_gid);

    let fmt = if is_elf(&data) { "elf64" } else { "flat" };
    crate::serial_verbose_println!("spawn: '{}' -> T{} ({}, {} pages, entry={:#x})",
        path, tid, fmt, total_user_pages, entry_point);

    // All setup complete (CR3, pending data, args, CWD, caps). Now make the thread runnable.
    crate::task::scheduler::wake_thread(tid);

    Ok(tid)
}

/// Trampoline: runs as a kernel thread, then transitions to user mode.
/// At this point, context_switch.asm has already loaded our CR3 (user PD).
pub(crate) extern "C" fn user_thread_trampoline() {
    enable_irqs_before_user_entry();
    #[cfg(target_arch = "x86_64")]
    crate::serial_verbose_println!("  [TRAMPOLINE] entered, tid={}", crate::task::scheduler::current_tid());
    let tid = crate::task::scheduler::current_tid();
    let (entry, user_stack, user_lr) =
        take_pending_program(tid, "trampoline");

    #[cfg(target_arch = "x86_64")]
    crate::serial_verbose_println!("  [TRAMPOLINE] tid={} entry={:#x} stack={:#x}",
        tid, entry, user_stack);
    let _ = user_lr;
    #[cfg(target_arch = "x86_64")]
    unsafe { jump_to_user_mode(entry, user_stack); }
    #[cfg(target_arch = "aarch64")]
    unsafe { jump_to_user_mode(entry, user_stack, user_lr); }
}

/// Store a pending entry point and user stack for a new intra-process thread.
/// Called by `scheduler::create_thread_in_current_process()`.
pub fn store_pending_thread(tid: u32, entry: u64, user_stack: u64, user_lr: u64) -> bool {
    try_store_pending_program(tid, entry, user_stack, user_lr)
}

/// Trampoline for intra-process threads created via SYS_THREAD_CREATE.
/// Identical to `user_thread_trampoline` — looks up the pending slot and jumps to user mode.
pub extern "C" fn thread_create_trampoline() {
    enable_irqs_before_user_entry();
    let tid = crate::task::scheduler::current_tid();
    let (entry, user_stack, user_lr) =
        take_pending_program(tid, "thread_create trampoline");

    let _ = user_lr;
    #[cfg(target_arch = "x86_64")]
    unsafe { jump_to_user_mode(entry, user_stack); }
    #[cfg(target_arch = "aarch64")]
    unsafe { jump_to_user_mode(entry, user_stack, user_lr); }
}

/// Transition to Ring 3 (user mode) for 64-bit programs.
///
/// On x86_64: builds an `iretq` frame with user CS=0x2B / SS=0x23 and jumps.
/// On AArch64: sets ELR_EL1/SP_EL0/SPSR_EL1 and issues `eret` to EL0.
#[cfg(target_arch = "x86_64")]
unsafe fn jump_to_user_mode(entry: u64, user_stack: u64) -> ! {
    crate::arch::x86::syscall_msr::debug_assert_gs_is_kernel();
    // Use explicit R14/R15 to avoid `mov ax, 0x23` clobbering an in(reg) operand
    // (MEMORY.md: hardcoded AX in asm! corrupts any in(reg) that the compiler
    //  allocates to RAX — and `pop rax` would clobber it too)
    core::arch::asm!(
        "cli",
        // Set data segment registers to user data segment
        "mov ax, 0x23",
        "mov ds, ax",
        "mov es, ax",
        "mov fs, ax",
        "mov gs, ax",
        // Build iretq frame on the kernel stack:
        //   SS, RSP, RFLAGS, CS, RIP
        "push 0x23",       // SS = user data segment
        "push r14",        // RSP = user stack pointer
        "pushfq",          // RFLAGS
        "pop rax",
        "or rax, 0x200",   // Set IF (interrupts enabled)
        "push rax",
        "push 0x2B",       // CS = user code 64-bit segment
        "push r15",        // RIP = program entry point
        // Preserve the PERCPU invariant across the ring 3 transition:
        // KERNEL_GS_BASE ← PERCPU, GS.base ← 0. See CLAUDE.md.
        crate::prepare_gs_for_ring3_asm!(),
        // Clear all GPRs to prevent kernel address leaks to user mode
        // (critical for exec: INT 0x80 frame leaves kernel values in regs)
        "xor eax, eax",
        "xor ebx, ebx",
        "xor ecx, ecx",
        "xor edx, edx",
        "xor esi, esi",
        "xor edi, edi",
        "xor ebp, ebp",
        "xor r8d, r8d",
        "xor r9d, r9d",
        "xor r10d, r10d",
        "xor r11d, r11d",
        "xor r12d, r12d",
        "xor r13d, r13d",
        "xor r14d, r14d",
        "xor r15d, r15d",
        "iretq",           // Enter Ring 3!
        in("r14") user_stack,
        in("r15") entry,
        options(noreturn)
    );
}

#[cfg(target_arch = "x86_64")]
#[inline(never)]
fn enable_irqs_before_user_entry() {
    crate::arch::hal::enable_interrupts();
}

#[cfg(target_arch = "aarch64")]
#[inline(never)]
fn enable_irqs_before_user_entry() {}

/// Transition to EL0 (user mode) for 64-bit AArch64 programs.
///
/// Sets ELR_EL1 to the entry point, SP_EL0 to the user stack, and
/// SPSR_EL1 to 0x0 (EL0t with all exceptions unmasked). Then clears
/// all general-purpose registers to prevent kernel address leaks and
/// issues `eret`.
#[cfg(target_arch = "aarch64")]
unsafe fn jump_to_user_mode(entry: u64, user_stack: u64, user_lr: u64) -> ! {
    core::arch::asm!(
        // Set the return address (ELR_EL1) and user stack (SP_EL0)
        "msr elr_el1, {entry}",
        "msr sp_el0, {sp}",
        // SPSR_EL1 = 0x0: EL0t (AArch64, EL0, SP_EL0), all DAIF unmasked
        "msr spsr_el1, xzr",
        // Ensure the updated exception return state is visible to `eret`.
        "isb",
        // Clear all general-purpose registers to prevent kernel leaks
        "mov x0, #0",
        "mov x1, #0",
        "mov x2, #0",
        "mov x3, #0",
        "mov x4, #0",
        "mov x5, #0",
        "mov x6, #0",
        "mov x7, #0",
        "mov x8, #0",
        "mov x9, #0",
        "mov x10, #0",
        "mov x11, #0",
        "mov x12, #0",
        "mov x13, #0",
        "mov x14, #0",
        "mov x15, #0",
        "mov x16, #0",
        "mov x17, #0",
        "mov x18, #0",
        "mov x19, #0",
        "mov x20, #0",
        "mov x21, #0",
        "mov x22, #0",
        "mov x23, #0",
        "mov x24, #0",
        "mov x25, #0",
        "mov x26, #0",
        "mov x27, #0",
        "mov x28, #0",
        "mov x29, #0",
        "mov x30, {lr}",
        "eret",
        entry = in(reg) entry,
        sp = in(reg) user_stack,
        lr = in(reg) user_lr,
        options(noreturn)
    );
}

