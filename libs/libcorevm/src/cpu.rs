//! CPU emulation core — state management and execution loop.
//!
//! The `Cpu` struct holds all architectural state (registers, FPU, SSE)
//! and implements the fetch-decode-execute cycle. The execution loop
//! catches instruction errors and routes them to the guest's IDT as
//! hardware exceptions.

#[inline(always)]
pub(crate) fn rdtsc() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem));
        (hi as u64) << 32 | lo as u64
    }
}

use crate::decoder::{CpuMode, Decoder};
use crate::error::{Result, VmError};
use crate::fpu_state::FpuState;
use crate::interrupts::InterruptController;
use crate::io::IoDispatch;
use crate::jit::block::{self, BlockKey};
use crate::jit::cache::DecodeCache;
use crate::memory::{AccessType, GuestMemory, MemoryBus, Mmu};
use crate::registers::SegmentDescriptor;
use crate::registers::{
    RegisterFile, SegReg, CR0_PE, CR0_PG, EFER_LMA, EFER_LME, MSR_EFER,
};
use crate::sse_state::SseState;


/// CPU execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    /// 16-bit real mode.
    RealMode,
    /// 32-bit protected mode.
    ProtectedMode,
    /// 64-bit long mode.
    LongMode,
}

/// Reason the CPU stopped executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    /// HLT instruction executed.
    Halted,
    /// Unhandled exception (double/triple fault or non-exception error).
    Exception(VmError),
    /// Maximum instruction count reached.
    InstructionLimit,
    /// Breakpoint (INT 3 or hardware debug breakpoint).
    Breakpoint,
    /// External stop request via `request_stop()`.
    StopRequested,
}

/// Internal result from executing a cached basic block.
///
/// Used by `execute_cached_block` to signal whether the main `run()` loop
/// should continue iterating or return an `ExitReason` to the caller.
enum BlockExitReason {
    /// Block completed (or exception was injected) — continue the main loop.
    Continue,
    /// Block produced a terminal exit — return this reason from `run()`.
    Exit(ExitReason),
}

/// Virtual x86 CPU.
pub struct Cpu {
    /// CPU registers (GPR, segment, control, MSR, etc.).
    pub regs: RegisterFile,
    /// x87 FPU state.
    pub fpu: FpuState,
    /// SSE register state.
    pub sse: SseState,
    /// Instruction decoder.
    pub decoder: Decoder,
    /// Current CPU mode.
    pub mode: Mode,
    /// Number of instructions executed since last reset.
    pub instruction_count: u64,
    /// If true, stop at the next instruction boundary.
    pub stop_requested: bool,
    /// A20 gate enabled (address line 20 masking for real-mode compat).
    pub a20_enabled: bool,
    /// RIP at the start of the last successfully decoded instruction.
    pub last_exec_rip: u64,
    /// CS selector at the start of the last decoded instruction.
    pub last_exec_cs: u16,
    /// Opcode of the last decoded instruction (for diagnostics).
    pub last_opcode: u16,
    /// Opcode of the instruction before `last_opcode`.
    pub prev_opcode: u16,
    /// Physical address of the last decoded instruction.
    pub last_fetch_addr: u64,
    /// RIP of the instruction before the current one (for crash diagnostics).
    pub prev_exec_rip: u64,
    /// CS selector of the instruction before the current one.
    pub prev_exec_cs: u16,
    /// Decode cache for pre-decoded basic blocks (JIT Phase 1).
    pub decode_cache: DecodeCache,
    /// JIT session for native code compilation and execution (Phase 2+3).
    pub jit_session: crate::jit::session::JitSession,
    /// Consecutive exception count at the same RIP (for #PF loop detection).
    consecutive_exception_rip: u64,
    consecutive_exception_vector: u8,
    consecutive_exception_error_code: u32,
    consecutive_exception_cr2_page: u64,
    consecutive_exception_count: u32,
    /// Set by jit_mem_read on page fault; checked after each read in JIT code.
    pub jit_fault: bool,
    /// Set when SMC dirty pages are detected; dispatcher checks this to exit.
    pub smc_pending: bool,
    /// Set by helper_execute_one when HLT is executed; dispatcher exits with EXIT_HALT.
    pub jit_halted: bool,
    /// Physical address that caused a JIT hashtable miss.
    pub pending_compile_phys: u64,
    /// CpuMode at the time of hashtable miss.
    pub pending_compile_mode: u8,
    /// CS base at the time of hashtable miss.
    pub pending_compile_cs_base: u64,
    /// True when delivering a hardware interrupt with vector < 32 (BIOS PIC range).
    /// Used to skip Task Gate handling that would incorrectly trigger #DF.
    is_hw_interrupt: bool,
    /// Local APIC/x2APIC ID used for CPUID topology reporting.
    pub apic_id: u32,
    /// Configured number of logical CPUs in the VM package.
    pub logical_cpu_count: u8,
    /// Ring buffer of recent exceptions for BSOD diagnosis.
    /// Each entry: (rip, vector, error_code, cr2).
    pub exc_ring: [(u64, u8, u32, u64); 32],
    pub exc_ring_idx: usize,
    /// Ring buffer for non-#PF exceptions only (vec != 14).
    pub exc_ring_nopf: [(u64, u8, u32, u64); 32],
    pub exc_ring_nopf_idx: usize,
    /// TSC-based timing counters for profiling the run loop.
    pub perf_tsc_interrupt: u64,
    pub perf_tsc_translate: u64,
    pub perf_tsc_smc: u64,
    pub perf_tsc_decode: u64,
    pub perf_tsc_jit_exec: u64,
    pub perf_tsc_interp: u64,
    pub perf_tsc_total: u64,
    pub perf_loop_count: u64,
    /// Host TSC frequency (Hz) for wall-clock-based guest TSC.
    pub tsc_host_freq: u64,
    /// Guest TSC frequency (Hz) — what RDTSC should report.
    pub tsc_guest_freq: u64,
    /// Host TSC value at VM start, for computing guest TSC offset.
    pub tsc_host_base: u64,
}

impl Cpu {
    /// Create a new CPU in real mode with power-on reset defaults.
    pub fn new() -> Self {
        Cpu {
            regs: RegisterFile::new(),
            fpu: FpuState::new(),
            sse: SseState::new(),
            decoder: Decoder::new(CpuMode::Real16),
            mode: Mode::RealMode,
            instruction_count: 0,
            stop_requested: false,
            a20_enabled: true,
            last_exec_rip: 0,
            last_exec_cs: 0,
            last_opcode: 0,
            prev_opcode: 0,
            last_fetch_addr: 0,
            prev_exec_rip: 0,
            prev_exec_cs: 0,
            decode_cache: DecodeCache::new(),
            jit_session: crate::jit::session::JitSession::new(),
            consecutive_exception_rip: 0,
            consecutive_exception_vector: 0xFF,
            consecutive_exception_error_code: 0,
            consecutive_exception_cr2_page: u64::MAX,
            consecutive_exception_count: 0,
            exc_ring: [(0, 0, 0, 0); 32],
            exc_ring_nopf: [(0, 0, 0, 0); 32],
            exc_ring_nopf_idx: 0,
            exc_ring_idx: 0,
            jit_fault: false,
            smc_pending: false,
            jit_halted: false,
            pending_compile_phys: 0,
            pending_compile_mode: 0,
            pending_compile_cs_base: 0,
            is_hw_interrupt: false,
            apic_id: 0,
            logical_cpu_count: 1,
            perf_tsc_interrupt: 0,
            perf_tsc_translate: 0,
            perf_tsc_smc: 0,
            perf_tsc_decode: 0,
            perf_tsc_jit_exec: 0,
            perf_tsc_interp: 0,
            perf_tsc_total: 0,
            perf_loop_count: 0,
            tsc_host_freq: 0,
            tsc_guest_freq: 2_000_000_000, // 2 GHz default
            tsc_host_base: 0,
        }
    }

    /// Reset the CPU to power-on state.
    pub fn reset(&mut self) {
        self.regs = RegisterFile::new();
        self.fpu = FpuState::new();
        self.sse = SseState::new();
        self.mode = Mode::RealMode;
        self.decoder.set_mode(CpuMode::Real16);
        self.instruction_count = 0;
        self.stop_requested = false;
        self.last_exec_rip = 0;
        self.last_exec_cs = 0;
        self.last_opcode = 0;
        self.prev_opcode = 0;
        self.last_fetch_addr = 0;
        self.prev_exec_rip = 0;
        self.prev_exec_cs = 0;
        self.decode_cache.flush();
        self.jit_session.flush();
        self.consecutive_exception_rip = 0;
        self.consecutive_exception_vector = 0xFF;
        self.consecutive_exception_error_code = 0;
        self.consecutive_exception_cr2_page = u64::MAX;
        self.consecutive_exception_count = 0;
        self.apic_id = 0;
        self.logical_cpu_count = 1;
    }

    /// Configure the CPU topology values used for CPUID/APIC reporting.
    ///
    /// The emulator currently executes a single vCPU, but the configured
    /// `logical_cpu_count` is exposed to guests so SMP-capable kernels can
    /// detect the intended topology.
    pub fn configure_topology(&mut self, apic_id: u32, logical_cpu_count: u8) {
        self.apic_id = apic_id;
        self.logical_cpu_count = logical_cpu_count.max(1);
    }

    fn exception_loop_key(error: &VmError) -> (u8, u32, u64) {
        let vector = error.exception_vector().unwrap_or(0xFF);
        let error_code = error.error_code().unwrap_or(0);
        let cr2_page = match *error {
            VmError::PageFault { address, .. } => address & !0xFFF,
            _ => u64::MAX,
        };
        (vector, error_code, cr2_page)
    }

    fn note_exception_repeat(&mut self, error: &VmError) {
        let (vector, error_code, cr2_page) = Self::exception_loop_key(error);
        if self.last_exec_rip == self.consecutive_exception_rip
            && vector == self.consecutive_exception_vector
            && error_code == self.consecutive_exception_error_code
            && cr2_page == self.consecutive_exception_cr2_page
        {
            self.consecutive_exception_count += 1;
        } else {
            self.consecutive_exception_rip = self.last_exec_rip;
            self.consecutive_exception_vector = vector;
            self.consecutive_exception_error_code = error_code;
            self.consecutive_exception_cr2_page = cr2_page;
            self.consecutive_exception_count = 1;
        }
    }

    /// Mask RIP to the appropriate width for the current CPU mode.
    ///
    /// In real mode and 16-bit protected mode, RIP is limited to 16 bits.
    /// In 32-bit protected mode (CS.D=1), RIP is limited to 32 bits.
    /// In long mode, RIP is used as-is.
    #[inline]
    /// Return the RIP mask for the current mode (avoids match per instruction).
    #[inline(always)]
    pub fn rip_mask(&self) -> u64 {
        match self.mode {
            Mode::RealMode => 0xFFFF,
            Mode::ProtectedMode => {
                if self.regs.seg[SegReg::Cs as usize].big {
                    0xFFFF_FFFF
                } else {
                    0xFFFF
                }
            }
            Mode::LongMode => u64::MAX,
        }
    }

    /// Decode an instruction that straddles a page boundary by building a
    /// combined buffer from two physical pages.
    fn decode_cross_page(
        &self,
        fetch_addr: u64,
        phys_addr: u64,
        memory: &GuestMemory,
        mmu: &Mmu,
    ) -> Result<crate::instruction::DecodedInst> {
        use crate::decoder::MAX_INST_LEN;

        let mut buf = [0u8; MAX_INST_LEN];
        let bytes_in_page = (0x1000 - (phys_addr & 0xFFF)) as usize;

        // Read bytes from the first (current) physical page.
        for i in 0..bytes_in_page {
            buf[i] = memory.read_u8(phys_addr + i as u64).unwrap_or(0xFF);
        }

        // Translate the next virtual page to get its physical address.
        let next_virt_page = (fetch_addr & !0xFFF) + 0x1000;
        let next_phys = mmu.translate_linear(
            next_virt_page,
            self.regs.cr3,
            AccessType::Execute,
            self.regs.cpl,
            &*memory,
        )?;

        // Read remaining bytes from the second physical page.
        let remaining = MAX_INST_LEN - bytes_in_page;
        for i in 0..remaining {
            buf[bytes_in_page + i] = memory.read_u8(next_phys + i as u64).unwrap_or(0xFF);
        }

        self.decoder.decode_from_buf(&buf, MAX_INST_LEN, phys_addr)
    }

    pub fn mask_rip(&mut self) {
        match self.mode {
            Mode::RealMode => {
                self.regs.rip &= 0xFFFF;
            }
            Mode::ProtectedMode => {
                if self.regs.seg[SegReg::Cs as usize].big {
                    self.regs.rip &= 0xFFFF_FFFF;
                } else {
                    self.regs.rip &= 0xFFFF;
                }
            }
            Mode::LongMode => {}
        }
    }

    /// Request the CPU to stop at the next instruction boundary.
    pub fn request_stop(&mut self) {
        self.stop_requested = true;
    }

    /// Derive the correct `CpuMode` from current control register state.
    fn compute_mode(&self) -> CpuMode {
        let pe = self.regs.cr0 & CR0_PE != 0;
        let pg = self.regs.cr0 & CR0_PG != 0;
        let efer = self.regs.read_msr(MSR_EFER);
        let lma = efer & EFER_LMA != 0;
        let cs_long = self.regs.seg[SegReg::Cs as usize].long_mode;
        let cs_big = self.regs.seg[SegReg::Cs as usize].big;

        if pe && pg && lma && cs_long {
            CpuMode::Long64
        } else if pe && cs_big {
            // 32-bit protected mode: CS.D=1 → default 32-bit operand/address
            CpuMode::Protected32
        } else if pe {
            // 16-bit protected mode: CS.D=0 → default 16-bit operand/address
            // (e.g., immediately after MOV CR0 enables PE, before far JMP
            // loads a 32-bit CS descriptor)
            CpuMode::Real16
        } else {
            CpuMode::Real16
        }
    }

    /// Update the CPU mode after a control register, EFER, or CS change.
    ///
    /// Also handles the automatic setting of EFER.LMA when CR0.PG is
    /// enabled with EFER.LME set (and vice versa).
    pub fn update_mode(&mut self) {
        // EFER.LMA is automatically set/cleared based on CR0.PG + EFER.LME
        let efer = self.regs.read_msr(MSR_EFER);
        let pg = self.regs.cr0 & CR0_PG != 0;
        let lme = efer & EFER_LME != 0;
        if pg && lme {
            self.regs.write_msr(MSR_EFER, efer | EFER_LMA);
        } else {
            self.regs.write_msr(MSR_EFER, efer & !EFER_LMA);
        }

        let new_mode = self.compute_mode();
        self.decoder.set_mode(new_mode);

        // The CPU mode (for segment lookups, privilege checks, etc.) is
        // determined by CR0.PE and EFER.LMA, independent of the CS.D bit.
        // CS.D only affects the decoder's default operand/address size.
        let pe = self.regs.cr0 & CR0_PE != 0;
        let lma = self.regs.read_msr(MSR_EFER) & EFER_LMA != 0;
        let cs_long = self.regs.seg[SegReg::Cs as usize].long_mode;
        self.mode = if pe && pg && lma && cs_long {
            Mode::LongMode
        } else if pe {
            Mode::ProtectedMode
        } else {
            Mode::RealMode
        };

        // Sync MMU state will be done by the caller (VmEngine.run updates Mmu)
    }

