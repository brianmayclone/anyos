//! Guest→Host instruction translator for JIT compilation (v2).
//!
//! Every instruction is translated to native code. Two strategies:
//! 1. **Native emit** — instruction semantics directly in x86-64 machine code
//! 2. **Helper call** — `call helper_execute_one(inst_ptr, cpu, mem, mmu, io, intr)`
//!
//! No interpreter fallback with re-decode. No ratio threshold.
//!
//! ## Register Convention
//!
//! | Host Register | Usage                                      |
//! |---------------|--------------------------------------------|
//! | RBX           | Saved `Cpu*` pointer (callee-saved)        |
//! | R12           | Saved `GuestMemory*` (callee-saved)        |
//! | R13           | Saved `Mmu*` (callee-saved)                |
//! | R14           | Saved `IoDispatch*` (callee-saved)         |
//! | R15           | GPR base pointer (`&cpu.regs.gpr[0]`)      |
//! | R9, R10       | Temporary operand registers                |
//! | R11           | Temporary (captured RFLAGS / scratch)      |
//! | RAX           | Temporary / helper return value            |
//! | RBP           | `InterruptController*` pointer             |

use alloc::vec::Vec;
use crate::decoder::CpuMode;
use crate::flags::OperandSize;
use crate::instruction::{DecodedInst, MemOperand, OpcodeMap, Operand, RepPrefix};
use crate::jit::block::BasicBlock;
use crate::jit::emitter::{Cc, Emitter, Reg, OpSize};
use crate::jit::helpers;
use crate::registers::{RegisterFile, SegmentDescriptor, SegReg};

// ── Compile-time layout assertions ────────────────────────────────────────────
const _: () = {
    assert!(core::mem::size_of::<RegisterFile>() >= 144 + 6 * 32);
    assert!(core::mem::size_of::<SegmentDescriptor>() == 32);
    assert!(core::mem::align_of::<SegmentDescriptor>() == 8);
    assert!(core::mem::offset_of!(SegmentDescriptor, base) == 0);
};

// ── RegisterFile field offsets (#[repr(C)]) ──
const GPR_OFFSET: i32 = core::mem::offset_of!(crate::registers::RegisterFile, gpr) as i32;
const RIP_OFFSET: i32 = core::mem::offset_of!(crate::registers::RegisterFile, rip) as i32;
const RFLAGS_OFFSET: i32 = core::mem::offset_of!(crate::registers::RegisterFile, rflags) as i32;

const ARITH_MASK: i32 = 0x8D5;
const NOT_ARITH_MASK: i32 = !0x8D5;

// ── Host register aliases ──
const CPU_PTR: Reg = Reg::Rbx;
const MEM_PTR: Reg = Reg::R12;
const MMU_PTR: Reg = Reg::R13;
const IO_PTR: Reg = Reg::R14;
const GPR_BASE: Reg = Reg::R15;
const TEMP1: Reg = Reg::R9;
const TEMP2: Reg = Reg::R10;
const FLAGS_TMP: Reg = Reg::R11;

const INTR_REG: Reg = Reg::Rbp;

// ── Cpu field offset for instruction_count ──
const INST_COUNT_OFF: i32 = core::mem::offset_of!(crate::cpu::Cpu, instruction_count) as i32;

// ── Segment descriptor layout ──
const SEG_ARRAY_OFFSET: i32 = 144;
const SEG_DESC_SIZE: i32 = 32;
const SEG_BASE_IN_DESC: i32 = 0;

// ── ALU operation dispatch ──
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AluOp {
    Add, Or, And, Sub, Xor, Cmp,
}

/// A compiled native code block ready for execution.
pub struct CompiledBlock {
    pub code: Vec<u8>,
    pub guest_instruction_count: u64,
    pub entry_phys_addr: u64,
}

/// Translates decoded basic blocks into native x86-64 machine code.
pub struct Translator {
    pub native_count: u64,
    pub helper_count: u64,
    current_mode: CpuMode,
    dispatch_loop_addr: u64,
    /// Per-opcode helper fallback counters: [opcode_map << 8 | opcode] → count.
    /// Primary opcodes: 0x0000..0x00FF, Secondary (0F): 0x0100..0x01FF.
    pub helper_opcode_counts: alloc::collections::BTreeMap<u16, u64>,
}

/// Per-block mutable state during translation.
struct BlockState {
    flags_dirty: bool,
}

impl Translator {
    pub fn new() -> Self {
        Translator {
            native_count: 0,
            helper_count: 0,
            current_mode: CpuMode::Real16,
            dispatch_loop_addr: 0,
            helper_opcode_counts: alloc::collections::BTreeMap::new(),
        }
    }

