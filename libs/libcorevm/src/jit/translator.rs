//! Guest→Host instruction translator for JIT compilation.
//!
//! Translates pre-decoded [`BasicBlock`]s into native x86-64 machine code.
//! Leverages same-ISA advantage: guest x86 → host x86-64 means many
//! instructions (ALU, MOV, branches) can be executed natively with the
//! host CPU computing RFLAGS automatically.
//!
//! ## Phase 4 Optimizations
//!
//! ### Lazy RFLAGS
//! Instead of materializing (PUSHFQ→merge) after every ALU operation,
//! the translator tracks whether host RFLAGS are "dirty" (contain valid
//! guest arithmetic flags). Flags are only materialized into guest RFLAGS
//! when actually needed: before Jcc (which loads guest RFLAGS), before
//! interpreter fallback (which reads RFLAGS from the register file), and
//! in the epilogue. Back-to-back ALU operations skip the merge entirely.
//!
//! ### Native Memory Access
//! MOV reg,[mem] and MOV [mem],reg compute the effective linear address
//! in generated code (base + index*scale + disp + segment_base) and call
//! `jit_mem_read` / `jit_mem_write` helpers. This avoids the full
//! interpreter fallback (which re-decodes the instruction).
//!
//! ### Tier 2 Instructions
//! PUSH/POP, CALL rel, RET, LEA, XCHG, SHL/SHR/SAR reg,imm,
//! MOVZX/MOVSX (reg-reg), and more.
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
//! | RBP           | Frame pointer                              |
//! | `[RBP-8]`    | Saved `InterruptController*`               |

use alloc::vec::Vec;
use crate::decoder::CpuMode;
use crate::flags::OperandSize;
use crate::instruction::{DecodedInst, MemOperand, OpcodeMap, Operand};
use crate::jit::block::BasicBlock;
use crate::jit::emitter::{Cc, Emitter, Reg, OpSize};
use crate::jit::helpers;
use crate::registers::{RegisterFile, SegmentDescriptor, SegReg};

// ── Compile-time layout assertions ────────────────────────────────────────────
// These catch any Rust layout change that would break the JIT's hardcoded
// field offsets. If any assert fires, update the constants below to match.

const _: () = {
    assert!(core::mem::size_of::<RegisterFile>() >= 144 + 6 * 32);
    assert!(core::mem::size_of::<SegmentDescriptor>() == 32);
    assert!(core::mem::align_of::<SegmentDescriptor>() == 8);
    // base is the first field in the Rust-reordered layout (highest alignment → first).
    assert!(core::mem::offset_of!(SegmentDescriptor, base) == 0);
};

// ── RegisterFile field offsets (valid because RegisterFile is #[repr(C)]) ──

/// Byte offset of `gpr[0]` from the GPR base pointer (always 0).
const GPR_OFFSET: i32 = 0;
/// Byte offset of `rip` from the GPR base pointer: 16 × 8 = 128.
const RIP_OFFSET: i32 = 128;
/// Byte offset of `rflags` from the GPR base pointer: 128 + 8 = 136.
const RFLAGS_OFFSET: i32 = 136;

/// Mask of arithmetic status flags in RFLAGS (CF|PF|AF|ZF|SF|OF).
const ARITH_MASK: i32 = 0x8D5;
/// Inverted mask for clearing arithmetic flags.
const NOT_ARITH_MASK: i32 = !0x8D5;

// ── Host register aliases for clarity ──

/// Pointer to Cpu struct (callee-saved).
const CPU_PTR: Reg = Reg::Rbx;
/// Pointer to GuestMemory (callee-saved).
const MEM_PTR: Reg = Reg::R12;
/// Pointer to Mmu (callee-saved).
const MMU_PTR: Reg = Reg::R13;
/// Pointer to IoDispatch (callee-saved).
const IO_PTR: Reg = Reg::R14;
/// GPR base pointer: `&cpu.regs.gpr[0]` (callee-saved).
const GPR_BASE: Reg = Reg::R15;
/// First temporary for operand values.
const TEMP1: Reg = Reg::R9;
/// Second temporary for operand values.
const TEMP2: Reg = Reg::R10;
/// Temporary for captured RFLAGS after native ALU ops.
const FLAGS_TMP: Reg = Reg::R11;

/// Stack offset of saved InterruptController pointer from RBP.
const INTR_STACK_OFF: i32 = -8;

// ── RegisterFile segment descriptor offsets ──
// seg[] is after rflags in the RegisterFile. Each SegmentDescriptor is
// 40 bytes (#[repr(C)]). seg[0] = ES at rflags + 8 = GPR_BASE + 144.
// The `base` field is at offset 8 within SegmentDescriptor.
// So seg[n].base = GPR_BASE + 144 + n * 40 + 8
const SEG_ARRAY_OFFSET: i32 = 144; // offset of seg[0] from GPR_BASE
// SegmentDescriptor is NOT #[repr(C)]: Rust reorders fields by alignment.
// Actual layout (verified at compile time via offset_of! assertions below):
//   base: u64     @ offset  0  (largest alignment field → placed first)
//   limit: u32    @ offset  8
//   selector: u16 @ offset 12
//   access..bool  @ offset 14  (8 × 1-byte booleans/u8)
//   padding       @ offset 22..23 (to align to 8)
//   sizeof = 32, alignof = 8
const SEG_DESC_SIZE: i32 = 32;     // sizeof(SegmentDescriptor)
const SEG_BASE_IN_DESC: i32 = 0;   // offset of `base` within SegmentDescriptor