    /// Read a segment descriptor from the GDT given a selector.
    ///
    /// Performs bounds checking against the GDTR limit and translates
    /// the GDT base address through paging if enabled.
    ///
    /// In long mode, system descriptors (TSS, LDT) are 16 bytes. The
    /// upper 8 bytes contain bits [63:32] of the base address. This
    /// method detects system descriptors and reads the full 16 bytes.
    ///
    /// # Errors
    ///
    /// Returns `VmError::GeneralProtection` if the selector index exceeds
    /// the GDT limit or if the memory read fails.
    pub fn read_gdt_descriptor(
        &self,
        selector: u16,
        memory: &GuestMemory,
        mmu: &Mmu,
    ) -> Result<SegmentDescriptor> {
        let index = (selector & 0xFFF8) as u64;
        if index + 7 > self.regs.gdtr.limit as u64 {
            return Err(VmError::GeneralProtection(selector as u32 & 0xFFFC));
        }
        let addr = self.regs.gdtr.base.wrapping_add(index);
        // GDT access is always an implicit supervisor operation (CPL=0).
        let phys = mmu.translate_linear(
            addr,
            self.regs.cr3,
            AccessType::Read,
            0,
            memory,
        )?;
        let raw = memory.read_u64(phys)?;
        let mut desc = SegmentDescriptor::from_raw(selector, raw);

        // Long-mode system descriptors (TSS, LDT) are 16 bytes wide.
        // Access byte bit 4 (S flag) = 0 indicates a system descriptor.
        // This applies whenever EFER.LMA=1, including compatibility mode.
        let is_system = (desc.access & 0x10) == 0;
        let lma = (self.regs.read_msr(MSR_EFER) & EFER_LMA) != 0;
        if lma && is_system && desc.present {
            if index + 15 > self.regs.gdtr.limit as u64 {
                return Err(VmError::GeneralProtection(selector as u32 & 0xFFFC));
            }
            let addr_hi = self.regs.gdtr.base.wrapping_add(index + 8);
            let phys_hi = mmu.translate_linear(
                addr_hi,
                self.regs.cr3,
                AccessType::Read,
                0,
                memory,
            )?;
            let raw_hi = memory.read_u64(phys_hi)?;
            // Bits [31:0] of the upper qword hold base[63:32].
            let base_upper = raw_hi & 0xFFFF_FFFF;
            desc.base |= base_upper << 32;
        }

        Ok(desc)
    }

    /// Load a segment register by reading its descriptor from the GDT.
    ///
    /// For null selectors (index 0), loads a null descriptor. Null selectors
    /// are allowed for DS, ES, FS, GS but not for CS or SS.
    pub fn load_segment_from_gdt(
        &mut self,
        seg: SegReg,
        selector: u16,
        memory: &GuestMemory,
        mmu: &Mmu,
    ) -> Result<()> {
        if (selector & 0xFFFC) == 0 {
            // Null selector — allowed for data segments.
            // CS never allows null. SS allows null in 64-bit mode at CPL 0/1/2
            // (Intel SDM Vol. 3A §5.4.1.1: null SS is valid in 64-bit mode).
            if matches!(seg, SegReg::Cs) {
                return Err(VmError::GeneralProtection(0));
            }
            if matches!(seg, SegReg::Ss) && !matches!(self.mode, Mode::LongMode) {
                return Err(VmError::GeneralProtection(0));
            }
            let desc = &mut self.regs.seg[seg as usize];
            desc.selector = selector;
            desc.base = 0;
            desc.limit = 0;
            desc.present = false;
            desc.is_code = false;
            desc.readable = false;
            desc.writable = false;
            return Ok(());
        }
        // LDT selectors (TI=1) not supported — use GDT regardless.
        let desc = self.read_gdt_descriptor(selector, memory, mmu)?;
        self.regs.seg[seg as usize] = desc;
        Ok(())
    }

    /// Get the stack operand size for the current mode.
    pub fn stack_size(&self) -> crate::flags::OperandSize {
        match self.mode {
            Mode::LongMode => crate::flags::OperandSize::Qword,
            Mode::ProtectedMode => {
                if self.regs.seg[SegReg::Ss as usize].big {
                    crate::flags::OperandSize::Dword
                } else {
                    crate::flags::OperandSize::Word
                }
            }
            Mode::RealMode => crate::flags::OperandSize::Word,
        }
    }

