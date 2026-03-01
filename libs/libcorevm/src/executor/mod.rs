//! Top-level instruction executor and dispatch.
//!
//! The [`execute`] function is the main entry point called by the CPU emulation
//! loop for each decoded instruction. It dispatches to `exec_primary` or
//! `exec_secondary` based on the opcode map, and those functions in turn call
//! the appropriate handler in the sub-modules (arith, logic, data, etc.).
//!
//! This module also contains shared helper functions for reading/writing
//! operands, computing effective addresses, and performing stack push/pop
//! operations on the guest stack.

pub mod arith;
pub mod control;
pub mod data;
pub mod fpu;
pub mod logic;
pub mod setcc;
pub mod sse;
pub mod stack;
pub mod string;
pub mod system;

use crate::cpu::{Cpu, Mode};
use crate::error::{Result, VmError};
use crate::flags::OperandSize;
use crate::instruction::{DecodedInst, MemOperand, Operand, OpcodeMap, RegOperand};
use crate::interrupts::InterruptController;
use crate::io::IoDispatch;
use crate::memory::{AccessType, GuestMemory, MemoryBus, Mmu};
use crate::registers::{GprIndex, SegReg};

// ── Public entry point ──

/// Execute a single decoded instruction.
///
/// Dispatches based on the opcode map to `exec_primary` (one-byte opcodes)
/// or `exec_secondary` (0F-prefixed two-byte opcodes). Three-byte escape
/// maps (0F 38, 0F 3A) are not yet supported and raise `#UD`.
///
/// # Errors
///
/// Returns a `VmError` if the instruction faults (e.g. `#GP`, `#PF`, `#UD`,
/// `#DE`) or produces a control-flow event like `VmError::Halted`.
pub fn execute(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
    io: &mut IoDispatch,
    interrupts: &mut InterruptController,
) -> Result<()> {
    match inst.opcode_map {
        OpcodeMap::Primary => exec_primary(cpu, inst, memory, mmu, io, interrupts),
        OpcodeMap::Secondary => exec_secondary(cpu, inst, memory, mmu, io, interrupts),
        OpcodeMap::Escape0F38 | OpcodeMap::Escape0F3A => {
            Err(VmError::UndefinedOpcode(inst.opcode as u8))
        }
    }
}

// ── Primary opcode dispatch (one-byte map) ──