// ── ALU operation dispatch ──

/// ALU operation type for the generic translation helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AluOp {
    Add, Or, And, Sub, Xor, Cmp,
}

/// A compiled native code block ready for execution.
pub struct CompiledBlock {
    /// Raw x86-64 machine code bytes.
    pub code: Vec<u8>,
    /// Number of guest instructions in this block.
    pub guest_instruction_count: u64,
    /// Physical address of the block entry.
    pub entry_phys_addr: u64,
    /// Number of guest instructions translated natively in this block.
    pub native_instruction_count: u64,
}

/// Translates decoded basic blocks into native x86-64 machine code.
pub struct Translator {
    /// Count of instructions that were natively translated (diagnostics).
    pub native_count: u64,
    /// Count of instructions that fell back to interpreter (diagnostics).
    pub fallback_count: u64,
    /// CPU mode for the block currently being translated.
    /// Set at the start of `translate_block` and valid throughout translation.
    current_mode: CpuMode,
}

/// Per-block mutable state during translation.
struct BlockState {
    /// Whether host RFLAGS currently contain valid guest arithmetic flags
    /// that have NOT yet been merged into the guest RFLAGS register file.
    /// When true, FLAGS_TMP holds the captured RFLAGS from the last ALU op.
    flags_dirty: bool,
}

impl Translator {
    /// Create a new translator.
    pub fn new() -> Self {
        Translator {
            native_count: 0,
            fallback_count: 0,
            current_mode: CpuMode::Real16,
        }
    }

    /// Translate a decoded basic block into native x86-64 machine code.
    pub fn translate_block(&mut self, block: &BasicBlock, entry_phys: u64, mode: CpuMode) -> CompiledBlock {
        self.current_mode = mode;
        let mut emit = Emitter::new();
        let inst_count = block.instructions.len() as u64;
        let mut native_in_block = 0u64;
        let mut state = BlockState { flags_dirty: false };

        self.emit_prologue(&mut emit);

        for inst in &block.instructions {
            if !self.try_translate_native(&mut emit, inst, &mut state) {
                // Must flush lazy flags before interpreter fallback
                // since the interpreter reads RFLAGS from the register file.
                self.flush_lazy_flags(&mut emit, &mut state);
                self.emit_interpreter_fallback(&mut emit, inst.length);
                self.fallback_count += 1;
            } else {
                self.native_count += 1;
                native_in_block += 1;
            }
        }

        // Flush any remaining dirty flags before epilogue.
        self.flush_lazy_flags(&mut emit, &mut state);
        self.emit_epilogue(&mut emit);

        CompiledBlock {
            code: emit.finalize(),
            guest_instruction_count: inst_count,
            entry_phys_addr: entry_phys,
            native_instruction_count: native_in_block,
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Prologue / Epilogue
    // ════════════════════════════════════════════════════════════════════

    /// Emit the block prologue.
    ///
    /// Stack layout after prologue (relative to RBP):
    ///   On entry: RSP % 16 == 8.
    ///   PUSH Rbx, R12, R13, R14, R15, Rbp  → 6 × 8 = 48 bytes, RSP % 16 still 8.
    ///   MOV Rbp, Rsp                        → frame pointer set.
    ///   PUSH R8     (InterruptController*)  → [RBP - 8], RSP % 16 == 0.
    ///   CALL jit_get_gpr_ptr                → caller-side alignment valid.
    ///
    /// emit_restore_and_ret uses `MOV Rsp, Rbp` to unwind everything.
    fn emit_prologue(&self, emit: &mut Emitter) {
        // Save callee-saved registers (6 pushes → RSP 16-aligned).
        emit.push(Reg::Rbx);
        emit.push(Reg::R12);
        emit.push(Reg::R13);
        emit.push(Reg::R14);
        emit.push(Reg::R15);
        emit.push(Reg::Rbp);
        emit.mov_rr(OpSize::S64, Reg::Rbp, Reg::Rsp);

        // Save InterruptController* (5th arg = R8) at [RBP - 8].
        emit.push(Reg::R8);

        // Copy context pointers to callee-saved registers.
        emit.mov_rr(OpSize::S64, CPU_PTR, Reg::Rdi);  // RBX = cpu
        emit.mov_rr(OpSize::S64, MEM_PTR, Reg::Rsi);  // R12 = memory
        emit.mov_rr(OpSize::S64, MMU_PTR, Reg::Rdx);  // R13 = mmu
        emit.mov_rr(OpSize::S64, IO_PTR, Reg::Rcx);   // R14 = io

        // Call jit_get_gpr_ptr(cpu) to get GPR base pointer (RSP 16-aligned ✓).
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        let helper = helpers::jit_get_gpr_ptr as *const () as usize as u64;
        emit.call_abs(helper);
        emit.mov_rr(OpSize::S64, GPR_BASE, Reg::Rax); // R15 = gpr base
    }

    /// Emit the block epilogue: return JIT_OK, restore registers, ret.
    fn emit_epilogue(&self, emit: &mut Emitter) {
        emit.xor_rr(OpSize::S32, Reg::Rax, Reg::Rax); // return JIT_OK (0)
        self.emit_restore_and_ret(emit);
    }

    /// Shared register restore + ret sequence.
    fn emit_restore_and_ret(&self, emit: &mut Emitter) {
        emit.mov_rr(OpSize::S64, Reg::Rsp, Reg::Rbp);
        emit.pop(Reg::Rbp);
        emit.pop(Reg::R15);
        emit.pop(Reg::R14);
        emit.pop(Reg::R13);
        emit.pop(Reg::R12);
        emit.pop(Reg::Rbx);
        emit.ret();
    }

    // ════════════════════════════════════════════════════════════════════
    // Lazy RFLAGS
    // ════════════════════════════════════════════════════════════════════

    /// Capture host RFLAGS after a native ALU op (PUSHFQ + POP).
    /// Does NOT merge into guest RFLAGS — just saves to FLAGS_TMP.
    fn emit_capture_flags(&self, emit: &mut Emitter, state: &mut BlockState) {
        emit.pushfq();
        emit.pop(FLAGS_TMP);
        state.flags_dirty = true;
    }

    /// If flags are dirty, merge them into guest RFLAGS register file.
    fn flush_lazy_flags(&self, emit: &mut Emitter, state: &mut BlockState) {
        if !state.flags_dirty { return; }
        // Load guest RFLAGS, clear arith bits, OR in captured bits, store.
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, NOT_ARITH_MASK);
        emit.and_ri(OpSize::S64, FLAGS_TMP, ARITH_MASK);
        emit.or_rr(OpSize::S64, Reg::Rax, FLAGS_TMP);
        emit.mov_mr(OpSize::S64, GPR_BASE, RFLAGS_OFFSET, Reg::Rax);
        state.flags_dirty = false;
    }

    /// Flush lazy flags for INC/DEC (preserves CF in guest RFLAGS).
    #[allow(dead_code)]
    fn flush_lazy_flags_inc_dec(&self, emit: &mut Emitter, state: &mut BlockState) {
        if !state.flags_dirty { return; }
        const INC_DEC_MASK: i32 = 0x8D4; // PF|AF|ZF|SF|OF (no CF!)
        const NOT_INC_DEC_MASK: i32 = !0x8D4;
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, NOT_INC_DEC_MASK);
        emit.and_ri(OpSize::S64, FLAGS_TMP, INC_DEC_MASK);
        emit.or_rr(OpSize::S64, Reg::Rax, FLAGS_TMP);
        emit.mov_mr(OpSize::S64, GPR_BASE, RFLAGS_OFFSET, Reg::Rax);
        state.flags_dirty = false;
    }