    /// Execute instructions until an exit condition is reached.
    ///
    /// # Arguments
    /// * `memory` — Guest physical memory
    /// * `mmu` — Memory management unit (segmentation + paging)
    /// * `interrupts` — Interrupt controller
    /// * `io` — Port I/O dispatcher
    /// * `max_instructions` — Stop after this many instructions (0 = unlimited)
    pub fn run(
        &mut self,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        interrupts: &mut InterruptController,
        io: &mut IoDispatch,
        max_instructions: u64,
    ) -> ExitReason {
        // Compute absolute target so the limit applies per-call, not cumulatively.
        let target = if max_instructions > 0 {
            self.instruction_count.saturating_add(max_instructions)
        } else {
            0
        };

        if self.jit_session.is_enabled() {
            return self.run_jit_session(memory, mmu, io, interrupts, target);
        }

        loop {
            let t_loop_start = rdtsc();

            if self.stop_requested {
                self.stop_requested = false;
                return ExitReason::StopRequested;
            }
            if target > 0 && self.instruction_count >= target {
                return ExitReason::InstructionLimit;
            }
            // Sync MMU state from control registers (fast-path: skips if unchanged).
            mmu.update_from_regs(self.regs.cr0, self.regs.cr4, self.regs.efer);
            mmu.rflags_ac = (self.regs.rflags & crate::flags::AC) != 0;

            crate::poll_external_irqs(interrupts, self.regs.rflags);

            // Check pending interrupts (only if IF=1 and no interrupt shadow)
            if let Some(vector) = interrupts.pending_interrupt(self.regs.rflags) {
                interrupts.acknowledge(vector);
                if let Err(e) = self.deliver_interrupt_hw(vector, memory, mmu, interrupts)
                {
                    return ExitReason::Exception(e);
                }
                interrupts.interrupt_shadow = false;
            }
            interrupts.interrupt_shadow = false;

            let t_after_intr = rdtsc();

            // Compute the linear address of the instruction
            let cs = &self.regs.seg[SegReg::Cs as usize];
            let fetch_addr = cs.base.wrapping_add(self.regs.rip);
            let fetch_addr = if !self.a20_enabled {
                fetch_addr & !0x10_0000
            } else {
                fetch_addr
            };

            // Translate through paging if enabled
            let phys_addr = match mmu.translate_linear(
                fetch_addr,
                self.regs.cr3,
                AccessType::Execute,
                self.regs.cpl,
                &*memory,
            ) {
                Ok(addr) => addr,
                Err(e) => {
                    if self.regs.rip < 0x1000 || (self.regs.seg[SegReg::Cs as usize].selector >= 0x60 && self.instruction_count > 50_000_000) {
                        libsyscall::serial_print(format_args!(
                            "[corevm] FETCH FAULT at CS:EIP={:04X}:{:08X} fetch_addr={:08X} last_exec={:04X}:{:08X} prev={:04X}:{:08X} CR3={:08X}: {:?}\n",
                            self.regs.seg[SegReg::Cs as usize].selector,
                            self.regs.rip as u32,
                            fetch_addr as u32,
                            self.last_exec_cs,
                            self.last_exec_rip as u32,
                            self.prev_exec_cs,
                            self.prev_exec_rip as u32,
                            self.regs.cr3 as u32,
                            e,
                        ));
                    }
                    if let Err(e2) =
                        self.inject_exception_from_error(&e, memory, mmu, interrupts)
                    {
                        return ExitReason::Exception(e2);
                    }
                    continue;
                }
            };

            let t_after_translate = rdtsc();

            // Save trace info for diagnostics before decode/execute.
            self.prev_exec_rip = self.last_exec_rip;
            self.prev_exec_cs = self.last_exec_cs;
            self.last_exec_rip = self.regs.rip;
            self.last_exec_cs = self.regs.seg[SegReg::Cs as usize].selector;
            self.last_fetch_addr = phys_addr;

            // ── Self-modifying code: invalidate decode cache for written pages ──
            self.drain_smc_invalidations();

            let t_after_smc = rdtsc();

            // ── Debugger hook (before decode cache) ──
            #[cfg(feature = "host_test")]
            if crate::debugger::should_break(self, phys_addr) {
                crate::debugger::enter_prompt(self, memory, mmu, phys_addr, "breakpoint");
            }

            // ── Hang detection trap: detect `jmp $` (EB FE) spin loops ──
            #[cfg(feature = "host_test")]
            if self.prev_exec_rip == self.regs.rip && self.regs.rip >= 0x80000000 {
                use crate::memory::MemoryBus;
                let _chk_p = mmu.translate_linear(self.regs.rip, self.regs.cr3, crate::memory::AccessType::Read, 0, memory).unwrap_or(0);
                let _b0 = memory.read_u8(_chk_p).unwrap_or(0);
                let _b1 = memory.read_u8(_chk_p + 1).unwrap_or(0);
            if _b0 == 0xEB && _b1 == 0xFE {
                fn trl2(mmu: &Mmu, cr3: u64, va: u64, mem: &GuestMemory) -> u64 {
                    mmu.translate_linear(va, cr3, crate::memory::AccessType::Read, 0, mem).unwrap_or(0)
                }
                fn rd32h(mmu: &Mmu, cr3: u64, va: u64, mem: &GuestMemory) -> u32 {
                    mem.read_u32(trl2(mmu, cr3, va, mem)).unwrap_or(0)
                }
                let cr3 = self.regs.cr3;
                eprintln!("[HANG-TRAP] RIP={:08X} EAX={:08X} ECX={:08X} EDX={:08X} ESP={:08X} EBP={:08X} CR2={:08X}",
                    self.regs.rip as u32, self.regs.gpr[0] as u32, self.regs.gpr[1] as u32,
                    self.regs.gpr[2] as u32, self.regs.sp() as u32, self.regs.gpr[5] as u32, self.regs.cr2 as u32);
                // Walk EBP chain
                let mut ebp = self.regs.gpr[5];
                for frame in 0..15u32 {
                    let next = rd32h(mmu, cr3, ebp, memory) as u64;
                    let ret = rd32h(mmu, cr3, ebp + 4, memory);
                    let a0 = rd32h(mmu, cr3, ebp + 8, memory);
                    let a1 = rd32h(mmu, cr3, ebp + 12, memory);
                    let a2 = rd32h(mmu, cr3, ebp + 16, memory);
                    let a3 = rd32h(mmu, cr3, ebp + 20, memory);
                    let a4 = rd32h(mmu, cr3, ebp + 24, memory);
                    eprintln!("[HANG-TRAP] frame{}: EBP={:08X} RET={:08X} args: {:08X} {:08X} {:08X} {:08X} {:08X}",
                        frame, ebp as u32, ret, a0, a1, a2, a3, a4);
                    if next == 0 || next < 0x80000000 { break; }
                    ebp = next;
                }
                // PE checksum verification of ntdll.dll mapped at VA 0x34000
                fn rd8h(mmu: &Mmu, cr3: u64, va: u64, mem: &GuestMemory) -> u8 {
                    mem.read_u8(trl2(mmu, cr3, va, mem)).unwrap_or(0)
                }
                fn rd16h(mmu: &Mmu, cr3: u64, va: u64, mem: &GuestMemory) -> u16 {
                    mem.read_u16(trl2(mmu, cr3, va, mem)).unwrap_or(0)
                }
                let base_va = 0x34000u64;
                // Check MZ header
                let mz = rd16h(mmu, cr3, base_va, memory);
                eprintln!("[HANG-TRAP] PE check: MZ={:04X}", mz);
                // Scan physical RAM for large PE images (ntdll-sized) — only once
                use core::sync::atomic::{AtomicBool, Ordering as AtOrd};
                static PE_SCANNED: AtomicBool = AtomicBool::new(false);
                if !PE_SCANNED.swap(true, AtOrd::Relaxed) {
                    let ram_size = memory.ram_size();
                    for pa in (0u64..ram_size as u64).step_by(0x1000) {
                        let b0 = memory.read_u8(pa).unwrap_or(0);
                        let b1 = memory.read_u8(pa + 1).unwrap_or(0);
                        if b0 == 0x4D && b1 == 0x5A {
                            let pe_off = memory.read_u32(pa + 0x3C).unwrap_or(0) as u64;
                            let pe_sig = if pe_off < 0x1000 { memory.read_u32(pa + pe_off).unwrap_or(0) } else { 0 };
                            if pe_sig == 0x00004550 {
                                let opt_off = pe_off + 24;
                                let img_size = memory.read_u32(pa + opt_off + 56).unwrap_or(0);
                                if img_size >= 0x10000 {
                                    eprintln!("[HANG-TRAP] PE at PA {:08X} pe_off={:X} img_size={:X}", pa, pe_off, img_size);
                                }
                            }
                        }
                    }
                    // Dump page directory for user-space VA 0x34000
                    let pd_base = cr3 & 0xFFFFF000;
                    let pde = memory.read_u32(pd_base).unwrap_or(0);
                    eprintln!("[HANG-TRAP] CR3={:08X} PDE[0]={:08X}", cr3, pde);
                    if pde & 1 != 0 {
                        let pt_base = (pde & 0xFFFFF000) as u64;
                        for pt_idx in 0x30u64..=0x45 {
                            let pte = memory.read_u32(pt_base + pt_idx * 4).unwrap_or(0);
                            if pte & 1 != 0 {
                                let pa = (pte & 0xFFFFF000) as u64;
                                let b0 = memory.read_u8(pa).unwrap_or(0);
                                let b1 = memory.read_u8(pa + 1).unwrap_or(0);
                                eprintln!("[HANG-TRAP] PTE[{:03X}] VA {:05X} = {:08X} PA {:08X} [{:02X} {:02X}]",
                                    pt_idx, pt_idx << 12, pte, pa, b0, b1);
                            }
                        }
                    } else {
                        eprintln!("[HANG-TRAP] PDE[0] not present!");
                    }
                }
                // Verify physical ntdll checksum at known PAs
                if !PE_SCANNED.load(AtOrd::Relaxed) { /* already scanned above */ }
                for &ntdll_pa in &[0xC9C000u64, 0x2039000, 0x0FF30000, 0xFF30000] {
                    let nm = memory.read_u16(ntdll_pa).unwrap_or(0);
                    if nm != 0x5A4D { continue; }
                    let pe_o = memory.read_u32(ntdll_pa + 0x3C).unwrap_or(0) as u64;
                    if pe_o >= 0x1000 { continue; }
                    let ps = memory.read_u32(ntdll_pa + pe_o).unwrap_or(0);
                    if ps != 0x4550 { continue; }
                    let oo = pe_o + 24;
                    let isz = memory.read_u32(ntdll_pa + oo + 56).unwrap_or(0);
                    if isz < 0xC0000 || isz > 0x100000 { continue; } // filter to ntdll-sized
                    let image_base = memory.read_u32(ntdll_pa + oo + 28).unwrap_or(0);
                    let sect_align = memory.read_u32(ntdll_pa + oo + 32).unwrap_or(0);
                    let file_align = memory.read_u32(ntdll_pa + oo + 36).unwrap_or(0);
                    let stored = memory.read_u32(ntdll_pa + oo + 64).unwrap_or(0);
                    let cs_off = oo + 64;
                    let mut s: u32 = 0;
                    for i in (0..isz as u64).step_by(2) {
                        if i >= cs_off && i < cs_off + 4 { continue; }
                        let w = memory.read_u16(ntdll_pa + i).unwrap_or(0) as u32;
                        s = s.wrapping_add(w);
                        s = (s >> 16) + (s & 0xFFFF);
                    }
                    s = (s >> 16) + (s & 0xFFFF);
                    s += isz;
                    // Also try computing as raw file (iterate up to file's raw extent, not SizeOfImage)
                    let num_sec_c = memory.read_u16(ntdll_pa + pe_o + 6).unwrap_or(0) as u64;
                    let opt_sz_c = memory.read_u16(ntdll_pa + pe_o + 20).unwrap_or(0) as u64;
                    let sec_base = pe_o + 24 + opt_sz_c;
                    let mut raw_end = 0u64;
                    for si in 0..num_sec_c.min(20) {
                        let so = sec_base + si * 40;
                        let rp = memory.read_u32(ntdll_pa + so + 20).unwrap_or(0) as u64;
                        let rs = memory.read_u32(ntdll_pa + so + 16).unwrap_or(0) as u64;
                        if rp + rs > raw_end { raw_end = rp + rs; }
                    }
                    // Check if data at PA is section-mapped (sections at VA offsets) or flat-file (sections at raw offsets)
                    // Section 0 (.text): if VA != rawptr, check which one matches
                    let sec0_va = memory.read_u32(ntdll_pa + sec_base + 12).unwrap_or(0);
                    let sec0_rp = memory.read_u32(ntdll_pa + sec_base + 20).unwrap_or(0);
                    let is_flat = if sec0_va != sec0_rp {
                        // Check if the data at sec0_rp offset looks like code (vs zeros at sec0_va)
                        let at_rp = memory.read_u32(ntdll_pa + sec0_rp as u64).unwrap_or(0);
                        let at_va = memory.read_u32(ntdll_pa + sec0_va as u64).unwrap_or(0);
                        // .text should have code; the wrong offset would have zeros or different data
                        eprintln!("[HANG-TRAP]   sec0 VA={:X} rawptr={:X} @VA={:08X} @rawptr={:08X}", sec0_va, sec0_rp, at_va, at_rp);
                        false // can't easily tell, but log it
                    } else { false };
                    // Compute raw-file-style checksum too
                    if raw_end > 0 && raw_end < 0x200000 {
                        let mut sf: u32 = 0;
                        for i in (0..raw_end).step_by(2) {
                            if i >= cs_off && i < cs_off + 4 { continue; }
                            let w = memory.read_u16(ntdll_pa + i).unwrap_or(0) as u32;
                            sf = sf.wrapping_add(w);
                            sf = (sf >> 16) + (sf & 0xFFFF);
                        }
                        sf = (sf >> 16) + (sf & 0xFFFF);
                        sf += raw_end as u32;
                        eprintln!("[HANG-TRAP]   raw_end={:X} raw_cksum={:08X}", raw_end, sf);
                    }
                    eprintln!("[HANG-TRAP] ntdll@PA{:X} isz={:X} imgbase={:08X} sa={:X} fa={:X} checksum: computed={:08X} stored={:08X} {}",
                        ntdll_pa, isz, image_base, sect_align, file_align, s, stored, if s == stored { "OK" } else { "MISMATCH" });
                    // Dump first 256 bytes for comparison with ISO
                    {
                        let mut hex = alloc::string::String::new();
                        for i in 0..256u64 {
                            if i % 32 == 0 && !hex.is_empty() {
                                eprintln!("[HEXDUMP] PA{:X}+{:03X}: {}", ntdll_pa, i - 32, hex);
                                hex.clear();
                            }
                            let b = memory.read_u8(ntdll_pa + i).unwrap_or(0);
                            use core::fmt::Write;
                            let _ = write!(hex, "{:02X} ", b);
                        }
                        if !hex.is_empty() {
                            eprintln!("[HEXDUMP] PA{:X}+{:03X}: {}", ntdll_pa, 256 - 32, hex);
                        }
                    }
                    // don't break — check all copies
                }
                // Scan for flat-file ntdll mappings (sec0 rawptr != VA)
                {
                    let ram_end = memory.ram_size().min(256 * 1024 * 1024) as u64;
                    let mut flat_count = 0u32;
                    let mut pa = 0u64;
                    while pa < ram_end {
                        let mz2 = memory.read_u16(pa).unwrap_or(0);
                        if mz2 == 0x5A4D {
                            let pe_o2 = memory.read_u32(pa + 0x3C).unwrap_or(0) as u64;
                            if pe_o2 < 0x400 {
                                let sig2 = memory.read_u32(pa + pe_o2).unwrap_or(0);
                                if sig2 == 0x4550 {
                                    let oo2 = pe_o2 + 24;
                                    let isz2 = memory.read_u32(pa + oo2 + 56).unwrap_or(0);
                                    if isz2 >= 0xC0000 && isz2 <= 0xD0000 {
                                        let stored2 = memory.read_u32(pa + oo2 + 64).unwrap_or(0);
                                        let num_s2 = memory.read_u16(pa + pe_o2 + 6).unwrap_or(0) as u64;
                                        let osz2 = memory.read_u16(pa + pe_o2 + 20).unwrap_or(0) as u64;
                                        let sb2 = pe_o2 + 24 + osz2;
                                        let s0_va = memory.read_u32(pa + sb2 + 12).unwrap_or(0);
                                        let s0_rp = memory.read_u32(pa + sb2 + 20).unwrap_or(0);
                                        let ib2 = memory.read_u32(pa + oo2 + 28).unwrap_or(0);
                                        if s0_va != s0_rp {
                                            flat_count += 1;
                                            eprintln!("[HANG-TRAP] FLAT ntdll@PA{:X} isz={:X} ib={:08X} stored={:08X} s0va={:X} s0rp={:X}",
                                                pa, isz2, ib2, stored2, s0_va, s0_rp);
                                            // Compute raw-file checksum
                                            let mut re2 = 0u64;
                                            for si in 0..num_s2.min(20) {
                                                let so = sb2 + si * 40;
                                                let rp = memory.read_u32(pa + so + 20).unwrap_or(0) as u64;
                                                let rs = memory.read_u32(pa + so + 16).unwrap_or(0) as u64;
                                                if rp + rs > re2 { re2 = rp + rs; }
                                            }
                                            let cs_off2 = oo2 + 64;
                                            let mut sf2: u32 = 0;
                                            for i in (0..re2).step_by(2) {
                                                if i >= cs_off2 && i < cs_off2 + 4 { continue; }
                                                let w = memory.read_u16(pa + i).unwrap_or(0) as u32;
                                                sf2 = sf2.wrapping_add(w);
                                                sf2 = (sf2 >> 16) + (sf2 & 0xFFFF);
                                            }
                                            sf2 = (sf2 >> 16) + (sf2 & 0xFFFF);
                                            sf2 += re2 as u32;
                                            eprintln!("[HANG-TRAP]   flat raw_end={:X} cksum={:08X} stored={:08X} {}",
                                                re2, sf2, stored2, if sf2 == stored2 { "OK" } else { "MISMATCH" });
                                        }
                                    }
                                }
                            }
                        }
                        pa += 0x1000;
                    }
                    if flat_count == 0 {
                        eprintln!("[HANG-TRAP] No flat-file ntdll mappings found");
                    }
                }
                // Compare first two ntdll copies byte-by-byte to find differences
                {
                    let pa_a = 0xC9C000u64;
                    let pa_b = 0x2039000u64;
                    let mz_a = memory.read_u16(pa_a).unwrap_or(0);
                    let mz_b = memory.read_u16(pa_b).unwrap_or(0);
                    if mz_a == 0x5A4D && mz_b == 0x5A4D {
                        let pe_o = memory.read_u32(pa_a + 0x3C).unwrap_or(0) as u64;
                        let oo = pe_o + 24;
                        let isz = memory.read_u32(pa_a + oo + 56).unwrap_or(0) as u64;
                        let mut diff_count = 0u32;
                        let mut first_diffs = alloc::vec::Vec::new();
                        for i in 0..isz.min(0xC4000) {
                            let ba = memory.read_u8(pa_a + i).unwrap_or(0);
                            let bb = memory.read_u8(pa_b + i).unwrap_or(0);
                            if ba != bb {
                                diff_count += 1;
                                if first_diffs.len() < 20 {
                                    first_diffs.push((i, ba, bb));
                                }
                            }
                        }
                        eprintln!("[HANG-TRAP] ntdll diff A(C9C000) vs B(2039000): {} bytes differ", diff_count);
                        for &(off, a, b) in &first_diffs {
                            eprintln!("[HANG-TRAP]  offset {:06X}: A={:02X} B={:02X}", off, a, b);
                        }
                        // Also show .reloc section info
                        let num_sec = memory.read_u16(pa_a + pe_o + 6).unwrap_or(0) as u64;
                        let opt_sz = memory.read_u16(pa_a + pe_o + 20).unwrap_or(0) as u64;
                        let sec_start = pe_o + 24 + opt_sz;
                        for s in 0..num_sec.min(20) {
                            let soff = sec_start + s * 40;
                            let mut name = [0u8; 8];
                            for k in 0..8 {
                                name[k] = memory.read_u8(pa_a + soff + k as u64).unwrap_or(0);
                            }
                            let va = memory.read_u32(pa_a + soff + 12).unwrap_or(0);
                            let vs = memory.read_u32(pa_a + soff + 8).unwrap_or(0);
                            let nm = core::str::from_utf8(&name).unwrap_or("???");
                            eprintln!("[HANG-TRAP]  sec{}: name={} VA={:08X} VSize={:08X}", s, nm.trim_end_matches('\0'), va, vs);
                        }
                    }
                }
                if mz == 0x5A4D {
                    let pe_off = rd32h(mmu, cr3, base_va + 0x3C, memory) as u64;
                    let pe_sig = rd32h(mmu, cr3, base_va + pe_off, memory);
                    eprintln!("[HANG-TRAP] PE sig={:08X} at offset {:X}", pe_sig, pe_off);
                    if pe_sig == 0x4550 {
                        // OptionalHeader starts at pe_off + 24
                        let opt_off = pe_off + 24;
                        let size_of_image = rd32h(mmu, cr3, base_va + opt_off + 56, memory);
                        let checksum_off = opt_off + 64;
                        let stored_checksum = rd32h(mmu, cr3, base_va + checksum_off, memory);
                        let size_of_headers = rd32h(mmu, cr3, base_va + opt_off + 60, memory);
                        eprintln!("[HANG-TRAP] SizeOfImage={:X} SizeOfHeaders={:X} StoredChecksum={:08X}",
                            size_of_image, size_of_headers, stored_checksum);

                        // Compute checksum over mapped image (NOT file image)
                        // Windows computes it over the mapped image in memory
                        let mut sum: u32 = 0;
                        let checksum_va = base_va + checksum_off;
                        let img_size = size_of_image as u64;
                        for i in (0..img_size).step_by(2) {
                            let va = base_va + i;
                            // Skip the 4-byte checksum field
                            if va >= checksum_va && va < checksum_va + 4 { continue; }
                            let w = rd16h(mmu, cr3, va, memory) as u32;
                            sum += w;
                            sum = (sum >> 16) + (sum & 0xFFFF);
                        }
                        sum = (sum >> 16) + (sum & 0xFFFF);
                        sum += img_size as u32;
                        eprintln!("[HANG-TRAP] ComputedChecksum={:08X}", sum);

                        // Dump first 256 bytes of the image
                        eprint!("[HANG-TRAP] First 64 bytes: ");
                        for i in 0..64u64 {
                            eprint!("{:02X} ", rd8h(mmu, cr3, base_va + i, memory));
                        }
                        eprintln!();

                        // Compare with ISO: find ntdll.dll on the ISO
                        // Dump section headers
                        let num_sections = rd16h(mmu, cr3, base_va + pe_off + 6, memory);
                        let opt_size = rd16h(mmu, cr3, base_va + pe_off + 20, memory);
                        let sec_start = pe_off + 24 + opt_size as u64;
                        eprintln!("[HANG-TRAP] Sections: {} at offset {:X}", num_sections, sec_start);
                        for s in 0..num_sections.min(8) {
                            let so = sec_start + s as u64 * 40;
                            let vsize = rd32h(mmu, cr3, base_va + so + 8, memory);
                            let vaddr = rd32h(mmu, cr3, base_va + so + 12, memory);
                            let rawsz = rd32h(mmu, cr3, base_va + so + 16, memory);
                            let rawptr = rd32h(mmu, cr3, base_va + so + 20, memory);
                            let mut name = [0u8; 8];
                            for k in 0..8 { name[k] = rd8h(mmu, cr3, base_va + so + k as u64, memory); }
                            eprintln!("[HANG-TRAP]  sec{}: name={} VA={:08X} VS={:08X} Raw={:08X} RS={:08X}",
                                s, core::str::from_utf8(&name).unwrap_or("?"), vaddr, vsize, rawptr, rawsz);
                        }
                    }
                }
                eprintln!("[HANG-TRAP] Last non-PF exceptions:");
                let n = self.exc_ring_nopf.len();
                for i in 0..n.min(8) {
                    let idx = (self.exc_ring_nopf_idx + n - 1 - i) % n;
                    let (rip, vec, ec, cr2_v) = self.exc_ring_nopf[idx];
                    if rip == 0 && vec == 0 { continue; }
                    eprintln!("[HANG-TRAP]  [{}] vec={} err={:08X} RIP={:08X} CR2={:08X}", i, vec, ec, rip as u32, cr2_v as u32);
                }
            } // inner: EB FE check
            } // outer: prev_exec_rip == rip

            // ── Bugcheck halt trap ──
            #[cfg(feature = "host_test")]
            if self.prev_exec_rip == self.regs.rip && self.regs.rip >= 0x80000000 {
                use crate::memory::MemoryBus;
                fn trl(mmu: &Mmu, cr3: u64, va: u64, mem: &GuestMemory) -> u64 {
                    mmu.translate_linear(va, cr3, crate::memory::AccessType::Read, 0, mem).unwrap_or(0)
                }
                fn rd8(mmu: &Mmu, cr3: u64, va: u64, mem: &GuestMemory) -> u8 {
                    mem.read_u8(trl(mmu, cr3, va, mem)).unwrap_or(0)
                }
                fn rd16(mmu: &Mmu, cr3: u64, va: u64, mem: &GuestMemory) -> u16 {
                    mem.read_u16(trl(mmu, cr3, va, mem)).unwrap_or(0)
                }
                fn rd32(mmu: &Mmu, cr3: u64, va: u64, mem: &GuestMemory) -> u32 {
                    mem.read_u32(trl(mmu, cr3, va, mem)).unwrap_or(0)
                }

                let cr3 = self.regs.cr3;
                eprintln!("[BUGCHECK-TRAP] First hit! ic={} from prev_rip={:08X}", self.instruction_count, self.prev_exec_rip);
                eprintln!("[BUGCHECK-TRAP] EAX={:08X} EBX={:08X} ECX={:08X} EDX={:08X}",
                    self.regs.gpr[0] as u32, self.regs.gpr[3] as u32,
                    self.regs.gpr[1] as u32, self.regs.gpr[2] as u32);
                eprintln!("[BUGCHECK-TRAP] ESP={:08X} EBP={:08X} ESI={:08X} EDI={:08X}",
                    self.regs.gpr[4] as u32, self.regs.gpr[5] as u32,
                    self.regs.gpr[6] as u32, self.regs.gpr[7] as u32);

                // Walk EBP chain to find KeBugCheckEx args (0x4C, C0000221, ...)
                // We scan each frame's args looking for the pattern.
                let mut bugcheck_code = 0u32;
                let mut param1 = 0u32;
                let mut param2 = 0u32;
                let mut param3 = 0u32;
                {
                    let mut ebp_scan = self.regs.gpr[5];
                    for _ in 0..15u32 {
                        let a0 = rd32(mmu, cr3, ebp_scan + 8, memory);
                        let a1 = rd32(mmu, cr3, ebp_scan + 12, memory);
                        let a2 = rd32(mmu, cr3, ebp_scan + 16, memory);
                        let a3 = rd32(mmu, cr3, ebp_scan + 20, memory);
                        if a0 == 0x4C && a1 == 0xC0000221 {
                            bugcheck_code = a0;
                            param1 = a1;
                            param2 = a2;
                            param3 = a3;
                            break;
                        }
                        let next = rd32(mmu, cr3, ebp_scan, memory) as u64;
                        if next == 0 || next < 0x80000000 { break; }
                        ebp_scan = next;
                    }
                }
                eprintln!("[BUGCHECK-TRAP] BugCheck={:08X} P1={:08X} P2={:08X} P3={:08X}", bugcheck_code, param1, param2, param3);

                if bugcheck_code == 0x4C && param1 == 0xC0000221 {
                    // param2 = UNICODE_STRING ptr: { Length: u16, MaxLen: u16, Buffer: *u16 }
                    let ustr_addr = param2 as u64;
                    eprintln!("[BUGCHECK-TRAP] UNICODE_STRING @ {:08X}:", param2);
                    eprint!("[BUGCHECK-TRAP]  struct bytes: ");
                    for k in 0..8usize {
                        eprint!("{:02X} ", rd8(mmu, cr3, ustr_addr + k as u64, memory));
                    }
                    eprintln!();
                    let ustr_len = rd16(mmu, cr3, ustr_addr, memory) as usize;
                    let ustr_buf_ptr = rd32(mmu, cr3, ustr_addr + 4, memory) as u64;
                    eprintln!("[BUGCHECK-TRAP]  len={} buf_ptr={:08X}", ustr_len, ustr_buf_ptr as u32);

                    // Decode the buffer as UTF-16LE
                    let mut dll_name = alloc::string::String::new();
                    if ustr_buf_ptr > 0 && ustr_len > 0 && ustr_len < 1024 {
                        eprint!("[BUGCHECK-TRAP]  raw buf: ");
                        for k in 0..ustr_len.min(80) {
                            eprint!("{:02X} ", rd8(mmu, cr3, ustr_buf_ptr + k as u64, memory));
                        }
                        eprintln!();
                        for i in 0..(ustr_len / 2) {
                            let lo = rd8(mmu, cr3, ustr_buf_ptr + i as u64 * 2, memory);
                            let _hi = rd8(mmu, cr3, ustr_buf_ptr + i as u64 * 2 + 1, memory);
                            if lo == 0 { break; }
                            dll_name.push(lo as char);
                        }
                    }
                    eprintln!("[BUGCHECK-TRAP] DLL name: \"{}\"", dll_name);

                    // Try 808ED918 from frame9 as a potential UNICODE_STRING
                    let f9_ptr = 0x808ED918u64;
                    let f9_len = rd16(mmu, cr3, f9_ptr, memory) as usize;
                    let f9_buf = rd32(mmu, cr3, f9_ptr + 4, memory) as u64;
                    eprintln!("[BUGCHECK-TRAP] 808ED918: len={} buf={:08X}", f9_len, f9_buf as u32);
                    if f9_buf > 0 && f9_len > 0 && f9_len < 256 {
                        let mut name = alloc::string::String::new();
                        for i in 0..(f9_len / 2) {
                            let lo = rd8(mmu, cr3, f9_buf + i as u64 * 2, memory);
                            if lo == 0 { break; }
                            name.push(lo as char);
                        }
                        eprintln!("[BUGCHECK-TRAP]  808ED918 string: \"{}\"", name);
                    }

                    // The real problem: look at the VA range 0x40000 where the write fault happened.
                    // The crash log showed a write fault at VA=0x40000. This is the address where
                    // Windows is mapping the DLL. Let's dump the page table entries for that range.
                    eprintln!("[BUGCHECK-TRAP] Page table analysis for low VA range:");
                    for page_va in [0x34000u64, 0x38000, 0x3C000, 0x40000, 0x44000, 0x48000, 0x80000, 0xE0000, 0xE7000, 0xE8000, 0xEC000] {
                        match mmu.translate_linear(page_va, cr3, crate::memory::AccessType::Read, 0, memory) {
                            Ok(phys) => {
                                let b0 = memory.read_u8(phys).unwrap_or(0);
                                let b1 = memory.read_u8(phys + 1).unwrap_or(0);
                                eprintln!("[BUGCHECK-TRAP]  VA {:08X} -> PA {:08X} (first bytes: {:02X} {:02X})", page_va, phys, b0, b1);
                            }
                            Err(e) => {
                                eprintln!("[BUGCHECK-TRAP]  VA {:08X} -> FAULT {:?}", page_va, e);
                            }
                        }
                    }

                    // Check the area at 0x40000 more carefully - dump PDE and PTE raw values
                    eprintln!("[BUGCHECK-TRAP] Raw page table walk for VA 0x40000:");
                    let pde_idx = 0x40000u64 >> 22;
                    let pde_addr = (cr3 & 0xFFFFF000) + pde_idx * 4;
                    let pde = memory.read_u32(pde_addr).unwrap_or(0);
                    eprintln!("[BUGCHECK-TRAP]  CR3={:08X} PDE[{}]@{:08X} = {:08X}", cr3 as u32, pde_idx, pde_addr, pde);
                    if (pde & 1) != 0 {
                        let pt_base = pde & 0xFFFFF000;
                        let pte_idx = (0x40000u64 >> 12) & 0x3FF;
                        let pte_addr = pt_base as u64 + pte_idx * 4;
                        let pte = memory.read_u32(pte_addr).unwrap_or(0);
                        eprintln!("[BUGCHECK-TRAP]  PT@{:08X} PTE[{}]@{:08X} = {:08X} (P={} RW={} US={} A={} D={})",
                            pt_base, pte_idx, pte_addr, pte,
                            pte & 1, (pte >> 1) & 1, (pte >> 2) & 1, (pte >> 5) & 1, (pte >> 6) & 1);
                    }
                }

                // Walk EBP chain
                let mut ebp = self.regs.gpr[5];
                for frame in 0..12u32 {
                    let next = rd32(mmu, cr3, ebp, memory) as u64;
                    let ret = rd32(mmu, cr3, ebp + 4, memory);
                    eprint!("[BUGCHECK-TRAP] frame{}: EBP={:08X} RET={:08X} args:", frame, ebp as u32, ret);
                    for a in 0..6u64 {
                        let val = rd32(mmu, cr3, ebp + 8 + a * 4, memory);
                        eprint!(" {:08X}", val);
                    }
                    eprintln!();
                    if next == 0 || next < 0x80000000 { break; }
                    ebp = next;
                }
            }



            // ── Decode Cache path ──
            let cs_base = self.regs.seg[SegReg::Cs as usize].base;
            let block_key = BlockKey {
                phys_addr,
                mode: self.decoder.mode(),
                cs_base,
            };

            if let Some(cached_block) = self.decode_cache.lookup(&block_key) {
                let block_exit = self.execute_cached_block_chain(
                    block_key,
                    cached_block,
                    target,
                    memory,
                    mmu,
                    io,
                    interrupts,
                );
                match block_exit {
                    BlockExitReason::Continue => continue,
                    BlockExitReason::Exit(reason) => return reason,
                }
            }

            // Cache miss: try to detect and cache a full basic block.
            if let Ok(new_block) = block::detect_basic_block(
                &self.decoder, &*memory, phys_addr,
            ) {
                self.decode_cache.insert(block_key, new_block);
                let cached_block = self.decode_cache.lookup(&block_key).unwrap();

                let block_exit = self.execute_cached_block_chain(
                    block_key,
                    cached_block,
                    target,
                    memory,
                    mmu,
                    io,
                    interrupts,
                );
                match block_exit {
                    BlockExitReason::Continue => continue,
                    BlockExitReason::Exit(reason) => return reason,
                }
            }

            // ── Fallback: single instruction decode + execute ──────
            // This path handles decode errors (logs diagnostics, injects #UD).
            let inst = match self.decoder.decode(&*memory, phys_addr) {
                Ok(inst) => inst,
                Err(VmError::FetchFault(_addr)) => {
                    // The instruction may straddle a page boundary.  Build a
                    // cross-page decode buffer using MMU translation for each
                    // byte so that the second page is correctly resolved.
                    let page_off = (phys_addr & 0xFFF) as usize;
                    if page_off > 0x1000 - crate::decoder::MAX_INST_LEN {
                        match self.decode_cross_page(fetch_addr, phys_addr, memory, mmu) {
                            Ok(inst) => inst,
                            Err(_) => {
                                let pf = VmError::PageFault {
                                    address: fetch_addr + (0x1000 - (fetch_addr & 0xFFF)),
                                    error_code: 0x10,
                                };
                                if let Err(e2) =
                                    self.inject_exception_from_error(&pf, memory, mmu, interrupts)
                                {
                                    return ExitReason::Exception(e2);
                                }
                                continue;
                            }
                        }
                    } else {
                        let pf = VmError::PageFault {
                            address: fetch_addr,
                            error_code: 0x10, // instruction fetch
                        };
                        if let Err(e2) =
                            self.inject_exception_from_error(&pf, memory, mmu, interrupts)
                        {
                            return ExitReason::Exception(e2);
                        }
                        continue;
                    }
                }
                Err(ref _decode_err) => {
                    // Log the raw bytes at the faulting IP for diagnostics.
                    use crate::memory::MemoryBus;
                    let b0 = memory.read_u8(phys_addr).unwrap_or(0xFF);
                    let b1 = memory.read_u8(phys_addr + 1).unwrap_or(0xFF);
                    let b2 = memory.read_u8(phys_addr + 2).unwrap_or(0xFF);
                    let b3 = memory.read_u8(phys_addr + 3).unwrap_or(0xFF);
                    let b4 = memory.read_u8(phys_addr + 4).unwrap_or(0xFF);
                    let b5 = memory.read_u8(phys_addr + 5).unwrap_or(0xFF);
                    libsyscall::serial_print(format_args!(
                        "[corevm] #UD at CS:IP={:04X}:{:X} phys={:X} bytes=[{:02X} {:02X} {:02X} {:02X} {:02X} {:02X}]\n",
                        self.regs.seg[SegReg::Cs as usize].selector,
                        self.regs.rip, phys_addr,
                        b0, b1, b2, b3, b4, b5,
                    ));
                    let ud = VmError::UndefinedOpcode(b0);
                    if let Err(e2) =
                        self.inject_exception_from_error(&ud, memory, mmu, interrupts)
                    {
                        return ExitReason::Exception(e2);
                    }
                    continue;
                }
            };

            self.prev_opcode = self.last_opcode;
            self.last_opcode = inst.opcode;

            // Execute the decoded instruction
            match crate::executor::execute(self, &inst, memory, mmu, io, interrupts) {
                Ok(()) => {
                    self.instruction_count += 1;
                    self.mask_rip();
                }
                Err(VmError::Halted) => {
                    self.instruction_count += 1;
                    return ExitReason::Halted;
                }
                Err(VmError::Breakpoint) => {
                    self.instruction_count += 1;
                    return ExitReason::Breakpoint;
                }
                Err(ref e) => {
                    // Detect infinite exception loops without killing legitimate
                    // demand-paging sequences that fault on the same instruction
                    // across different pages (common in Windows memset/stos paths).
                    self.note_exception_repeat(e);
                    // Log only the first few occurrences (avoid flooding serial).
                    if self.consecutive_exception_count <= 3 {
                        use crate::memory::MemoryBus;
                        let b0 = memory.read_u8(phys_addr).unwrap_or(0xFF);
                        let b1 = memory.read_u8(phys_addr + 1).unwrap_or(0xFF);
                        let b2 = memory.read_u8(phys_addr + 2).unwrap_or(0xFF);
                        let b3 = memory.read_u8(phys_addr + 3).unwrap_or(0xFF);
                        libsyscall::serial_print(format_args!(
                            "[corevm] exec error at CS:IP={:04X}:{:X} phys={:X} opcode=0x{:04X} bytes=[{:02X} {:02X} {:02X} {:02X}] modrm_reg={} CS.base={:X}: {:?}\n",
                            self.regs.seg[SegReg::Cs as usize].selector,
                            self.last_exec_rip,
                            phys_addr,
                            inst.opcode,
                            b0, b1, b2, b3,
                            inst.modrm_reg(),
                            self.regs.seg[SegReg::Cs as usize].base,
                            e
                        ));
                        libsyscall::serial_print(format_args!(
                            "[corevm]  regs: EAX={:08X} EBX={:08X} ECX={:08X} EDX={:08X} ESP={:08X} EBP={:08X} ESI={:08X} EDI={:08X}\n",
                            self.regs.gpr[0] as u32, self.regs.gpr[3] as u32,
                            self.regs.gpr[1] as u32, self.regs.gpr[2] as u32,
                            self.regs.sp() as u32, self.regs.gpr[5] as u32,
                            self.regs.gpr[6] as u32, self.regs.gpr[7] as u32,
                        ));
                        libsyscall::serial_print(format_args!(
                            "[corevm]  CR0={:08X} CR2={:08X} CR3={:08X} CR4={:08X} EFLAGS={:08X} IDTR={:X}:{:04X} ic={}\n",
                            self.regs.cr0 as u32, self.regs.cr2 as u32,
                            self.regs.cr3 as u32, self.regs.cr4 as u32,
                            self.regs.rflags as u32,
                            self.regs.idtr.base, self.regs.idtr.limit,
                            self.instruction_count,
                        ));
                        // Show the SS descriptor and stack content for debugging
                        let ss = &self.regs.seg[SegReg::Ss as usize];
                        libsyscall::serial_print(format_args!(
                            "[corevm]  SS={:04X} (base={:X}) DS={:04X} (base={:X})\n",
                            ss.selector, ss.base,
                            self.regs.seg[SegReg::Ds as usize].selector,
                            self.regs.seg[SegReg::Ds as usize].base,
                        ));
                    }
                    // Triple-fault: if the same RIP keeps faulting, the guest is stuck.
                    if self.consecutive_exception_count > 20 {
                        libsyscall::serial_print(format_args!(
                            "[corevm] triple fault: exception loop at CS:IP={:04X}:{:X} ({} repeats) CR2={:X} ESP={:X} err={:?}\n",
                            self.regs.seg[SegReg::Cs as usize].selector,
                            self.last_exec_rip,
                            self.consecutive_exception_count,
                            self.regs.cr2,
                            self.regs.sp(),
                            e,
                        ));
                        return ExitReason::Exception(VmError::DoubleFault);
                    }
                    match self.inject_exception_from_error(e, memory, mmu, interrupts) {
                        Err(ref e2) => {
                            libsyscall::serial_print(format_args!(
                                "[corevm] exception delivery failed: {:?}\n", e2
                            ));
                            return ExitReason::Exception(*e2);
                        }
                        Ok(()) => {
                            // Exception delivered — loop will re-enter at handler.
                        }
                    }
                }
            }
        }
    }

