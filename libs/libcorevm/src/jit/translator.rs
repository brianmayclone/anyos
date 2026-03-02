//! Guest→Host instruction translator for JIT compilation.
//!
//! Translates pre-decoded [`BasicBlock`]s into native x86-64 machine code.
//! Leverages same-ISA advantage: guest x86 → host x86-64 means many
//! instructions (ALU, MOV, branches) can be executed natively with the
//! host CPU computing RFLAGS automatically.
//!
//! ## Natively Translated (Phase 3)
//!
//! - **ALU reg,reg**: ADD, OR, AND, SUB, XOR, CMP (16/32/64-bit)
//! - **ALU reg,imm**: ADD, OR, AND, SUB, XOR, CMP (Group 1)
//! - **ALU acc,imm**: ADD, OR, AND, SUB, XOR, CMP (short forms)
//! - **TEST reg,reg**: AND without storing result
//! - **INC/DEC reg**: Preserves CF
//! - **MOV reg,reg**: Register-to-register moves
//! - **MOV reg,imm**: Immediate loads
//! - **NOP**: No operation
//! - **JMP rel**: Unconditional relative jumps
//! - **Jcc rel**: Conditional branches (evaluates guest RFLAGS natively)
//!
//! ## Interpreter Fallback
//!
//! Everything else (memory access, PUSH/POP, CALL/RET, FPU, SSE, system
//! instructions) is delegated to [`jit_interpret_one`] which re-decodes
//! and executes via the existing interpreter.
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
//! | R11           | Temporary (captured RFLAGS)                |
//! | RAX           | Temporary / helper return value            |
//! | RBP           | Frame pointer                              |
//! | `[RBP-8]`    | Saved `InterruptController*`               |

use alloc::vec::Vec;
use crate::flags::OperandSize;
use crate::instruction::{DecodedInst, OpcodeMap, Operand};
use crate::jit::block::BasicBlock;
use crate::jit::emitter::{Cc, Emitter, Reg, OpSize};
use crate::jit::helpers;

// ── RegisterFile field offsets (valid because RegisterFile is #[repr(C)]) ──

/// Byte offset of `gpr[0]` from the GPR base pointer (always 0).
const GPR_OFFSET: i32 = 0;
/// Byte offset of `rip` from the GPR base pointer: 16 × 8 = 128.
const RIP_OFFSET: i32 = 128;
/// Byte offset of `rflags` from the GPR base pointer: 128 + 8 = 136.
const RFLAGS_OFFSET: i32 = 136;

/// Mask of arithmetic status flags in RFLAGS (CF|PF|AF|ZF|SF|OF).
const ARITH_MASK: i32 = 0x8D5;
/// Inverted mask for clearing arithmetic flags: sign-extends correctly.
const NOT_ARITH_MASK: i32 = !0x8D5; // = -2262 → sign-extends to 0xFFFFFFFFFFFFF72A

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
}

/// Translates decoded basic blocks into native x86-64 machine code.
pub struct Translator {
    /// Count of instructions that were natively translated (diagnostics).
    pub native_count: u64,
    /// Count of instructions that fell back to interpreter (diagnostics).
    pub fallback_count: u64,
}

impl Translator {
    /// Create a new translator.
    pub fn new() -> Self {
        Translator {
            native_count: 0,
            fallback_count: 0,
        }
    }