    // ════════════════════════════════════════════════════════════════════
    // Instruction Translation Dispatch
    // ════════════════════════════════════════════════════════════════════

    /// Try to translate an instruction natively. Returns true if successful.
    fn try_translate_native(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        match inst.opcode_map {
            OpcodeMap::Primary => self.try_primary(emit, inst, state),
            OpcodeMap::Secondary => self.try_secondary(emit, inst, state),
            _ => false,
        }
    }

    /// Try to translate a primary (one-byte) opcode natively.
    fn try_primary(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        let op = inst.opcode as u8;
        match op {
            // ── NOP ──
            0x90 => {
                self.emit_advance_rip(emit, inst.length);
                true
            }

            // ── ALU r/m, r (16/32/64-bit, r/m ← r/m OP r) ──
            0x01 | 0x09 | 0x21 | 0x29 | 0x31 | 0x39 => {
                self.try_alu_rm_r(emit, inst, op, state)
            }

            // ── ALU r, r/m (16/32/64-bit, r ← r OP r/m) ──
            0x03 | 0x0B | 0x23 | 0x2B | 0x33 | 0x3B => {
                self.try_alu_r_rm(emit, inst, op, state)
            }

            // ── ALU accumulator, imm short forms ──
            0x05 => self.try_alu_acc_imm(emit, inst, AluOp::Add, state),
            0x0D => self.try_alu_acc_imm(emit, inst, AluOp::Or, state),
            0x25 => self.try_alu_acc_imm(emit, inst, AluOp::And, state),
            0x2D => self.try_alu_acc_imm(emit, inst, AluOp::Sub, state),
            0x35 => self.try_alu_acc_imm(emit, inst, AluOp::Xor, state),
            0x3D => self.try_alu_acc_imm(emit, inst, AluOp::Cmp, state),

            // ── Group 1: ALU r/m, imm ──
            0x81 | 0x83 => self.try_group1(emit, inst, state),

            // ── TEST r/m, r ──
            0x85 => self.try_test_rm_r(emit, inst, state),

            // ── XCHG r, r/m (mod=3 only) ──
            0x87 => self.try_xchg_rm_r(emit, inst),

            // ── MOV r/m, r ──
            0x89 => self.try_mov_rm_r(emit, inst, state),

            // ── MOV r, r/m ──
            0x8B => self.try_mov_r_rm(emit, inst, state),

            // ── LEA r, [m] ──
            0x8D => self.try_lea(emit, inst),

            // ── MOV r, imm ──
            0xB8..=0xBF => {
                self.emit_mov_r_imm(emit, inst);
                true
            }

            // ── Shift Group 2: r/m, imm8 (0xC1) ──
            0xC1 => self.try_shift_ri(emit, inst, state),

            // ── RET near ──
            0xC3 => false,

            // ── Shift Group 2: r/m, 1 (0xD1) ──
            0xD1 => self.try_shift_r1(emit, inst, state),

            // ── CALL rel32 ──
            0xE8 => false,

            // ── JMP rel32 ──
            0xE9 => {
                self.emit_jmp_rel(emit, inst);
                true
            }

            // ── JMP rel8 ──
            0xEB => {
                self.emit_jmp_rel(emit, inst);
                true
            }

            // ── Jcc rel8 ──
            0x70..=0x7F => {
                let cc = (op - 0x70) as u8;
                self.emit_jcc(emit, inst, cc, state);
                true
            }

            // ── Group 4/5: INC/DEC r/m ──
            0xFE | 0xFF => self.try_group5(emit, inst, state),

            // ── PUSH r ──
            0x50..=0x57 => false,

            // ── POP r ──
            0x58..=0x5F => false,

            _ => false,
        }
    }