    /// Drain self-modifying-code dirty page markers and invalidate caches.
    #[inline(always)]
    fn drain_smc_invalidations(&mut self) {
        // Fast-path: check dirty count without draining.
        if !crate::memory::smc::has_dirty() {
            return;
        }
        let mut pages_buf = [0u64; 256];
        let smc_result = crate::memory::smc::drain_to_buf(&mut pages_buf);
        match smc_result {
            crate::memory::smc::DrainResult::None => {}
            crate::memory::smc::DrainResult::Pages(n) => {
                for page in pages_buf.iter().take(n) {
                    self.decode_cache.invalidate_page(*page);
                    self.jit_session.invalidate_page(*page);
                }
                self.smc_pending = true;
            }
            crate::memory::smc::DrainResult::Overflow => {
                self.decode_cache.flush();
                self.jit_session.flush();
                self.smc_pending = true;
            }
        }
    }

    /// Execute one cached block and keep chaining into subsequent cached blocks
    /// while it is safe to do so.
    fn execute_cached_block_chain(
        &mut self,
        mut block_key: BlockKey,
        mut block: alloc::sync::Arc<crate::jit::block::BasicBlock>,
        target: u64,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        io: &mut IoDispatch,
        interrupts: &mut InterruptController,
    ) -> BlockExitReason {
        loop {
            let block_exit = self.execute_cached_block(
                &block.instructions,
                block_key.phys_addr,
                memory,
                mmu,
                io,
                interrupts,
            );
            match block_exit {
                BlockExitReason::Continue => {}
                BlockExitReason::Exit(reason) => return BlockExitReason::Exit(reason),
            }

            if target > 0 && self.instruction_count >= target {
                return BlockExitReason::Exit(ExitReason::InstructionLimit);
            }

            if block.exits_with_branch {
                return BlockExitReason::Continue;
            }

            // Give pending interrupts a chance before chaining further blocks.
            if interrupts.pending_interrupt(self.regs.rflags).is_some() || interrupts.interrupt_shadow {
                return BlockExitReason::Continue;
            }

            self.drain_smc_invalidations();

            let cs = &self.regs.seg[SegReg::Cs as usize];
            let fetch_addr = if self.a20_enabled {
                cs.base.wrapping_add(self.regs.rip)
            } else {
                cs.base.wrapping_add(self.regs.rip) & !0x10_0000
            };
            let next_phys = match mmu.translate_linear(
                fetch_addr,
                self.regs.cr3,
                AccessType::Execute,
                self.regs.cpl,
                &*memory,
            ) {
                Ok(p) => p,
                Err(_) => return BlockExitReason::Continue,
            };

            let next_key = BlockKey {
                phys_addr: next_phys,
                mode: self.decoder.mode(),
                cs_base: cs.base,
            };

            let Some(next_block) = self.decode_cache.lookup(&next_key) else {
                return BlockExitReason::Continue;
            };
            block = next_block;
            block_key = next_key;
        }
    }

