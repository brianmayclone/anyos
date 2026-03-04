//! CPU emulation core — state management and execution loop.
//!
//! The `Cpu` struct holds all architectural state (registers, FPU, SSE)
//! and implements the fetch-decode-execute cycle. The execution loop
//! catches instruction errors and routes them to the guest's IDT as
//! hardware exceptions.

use crate::decoder::{CpuMode, Decoder};
use crate::error::{Result, VmError};
use crate::fpu_state::FpuState;
use crate::interrupts::InterruptController;
use crate::io::IoDispatch;
use crate::jit::block::{self, BlockKey};
use crate::jit::cache::DecodeCache;
use crate::jit::JitEngine;
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
    stop_requested: bool,
    /// A20 gate enabled (address line 20 masking for real-mode compat).
    pub a20_enabled: bool,
    /// RIP at the start of the last successfully decoded instruction.
    pub last_exec_rip: u64,
    /// CS selector at the start of the last decoded instruction.
    pub last_exec_cs: u16,
    /// Opcode of the last decoded instruction (for diagnostics).
    pub last_opcode: u16,
    /// Physical address of the last decoded instruction.
    pub last_fetch_addr: u64,
    /// Decode cache for pre-decoded basic blocks (JIT Phase 1).
    pub decode_cache: DecodeCache,
    /// JIT engine for native code compilation and execution (Phase 2+3).
    pub jit_engine: JitEngine,
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
            last_fetch_addr: 0,
            decode_cache: DecodeCache::new(),
            jit_engine: JitEngine::new(),
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
        self.last_fetch_addr = 0;
        self.decode_cache.flush();
        self.jit_engine.flush();
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
        loop {
            // Check stop request and instruction limit every iteration.
            // The branch predictor handles these near-always-false checks
            // with negligible overhead. The previous `& 0xFF` gating was
            // broken: with basic blocks of 2 instructions, an odd starting
            // instruction_count stays odd forever and never hits a multiple
            // of 256, causing run() to never return.
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
                if let Err(e) = self.deliver_interrupt(vector, false, None, memory, mmu, interrupts)
                {
                    return ExitReason::Exception(e);
                }
                // Clear interrupt shadow after delivery
                interrupts.interrupt_shadow = false;
            }

            // Clear interrupt shadow for the next instruction
            interrupts.interrupt_shadow = false;

            // Compute the linear address of the instruction
            let cs = &self.regs.seg[SegReg::Cs as usize];
            let fetch_addr = cs.base.wrapping_add(self.regs.rip);

            // Apply A20 gate masking
            let fetch_addr = if !self.a20_enabled {
                fetch_addr & !0x10_0000 // Clear bit 20
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
                    if let Err(e2) =
                        self.inject_exception_from_error(&e, memory, mmu, interrupts)
                    {
                        return ExitReason::Exception(e2);
                    }
                    continue;
                }
            };

            // Save trace info for diagnostics before decode/execute.
            self.last_exec_rip = self.regs.rip;
            self.last_exec_cs = self.regs.seg[SegReg::Cs as usize].selector;
            self.last_fetch_addr = phys_addr;

            // ── Decode Cache path ──────────────────────────────────
            // Try the decode cache first: look up a pre-decoded basic block
            // by (phys_addr, mode, cs_base). On hit, execute the cached
            // instruction sequence without re-decoding.
            let cs_base = self.regs.seg[SegReg::Cs as usize].base;
            let block_key = BlockKey {
                phys_addr,
                mode: self.decoder.mode(),
                cs_base,
            };

            if let Some(cached_block) = self.decode_cache.lookup(&block_key) {
                // Clone the instruction list so we can mutably borrow self
                // during execution. The Vec<DecodedInst> is stack-allocated
                // metadata — the actual data is small (typically 1-64 entries).
                let block_insts = cached_block.instructions.clone();

                // ── JIT path: compile and execute natively ──
                if self.jit_engine.is_enabled() {
                    let jit_result = self.jit_execute_block(
                        &block_key, &block_insts,
                        memory, mmu, io, interrupts,
                    );
                    match jit_result {
                        BlockExitReason::Continue => continue,
                        BlockExitReason::Exit(reason) => return reason,
                    }
                }

                let block_exit = self.execute_cached_block(
                    &block_insts, phys_addr, fetch_addr, memory, mmu, io, interrupts,
                );
                match block_exit {
                    BlockExitReason::Continue => continue,
                    BlockExitReason::Exit(reason) => return reason,
                }
            }

            // Cache miss: try to detect and cache a full basic block.
            // If detection succeeds, cache it and execute the block.
            // If detection fails (e.g. decode error on first instruction),
            // fall through to single-instruction decode below.
            if let Ok(new_block) = block::detect_basic_block(
                &self.decoder, &*memory, phys_addr,
            ) {
                // ── JIT path: compile and execute natively ──
                if self.jit_engine.is_enabled() {
                    let block_insts = new_block.instructions.clone();
                    self.decode_cache.insert(block_key, new_block);
                    let jit_result = self.jit_execute_block(
                        &block_key, &block_insts,
                        memory, mmu, io, interrupts,
                    );
                    match jit_result {
                        BlockExitReason::Continue => continue,
                        BlockExitReason::Exit(reason) => return reason,
                    }
                }

                let block_insts = new_block.instructions.clone();
                self.decode_cache.insert(block_key, new_block);
                let block_exit = self.execute_cached_block(
                    &block_insts, phys_addr, fetch_addr, memory, mmu, io, interrupts,
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

            self.last_opcode = inst.opcode;

            // Execute the decoded instruction
            match crate::executor::execute(self, &inst, memory, mmu, io, interrupts) {
                Ok(()) => {
                    self.instruction_count += 1;
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
                    if let Err(e2) =
                        self.inject_exception_from_error(e, memory, mmu, interrupts)
                    {
                        return ExitReason::Exception(e2);
                    }
                }
            }
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
        _block_fetch: u64,
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
            self.last_opcode = inst.opcode;

            match crate::executor::execute(self, inst, memory, mmu, io, interrupts) {
                Ok(()) => {
                    self.instruction_count += 1;
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
                    use crate::memory::MemoryBus;
                    let b0 = memory.read_u8(inst_phys).unwrap_or(0xFF);
                    let b1 = memory.read_u8(inst_phys + 1).unwrap_or(0xFF);
                    let b2 = memory.read_u8(inst_phys + 2).unwrap_or(0xFF);
                    let b3 = memory.read_u8(inst_phys + 3).unwrap_or(0xFF);
                    libsyscall::serial_print(format_args!(
                        "[corevm] exec error at CS:IP={:04X}:{:X} phys={:X} opcode=0x{:04X} bytes=[{:02X} {:02X} {:02X} {:02X}] modrm_reg={} CS.base={:X}: {:?}\n",
                        self.regs.seg[SegReg::Cs as usize].selector,
                        self.last_exec_rip,
                        inst_phys,
                        inst.opcode,
                        b0, b1, b2, b3,
                        inst.modrm_reg(),
                        self.regs.seg[SegReg::Cs as usize].base,
                        e
                    ));
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
    /// Returns `BlockExitReason` for the main run() loop.
    fn jit_execute_block(
        &mut self,
        key: &BlockKey,
        instructions: &[crate::instruction::DecodedInst],
        memory: &mut GuestMemory,
        mmu: &mut Mmu,
        io: &mut IoDispatch,
        interrupts: &mut InterruptController,
    ) -> BlockExitReason {
        use crate::jit::helpers::{JIT_OK, JIT_EXIT_BLOCK};

        // Look up or compile the block.
        let (code_offset, inst_count) = match self.jit_engine.lookup_compiled(key) {
            Some(entry) => entry,
            None => {
                // Build a BasicBlock wrapper for the translator.
                let block = crate::jit::block::BasicBlock {
                    instructions: instructions.into(),
                    byte_len: instructions.iter().map(|i| i.length as usize).sum(),
                    exits_with_branch: false,
                };

                // Switch to writable, compile, then switch back to executable.
                self.jit_engine.make_writable();
                let result = self.jit_engine.compile_block(*key, &block);
                self.jit_engine.make_executable();

                match result {
                    Some(entry) => entry,
                    None => {
                        // JIT buffer full — fall back to interpreter path.
                        return self.execute_cached_block(
                            instructions, key.phys_addr, key.cs_base.wrapping_add(
                                self.regs.rip,
                            ),
                            memory, mmu, io, interrupts,
                        );
                    }
                }
            }
        };

        // Call the compiled native code.
        //
        // Safety: The code was emitted by our translator and follows the C ABI:
        //   fn(cpu: &mut Cpu, mem: &mut GuestMemory, mmu: &mut Mmu,
        //      io: &mut IoDispatch, intr: &mut InterruptController) -> u32
        type JitBlockFn = unsafe extern "C" fn(
            &mut Cpu, &mut GuestMemory, &mut Mmu, &mut IoDispatch,
            &mut InterruptController,
        ) -> u32;

        let result = unsafe {
            let ptr = self.jit_engine.code_ptr(code_offset);
            let func: JitBlockFn = core::mem::transmute(ptr);
            func(self, memory, mmu, io, interrupts)
        };

        // Update instruction count with the number of guest instructions.
        self.instruction_count += inst_count;

        match result {
            JIT_OK => BlockExitReason::Continue,
            JIT_EXIT_BLOCK => {
                // The block signaled an early exit (HLT, page fault, etc.).
                // The interpreter fallback in the JIT code already handled
                // exception injection. Return to the main loop to re-check
                // the new RIP.
                BlockExitReason::Continue
            }
            _ => {
                // Unexpected return code — treat as exit.
                BlockExitReason::Continue
            }
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

        if let Some(addr) = cr2_val {
            self.regs.cr2 = addr;
        }

        // Double fault detection
        if interrupts.handling_exception {
            interrupts.handling_exception = false;
            return Err(VmError::DoubleFault);
        }
        interrupts.handling_exception = true;

        let result = self.deliver_interrupt(
            vector,
            error_code.is_some(),
            error_code,
            memory,
            mmu,
            interrupts,
        );

        interrupts.handling_exception = false;
        result
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
            &*memory,
        )?;

        if !entry.present {
            return Err(VmError::GeneralProtection((vector as u32) * 8 + 2));
        }

        // Save old state
        let old_eflags = self.regs.rflags as u32;
        let old_cs = self.regs.seg[SegReg::Cs as usize].selector;
        let old_eip = self.regs.rip as u32;

        // TODO: Privilege level transition (load new SS:ESP from TSS)
        // For now, assume same privilege level

        let ss_base = self.regs.seg[SegReg::Ss as usize].base;

        // Push EFLAGS
        let esp = self.regs.sp().wrapping_sub(4);
        self.regs.set_sp(esp);
        let phys = mmu.translate_linear(ss_base + esp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
        memory.write_u32(phys, old_eflags)?;

        // Push CS
        let esp = self.regs.sp().wrapping_sub(4);
        self.regs.set_sp(esp);
        let phys = mmu.translate_linear(ss_base + esp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
        memory.write_u32(phys, old_cs as u32)?;

        // Push EIP
        let esp = self.regs.sp().wrapping_sub(4);
        self.regs.set_sp(esp);
        let phys = mmu.translate_linear(ss_base + esp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
        memory.write_u32(phys, old_eip)?;

        // Push error code if applicable
        if has_error_code {
            let ec = error_code.unwrap_or(0);
            let esp = self.regs.sp().wrapping_sub(4);
            self.regs.set_sp(esp);
            let phys = mmu.translate_linear(ss_base + esp, self.regs.cr3, AccessType::Write, self.regs.cpl, &*memory)?;
            memory.write_u32(phys, ec)?;
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

        // Load handler CS from GDT.
        self.load_segment_from_gdt(SegReg::Cs, entry.selector, &*memory, mmu)?;
        self.update_mode();
        self.regs.rip = entry.offset;
        self.regs.cpl = 0; // Handler runs in ring 0

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