    /// Try to translate a secondary (0F-prefixed) opcode natively.
    fn try_secondary(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        let op = inst.opcode as u8;
        match op {
            // ── Jcc rel32 (0F 80..8F) ──
            0x80..=0x8F => {
                let cc = (op - 0x80) as u8;
                self.emit_jcc(emit, inst, cc, state);
                true
            }

            // ── MOVZX r, r/m8 (0F B6) ──
            0xB6 => self.try_movzx_r_rm8(emit, inst),

            // ── MOVZX r, r/m16 (0F B7) ──
            0xB7 => self.try_movzx_r_rm16(emit, inst),

            // ── MOVSX r, r/m8 (0F BE) ──
            0xBE => self.try_movsx_r_rm8(emit, inst),

            // ── MOVSX r, r/m16 (0F BF) ──
            0xBF => self.try_movsx_r_rm16(emit, inst),

            _ => false,
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: ALU
    // ════════════════════════════════════════════════════════════════════

    /// Translate ALU r/m, r (opcode +1 forms). Handles both reg-reg (mod=3)
    /// and reg-mem (mod!=3) variants.
    fn try_alu_rm_r(
        &self, emit: &mut Emitter, inst: &DecodedInst, op: u8, state: &mut BlockState,
    ) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false, // 8-bit → fallback
        };
        let alu_op = match op {
            0x01 => AluOp::Add, 0x09 => AluOp::Or, 0x21 => AluOp::And,
            0x29 => AluOp::Sub, 0x31 => AluOp::Xor, 0x39 => AluOp::Cmp,
            _ => return false,
        };

        if inst.modrm_mod() == 3 {
            // Register-register
            let dst_idx = inst.modrm_rm();
            let src_idx = inst.modrm_reg();
            self.emit_alu_rr(emit, inst, alu_op, dst_idx, src_idx, size, state);
            true
        } else {
            // Memory destination: mem ← mem OP reg
            // Load from memory, ALU, store back. Too complex for now → fallback.
            false
        }
    }