    /// Translate a basic block. `inst_ptrs` contains raw pointers to the
    /// DecodedInst objects in the decode cache (stable across the JIT block's
    /// lifetime). These pointers are embedded into generated code for
    /// helper_execute_one calls.
    pub fn translate_block(
        &mut self,
        block: &BasicBlock,
        inst_ptrs: &[*const DecodedInst],
        entry_phys: u64,
        mode: CpuMode,
        dispatch_loop_addr: u64,
    ) -> CompiledBlock {
        self.current_mode = mode;
        self.dispatch_loop_addr = dispatch_loop_addr;
        let mut emit = Emitter::new();
        let mut state = BlockState { flags_dirty: false };

        // No prologue — the dispatcher already set up context registers.

        let mut pending_native_count: u32 = 0;

        for (i, inst) in block.instructions.iter().enumerate() {
            if !self.try_translate_native(&mut emit, inst, &mut state) {
                // Flush pending native instruction count before helper call
                if pending_native_count > 0 {
                    Self::emit_bump_inst_count(&mut emit, pending_native_count);
                    pending_native_count = 0;
                }
                // Flush lazy flags before helper call (helper reads RFLAGS).
                self.flush_lazy_flags(&mut emit, &mut state);
                self.emit_helper_call(&mut emit, inst, inst_ptrs[i]);
                self.helper_count += 1;
                let key = match inst.opcode_map {
                    OpcodeMap::Primary => inst.opcode as u16 & 0xFF,
                    OpcodeMap::Secondary => 0x100 | (inst.opcode as u16 & 0xFF),
                    _ => 0x200 | (inst.opcode as u16 & 0xFF),
                };
                *self.helper_opcode_counts.entry(key).or_insert(0) += 1;
            } else {
                self.native_count += 1;
                pending_native_count += 1;
            }
        }

        // Flush remaining native instruction count
        if pending_native_count > 0 {
            Self::emit_bump_inst_count(&mut emit, pending_native_count);
        }

        // Flush any remaining dirty flags, then jump back to dispatcher.
        self.flush_lazy_flags(&mut emit, &mut state);
        emit.xor_rr(OpSize::S32, Reg::Rax, Reg::Rax); // return JIT_OK (0)
        self.emit_exit_to_dispatcher(&mut emit, dispatch_loop_addr);

        CompiledBlock {
            code: emit.finalize(),
            guest_instruction_count: block.instructions.len() as u64,
            entry_phys_addr: entry_phys,
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Prologue / Epilogue
    // ════════════════════════════════════════════════════════════════════

    /// Emit `add qword [cpu + instruction_count], n` using a temp register.
    fn emit_bump_inst_count(emit: &mut Emitter, n: u32) {
        emit.mov_rm(OpSize::S64, Reg::Rax, CPU_PTR, INST_COUNT_OFF);
        emit.add_ri(OpSize::S64, Reg::Rax, n as i32);
        emit.mov_mr(OpSize::S64, CPU_PTR, INST_COUNT_OFF, Reg::Rax);
    }

    /// Jump back to the dispatcher loop. Replaces the old restore-and-ret
    /// sequence — blocks no longer have their own prologue/epilogue.
    fn emit_exit_to_dispatcher(&self, emit: &mut Emitter, dispatch_loop_addr: u64) {
        emit.jmp_abs(dispatch_loop_addr);
    }

    // ════════════════════════════════════════════════════════════════════
    // Lazy RFLAGS
    // ════════════════════════════════════════════════════════════════════

    fn emit_capture_flags(&self, emit: &mut Emitter, state: &mut BlockState) {
        emit.pushfq();
        emit.pop(FLAGS_TMP);
        state.flags_dirty = true;
    }

    fn flush_lazy_flags(&self, emit: &mut Emitter, state: &mut BlockState) {
        if !state.flags_dirty { return; }
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, NOT_ARITH_MASK);
        emit.and_ri(OpSize::S64, FLAGS_TMP, ARITH_MASK);
        emit.or_rr(OpSize::S64, Reg::Rax, FLAGS_TMP);
        emit.mov_mr(OpSize::S64, GPR_BASE, RFLAGS_OFFSET, Reg::Rax);
        state.flags_dirty = false;
    }

    // ════════════════════════════════════════════════════════════════════
    // Helper Call (replaces interpreter fallback)
    // ════════════════════════════════════════════════════════════════════

    /// Emit a call to `helper_execute_one` for one guest instruction.
    /// The instruction pointer is embedded directly — no re-decode.
    fn emit_helper_call(&self, emit: &mut Emitter, inst: &DecodedInst, inst_ptr: *const DecodedInst) {
        // helper_execute_one(inst, cpu, memory, mmu, io, interrupts) -> u32
        // SysV ABI: rdi=inst, rsi=cpu, rdx=memory, rcx=mmu, r8=io, r9=interrupts
        emit.mov_ri64(Reg::Rdi, inst_ptr as u64);
        emit.mov_rr(OpSize::S64, Reg::Rsi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rcx, MMU_PTR);
        emit.mov_rr(OpSize::S64, Reg::R8, IO_PTR);
        emit.mov_rr(OpSize::S64, Reg::R9, INTR_REG); // R9 = interrupts (RBP)
        let helper_addr = helpers::helper_execute_one as *const () as usize as u64;
        emit.call_abs(helper_addr);

        if is_control_flow_instruction(inst) {
            // Control-flow instructions always exit the block after helper.
            // helper_execute_one already updated RIP.
            // Return JIT_EXIT_BLOCK so dispatcher re-enters at new RIP.
            emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
            self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
        } else {
            // Non-control-flow: check if helper requested exit.
            let ok = emit.new_label();
            emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_OK as i32);
            emit.jcc_label(Cc::E, ok);
            // Exit block on error.
            emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
            self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
            emit.bind_label(ok);
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Instruction Translation Dispatch
    // ════════════════════════════════════════════════════════════════════

    fn try_translate_native(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        match inst.opcode_map {
            OpcodeMap::Primary => self.try_primary(emit, inst, state),
            OpcodeMap::Secondary => self.try_secondary(emit, inst, state),
            _ => false,
        }
    }

    fn try_primary(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        let op = inst.opcode as u8;
        match op {
            // NOP
            0x90 => {
                self.emit_advance_rip(emit, inst.length);
                true
            }

            // ALU r/m, r (16/32/64-bit)
            0x01 | 0x09 | 0x21 | 0x29 | 0x31 | 0x39 => {
                self.try_alu_rm_r(emit, inst, op, state)
            }

            // ALU r, r/m (16/32/64-bit)
            0x03 | 0x0B | 0x23 | 0x2B | 0x33 | 0x3B => {
                self.try_alu_r_rm(emit, inst, op, state)
            }

            // ALU accumulator, imm
            0x05 => self.try_alu_acc_imm(emit, inst, AluOp::Add, state),
            0x0D => self.try_alu_acc_imm(emit, inst, AluOp::Or, state),
            0x25 => self.try_alu_acc_imm(emit, inst, AluOp::And, state),
            0x2D => self.try_alu_acc_imm(emit, inst, AluOp::Sub, state),
            0x35 => self.try_alu_acc_imm(emit, inst, AluOp::Xor, state),
            0x3D => self.try_alu_acc_imm(emit, inst, AluOp::Cmp, state),

            // Group 1: ALU r/m, imm
            0x81 | 0x83 => self.try_group1(emit, inst, state),

            // TEST r/m, r
            0x85 => self.try_test_rm_r(emit, inst, state),

            // XCHG r, r/m (mod=3 only)
            0x87 => self.try_xchg_rm_r(emit, inst),

            // MOV r/m, r
            0x89 => self.try_mov_rm_r(emit, inst, state),

            // MOV r, r/m
            0x8B => self.try_mov_r_rm(emit, inst, state),

            // LEA r, [m]
            0x8D => self.try_lea(emit, inst),

            // MOV r, imm
            0xB8..=0xBF => {
                self.emit_mov_r_imm(emit, inst);
                true
            }

            // Shift Group 2: r/m, imm8 (0xC1)
            0xC1 => self.try_shift_ri(emit, inst, state),

            // Shift Group 2: r/m, 1 (0xD1)
            0xD1 => self.try_shift_r1(emit, inst, state),

            // JMP rel32/rel8
            0xE9 | 0xEB => {
                self.emit_jmp_rel(emit, inst);
                true
            }

            // Jcc rel8
            0x70..=0x7F => {
                let cc = (op - 0x70) as u8;
                self.emit_jcc(emit, inst, cc, state);
                true
            }

            // Group 4/5: INC/DEC r/m
            0xFE | 0xFF => self.try_group5(emit, inst, state),

            // PUSH r (all modes except Real16 where segment semantics are tricky)
            0x50..=0x57 => {
                if self.current_mode != CpuMode::Real16 {
                    self.try_push_r(emit, inst, state)
                } else {
                    false
                }
            }

            // POP r
            0x58..=0x5F => {
                if self.current_mode != CpuMode::Real16 {
                    self.try_pop_r(emit, inst, state)
                } else {
                    false
                }
            }

            // RET near
            0xC3 => {
                if self.current_mode != CpuMode::Real16 {
                    self.try_ret_near(emit, inst, state)
                } else {
                    false
                }
            }

            // CALL rel32
            0xE8 => {
                if self.current_mode != CpuMode::Real16 {
                    self.try_call_rel(emit, inst, state)
                } else {
                    false
                }
            }

            0x40..=0x47 => self.try_inc_dec_short(emit, inst, op, true, state),
            0x48..=0x4F => self.try_inc_dec_short(emit, inst, op, false, state),
            0x68 => self.try_push_imm(emit, inst, state, false),
            0x6A => self.try_push_imm(emit, inst, state, true),
            0x88 => self.try_mov_rm8_r8(emit, inst, state),
            0x8A => self.try_mov_r8_rm8(emit, inst, state),
            0xA1 => self.try_mov_eax_moffs(emit, inst, state),
            0xA3 => self.try_mov_moffs_eax(emit, inst, state),
            0xC2 => {
                if self.current_mode != CpuMode::Real16 {
                    self.try_ret_near_imm(emit, inst, state)
                } else {
                    false
                }
            }
            0xC7 => self.try_mov_rm_imm(emit, inst, state),
            0x3C => self.try_cmp_al_imm8(emit, inst, state),
            0x80 => self.try_group1_8bit(emit, inst, state),
            0x84 => self.try_test_rm8_r8(emit, inst, state),

            // MOV r8, imm8 (0xB0-0xB7)
            0xB0..=0xB7 => self.try_mov_r8_imm8(emit, inst),

            // MOV r/m8, imm8 (0xC6)
            0xC6 => self.try_mov_rm8_imm8(emit, inst, state),

            // Shift Group 2: r/m, CL (0xD3)
            0xD3 => self.try_shift_rcl(emit, inst, state),

            // Group 3: TEST/NOT/NEG (0xF7)
            0xF7 => self.try_group3(emit, inst, state),

            // ALU r/m8, r8 (CMP/SUB/ADD/OR/AND/XOR)
            0x38 => self.try_alu_rm8_r8(emit, inst, AluOp::Cmp, state),
            0x28 => self.try_alu_rm8_r8(emit, inst, AluOp::Sub, state),
            0x00 => self.try_alu_rm8_r8(emit, inst, AluOp::Add, state),
            0x08 => self.try_alu_rm8_r8(emit, inst, AluOp::Or, state),
            0x20 => self.try_alu_rm8_r8(emit, inst, AluOp::And, state),
            0x30 => self.try_alu_rm8_r8(emit, inst, AluOp::Xor, state),

            // ALU r8, r/m8
            0x3A => self.try_alu_r8_rm8(emit, inst, AluOp::Cmp, state),
            0x2A => self.try_alu_r8_rm8(emit, inst, AluOp::Sub, state),
            0x02 => self.try_alu_r8_rm8(emit, inst, AluOp::Add, state),
            0x0A => self.try_alu_r8_rm8(emit, inst, AluOp::Or, state),
            0x22 => self.try_alu_r8_rm8(emit, inst, AluOp::And, state),
            0x32 => self.try_alu_r8_rm8(emit, inst, AluOp::Xor, state),

            _ => false,
        }
    }

    fn try_secondary(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        let op = inst.opcode as u8;
        match op {
            // Jcc rel32 (0F 80..8F)
            0x80..=0x8F => {
                let cc = (op - 0x80) as u8;
                self.emit_jcc(emit, inst, cc, state);
                true
            }

            // CMOVcc r, r/m (0F 40-4F)
            0x40..=0x4F => self.try_cmovcc(emit, inst, op, state),

            // SETcc r/m8 (0F 90-9F)
            0x90..=0x9F => self.try_setcc(emit, inst, op, state),

            // IMUL r, r/m (0F AF)
            0xAF => self.try_imul_r_rm(emit, inst, state),

            // MOVZX r, r/m8
            0xB6 => self.try_movzx_r_rm8(emit, inst, state),
            // MOVZX r, r/m16
            0xB7 => self.try_movzx_r_rm16(emit, inst),
            // MOVSX r, r/m8
            0xBE => self.try_movsx_r_rm8(emit, inst, state),
            // MOVSX r, r/m16
            0xBF => self.try_movsx_r_rm16(emit, inst, state),

            _ => false,
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: ALU
    // ════════════════════════════════════════════════════════════════════

    fn try_alu_rm_r(
        &self, emit: &mut Emitter, inst: &DecodedInst, op: u8, state: &mut BlockState,
    ) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let alu_op = match op {
            0x01 => AluOp::Add, 0x09 => AluOp::Or, 0x21 => AluOp::And,
            0x29 => AluOp::Sub, 0x31 => AluOp::Xor, 0x39 => AluOp::Cmp,
            _ => return false,
        };
        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_rm();
            let src_idx = inst.modrm_reg();
            self.emit_alu_rr(emit, inst, alu_op, dst_idx, src_idx, size, state);
            true
        } else {
            // ALU [mem], r — read-modify-write (read-only for CMP)
            let mem_op = match &inst.operands[0] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            state.flags_dirty = false;
            let src_idx = inst.modrm_reg();
            self.emit_load_gpr(emit, TEMP2, src_idx, size);
            self.emit_compute_linear(emit, mem_op, inst);
            // Save linear + src on stack
            emit.push(Reg::Rax);   // linear
            emit.push(TEMP2);      // src value (stack aligned: 2 pushes)

            // Read current value from memory
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, size);
            emit.mov_rr(OpSize::S64, TEMP1, Reg::Rax);

            // Restore src from stack
            emit.pop(TEMP2);

            // Perform ALU
            emit_native_alu_rr(emit, alu_op, size, TEMP1, TEMP2);
            self.emit_capture_flags(emit, state);

            if alu_op != AluOp::Cmp {
                // Write back
                emit.pop(Reg::Rax); // linear
                emit.push(TEMP1);
                emit.sub_ri(OpSize::S64, Reg::Rsp, 8);
                emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
                emit.mov_ri32(Reg::R8, opsize_bytes(size) as u32);
                emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8);
                let wr = helpers::jit_mem_write as *const () as usize as u64;
                emit.call_abs(wr);
                emit.add_ri(OpSize::S64, Reg::Rsp, 8);
                emit.pop(TEMP1);
                let ok = emit.new_label();
                emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
                emit.jcc_label(Cc::Ne, ok);
                emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
                self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
                emit.bind_label(ok);
            } else {
                emit.pop(Reg::Rax); // discard linear
            }

            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    fn try_alu_r_rm(
        &self, emit: &mut Emitter, inst: &DecodedInst, op: u8, state: &mut BlockState,
    ) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let alu_op = match op {
            0x03 => AluOp::Add, 0x0B => AluOp::Or, 0x23 => AluOp::And,
            0x2B => AluOp::Sub, 0x33 => AluOp::Xor, 0x3B => AluOp::Cmp,
            _ => return false,
        };
        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_reg();
            let src_idx = inst.modrm_rm();
            self.emit_alu_rr(emit, inst, alu_op, dst_idx, src_idx, size, state);
            true
        } else {
            // ALU r, [mem] — dst is register, src from memory
            let mem_op = match &inst.operands[1] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            state.flags_dirty = false;
            let dst_idx = inst.modrm_reg();
            self.emit_compute_linear(emit, mem_op, inst);
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, size);
            emit.mov_rr(OpSize::S64, TEMP2, Reg::Rax); // src from memory
            self.emit_load_gpr(emit, TEMP1, dst_idx, size);
            emit_native_alu_rr(emit, alu_op, size, TEMP1, TEMP2);
            self.emit_capture_flags(emit, state);
            if alu_op != AluOp::Cmp {
                self.emit_store_gpr(emit, dst_idx, TEMP1, size);
            }
            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    fn try_alu_acc_imm(
        &self, emit: &mut Emitter, inst: &DecodedInst, alu_op: AluOp, state: &mut BlockState,
    ) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let imm = inst.immediate as i64 as i32;
        self.emit_alu_ri(emit, inst, alu_op, 0, imm, size, state);
        true
    }

