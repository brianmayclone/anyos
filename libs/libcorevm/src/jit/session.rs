//! JIT session with emitted dispatcher trampoline.
//!
//! The dispatcher is a native x86-64 loop that:
//! 1. Checks stop conditions (stop_requested, instruction limit, faults, SMC, interrupts)
//! 2. Computes the guest linear address from CS:RIP
//! 3. Translates it to physical via `jit_mmu_translate`
//! 4. Looks up the physical address in a flat hashtable
//! 5. On hit: jumps to the compiled block
//! 6. On miss: stores compile info and exits

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::jit::emitter::{Cc, Emitter, OpSize, Reg};
use crate::jit::executable_mem::JitBuffer;
use crate::jit::helpers;
use crate::jit::lookup_table::{
    JitLookupTable, ENTRY_CODE_OFF, ENTRY_MODE_CS_OFF, ENTRY_PHYS_OFF, ENTRY_SIZE,
};
use crate::jit::translator::Translator;

// ── Exit reason constants ────────────────────────────────────────────────

pub const EXIT_NEEDS_COMPILE: u32 = 1;
pub const EXIT_INTERRUPT: u32 = 2;
pub const EXIT_LIMIT: u32 = 3;
pub const EXIT_STOP: u32 = 4;
pub const EXIT_SMC: u32 = 5;
pub const EXIT_HALT: u32 = 6;
pub const EXIT_FAULT: u32 = 7;

// ── Dispatcher function type ─────────────────────────────────────────────

/// Signature of the emitted dispatcher trampoline.
///
/// Arguments: cpu, memory, mmu, io, interrupts, target_instruction_count.
/// Returns an exit reason constant.
pub type DispatcherFn = unsafe extern "C" fn(
    *mut u8, *mut u8, *mut u8, *mut u8, *mut u8, u64,
) -> u32;

// ── Register convention (persistent across JIT session) ──────────────────

const CPU: Reg = Reg::Rbx;
const MEM: Reg = Reg::R12;
const MMU: Reg = Reg::R13;
const IO: Reg = Reg::R14;
const INTR: Reg = Reg::Rbp;
const GPR_BASE: Reg = Reg::R15;

// ── RegisterFile layout (repr(C)) ────────────────────────────────────────

const RIP_OFF: i32 = 128;
const RFLAGS_OFF: i32 = 136;
const CS_BASE_OFF: i32 = 144 + 1 * 32 + 0; // seg[1].base = 176

// ── JitSession ───────────────────────────────────────────────────────────

pub struct JitSession {
    pub buffer: JitBuffer,
    pub translator: Translator,
    pub lookup: JitLookupTable,
    dispatcher_offset: usize,
    pub dispatch_loop_offset: usize,
    pub code_pages: BTreeSet<u64>,
    enabled: bool,
    pub blocks_compiled: u64,
}