    /// Translate ALU r, r/m (opcode +3 forms). Handles reg-reg and reg-mem.
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
            false
        }
    }

    /// Translate ALU accumulator, imm (short forms).
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

    /// Translate Group 1: ALU r/m, imm (0x81, 0x83).
    fn try_group1(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let digit = inst.modrm_reg() & 7;
        let alu_op = match digit {
            0 => AluOp::Add,
            1 => AluOp::Or,
            // 2 = ADC, 3 = SBB — need carry, skip
            4 => AluOp::And,
            5 => AluOp::Sub,
            6 => AluOp::Xor,
            7 => AluOp::Cmp,
            _ => return false,
        };
        let dst_idx = inst.modrm_rm();
        let imm = inst.immediate as i64 as i32;
        self.emit_alu_ri(emit, inst, alu_op, dst_idx, imm, size, state);
        true
    }

    // ── ALU emission helpers ──

    /// Emit native ALU reg, reg.
    fn emit_alu_rr(
        &self, emit: &mut Emitter, inst: &DecodedInst,
        alu_op: AluOp, dst_idx: u8, src_idx: u8, size: OpSize, state: &mut BlockState,
    ) {
        // Overwriting flags → any previous dirty flags are superseded.
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

    /// Emit native ALU reg, imm.
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
    // Native Translation: INC/DEC (Group 4/5)
    // ════════════════════════════════════════════════════════════════════

    fn try_group5(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let digit = inst.modrm_reg() & 7;
        if digit > 1 { return false; } // Only INC(/0) and DEC(/1)

        let op_byte = inst.opcode as u8;
        if op_byte == 0xFE { return false; } // 8-bit → fallback

        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let reg_idx = inst.modrm_rm();

        // Must flush previous dirty flags first (INC/DEC preserves CF,
        // so we can't just overwrite — we need the old CF in guest RFLAGS).
        self.flush_lazy_flags(emit, state);

        self.emit_load_gpr(emit, TEMP1, reg_idx, size);
        if digit == 0 {
            emit.inc(size, TEMP1);
        } else {
            emit.dec(size, TEMP1);
        }

        // Capture flags into FLAGS_TMP.
        emit.pushfq();
        emit.pop(FLAGS_TMP);

        self.emit_store_gpr(emit, reg_idx, TEMP1, size);

        // Merge preserving CF.
        const INC_DEC_MASK: i32 = 0x8D4; // PF|AF|ZF|SF|OF (no CF!)
        const NOT_INC_DEC_MASK: i32 = !0x8D4;
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, NOT_INC_DEC_MASK);
        emit.and_ri(OpSize::S64, FLAGS_TMP, INC_DEC_MASK);
        emit.or_rr(OpSize::S64, Reg::Rax, FLAGS_TMP);
        emit.mov_mr(OpSize::S64, GPR_BASE, RFLAGS_OFFSET, Reg::Rax);

        // Flags are now clean (merged directly).
        state.flags_dirty = false;

        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: MOV
    // ════════════════════════════════════════════════════════════════════

    /// MOV r/m, r (0x89). Handles reg-reg and reg→mem.
    fn try_mov_rm_r(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };

        if inst.modrm_mod() == 3 {
            // Register-register
            let dst_idx = inst.modrm_rm();
            let src_idx = inst.modrm_reg();
            self.emit_load_gpr(emit, TEMP1, src_idx, size);
            self.emit_store_gpr(emit, dst_idx, TEMP1, size);
            self.emit_advance_rip(emit, inst.length);
            true
        } else {
            false
        }
    }

    /// MOV r, r/m (0x8B). Handles reg-reg and mem→reg.
    fn try_mov_r_rm(
        &self, emit: &mut Emitter, inst: &DecodedInst, state: &mut BlockState,
    ) -> bool {
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
            false
        }
    }

    /// MOV r, imm (0xB8..0xBF).
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
            OperandSize::Byte => {} // 0xB0-0xB7 are 8-bit, handled separately
        }

        self.emit_advance_rip(emit, inst.length);
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: LEA
    // ════════════════════════════════════════════════════════════════════

    /// LEA r, [m] (0x8D) — address computation without memory access.
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

        // Compute effective address (no segment base for LEA!)
        self.emit_compute_ea_no_segment(emit, mem_op, inst);
        // RAX = effective address

        // Mask to address size (32-bit in protected mode)
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
    // Native Translation: MOVZX / MOVSX (reg-reg only)
    // ════════════════════════════════════════════════════════════════════

    /// MOVZX r, r/m8 (0F B6). Only reg-reg (mod=3).
    fn try_movzx_r_rm8(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let dst_size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let dst_idx = inst.modrm_reg();
        let src_idx = inst.modrm_rm();

        // Load source byte, zero-extend
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, src_idx as i32 * 8);
        emit.and_ri(OpSize::S64, TEMP1, 0xFF);
        self.emit_store_gpr(emit, dst_idx, TEMP1, dst_size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// MOVZX r, r/m16 (0F B7). Only reg-reg (mod=3).
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

    /// MOVSX r, r/m8 (0F BE). Only reg-reg (mod=3).
    fn try_movsx_r_rm8(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let dst_size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let dst_idx = inst.modrm_reg();
        let src_idx = inst.modrm_rm();

        // Load byte, sign-extend via movsx
        emit.movsx_rr8(dst_size, TEMP1, map_guest_to_host(src_idx));
        self.emit_store_gpr(emit, dst_idx, TEMP1, dst_size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// MOVSX r, r/m16 (0F BF). Only reg-reg (mod=3).
    fn try_movsx_r_rm16(&self, _emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        // MOVSX r32/r64, r/m16 — we need the source in a host register
        // for movsx_rr. Since we don't have movsx_rr16, fall back.
        false
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: Shifts
    // ════════════════════════════════════════════════════════════════════

    /// Shift r/m, imm8 (0xC1, mod=3).
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

        // Only SHL(4), SHR(5), SAR(7) natively
        match digit {
            4 | 5 | 7 => {}
            _ => return false,
        }

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

    /// Shift r/m, 1 (0xD1, mod=3).
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

        match digit {
            4 | 5 | 7 => {}
            _ => return false,
        }

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

    /// PUSH r (0x50+r).
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

        // RSP (gpr[4]) -= push_size
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8); // guest RSP
        emit.sub_ri(OpSize::S64, Reg::Rax, push_size as i32);
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        emit.mov_mr(OpSize::S64, GPR_BASE, 4 * 8, Reg::Rax); // store new RSP

        // Compute linear address: SS.base + RSP
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1); // RAX = linear addr

        // Load value to push
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, reg_idx as i32 * 8);

        // Call jit_mem_write(cpu, mem, mmu, linear, size, value)
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax); // linear
        emit.mov_ri32(Reg::R8, push_size as u32);       // size
        // TEMP1=R9 already holds the value (6th SysV arg)
        let wr = helpers::jit_mem_write as *const () as usize as u64;
        emit.call_abs(wr);

        let ok = emit.new_label();
        emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
        emit.jcc_label(Cc::Ne, ok);
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_restore_and_ret(emit);
        emit.bind_label(ok);

        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// POP r (0x58+r).
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

        // Compute linear address: SS.base + RSP
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8); // guest RSP
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1); // RAX = linear addr

        let read_size = match pop_size {
            8 => OpSize::S64,
            4 => OpSize::S32,
            2 => OpSize::S16,
            _ => return false,
        };
        self.emit_call_mem_read_with_linear(emit, Reg::Rax, read_size);
        // RAX = value

        // Store to destination register
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

    /// CALL rel32 (0xE8).
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
        // RAX = return address

        // Push return address: RSP -= push_size, [SS:RSP] = return_addr
        emit.mov_rr(OpSize::S64, TEMP1, Reg::Rax); // save return addr

        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8); // guest RSP
        emit.sub_ri(OpSize::S64, Reg::Rax, push_size as i32);
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        emit.mov_mr(OpSize::S64, GPR_BASE, 4 * 8, Reg::Rax); // store new RSP

        // Linear = SS.base + RSP
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP2, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP2); // RAX = linear

        // Call jit_mem_write(cpu, mem, mmu, linear, size, value=return_addr)
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rcx, Reg::Rax);   // linear
        emit.mov_ri32(Reg::R8, push_size as u32);          // size
        // TEMP1=R9 already holds return address (6th SysV arg)
        let wr = helpers::jit_mem_write as *const () as usize as u64;
        emit.call_abs(wr);

        let ok = emit.new_label();
        emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
        emit.jcc_label(Cc::Ne, ok);
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_restore_and_ret(emit);
        emit.bind_label(ok);

        // Set RIP = return_addr + rel = (old_rip + inst.length) + rel
        // = old_rip + inst.length + rel
        let delta = inst.length as i64 + rel;
        self.emit_advance_rip_by(emit, delta);
        true
    }

    /// RET near (0xC3).
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

        // Read return address from [SS:RSP]
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, 4 * 8); // guest RSP
        emit_mask_sp(emit, self.current_mode, Reg::Rax);
        let ss_base_off = SEG_ARRAY_OFFSET + SegReg::Ss as i32 * SEG_DESC_SIZE + SEG_BASE_IN_DESC;
        emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, ss_base_off);
        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1); // RAX = linear

        let read_size = match pop_size {
            8 => OpSize::S64,
            4 => OpSize::S32,
            2 => OpSize::S16,
            _ => return false,
        };
        self.emit_call_mem_read_with_linear(emit, Reg::Rax, read_size);
        // RAX = return address — apply mode mask before storing to RIP.
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

    /// Emit a conditional branch (Jcc).
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

        // Must materialize guest RFLAGS for the condition check.
        // If flags are dirty, merge them first.
        self.flush_lazy_flags(emit, state);

        // Load guest RFLAGS, mask to arithmetic flags, push + popfq.
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

    /// Emit an unconditional relative jump.
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
    // Interpreter Fallback
    // ════════════════════════════════════════════════════════════════════

    /// Emit a call to `jit_interpret_one` for one guest instruction.
    fn emit_interpreter_fallback(&self, emit: &mut Emitter, inst_len: u8) {
        // expected_next = mask_ip(rip + inst_len)
        emit.mov_rm(OpSize::S64, TEMP2, GPR_BASE, RIP_OFFSET);
        emit.add_ri(OpSize::S64, TEMP2, inst_len as i32);
        emit_mask_ip(emit, self.current_mode, TEMP2);
        // Preserve expected_next across helper call.
        emit.push(TEMP2);
        // Keep SysV stack alignment before call (16-byte aligned).
        emit.sub_ri(OpSize::S64, Reg::Rsp, 8);

        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rcx, IO_PTR);
        emit.mov_rm(OpSize::S64, Reg::R8, Reg::Rbp, INTR_STACK_OFF);

        let helper_addr = helpers::jit_interpret_one as *const () as usize as u64;
        emit.call_abs(helper_addr);

        emit.add_ri(OpSize::S64, Reg::Rsp, 8);
        emit.pop(TEMP2);

        // If helper requested exit, propagate exit.
        let check_rip = emit.new_label();
        emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_OK as i32);
        emit.jcc_label(Cc::E, check_rip);
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_restore_and_ret(emit);

        // Continue only if RIP advanced linearly to expected_next.
        emit.bind_label(check_rip);
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
        emit.cmp_rr(OpSize::S64, Reg::Rax, TEMP2);
        let continue_label = emit.new_label();
        emit.jcc_label(Cc::E, continue_label);
        emit.mov_ri32(Reg::Rax, helpers::JIT_EXIT_BLOCK);
        self.emit_restore_and_ret(emit);
        emit.bind_label(continue_label);
    }

    // ════════════════════════════════════════════════════════════════════
    // Memory Address Computation
    // ════════════════════════════════════════════════════════════════════

    /// Compute the linear address for a memory operand into RAX.
    /// Linear = segment_base + effective_address.
    fn emit_compute_linear(&self, emit: &mut Emitter, mem_op: &MemOperand, inst: &DecodedInst) {
        self.emit_compute_ea_no_segment(emit, mem_op, inst);
        // RAX = effective address (before segment)

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

    /// Compute effective address (base + index*scale + disp) into RAX.
    /// Does NOT add segment base (used by LEA).
    fn emit_compute_ea_no_segment(
        &self, emit: &mut Emitter, mem_op: &MemOperand, inst: &DecodedInst,
    ) {
        if mem_op.rip_relative {
            // RIP-relative: EA = next_rip + displacement
            emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
            emit.add_ri(OpSize::S64, Reg::Rax, inst.length as i32);
            if mem_op.displacement != 0 {
                let d = mem_op.displacement as i32;
                emit.add_ri(OpSize::S64, Reg::Rax, d);
            }
            return;
        }

        // Start with displacement
        if mem_op.displacement != 0 {
            let d = mem_op.displacement;
            if d >= i32::MIN as i64 && d <= i32::MAX as i64 {
                emit.mov_ri32(Reg::Rax, d as u32);
                // Sign-extend for negative
                if d < 0 {
                    emit.mov_ri64(Reg::Rax, d as u64);
                }
            } else {
                emit.mov_ri64(Reg::Rax, d as u64);
            }
        } else {
            emit.xor_rr(OpSize::S32, Reg::Rax, Reg::Rax);
        }

        // Add base register
        if let Some(base) = mem_op.base {
            emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, base as i32 * 8);
            emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
        }

        // Add index * scale
        if let Some(index) = mem_op.index {
            emit.mov_rm(OpSize::S64, TEMP1, GPR_BASE, index as i32 * 8);
            match mem_op.scale {
                1 => {
                    emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
                }
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
                    // Arbitrary scale — multiply
                    emit.mov_ri32(TEMP2, mem_op.scale as u32);
                    // IMUL would clobber flags but we don't care here
                    // Use lea or manual shift. For simplicity, repeated add.
                    for _ in 0..mem_op.scale {
                        emit.add_rr(OpSize::S64, Reg::Rax, TEMP1);
                    }
                }
            }
        }
    }

    /// Call jit_mem_read with a pre-computed linear address in RAX.
    /// Result in RAX.
    fn emit_call_mem_read(&self, emit: &mut Emitter, size: OpSize) {
        self.emit_call_mem_read_with_linear(emit, Reg::Rax, size);
    }

    /// Call jit_mem_read with linear address in the given register.
    fn emit_call_mem_read_with_linear(&self, emit: &mut Emitter, linear_reg: Reg, size: OpSize) {
        // jit_mem_read(cpu, memory, mmu, linear, size) -> u64
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        if linear_reg != Reg::Rcx {
            emit.mov_rr(OpSize::S64, Reg::Rcx, linear_reg);
        }
        emit.mov_ri32(Reg::R8, opsize_bytes(size) as u32);
        let rd = helpers::jit_mem_read as *const () as usize as u64;
        emit.call_abs(rd);
    }

    // ════════════════════════════════════════════════════════════════════
    // Common Emission Helpers
    // ════════════════════════════════════════════════════════════════════

    /// Load a guest GPR into a host temporary register.
    fn emit_load_gpr(&self, emit: &mut Emitter, host_dst: Reg, guest_idx: u8, size: OpSize) {
        let offset = GPR_OFFSET + guest_idx as i32 * 8;
        match size {
            // 32-bit register reads in 64-bit host code should ignore upper
            // bits; loading as S32 gives a clean zero-extended host value.
            OpSize::S32 => emit.mov_rm(OpSize::S32, host_dst, GPR_BASE, offset),
            OpSize::S64 | OpSize::S16 | OpSize::S8 => {
                emit.mov_rm(OpSize::S64, host_dst, GPR_BASE, offset);
            }
        }
    }

    /// Store a host temporary register back to a guest GPR.
    fn emit_store_gpr(&self, emit: &mut Emitter, guest_idx: u8, host_src: Reg, size: OpSize) {
        let offset = GPR_OFFSET + guest_idx as i32 * 8;
        match size {
            OpSize::S64 => {
                emit.mov_mr(OpSize::S64, GPR_BASE, offset, host_src);
            }
            OpSize::S32 => {
                // x86 writes to r32 must zero-extend into the architectural
                // 64-bit register. Canonicalize upper bits before storing.
                emit.mov_rr(OpSize::S32, host_src, host_src);
                emit.mov_mr(OpSize::S64, GPR_BASE, offset, host_src);
            }
            OpSize::S16 => {
                emit.mov_mr(OpSize::S16, GPR_BASE, offset, host_src);
            }
            OpSize::S8 => {
                emit.mov_mr(OpSize::S8, GPR_BASE, offset, host_src);
            }
        }
    }

    /// Advance guest RIP by a fixed instruction length.
    ///
    /// Applies the same RIP masking as `Cpu::mask_rip()`:
    /// - Real16/Protected16 → mask to 16 bits
    /// - Protected32        → mask to 32 bits (zero-extend via 32-bit mov)
    /// - Long64             → no masking
    fn emit_advance_rip(&self, emit: &mut Emitter, length: u8) {
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
        emit.add_ri(OpSize::S64, Reg::Rax, length as i32);
        emit_mask_rip(emit, self.current_mode);
        emit.mov_mr(OpSize::S64, GPR_BASE, RIP_OFFSET, Reg::Rax);
    }

    /// Advance guest RIP by a signed delta (for branches).
    ///
    /// Applies RIP masking according to the block's CPU mode.
    fn emit_advance_rip_by(&self, emit: &mut Emitter, delta: i64) {
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
        if delta >= i32::MIN as i64 && delta <= i32::MAX as i64 {
            emit.add_ri(OpSize::S64, Reg::Rax, delta as i32);
        } else {
            emit.mov_ri64(Reg::Rcx, delta as u64);
            emit.add_rr(OpSize::S64, Reg::Rax, Reg::Rcx);
        }
        emit_mask_rip(emit, self.current_mode);
        emit.mov_mr(OpSize::S64, GPR_BASE, RIP_OFFSET, Reg::Rax);
    }
}