    /// Execute a cached basic block (pre-decoded instruction sequence).
    ///
    /// Iterates through the block's instructions, executing each one and
    /// handling errors identically to the single-instruction path. Returns
    /// a `BlockExitReason` indicating whether the main loop should continue
    /// or return an `ExitReason` to the caller.
    fn execute_cached_block(
        &mut self,
        instructions: &[crate::instruction::DecodedInst],
        block_phys: u64,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        io: &mut IoDispatch,
        interrupts: &mut InterruptController,
    ) -> BlockExitReason {
        let mut inst_phys = block_phys;
        for inst in instructions {
            mmu.update_from_regs(self.regs.cr0, self.regs.cr4, self.regs.efer);
            mmu.rflags_ac = (self.regs.rflags & crate::flags::AC) != 0;
            crate::poll_external_irqs(interrupts, self.regs.rflags);
            if let Some(vector) = interrupts.pending_interrupt(self.regs.rflags) {
                interrupts.acknowledge(vector);
                if let Err(e) = self.deliver_interrupt_hw(vector, memory, mmu, interrupts) {
                    return BlockExitReason::Exit(ExitReason::Exception(e));
                }
                interrupts.interrupt_shadow = false;
                return BlockExitReason::Continue;
            }
            interrupts.interrupt_shadow = false;

            self.last_exec_rip = self.regs.rip;
            self.last_exec_cs = self.regs.seg[SegReg::Cs as usize].selector;
            self.last_fetch_addr = inst_phys;
            self.prev_opcode = self.last_opcode;
            self.last_opcode = inst.opcode;

            match crate::executor::execute(self, inst, memory, mmu, io, interrupts) {
                Ok(()) => {
                    self.instruction_count += 1;
                    self.mask_rip();
                }
                Err(VmError::Halted) => {
                    self.instruction_count += 1;
                    return BlockExitReason::Exit(ExitReason::Halted);
                }
                Err(VmError::Breakpoint) => {
                    self.instruction_count += 1;
                    return BlockExitReason::Exit(ExitReason::Breakpoint);
                }
                Err(ref e) => {
                    self.note_exception_repeat(e);
                    #[cfg(feature = "host_test")]
                    if self.consecutive_exception_count <= 3 {
                        use crate::memory::MemoryBus;
                        let b0 = memory.read_u8(inst_phys).unwrap_or(0xFF);
                        let b1 = memory.read_u8(inst_phys + 1).unwrap_or(0xFF);
                        let b2 = memory.read_u8(inst_phys + 2).unwrap_or(0xFF);
                        let b3 = memory.read_u8(inst_phys + 3).unwrap_or(0xFF);
                        eprintln!(
                            "[corevm] cached-block error at CS:IP={:04X}:{:X} phys={:X} opcode=0x{:04X} bytes=[{:02X} {:02X} {:02X} {:02X}] err={:?}",
                            self.regs.seg[SegReg::Cs as usize].selector,
                            self.last_exec_rip,
                            inst_phys,
                            inst.opcode,
                            b0,
                            b1,
                            b2,
                            b3,
                            e,
                        );
                        eprintln!(
                            "[corevm]  regs: EAX={:08X} EBX={:08X} ECX={:08X} EDX={:08X} ESP={:08X} EBP={:08X} ESI={:08X} EDI={:08X} CR2={:08X} CR3={:08X} EFLAGS={:08X}",
                            self.regs.gpr[0] as u32,
                            self.regs.gpr[3] as u32,
                            self.regs.gpr[1] as u32,
                            self.regs.gpr[2] as u32,
                            self.regs.sp() as u32,
                            self.regs.gpr[5] as u32,
                            self.regs.gpr[6] as u32,
                            self.regs.gpr[7] as u32,
                            self.regs.cr2 as u32,
                            self.regs.cr3 as u32,
                            self.regs.rflags as u32,
                        );
                    }
                    if self.consecutive_exception_count > 20 {
                        #[cfg(feature = "host_test")]
                        eprintln!("[corevm] exception loop: CS:IP={:04X}:{:X} ({} repeats) CR2={:X} ESP={:X} err={:?}",
                            self.regs.seg[SegReg::Cs as usize].selector,
                            self.last_exec_rip, self.consecutive_exception_count,
                            self.regs.cr2, self.regs.sp(), e);
                        return BlockExitReason::Exit(ExitReason::Exception(VmError::DoubleFault));
                    }
                    if let Err(e2) =
                        self.inject_exception_from_error(e, memory, mmu, interrupts)
                    {
                        return BlockExitReason::Exit(ExitReason::Exception(e2));
                    }
                    // Exception injected — break out of block, let main loop
                    // re-enter at the exception handler address.
                    return BlockExitReason::Continue;
                }
            }

            inst_phys += inst.length as u64;

            // Check for stop request between instructions within the block.
            if self.stop_requested {
                self.stop_requested = false;
                return BlockExitReason::Exit(ExitReason::StopRequested);
            }
        }
        BlockExitReason::Continue
    }

    /// Compile (if needed) and execute a JIT-compiled basic block.
    ///
    /// Looks up the compiled block in the JIT engine's code cache. On miss,
    /// compiles the decoded block and stores the native code. Then executes
    /// the compiled code via function pointer call.
    ///
    /// The JIT block function follows the C calling convention:
    /// `fn(cpu, memory, mmu, io, interrupts) -> u32`
    ///
    /// Chain JIT blocks: after executing one block, try to chain directly
    /// to the next without going through the full run() loop.
    fn run_jit_session(
        &mut self,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        io: &mut IoDispatch,
        interrupts: &mut InterruptController,
        target: u64,
    ) -> ExitReason {
        use crate::jit::session::*;

        // Ensure dispatcher is emitted
        if self.jit_session.dispatch_loop_offset == 0 {
            self.jit_session.emit_dispatcher();
            self.jit_session.buffer.make_executable();
        }

        loop {
            mmu.update_from_regs(self.regs.cr0, self.regs.cr4, self.regs.efer);
            mmu.rflags_ac = (self.regs.rflags & crate::flags::AC) != 0;

            let reason = unsafe {
                let func = self.jit_session.dispatcher_fn();
                func(
                    self as *mut Cpu as *mut u8,
                    memory as *mut GuestMemory as *mut u8,
                    mmu as *mut Mmu as *mut u8,
                    io as *mut IoDispatch as *mut u8,
                    interrupts as *mut InterruptController as *mut u8,
                    target,
                )
            };

            #[cfg(feature = "host_test")]
            {
                static JIT_LOOP: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
                let n = JIT_LOOP.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                if n % 100_000 == 0 && n <= 500_000 {
                    let cs = self.regs.seg[crate::registers::SegReg::Cs as usize].selector;
                    eprintln!("[jit-loop] n={} reason={} ic={} target={} cs:rip={:04x}:{:04x}",
                        n, reason, self.instruction_count, target, cs, self.regs.rip as u16);
                }
            }

            match reason {
                EXIT_NEEDS_COMPILE => {
                    self.jit_compile_pending(memory, mmu, io, interrupts);
                }
                EXIT_INTERRUPT => {
                    if let Some(vector) = interrupts.pending_interrupt(self.regs.rflags) {
                        interrupts.acknowledge(vector);
                        if let Err(e) = self.deliver_interrupt_hw(vector, memory, mmu, interrupts) {
                            return ExitReason::Exception(e);
                        }
                    }
                    interrupts.interrupt_shadow = false;
                }
                EXIT_SMC => {
                    self.smc_pending = false;
                    self.drain_smc_invalidations();
                }
                EXIT_FAULT => {
                    self.jit_fault = false;
                    // Fall through — will re-enter dispatcher which recomputes phys addr
                }
                EXIT_LIMIT => return ExitReason::InstructionLimit,
                EXIT_STOP => {
                    self.stop_requested = false;
                    return ExitReason::StopRequested;
                }
                EXIT_HALT => {
                    self.jit_halted = false;
                    return ExitReason::Halted;
                }
                _ => return ExitReason::Halted,
            }
        }
    }

    fn jit_compile_pending(&mut self, memory: &mut GuestMemory, mmu: &mut Mmu, io: &mut IoDispatch, interrupts: &mut InterruptController) {
        use crate::jit::block::{BasicBlock, BlockKey, detect_basic_block};
        use crate::decoder::CpuMode;

        let phys = self.pending_compile_phys;
        let mode = match self.pending_compile_mode {
            0 => CpuMode::Real16,
            1 => CpuMode::Protected32,
            _ => CpuMode::Long64,
        };
        let cs_base = self.pending_compile_cs_base;
        let key = BlockKey { phys_addr: phys, mode, cs_base };

        // Get or detect basic block
        let block = if let Some(cached) = self.decode_cache.lookup(&key) {
            cached
        } else if let Ok(new_block) = detect_basic_block(&self.decoder, &*memory, phys) {
            self.decode_cache.insert(key, new_block);
            match self.decode_cache.lookup(&key) {
                Some(b) => b,
                None => {
                    // Fallback: interpret one instruction to make progress
                    self.interpret_one(memory, mmu, io, interrupts);
                    return;
                }
            }
        } else {
            // Cannot decode block — interpret one instruction to avoid stuck loop
            self.interpret_one(memory, mmu, io, interrupts);
            return;
        };

        let inst_ptrs: alloc::vec::Vec<*const crate::instruction::DecodedInst> =
            block.instructions.iter().map(|i| i as *const _).collect();

        let bb = BasicBlock {
            instructions: block.instructions.clone(),
            byte_len: block.byte_len,
            exits_with_branch: block.exits_with_branch,
        };

        let dispatch_loop_addr = self.jit_session.dispatch_loop_ptr();

        let compiled = self.jit_session.translator.translate_block(
            &bb, &inst_ptrs, phys, mode, dispatch_loop_addr,
        );

        self.jit_session.buffer.make_writable();
        if let Some(code_offset) = self.jit_session.buffer.emit(&compiled.code) {
            let code_ptr = unsafe { self.jit_session.buffer.code_ptr(code_offset) };
            if self.jit_session.lookup.insert(phys, mode as u8, cs_base, code_ptr as u64) {
                let page = phys & !0xFFF;
                self.jit_session.code_pages.insert(page);
                crate::memory::smc::mark_code_page(page);
                self.jit_session.blocks_compiled += 1;
            } else {
                // Hashtable full at this bucket — interpret to make progress
                self.jit_session.buffer.make_executable();
                self.interpret_one(memory, mmu, io, interrupts);
                return;
            }
        } else {
            // Buffer full — interpret to make progress
            self.interpret_one(memory, mmu, io, interrupts);
            return;
        }
        self.jit_session.buffer.make_executable();
    }

    /// Interpret a single instruction at the current CS:RIP (fallback for JIT).
    fn interpret_one(
        &mut self,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        io: &mut IoDispatch,
        interrupts: &mut InterruptController,
    ) {
        let cs_base = self.regs.seg[SegReg::Cs as usize].base;
        let linear = cs_base.wrapping_add(self.regs.rip);
        let phys = match mmu.translate_linear(
            linear, self.regs.cr3, AccessType::Execute, self.regs.cpl, memory,
        ) {
            Ok(p) => p,
            Err(_) => return,
        };
        let inst = match self.decoder.decode(&*memory, phys) {
            Ok(i) => i,
            Err(_) => return,
        };
        match crate::executor::execute(self, &inst, memory, mmu, io, interrupts) {
            Ok(()) | Err(VmError::Halted) => {
                self.instruction_count += 1;
                self.mask_rip();
            }
            Err(_) => {}
        }
    }