    fn try_group1(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let digit = inst.modrm_reg() & 7;
        let alu_op = match digit {
            0 => AluOp::Add,
            1 => AluOp::Or,
            4 => AluOp::And,
            5 => AluOp::Sub,
            6 => AluOp::Xor,
            7 => AluOp::Cmp,
            _ => return false, // ADC, SBB need carry
        };
        let imm = inst.immediate as i64 as i32;

        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_rm();
            self.emit_alu_ri(emit, inst, alu_op, dst_idx, imm, size, state);
            true
        } else {
            // ALU [mem], imm — read-modify-write
            let mem_op = match &inst.operands[0] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            state.flags_dirty = false;

            // Compute linear address
            self.emit_compute_linear(emit, mem_op, inst);
            // Save linear address on stack
            emit.push(Reg::Rax);
            emit.sub_ri(OpSize::S64, Reg::Rsp, 8); // align

            // Read current value
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, size);
            // RAX = value from memory
            emit.mov_rr(OpSize::S64, TEMP1, Reg::Rax);

            // Perform ALU operation
            emit_native_alu_ri(emit, alu_op, size, TEMP1, imm);
            self.emit_capture_flags(emit, state);

            if alu_op != AluOp::Cmp {
                // Write back: restore linear from stack
                emit.add_ri(OpSize::S64, Reg::Rsp, 8);
                emit.pop(Reg::Rax); // linear address
                // Save result
                emit.push(TEMP1);
                emit.sub_ri(OpSize::S64, Reg::Rsp, 8);
                emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
                emit.mov_ri32(Reg::R8, opsize_bytes(size) as u32);
                emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8);
                let wr = helpers::jit_mem_write as *const () as usize as u64;
                emit.call_abs(wr);
                emit.add_ri(OpSize::S64, Reg::Rsp, 8);
                emit.pop(TEMP1);
                let ok = emit.new_label();
                emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
                emit.jcc_label(Cc::Ne, ok);
                emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
                self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
                emit.bind_label(ok);
            } else {
                // CMP: no write-back, just clean up stack
                emit.add_ri(OpSize::S64, Reg::Rsp, 8);
                emit.pop(Reg::Rax);
            }

            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    fn emit_alu_rr(
        &self, emit: &mut Emitter, inst: &DecodedInst,
        alu_op: AluOp, dst_idx: u8, src_idx: u8, size: OpSize, state: &mut BlockState,
    ) {
        state.flags_dirty = false;
        self.emit_load_gpr(emit, TEMP1, dst_idx, size);
        self.emit_load_gpr(emit, TEMP2, src_idx, size);
        emit_native_alu_rr(emit, alu_op, size, TEMP1, TEMP2);
        self.emit_capture_flags(emit, state);
        if alu_op != AluOp::Cmp {
            self.emit_store_gpr(emit, dst_idx, TEMP1, size);
        }
        self.emit_advance_rip(emit, inst.length);
    }

    fn emit_alu_ri(
        &self, emit: &mut Emitter, inst: &DecodedInst,
        alu_op: AluOp, dst_idx: u8, imm: i32, size: OpSize, state: &mut BlockState,
    ) {
        state.flags_dirty = false;
        self.emit_load_gpr(emit, TEMP1, dst_idx, size);
        emit_native_alu_ri(emit, alu_op, size, TEMP1, imm);
        self.emit_capture_flags(emit, state);
        if alu_op != AluOp::Cmp {
            self.emit_store_gpr(emit, dst_idx, TEMP1, size);
        }
        self.emit_advance_rip(emit, inst.length);
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: TEST
    // ════════════════════════════════════════════════════════════════════

    fn try_test_rm_r(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let a_idx = inst.modrm_rm();
        let b_idx = inst.modrm_reg();
        state.flags_dirty = false;
        self.emit_load_gpr(emit, TEMP1, a_idx, size);
        self.emit_load_gpr(emit, TEMP2, b_idx, size);
        emit.test_rr(size, TEMP1, TEMP2);
        self.emit_capture_flags(emit, state);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: INC/DEC
    // ════════════════════════════════════════════════════════════════════

    fn try_group5(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        let digit = inst.modrm_reg() & 7;
        if digit > 1 { return false; } // Only INC(0) / DEC(1)
        let op_byte = inst.opcode as u8;
        if op_byte == 0xFE { return false; } // 8-bit

        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };

        self.flush_lazy_flags(emit, state);

        if inst.modrm_mod() == 3 {
            let reg_idx = inst.modrm_rm();
            self.emit_load_gpr(emit, TEMP1, reg_idx, size);
            if digit == 0 { emit.inc(size, TEMP1); } else { emit.dec(size, TEMP1); }

            emit.pushfq();
            emit.pop(FLAGS_TMP);
            self.emit_store_gpr(emit, reg_idx, TEMP1, size);
        } else {
            // INC/DEC [mem] — read-modify-write
            let mem_op = match &inst.operands[0] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.emit_compute_linear(emit, mem_op, inst);
            emit.push(Reg::Rax);
            emit.sub_ri(OpSize::S64, Reg::Rsp, 8);

            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, size);
            emit.mov_rr(OpSize::S64, TEMP1, Reg::Rax);

            if digit == 0 { emit.inc(size, TEMP1); } else { emit.dec(size, TEMP1); }

            emit.pushfq();
            emit.pop(FLAGS_TMP);

            // Write back
            emit.add_ri(OpSize::S64, Reg::Rsp, 8);
            emit.pop(Reg::Rax); // linear
            emit.push(TEMP1);
            emit.sub_ri(OpSize::S64, Reg::Rsp, 8);
            emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
            emit.mov_ri32(Reg::R8, opsize_bytes(size) as u32);
            emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8);
            let wr = helpers::jit_mem_write as *const () as usize as u64;
            emit.call_abs(wr);
            emit.add_ri(OpSize::S64, Reg::Rsp, 8);
            emit.pop(TEMP1);
            let ok = emit.new_label();
            emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
            emit.jcc_label(Cc::Ne, ok);
            emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
            self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
            emit.bind_label(ok);
        }

        // Merge preserving CF.
        const INC_DEC_MASK: i32 = 0x8D4;
        const NOT_INC_DEC_MASK: i32 = !0x8D4;
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, NOT_INC_DEC_MASK);
        emit.and_ri(OpSize::S64, FLAGS_TMP, INC_DEC_MASK);
        emit.or_rr(OpSize::S64, Reg::Rax, FLAGS_TMP);
        emit.mov_mr(OpSize::S64, GPR_BASE, RFLAGS_OFFSET, Reg::Rax);
        state.flags_dirty = false;

        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: MOV
    // ════════════════════════════════════════════════════════════════════

    fn try_mov_rm_r(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_rm();
            let src_idx = inst.modrm_reg();
            self.emit_load_gpr(emit, TEMP1, src_idx, size);
            self.emit_store_gpr(emit, dst_idx, TEMP1, size);
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            // MOV [mem], r — store register to memory
            let mem_op = match &inst.operands[0] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            let src_idx = inst.modrm_reg();
            // Save source value before computing address (clobbers TEMP1)
            self.emit_load_gpr(emit, TEMP2, src_idx, size);
            // Compute linear address into RAX
            self.emit_compute_linear(emit, mem_op, inst);
            // Push value onto stack to preserve across call
            emit.push(TEMP2);
            emit.sub_ri(OpSize::S64, Reg::Rsp, 8); // align
            // jit_mem_write(cpu, mem, mmu, linear, size, value)
            emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax); // linear
            emit.mov_ri32(Reg::R8, opsize_bytes(size) as u32);
            // Restore value into R9 (6th arg)
            emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8); // value from stack
            let wr = helpers::jit_mem_write as *const () as usize as u64;
            emit.call_abs(wr);
            emit.add_ri(OpSize::S64, Reg::Rsp, 8); // undo align
            emit.pop(TEMP2); // clean stack

            let ok = emit.new_label();
            emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
            emit.jcc_label(Cc::Ne, ok);
            emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
            self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
            emit.bind_label(ok);

            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    fn try_mov_r_rm(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_reg();
            let src_idx = inst.modrm_rm();
            self.emit_load_gpr(emit, TEMP1, src_idx, size);
            self.emit_store_gpr(emit, dst_idx, TEMP1, size);
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            // MOV r, [mem] — load from memory
            let mem_op = match &inst.operands[1] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            let dst_idx = inst.modrm_reg();
            self.emit_compute_linear(emit, mem_op, inst);
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, size);
            self.emit_store_gpr(emit, dst_idx, Reg::Rax, size);
            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    fn emit_mov_r_imm(&self, emit: &mut Emitter, inst: &DecodedInst) {
        let reg_idx = (inst.opcode as u8 & 7) | if inst.prefix.rex_b() { 8 } else { 0 };
        let imm = inst.immediate;
        match inst.operand_size {
            OperandSize::Qword => {
                emit.mov_ri64(TEMP1, imm);
                emit.mov_mr(OpSize::S64, GPR_BASE, reg_idx as i32 * 8, TEMP1);
            }
            OperandSize::Dword => {
                emit.mov_ri32(TEMP1, imm as u32);
                emit.mov_mr(OpSize::S64, GPR_BASE, reg_idx as i32 * 8, TEMP1);
            }
            OperandSize::Word => {
                emit.mov_ri32(TEMP1, imm as u32);
                emit.mov_mr(OpSize::S16, GPR_BASE, reg_idx as i32 * 8, TEMP1);
            }
            OperandSize::Byte => {}
        }
        self.emit_advance_rip(emit, inst.length);
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: LEA
    // ════════════════════════════════════════════════════════════════════

    fn try_lea(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let mem_op = match &inst.operands[1] {
            Operand::Memory(m) => m,
            _ => return false,
        };
        let dst_idx = inst.modrm_reg();
        self.emit_compute_ea_no_segment(emit, mem_op, inst);
        let addr_mask = inst.address_size.mask();
        if addr_mask != u64::MAX {
            emit.mov_ri64(TEMP1, addr_mask);
            emit.and_rr(OpSize::S64, Reg::Rax, TEMP1);
        }
        self.emit_store_gpr(emit, dst_idx, Reg::Rax, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: XCHG
    // ════════════════════════════════════════════════════════════════════

    fn try_xchg_rm_r(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let a_idx = inst.modrm_rm();
        let b_idx = inst.modrm_reg();
        self.emit_load_gpr(emit, TEMP1, a_idx, size);
        self.emit_load_gpr(emit, TEMP2, b_idx, size);
        self.emit_store_gpr(emit, a_idx, TEMP2, size);
        self.emit_store_gpr(emit, b_idx, TEMP1, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: MOVZX / MOVSX
    // ════════════════════════════════════════════════════════════════════

    fn try_movzx_r_rm8(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let dst_size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let dst_idx = inst.modrm_reg();
        if inst.modrm_mod() == 3 {
            let src_idx = inst.modrm_rm();
            emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, src_idx as i32 * 8);
            emit.and_ri(OpSize::S64, TEMP1, 0xFF);
            self.emit_store_gpr(emit, dst_idx, TEMP1, dst_size);
        } else {
            let mem_op = match &inst.operands[1] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            self.emit_compute_linear(emit, mem_op, inst);
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, OpSize::S8);
            emit.and_ri(OpSize::S64, Reg::Rax, 0xFF);
            self.emit_store_gpr(emit, dst_idx, Reg::Rax, dst_size);
        }
        self.emit_advance_rip(emit, inst.length);
        true
    }

    fn try_movzx_r_rm16(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let dst_size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let dst_idx = inst.modrm_reg();
        let src_idx = inst.modrm_rm();
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, src_idx as i32 * 8);
        emit.and_ri(OpSize::S64, TEMP1, 0xFFFF);
        self.emit_store_gpr(emit, dst_idx, TEMP1, dst_size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    fn try_movsx_r_rm8(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let dst_size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let dst_idx = inst.modrm_reg();
        if inst.modrm_mod() == 3 {
            let src_idx = inst.modrm_rm();
            emit.mov_rm(OpSize::S64, TEMP2, GPR_BASE, src_idx as i32 * 8);
            emit.movsx_rr8(dst_size, TEMP1, TEMP2);
        } else {
            let mem_op = match &inst.operands[1] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            self.emit_compute_linear(emit, mem_op, inst);
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, OpSize::S8);
            emit.movsx_rr8(dst_size, TEMP1, Reg::Rax);
        }
        self.emit_store_gpr(emit, dst_idx, TEMP1, dst_size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: Shifts
    // ════════════════════════════════════════════════════════════════════

    fn try_shift_ri(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let digit = inst.modrm_reg() & 7;
        let reg_idx = inst.modrm_rm();
        let count = inst.immediate as u8;
        match digit { 4 | 5 | 7 => {} _ => return false }

        state.flags_dirty = false;
        self.emit_load_gpr(emit, TEMP1, reg_idx, size);
        match digit {
            4 => emit.shl_ri(size, TEMP1, count),
            5 => emit.shr_ri(size, TEMP1, count),
            7 => emit.sar_ri(size, TEMP1, count),
            _ => unreachable!(),
        }
        self.emit_capture_flags(emit, state);
        self.emit_store_gpr(emit, reg_idx, TEMP1, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    fn try_shift_r1(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let digit = inst.modrm_reg() & 7;
        let reg_idx = inst.modrm_rm();
        match digit { 4 | 5 | 7 => {} _ => return false }

        state.flags_dirty = false;
        self.emit_load_gpr(emit, TEMP1, reg_idx, size);
        match digit {
            4 => emit.shl_ri(size, TEMP1, 1),
            5 => emit.shr_ri(size, TEMP1, 1),
            7 => emit.sar_ri(size, TEMP1, 1),
            _ => unreachable!(),
        }
        self.emit_capture_flags(emit, state);
        self.emit_store_gpr(emit, reg_idx, TEMP1, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: PUSH / POP
    // ════════════════════════════════════════════════════════════════════

    fn try_push_r(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        self.flush_lazy_flags(emit, state);
        let reg_idx = (inst.opcode as u8 & 7) | if inst.prefix.rex_b() { 8 } else { 0 };
        let push_size = match inst.operand_size {
            OperandSize::Qword => 8u8,
            OperandSize::Dword => 4,
            OperandSize::Word => 2,
            _ => return false,
        };

        // RSP -= push_size
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8);
        emit.sub_ri(OpSize::S64, Reg::Rax, push_size as i32);
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        emit.mov_mr(OpSize::S64, GPR_BASE, 4 * 8, Reg::Rax);

        // Linear = SS.base + RSP
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);

        // Load value to push
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, reg_idx as i32 * 8);

        // jit_mem_write(cpu, mem, mmu, linear, size, value)
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
        emit.mov_ri32(Reg::R8, push_size as u32);
        let wr = helpers::jit_mem_write as *const () as usize as u64;
        emit.call_abs(wr);

        let ok = emit.new_label();
        emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
        emit.jcc_label(Cc::Ne, ok);
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
        emit.bind_label(ok);

        self.emit_advance_rip(emit, inst.length);
        true
    }

    fn try_pop_r(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        self.flush_lazy_flags(emit, state);
        let reg_idx = (inst.opcode as u8 & 7) | if inst.prefix.rex_b() { 8 } else { 0 };
        let pop_size = match inst.operand_size {
            OperandSize::Qword => 8u8,
            OperandSize::Dword => 4,
            OperandSize::Word => 2,
            _ => return false,
        };

        // Linear = SS.base + RSP
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8);
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);

        let read_size = match pop_size {
            8 => OpSize::S64, 4 => OpSize::S32, 2 => OpSize::S16, _ => return false,
        };
        self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, read_size);

        self.emit_store_gpr(emit, reg_idx, Reg::Rax, read_size);

        // RSP += pop_size
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, 4 * 8);
        emit.add_ri(OpSize::S64, TEMP1, pop_size as i32);
        emit_mask_sp(emit, self.current_mode, TEMP1);
        emit.mov_mr(OpSize::S64, GPR_BASE, 4 * 8, TEMP1);

        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: CALL / RET
    // ════════════════════════════════════════════════════════════════════

    fn try_call_rel(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        self.flush_lazy_flags(emit, state);
        let rel = match inst.operands[0] {
            Operand::RelativeOffset(off) => off,
            _ => return false,
        };
        let push_size = match inst.operand_size {
            OperandSize::Qword => 8u8,
            OperandSize::Dword => 4,
            OperandSize::Word => 2,
            _ => return false,
        };

        // return_addr = RIP + inst.length
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
        emit.add_ri(OpSize::S64, Reg::Rax, inst.length as i32);
        emit.mov_rr(OpSize::S64, TEMP1, Reg::Rax); // save return addr

        // RSP -= push_size
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8);
        emit.sub_ri(OpSize::S64, Reg::Rax, push_size as i32);
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        emit.mov_mr(OpSize::S64, GPR_BASE, 4 * 8, Reg::Rax);

        // Linear = SS.base + RSP
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP2, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP2);

        // jit_mem_write(cpu, mem, mmu, linear, size, value=return_addr)
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
        emit.mov_ri32(Reg::R8, push_size as u32);
        let wr = helpers::jit_mem_write as *const () as usize as u64;
        emit.call_abs(wr);

        let ok = emit.new_label();
        emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
        emit.jcc_label(Cc::Ne, ok);
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
        emit.bind_label(ok);

        let delta = inst.length as i64 + rel;
        self.emit_advance_rip_by(emit, delta);
        true
    }

    fn try_ret_near(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        self.flush_lazy_flags(emit, state);
        let pop_size = match inst.operand_size {
            OperandSize::Qword => 8u8,
            OperandSize::Dword => 4,
            OperandSize::Word => 2,
            _ => return false,
        };

        // Linear = SS.base + RSP
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8);
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);

        let read_size = match pop_size {
            8 => OpSize::S64, 4 => OpSize::S32, 2 => OpSize::S16, _ => return false,
        };
        self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, read_size);

        emit_mask_rip(emit, self.current_mode);
        emit.mov_mr(OpSize::S64, GPR_BASE, RIP_OFFSET, Reg::Rax);

        // RSP += pop_size
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, 4 * 8);
        emit.add_ri(OpSize::S64, TEMP1, pop_size as i32);
        emit_mask_sp(emit, self.current_mode, TEMP1);
        emit.mov_mr(OpSize::S64, GPR_BASE, 4 * 8, TEMP1);

        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: Branches
    // ════════════════════════════════════════════════════════════════════

    fn emit_jcc(
        &self, emit: &mut Emitter, inst: &DecodedInst, cc: u8, state: &mut BlockState,
    ) {
        let rel = match inst.operands[0] {
            Operand::RelativeOffset(off) => off,
            _ => {
                self.emit_advance_rip(emit, inst.length);
                return;
            }
        };

        self.flush_lazy_flags(emit, state);

        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, ARITH_MASK | 0x02);
        emit.push(Reg::Rax);
        emit.popfq();

        let taken_label = emit.new_label();
        emit.jcc_label(map_cc(cc), taken_label);

        // Not-taken
        self.emit_advance_rip(emit, inst.length);
        let done_label = emit.new_label();
        emit.jmp_label(done_label);

        // Taken
        emit.bind_label(taken_label);
        let delta = inst.length as i64 + rel;
        self.emit_advance_rip_by(emit, delta);

        emit.bind_label(done_label);
    }

    fn emit_jmp_rel(&self, emit: &mut Emitter, inst: &DecodedInst) {
        let rel = match inst.operands[0] {
            Operand::RelativeOffset(off) => off,
            _ => {
                self.emit_advance_rip(emit, inst.length);
                return;
            }
        };
        let delta = inst.length as i64 + rel;
        self.emit_advance_rip_by(emit, delta);
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: 8-bit MOV
    // ════════════════════════════════════════════════════════════════════

    /// MOV r/m8, r8 (0x88)
    fn try_mov_rm8_r8(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let has_rex = inst.prefix.has_rex();
        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_rm();
            let src_idx = inst.modrm_reg();
            self.emit_load_gpr8(emit, TEMP1, src_idx, has_rex);
            self.emit_store_gpr8(emit, dst_idx, TEMP1, has_rex);
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            let mem_op = match &inst.operands[0] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            let src_idx = inst.modrm_reg();
            self.emit_load_gpr8(emit, TEMP2, src_idx, has_rex);
            self.emit_compute_linear(emit, mem_op, inst);
            emit.push(TEMP2);
            emit.sub_ri(OpSize::S64, Reg::Rsp, 8);
            emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
            emit.mov_ri32(Reg::R8, 1);
            emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8);
            let wr = helpers::jit_mem_write as *const () as usize as u64;
            emit.call_abs(wr);
            emit.add_ri(OpSize::S64, Reg::Rsp, 8);
            emit.pop(TEMP2);
            let ok = emit.new_label();
            emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
            emit.jcc_label(Cc::Ne, ok);
            emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
            self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
            emit.bind_label(ok);
            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    /// MOV r8, r/m8 (0x8A)
    fn try_mov_r8_rm8(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let has_rex = inst.prefix.has_rex();
        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_reg();
            let src_idx = inst.modrm_rm();
            self.emit_load_gpr8(emit, TEMP1, src_idx, has_rex);
            self.emit_store_gpr8(emit, dst_idx, TEMP1, has_rex);
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            let mem_op = match &inst.operands[1] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            let dst_idx = inst.modrm_reg();
            self.emit_compute_linear(emit, mem_op, inst);
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, OpSize::S8);
            self.emit_store_gpr8(emit, dst_idx, Reg::Rax, has_rex);
            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: 8-bit ALU / TEST / CMP
    // ════════════════════════════════════════════════════════════════════

    /// CMP AL, imm8 (0x3C)
    fn try_cmp_al_imm8(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        state.flags_dirty = false;
        self.emit_load_gpr(emit, TEMP1, 0, OpSize::S8);
        emit.and_ri(OpSize::S64, TEMP1, 0xFF);
        let imm = inst.immediate as u8;
        emit.cmp_ri(OpSize::S8, TEMP1, imm as i32);
        self.emit_capture_flags(emit, state);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// Group 1: ALU r/m8, imm8 (0x80)
    fn try_group1_8bit(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let digit = inst.modrm_reg() & 7;
        let alu_op = match digit {
            0 => AluOp::Add,
            1 => AluOp::Or,
            4 => AluOp::And,
            5 => AluOp::Sub,
            6 => AluOp::Xor,
            7 => AluOp::Cmp,
            _ => return false,
        };
        let imm = inst.immediate as u8 as i32;

        let has_rex = inst.prefix.has_rex();
        if inst.modrm_mod() == 3 {
            let raw_idx = inst.modrm_rm();
            state.flags_dirty = false;
            self.emit_load_gpr8(emit, TEMP1, raw_idx, has_rex);
            emit_native_alu_ri(emit, alu_op, OpSize::S8, TEMP1, imm);
            self.emit_capture_flags(emit, state);
            if alu_op != AluOp::Cmp {
                self.emit_store_gpr8(emit, raw_idx, TEMP1, has_rex);
            }
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            // ALU [mem8], imm8 — read-modify-write
            let mem_op = match &inst.operands[0] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            state.flags_dirty = false;

            // Compute linear address
            self.emit_compute_linear(emit, mem_op, inst);
            // Save linear address on stack
            emit.push(Reg::Rax);
            emit.sub_ri(OpSize::S64, Reg::Rsp, 8); // align

            // Read current byte
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, OpSize::S8);
            emit.and_ri(OpSize::S64, Reg::Rax, 0xFF);
            emit.mov_rr(OpSize::S64, TEMP1, Reg::Rax);

            // Perform ALU operation
            emit_native_alu_ri(emit, alu_op, OpSize::S8, TEMP1, imm);
            self.emit_capture_flags(emit, state);

            if alu_op != AluOp::Cmp {
                // Write back
                emit.add_ri(OpSize::S64, Reg::Rsp, 8);
                emit.pop(Reg::Rax); // linear address
                emit.push(TEMP1);
                emit.sub_ri(OpSize::S64, Reg::Rsp, 8);
                emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
                emit.mov_ri32(Reg::R8, 1);
                emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8);
                let wr = helpers::jit_mem_write as *const () as usize as u64;
                emit.call_abs(wr);
                emit.add_ri(OpSize::S64, Reg::Rsp, 8);
                emit.pop(TEMP1);
                let ok = emit.new_label();
                emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
                emit.jcc_label(Cc::Ne, ok);
                emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
                self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
                emit.bind_label(ok);
            } else {
                // CMP: no write-back, just clean up stack
                emit.add_ri(OpSize::S64, Reg::Rsp, 8);
                emit.pop(Reg::Rax);
            }

            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    /// TEST r/m8, r8 (0x84), reg-reg only
    fn try_test_rm8_r8(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let a_idx = inst.modrm_rm();
        let b_idx = inst.modrm_reg();
        let has_rex = inst.prefix.has_rex();
        state.flags_dirty = false;
        self.emit_load_gpr8(emit, TEMP1, a_idx, has_rex);
        self.emit_load_gpr8(emit, TEMP2, b_idx, has_rex);
        emit.test_rr(OpSize::S8, TEMP1, TEMP2);
        self.emit_capture_flags(emit, state);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: 8-bit ALU r/m8,r8 and r8,r/m8
    // ════════════════════════════════════════════════════════════════════

    /// ALU r/m8, r8 (0x00/0x08/0x20/0x28/0x30/0x38)
    fn try_alu_rm8_r8(
        &self, emit: &mut Emitter, inst: &DecodedInst, alu_op: AluOp, state: &mut BlockState,
    ) -> bool {
        let has_rex = inst.prefix.has_rex();
        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_rm();
            let src_idx = inst.modrm_reg();
            state.flags_dirty = false;
            self.emit_load_gpr8(emit, TEMP1, dst_idx, has_rex);
            self.emit_load_gpr8(emit, TEMP2, src_idx, has_rex);
            emit_native_alu_rr(emit, alu_op, OpSize::S8, TEMP1, TEMP2);
            self.emit_capture_flags(emit, state);
            if alu_op != AluOp::Cmp {
                self.emit_store_gpr8(emit, dst_idx, TEMP1, has_rex);
            }
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            // ALU [mem8], r8 — read-modify-write
            let mem_op = match &inst.operands[0] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            let src_idx = inst.modrm_reg();
            self.flush_lazy_flags(emit, state);
            state.flags_dirty = false;
            self.emit_load_gpr8(emit, TEMP2, src_idx, has_rex);
            self.emit_compute_linear(emit, mem_op, inst);
            emit.push(Reg::Rax);
            emit.push(TEMP2);
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, OpSize::S8);
            emit.and_ri(OpSize::S64, Reg::Rax, 0xFF);
            emit.mov_rr(OpSize::S64, TEMP1, Reg::Rax);
            emit.pop(TEMP2);
            emit_native_alu_rr(emit, alu_op, OpSize::S8, TEMP1, TEMP2);
            self.emit_capture_flags(emit, state);
            if alu_op != AluOp::Cmp {
                emit.pop(Reg::Rax); // linear
                emit.push(TEMP1);
                emit.sub_ri(OpSize::S64, Reg::Rsp, 8);
                emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
                emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
                emit.mov_ri32(Reg::R8, 1);
                emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8);
                let wr = helpers::jit_mem_write as *const () as usize as u64;
                emit.call_abs(wr);
                emit.add_ri(OpSize::S64, Reg::Rsp, 8);
                emit.pop(TEMP1);
                let ok = emit.new_label();
                emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
                emit.jcc_label(Cc::Ne, ok);
                emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
                self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
                emit.bind_label(ok);
            } else {
                emit.pop(Reg::Rax); // discard linear
            }
            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    /// ALU r8, r/m8 (0x02/0x0A/0x22/0x2A/0x32/0x3A)
    fn try_alu_r8_rm8(
        &self, emit: &mut Emitter, inst: &DecodedInst, alu_op: AluOp, state: &mut BlockState,
    ) -> bool {
        let dst_idx = inst.modrm_reg();
        let has_rex = inst.prefix.has_rex();
        if inst.modrm_mod() == 3 {
            let src_idx = inst.modrm_rm();
            state.flags_dirty = false;
            self.emit_load_gpr8(emit, TEMP1, dst_idx, has_rex);
            self.emit_load_gpr8(emit, TEMP2, src_idx, has_rex);
            emit_native_alu_rr(emit, alu_op, OpSize::S8, TEMP1, TEMP2);
            self.emit_capture_flags(emit, state);
            if alu_op != AluOp::Cmp {
                self.emit_store_gpr8(emit, dst_idx, TEMP1, has_rex);
            }
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            // ALU r8, [mem8]
            let mem_op = match &inst.operands[1] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            state.flags_dirty = false;
            self.emit_compute_linear(emit, mem_op, inst);
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, OpSize::S8);
            emit.and_ri(OpSize::S64, Reg::Rax, 0xFF);
            emit.mov_rr(OpSize::S64, TEMP2, Reg::Rax);
            self.emit_load_gpr8(emit, TEMP1, dst_idx, has_rex);
            emit_native_alu_rr(emit, alu_op, OpSize::S8, TEMP1, TEMP2);
            self.emit_capture_flags(emit, state);
            if alu_op != AluOp::Cmp {
                self.emit_store_gpr8(emit, dst_idx, TEMP1, has_rex);
            }
            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: MOV moffs
    // ════════════════════════════════════════════════════════════════════

    /// MOV eAX, moffs (0xA1)
    fn try_mov_eax_moffs(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let mem_op = match &inst.operands[1] {
            Operand::Memory(m) => m,
            _ => return false,
        };
        self.flush_lazy_flags(emit, state);
        self.emit_compute_linear(emit, mem_op, inst);
        self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, size);
        self.emit_store_gpr(emit, 0, Reg::Rax, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// MOV moffs, eAX (0xA3)
    fn try_mov_moffs_eax(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let mem_op = match &inst.operands[0] {
            Operand::Memory(m) => m,
            _ => return false,
        };
        self.flush_lazy_flags(emit, state);
        // Load value from EAX first
        self.emit_load_gpr(emit, TEMP2, 0, size);
        // Compute linear address
        self.emit_compute_linear(emit, mem_op, inst);
        // jit_mem_write(cpu, mem, mmu, linear, size, value)
        emit.push(TEMP2);
        emit.sub_ri(OpSize::S64, Reg::Rsp, 8);
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
        emit.mov_ri32(Reg::R8, opsize_bytes(size) as u32);
        emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8);
        let wr = helpers::jit_mem_write as *const () as usize as u64;
        emit.call_abs(wr);
        emit.add_ri(OpSize::S64, Reg::Rsp, 8);
        emit.pop(TEMP2);
        let ok = emit.new_label();
        emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
        emit.jcc_label(Cc::Ne, ok);
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
        emit.bind_label(ok);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: INC/DEC short form, PUSH imm, RET imm, MOV r/m imm
    // ════════════════════════════════════════════════════════════════════

    /// INC/DEC r short form (0x40-0x47 / 0x48-0x4F) — Protected32 only
    fn try_inc_dec_short(
        &self, emit: &mut Emitter, inst: &DecodedInst, op: u8, is_inc: bool, state: &mut BlockState,
    ) -> bool {
        // In Long64 mode, 0x40-0x4F are REX prefixes, not INC/DEC
        if self.current_mode == CpuMode::Long64 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let reg_idx = op & 7;

        self.flush_lazy_flags(emit, state);
        self.emit_load_gpr(emit, TEMP1, reg_idx, size);
        if is_inc { emit.inc(size, TEMP1); } else { emit.dec(size, TEMP1); }

        emit.pushfq();
        emit.pop(FLAGS_TMP);
        self.emit_store_gpr(emit, reg_idx, TEMP1, size);

        const INC_DEC_MASK: i32 = 0x8D4;
        const NOT_INC_DEC_MASK: i32 = !0x8D4;
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, NOT_INC_DEC_MASK);
        emit.and_ri(OpSize::S64, FLAGS_TMP, INC_DEC_MASK);
        emit.or_rr(OpSize::S64, Reg::Rax, FLAGS_TMP);
        emit.mov_mr(OpSize::S64, GPR_BASE, RFLAGS_OFFSET, Reg::Rax);
        state.flags_dirty = false;

        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// PUSH imm32 (0x68) or PUSH imm8 sign-extended (0x6A)
    fn try_push_imm(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState, _is_imm8: bool,
    ) -> bool {
        if self.current_mode == CpuMode::Real16 { return false; }
        self.flush_lazy_flags(emit, state);
        let push_size = match inst.operand_size {
            OperandSize::Qword => 8u8,
            OperandSize::Dword => 4,
            OperandSize::Word => 2,
            _ => return false,
        };
        let imm = inst.immediate;

        // RSP -= push_size
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8);
        emit.sub_ri(OpSize::S64, Reg::Rax, push_size as i32);
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        emit.mov_mr(OpSize::S64, GPR_BASE, 4 * 8, Reg::Rax);

        // Linear = SS.base + RSP
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);

        // Load immediate value
        emit.mov_ri64(TEMP1, imm);

        // jit_mem_write(cpu, mem, mmu, linear, size, value)
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
        emit.mov_ri32(Reg::R8, push_size as u32);
        let wr = helpers::jit_mem_write as *const () as usize as u64;
        emit.call_abs(wr);

        let ok = emit.new_label();
        emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
        emit.jcc_label(Cc::Ne, ok);
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
        emit.bind_label(ok);

        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// RET near imm16 (0xC2) — like RET but pops imm16 extra bytes from stack.
    fn try_ret_near_imm(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        self.flush_lazy_flags(emit, state);
        let pop_size = match inst.operand_size {
            OperandSize::Qword => 8u8,
            OperandSize::Dword => 4,
            OperandSize::Word => 2,
            _ => return false,
        };
        let extra = inst.immediate as u16;

        // Read return address from [SS:RSP]
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8);
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);

        let read_size = match pop_size {
            8 => OpSize::S64, 4 => OpSize::S32, 2 => OpSize::S16, _ => return false,
        };
        self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, read_size);
        emit_mask_rip(emit, self.current_mode);
        emit.mov_mr(OpSize::S64, GPR_BASE, RIP_OFFSET, Reg::Rax);

        // RSP += pop_size + extra
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, 4 * 8);
        emit.add_ri(OpSize::S64, TEMP1, pop_size as i32 + extra as i32);
        emit_mask_sp(emit, self.current_mode, TEMP1);
        emit.mov_mr(OpSize::S64, GPR_BASE, 4 * 8, TEMP1);

        true
    }

    /// MOV r/m, imm32 (0xC7)
    fn try_mov_rm_imm(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_rm();
            let imm = inst.immediate;
            match size {
                OpSize::S64 => {
                    emit.mov_ri64(TEMP1, imm);
                    self.emit_store_gpr(emit, dst_idx, TEMP1, size);
                }
                _ => {
                    emit.mov_ri32(TEMP1, imm as u32);
                    self.emit_store_gpr(emit, dst_idx, TEMP1, size);
                }
            }
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            // MOV [mem], imm
            let mem_op = match &inst.operands[0] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            let imm = inst.immediate;
            emit.mov_ri64(TEMP2, imm);
            self.emit_compute_linear(emit, mem_op, inst);
            emit.push(TEMP2);
            emit.sub_ri(OpSize::S64, Reg::Rsp, 8);
            emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
            emit.mov_ri32(Reg::R8, opsize_bytes(size) as u32);
            emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8);
            let wr = helpers::jit_mem_write as *const () as usize as u64;
            emit.call_abs(wr);
            emit.add_ri(OpSize::S64, Reg::Rsp, 8);
            emit.pop(TEMP2);
            let ok = emit.new_label();
            emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
            emit.jcc_label(Cc::Ne, ok);
            emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
            self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
            emit.bind_label(ok);
            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Memory Access Helpers
    // ════════════════════════════════════════════════════════════════════

    /// Call jit_mem_read with linear address in the given register.
    /// After the call, checks cpu.jit_fault and exits block if set.
    /// Result in RAX.
    fn emit_call_mem_read_with_linear_and_fault_check(
        &self, emit: &mut Emitter, linear_reg: Reg, size: OpSize,
    ) {
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        if linear_reg != Reg::Rcx {
            emit.mov_rr(OpSize::S64, Reg::Rcx, linear_reg);
        }
        emit.mov_ri32(Reg::R8, opsize_bytes(size) as u32);
        let rd = helpers::jit_mem_read as *const () as usize as u64;
        emit.call_abs(rd);

        // Check cpu.jit_fault
        let fault_offset = core::mem::offset_of!(crate::cpu::Cpu, jit_fault) as i32;
        let ok = emit.new_label();
        emit.movzx_rm8(OpSize::S32, TEMP1, CPU_PTR, fault_offset);
        emit.test_rr(OpSize::S32, TEMP1, TEMP1);
        emit.jcc_label(Cc::E, ok);
        emit.mov_mi32(OpSize::S8, CPU_PTR, fault_offset, 0);
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
        emit.bind_label(ok);
    }

    // ════════════════════════════════════════════════════════════════════
    // Address Computation
    // ════════════════════════════════════════════════════════════════════

    /// Compute linear address for a memory operand into RAX.
    /// Linear = segment_base + effective_address.
    fn emit_compute_linear(&self, emit: &mut Emitter, mem_op: &MemOperand, inst: &DecodedInst) {
        self.emit_compute_ea_no_segment(emit, mem_op, inst);
        // Mask to address size
        let addr_mask = inst.address_size.mask();
        if addr_mask != u64::MAX {
            emit.mov_ri64(TEMP1, addr_mask);
            emit.and_rr(OpSize::S64, Reg::Rax, TEMP1);
        }
        // Add segment base
        let seg = mem_op.segment;
        let seg_off = SEG_ARRAY_OFFSET + seg as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, seg_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
    }

    fn emit_compute_ea_no_segment(
        &self, emit: &mut Emitter, mem_op: &MemOperand, inst: &DecodedInst,
    ) {
        if mem_op.rip_relative {
            emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
            emit.add_ri(OpSize::S64, Reg::Rax, inst.length as i32);
            if mem_op.displacement != 0 {
                let d = mem_op.displacement as i32;
                emit.add_ri(OpSize::S64, Reg::Rax, d);
            }
            return;
        }

        if mem_op.displacement != 0 {
            let d = mem_op.displacement;
            if d < 0 {
                emit.mov_ri64(Reg::Rax, d as u64);
            } else if d >= i32::MIN as i64 && d <= i32::MAX as i64 {
                emit.mov_ri32(Reg::Rax, d as u32);
            } else {
                emit.mov_ri64(Reg::Rax, d as u64);
            }
        } else {
            emit.xor_rr(OpSize::S32, Reg::Rax, Reg::Rax);
        }

        if let Some(base) = mem_op.base {
            emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, base as i32 * 8);
            emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
        }

        if let Some(index) = mem_op.index {
            emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, index as i32 * 8);
            match mem_op.scale {
                1 => emit.add_rr(OpSize::S64, Reg::Rax, TEMP1),
                2 => {
                    emit.shl_ri(OpSize::S64, TEMP1, 1);
                    emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
                }
                4 => {
                    emit.shl_ri(OpSize::S64, TEMP1, 2);
                    emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
                }
                8 => {
                    emit.shl_ri(OpSize::S64, TEMP1, 3);
                    emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
                }
                _ => {
                    for _ in 0..mem_op.scale {
                        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
                    }
                }
            }
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Common Emission Helpers
    // ════════════════════════════════════════════════════════════════════

    /// Load an 8-bit register, handling high-byte regs (AH/CH/DH/BH) in non-REX mode.
    /// `raw_idx` is the modrm index (0-7), `has_rex` indicates REX prefix presence.
    /// Result in host_dst, low byte only (caller should mask with 0xFF if needed).
    fn emit_load_gpr8(&self, emit: &mut Emitter, host_dst: Reg, raw_idx: u8, has_rex: bool) {
        if !has_rex && raw_idx >= 4 {
            // High-byte: AH(4)=gpr[0]:1, CH(5)=gpr[1]:1, DH(6)=gpr[2]:1, BH(7)=gpr[3]:1
            let gpr_idx = raw_idx - 4;
            let offset = GPR_OFFSET + gpr_idx as i32 * 8;
            emit.mov_rm(OpSize::S64, host_dst, GPR_BASE, offset);
            emit.shr_ri(OpSize::S64, host_dst, 8);
            emit.and_ri(OpSize::S64, host_dst, 0xFF);
        } else {
            let offset = GPR_OFFSET + raw_idx as i32 * 8;
            emit.mov_rm(OpSize::S64, host_dst, GPR_BASE, offset);
            emit.and_ri(OpSize::S64, host_dst, 0xFF);
        }
    }

    /// Store an 8-bit register, handling high-byte regs (AH/CH/DH/BH) in non-REX mode.
    /// `host_src` low byte is the value to store.
    fn emit_store_gpr8(&self, emit: &mut Emitter, raw_idx: u8, host_src: Reg, has_rex: bool) {
        if !has_rex && raw_idx >= 4 {
            // High-byte: merge into byte 1 of gpr[raw_idx-4]
            let gpr_idx = raw_idx - 4;
            let offset = GPR_OFFSET + gpr_idx as i32 * 8;
            emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, offset);
            emit.and_ri(OpSize::S64, Reg::Rax, !0xFF00i32); // clear byte 1
            emit.mov_rr(OpSize::S64, TEMP2, host_src);
            emit.and_ri(OpSize::S64, TEMP2, 0xFF);
            emit.shl_ri(OpSize::S64, TEMP2, 8);
            emit.or_rr(OpSize::S64, Reg::Rax, TEMP2);
            emit.mov_mr(OpSize::S64, GPR_BASE, offset, Reg::Rax);
        } else {
            emit.mov_mr(OpSize::S8, GPR_BASE, GPR_OFFSET + raw_idx as i32 * 8, host_src);
        }
    }

    fn emit_load_gpr(&self, emit: &mut Emitter, host_dst: Reg, guest_idx: u8, size: OpSize) {
        let offset = GPR_OFFSET + guest_idx as i32 * 8;
        match size {
            OpSize::S32 => emit.mov_rm(OpSize::S32, host_dst, GPR_BASE, offset),
            OpSize::S64 | OpSize::S16 | OpSize::S8 => {
                emit.mov_rm(OpSize::S64, host_dst, GPR_BASE, offset);
            }
        }
    }

    fn emit_store_gpr(&self, emit: &mut Emitter, guest_idx: u8, host_src: Reg, size: OpSize) {
        let offset = GPR_OFFSET + guest_idx as i32 * 8;
        match size {
            OpSize::S64 => emit.mov_mr(OpSize::S64, GPR_BASE, offset, host_src),
            OpSize::S32 => {
                emit.mov_rr(OpSize::S32, host_src, host_src); // zero-extend
                emit.mov_mr(OpSize::S64, GPR_BASE, offset, host_src);
            }
            OpSize::S16 => emit.mov_mr(OpSize::S16, GPR_BASE, offset, host_src),
            OpSize::S8 => emit.mov_mr(OpSize::S8, GPR_BASE, offset, host_src),
        }
    }

    fn emit_advance_rip(&self, emit: &mut Emitter, length: u8) {
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
        emit.add_ri(OpSize::S64, Reg::Rax, length as i32);
        emit_mask_rip(emit, self.current_mode);
        emit.mov_mr(OpSize::S64, GPR_BASE, RIP_OFFSET, Reg::Rax);
    }

    fn emit_advance_rip_by(&self, emit: &mut Emitter, delta: i64) {
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
        if delta >= i32::MIN as i64 && delta <= i32::MAX as i64 {
            emit.add_ri(OpSize::S64, Reg::Rax, delta as i32);
        } else {
            emit.mov_ri64(TEMP1, delta as u64);
            emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
        }
        emit_mask_rip(emit, self.current_mode);
        emit.mov_mr(OpSize::S64, GPR_BASE, RIP_OFFSET, Reg::Rax);
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: MOV r8, imm8 / MOV r/m8, imm8
    // ════════════════════════════════════════════════════════════════════

    /// MOV r8, imm8 (0xB0-0xB7)
    fn try_mov_r8_imm8(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        let raw_idx = (inst.opcode as u8 & 7) | if inst.prefix.rex_b() { 8 } else { 0 };
        let imm = inst.immediate as u8;
        let has_rex = inst.prefix.has_rex();
        emit.mov_ri32(TEMP1, imm as u32);
        self.emit_store_gpr8(emit, raw_idx, TEMP1, has_rex);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// MOV r/m8, imm8 (0xC6)
    fn try_mov_rm8_imm8(&self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState) -> bool {
        let has_rex = inst.prefix.has_rex();
        let imm = inst.immediate as u8;
        if inst.modrm_mod() == 3 {
            let dst_idx = inst.modrm_rm();
            emit.mov_ri32(TEMP1, imm as u32);
            self.emit_store_gpr8(emit, dst_idx, TEMP1, has_rex);
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            let mem_op = match &inst.operands[0] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            emit.mov_ri32(TEMP2, imm as u32);
            self.emit_compute_linear(emit, mem_op, inst);
            emit.push(TEMP2);
            emit.sub_ri(OpSize::S64, Reg::Rsp, 8);
            emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
            emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);
            emit.mov_ri32(Reg::R8, 1);
            emit.mov_rm(OpSize::S64, TEMP1, Reg::Rsp, 8);
            let wr = helpers::jit_mem_write as *const () as usize as u64;
            emit.call_abs(wr);
            emit.add_ri(OpSize::S64, Reg::Rsp, 8);
            emit.pop(TEMP2);
            let ok = emit.new_label();
            emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
            emit.jcc_label(Cc::Ne, ok);
            emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
            self.emit_exit_to_dispatcher(emit, self.dispatch_loop_addr);
            emit.bind_label(ok);
            self.emit_advance_rip(emit, inst.length);
            true
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: Shift by CL
    // ════════════════════════════════════════════════════════════════════

    /// Shift r/m by CL (0xD3), reg-reg only
    fn try_shift_rcl(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let digit = inst.modrm_reg() & 7;
        let reg_idx = inst.modrm_rm();
        match digit { 4 | 5 | 7 => {} _ => return false }

        state.flags_dirty = false;
        self.emit_load_gpr(emit, TEMP1, reg_idx, size);
        // Load CL (gpr[1] low byte) into RCX for variable shift
        emit.mov_rm(OpSize::S64, Reg::Rcx, GPR_BASE, GPR_OFFSET + 1 * 8);
        match digit {
            4 => emit.shl_cl(size, TEMP1),
            5 => emit.shr_cl(size, TEMP1),
            7 => emit.sar_cl(size, TEMP1),
            _ => unreachable!(),
        }
        self.emit_capture_flags(emit, state);
        self.emit_store_gpr(emit, reg_idx, TEMP1, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: Group 3 (TEST/NOT/NEG)
    // ════════════════════════════════════════════════════════════════════

    /// Group 3: TEST r/m, imm (digit 0), NOT r/m (digit 2), NEG r/m (digit 3) — 0xF7
    fn try_group3(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let digit = inst.modrm_reg() & 7;
        let reg_idx = inst.modrm_rm();

        match digit {
            0 => {
                // TEST r/m, imm
                state.flags_dirty = false;
                self.emit_load_gpr(emit, TEMP1, reg_idx, size);
                let imm = inst.immediate as i64 as i32;
                emit.mov_ri32(TEMP2, imm as u32);
                emit.test_rr(size, TEMP1, TEMP2);
                self.emit_capture_flags(emit, state);
                self.emit_advance_rip(emit, inst.length);
                true
            }
            2 => {
                // NOT r/m
                self.emit_load_gpr(emit, TEMP1, reg_idx, size);
                emit.not(size, TEMP1);
                self.emit_store_gpr(emit, reg_idx, TEMP1, size);
                self.emit_advance_rip(emit, inst.length);
                true
            }
            3 => {
                // NEG r/m
                state.flags_dirty = false;
                self.emit_load_gpr(emit, TEMP1, reg_idx, size);
                emit.neg(size, TEMP1);
                self.emit_capture_flags(emit, state);
                self.emit_store_gpr(emit, reg_idx, TEMP1, size);
                self.emit_advance_rip(emit, inst.length);
                true
            }
            _ => false,
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: CMOVcc / SETcc / IMUL / MOVSX r16
    // ════════════════════════════════════════════════════════════════════

    /// CMOVcc r, r/m (0F 40-4F), reg-reg only
    fn try_cmovcc(
        &self, emit: &mut Emitter, inst: &DecodedInst, op: u8, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let cc = (op - 0x40) as u8;
        let dst_idx = inst.modrm_reg();
        let src_idx = inst.modrm_rm();

        self.flush_lazy_flags(emit, state);

        // Restore host flags from guest RFLAGS
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, ARITH_MASK | 0x02);
        emit.push(Reg::Rax);
        emit.popfq();

        self.emit_load_gpr(emit, TEMP1, dst_idx, size);
        self.emit_load_gpr(emit, TEMP2, src_idx, size);
        emit.cmovcc(map_cc(cc), size, TEMP1, TEMP2);
        self.emit_store_gpr(emit, dst_idx, TEMP1, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// SETcc r/m8 (0F 90-9F), reg only
    fn try_setcc(
        &self, emit: &mut Emitter, inst: &DecodedInst, op: u8, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let cc = (op - 0x90) as u8;
        let dst_idx = inst.modrm_rm();
        let has_rex = inst.prefix.has_rex();

        self.flush_lazy_flags(emit, state);

        // Restore host flags from guest RFLAGS
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, ARITH_MASK | 0x02);
        emit.push(Reg::Rax);
        emit.popfq();

        emit.setcc(map_cc(cc), TEMP1);
        self.emit_store_gpr8(emit, dst_idx, TEMP1, has_rex);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// IMUL r, r/m (0F AF), reg-reg only
    fn try_imul_r_rm(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let dst_idx = inst.modrm_reg();
        let src_idx = inst.modrm_rm();

        state.flags_dirty = false;
        self.emit_load_gpr(emit, TEMP1, dst_idx, size);
        self.emit_load_gpr(emit, TEMP2, src_idx, size);
        emit.imul_rr(size, TEMP1, TEMP2);
        self.emit_capture_flags(emit, state);
        self.emit_store_gpr(emit, dst_idx, TEMP1, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// MOVSX r, r/m16 (0F BF)
    fn try_movsx_r_rm16(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        let dst_size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let dst_idx = inst.modrm_reg();
        if inst.modrm_mod() == 3 {
            let src_idx = inst.modrm_rm();
            // Load 16-bit value, sign-extend manually
            emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, src_idx as i32 * 8);
            emit.and_ri(OpSize::S64, TEMP1, 0xFFFF);
            // Sign-extend from 16 bits: shift left 48, arithmetic shift right 48
            emit.shl_ri(OpSize::S64, TEMP1, 48);
            emit.sar_ri(OpSize::S64, TEMP1, 48);
            self.emit_store_gpr(emit, dst_idx, TEMP1, dst_size);
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            let mem_op = match &inst.operands[1] {
                Operand::Memory(m) => m,
                _ => return false,
            };
            self.flush_lazy_flags(emit, state);
            self.emit_compute_linear(emit, mem_op, inst);
            self.emit_call_mem_read_with_linear_and_fault_check(emit, Reg::Rax, OpSize::S16);
            emit.and_ri(OpSize::S64, Reg::Rax, 0xFFFF);
            emit.shl_ri(OpSize::S64, Reg::Rax, 48);
            emit.sar_ri(OpSize::S64, Reg::Rax, 48);
            self.emit_store_gpr(emit, dst_idx, Reg::Rax, dst_size);
            self.emit_advance_rip(emit, inst.length);
            true
        }
    }
}

// ════════════════════════════════════════════════════════════════════════
// Free functions
// ════════════════════════════════════════════════════════════════════════

/// Determine if an instruction modifies control flow (branches, CALL, RET,
/// INT, IRET, HLT, etc.). Used to decide whether to exit the JIT block
/// after a helper_execute_one call.
fn is_control_flow_instruction(inst: &DecodedInst) -> bool {
    if inst.rep != RepPrefix::None { return true; }
    match inst.opcode_map {
        OpcodeMap::Primary => {
            let op = inst.opcode as u8;
            match op {
                0x70..=0x7F => true,    // Jcc rel8
                0x9A => true,           // CALL far
                0xC2 | 0xC3 => true,    // RET
                0xCA | 0xCB => true,    // RETF
                0xCC | 0xCD | 0xCE => true, // INT
                0xCF => true,           // IRET
                0xE0..=0xE3 => true,    // LOOP/JCXZ
                0xE8 => true,           // CALL rel
                0xE9 | 0xEA | 0xEB => true, // JMP
                0xE4..=0xE7 => true,    // IN/OUT imm8
                0xEC..=0xEF => true,    // IN/OUT DX
                0xF4 => true,           // HLT
                0xFA | 0xFB => true,    // CLI, STI
                0xFF => {
                    let reg = inst.modrm_reg() & 7;
                    matches!(reg, 2 | 3 | 4 | 5) // CALL/JMP indirect
                }
                _ => false,
            }
        }
        OpcodeMap::Secondary => {
            let op = inst.opcode as u8;
            match op {
                0x80..=0x8F => true,    // Jcc rel32
                0x05 | 0x07 => true,    // SYSCALL/SYSRET
                0x00 => {
                    let reg = inst.modrm_reg() & 7;
                    matches!(reg, 2 | 3) // LLDT, LTR
                }
                0x01 => {
                    let reg = inst.modrm_reg() & 7;
                    matches!(reg, 2 | 3 | 7) // LGDT, LIDT, INVLPG
                }
                0x20..=0x23 => true,    // MOV CR/DR
                0x30 | 0x32 => true,    // WRMSR/RDMSR
                0x09 => true,           // WBINVD
                _ => false,
            }
        }
        _ => false,
    }
}

fn map_operand_size(size: OperandSize) -> Option<OpSize> {
    match size {
        OperandSize::Word => Some(OpSize::S16),
        OperandSize::Dword => Some(OpSize::S32),
        OperandSize::Qword => Some(OpSize::S64),
        OperandSize::Byte => None,
    }
}

fn opsize_bytes(size: OpSize) -> u8 {
    match size {
        OpSize::S8 => 1, OpSize::S16 => 2, OpSize::S32 => 4, OpSize::S64 => 8,
    }
}

fn emit_native_alu_rr(emit: &mut Emitter, op: AluOp, size: OpSize, dst: Reg, src: Reg) {
    match op {
        AluOp::Add => emit.add_rr(size, dst, src),
        AluOp::Or => emit.or_rr(size, dst, src),
        AluOp::And => emit.and_rr(size, dst, src),
        AluOp::Sub => emit.sub_rr(size, dst, src),
        AluOp::Xor => emit.xor_rr(size, dst, src),
        AluOp::Cmp => emit.cmp_rr(size, dst, src),
    }
}

fn emit_native_alu_ri(emit: &mut Emitter, op: AluOp, size: OpSize, dst: Reg, imm: i32) {
    match op {
        AluOp::Add => emit.add_ri(size, dst, imm),
        AluOp::Or => emit.or_ri(size, dst, imm),
        AluOp::And => emit.and_ri(size, dst, imm),
        AluOp::Sub => emit.sub_ri(size, dst, imm),
        AluOp::Xor => emit.xor_ri(size, dst, imm),
        AluOp::Cmp => emit.cmp_ri(size, dst, imm),
    }
}

/// Mask RIP in RAX according to CPU mode.
fn emit_mask_rip(emit: &mut Emitter, mode: CpuMode) {
    match mode {
        CpuMode::Real16 => {
            emit.and_ri(OpSize::S64, Reg::Rax, 0xFFFF);
        }
        CpuMode::Protected32 => {
            emit.mov_rr(OpSize::S32, Reg::Rax, Reg::Rax); // zero-extend
        }
        CpuMode::Long64 => {} // no mask
    }
}

/// Mask SP in the given register according to CPU mode.
fn emit_mask_sp(emit: &mut Emitter, mode: CpuMode, reg: Reg) {
    match mode {
        CpuMode::Real16 => {
            emit.and_ri(OpSize::S64, reg, 0xFFFF);
        }
        CpuMode::Protected32 => {
            emit.mov_rr(OpSize::S32, reg, reg); // zero-extend
        }
        CpuMode::Long64 => {}
    }
}

/// Map x86 condition code to emitter Cc enum.
fn map_cc(cc: u8) -> Cc {
    match cc {
        0x0 => Cc::O,  0x1 => Cc::No,  0x2 => Cc::B,   0x3 => Cc::Ae,
        0x4 => Cc::E,  0x5 => Cc::Ne,  0x6 => Cc::Be,  0x7 => Cc::A,
        0x8 => Cc::S,  0x9 => Cc::Ns,  0xA => Cc::P,   0xB => Cc::Np,
        0xC => Cc::L,  0xD => Cc::Ge,  0xE => Cc::Le,  0xF => Cc::G,
        _ => Cc::E,
    }
}

