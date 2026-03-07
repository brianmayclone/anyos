//! CPU emulation core — state management and execution loop.
//!
//! The `Cpu` struct holds all architectural state (registers, FPU, SSE)
//! and implements the fetch-decode-execute cycle. The execution loop
//! catches instruction errors and routes them to the guest's IDT as
//! hardware exceptions.

#[inline(always)]
fn rdtsc() -> u64 {
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
    /// TSC-based timing counters for profiling the run loop.
    pub perf_tsc_interrupt: u64,
    pub perf_tsc_translate: u64,
    pub perf_tsc_smc: u64,
    pub perf_tsc_decode: u64,
    pub perf_tsc_jit_exec: u64,
    pub perf_tsc_interp: u64,
    pub perf_tsc_total: u64,
    pub perf_loop_count: u64,
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
            consecutive_exception_count: 0,
            exc_ring: [(0, 0, 0, 0); 32],
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
        self.mode = if pe && pg && lma {
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
        let phys = mmu.translate_linear(
            addr,
            self.regs.cr3,
            AccessType::Read,
            self.regs.cpl,
            memory,
        )?;
        let raw = memory.read_u64(phys)?;
        let mut desc = SegmentDescriptor::from_raw(selector, raw);

        // Long-mode system descriptors (TSS, LDT) are 16 bytes wide.
        // Access byte bit 4 (S flag) = 0 indicates a system descriptor.
        let is_system = (desc.access & 0x10) == 0;
        if matches!(self.mode, Mode::LongMode) && is_system && desc.present {
            if index + 15 > self.regs.gdtr.limit as u64 {
                return Err(VmError::GeneralProtection(selector as u32 & 0xFFFC));
            }
            let addr_hi = self.regs.gdtr.base.wrapping_add(index + 8);
            let phys_hi = mmu.translate_linear(
                addr_hi,
                self.regs.cr3,
                AccessType::Read,
                self.regs.cpl,
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
            // Null selector — allowed for data segments, not CS/SS.
            if matches!(seg, SegReg::Cs | SegReg::Ss) {
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
                    // Detect infinite exception loops (e.g. #PF handler itself faults).
                    // Track by faulting RIP — the handler runs instructions between
                    // faults, so we can't rely on strictly consecutive errors.
                    if self.last_exec_rip == self.consecutive_exception_rip {
                        self.consecutive_exception_count += 1;
                    } else {
                        self.consecutive_exception_rip = self.last_exec_rip;
                        self.consecutive_exception_count = 1;
                    }
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
                    // Detect infinite exception loops (same check as fallback path).
                    if self.last_exec_rip == self.consecutive_exception_rip {
                        self.consecutive_exception_count += 1;
                    } else {
                        self.consecutive_exception_rip = self.last_exec_rip;
                        self.consecutive_exception_count = 1;
                    }
                    if self.consecutive_exception_count > 20 {
                        #[cfg(feature = "host_test")]
                        eprintln!("[corevm] exception loop: CS:IP={:04X}:{:X} ({} repeats) CR2={:X} ESP={:X}",
                            self.regs.seg[SegReg::Cs as usize].selector,
                            self.last_exec_rip, self.consecutive_exception_count,
                            self.regs.cr2, self.regs.sp());
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
            VmError::UndefinedOpcode(_) => (6, None, None),
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

        // Double fault detection: if we're already handling an exception and
        // another contributory exception occurs, deliver #DF (vector 8) instead.
        // Only #PF, #GP, #SS, #NP, #TS, #DF, #AC are contributory; others nest normally.
        let is_contributory = matches!(vector, 0 | 8 | 10 | 11 | 12 | 13 | 14 | 17);
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

        let result = self.deliver_interrupt(
            vector,
            error_code.is_some(),
            error_code,
            memory,
            mmu,
            interrupts,
        );

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
        match self.mode {
            Mode::RealMode => {
                self.deliver_interrupt_real(vector, memory, mmu)
            }
            Mode::ProtectedMode => {
                self.deliver_interrupt_protected(
                    vector,
                    has_error_code,
                    error_code,
                    memory,
                    mmu,
                    interrupts,
                )
            }
            Mode::LongMode => {
                self.deliver_interrupt_long(
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

        let entry = interrupts.read_idt_entry_protected(
            vector,
            self.regs.idtr.base,
            self.regs.idtr.limit,
            self.regs.cr3,
            self.regs.cpl,
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

        // Determine target CPL from target code segment descriptor.
        let target_cs_desc = self.read_gdt_descriptor(entry.selector, memory, mmu)?;
        let target_cpl = target_cs_desc.dpl;

        // Inter-privilege interrupt entry: switch stack via TSS ring stack.
        if target_cpl < old_cpl {
            let (new_ss, new_esp) = self.read_tss_ring_stack32(target_cpl, memory, mmu)?;

            self.load_segment_from_gdt(SegReg::Ss, new_ss, memory, mmu)?;
            self.regs.set_sp(new_esp as u64);
            self.regs.cpl = target_cpl;
        }

        let ss_base = self.regs.seg[SegReg::Ss as usize].base;
        let gate_is_16 = matches!(
            entry.gate_type,
            crate::interrupts::GateType::Interrupt16 | crate::interrupts::GateType::Trap16
        );

        // Push old SS:ESP only on privilege-level change.
        if target_cpl < old_cpl {
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
                let esp = self.regs.sp().wrapping_sub(4);
                self.regs.set_sp(esp);
                let phys = mmu.translate_linear(
                    ss_base + esp,
                    self.regs.cr3,
                    AccessType::Write,
                    self.regs.cpl,
                    &*memory,
                )?;
                memory.write_u32(phys, old_ss as u32)?;

                let esp = self.regs.sp().wrapping_sub(4);
                self.regs.set_sp(esp);
                let phys = mmu.translate_linear(
                    ss_base + esp,
                    self.regs.cr3,
                    AccessType::Write,
                    self.regs.cpl,
                    &*memory,
                )?;
                memory.write_u32(phys, old_esp)?;
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

        let entry = interrupts.read_idt_entry_long(
            vector,
            self.regs.idtr.base,
            self.regs.idtr.limit,
            self.regs.cr3,
            self.regs.cpl,
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

        // In long mode, the stack is always 64-bit
        // TODO: IST stack switching, privilege level transition

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