    /// Inject an exception derived from a VmError into the guest.
    fn inject_exception_from_error(
        &mut self,
        error: &VmError,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        interrupts: &mut InterruptController,
    ) -> Result<()> {
        // Fix up MMIO page faults: Windows HAL accesses LAPIC and I/O APIC
        // MMIO regions before the HAL's mapping code has run, causing infinite
        // #PF recursion (MmAccessFault crashes because MmPfnDatabase is NULL).
        // Detect these and write the PDE/PTE to create the mapping, then
        // re-execute the faulting instruction.
        if let VmError::PageFault { address, .. } = error {
            let va = *address;
            if self.mode == Mode::ProtectedMode
                && (self.regs.cr0 & CR0_PG) != 0
                && !mmu.pae
            {
                // Determine if this VA should map to a known MMIO physical address.
                let mmio_phys: Option<u64> = if va >= 0xFFFE_0000 && va < 0xFFFE_1000 {
                    // Local APIC: VA 0xFFFE0xxx → APIC base from MSR
                    let apic_base = self.regs.read_msr(0x1B) & 0xFFFF_F000;
                    if apic_base != 0 { Some(apic_base) } else { None }
                } else {
                    None
                };

                if let Some(phys) = mmio_phys {
                    let cr3 = self.regs.cr3 & 0xFFFF_F000;
                    let pde_idx = (va >> 22) & 0x3FF;
                    let pde_addr = cr3 + pde_idx * 4;
                    let mut pde = memory.read_u32(pde_addr).unwrap_or(0);

                    // If PDE is not present, allocate a page table page.
                    if pde & 1 == 0 {
                        // Use a scratch page from end of RAM for the page table.
                        // Each PDE index gets its own page to avoid conflicts.
                        let ram_size = memory.ram_size() as u64;
                        let pt_page = (ram_size & 0xFFFF_F000) - 0x1000 * (1024 - pde_idx);
                        if pt_page < ram_size {
                            // Zero the page table page
                            for i in 0..1024 {
                                memory.fast_write_u32(pt_page + i * 4, 0);
                            }
                            // Write PDE: present + read/write + user
                            pde = (pt_page as u32) | 0x23;
                            memory.fast_write_u32(pde_addr, pde);
                        }
                    }

                    if pde & 1 != 0 {
                        let pt_base = (pde as u64) & 0xFFFF_F000;
                        let pte_idx = (va >> 12) & 0x3FF;
                        let pte_addr = pt_base + pte_idx * 4;
                        let pte = memory.read_u32(pte_addr).unwrap_or(0);
                        if pte & 1 == 0 {
                            // Map VA → MMIO with P|RW|PWT|PCD
                            let new_pte = (phys as u32) | 0x1B;
                            memory.fast_write_u32(pte_addr, new_pte);
                            mmu.flush_tlb();
                            return Ok(());
                        }
                    }
                }
            }
        }


        let (vector, error_code, cr2_val) = match error {
            VmError::DivideByZero => (0, None, None),
            VmError::DebugException => (1, None, None),
            VmError::Breakpoint => (3, None, None),
            VmError::Overflow => (4, None, None),
            VmError::BoundRange => (5, None, None),
            VmError::UndefinedOpcode(op) => {
                #[cfg(feature = "host_test")]
                {
                    let rip = self.regs.rip;
                    let phys = mmu.translate_linear(rip, self.regs.cr3, crate::memory::AccessType::Read, 0, memory).unwrap_or(rip);
                    let mut bytes = [0u8; 16];
                    for i in 0..16u64 {
                        bytes[i as usize] = memory.read_u8(phys.wrapping_add(i)).unwrap_or(0xFF);
                    }
                    eprintln!("[#UD] RIP={:08X} op=0x{:02X} bytes={:02X?}", rip, op, bytes);
                }
                (6, None, None)
            }
            VmError::DoubleFault => (8, Some(0u32), None),
            VmError::InvalidTss(ec) => (10, Some(*ec), None),
            VmError::SegmentNotPresent(ec) => (11, Some(*ec), None),
            VmError::StackFault(ec) => (12, Some(*ec), None),
            VmError::GeneralProtection(ec) => (13, Some(*ec), None),
            VmError::PageFault {
                address,
                error_code,
            } => (14, Some(*error_code), Some(*address)),
            VmError::FpuError => (16, None, None),
            VmError::AlignmentCheck => (17, Some(0u32), None),
            VmError::SimdException => (19, None, None),
            // Non-exception errors cannot be injected
            _ => return Err(*error),
        };

        // Record in exception ring buffer for BSOD diagnosis.
        {
            let idx = self.exc_ring_idx % self.exc_ring.len();
            self.exc_ring[idx] = (
                self.regs.rip,
                vector,
                error_code.unwrap_or(0),
                cr2_val.unwrap_or(0),
            );
            self.exc_ring_idx += 1;
            if vector != 14 {
                let nidx = self.exc_ring_nopf_idx % self.exc_ring_nopf.len();
                self.exc_ring_nopf[nidx] = (
                    self.regs.rip,
                    vector,
                    error_code.unwrap_or(0),
                    cr2_val.unwrap_or(0),
                );
                self.exc_ring_nopf_idx += 1;
            }
        }

        if let Some(addr) = cr2_val {
            self.regs.cr2 = addr;
        }

        // ── Debugger hook: exception breakpoints ──
        #[cfg(feature = "host_test")]
        if crate::debugger::on_exception(vector, error_code, cr2_val.unwrap_or(0), self) {
            crate::debugger::enter_prompt(self, memory, &*mmu, 0,
                &alloc::format!("exception #{} err={:?} CR2={:#010x}",
                    vector, error_code, cr2_val.unwrap_or(0)));
        }

        // Double fault detection (Intel SDM Vol. 3A §6.15):
        // Contributory exceptions: #DE(0), #TS(10), #NP(11), #SS(12), #GP(13).
        // #PF(14) is special: contributory-during-#PF or #PF-during-#PF → #DF.
        // #DF(8), #AC(17) are NOT contributory.
        let is_contributory = matches!(vector, 0 | 10 | 11 | 12 | 13 | 14);
        if interrupts.handling_exception && is_contributory {
            if vector == 8 || interrupts.handling_double_fault {
                // Triple fault: #DF during #DF handling → shutdown
                #[cfg(feature = "host_test")]
                eprintln!("[corevm] TRIPLE FAULT: vec={} at CS:IP={:#06x}:{:#010x} CR2={:#010x}",
                    vector, self.regs.seg[SegReg::Cs as usize].selector, self.regs.rip, self.regs.cr2);
                return Err(crate::error::VmError::Shutdown);
            }
            #[cfg(feature = "host_test")]
            eprintln!("[corevm] DOUBLE FAULT (handling): vec={} err={:?} at CS:IP={:#06x}:{:#010x} CR2={:#010x} ESP={:#010x}",
                vector, error_code, self.regs.seg[SegReg::Cs as usize].selector, self.regs.rip, self.regs.cr2, self.regs.sp());
            interrupts.handling_exception = false;
            interrupts.handling_double_fault = true;
            let result = self.deliver_interrupt(8, true, Some(0), memory, mmu, interrupts);
            interrupts.handling_double_fault = false;
            return result;
        }
        interrupts.handling_exception = is_contributory;
        let orig_cr2 = self.regs.cr2;
        #[cfg(feature = "host_test")]
        let orig_esp = self.regs.sp();
        #[cfg(feature = "host_test")]
        let orig_rip = self.regs.rip;

        // Intel SDM Vol. 3A §6.8.3: For fault-type exceptions (#DE, #DB, #UD,
        // #GP, #PF, etc.), RF is set in the EFLAGS image pushed on the stack
        // so that IRET returns with RF=1, preventing re-triggering for #DB.
        // For trap-type exceptions (#BP, #OF) and interrupts, RF is cleared.
        let is_fault = matches!(vector, 0 | 1 | 5 | 6 | 7 | 10 | 11 | 12 | 13 | 14 | 17);
        if is_fault {
            self.regs.rflags |= crate::flags::RF;
        } else {
            self.regs.rflags &= !crate::flags::RF;
        }

        let result = self.deliver_interrupt(
            vector,
            error_code.is_some(),
            error_code,
            memory,
            mmu,
            interrupts,
        );

        // Clear RF after delivery — the pushed image already has it set/cleared.
        self.regs.rflags &= !crate::flags::RF;

        match result {
            Ok(()) => {
                interrupts.handling_exception = false;
                Ok(())
            }
            Err(e) if is_contributory => {
                // Exception delivery itself faulted — this is a double fault.
                #[cfg(feature = "host_test")]
                {
                    eprintln!("[corevm] DOUBLE FAULT (delivery failed): vec={} err={:?} orig_CR2={:#010x} orig_ESP={:#010x} orig_RIP={:#010x} delivery_err={} cur_ESP={:#010x} EAX={:#010x} EBX={:#010x} ECX={:#010x} EDX={:#010x}",
                        vector, error_code, orig_cr2, orig_esp, orig_rip, e, self.regs.sp(),
                        self.regs.gpr[0] as u32, self.regs.gpr[3] as u32, self.regs.gpr[1] as u32, self.regs.gpr[2] as u32);
                    // Dump instruction bytes at orig_RIP
                    if let Ok(phys) = mmu.translate_linear(orig_rip, self.regs.cr3, crate::memory::AccessType::Execute, 0, &*memory) {
                        use crate::memory::MemoryBus;
                        let b: [u8; 8] = core::array::from_fn(|i| memory.read_u8(phys + i as u64).unwrap_or(0xFF));
                        eprintln!("[corevm]   Instr @ {:#010x} phys={:#010x}: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                            orig_rip, phys, b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]);
                    }
                    // Full stack dump to see interrupt frames
                    {
                        let stack_va = orig_esp as u64;
                        let cr3v = self.regs.cr3;
                        if let Ok(stack_phys) = mmu.translate_linear(stack_va, cr3v, crate::memory::AccessType::Read, 0, &*memory) {
                            eprintln!("[corevm]   Stack dump at ESP={:#010x}:", stack_va);
                            for i in 0..20 {
                                let off = i as u64 * 4;
                                if let Ok(val) = memory.read_u32(stack_phys + off) {
                                    eprintln!("[corevm]     +{:02x}: {:#010x}", off, val);
                                }
                            }
                        }
                    }
                    // Check page table mapping for handler and stack
                    {
                        let cr3v = self.regs.cr3;
                        let pd_base = (cr3v & 0xFFFFF000) as u64;
                        // PDE[514] covers 0x80800000-0x80BFFFFF
                        let pde = memory.read_u32(pd_base + 514 * 4).unwrap_or(0);
                        let pt_base = (pde as u64) & 0xFFFFF000;
                        // PTE[4] = VA 0x80804xxx (stack), PTE[13] = VA 0x8080Dxxx (handler)
                        let pte4 = memory.read_u32(pt_base + 4 * 4).unwrap_or(0);
                        let pte13 = memory.read_u32(pt_base + 13 * 4).unwrap_or(0);
                        eprintln!("[corevm]   PDE[514]={:#010x} PT @ {:#010x}", pde, pt_base);
                        eprintln!("[corevm]   PTE[4](stack 0x80804)={:#010x}  PTE[13](handler 0x8080D)={:#010x}", pte4, pte13);
                        // Also dump handler code from PTE[13]'s physical page
                        let handler_phys_page = (pte13 as u64) & 0xFFFFF000;
                        let handler_offset = 0xD867u64 & 0xFFF; // offset within page
                        let handler_phys = handler_phys_page + handler_offset;
                        let mut s = alloc::string::String::new();
                        for i in 0..16 {
                            if let Ok(b) = memory.read_u8(handler_phys + i) {
                                use core::fmt::Write;
                                let _ = write!(s, "{:02x} ", b);
                            }
                        }
                        eprintln!("[corevm]   Handler actual phys={:#010x}: {}", handler_phys, s);
                    }
                    // Dump IDT entry for the faulting vector
                    let idtr_base = self.regs.idtr.base;
                    let idt_entry_addr = idtr_base + vector as u64 * 8;
                    if let Ok(idt_phys) = mmu.translate_linear(idt_entry_addr, self.regs.cr3, crate::memory::AccessType::Read, 0, &*memory) {
                        let lo = memory.read_u32(idt_phys).unwrap_or(0);
                        let hi = memory.read_u32(idt_phys + 4).unwrap_or(0);
                        let handler = (lo & 0xFFFF) | (hi & 0xFFFF0000);
                        let sel = (lo >> 16) & 0xFFFF;
                        let typ = (hi >> 8) & 0x1F;
                        eprintln!("[corevm]   IDT[{}]: handler={:#010x} sel={:#06x} type={:#x}",
                            vector, handler, sel, typ);
                    }
                    // Walk page table for both orig_esp and the failed push address
                    let cr3 = self.regs.cr3 as u32;
                    let pd_base = cr3 & 0xFFFFF000;
                    for &walk_addr in &[orig_esp as u32, self.regs.sp() as u32] {
                        let pd_idx = walk_addr >> 22;
                        let pde_phys = pd_base as u64 + pd_idx as u64 * 4;
                        if let Ok(pde) = memory.read_u32(pde_phys) {
                            eprintln!("[corevm]   PT walk {:#010x}: PD[{}]={:#010x}", walk_addr, pd_idx, pde);
                            if pde & 1 != 0 {
                                let pt_base = pde & 0xFFFFF000;
                                let pt_idx = (walk_addr >> 12) & 0x3FF;
                                let pte_phys = pt_base as u64 + pt_idx as u64 * 4;
                                if let Ok(pte) = memory.read_u32(pte_phys) {
                                    eprintln!("[corevm]   PTE[{}]={:#010x} flags={}", pt_idx, pte,
                                        if pte & 1 == 0 { "NOT_PRESENT" }
                                        else if pte & 2 == 0 { "READ_ONLY" }
                                        else { "RW" });
                                }
                            }
                        }
                    }
                }
                if interrupts.handling_double_fault {
                    #[cfg(feature = "host_test")]
                    eprintln!("[corevm] TRIPLE FAULT — shutting down");
                    return Err(crate::error::VmError::Shutdown);
                }
                interrupts.handling_exception = false;
                interrupts.handling_double_fault = true;
                let r = self.deliver_interrupt(8, true, Some(0), memory, mmu, interrupts);
                interrupts.handling_double_fault = false;
                r
            }
            Err(e) => {
                interrupts.handling_exception = false;
                Err(e)
            }
        }
    }

    /// Deliver an interrupt or exception to the guest CPU.
    ///
    /// Pushes the appropriate stack frame (flags, CS, IP/EIP/RIP, optional
    /// error code) and loads the handler address from the IVT/IDT.
    pub fn deliver_interrupt(
        &mut self,
        vector: u8,
        has_error_code: bool,
        error_code: Option<u32>,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        interrupts: &mut InterruptController,
    ) -> Result<()> {
        // Intel SDM Vol. 3A §6.14.2: In IA-32e mode (EFER.LMA=1), the processor
        // always uses 16-byte IDT gate descriptors and 64-bit interrupt delivery,
        // even when executing in compatibility mode (CS.L=0).
        let lma = (self.regs.read_msr(MSR_EFER) & EFER_LMA) != 0;
        if lma {
            self.deliver_interrupt_long(
                vector,
                has_error_code,
                error_code,
                memory,
                mmu,
                interrupts,
            )
        } else {
            match self.mode {
                Mode::RealMode => {
                    self.deliver_interrupt_real(vector, memory, mmu)
                }
                _ => {
                    self.deliver_interrupt_protected(
                        vector,
                        has_error_code,
                        error_code,
                        memory,
                        mmu,
                        interrupts,
                    )
                }
            }
        }
    }

    /// Deliver a hardware interrupt. Like `deliver_interrupt` but forces
    /// interrupt/trap gate semantics even if the IDT entry is a Task Gate
    /// when the vector is in the exception range (0-31). This prevents
    /// BIOS-era PIC vectors (IRQ0→vec 8) from hitting the OS's #DF Task Gate.
    pub fn deliver_interrupt_hw(
        &mut self,
        vector: u8,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        interrupts: &mut InterruptController,
    ) -> Result<()> {
        self.is_hw_interrupt = vector < 32;
        let r = self.deliver_interrupt(vector, false, None, memory, mmu, interrupts);
        self.is_hw_interrupt = false;
        r
    }

    /// Real-mode interrupt delivery: push FLAGS, CS, IP; load from IVT.
    fn deliver_interrupt_real(
        &mut self,
        vector: u8,
        memory: &mut GuestMemory,
        _mmu: &mut Mmu,
    ) -> Result<()> {
        use crate::flags::{IF, TF};
        use crate::memory::MemoryBus;

        // Read IVT entry: 4 bytes at vector * 4
        let ivt_addr = (vector as u64) * 4;
        let offset = memory.read_u16(ivt_addr)? as u64;
        let segment = memory.read_u16(ivt_addr + 2)?;

        // Push FLAGS (16-bit)
        let flags16 = (self.regs.rflags & 0xFFFF) as u16;
        let sp = self.regs.sp().wrapping_sub(2) & 0xFFFF;
        self.regs.set_sp(sp);
        let ss_base = self.regs.seg[SegReg::Ss as usize].base;
        memory.write_u16(ss_base + sp, flags16)?;

        // Push CS
        let cs_sel = self.regs.seg[SegReg::Cs as usize].selector;
        let sp = self.regs.sp().wrapping_sub(2) & 0xFFFF;
        self.regs.set_sp(sp);
        memory.write_u16(ss_base + sp, cs_sel)?;

        // Push IP
        let ip = (self.regs.rip & 0xFFFF) as u16;
        let sp = self.regs.sp().wrapping_sub(2) & 0xFFFF;
        self.regs.set_sp(sp);
        memory.write_u16(ss_base + sp, ip)?;

        // Clear IF and TF
        self.regs.rflags &= !(IF | TF);

        // Load new CS:IP
        self.regs.load_segment_real(SegReg::Cs, segment);
        self.regs.rip = offset;

        Ok(())
    }

    /// Protected-mode interrupt delivery via 32-bit IDT gate.
    fn deliver_interrupt_protected(
        &mut self,
        vector: u8,
        has_error_code: bool,
        error_code: Option<u32>,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        interrupts: &mut InterruptController,
    ) -> Result<()> {
        use crate::flags::{IF, TF};

        // IDT access during interrupt delivery is an implicit supervisor access.
        let entry = interrupts.read_idt_entry_protected(
            vector,
            self.regs.idtr.base,
            self.regs.idtr.limit,
            self.regs.cr3,
            0,
            mmu,
            &*memory,
        )?;

        if !entry.present {
            return Err(VmError::GeneralProtection((vector as u32) * 8 + 2));
        }

        // Task gate: perform a hardware task switch instead of stack-based delivery.
        if entry.gate_type == crate::interrupts::GateType::Task {
            // Drop hardware interrupts that hit a Task Gate in the exception range.
            // This happens when the PIC still uses BIOS defaults (IRQ0→vec 8) but
            // the OS IDT has a Task Gate for #DF at vector 8.
            if self.is_hw_interrupt {
                return Ok(());
            }
            #[cfg(feature = "host_test")]
            eprintln!("[corevm] TASK GATE for vec={} selector={:#06x} at CS:IP={:#06x}:{:#010x} ESP={:#010x}",
                vector, entry.selector, self.regs.seg[SegReg::Cs as usize].selector, self.regs.rip, self.regs.sp());
            return self.task_switch_32(
                entry.selector, has_error_code, error_code,
                memory, mmu,
            );
        }

        // Save old state
        let old_eflags = self.regs.rflags as u32;
        let old_cs = self.regs.seg[SegReg::Cs as usize].selector;
        let old_eip = self.regs.rip as u32;
        let old_ss = self.regs.seg[SegReg::Ss as usize].selector;
        let old_esp = self.regs.sp() as u32;
        let old_cpl = self.regs.cpl;
        let from_v86 = (self.regs.rflags & crate::flags::VM) != 0;

        // Save V86 segment registers before they get clobbered by stack switch.
        let old_gs = self.regs.seg[SegReg::Gs as usize].selector;
        let old_fs = self.regs.seg[SegReg::Fs as usize].selector;
        let old_ds = self.regs.seg[SegReg::Ds as usize].selector;
        let old_es = self.regs.seg[SegReg::Es as usize].selector;

        // Determine target CPL from target code segment descriptor.
        let target_cs_desc = self.read_gdt_descriptor(entry.selector, memory, mmu)?;
        let target_cpl = target_cs_desc.dpl;

        // V86 mode interrupts always switch to ring 0 via TSS, and clear VM.
        // Inter-privilege interrupt entry: switch stack via TSS ring stack.
        if from_v86 || target_cpl < old_cpl {
            let ring = if from_v86 { 0 } else { target_cpl };
            let (new_ss, new_esp) = self.read_tss_ring_stack32(ring, memory, mmu)?;

            // Clear VM flag before switching stack context.
            if from_v86 {
                self.regs.rflags &= !crate::flags::VM;
            }

            self.load_segment_from_gdt(SegReg::Ss, new_ss, memory, mmu)?;
            self.regs.set_sp(new_esp as u64);
            self.regs.cpl = target_cpl;
        }

        let ss_base = self.regs.seg[SegReg::Ss as usize].base;
        let gate_is_16 = matches!(
            entry.gate_type,
            crate::interrupts::GateType::Interrupt16 | crate::interrupts::GateType::Trap16
        );

        // Helper macro to push a 32-bit value onto the new stack.
        macro_rules! push32 {
            ($val:expr) => {{
                let esp = self.regs.sp().wrapping_sub(4);
                self.regs.set_sp(esp);
                let phys = mmu.translate_linear(
                    ss_base + esp,
                    self.regs.cr3,
                    AccessType::Write,
                    self.regs.cpl,
                    &*memory,
                )?;
                memory.write_u32(phys, $val)?;
            }};
        }

        // V86 interrupt frame: push GS, FS, DS, ES first, then SS:ESP.
        if from_v86 {
            push32!(old_gs as u32);
            push32!(old_fs as u32);
            push32!(old_ds as u32);
            push32!(old_es as u32);
            // Null out the data segment registers.
            // Null out the data segment registers (selector=0, not present).
            for seg in [SegReg::Gs, SegReg::Fs, SegReg::Ds, SegReg::Es] {
                let d = &mut self.regs.seg[seg as usize];
                d.selector = 0;
                d.base = 0;
                d.limit = 0;
                d.present = false;
            }
        }

        // Push old SS:ESP on privilege-level change or V86 exit.
        if from_v86 || target_cpl < old_cpl {
            if gate_is_16 {
                let esp = self.regs.sp().wrapping_sub(2);
                self.regs.set_sp(esp);
                let phys = mmu.translate_linear(
                    ss_base + esp,
                    self.regs.cr3,
                    AccessType::Write,
                    self.regs.cpl,
                    &*memory,
                )?;
                memory.write_u16(phys, old_ss)?;

                let esp = self.regs.sp().wrapping_sub(2);
                self.regs.set_sp(esp);
                let phys = mmu.translate_linear(
                    ss_base + esp,
                    self.regs.cr3,
                    AccessType::Write,
                    self.regs.cpl,
                    &*memory,
                )?;
                memory.write_u16(phys, old_esp as u16)?;
            } else {
                push32!(old_ss as u32);
                push32!(old_esp);
            }
        }

        if gate_is_16 {
            // 16-bit gate frame: FLAGS, CS, IP (words).
            let esp = self.regs.sp().wrapping_sub(2);
            self.regs.set_sp(esp);
            let phys = mmu.translate_linear(
                ss_base + esp,
                self.regs.cr3,
                AccessType::Write,
                self.regs.cpl,
                &*memory,
            )?;
            memory.write_u16(phys, old_eflags as u16)?;

            let esp = self.regs.sp().wrapping_sub(2);
            self.regs.set_sp(esp);
            let phys = mmu.translate_linear(
                ss_base + esp,
                self.regs.cr3,
                AccessType::Write,
                self.regs.cpl,
                &*memory,
            )?;
            memory.write_u16(phys, old_cs)?;

            let esp = self.regs.sp().wrapping_sub(2);
            self.regs.set_sp(esp);
            let phys = mmu.translate_linear(
                ss_base + esp,
                self.regs.cr3,
                AccessType::Write,
                self.regs.cpl,
                &*memory,
            )?;
            memory.write_u16(phys, old_eip as u16)?;
        } else {
            // 32-bit gate frame: EFLAGS, CS, EIP (dwords).
            let esp = self.regs.sp().wrapping_sub(4);
            self.regs.set_sp(esp);
            let phys = mmu.translate_linear(
                ss_base + esp,
                self.regs.cr3,
                AccessType::Write,
                self.regs.cpl,
                &*memory,
            )?;
            memory.write_u32(phys, old_eflags)?;

            let esp = self.regs.sp().wrapping_sub(4);
            self.regs.set_sp(esp);
            let phys = mmu.translate_linear(
                ss_base + esp,
                self.regs.cr3,
                AccessType::Write,
                self.regs.cpl,
                &*memory,
            )?;
            memory.write_u32(phys, old_cs as u32)?;

            let esp = self.regs.sp().wrapping_sub(4);
            self.regs.set_sp(esp);
            let phys = mmu.translate_linear(
                ss_base + esp,
                self.regs.cr3,
                AccessType::Write,
                self.regs.cpl,
                &*memory,
            )?;
            memory.write_u32(phys, old_eip)?;
        }

        // Push error code if applicable
        if has_error_code {
            let ec = error_code.unwrap_or(0);
            if gate_is_16 {
                let esp = self.regs.sp().wrapping_sub(2);
                self.regs.set_sp(esp);
                let phys = mmu.translate_linear(
                    ss_base + esp,
                    self.regs.cr3,
                    AccessType::Write,
                    self.regs.cpl,
                    &*memory,
                )?;
                memory.write_u16(phys, ec as u16)?;
            } else {
                let esp = self.regs.sp().wrapping_sub(4);
                self.regs.set_sp(esp);
                let phys = mmu.translate_linear(
                    ss_base + esp,
                    self.regs.cr3,
                    AccessType::Write,
                    self.regs.cpl,
                    &*memory,
                )?;
                memory.write_u32(phys, ec)?;
            }
        }

        // Clear IF for interrupt gates (not trap gates)
        match entry.gate_type {
            crate::interrupts::GateType::Interrupt32 | crate::interrupts::GateType::Interrupt16 => {
                self.regs.rflags &= !IF;
            }
            _ => {}
        }
        // Clear TF
        self.regs.rflags &= !TF;

        // Log exception delivery for debugging
        if self.consecutive_exception_count <= 3 {
            libsyscall::serial_print(format_args!(
                "[corevm]  -> deliver vec={} gate={:?} frame={} to CS={:04X}:{:08X} old_CS:EIP={:04X}:{:08X} old_ESP={:08X}\n",
                vector,
                entry.gate_type,
                if gate_is_16 { 16 } else { 32 },
                entry.selector,
                entry.offset as u32,
                old_cs, old_eip, self.regs.sp() as u32,
            ));
        }

        // Load handler CS from GDT.
        self.load_segment_from_gdt(SegReg::Cs, entry.selector, &*memory, mmu)?;
        self.update_mode();
        self.regs.rip = entry.offset;
        self.regs.cpl = target_cpl;

        Ok(())
    }

    /// Long-mode interrupt delivery via 64-bit IDT gate.
    fn deliver_interrupt_long(
        &mut self,
        vector: u8,
        has_error_code: bool,
        error_code: Option<u32>,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        interrupts: &mut InterruptController,
    ) -> Result<()> {
        use crate::flags::IF;
        use crate::flags::TF;

        // IDT access during interrupt delivery is an implicit supervisor access
        // (Intel SDM Vol. 3A §6.13) — always use CPL=0 regardless of current privilege.
        let entry = interrupts.read_idt_entry_long(
            vector,
            self.regs.idtr.base,
            self.regs.idtr.limit,
            self.regs.cr3,
            0,
            mmu,
            &*memory,
        )?;

        if !entry.present {
            return Err(VmError::GeneralProtection((vector as u32) * 16 + 2));
        }

        // Save old state
        let old_rflags = self.regs.rflags;
        let old_cs = self.regs.seg[SegReg::Cs as usize].selector;
        let old_rip = self.regs.rip;
        let old_rsp = self.regs.sp();
        let old_ss = self.regs.seg[SegReg::Ss as usize].selector;

        // Long mode stack switching:
        // 1. IST (Interrupt Stack Table): if IDT entry IST != 0, load RSP from TSS.IST[n]
        // 2. Privilege change (CPL 3→0): load RSP from TSS.RSP0
        // 3. Same privilege: keep current RSP
        let target_cpl = (entry.selector & 3) as u8; // handler's DPL from CS selector RPL
        let target_cpl = 0u8; // interrupt handlers always run at ring 0 in long mode

        if entry.ist != 0 {
            // IST stack: read TSS.IST[n] (offset 0x24 + (ist-1)*8)
            let ist_index = (entry.ist - 1) as u64;
            let tss_base = self.read_tss_base64(memory, mmu)?;
            let ist_offset = 0x24 + ist_index * 8;
            let ist_phys = mmu.translate_linear(tss_base + ist_offset, self.regs.cr3, AccessType::Read, 0, memory)?;
            let new_rsp = memory.read_u64(ist_phys)?;
            self.regs.set_sp(new_rsp);
            // Load flat kernel SS (selector 0, which in long mode is valid)
            self.regs.seg[SegReg::Ss as usize].selector = 0;
            self.regs.seg[SegReg::Ss as usize].base = 0;
        } else if self.regs.cpl > target_cpl {
            // Privilege level change: load RSP0 from 64-bit TSS (offset 0x04)
            let tss_base = self.read_tss_base64(memory, mmu)?;
            let rsp0_phys = mmu.translate_linear(tss_base + 4, self.regs.cr3, AccessType::Read, 0, memory)?;
            let new_rsp = memory.read_u64(rsp0_phys)?;
            self.regs.set_sp(new_rsp);
            // Load flat kernel SS
            self.regs.seg[SegReg::Ss as usize].selector = 0;
            self.regs.seg[SegReg::Ss as usize].base = 0;
        }
        // else: same privilege, keep current RSP

        // Switch to target privilege level before pushing frame onto kernel stack
        self.regs.cpl = target_cpl;

        // Push SS
        let rsp = self.regs.sp().wrapping_sub(8);
        self.regs.set_sp(rsp);
        let phys = mmu.translate_linear(rsp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
        memory.write_u64(phys, old_ss as u64)?;

        // Push old RSP
        let rsp = self.regs.sp().wrapping_sub(8);
        self.regs.set_sp(rsp);
        let phys = mmu.translate_linear(rsp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
        memory.write_u64(phys, old_rsp)?;

        // Push RFLAGS
        let rsp = self.regs.sp().wrapping_sub(8);
        self.regs.set_sp(rsp);
        let phys = mmu.translate_linear(rsp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
        memory.write_u64(phys, old_rflags)?;

        // Push CS
        let rsp = self.regs.sp().wrapping_sub(8);
        self.regs.set_sp(rsp);
        let phys = mmu.translate_linear(rsp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
        memory.write_u64(phys, old_cs as u64)?;

        // Push RIP
        let rsp = self.regs.sp().wrapping_sub(8);
        self.regs.set_sp(rsp);
        let phys = mmu.translate_linear(rsp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
        memory.write_u64(phys, old_rip)?;

        // Push error code if applicable
        if has_error_code {
            let ec = error_code.unwrap_or(0);
            let rsp = self.regs.sp().wrapping_sub(8);
            self.regs.set_sp(rsp);
            let phys = mmu.translate_linear(rsp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
            memory.write_u64(phys, ec as u64)?;
        }

        // Clear IF for interrupt gates
        match entry.gate_type {
            crate::interrupts::GateType::Interrupt64 => {
                self.regs.rflags &= !IF;
            }
            _ => {}
        }
        // Clear TF
        self.regs.rflags &= !TF;

        // Load handler CS from GDT.
        self.load_segment_from_gdt(SegReg::Cs, entry.selector, &*memory, mmu)?;
        self.update_mode();
        self.regs.rip = entry.offset;
        self.regs.cpl = 0;

        Ok(())
    }
}

impl Cpu {
    /// Read the 64-bit TSS base address from the GDT descriptor pointed to by TR.
    fn read_tss_base64(
        &self,
        memory: &GuestMemory,
        mmu: &Mmu,
    ) -> Result<u64> {
        let tr = self.regs.tr;
        if (tr & 0xFFFC) == 0 {
            return Err(VmError::InvalidTss(0));
        }
        let tss_desc = self.read_gdt_descriptor(tr, memory, mmu)?;
        Ok(tss_desc.base)
    }

    /// Read 32-bit ring stack selector/pointer (SS:ESP) from current 32-bit TSS.
    fn read_tss_ring_stack32(
        &self,
        ring: u8,
        memory: &GuestMemory,
        mmu: &Mmu,
    ) -> Result<(u16, u32)> {
        if ring > 2 {
            return Err(VmError::InvalidTss(0));
        }
        let tr = self.regs.tr;
        if (tr & 0xFFFC) == 0 {
            return Err(VmError::InvalidTss(0));
        }
        let tss_desc = self.read_gdt_descriptor(tr, memory, mmu)?;
        let tss_base = tss_desc.base;
        let off_esp = 4u64 + (ring as u64) * 8;
        let off_ss = 8u64 + (ring as u64) * 8;

        // TSS base is linear, translate through paging as supervisor.
        let esp_phys = mmu.translate_linear(tss_base + off_esp, self.regs.cr3, AccessType::Read, 0, memory)?;
        let ss_phys = mmu.translate_linear(tss_base + off_ss, self.regs.cr3, AccessType::Read, 0, memory)?;
        let esp = memory.read_u32(esp_phys)?;
        let ss = memory.read_u16(ss_phys)?;
        Ok((ss, esp))
    }

    /// Perform a 32-bit hardware task switch via a Task Gate.
    ///
    /// This implements the x86 hardware task switch mechanism:
    /// 1. Save current CPU state into the old TSS (current TR)
    /// 2. Load new CPU state from the new TSS (task gate selector)
    /// 3. Update TR, load CR3, reload all segments
    /// 4. Push error code if present
    fn task_switch_32(
        &mut self,
        new_tss_selector: u16,
        has_error_code: bool,
        error_code: Option<u32>,
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
    ) -> Result<()> {
        use crate::memory::{AccessType, MemoryBus};

        // Helper: read u32 from TSS at given offset (linear address, translated via paging).
        let tss_read_u32 = |base: u64, off: u64, cr3: u64, mmu: &Mmu, mem: &GuestMemory| -> Result<u32> {
            let phys = mmu.translate_linear(base + off, cr3, AccessType::Read, 0, mem)?;
            mem.read_u32(phys)
        };
        let tss_read_u16 = |base: u64, off: u64, cr3: u64, mmu: &Mmu, mem: &GuestMemory| -> Result<u16> {
            let phys = mmu.translate_linear(base + off, cr3, AccessType::Read, 0, mem)?;
            mem.read_u16(phys)
        };

        // ── 1. Read old TSS descriptor (current TR) ──
        let old_tr = self.regs.tr;

        // If switching to the same TSS we're already in, this is a recursive
        // fault in the #DF handler → triple fault.
        if old_tr == new_tss_selector {
            #[cfg(feature = "host_test")]
            eprintln!("[corevm] TRIPLE FAULT: task switch to same TSS {:#06x}", new_tss_selector);
            return Err(crate::error::VmError::Shutdown);
        }

        let old_tss_desc = self.read_gdt_descriptor(old_tr, memory, mmu)?;
        let old_tss_base = old_tss_desc.base;

        // ── 2. Save current state into old TSS ──
        // 32-bit TSS layout:
        //   0x00: back_link, 0x04: ESP0, 0x08: SS0, 0x0C: ESP1, 0x10: SS1,
        //   0x14: ESP2, 0x18: SS2, 0x1C: CR3, 0x20: EIP, 0x24: EFLAGS,
        //   0x28: EAX, 0x2C: ECX, 0x30: EDX, 0x34: EBX, 0x38: ESP,
        //   0x3C: EBP, 0x40: ESI, 0x44: EDI,
        //   0x48: ES, 0x4C: CS, 0x50: SS, 0x54: DS, 0x58: FS, 0x5C: GS,
        //   0x60: LDTR, 0x64: (reserved+IOPB offset)
        let save_u32 = |base: u64, off: u64, val: u32, cr3: u64, mmu: &Mmu, mem: &mut GuestMemory| -> Result<()> {
            let phys = mmu.translate_linear(base + off, cr3, AccessType::Write, 0, mem)?;
            mem.write_u32(phys, val)
        };
        let cr3 = self.regs.cr3;
        save_u32(old_tss_base, 0x20, self.regs.rip as u32, cr3, mmu, memory)?;
        save_u32(old_tss_base, 0x24, self.regs.rflags as u32, cr3, mmu, memory)?;
        save_u32(old_tss_base, 0x28, self.regs.gpr[0] as u32, cr3, mmu, memory)?;  // EAX
        save_u32(old_tss_base, 0x2C, self.regs.gpr[1] as u32, cr3, mmu, memory)?;  // ECX
        save_u32(old_tss_base, 0x30, self.regs.gpr[2] as u32, cr3, mmu, memory)?;  // EDX
        save_u32(old_tss_base, 0x34, self.regs.gpr[3] as u32, cr3, mmu, memory)?;  // EBX
        save_u32(old_tss_base, 0x38, self.regs.sp() as u32, cr3, mmu, memory)?;     // ESP
        save_u32(old_tss_base, 0x3C, self.regs.gpr[5] as u32, cr3, mmu, memory)?;  // EBP
        save_u32(old_tss_base, 0x40, self.regs.gpr[6] as u32, cr3, mmu, memory)?;  // ESI
        save_u32(old_tss_base, 0x44, self.regs.gpr[7] as u32, cr3, mmu, memory)?;  // EDI
        save_u32(old_tss_base, 0x48, self.regs.seg[SegReg::Es as usize].selector as u32, cr3, mmu, memory)?;
        save_u32(old_tss_base, 0x4C, self.regs.seg[SegReg::Cs as usize].selector as u32, cr3, mmu, memory)?;
        save_u32(old_tss_base, 0x50, self.regs.seg[SegReg::Ss as usize].selector as u32, cr3, mmu, memory)?;
        save_u32(old_tss_base, 0x54, self.regs.seg[SegReg::Ds as usize].selector as u32, cr3, mmu, memory)?;
        save_u32(old_tss_base, 0x58, self.regs.seg[SegReg::Fs as usize].selector as u32, cr3, mmu, memory)?;
        save_u32(old_tss_base, 0x5C, self.regs.seg[SegReg::Gs as usize].selector as u32, cr3, mmu, memory)?;

        // ── 3. Read new TSS descriptor ──
        let new_tss_desc = self.read_gdt_descriptor(new_tss_selector, memory, mmu)?;
        let new_tss_base = new_tss_desc.base;

        // Store back-link to old task in new TSS[0x00]
        save_u32(new_tss_base, 0x00, old_tr as u32, cr3, mmu, memory)?;

        // ── 5. Load new state from new TSS ──
        let new_cr3 = tss_read_u32(new_tss_base, 0x1C, cr3, mmu, memory)?;
        let new_eip = tss_read_u32(new_tss_base, 0x20, cr3, mmu, memory)?;
        let new_eflags = tss_read_u32(new_tss_base, 0x24, cr3, mmu, memory)?;
        let new_eax = tss_read_u32(new_tss_base, 0x28, cr3, mmu, memory)?;
        let new_ecx = tss_read_u32(new_tss_base, 0x2C, cr3, mmu, memory)?;
        let new_edx = tss_read_u32(new_tss_base, 0x30, cr3, mmu, memory)?;
        let new_ebx = tss_read_u32(new_tss_base, 0x34, cr3, mmu, memory)?;
        let new_esp = tss_read_u32(new_tss_base, 0x38, cr3, mmu, memory)?;
        let new_ebp = tss_read_u32(new_tss_base, 0x3C, cr3, mmu, memory)?;
        let new_esi = tss_read_u32(new_tss_base, 0x40, cr3, mmu, memory)?;
        let new_edi = tss_read_u32(new_tss_base, 0x44, cr3, mmu, memory)?;
        let new_es  = tss_read_u16(new_tss_base, 0x48, cr3, mmu, memory)?;
        let new_cs  = tss_read_u16(new_tss_base, 0x4C, cr3, mmu, memory)?;
        let new_ss  = tss_read_u16(new_tss_base, 0x50, cr3, mmu, memory)?;
        let new_ds  = tss_read_u16(new_tss_base, 0x54, cr3, mmu, memory)?;
        let new_fs  = tss_read_u16(new_tss_base, 0x58, cr3, mmu, memory)?;
        let new_gs  = tss_read_u16(new_tss_base, 0x5C, cr3, mmu, memory)?;
        let new_ldtr = tss_read_u16(new_tss_base, 0x60, cr3, mmu, memory)?;

        #[cfg(feature = "host_test")]
        eprintln!("[corevm] TASK SWITCH: old_tr={:#06x} old_tss_base={:#010x} -> new_sel={:#06x} new_tss_base={:#010x}",
            old_tr, old_tss_base, new_tss_selector, new_tss_base);
        #[cfg(feature = "host_test")]
        eprintln!("[corevm]   new CR3={:#010x} EIP={:#010x} EFLAGS={:#010x} ESP={:#010x} EBP={:#010x}",
            new_cr3, new_eip, new_eflags, new_esp, new_ebp);
        #[cfg(feature = "host_test")]
        eprintln!("[corevm]   new CS={:#06x} SS={:#06x} DS={:#06x} ES={:#06x} FS={:#06x} GS={:#06x}",
            new_cs, new_ss, new_ds, new_es, new_fs, new_gs);
        #[cfg(feature = "host_test")]
        eprintln!("[corevm]   new EAX={:#010x} EBX={:#010x} ECX={:#010x} EDX={:#010x} ESI={:#010x} EDI={:#010x}",
            new_eax, new_ebx, new_ecx, new_edx, new_esi, new_edi);

        // ── 6. Switch CR3 and flush TLB ──
        self.regs.cr3 = new_cr3 as u64;
        mmu.flush_tlb();
        mmu.update_from_regs(self.regs.cr0, self.regs.cr4, self.regs.efer);
            mmu.rflags_ac = (self.regs.rflags & crate::flags::AC) != 0;

        // ── 7. Load registers ──
        self.regs.rip = new_eip as u64;
        self.regs.rflags = (new_eflags as u64) | 0x02; // bit 1 always set
        // Set NT (Nested Task) flag to indicate this came from a task switch
        self.regs.rflags |= 1 << 14; // NT flag
        self.regs.gpr[0] = new_eax as u64;  // EAX
        self.regs.gpr[1] = new_ecx as u64;  // ECX
        self.regs.gpr[2] = new_edx as u64;  // EDX
        self.regs.gpr[3] = new_ebx as u64;  // EBX
        self.regs.set_sp(new_esp as u64);
        self.regs.gpr[5] = new_ebp as u64;  // EBP
        self.regs.gpr[6] = new_esi as u64;  // ESI
        self.regs.gpr[7] = new_edi as u64;  // EDI

        // ── 8. Load segment registers ──
        self.load_segment_from_gdt(SegReg::Es, new_es, memory, mmu)?;
        self.load_segment_from_gdt(SegReg::Cs, new_cs, memory, mmu)?;
        self.load_segment_from_gdt(SegReg::Ss, new_ss, memory, mmu)?;
        self.load_segment_from_gdt(SegReg::Ds, new_ds, memory, mmu)?;
        self.load_segment_from_gdt(SegReg::Fs, new_fs, memory, mmu)?;
        self.load_segment_from_gdt(SegReg::Gs, new_gs, memory, mmu)?;

        // Update TR to new TSS selector
        self.regs.tr = new_tss_selector;
        // Update LDTR
        self.regs.ldtr = new_ldtr;

        // Update CPL from new CS
        self.regs.cpl = (new_cs & 3) as u8;

        // Update decoder mode — task switches are always in protected mode.
        self.decoder.set_mode(CpuMode::Protected32);
        self.mode = Mode::ProtectedMode;

        // ── 9. Push error code if present ──
        if has_error_code {
            if let Some(ec) = error_code {
                let ss_base = self.regs.seg[SegReg::Ss as usize].base;
                let esp = self.regs.sp().wrapping_sub(4);
                self.regs.set_sp(esp);
                let phys = mmu.translate_linear(
                    ss_base + esp,
                    self.regs.cr3,
                    AccessType::Write,
                    self.regs.cpl,
                    memory,
                )?;
                memory.write_u32(phys, ec)?;
            }
        }

        // Clear IF and TF
        self.regs.rflags &= !(crate::flags::IF | crate::flags::TF);

        Ok(())
    }
}
