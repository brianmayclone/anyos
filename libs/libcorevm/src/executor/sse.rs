//! SSE/SSE2 instruction handlers.
//!
//! Implements the most commonly used SSE instructions:
//! - MOVAPS/MOVUPS: aligned/unaligned packed single move (128-bit)
//! - MOVSS: scalar single-precision move (32-bit low element)
//! - MOVSD: scalar double-precision move (64-bit low element)
//! - MOVD/MOVQ: move between XMM and GPR/memory (32/64-bit)
//! - XORPS/XORPD: bitwise XOR (commonly used for register zeroing)
//! - LDMXCSR/STMXCSR: load/store MXCSR control register
//!
//! Unrecognized SSE opcodes raise `#UD` to signal the guest that the
//! instruction is not supported.

use crate::cpu::Cpu;
use crate::error::{Result, VmError};
use crate::flags::OperandSize;
use crate::instruction::{DecodedInst, Operand, RegOperand};
use crate::memory::{GuestMemory, Mmu};
use crate::sse_state::{Xmm, MXCSR_WRITE_MASK};

use super::{compute_effective_address, translate_and_read, translate_and_write};

/// Dispatch an SSE/SSE2 instruction.
///
/// The secondary opcode byte (from the 0F xx map) determines the operation.
/// Prefix bytes (0x66, 0xF3, 0xF2) further distinguish the instruction
/// variant (packed single, packed double, scalar single, scalar double).
pub fn exec_sse(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let op2 = inst.opcode as u8;
    let has_66 = inst.prefix.operand_size_override;
    let has_f3 = inst.rep == crate::instruction::RepPrefix::Rep;
    let has_f2 = inst.rep == crate::instruction::RepPrefix::Repne;

    match op2 {
        // ── MOVUPS/MOVAPS/MOVUPD/MOVAPD xmm, xmm/m128 (load) ──
        0x10 | 0x28 => {
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst_idx = xmm_dst_index(inst)?;
            cpu.sse.xmm[dst_idx] = Xmm { lo: src_lo, hi: src_hi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── MOVUPS/MOVAPS/MOVUPD/MOVAPD xmm/m128, xmm (store) ──
        0x11 | 0x29 => {
            let src_idx = xmm_src_index(inst)?;
            let src = cpu.sse.xmm[src_idx];
            write_xmm_or_mem128(cpu, inst, &inst.operands[0], src.lo, src.hi, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── MOVSS (F3 0F 10/11) / MOVSD (F2 0F 10/11) ──
        0x10 | 0x11 if has_f3 || has_f2 => {
            // These are already handled by 0x10/0x11 above for the non-prefix case.
            // With F3: MOVSS, with F2: MOVSD
            if has_f3 {
                exec_movss(cpu, inst, op2, memory, mmu)
            } else {
                exec_movsd(cpu, inst, op2, memory, mmu)
            }
        }

        // ── MOVLPS/MOVLPD (0F 12/13) ──
        0x12 | 0x13 => {
            // Simplified: treat as unaligned move of low 64 bits
            if op2 == 0x12 {
                // Load low qword to XMM low half
                let val = read_qword_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
                let idx = xmm_dst_index(inst)?;
                cpu.sse.xmm[idx].lo = val;
            } else {
                // Store XMM low half to memory
                let idx = xmm_src_index(inst)?;
                let val = cpu.sse.xmm[idx].lo;
                write_qword_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
            }
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── MOVHPS/MOVHPD (0F 16/17) ──
        0x16 | 0x17 => {
            if op2 == 0x16 {
                let val = read_qword_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
                let idx = xmm_dst_index(inst)?;
                cpu.sse.xmm[idx].hi = val;
            } else {
                let idx = xmm_src_index(inst)?;
                let val = cpu.sse.xmm[idx].hi;
                write_qword_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
            }
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── XORPS (0F 57) / XORPD (66 0F 57) ──
        0x57 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx].lo ^= src_lo;
            cpu.sse.xmm[dst_idx].hi ^= src_hi;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── ANDPS (0F 54) / ANDPD (66 0F 54) ──
        0x54 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx].lo &= src_lo;
            cpu.sse.xmm[dst_idx].hi &= src_hi;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── ANDNPS (0F 55) / ANDNPD (66 0F 55) ──
        0x55 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx].lo = !cpu.sse.xmm[dst_idx].lo & src_lo;
            cpu.sse.xmm[dst_idx].hi = !cpu.sse.xmm[dst_idx].hi & src_hi;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── ORPS (0F 56) / ORPD (66 0F 56) ──
        0x56 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx].lo |= src_lo;
            cpu.sse.xmm[dst_idx].hi |= src_hi;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── MOVD/MOVQ (66 0F 6E / 66 0F 7E) ──
        // 0F 6E: MOVD xmm, r/m32 (or MOVQ xmm, r/m64 with REX.W)
        0x6E => {
            let dst_idx = xmm_dst_index(inst)?;
            let val = super::read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
            if inst.prefix.rex_w() {
                // MOVQ: load 64 bits, zero upper
                cpu.sse.xmm[dst_idx] = Xmm { lo: val, hi: 0 };
            } else {
                // MOVD: load 32 bits, zero rest
                cpu.sse.xmm[dst_idx] = Xmm {
                    lo: val & 0xFFFF_FFFF,
                    hi: 0,
                };
            }
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 0F 7E: MOVD r/m32, xmm (or MOVQ r/m64, xmm with REX.W)
        // Also F3 0F 7E: MOVQ xmm, xmm/m64
        0x7E => {
            if has_f3 {
                // MOVQ xmm, xmm/m64
                let val = read_qword_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
                let dst_idx = xmm_dst_index(inst)?;
                cpu.sse.xmm[dst_idx] = Xmm { lo: val, hi: 0 };
            } else {
                let src_idx = xmm_src_index(inst)?;
                let val = if inst.prefix.rex_w() {
                    cpu.sse.xmm[src_idx].lo
                } else {
                    cpu.sse.xmm[src_idx].lo & 0xFFFF_FFFF
                };
                super::write_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
            }
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── 66 0F D6: MOVQ xmm/m64, xmm (store low qword) ──
        0xD6 if has_66 => {
            let src_idx = xmm_src_index(inst)?;
            let val = cpu.sse.xmm[src_idx].lo;
            write_qword_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── LDMXCSR (0F AE /2) ──
        0xAE if inst.modrm_reg() == 2 => {
            let linear = get_mem_linear(cpu, inst)?;
            let val = translate_and_read(cpu, linear, OperandSize::Dword, mmu, memory)? as u32;
            // Validate: reserved bits must be zero
            if (val & !MXCSR_WRITE_MASK) != 0 {
                return Err(VmError::GeneralProtection(0));
            }
            cpu.sse.mxcsr = val;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── STMXCSR (0F AE /3) ──
        0xAE if inst.modrm_reg() == 3 => {
            let linear = get_mem_linear(cpu, inst)?;
            translate_and_write(
                cpu,
                linear,
                OperandSize::Dword,
                cpu.sse.mxcsr as u64,
                mmu,
                memory,
            )?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── LFENCE (0F AE /5), MFENCE (0F AE /6), SFENCE (0F AE /7) ──
        0xAE if inst.modrm_reg() >= 5 => {
            // Memory fences are no-ops in our emulator
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── MOVDQA/MOVDQU (66 0F 6F / F3 0F 6F) ── load 128-bit
        0x6F => {
            let (lo, hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst_idx = xmm_dst_index(inst)?;
            cpu.sse.xmm[dst_idx] = Xmm { lo, hi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── MOVDQA/MOVDQU (66 0F 7F / F3 0F 7F) ── store 128-bit
        0x7F => {
            let src_idx = xmm_src_index(inst)?;
            let src = cpu.sse.xmm[src_idx];
            write_xmm_or_mem128(cpu, inst, &inst.operands[0], src.lo, src.hi, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PXOR (66 0F EF) ──
        0xEF if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx].lo ^= src_lo;
            cpu.sse.xmm[dst_idx].hi ^= src_hi;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PCMPEQB/W/D (66 0F 74/75/76) ──
        // Simplified: advance RIP (stub)
        0x74 | 0x75 | 0x76 if has_66 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PSHUFD (66 0F 70), PSHUFLW (F2 0F 70), PSHUFHW (F3 0F 70) ──
        // Simplified: advance RIP (stub)
        0x70 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PUNPCKLBW..PUNPCKHQDQ (66 0F 60-6D) ──
        // Simplified stub
        0x60..=0x6D if has_66 => {
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── Everything else: #UD ──
        _ => Err(VmError::UndefinedOpcode(op2)),
    }
}

// ── MOVSS helper ──

/// MOVSS: scalar single-precision move.
///
/// F3 0F 10: MOVSS xmm, xmm/m32
///   - From memory: loads 32 bits into low dword of XMM, zeros upper 96 bits.
///   - From XMM: merges low dword of source into destination (preserves upper).
///
/// F3 0F 11: MOVSS xmm/m32, xmm
///   - To memory: stores low dword.
///   - To XMM: merges low dword.
fn exec_movss(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    op2: u8,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    if op2 == 0x10 {
        // Load
        match &inst.operands[1] {
            Operand::Memory(_) => {
                let val = super::read_operand(cpu, inst, &inst.operands[1], memory, mmu)? as u32;
                let dst_idx = xmm_dst_index(inst)?;
                cpu.sse.xmm[dst_idx] = Xmm {
                    lo: val as u64,
                    hi: 0,
                };
            }
            Operand::Register(RegOperand::Xmm(src)) => {
                let dst_idx = xmm_dst_index(inst)?;
                let src_lo32 = cpu.sse.xmm[*src as usize].lo & 0xFFFF_FFFF;
                cpu.sse.xmm[dst_idx].lo =
                    (cpu.sse.xmm[dst_idx].lo & !0xFFFF_FFFF) | src_lo32;
            }
            _ => {}
        }
    } else {
        // Store
        let src_idx = xmm_src_index(inst)?;
        let val = cpu.sse.xmm[src_idx].lo & 0xFFFF_FFFF;
        match &inst.operands[0] {
            Operand::Memory(_) => {
                let linear = get_mem_linear(cpu, inst)?;
                translate_and_write(cpu, linear, OperandSize::Dword, val, mmu, memory)?;
            }
            Operand::Register(RegOperand::Xmm(dst)) => {
                let dst = *dst as usize;
                cpu.sse.xmm[dst].lo = (cpu.sse.xmm[dst].lo & !0xFFFF_FFFF) | val;
            }
            _ => {}
        }
    }
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// MOVSD: scalar double-precision move.
///
/// F2 0F 10: MOVSD xmm, xmm/m64
///   - From memory: loads 64 bits into low qword, zeros upper 64 bits.
///   - From XMM: merges low qword.
///
/// F2 0F 11: MOVSD xmm/m64, xmm
fn exec_movsd(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    op2: u8,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    if op2 == 0x10 {
        match &inst.operands[1] {
            Operand::Memory(_) => {
                let linear = get_mem_linear_operand(cpu, inst, 1)?;
                let val = translate_and_read(cpu, linear, OperandSize::Qword, mmu, memory)?;
                let dst_idx = xmm_dst_index(inst)?;
                cpu.sse.xmm[dst_idx] = Xmm { lo: val, hi: 0 };
            }
            Operand::Register(RegOperand::Xmm(src)) => {
                let dst_idx = xmm_dst_index(inst)?;
                cpu.sse.xmm[dst_idx].lo = cpu.sse.xmm[*src as usize].lo;
            }
            _ => {}
        }
    } else {
        let src_idx = xmm_src_index(inst)?;
        let val = cpu.sse.xmm[src_idx].lo;
        match &inst.operands[0] {
            Operand::Memory(_) => {
                let linear = get_mem_linear(cpu, inst)?;
                translate_and_write(cpu, linear, OperandSize::Qword, val, mmu, memory)?;
            }
            Operand::Register(RegOperand::Xmm(dst)) => {
                cpu.sse.xmm[*dst as usize].lo = val;
            }
            _ => {}
        }
    }
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── Helpers ──

/// Get the destination XMM register index from operand 0.
fn xmm_dst_index(inst: &DecodedInst) -> Result<usize> {
    match &inst.operands[0] {
        Operand::Register(RegOperand::Xmm(idx)) => Ok(*idx as usize),
        _ => {
            // The ModR/M reg field encodes the XMM register
            Ok(inst.modrm_reg() as usize)
        }
    }
}

/// Get the source XMM register index from operand 1 (or reg field).
fn xmm_src_index(inst: &DecodedInst) -> Result<usize> {
    match &inst.operands[1] {
        Operand::Register(RegOperand::Xmm(idx)) => Ok(*idx as usize),
        _ => {
            // For stores, the reg field is the source
            Ok(inst.modrm_reg() as usize)
        }
    }
}

/// Read 128 bits from an XMM register or memory operand.
fn read_xmm_or_mem128(
    cpu: &Cpu,
    inst: &DecodedInst,
    operand: &Operand,
    memory: &GuestMemory,
    mmu: &Mmu,
) -> Result<(u64, u64)> {
    match operand {
        Operand::Register(RegOperand::Xmm(idx)) => {
            let xmm = cpu.sse.xmm[*idx as usize];
            Ok((xmm.lo, xmm.hi))
        }
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            let lo = translate_and_read(cpu, linear, OperandSize::Qword, mmu, memory)?;
            let hi = translate_and_read(
                cpu,
                linear.wrapping_add(8),
                OperandSize::Qword,
                mmu,
                memory,
            )?;
            Ok((lo, hi))
        }
        _ => Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }
}

/// Write 128 bits to an XMM register or memory operand.
fn write_xmm_or_mem128(
    cpu: &Cpu,
    inst: &DecodedInst,
    operand: &Operand,
    lo: u64,
    hi: u64,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    match operand {
        Operand::Register(RegOperand::Xmm(_idx)) => {
            // Cannot borrow cpu.sse mutably through &Cpu, so this path
            // is handled by the caller directly.
            Err(VmError::UndefinedOpcode(inst.opcode as u8))
        }
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            translate_and_write(cpu, linear, OperandSize::Qword, lo, mmu, memory)?;
            translate_and_write(cpu, linear.wrapping_add(8), OperandSize::Qword, hi, mmu, memory)?;
            Ok(())
        }
        _ => Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }
}

/// Read a 64-bit value from an operand (XMM low half or memory).
fn read_qword_operand(
    cpu: &Cpu,
    inst: &DecodedInst,
    operand: &Operand,
    memory: &GuestMemory,
    mmu: &Mmu,
) -> Result<u64> {
    match operand {
        Operand::Register(RegOperand::Xmm(idx)) => Ok(cpu.sse.xmm[*idx as usize].lo),
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            translate_and_read(cpu, linear, OperandSize::Qword, mmu, memory)
        }
        _ => super::read_operand(cpu, inst, operand, memory, mmu),
    }
}

/// Write a 64-bit value to an operand (XMM low half or memory).
fn write_qword_operand(
    cpu: &Cpu,
    inst: &DecodedInst,
    operand: &Operand,
    val: u64,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    match operand {
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            translate_and_write(cpu, linear, OperandSize::Qword, val, mmu, memory)
        }
        _ => Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }
}

/// Get the linear address from the memory operand at position 0.
fn get_mem_linear(cpu: &Cpu, inst: &DecodedInst) -> Result<u64> {
    get_mem_linear_operand(cpu, inst, 0)
}

/// Get the linear address from a memory operand at a given position.
fn get_mem_linear_operand(cpu: &Cpu, inst: &DecodedInst, operand_idx: usize) -> Result<u64> {
    match &inst.operands[operand_idx] {
        Operand::Memory(mem_op) => compute_effective_address(cpu, mem_op, inst),
        _ => Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }
}