    /// Translate a decoded basic block into native x86-64 machine code.
    pub fn translate_block(&mut self, block: &BasicBlock, entry_phys: u64) -> CompiledBlock {
        let mut emit = Emitter::new();
        let inst_count = block.instructions.len() as u64;

        self.emit_prologue(&mut emit);

        for inst in &block.instructions {
            if !self.try_translate_native(&mut emit, inst) {
                self.emit_interpreter_fallback(&mut emit);
                self.fallback_count += 1;
            } else {
                self.native_count += 1;
            }
        }

        self.emit_epilogue(&mut emit);

        CompiledBlock {
            code: emit.finalize(),
            guest_instruction_count: inst_count,
            entry_phys_addr: entry_phys,
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Prologue / Epilogue
    // ════════════════════════════════════════════════════════════════════

    /// Emit the block prologue.
    ///
    /// Saves callee-saved registers, copies the five context pointers into
    /// callee-saved slots, then calls `jit_get_gpr_ptr` to obtain the GPR
    /// base pointer for fast register access.
    fn emit_prologue(&self, emit: &mut Emitter) {
        // Save callee-saved registers (7 pushes → RSP 16-aligned after CALL).
        emit.push(Reg::Rbx);
        emit.push(Reg::R12);
        emit.push(Reg::R13);
        emit.push(Reg::R14);
        emit.push(Reg::R15);
        emit.push(Reg::Rbp);
        emit.mov_rr(OpSize::S64, Reg::Rbp, Reg::Rsp);

        // Save InterruptController* (R8) on the stack.
        emit.push(Reg::R8); // [RBP - 8]

        // Copy context pointers to callee-saved registers.
        emit.mov_rr(OpSize::S64, CPU_PTR, Reg::Rdi);  // RBX = cpu
        emit.mov_rr(OpSize::S64, MEM_PTR, Reg::Rsi);  // R12 = memory
        emit.mov_rr(OpSize::S64, MMU_PTR, Reg::Rdx);  // R13 = mmu
        emit.mov_rr(OpSize::S64, IO_PTR, Reg::Rcx);   // R14 = io

        // Call jit_get_gpr_ptr(cpu) to get GPR base pointer.
        // RDI already = cpu (we just moved it, but let's reload since
        // the calling convention says RDI = first arg).
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

    /// Shared register restore + ret sequence (used by epilogue and early exits).
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
    // Instruction Translation Dispatch
    // ════════════════════════════════════════════════════════════════════

    /// Try to translate an instruction natively. Returns true if successful.
    fn try_translate_native(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        match inst.opcode_map {
            OpcodeMap::Primary => self.try_primary(emit, inst),
            OpcodeMap::Secondary => self.try_secondary(emit, inst),
            _ => false,
        }
    }

    /// Try to translate a primary (one-byte) opcode natively.
    fn try_primary(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        let op = inst.opcode as u8;
        match op {
            // ── NOP ──
            0x90 => {
                self.emit_advance_rip(emit, inst.length);
                true
            }

            // ── ALU r/m, r (16/32/64-bit, direction: r/m ← r/m OP r) ──
            0x01 | 0x09 | 0x21 | 0x29 | 0x31 | 0x39 => {
                self.try_alu_rm_r(emit, inst, op)
            }

            // ── ALU r, r/m (16/32/64-bit, direction: r ← r OP r/m) ──
            0x03 | 0x0B | 0x23 | 0x2B | 0x33 | 0x3B => {
                self.try_alu_r_rm(emit, inst, op)
            }

            // ── ALU accumulator, imm (16/32/64-bit short forms) ──
            0x05 => self.try_alu_acc_imm(emit, inst, AluOp::Add),
            0x0D => self.try_alu_acc_imm(emit, inst, AluOp::Or),
            0x25 => self.try_alu_acc_imm(emit, inst, AluOp::And),
            0x2D => self.try_alu_acc_imm(emit, inst, AluOp::Sub),
            0x35 => self.try_alu_acc_imm(emit, inst, AluOp::Xor),
            0x3D => self.try_alu_acc_imm(emit, inst, AluOp::Cmp),

            // ── Group 1: ALU r/m, imm (0x81 = imm32, 0x83 = imm8 sign-ext) ──
            0x81 | 0x83 => self.try_group1(emit, inst),

            // ── TEST r/m, r ──
            0x85 => self.try_test_rm_r(emit, inst),

            // ── MOV r/m, r (16/32/64-bit) ──
            0x89 => self.try_mov_rm_r(emit, inst),

            // ── MOV r, r/m (16/32/64-bit) ──
            0x8B => self.try_mov_r_rm(emit, inst),

            // ── MOV r32/64, imm32/64 ──
            0xB8..=0xBF => {
                self.emit_mov_r_imm(emit, inst);
                true
            }

            // ── Jcc rel8 ──
            0x70..=0x7F => {
                let cc = (op - 0x70) as u8;
                self.emit_jcc(emit, inst, cc);
                true
            }

            // ── JMP rel8 ──
            0xEB => {
                self.emit_jmp_rel(emit, inst);
                true
            }

            // ── JMP rel32 ──
            0xE9 => {
                self.emit_jmp_rel(emit, inst);
                true
            }

            // ── Group 4/5: INC/DEC r/m ──
            0xFE | 0xFF => self.try_group5_inc_dec(emit, inst),

            _ => false,
        }
    }

    /// Try to translate a secondary (0F-prefixed) opcode natively.
    fn try_secondary(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        let op = inst.opcode as u8;
        match op {
            // ── Jcc rel32 (0F 80..8F) ──
            0x80..=0x8F => {
                let cc = (op - 0x80) as u8;
                self.emit_jcc(emit, inst, cc);
                true
            }

            _ => false,
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: ALU register-register
    // ════════════════════════════════════════════════════════════════════

    /// Translate ALU r/m, r (opcode +1 forms: ADD=01, OR=09, etc.).
    /// Only handles mod=3 (register-register). Memory operands fall back.
    fn try_alu_rm_r(&self, emit: &mut Emitter, inst: &DecodedInst, op: u8) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false, // 8-bit → fallback
        };
        let alu_op = match op {
            0x01 => AluOp::Add, 0x09 => AluOp::Or, 0x21 => AluOp::And,
            0x29 => AluOp::Sub, 0x31 => AluOp::Xor, 0x39 => AluOp::Cmp,
            _ => return false,
        };
        // r/m = destination, reg = source
        let dst_idx = inst.modrm_rm();
        let src_idx = inst.modrm_reg();
        self.emit_alu_rr(emit, inst, alu_op, dst_idx, src_idx, size);
        true
    }

    /// Translate ALU r, r/m (opcode +3 forms: ADD=03, OR=0B, etc.).
    fn try_alu_r_rm(&self, emit: &mut Emitter, inst: &DecodedInst, op: u8) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let alu_op = match op {
            0x03 => AluOp::Add, 0x0B => AluOp::Or, 0x23 => AluOp::And,
            0x2B => AluOp::Sub, 0x33 => AluOp::Xor, 0x3B => AluOp::Cmp,
            _ => return false,
        };
        // reg = destination, r/m = source
        let dst_idx = inst.modrm_reg();
        let src_idx = inst.modrm_rm();
        self.emit_alu_rr(emit, inst, alu_op, dst_idx, src_idx, size);
        true
    }

    /// Translate ALU accumulator, imm (short forms: 0x05=ADD, 0x0D=OR, etc.).
    fn try_alu_acc_imm(&self, emit: &mut Emitter, inst: &DecodedInst, alu_op: AluOp) -> bool {
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        // Destination is always register 0 (RAX/EAX/AX).
        let imm = inst.immediate as i64 as i32;
        self.emit_alu_ri(emit, inst, alu_op, 0, imm, size);
        true
    }

    /// Translate Group 1: ALU r/m, imm (0x81, 0x83).
    fn try_group1(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let digit = inst.modrm_reg() & 7;
        let alu_op = match digit {
            0 => AluOp::Add,
            1 => AluOp::Or,
            // 2 = ADC, 3 = SBB — need carry, skip for Phase 3
            4 => AluOp::And,
            5 => AluOp::Sub,
            6 => AluOp::Xor,
            7 => AluOp::Cmp,
            _ => return false,
        };
        let dst_idx = inst.modrm_rm();
        let imm = inst.immediate as i64 as i32;
        self.emit_alu_ri(emit, inst, alu_op, dst_idx, imm, size);
        true
    }

    // ── ALU code emission helpers ──

    /// Emit native code for an ALU reg, reg operation.
    fn emit_alu_rr(
        &self, emit: &mut Emitter, inst: &DecodedInst,
        alu_op: AluOp, dst_idx: u8, src_idx: u8, size: OpSize,
    ) {
        // 1. Load guest registers into temporaries.
        self.emit_load_gpr(emit, TEMP1, dst_idx, size);
        self.emit_load_gpr(emit, TEMP2, src_idx, size);

        // 2. Execute native ALU operation (host CPU sets RFLAGS).
        emit_native_alu_rr(emit, alu_op, size, TEMP1, TEMP2);

        // 3. Capture host RFLAGS immediately.
        self.emit_capture_flags(emit);

        // 4. Store result (CMP doesn't write back).
        if alu_op != AluOp::Cmp {
            self.emit_store_gpr(emit, dst_idx, TEMP1, size);
        }

        // 5. Merge captured arithmetic flags into guest RFLAGS.
        self.emit_merge_arith_flags(emit);

        // 6. Advance guest RIP.
        self.emit_advance_rip(emit, inst.length);
    }

    /// Emit native code for an ALU reg, imm operation.
    fn emit_alu_ri(
        &self, emit: &mut Emitter, inst: &DecodedInst,
        alu_op: AluOp, dst_idx: u8, imm: i32, size: OpSize,
    ) {
        // 1. Load guest register.
        self.emit_load_gpr(emit, TEMP1, dst_idx, size);

        // 2. Execute native ALU with immediate.
        emit_native_alu_ri(emit, alu_op, size, TEMP1, imm);

        // 3. Capture host RFLAGS.
        self.emit_capture_flags(emit);

        // 4. Store result (CMP doesn't write back).
        if alu_op != AluOp::Cmp {
            self.emit_store_gpr(emit, dst_idx, TEMP1, size);
        }

        // 5. Merge flags.
        self.emit_merge_arith_flags(emit);

        // 6. Advance RIP.
        self.emit_advance_rip(emit, inst.length);
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: TEST
    // ════════════════════════════════════════════════════════════════════

    /// Translate TEST r/m, r (0x85 with mod=3).
    fn try_test_rm_r(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let a_idx = inst.modrm_rm();
        let b_idx = inst.modrm_reg();

        self.emit_load_gpr(emit, TEMP1, a_idx, size);
        self.emit_load_gpr(emit, TEMP2, b_idx, size);
        emit.test_rr(size, TEMP1, TEMP2);

        self.emit_capture_flags(emit);
        self.emit_merge_arith_flags(emit);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: INC/DEC
    // ════════════════════════════════════════════════════════════════════

    /// Translate Group 4/5 INC/DEC r/m (0xFE/0xFF with /0 or /1, mod=3).
    fn try_group5_inc_dec(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let digit = inst.modrm_reg() & 7;
        if digit > 1 { return false; } // Only INC(/0) and DEC(/1)

        let op_byte = inst.opcode as u8;
        // 0xFE = 8-bit, 0xFF = 16/32/64-bit
        if op_byte == 0xFE { return false; } // 8-bit → fallback

        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let reg_idx = inst.modrm_rm();

        // Load guest register.
        self.emit_load_gpr(emit, TEMP1, reg_idx, size);

        // Execute native INC or DEC.
        if digit == 0 {
            emit.inc(size, TEMP1);
        } else {
            emit.dec(size, TEMP1);
        }

        // Capture flags. INC/DEC preserves CF — we need special handling.
        emit.pushfq();
        emit.pop(FLAGS_TMP); // FLAGS_TMP = host flags after INC/DEC

        // Store result.
        self.emit_store_gpr(emit, reg_idx, TEMP1, size);

        // Merge flags preserving guest CF (INC/DEC don't modify CF).
        // Guest RFLAGS bits to keep: CF + non-arithmetic flags.
        // New from host: PF, AF, ZF, SF, OF.
        const INC_DEC_MASK: i32 = 0x8D4; // PF|AF|ZF|SF|OF (no CF!)
        const NOT_INC_DEC_MASK: i32 = !0x8D4; // keep CF and system flags

        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, NOT_INC_DEC_MASK);
        emit.and_ri(OpSize::S64, FLAGS_TMP, INC_DEC_MASK);
        emit.or_rr(OpSize::S64, Reg::Rax, FLAGS_TMP);
        emit.mov_mr(OpSize::S64, GPR_BASE, RFLAGS_OFFSET, Reg::Rax);

        self.emit_advance_rip(emit, inst.length);
        true
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: MOV
    // ════════════════════════════════════════════════════════════════════

    /// Translate MOV r/m, r (0x89 with mod=3).
    fn try_mov_rm_r(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let dst_idx = inst.modrm_rm();
        let src_idx = inst.modrm_reg();

        self.emit_load_gpr(emit, TEMP1, src_idx, size);
        self.emit_store_gpr(emit, dst_idx, TEMP1, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// Translate MOV r, r/m (0x8B with mod=3).
    fn try_mov_r_rm(&self, emit: &mut Emitter, inst: &DecodedInst) -> bool {
        if inst.modrm_mod() != 3 { return false; }
        let size = match map_operand_size(inst.operand_size) {
            Some(s) => s,
            None => return false,
        };
        let dst_idx = inst.modrm_reg();
        let src_idx = inst.modrm_rm();

        self.emit_load_gpr(emit, TEMP1, src_idx, size);
        self.emit_store_gpr(emit, dst_idx, TEMP1, size);
        self.emit_advance_rip(emit, inst.length);
        true
    }

    /// Translate MOV r, imm (0xB8..0xBF for 32/64-bit).
    fn emit_mov_r_imm(&self, emit: &mut Emitter, inst: &DecodedInst) {
        // Register index: low 3 bits of opcode + REX.B extension.
        let reg_idx = (inst.opcode as u8 & 7) | if inst.prefix.rex_b() { 8 } else { 0 };
        let imm = inst.immediate;

        match inst.operand_size {
            OperandSize::Qword => {
                // 64-bit immediate.
                emit.mov_ri64(TEMP1, imm);
                emit.mov_mr(OpSize::S64, GPR_BASE, reg_idx as i32 * 8, TEMP1);
            }
            OperandSize::Dword => {
                // 32-bit immediate, zero-extends to 64-bit.
                emit.mov_ri32(TEMP1, imm as u32);
                emit.mov_mr(OpSize::S64, GPR_BASE, reg_idx as i32 * 8, TEMP1);
            }
            OperandSize::Word => {
                // 16-bit immediate, preserves upper bits.
                emit.mov_ri32(TEMP1, imm as u32); // load 32-bit (upper zeroed)
                emit.mov_mr(OpSize::S16, GPR_BASE, reg_idx as i32 * 8, TEMP1);
            }
            OperandSize::Byte => {
                // 8-bit MOV imm to register — fall back handled at call site.
                // Shouldn't reach here since 0xB0-0xB7 are 8-bit.
            }
        }

        self.emit_advance_rip(emit, inst.length);
    }

    // ════════════════════════════════════════════════════════════════════
    // Native Translation: Branches
    // ════════════════════════════════════════════════════════════════════

    /// Emit a conditional branch (Jcc).
    ///
    /// Loads guest RFLAGS into host RFLAGS via PUSH/POPFQ (masked to
    /// arithmetic flags only), then uses the native Jcc instruction.
    fn emit_jcc(&self, emit: &mut Emitter, inst: &DecodedInst, cc: u8) {
        // Extract the relative offset from the instruction.
        let rel = match inst.operands[0] {
            Operand::RelativeOffset(off) => off,
            _ => {
                // Unexpected operand — shouldn't happen for Jcc.
                self.emit_advance_rip(emit, inst.length);
                return;
            }
        };

        // Load guest RFLAGS, mask to arithmetic flags only, push + popfq.
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        emit.and_ri(OpSize::S64, Reg::Rax, ARITH_MASK | 0x02); // keep arith + bit1 fixed
        emit.push(Reg::Rax);
        emit.popfq();

        // Native conditional jump.
        let taken_label = emit.new_label();
        emit.jcc_label(map_cc(cc), taken_label);

        // ── Not-taken path: RIP += inst.length ──
        self.emit_advance_rip(emit, inst.length);
        let done_label = emit.new_label();
        emit.jmp_label(done_label);

        // ── Taken path: RIP += inst.length + rel ──
        emit.bind_label(taken_label);
        let delta = inst.length as i64 + rel;
        self.emit_advance_rip_by(emit, delta);

        emit.bind_label(done_label);
    }

    /// Emit an unconditional relative jump (JMP rel8 / JMP rel32).
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
    fn emit_interpreter_fallback(&self, emit: &mut Emitter) {
        // Load arguments for jit_interpret_one(cpu, memory, mmu, io, interrupts).
        emit.mov_rr(OpSize::S64, Reg::Rdi, CPU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rsi, MEM_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rdx, MMU_PTR);
        emit.mov_rr(OpSize::S64, Reg::Rcx, IO_PTR);
        emit.mov_rm(OpSize::S64, Reg::R8, Reg::Rbp, INTR_STACK_OFF);

        let helper_addr = helpers::jit_interpret_one as *const () as usize as u64;
        emit.call_abs(helper_addr);

        // Check return value: if EAX == JIT_EXIT_BLOCK, early exit.
        let continue_label = emit.new_label();
        emit.cmp_ri(OpSize::S32, Reg::Rax, helpers::JIT_EXIT_BLOCK as i32);
        emit.jcc_label(Cc::Ne, continue_label);

        // Early exit path.
        self.emit_restore_and_ret(emit);

        emit.bind_label(continue_label);
    }

    // ════════════════════════════════════════════════════════════════════
    // Common Emission Helpers
    // ════════════════════════════════════════════════════════════════════

    /// Load a guest GPR into a host temporary register.
    fn emit_load_gpr(&self, emit: &mut Emitter, host_dst: Reg, guest_idx: u8, _size: OpSize) {
        let offset = GPR_OFFSET + guest_idx as i32 * 8;
        // Always load 64-bit from the register file. The ALU operation
        // will use the correct operand size prefix.
        emit.mov_rm(OpSize::S64, host_dst, GPR_BASE, offset);
    }

    /// Store a host temporary register back to a guest GPR.
    fn emit_store_gpr(&self, emit: &mut Emitter, guest_idx: u8, host_src: Reg, size: OpSize) {
        let offset = GPR_OFFSET + guest_idx as i32 * 8;
        match size {
            OpSize::S64 => {
                emit.mov_mr(OpSize::S64, GPR_BASE, offset, host_src);
            }
            OpSize::S32 => {
                // 32-bit op zero-extends the host register to 64-bit.
                // Write back the full 64-bit value (upper 32 = 0).
                emit.mov_mr(OpSize::S64, GPR_BASE, offset, host_src);
            }
            OpSize::S16 => {
                // 16-bit: preserve upper 48 bits, write only low 16.
                emit.mov_mr(OpSize::S16, GPR_BASE, offset, host_src);
            }
            OpSize::S8 => {
                // 8-bit: write only low byte.
                emit.mov_mr(OpSize::S8, GPR_BASE, offset, host_src);
            }
        }
    }

    /// Capture host RFLAGS after a native ALU operation.
    fn emit_capture_flags(&self, emit: &mut Emitter) {
        emit.pushfq();
        emit.pop(FLAGS_TMP);
    }

    /// Merge captured arithmetic flags into guest RFLAGS.
    ///
    /// Clears CF|PF|AF|ZF|SF|OF in guest RFLAGS, then ORs in the
    /// corresponding bits from the captured host RFLAGS.
    fn emit_merge_arith_flags(&self, emit: &mut Emitter) {
        // Load guest RFLAGS.
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RFLAGS_OFFSET);
        // Clear arithmetic flags in guest.
        emit.and_ri(OpSize::S64, Reg::Rax, NOT_ARITH_MASK);
        // Keep only arithmetic flags from captured host RFLAGS.
        emit.and_ri(OpSize::S64, FLAGS_TMP, ARITH_MASK);
        // Merge.
        emit.or_rr(OpSize::S64, Reg::Rax, FLAGS_TMP);
        // Store back.
        emit.mov_mr(OpSize::S64, GPR_BASE, RFLAGS_OFFSET, Reg::Rax);
    }

    /// Advance guest RIP by a fixed instruction length.
    fn emit_advance_rip(&self, emit: &mut Emitter, length: u8) {
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
        emit.add_ri(OpSize::S64, Reg::Rax, length as i32);
        emit.mov_mr(OpSize::S64, GPR_BASE, RIP_OFFSET, Reg::Rax);
    }

    /// Advance guest RIP by a signed delta (for branches).
    fn emit_advance_rip_by(&self, emit: &mut Emitter, delta: i64) {
        emit.mov_rm(OpSize::S64, Reg::Rax, GPR_BASE, RIP_OFFSET);
        if delta >= i32::MIN as i64 && delta <= i32::MAX as i64 {
            emit.add_ri(OpSize::S64, Reg::Rax, delta as i32);
        } else {
            // Large offset: load as 64-bit immediate.
            emit.mov_ri64(Reg::Rcx, delta as u64);
            emit.add_rr(OpSize::S64, Reg::Rax, Reg::Rcx);
        }
        emit.mov_mr(OpSize::S64, GPR_BASE, RIP_OFFSET, Reg::Rax);
    }
}

// ════════════════════════════════════════════════════════════════════════
// Free functions: ALU emission + operand size mapping
// ════════════════════════════════════════════════════════════════════════

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
///
/// Returns `None` for 8-bit (not supported in Phase 3 native translation).
fn map_operand_size(size: OperandSize) -> Option<OpSize> {
    match size {
        OperandSize::Byte  => None,
        OperandSize::Word  => Some(OpSize::S16),
        OperandSize::Dword => Some(OpSize::S32),
        OperandSize::Qword => Some(OpSize::S64),
    }
}

/// Map a guest x86 condition code (0-15) to the emitter's `Cc` enum.
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