impl JitSession {
    pub fn new() -> Self {
        JitSession {
            buffer: JitBuffer::new(),
            translator: Translator::new(),
            lookup: JitLookupTable::new(),
            dispatcher_offset: 0,
            dispatch_loop_offset: 0,
            code_pages: BTreeSet::new(),
            enabled: false,
            blocks_compiled: 0,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn flush(&mut self) {
        self.lookup.flush();
        self.code_pages.clear();
        self.buffer.reset();
        self.dispatcher_offset = 0;
        self.dispatch_loop_offset = 0;
    }

    pub fn invalidate_page(&mut self, page_phys: u64) {
        self.lookup.invalidate_page(page_phys);
    }

    pub fn blocks_compiled(&self) -> u64 {
        self.blocks_compiled
    }

    pub fn native_count(&self) -> u64 {
        self.translator.native_count
    }

    pub fn fallback_count(&self) -> u64 {
        self.translator.helper_count
    }

    pub fn top_helper_opcodes(&self, n: usize) -> Vec<(u16, u64)> {
        let mut entries: Vec<(u16, u64)> = self
            .translator
            .helper_opcode_counts
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(n);
        entries
    }

    /// Absolute address of the dispatch_loop label in the JIT buffer.
    /// Compiled blocks jump here to re-enter the dispatcher.
    pub fn dispatch_loop_ptr(&self) -> u64 {
        unsafe { self.buffer.code_ptr(self.dispatch_loop_offset) as u64 }
    }

    /// Get the dispatcher as a callable function pointer.
    ///
    /// # Safety
    /// `emit_dispatcher` must have been called and the buffer must be executable.
    pub unsafe fn dispatcher_fn(&self) -> DispatcherFn {
        let ptr = self.buffer.code_ptr(self.dispatcher_offset);
        core::mem::transmute(ptr)
    }

    /// Emit the dispatcher trampoline into the JIT buffer.
    ///
    /// This must be called before any blocks are compiled, since blocks
    /// jump back to the dispatch_loop label.
    pub fn emit_dispatcher(&mut self) {
        // Cpu field offsets (Cpu is NOT repr(C), so use offset_of!)
        let stop_off = core::mem::offset_of!(crate::cpu::Cpu, stop_requested) as i32;
        let inst_count_off = core::mem::offset_of!(crate::cpu::Cpu, instruction_count) as i32;
        let jit_fault_off = core::mem::offset_of!(crate::cpu::Cpu, jit_fault) as i32;
        let smc_off = core::mem::offset_of!(crate::cpu::Cpu, smc_pending) as i32;
        let halted_off = core::mem::offset_of!(crate::cpu::Cpu, jit_halted) as i32;
        let regs_off = core::mem::offset_of!(crate::cpu::Cpu, regs) as i32;
        let a20_off = core::mem::offset_of!(crate::cpu::Cpu, a20_enabled) as i32;
        let pending_phys_off =
            core::mem::offset_of!(crate::cpu::Cpu, pending_compile_phys) as i32;
        let pending_mode_off =
            core::mem::offset_of!(crate::cpu::Cpu, pending_compile_mode) as i32;
        let pending_cs_base_off =
            core::mem::offset_of!(crate::cpu::Cpu, pending_compile_cs_base) as i32;

        // Lookup table constants (embedded as immediates)
        let entries_ptr = self.lookup.entries_ptr() as u64;
        let mask = self.lookup.mask();

        // Helper function addresses
        let mmu_translate_addr = helpers::jit_mmu_translate as *const () as u64;
        let check_interrupt_addr = helpers::jit_check_interrupt as *const () as u64;
        let get_cpu_mode_addr = helpers::jit_get_cpu_mode as *const () as u64;

        let mut e = Emitter::new();

        // Allocate labels
        let dispatch_loop = e.new_label();
        let exit_stop = e.new_label();
        let exit_limit = e.new_label();
        let exit_fault = e.new_label();
        let exit_smc = e.new_label();
        let exit_interrupt = e.new_label();
        let exit_halt = e.new_label();
        let exit_compile = e.new_label();
        let dispatch_exit = e.new_label();
        let no_intr = e.new_label();
        let no_a20_mask = e.new_label();

        // ── 1. Entry: save callee-saved registers ──
        e.push(Reg::Rbx);
        e.push(Reg::R12);
        e.push(Reg::R13);
        e.push(Reg::R14);
        e.push(Reg::R15);
        e.push(Reg::Rbp);
        // sub rsp, 24 for alignment + local storage
        // 6 pushes (48) + return addr (8) = 56 bytes. sub 24 → 80 bytes, RSP % 16 == 0.
        e.sub_ri(OpSize::S64, Reg::Rsp, 24);

        // Save target instruction count at [rsp+0]
        e.mov_mr(OpSize::S64, Reg::Rsp, 0, Reg::R9);

        // Load context registers from arguments
        // rdi=cpu, rsi=mem, rdx=mmu, rcx=io, r8=intr, r9=target (already saved)
        e.mov_rr(OpSize::S64, CPU, Reg::Rdi);       // rbx = cpu
        e.mov_rr(OpSize::S64, MEM, Reg::Rsi);       // r12 = mem
        e.mov_rr(OpSize::S64, MMU, Reg::Rdx);       // r13 = mmu
        e.mov_rr(OpSize::S64, IO, Reg::Rcx);        // r14 = io
        e.mov_rr(OpSize::S64, INTR, Reg::R8);       // rbp = intr

        // r15 = &cpu.regs.gpr[0] = rbx + offset_of!(Cpu, regs) + 0
        e.lea(OpSize::S64, GPR_BASE, CPU, regs_off);

        // ── 2. dispatch_loop ──
        e.bind_label(dispatch_loop);

        // 2a. Check stop_requested
        e.movzx_rm8(OpSize::S32, Reg::Rax, CPU, stop_off);
        e.test_rr(OpSize::S32, Reg::Rax, Reg::Rax);
        e.jcc_label(Cc::Ne, exit_stop);

        // 2b. Check instruction limit
        e.mov_rm(OpSize::S64, Reg::Rax, CPU, inst_count_off);
        e.cmp_rm(OpSize::S64, Reg::Rax, Reg::Rsp, 0);
        e.jcc_label(Cc::Ae, exit_limit);

        // 2c. Check jit_fault
        e.movzx_rm8(OpSize::S32, Reg::Rax, CPU, jit_fault_off);
        e.test_rr(OpSize::S32, Reg::Rax, Reg::Rax);
        e.jcc_label(Cc::Ne, exit_fault);

        // 2d. Check smc_pending
        e.movzx_rm8(OpSize::S32, Reg::Rax, CPU, smc_off);
        e.test_rr(OpSize::S32, Reg::Rax, Reg::Rax);
        e.jcc_label(Cc::Ne, exit_smc);

        // 2d2. Check jit_halted
        e.movzx_rm8(OpSize::S32, Reg::Rax, CPU, halted_off);
        e.test_rr(OpSize::S32, Reg::Rax, Reg::Rax);
        e.jcc_label(Cc::Ne, exit_halt);

        // 2e. Check interrupts: if IF set and interrupt pending, exit
        e.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFF);
        e.test_ri(OpSize::S32, Reg::Rax, 0x200); // IF flag
        e.jcc_label(Cc::E, no_intr);
        // call jit_check_interrupt(rbp=intr, rax=rflags)
        // System V ABI: rdi=intr, rsi=rflags
        e.mov_rr(OpSize::S64, Reg::Rdi, INTR);
        e.mov_rr(OpSize::S64, Reg::Rsi, Reg::Rax);
        e.call_abs(check_interrupt_addr);
        e.test_rr(OpSize::S32, Reg::Rax, Reg::Rax);
        e.jcc_label(Cc::Ne, exit_interrupt);
        e.bind_label(no_intr);

        // 2f. Compute linear address = CS.base + RIP
        e.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, CS_BASE_OFF);
        e.mov_rm(OpSize::S64, Reg::Rcx, GPR_BASE, RIP_OFF);
        e.add_rr(OpSize::S64, Reg::Rax, Reg::Rcx);

        // A20 check: if !a20_enabled, mask bit 20
        e.movzx_rm8(OpSize::S32, Reg::Rdx, CPU, a20_off);
        e.test_rr(OpSize::S32, Reg::Rdx, Reg::Rdx);
        e.jcc_label(Cc::Ne, no_a20_mask);
        // Mask off bit 20: and rax, ~0x10_0000
        e.mov_ri64(Reg::Rdx, !0x10_0000u64);
        e.and_rr(OpSize::S64, Reg::Rax, Reg::Rdx);
        e.bind_label(no_a20_mask);

        // 2g. Call jit_mmu_translate(cpu, mmu, mem, linear_addr)
        // System V: rdi=cpu, rsi=mmu, rdx=mem, rcx=linear
        e.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax); // linear in rcx
        e.mov_rr(OpSize::S64, Reg::Rdi, CPU);
        e.mov_rr(OpSize::S64, Reg::Rsi, MMU);
        e.mov_rr(OpSize::S64, Reg::Rdx, MEM);
        // rcx already has linear
        e.call_abs(mmu_translate_addr);
        // rax = phys_addr or u64::MAX on fault
        e.mov_ri64(Reg::Rcx, u64::MAX);
        e.cmp_rr(OpSize::S64, Reg::Rax, Reg::Rcx);
        e.jcc_label(Cc::E, exit_fault);