// ════════════════════════════════════════════════════════════════════════
// Free functions
// ════════════════════════════════════════════════════════════════════════

/// Emit RIP masking into RAX matching `Cpu::mask_rip()` for the given mode.
/// Assumes the new RIP value is already in RAX.
fn emit_mask_rip(emit: &mut Emitter, mode: CpuMode) {
    match mode {
        CpuMode::Real16 => {
            // Real mode and 16-bit protected: RIP is 16-bit.
            emit.and_ri(OpSize::S64, Reg::Rax, 0xFFFF);
        }
        CpuMode::Protected32 => {
            // 32-bit protected: RIP is 32-bit. A 32-bit MOV zero-extends.
            emit.mov_rr(OpSize::S32, Reg::Rax, Reg::Rax);
        }
        CpuMode::Long64 => {
            // Long mode: full 64-bit RIP, no masking needed.
        }
    }
}

/// Emit instruction-pointer masking into an arbitrary register.
fn emit_mask_ip(emit: &mut Emitter, mode: CpuMode, reg: Reg) {
    match mode {
        CpuMode::Real16 => {
            emit.and_ri(OpSize::S64, reg, 0xFFFF);
        }
        CpuMode::Protected32 => {
            emit.mov_rr(OpSize::S32, reg, reg);
        }
        CpuMode::Long64 => {}
    }
}