/// Dispatch a primary (one-byte) opcode.
///
/// The 256-entry match covers every primary opcode byte. Group opcodes
/// (0x80-0x83, 0xC0-0xC1, 0xD0-0xD3, 0xF6-0xF7, 0xFE-0xFF) use the
/// ModR/M reg field to further distinguish sub-operations.
fn exec_primary(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
    io: &mut IoDispatch,
    interrupts: &mut InterruptController,
) -> Result<()> {
    let op = inst.opcode as u8;
    match op {
        // ── ADD ──
        0x00..=0x05 => arith::exec_add(cpu, inst, memory, mmu),

        // ── PUSH ES / POP ES (32-bit only) ──
        0x06 => stack::exec_push_seg(cpu, inst, memory, mmu),
        0x07 => stack::exec_pop_seg(cpu, inst, memory, mmu),

        // ── OR ──
        0x08..=0x0D => logic::exec_or(cpu, inst, memory, mmu),

        // ── PUSH CS (32-bit only) ──
        0x0E => stack::exec_push_seg(cpu, inst, memory, mmu),

        // 0x0F = two-byte escape (handled by OpcodeMap::Secondary)
        0x0F => Err(VmError::UndefinedOpcode(op)),

        // ── ADC ──
        0x10..=0x15 => arith::exec_adc(cpu, inst, memory, mmu),

        // ── PUSH SS / POP SS ──
        0x16 => stack::exec_push_seg(cpu, inst, memory, mmu),
        0x17 => stack::exec_pop_seg(cpu, inst, memory, mmu),

        // ── SBB ──
        0x18..=0x1D => arith::exec_sbb(cpu, inst, memory, mmu),

        // ── PUSH DS / POP DS ──
        0x1E => stack::exec_push_seg(cpu, inst, memory, mmu),
        0x1F => stack::exec_pop_seg(cpu, inst, memory, mmu),

        // ── AND ──
        0x20..=0x25 => logic::exec_and(cpu, inst, memory, mmu),

        // 0x26 = ES segment override prefix (handled by decoder)
        0x26 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── DAA (32-bit only) ──
        0x27 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── SUB ──
        0x28..=0x2D => arith::exec_sub(cpu, inst, memory, mmu),

        // 0x2E = CS segment override prefix
        0x2E => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── DAS (32-bit only) ──
        0x2F => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── XOR ──
        0x30..=0x35 => logic::exec_xor(cpu, inst, memory, mmu),

        // 0x36 = SS segment override prefix
        0x36 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── AAA (32-bit only) ──
        0x37 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── CMP ──
        0x38..=0x3D => arith::exec_cmp(cpu, inst, memory, mmu),

        // 0x3E = DS segment override prefix
        0x3E => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── AAS (32-bit only) ──
        0x3F => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── INC r16/r32 (32-bit encoding, REX prefix in 64-bit) ──
        0x40..=0x47 => {
            if matches!(cpu.mode, Mode::LongMode) {
                // REX prefix — should have been consumed by decoder
                Err(VmError::UndefinedOpcode(op))
            } else {
                arith::exec_inc(cpu, inst, memory, mmu)
            }
        }

        // ── DEC r16/r32 (32-bit encoding, REX prefix in 64-bit) ──
        0x48..=0x4F => {
            if matches!(cpu.mode, Mode::LongMode) {
                Err(VmError::UndefinedOpcode(op))
            } else {
                arith::exec_dec(cpu, inst, memory, mmu)
            }
        }

        // ── PUSH r16/r32/r64 ──
        0x50..=0x57 => stack::exec_push_reg(cpu, inst, memory, mmu),

        // ── POP r16/r32/r64 ──
        0x58..=0x5F => stack::exec_pop_reg(cpu, inst, memory, mmu),

        // ── PUSHA/PUSHAD (32-bit only) ──
        0x60 => stack::exec_pusha(cpu, inst, memory, mmu),

        // ── POPA/POPAD (32-bit only) ──
        0x61 => stack::exec_popa(cpu, inst, memory, mmu),

        // ── BOUND (32-bit only) ──
        0x62 => Err(VmError::UndefinedOpcode(op)),

        // ── ARPL (32-bit) / MOVSXD (64-bit) ──
        0x63 => {
            if matches!(cpu.mode, Mode::LongMode) {
                data::exec_movsxd(cpu, inst, memory, mmu)
            } else {
                // ARPL not implemented
                Err(VmError::UndefinedOpcode(op))
            }
        }

        // 0x64 = FS segment override prefix
        // 0x65 = GS segment override prefix
        // 0x66 = operand-size override prefix
        // 0x67 = address-size override prefix
        0x64..=0x67 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PUSH imm16/imm32 ──
        0x68 => stack::exec_push_imm(cpu, inst, memory, mmu),

        // ── IMUL r, r/m, imm16/imm32 ──
        0x69 => arith::exec_imul_3op(cpu, inst, memory, mmu),

        // ── PUSH imm8 (sign-extended) ──
        0x6A => stack::exec_push_imm(cpu, inst, memory, mmu),

        // ── IMUL r, r/m, imm8 ──
        0x6B => arith::exec_imul_3op(cpu, inst, memory, mmu),

        // ── INS/OUTS string I/O (not implemented) ──
        0x6C..=0x6F => Err(VmError::UndefinedOpcode(op)),

        // ── Jcc short (rel8) ──
        0x70..=0x7F => control::exec_jcc(cpu, inst),

        // ── Group 1: immediate ALU ops (ADD/OR/ADC/SBB/AND/SUB/XOR/CMP r/m, imm) ──
        0x80..=0x83 => {
            let reg = inst.modrm_reg() & 7;
            match reg {
                0 => arith::exec_add(cpu, inst, memory, mmu),
                1 => logic::exec_or(cpu, inst, memory, mmu),
                2 => arith::exec_adc(cpu, inst, memory, mmu),
                3 => arith::exec_sbb(cpu, inst, memory, mmu),
                4 => logic::exec_and(cpu, inst, memory, mmu),
                5 => arith::exec_sub(cpu, inst, memory, mmu),
                6 => logic::exec_xor(cpu, inst, memory, mmu),
                7 => arith::exec_cmp(cpu, inst, memory, mmu),
                _ => unreachable!(),
            }
        }

        // ── TEST r/m8, r8 ──
        0x84 => logic::exec_test(cpu, inst, memory, mmu),
        // ── TEST r/m16-64, r16-64 ──
        0x85 => logic::exec_test(cpu, inst, memory, mmu),

        // ── XCHG r/m, r ──
        0x86 | 0x87 => data::exec_xchg(cpu, inst, memory, mmu),

        // ── MOV r/m, r and MOV r, r/m ──
        0x88..=0x8B => data::exec_mov(cpu, inst, memory, mmu),

        // ── MOV r/m16, Sreg ──
        0x8C => data::exec_mov_seg(cpu, inst, memory, mmu),

        // ── LEA r, m ──
        0x8D => data::exec_lea(cpu, inst),

        // ── MOV Sreg, r/m16 ──
        0x8E => data::exec_mov_seg(cpu, inst, memory, mmu),

        // ── POP r/m ──
        0x8F => stack::exec_pop_rm(cpu, inst, memory, mmu),

        // ── NOP / XCHG rAX, r ──
        0x90 => {
            // 0x90 is NOP (XCHG EAX, EAX) — but PAUSE with F3 prefix
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── XCHG rAX, r16-64 ──
        0x91..=0x97 => data::exec_xchg(cpu, inst, memory, mmu),

        // ── CBW/CWDE/CDQE ──
        0x98 => data::exec_cbw_cwde_cdqe(cpu, inst),

        // ── CWD/CDQ/CQO ──
        0x99 => data::exec_cwd_cdq_cqo(cpu, inst),

        // ── CALL far (32-bit only) ──
        0x9A => control::exec_call_far(cpu, inst, memory, mmu),

        // ── FWAIT ──
        0x9B => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PUSHF/PUSHFD/PUSHFQ ──
        0x9C => stack::exec_pushf(cpu, inst, memory, mmu),

        // ── POPF/POPFD/POPFQ ──
        0x9D => stack::exec_popf(cpu, inst, memory, mmu),

        // ── SAHF ──
        0x9E => data::exec_sahf(cpu, inst),

        // ── LAHF ──
        0x9F => data::exec_lahf(cpu, inst),

        // ── MOV AL/AX/EAX/RAX, moffs ──
        0xA0..=0xA3 => data::exec_mov(cpu, inst, memory, mmu),

        // ── MOVS (string) ──
        0xA4 | 0xA5 => string::exec_movs(cpu, inst, memory, mmu),

        // ── CMPS (string) ──
        0xA6 | 0xA7 => string::exec_cmps(cpu, inst, memory, mmu),

        // ── TEST AL/AX/EAX/RAX, imm ──
        0xA8 | 0xA9 => logic::exec_test(cpu, inst, memory, mmu),

        // ── STOS (string) ──
        0xAA | 0xAB => string::exec_stos(cpu, inst, memory, mmu),

        // ── LODS (string) ──
        0xAC | 0xAD => string::exec_lods(cpu, inst, memory, mmu),

        // ── SCAS (string) ──
        0xAE | 0xAF => string::exec_scas(cpu, inst, memory, mmu),

        // ── MOV r8, imm8 ──
        0xB0..=0xB7 => data::exec_mov(cpu, inst, memory, mmu),

        // ── MOV r16-64, imm16-64 ──
        0xB8..=0xBF => data::exec_mov(cpu, inst, memory, mmu),

        // ── Group 2: shift/rotate r/m, imm8 ──
        0xC0 | 0xC1 => exec_group2(cpu, inst, memory, mmu),

        // ── RET near imm16 ──
        0xC2 => control::exec_ret_near_imm(cpu, inst, memory, mmu),

        // ── RET near ──
        0xC3 => control::exec_ret_near(cpu, inst, memory, mmu),

        // ── LES/LDS (32-bit only, invalid in 64-bit) ──
        0xC4 | 0xC5 => Err(VmError::UndefinedOpcode(op)),

        // ── MOV r/m8, imm8 ──
        0xC6 => data::exec_mov(cpu, inst, memory, mmu),
        // ── MOV r/m16-64, imm16-32 ──
        0xC7 => data::exec_mov(cpu, inst, memory, mmu),

        // ── ENTER ──
        0xC8 => stack::exec_enter(cpu, inst, memory, mmu),

        // ── LEAVE ──
        0xC9 => stack::exec_leave(cpu, inst, memory, mmu),

        // ── RET far imm16 ──
        0xCA => control::exec_ret_far(cpu, inst, memory, mmu),

        // ── RET far ──
        0xCB => control::exec_ret_far(cpu, inst, memory, mmu),

        // ── INT 3 ──
        0xCC => control::exec_int3(cpu, inst, memory, mmu, interrupts),

        // ── INT imm8 ──
        0xCD => control::exec_int(cpu, inst, memory, mmu, interrupts),

        // ── INTO ──
        0xCE => control::exec_into(cpu, inst, memory, mmu, interrupts),

        // ── IRET/IRETD/IRETQ ──
        0xCF => control::exec_iret(cpu, inst, memory, mmu),

        // ── Group 2: shift/rotate r/m, 1 ──
        0xD0 | 0xD1 => exec_group2(cpu, inst, memory, mmu),

        // ── Group 2: shift/rotate r/m, CL ──
        0xD2 | 0xD3 => exec_group2(cpu, inst, memory, mmu),

        // ── AAM / AAD (32-bit only) ──
        0xD4 | 0xD5 => Err(VmError::UndefinedOpcode(op)),

        // ── SALC (undocumented, 32-bit) ──
        0xD6 => Err(VmError::UndefinedOpcode(op)),

        // ── XLAT ──
        0xD7 => data::exec_xlat(cpu, inst, memory, mmu),

        // ── x87 FPU escape opcodes (D8-DF) ──
        0xD8..=0xDF => fpu::exec_fpu(cpu, inst, memory, mmu),

        // ── LOOPNE / LOOPE / LOOP ──
        0xE0..=0xE2 => control::exec_loop(cpu, inst),

        // ── JCXZ/JECXZ/JRCXZ ──
        0xE3 => control::exec_jcxz(cpu, inst),

        // ── IN AL, imm8 ──
        0xE4 => exec_in_imm(cpu, inst, io),
        // ── IN eAX, imm8 ──
        0xE5 => exec_in_imm(cpu, inst, io),
        // ── OUT imm8, AL ──
        0xE6 => exec_out_imm(cpu, inst, io),
        // ── OUT imm8, eAX ──
        0xE7 => exec_out_imm(cpu, inst, io),

        // ── CALL rel16/rel32 ──
        0xE8 => control::exec_call_rel(cpu, inst, memory, mmu),

        // ── JMP rel16/rel32 ──
        0xE9 => control::exec_jmp_rel(cpu, inst),

        // ── JMP far (32-bit only) ──
        0xEA => control::exec_jmp_far(cpu, inst, memory, mmu),

        // ── JMP rel8 ──
        0xEB => control::exec_jmp_rel(cpu, inst),

        // ── IN AL, DX ──
        0xEC => exec_in_dx(cpu, inst, io),
        // ── IN eAX, DX ──
        0xED => exec_in_dx(cpu, inst, io),
        // ── OUT DX, AL ──
        0xEE => exec_out_dx(cpu, inst, io),
        // ── OUT DX, eAX ──
        0xEF => exec_out_dx(cpu, inst, io),

        // ── LOCK prefix (handled by decoder) ──
        0xF0 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── INT 1 (debug trap, ICEBP) ──
        0xF1 => Err(VmError::DebugException),

        // ── REPNE / REP prefix (handled by decoder) ──
        0xF2 | 0xF3 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── HLT ──
        0xF4 => system::exec_hlt(cpu, inst),

        // ── CMC (complement carry flag) ──
        0xF5 => {
            cpu.regs.rflags ^= crate::flags::CF;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── Group 3 Eb: TEST/NOT/NEG/MUL/IMUL/DIV/IDIV r/m8 ──
        0xF6 => {
            let reg = inst.modrm_reg() & 7;
            match reg {
                0 | 1 => logic::exec_test(cpu, inst, memory, mmu),
                2 => logic::exec_not(cpu, inst, memory, mmu),
                3 => arith::exec_neg(cpu, inst, memory, mmu),
                4 => arith::exec_mul(cpu, inst, memory, mmu),
                5 => arith::exec_imul_1op(cpu, inst, memory, mmu),
                6 => arith::exec_div(cpu, inst, memory, mmu),
                7 => arith::exec_idiv(cpu, inst, memory, mmu),
                _ => unreachable!(),
            }
        }

        // ── Group 3 Ev: TEST/NOT/NEG/MUL/IMUL/DIV/IDIV r/m16-64 ──
        0xF7 => {
            let reg = inst.modrm_reg() & 7;
            match reg {
                0 | 1 => logic::exec_test(cpu, inst, memory, mmu),
                2 => logic::exec_not(cpu, inst, memory, mmu),
                3 => arith::exec_neg(cpu, inst, memory, mmu),
                4 => arith::exec_mul(cpu, inst, memory, mmu),
                5 => arith::exec_imul_1op(cpu, inst, memory, mmu),
                6 => arith::exec_div(cpu, inst, memory, mmu),
                7 => arith::exec_idiv(cpu, inst, memory, mmu),
                _ => unreachable!(),
            }
        }

        // ── CLC ──
        0xF8 => {
            cpu.regs.rflags &= !crate::flags::CF;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── STC ──
        0xF9 => {
            cpu.regs.rflags |= crate::flags::CF;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── CLI ──
        0xFA => {
            cpu.regs.rflags &= !crate::flags::IF;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── STI ──
        0xFB => {
            cpu.regs.rflags |= crate::flags::IF;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── CLD ──
        0xFC => {
            cpu.regs.rflags &= !crate::flags::DF;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── STD ──
        0xFD => {
            cpu.regs.rflags |= crate::flags::DF;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── Group 4: INC/DEC r/m8 ──
        0xFE => {
            let reg = inst.modrm_reg() & 7;
            match reg {
                0 => arith::exec_inc(cpu, inst, memory, mmu),
                1 => arith::exec_dec(cpu, inst, memory, mmu),
                _ => Err(VmError::UndefinedOpcode(op)),
            }
        }

        // ── Group 5: INC/DEC/CALL/CALL far/JMP/JMP far/PUSH r/m ──
        0xFF => {
            let reg = inst.modrm_reg() & 7;
            match reg {
                0 => arith::exec_inc(cpu, inst, memory, mmu),
                1 => arith::exec_dec(cpu, inst, memory, mmu),
                2 => control::exec_call_rm(cpu, inst, memory, mmu),
                3 => control::exec_call_far(cpu, inst, memory, mmu),
                4 => control::exec_jmp_rm(cpu, inst, memory, mmu),
                5 => control::exec_jmp_far(cpu, inst, memory, mmu),
                6 => stack::exec_push_rm(cpu, inst, memory, mmu),
                _ => Err(VmError::UndefinedOpcode(op)),
            }
        }
    }
}

// ── Secondary opcode dispatch (0F xx) ──

/// Dispatch a secondary (0F-prefixed two-byte) opcode.
fn exec_secondary(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
    _io: &mut IoDispatch,
    _interrupts: &mut InterruptController,
) -> Result<()> {
    // The secondary byte is in the low 8 bits of inst.opcode
    let op2 = inst.opcode as u8;
    match op2 {
        // ── Group 6 (SLDT/STR/LLDT/LTR/VERR/VERW) — mostly unimplemented ──
        0x00 => Err(VmError::UndefinedOpcode(op2)),

        // ── Group 7 (SGDT/SIDT/LGDT/LIDT/SMSW/LMSW/INVLPG/SWAPGS) ──
        0x01 => {
            let reg = inst.modrm_reg() & 7;
            match reg {
                0 => system::exec_sgdt(cpu, inst, memory, mmu),
                1 => system::exec_sidt(cpu, inst, memory, mmu),
                2 => system::exec_lgdt(cpu, inst, memory, mmu),
                3 => system::exec_lidt(cpu, inst, memory, mmu),
                4 => system::exec_smsw(cpu, inst, memory, mmu),
                6 => system::exec_lmsw(cpu, inst, memory, mmu),
                7 => {
                    // INVLPG if modrm_mod != 3, else SWAPGS/RDTSCP
                    if inst.modrm_mod() != 3 {
                        system::exec_invlpg(cpu, inst, memory, mmu)
                    } else {
                        let rm = inst.modrm_rm() & 7;
                        match rm {
                            0 => system::exec_swapgs(cpu, inst),
                            _ => Err(VmError::UndefinedOpcode(op2)),
                        }
                    }
                }
                _ => Err(VmError::UndefinedOpcode(op2)),
            }
        }

        // ── MOV r, CRn / MOV CRn, r ──
        0x20 | 0x22 => system::exec_mov_cr(cpu, inst),

        // ── MOV r, DRn / MOV DRn, r ──
        0x21 | 0x23 => system::exec_mov_dr(cpu, inst),

        // ── WRMSR ──
        0x30 => system::exec_wrmsr(cpu, inst),

        // ── RDTSC ──
        0x31 => system::exec_rdtsc(cpu, inst),

        // ── RDMSR ──
        0x32 => system::exec_rdmsr(cpu, inst),

        // ── CPUID ──
        0x05 => system::exec_syscall(cpu, inst),
        0x07 => system::exec_sysret(cpu, inst),

        // ── SYSENTER/SYSEXIT (not implemented) ──
        0x34 | 0x35 => Err(VmError::UndefinedOpcode(op2)),

        // ── WBINVD ──
        0x09 => system::exec_wbinvd(cpu, inst),

        // ── CLTS ──
        0x06 => system::exec_clts(cpu, inst),

        // ── CMOVcc ──
        0x40..=0x4F => data::exec_cmov(cpu, inst, memory, mmu),

        // ── Jcc near (rel16/rel32) ──
        0x80..=0x8F => control::exec_jcc(cpu, inst),

        // ── SETcc ──
        0x90..=0x9F => setcc::exec_setcc(cpu, inst, memory, mmu),

        // ── PUSH FS ──
        0xA0 => stack::exec_push_seg(cpu, inst, memory, mmu),
        // ── POP FS ──
        0xA1 => stack::exec_pop_seg(cpu, inst, memory, mmu),

        // ── CPUID ──
        0xA2 => system::exec_cpuid(cpu, inst),

        // ── BT r/m, r ──
        0xA3 => logic::exec_bt(cpu, inst, memory, mmu),

        // ── SHLD r/m, r, imm8 ──
        0xA4 => logic::exec_shl(cpu, inst, memory, mmu),
        // ── SHLD r/m, r, CL ──
        0xA5 => logic::exec_shl(cpu, inst, memory, mmu),

        // ── PUSH GS ──
        0xA8 => stack::exec_push_seg(cpu, inst, memory, mmu),
        // ── POP GS ──
        0xA9 => stack::exec_pop_seg(cpu, inst, memory, mmu),

        // ── BTS r/m, r ──
        0xAB => logic::exec_bts(cpu, inst, memory, mmu),

        // ── SHRD r/m, r, imm8 ──
        0xAC => logic::exec_shr(cpu, inst, memory, mmu),
        // ── SHRD r/m, r, CL ──
        0xAD => logic::exec_shr(cpu, inst, memory, mmu),

        // ── IMUL r, r/m ──
        0xAF => arith::exec_imul_2op(cpu, inst, memory, mmu),

        // ── CMPXCHG ──
        0xB0 | 0xB1 => data::exec_cmpxchg(cpu, inst, memory, mmu),

        // ── BTR r/m, r ──
        0xB3 => logic::exec_btr(cpu, inst, memory, mmu),

        // ── MOVZX r, r/m8 ──
        0xB6 => data::exec_movzx(cpu, inst, memory, mmu),
        // ── MOVZX r, r/m16 ──
        0xB7 => data::exec_movzx(cpu, inst, memory, mmu),

        // ── Group 8: BT/BTS/BTR/BTC r/m, imm8 ──
        0xBA => {
            let reg = inst.modrm_reg() & 7;
            match reg {
                4 => logic::exec_bt(cpu, inst, memory, mmu),
                5 => logic::exec_bts(cpu, inst, memory, mmu),
                6 => logic::exec_btr(cpu, inst, memory, mmu),
                7 => logic::exec_btc(cpu, inst, memory, mmu),
                _ => Err(VmError::UndefinedOpcode(op2)),
            }
        }

        // ── BTC r/m, r ──
        0xBB => logic::exec_btc(cpu, inst, memory, mmu),

        // ── BSF ──
        0xBC => logic::exec_bsf(cpu, inst, memory, mmu),

        // ── BSR ──
        0xBD => logic::exec_bsr(cpu, inst, memory, mmu),

        // ── MOVSX r, r/m8 ──
        0xBE => data::exec_movsx(cpu, inst, memory, mmu),
        // ── MOVSX r, r/m16 ──
        0xBF => data::exec_movsx(cpu, inst, memory, mmu),

        // ── XADD ──
        0xC0 | 0xC1 => data::exec_xadd(cpu, inst, memory, mmu),

        // ── BSWAP r32/r64 ──
        0xC8..=0xCF => data::exec_bswap(cpu, inst),

        // ── SSE instructions (various prefixes) ──
        0x10..=0x17 | 0x28..=0x2F | 0x50..=0x7F | 0xC2..=0xC6 | 0xD0..=0xFE => {
            sse::exec_sse(cpu, inst, memory, mmu)
        }

        // ── Everything else: #UD ──
        _ => Err(VmError::UndefinedOpcode(op2)),
    }
}

// ── Shift/Rotate group dispatcher ──

/// Dispatch Group 2 (shift/rotate) opcodes (C0, C1, D0-D3).
fn exec_group2(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let reg = inst.modrm_reg() & 7;
    match reg {
        0 => logic::exec_rol(cpu, inst, memory, mmu),
        1 => logic::exec_ror(cpu, inst, memory, mmu),
        2 => logic::exec_rcl(cpu, inst, memory, mmu),
        3 => logic::exec_rcr(cpu, inst, memory, mmu),
        4 => logic::exec_shl(cpu, inst, memory, mmu),
        5 => logic::exec_shr(cpu, inst, memory, mmu),
        6 => logic::exec_shl(cpu, inst, memory, mmu), // SAL is an alias for SHL
        7 => logic::exec_sar(cpu, inst, memory, mmu),
        _ => unreachable!(),
    }
}

// ── I/O helpers ──

/// IN AL/AX/EAX, imm8 — read from an immediate port number.
fn exec_in_imm(cpu: &mut Cpu, inst: &DecodedInst, io: &mut IoDispatch) -> Result<()> {
    let port = inst.immediate as u16;
    let size = inst.operand_size.bytes() as u8;
    let size_io = if size > 4 { 4 } else { size };
    let val = io.port_in(port, size_io)?;
    match size_io {
        1 => cpu.regs.write_gpr8(GprIndex::Rax as u8, false, val as u8),
        2 => cpu.regs.write_gpr16(GprIndex::Rax as u8, val as u16),
        _ => cpu.regs.write_gpr32(GprIndex::Rax as u8, val),
    }
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// IN AL/AX/EAX, DX — read from port number in DX.
fn exec_in_dx(cpu: &mut Cpu, inst: &DecodedInst, io: &mut IoDispatch) -> Result<()> {
    let port = cpu.regs.read_gpr16(GprIndex::Rdx as u8);
    let size = inst.operand_size.bytes() as u8;
    let size_io = if size > 4 { 4 } else { size };
    let val = io.port_in(port, size_io)?;
    match size_io {
        1 => cpu.regs.write_gpr8(GprIndex::Rax as u8, false, val as u8),
        2 => cpu.regs.write_gpr16(GprIndex::Rax as u8, val as u16),
        _ => cpu.regs.write_gpr32(GprIndex::Rax as u8, val),
    }
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// OUT imm8, AL/AX/EAX — write to an immediate port number.
fn exec_out_imm(cpu: &mut Cpu, inst: &DecodedInst, io: &mut IoDispatch) -> Result<()> {
    let port = inst.immediate as u16;
    let size = inst.operand_size.bytes() as u8;
    let size_io = if size > 4 { 4 } else { size };
    let val = cpu.regs.read_gpr(GprIndex::Rax as u8, inst.operand_size, false) as u32;
    io.port_out(port, size_io, val)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// OUT DX, AL/AX/EAX — write to port number in DX.
fn exec_out_dx(cpu: &mut Cpu, inst: &DecodedInst, io: &mut IoDispatch) -> Result<()> {
    let port = cpu.regs.read_gpr16(GprIndex::Rdx as u8);
    let size = inst.operand_size.bytes() as u8;
    let size_io = if size > 4 { 4 } else { size };
    let val = cpu.regs.read_gpr(GprIndex::Rax as u8, inst.operand_size, false) as u32;
    io.port_out(port, size_io, val)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── Shared operand access helpers ──

/// Read the value of an instruction operand.
///
/// Handles register, memory, immediate, and relative-offset operands.
/// For memory operands, computes the effective address, translates via
/// the MMU, and reads the appropriate number of bytes.
pub fn read_operand(
    cpu: &Cpu,
    inst: &DecodedInst,
    operand: &Operand,
    memory: &GuestMemory,
    mmu: &Mmu,
) -> Result<u64> {
    match operand {
        Operand::None => Ok(0),
        Operand::Register(reg_op) => read_reg_operand(cpu, inst, reg_op),
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            translate_and_read(cpu, linear, mem_op.size, mmu, memory)
        }
        Operand::Immediate(imm) => Ok(*imm),
        Operand::RelativeOffset(off) => Ok(*off as u64),
        Operand::FarPointer { offset, .. } => Ok(*offset),
    }
}

/// Write a value to an instruction operand.
///
/// Handles register and memory operands. Writing to an immediate or
/// relative-offset operand returns `#GP(0)`.
pub fn write_operand(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    operand: &Operand,
    val: u64,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    match operand {
        Operand::Register(reg_op) => write_reg_operand(cpu, inst, reg_op, val),
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            translate_and_write(cpu, linear, mem_op.size, val, mmu, memory)
        }
        _ => Err(VmError::GeneralProtection(0)),
    }
}

/// Read a register operand value.
fn read_reg_operand(cpu: &Cpu, inst: &DecodedInst, reg: &RegOperand) -> Result<u64> {
    match reg {
        RegOperand::Gpr(idx) => Ok(cpu.regs.read_gpr(*idx, inst.operand_size, inst.prefix.has_rex())),
        RegOperand::Seg(sr) => Ok(cpu.regs.segment(*sr).selector as u64),
        RegOperand::Cr(cr) => match cr {
            0 => Ok(cpu.regs.cr0),
            2 => Ok(cpu.regs.cr2),
            3 => Ok(cpu.regs.cr3),
            4 => Ok(cpu.regs.cr4),
            8 => Ok(cpu.regs.cr8),
            _ => Err(VmError::UndefinedOpcode(0)),
        },
        RegOperand::Dr(dr) => {
            if (*dr as usize) < cpu.regs.dr.len() {
                Ok(cpu.regs.dr[*dr as usize])
            } else {
                Err(VmError::UndefinedOpcode(0))
            }
        }
        RegOperand::Xmm(idx) => Ok(cpu.sse.xmm[*idx as usize].lo),
        RegOperand::Fpu(idx) => Ok(cpu.fpu.st(*idx).to_bits()),
    }
}

/// Write a value to a register operand.
fn write_reg_operand(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    reg: &RegOperand,
    val: u64,
) -> Result<()> {
    match reg {
        RegOperand::Gpr(idx) => {
            cpu.regs.write_gpr(*idx, inst.operand_size, inst.prefix.has_rex(), val);
            Ok(())
        }
        RegOperand::Seg(sr) => {
            let sel = val as u16;
            match cpu.mode {
                Mode::RealMode => cpu.regs.load_segment_real(*sr, sel),
                _ => {
                    // In protected/long mode, a full descriptor load would be
                    // needed. For simplicity we update the selector field;
                    // the caller (exec_mov_seg) handles full descriptor loading.
                    cpu.regs.segment_mut(*sr).selector = sel;
                }
            }
            Ok(())
        }
        RegOperand::Cr(cr) => {
            match cr {
                0 => cpu.regs.cr0 = val,
                2 => cpu.regs.cr2 = val,
                3 => cpu.regs.cr3 = val,
                4 => cpu.regs.cr4 = val,
                8 => cpu.regs.cr8 = val,
                _ => return Err(VmError::UndefinedOpcode(0)),
            }
            Ok(())
        }
        RegOperand::Dr(dr) => {
            if (*dr as usize) < cpu.regs.dr.len() {
                cpu.regs.dr[*dr as usize] = val;
                Ok(())
            } else {
                Err(VmError::UndefinedOpcode(0))
            }
        }
        RegOperand::Xmm(idx) => {
            cpu.sse.xmm[*idx as usize].lo = val;
            Ok(())
        }
        RegOperand::Fpu(idx) => {
            cpu.fpu.set_st(*idx, f64::from_bits(val));
            Ok(())
        }
    }
}

/// Compute the linear (virtual) effective address for a memory operand.
///
/// Assembles the address from the base register, index register (scaled),
/// displacement, and segment base. RIP-relative addressing is handled for
/// 64-bit mode.
pub fn compute_effective_address(
    cpu: &Cpu,
    mem_op: &MemOperand,
    inst: &DecodedInst,
) -> Result<u64> {
    let mut addr: u64 = 0;

    // Base register
    if let Some(base) = mem_op.base {
        addr = addr.wrapping_add(cpu.regs.read_gpr64(base));
    }

    // Index register * scale
    if let Some(index) = mem_op.index {
        let index_val = cpu.regs.read_gpr64(index);
        addr = addr.wrapping_add(index_val.wrapping_mul(mem_op.scale as u64));
    }

    // Displacement
    addr = addr.wrapping_add(mem_op.displacement as u64);

    // RIP-relative addressing
    if mem_op.rip_relative {
        // RIP points to the *next* instruction at this point
        let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);
        addr = next_rip.wrapping_add(mem_op.displacement as u64);
    }

    // Mask to address size
    let mask = inst.address_size.mask();
    addr &= mask;

    // Add segment base
    let seg_desc = cpu.regs.segment(mem_op.segment);
    addr = seg_desc.base.wrapping_add(addr);

    Ok(addr)
}

/// Translate a linear address via the MMU and read a value of the given size.
pub fn translate_and_read(
    cpu: &Cpu,
    linear: u64,
    size: OperandSize,
    mmu: &Mmu,
    memory: &GuestMemory,
) -> Result<u64> {
    let phys = mmu.translate_linear(linear, cpu.regs.cr3, AccessType::Read, cpu.regs.cpl, memory)?;
    match size {
        OperandSize::Byte => Ok(memory.read_u8(phys)? as u64),
        OperandSize::Word => Ok(memory.read_u16(phys)? as u64),
        OperandSize::Dword => Ok(memory.read_u32(phys)? as u64),
        OperandSize::Qword => Ok(memory.read_u64(phys)?),
    }
}

/// Translate a linear address via the MMU and write a value of the given size.
pub fn translate_and_write(
    cpu: &Cpu,
    linear: u64,
    size: OperandSize,
    val: u64,
    mmu: &Mmu,
    memory: &mut GuestMemory,
) -> Result<()> {
    let phys = mmu.translate_linear(linear, cpu.regs.cr3, AccessType::Write, cpu.regs.cpl, memory)?;
    match size {
        OperandSize::Byte => memory.write_u8(phys, val as u8),
        OperandSize::Word => memory.write_u16(phys, val as u16),
        OperandSize::Dword => memory.write_u32(phys, val as u32),
        OperandSize::Qword => memory.write_u64(phys, val),
    }
}

/// Push a value onto the guest stack.
///
/// Decrements RSP/ESP/SP by the operand size, then writes the value at the
/// new stack pointer address. The stack segment (SS) is used for translation.
pub fn push_val(
    cpu: &mut Cpu,
    val: u64,
    size: OperandSize,
    mmu: &Mmu,
    memory: &mut GuestMemory,
) -> Result<()> {
    let bytes = size.bytes() as u64;
    let sp = cpu.regs.sp().wrapping_sub(bytes);

    // Compute the linear address through SS
    let ss = cpu.regs.segment(SegReg::Ss);
    let linear = ss.base.wrapping_add(sp);

    translate_and_write(cpu, linear, size, val, mmu, memory)?;

    // Update SP (mask to address width)
    match stack_address_size(cpu) {
        OperandSize::Word => cpu.regs.write_gpr16(GprIndex::Rsp as u8, sp as u16),
        OperandSize::Dword => cpu.regs.write_gpr32(GprIndex::Rsp as u8, sp as u32),
        _ => cpu.regs.set_sp(sp),
    }

    Ok(())
}

/// Pop a value from the guest stack.
///
/// Reads the value at the current stack pointer, then increments RSP/ESP/SP
/// by the operand size. The stack segment (SS) is used for translation.
pub fn pop_val(
    cpu: &mut Cpu,
    size: OperandSize,
    mmu: &Mmu,
    memory: &GuestMemory,
) -> Result<u64> {
    let sp = cpu.regs.sp();
    let bytes = size.bytes() as u64;

    // Compute the linear address through SS
    let ss = cpu.regs.segment(SegReg::Ss);
    let linear = ss.base.wrapping_add(sp);

    let val = translate_and_read(cpu, linear, size, mmu, memory)?;

    // Update SP
    let new_sp = sp.wrapping_add(bytes);
    match stack_address_size(cpu) {
        OperandSize::Word => cpu.regs.write_gpr16(GprIndex::Rsp as u8, new_sp as u16),
        OperandSize::Dword => cpu.regs.write_gpr32(GprIndex::Rsp as u8, new_sp as u32),
        _ => cpu.regs.set_sp(new_sp),
    }

    Ok(val)
}

/// Determine the stack address size based on the current CPU mode.
pub fn stack_address_size(cpu: &Cpu) -> OperandSize {
    match cpu.mode {
        Mode::RealMode => OperandSize::Word,
        Mode::ProtectedMode => {
            if cpu.regs.segment(SegReg::Ss).big {
                OperandSize::Dword
            } else {
                OperandSize::Word
            }
        }
        Mode::LongMode => OperandSize::Qword,
    }
}

/// Determine the default operand size for stack operations (push/pop width).
pub fn stack_operand_size(cpu: &Cpu, inst: &DecodedInst) -> OperandSize {
    match cpu.mode {
        Mode::LongMode => {
            // In 64-bit mode, push/pop default to 64-bit, 0x66 prefix gives 16-bit
            if inst.prefix.operand_size_override {
                OperandSize::Word
            } else {
                OperandSize::Qword
            }
        }
        _ => inst.operand_size,
    }
}