        // rax = phys_addr. Save it in r11 for later.
        e.mov_rr(OpSize::S64, Reg::R11, Reg::Rax);

        // 2h. Hashtable lookup — Fibonacci hash
        // hash = ((phys * 0x9E3779B97F4A7C15) >> 48) & mask
        // Use mul: mov rcx, golden; mul rcx → rdx:rax, then shr rdx:rax
        // But mul clobbers rax/rdx. rax = phys (need to keep in r11).
        // r11 already has phys from the mov above.
        // Use: mov rcx, golden; mov rax, r11; mul rcx → rdx = high, rax = low
        // We want bits 48..63 of the 64-bit product = (result >> 48).
        // Since mul gives 128-bit result and we want bits 48-63 of the LOW 64 bits:
        //   shr rax, 48 gives bits 48-63 of low word. But that's only 16 bits.
        //   Actually we want the full product's bits 48-63.
        //   For Fibonacci hashing: h = (phys * golden) >> 48, where * is mod 2^64.
        //   So: mov rcx, golden; imul rax, r11, <can't do 64-bit imm>
        //   Instead: mov rax, golden; imul rax, r11 (2-op form: rax = rax * r11)
        //   Then shr rax, 48; and eax, mask; mov rcx, rax
        // Need imul_rr. Let's emit it manually: 0x48 0x0F 0xAF 0xC3 for imul rax, r11
        e.mov_ri64(Reg::Rax, 0x9E3779B97F4A7C15u64);
        // imul rax, r11 → REX.W 0F AF ModRM(rax, r11)
        // REX = 0x49 (W=1, B=1 for r11), opcode 0x0F 0xAF, ModRM = 0xC3 (reg=rax(0), rm=r11(3))
        e.emit_raw(&[0x49, 0x0F, 0xAF, 0xC3]);
        e.shr_ri(OpSize::S64, Reg::Rax, 48);
        e.and_ri(OpSize::S64, Reg::Rax, mask as i32);
        e.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);

        // Compute entry pointer: entries_ptr + rcx * 24
        // rcx * 24 = rcx * 3 * 8: lea rcx, [rcx + rcx*2] then shl 3
        // We need SIB for lea [rcx + rcx*2]. Use manual: rcx = rcx + rcx + rcx via add
        e.mov_rr(OpSize::S64, Reg::Rdx, Reg::Rcx);
        e.add_rr(OpSize::S64, Reg::Rcx, Reg::Rdx);
        e.add_rr(OpSize::S64, Reg::Rcx, Reg::Rdx); // rcx = rcx_orig * 3
        e.shl_ri(OpSize::S64, Reg::Rcx, 3);          // rcx = rcx_orig * 24

        // rdx = entries_ptr
        e.mov_ri64(Reg::Rdx, entries_ptr);
        e.add_rr(OpSize::S64, Reg::Rcx, Reg::Rdx); // rcx = &entries[hash]

        // Get mode_cs for comparison: call jit_get_cpu_mode(cpu) -> al
        // Save rcx (entry ptr) and r11 (phys) across call
        e.push(Reg::Rcx);
        e.push(Reg::R11);
        e.mov_rr(OpSize::S64, Reg::Rdi, CPU);
        e.call_abs(get_cpu_mode_addr);
        // al = mode. Build mode_cs = (mode as u64) | (cs_base << 8)
        e.movzx_rr8(OpSize::S64, Reg::Rax, Reg::Rax);
        e.mov_rm(OpSize::S64, Reg::Rdi, GPR_BASE, CS_BASE_OFF);
        e.shl_ri(OpSize::S64, Reg::Rdi, 8);
        e.or_rr(OpSize::S64, Reg::Rax, Reg::Rdi);
        // rax = mode_cs
        e.pop(Reg::R11);
        e.pop(Reg::Rcx);

        // Probe slots 0-3 (matching MAX_PROBES=4 in insert)
        // rcx = &entries[hash], r11 = phys, rax = mode_cs
        for i in 0..4 {
            let miss_label = if i < 3 {
                let next = e.new_label();
                // Compare phys_addr
                e.cmp_rm(OpSize::S64, Reg::R11, Reg::Rcx, ENTRY_PHYS_OFF);
                e.jcc_label(Cc::Ne, next);
                e.cmp_rm(OpSize::S64, Reg::Rax, Reg::Rcx, ENTRY_MODE_CS_OFF);
                e.jcc_label(Cc::Ne, next);
                // Hit!
                e.mov_rm(OpSize::S64, Reg::Rdx, Reg::Rcx, ENTRY_CODE_OFF);
                e.jmp_reg(Reg::Rdx);
                e.bind_label(next);
                e.add_ri(OpSize::S64, Reg::Rcx, ENTRY_SIZE as i32);
                next
            } else {
                // Last probe — miss goes to exit_compile
                e.cmp_rm(OpSize::S64, Reg::R11, Reg::Rcx, ENTRY_PHYS_OFF);
                e.jcc_label(Cc::Ne, exit_compile);
                e.cmp_rm(OpSize::S64, Reg::Rax, Reg::Rcx, ENTRY_MODE_CS_OFF);
                e.jcc_label(Cc::Ne, exit_compile);
                // Hit on last probe!
                e.mov_rm(OpSize::S64, Reg::Rdx, Reg::Rcx, ENTRY_CODE_OFF);
                e.jmp_reg(Reg::Rdx);
                exit_compile // unused but needed for type
            };
        }

        // ── 3. Exit paths ──

        // exit_compile: store pending compile info, exit with NEEDS_COMPILE
        e.bind_label(exit_compile);
        e.mov_mr(OpSize::S64, CPU, pending_phys_off, Reg::R11);
        // Store mode (low byte of rax = mode_cs)
        e.mov_mr(OpSize::S8, CPU, pending_mode_off, Reg::Rax);
        // Store cs_base: shift mode_cs >> 8
        e.mov_rr(OpSize::S64, Reg::Rdx, Reg::Rax);
        e.shr_ri(OpSize::S64, Reg::Rdx, 8);
        e.mov_mr(OpSize::S64, CPU, pending_cs_base_off, Reg::Rdx);
        e.mov_ri32(Reg::Rax, EXIT_NEEDS_COMPILE);
        e.jmp_label(dispatch_exit);

        e.bind_label(exit_stop);
        e.mov_ri32(Reg::Rax, EXIT_STOP);
        e.jmp_label(dispatch_exit);

        e.bind_label(exit_limit);
        e.mov_ri32(Reg::Rax, EXIT_LIMIT);
        e.jmp_label(dispatch_exit);

        e.bind_label(exit_fault);
        e.mov_ri32(Reg::Rax, EXIT_FAULT);
        e.jmp_label(dispatch_exit);

        e.bind_label(exit_smc);
        e.mov_ri32(Reg::Rax, EXIT_SMC);
        e.jmp_label(dispatch_exit);

        e.bind_label(exit_halt);
        e.mov_ri32(Reg::Rax, EXIT_HALT);
        e.jmp_label(dispatch_exit);

        e.bind_label(exit_interrupt);
        e.mov_ri32(Reg::Rax, EXIT_INTERRUPT);
        e.jmp_label(dispatch_exit);

        // ── 4. dispatch_exit: epilogue ──
        e.bind_label(dispatch_exit);
        e.add_ri(OpSize::S64, Reg::Rsp, 24);
        e.pop(Reg::Rbp);
        e.pop(Reg::R15);
        e.pop(Reg::R14);
        e.pop(Reg::R13);
        e.pop(Reg::R12);
        e.pop(Reg::Rbx);
        e.ret();

        // Save label offsets before finalize consumes the emitter
        let dispatcher_off = 0usize; // entry is at the start
        let loop_off = e.label_offset(dispatch_loop);

        let code = e.finalize();
        let offset = self.buffer.emit(&code).expect("JIT buffer full emitting dispatcher");

        self.dispatcher_offset = offset + dispatcher_off;
        self.dispatch_loop_offset = offset + loop_off;
    }
}