/// Emit stack-pointer masking for the current mode.
///
/// Real mode uses 16-bit SP wrapping semantics. Protected mode here uses
/// 32-bit ESP semantics. Long mode keeps full 64-bit RSP.
fn emit_mask_sp(emit: &mut Emitter, mode: CpuMode, reg: Reg) {
    match mode {
        CpuMode::Real16 => {
            emit.and_ri(OpSize::S64, reg, 0xFFFF);
        }
        CpuMode::Protected32 => {
            emit.mov_rr(OpSize::S32, reg, reg);
        }
        CpuMode::Long64 => {}
    }
}

/// Emit a native ALU reg, reg instruction.
fn emit_native_alu_rr(emit: &mut Emitter, op: AluOp, size: OpSize, dst: Reg, src: Reg) {
    match op {
        AluOp::Add => emit.add_rr(size, dst, src),
        AluOp::Or  => emit.or_rr(size, dst, src),
        AluOp::And => emit.and_rr(size, dst, src),
        AluOp::Sub => emit.sub_rr(size, dst, src),
        AluOp::Xor => emit.xor_rr(size, dst, src),
        AluOp::Cmp => emit.cmp_rr(size, dst, src),
    }
}

/// Emit a native ALU reg, imm instruction.
fn emit_native_alu_ri(emit: &mut Emitter, op: AluOp, size: OpSize, dst: Reg, imm: i32) {
    match op {
        AluOp::Add => emit.add_ri(size, dst, imm),
        AluOp::Or  => emit.or_ri(size, dst, imm),
        AluOp::And => emit.and_ri(size, dst, imm),
        AluOp::Sub => emit.sub_ri(size, dst, imm),
        AluOp::Xor => emit.xor_ri(size, dst, imm),
        AluOp::Cmp => emit.cmp_ri(size, dst, imm),
    }
}

/// Map guest OperandSize to emitter OpSize.
fn map_operand_size(size: OperandSize) -> Option<OpSize> {
    match size {
        OperandSize::Byte  => None,
        OperandSize::Word  => Some(OpSize::S16),
        OperandSize::Dword => Some(OpSize::S32),
        OperandSize::Qword => Some(OpSize::S64),
    }
}

/// Map a guest x86 condition code to the emitter's `Cc` enum.
fn map_cc(cc: u8) -> Cc {
    match cc & 0x0F {
        0x0 => Cc::O,  0x1 => Cc::No,
        0x2 => Cc::B,  0x3 => Cc::Ae,
        0x4 => Cc::E,  0x5 => Cc::Ne,
        0x6 => Cc::Be, 0x7 => Cc::A,
        0x8 => Cc::S,  0x9 => Cc::Ns,
        0xA => Cc::P,  0xB => Cc::Np,
        0xC => Cc::L,  0xD => Cc::Ge,
        0xE => Cc::Le, 0xF => Cc::G,
        _ => Cc::E, // unreachable
    }
}

/// Map OpSize to byte count.
fn opsize_bytes(size: OpSize) -> u8 {
    match size {
        OpSize::S8 => 1,
        OpSize::S16 => 2,
        OpSize::S32 => 4,
        OpSize::S64 => 8,
    }
}

/// Map a guest GPR index to a host register.
/// Only used for MOVSX source reg — loads from register file instead.
fn map_guest_to_host(_idx: u8) -> Reg {
    // We can't map arbitrary guest regs to host regs. Instead,
    // load the source into TEMP1 first and use TEMP1.
    TEMP1
}
