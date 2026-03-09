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
use crate::instruction::{DecodedInst, OpcodeMap, Operand, RegOperand};
use crate::memory::{GuestMemory, Mmu};
use crate::sse_state::{Xmm, MXCSR_WRITE_MASK};

use super::{compute_effective_address, read_operand, translate_and_read, translate_and_write, write_operand};

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
        // ── MOVSS (F3 0F 10/11) / MOVSD (F2 0F 10/11) ──
        0x10 | 0x11 if has_f3 || has_f2 => {
            if has_f3 {
                exec_movss(cpu, inst, op2, memory, mmu)
            } else {
                exec_movsd(cpu, inst, op2, memory, mmu)
            }
        }

        // ── MOVUPS/MOVAPS/MOVUPD/MOVAPD xmm, xmm/m128 (load) ──
        0x10 | 0x28 => {
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst_idx = xmm_dst_index(inst)?;
            cpu.sse.xmm[dst_idx] = Xmm { lo: src_lo, hi: src_hi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── MOVUPS/MOVAPS/MOVUPD/MOVAPD/MOVNTPS/MOVNTPD xmm/m128, xmm (store) ──
        0x11 | 0x29 | 0x2B => {
            let src_idx = xmm_src_index(inst)?;
            let src = cpu.sse.xmm[src_idx];
            write_xmm_or_mem128(cpu, inst, &inst.operands[0], src.lo, src.hi, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
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

        // ── FXSAVE (0F AE /0) ──
        0xAE if inst.modrm_reg() == 0 => exec_fxsave(cpu, inst, memory, mmu),

        // ── FXRSTOR (0F AE /1) ──
        0xAE if inst.modrm_reg() == 1 => exec_fxrstor(cpu, inst, memory, mmu),

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

        // ── PCMPEQB (66 0F 74) ──
        0x74 if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let d = cpu.sse.xmm[dst_idx];
            let mut rlo = 0u64;
            let mut rhi = 0u64;
            for i in 0..8 {
                let db = ((d.lo >> (i * 8)) & 0xFF) as u8;
                let sb = ((src_lo >> (i * 8)) & 0xFF) as u8;
                if db == sb { rlo |= 0xFFu64 << (i * 8); }
            }
            for i in 0..8 {
                let db = ((d.hi >> (i * 8)) & 0xFF) as u8;
                let sb = ((src_hi >> (i * 8)) & 0xFF) as u8;
                if db == sb { rhi |= 0xFFu64 << (i * 8); }
            }
            cpu.sse.xmm[dst_idx] = Xmm { lo: rlo, hi: rhi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // ── PCMPEQW (66 0F 75) ──
        0x75 if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let d = cpu.sse.xmm[dst_idx];
            let mut rlo = 0u64;
            let mut rhi = 0u64;
            for i in 0..4 {
                let dw = ((d.lo >> (i * 16)) & 0xFFFF) as u16;
                let sw = ((src_lo >> (i * 16)) & 0xFFFF) as u16;
                if dw == sw { rlo |= 0xFFFFu64 << (i * 16); }
            }
            for i in 0..4 {
                let dw = ((d.hi >> (i * 16)) & 0xFFFF) as u16;
                let sw = ((src_hi >> (i * 16)) & 0xFFFF) as u16;
                if dw == sw { rhi |= 0xFFFFu64 << (i * 16); }
            }
            cpu.sse.xmm[dst_idx] = Xmm { lo: rlo, hi: rhi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // ── PCMPEQD (66 0F 76) ──
        0x76 if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let d = cpu.sse.xmm[dst_idx];
            let mut rlo = 0u64;
            let mut rhi = 0u64;
            for i in 0..2 {
                let dd = ((d.lo >> (i * 32)) & 0xFFFF_FFFF) as u32;
                let sd = ((src_lo >> (i * 32)) & 0xFFFF_FFFF) as u32;
                if dd == sd { rlo |= 0xFFFF_FFFFu64 << (i * 32); }
            }
            for i in 0..2 {
                let dd = ((d.hi >> (i * 32)) & 0xFFFF_FFFF) as u32;
                let sd = ((src_hi >> (i * 32)) & 0xFFFF_FFFF) as u32;
                if dd == sd { rhi |= 0xFFFF_FFFFu64 << (i * 32); }
            }
            cpu.sse.xmm[dst_idx] = Xmm { lo: rlo, hi: rhi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PSHUFD (66 0F 70), PSHUFLW (F2 0F 70), PSHUFHW (F3 0F 70) ──
        0x70 => {
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst_idx = xmm_dst_index(inst)?;
            // imm8 is in operands[2]
            let imm = match &inst.operands[2] {
                Operand::Immediate(v) => *v as u8,
                _ => 0,
            };
            if has_66 {
                // PSHUFD: shuffle 32-bit dwords
                let dwords = [
                    (src_lo & 0xFFFF_FFFF) as u32,
                    ((src_lo >> 32) & 0xFFFF_FFFF) as u32,
                    (src_hi & 0xFFFF_FFFF) as u32,
                    ((src_hi >> 32) & 0xFFFF_FFFF) as u32,
                ];
                let r0 = dwords[(imm & 3) as usize] as u64;
                let r1 = dwords[((imm >> 2) & 3) as usize] as u64;
                let r2 = dwords[((imm >> 4) & 3) as usize] as u64;
                let r3 = dwords[((imm >> 6) & 3) as usize] as u64;
                cpu.sse.xmm[dst_idx] = Xmm { lo: r0 | (r1 << 32), hi: r2 | (r3 << 32) };
            } else if has_f2 {
                // PSHUFLW: shuffle low 4 words, high 64 bits unchanged
                let words = [
                    (src_lo & 0xFFFF) as u16,
                    ((src_lo >> 16) & 0xFFFF) as u16,
                    ((src_lo >> 32) & 0xFFFF) as u16,
                    ((src_lo >> 48) & 0xFFFF) as u16,
                ];
                let r0 = words[(imm & 3) as usize] as u64;
                let r1 = words[((imm >> 2) & 3) as usize] as u64;
                let r2 = words[((imm >> 4) & 3) as usize] as u64;
                let r3 = words[((imm >> 6) & 3) as usize] as u64;
                cpu.sse.xmm[dst_idx] = Xmm { lo: r0 | (r1 << 16) | (r2 << 32) | (r3 << 48), hi: src_hi };
            } else if has_f3 {
                // PSHUFHW: shuffle high 4 words, low 64 bits unchanged
                let words = [
                    (src_hi & 0xFFFF) as u16,
                    ((src_hi >> 16) & 0xFFFF) as u16,
                    ((src_hi >> 32) & 0xFFFF) as u16,
                    ((src_hi >> 48) & 0xFFFF) as u16,
                ];
                let r0 = words[(imm & 3) as usize] as u64;
                let r1 = words[((imm >> 2) & 3) as usize] as u64;
                let r2 = words[((imm >> 4) & 3) as usize] as u64;
                let r3 = words[((imm >> 6) & 3) as usize] as u64;
                cpu.sse.xmm[dst_idx] = Xmm { lo: src_lo, hi: r0 | (r1 << 16) | (r2 << 32) | (r3 << 48) };
            } else {
                // PSHUFW (MMX, no prefix) — treat as no-op for now
            }
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PUNPCKLBW..PUNPCKHQDQ (66 0F 60-6D) ──
        0x60..=0x6D if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let d = cpu.sse.xmm[dst_idx];
            let result = match op2 {
                0x60 => { // PUNPCKLBW: interleave low bytes
                    let mut lo = 0u64; let mut hi = 0u64;
                    for i in 0..8u32 {
                        let db = (d.lo >> (i * 8)) & 0xFF;
                        let sb = (src_lo >> (i * 8)) & 0xFF;
                        let pos = i * 16;
                        if pos < 64 { lo |= (db | (sb << 8)) << pos; }
                        else { hi |= (db | (sb << 8)) << (pos - 64); }
                    }
                    Xmm { lo, hi }
                }
                0x61 => { // PUNPCKLWD: interleave low words
                    let mut lo = 0u64; let mut hi = 0u64;
                    for i in 0..4u32 {
                        let dw = (d.lo >> (i * 16)) & 0xFFFF;
                        let sw = (src_lo >> (i * 16)) & 0xFFFF;
                        let pos = i * 32;
                        if pos < 64 { lo |= (dw | (sw << 16)) << pos; }
                        else { hi |= (dw | (sw << 16)) << (pos - 64); }
                    }
                    Xmm { lo, hi }
                }
                0x62 => { // PUNPCKLDQ: interleave low dwords
                    let d0 = d.lo & 0xFFFF_FFFF;
                    let d1 = (d.lo >> 32) & 0xFFFF_FFFF;
                    let s0 = src_lo & 0xFFFF_FFFF;
                    let s1 = (src_lo >> 32) & 0xFFFF_FFFF;
                    Xmm { lo: d0 | (s0 << 32), hi: d1 | (s1 << 32) }
                }
                0x63 => { // PACKSSWB
                    // Simplified: just advance RIP for now
                    cpu.sse.xmm[dst_idx] // no change
                }
                0x64 => { // PCMPGTB: packed compare greater than bytes
                    let mut rlo = 0u64; let mut rhi = 0u64;
                    for i in 0..8 {
                        let db = ((d.lo >> (i * 8)) & 0xFF) as i8;
                        let sb = ((src_lo >> (i * 8)) & 0xFF) as i8;
                        if db > sb { rlo |= 0xFFu64 << (i * 8); }
                    }
                    for i in 0..8 {
                        let db = ((d.hi >> (i * 8)) & 0xFF) as i8;
                        let sb = ((src_hi >> (i * 8)) & 0xFF) as i8;
                        if db > sb { rhi |= 0xFFu64 << (i * 8); }
                    }
                    Xmm { lo: rlo, hi: rhi }
                }
                0x65 => { // PCMPGTW: packed compare greater than words
                    let mut rlo = 0u64; let mut rhi = 0u64;
                    for i in 0..4 {
                        let dw = ((d.lo >> (i * 16)) & 0xFFFF) as i16;
                        let sw = ((src_lo >> (i * 16)) & 0xFFFF) as i16;
                        if dw > sw { rlo |= 0xFFFFu64 << (i * 16); }
                    }
                    for i in 0..4 {
                        let dw = ((d.hi >> (i * 16)) & 0xFFFF) as i16;
                        let sw = ((src_hi >> (i * 16)) & 0xFFFF) as i16;
                        if dw > sw { rhi |= 0xFFFFu64 << (i * 16); }
                    }
                    Xmm { lo: rlo, hi: rhi }
                }
                0x66 => { // PCMPGTD: packed compare greater than dwords
                    let mut rlo = 0u64; let mut rhi = 0u64;
                    for i in 0..2 {
                        let dd = ((d.lo >> (i * 32)) & 0xFFFF_FFFF) as i32;
                        let sd = ((src_lo >> (i * 32)) & 0xFFFF_FFFF) as i32;
                        if dd > sd { rlo |= 0xFFFF_FFFFu64 << (i * 32); }
                    }
                    for i in 0..2 {
                        let dd = ((d.hi >> (i * 32)) & 0xFFFF_FFFF) as i32;
                        let sd = ((src_hi >> (i * 32)) & 0xFFFF_FFFF) as i32;
                        if dd > sd { rhi |= 0xFFFF_FFFFu64 << (i * 32); }
                    }
                    Xmm { lo: rlo, hi: rhi }
                }
                0x67 => { // PACKUSWB
                    cpu.sse.xmm[dst_idx] // stub
                }
                0x68 => { // PUNPCKHBW: interleave high bytes
                    let mut lo = 0u64; let mut hi = 0u64;
                    for i in 0..8u32 {
                        let db = (d.hi >> (i * 8)) & 0xFF;
                        let sb = (src_hi >> (i * 8)) & 0xFF;
                        let pos = i * 16;
                        if pos < 64 { lo |= (db | (sb << 8)) << pos; }
                        else { hi |= (db | (sb << 8)) << (pos - 64); }
                    }
                    Xmm { lo, hi }
                }
                0x69 => { // PUNPCKHWD: interleave high words
                    let mut lo = 0u64; let mut hi = 0u64;
                    for i in 0..4u32 {
                        let dw = (d.hi >> (i * 16)) & 0xFFFF;
                        let sw = (src_hi >> (i * 16)) & 0xFFFF;
                        let pos = i * 32;
                        if pos < 64 { lo |= (dw | (sw << 16)) << pos; }
                        else { hi |= (dw | (sw << 16)) << (pos - 64); }
                    }
                    Xmm { lo, hi }
                }
                0x6A => { // PUNPCKHDQ: interleave high dwords
                    let d0 = d.hi & 0xFFFF_FFFF;
                    let d1 = (d.hi >> 32) & 0xFFFF_FFFF;
                    let s0 = src_hi & 0xFFFF_FFFF;
                    let s1 = (src_hi >> 32) & 0xFFFF_FFFF;
                    Xmm { lo: d0 | (s0 << 32), hi: d1 | (s1 << 32) }
                }
                0x6B => { // PACKSSDW
                    cpu.sse.xmm[dst_idx] // stub
                }
                0x6C => { // PUNPCKLQDQ: interleave low qwords
                    Xmm { lo: d.lo, hi: src_lo }
                }
                0x6D => { // PUNPCKHQDQ: interleave high qwords
                    Xmm { lo: d.hi, hi: src_hi }
                }
                _ => unreachable!(),
            };
            cpu.sse.xmm[dst_idx] = result;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PEXTRW (66 0F C5) ── extract word from XMM to GPR
        0xC5 if has_66 => {
            let src_idx = match &inst.operands[1] {
                Operand::Register(RegOperand::Xmm(idx)) => *idx as usize,
                _ => return Err(VmError::UndefinedOpcode(op2)),
            };
            let x = cpu.sse.xmm[src_idx];
            let sel = (inst.immediate & 7) as u32;
            // XMM is 128 bits stored as lo:hi (each u64)
            let word = if sel < 4 {
                ((x.lo >> (sel * 16)) & 0xFFFF) as u32
            } else {
                ((x.hi >> ((sel - 4) * 16)) & 0xFFFF) as u32
            };
            write_operand(cpu, inst, &inst.operands[0], word as u64, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PMOVMSKB (66 0F D7) ── extract byte sign bits to GPR
        0xD7 if has_66 => {
            let src_idx = match &inst.operands[1] {
                Operand::Register(RegOperand::Xmm(idx)) => *idx as usize,
                _ => return Err(VmError::UndefinedOpcode(op2)),
            };
            let x = cpu.sse.xmm[src_idx];
            let mut mask = 0u32;
            for i in 0..8 { mask |= (((x.lo >> (i * 8 + 7)) & 1) as u32) << i; }
            for i in 0..8 { mask |= (((x.hi >> (i * 8 + 7)) & 1) as u32) << (i + 8); }
            write_operand(cpu, inst, &inst.operands[0], mask as u64, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── MOVMSKPS (0F 50) ── extract float sign bits to GPR
        0x50 if !has_66 && !has_f2 && !has_f3 => {
            let src_idx = match &inst.operands[1] {
                Operand::Register(RegOperand::Xmm(idx)) => *idx as usize,
                _ => return Err(VmError::UndefinedOpcode(op2)),
            };
            let x = cpu.sse.xmm[src_idx];
            let mask = (((x.lo >> 31) & 1) | (((x.lo >> 63) & 1) << 1)
                | (((x.hi >> 31) & 1) << 2) | (((x.hi >> 63) & 1) << 3)) as u32;
            write_operand(cpu, inst, &inst.operands[0], mask as u64, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── MOVMSKPD (66 0F 50) ── extract double sign bits to GPR
        0x50 if has_66 => {
            let src_idx = match &inst.operands[1] {
                Operand::Register(RegOperand::Xmm(idx)) => *idx as usize,
                _ => return Err(VmError::UndefinedOpcode(op2)),
            };
            let x = cpu.sse.xmm[src_idx];
            let mask = (((x.lo >> 63) & 1) | (((x.hi >> 63) & 1) << 1)) as u32;
            write_operand(cpu, inst, &inst.operands[0], mask as u64, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── POR (66 0F EB) ── packed bitwise OR
        0xEB if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx].lo |= src_lo;
            cpu.sse.xmm[dst_idx].hi |= src_hi;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PAND (66 0F DB) ── packed bitwise AND
        0xDB if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx].lo &= src_lo;
            cpu.sse.xmm[dst_idx].hi &= src_hi;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PANDN (66 0F DF) ── packed bitwise AND-NOT
        0xDF if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx].lo = !cpu.sse.xmm[dst_idx].lo & src_lo;
            cpu.sse.xmm[dst_idx].hi = !cpu.sse.xmm[dst_idx].hi & src_hi;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── PSRLDQ (66 0F 73 /3 imm8) ── byte shift right
        // ── PSLLDQ (66 0F 73 /7 imm8) ── byte shift left
        0x73 if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let imm = match &inst.operands[1] {
                Operand::Immediate(v) => (*v as u8).min(16),
                _ => 0,
            };
            let x = cpu.sse.xmm[dst_idx];
            let mut bytes = [0u8; 16];
            bytes[..8].copy_from_slice(&x.lo.to_le_bytes());
            bytes[8..].copy_from_slice(&x.hi.to_le_bytes());
            let modrm_reg = (inst.modrm.unwrap_or(0) >> 3) & 7;
            let mut result = [0u8; 16];
            if modrm_reg == 3 { // PSRLDQ
                let shift = imm as usize;
                for i in 0..16 {
                    result[i] = if i + shift < 16 { bytes[i + shift] } else { 0 };
                }
            } else if modrm_reg == 7 { // PSLLDQ
                let shift = imm as usize;
                for i in 0..16 {
                    result[i] = if i >= shift { bytes[i - shift] } else { 0 };
                }
            }
            cpu.sse.xmm[dst_idx].lo = u64::from_le_bytes(result[..8].try_into().unwrap());
            cpu.sse.xmm[dst_idx].hi = u64::from_le_bytes(result[8..].try_into().unwrap());
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── Group 12: PSRLW/PSRAW/PSLLW (66 0F 71 /reg imm8) ──
        0x71 if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let imm = match &inst.operands[1] { Operand::Immediate(v) => *v as u16, _ => 0 };
            let modrm_reg = (inst.modrm.unwrap_or(0) >> 3) & 7;
            let d = &mut cpu.sse.xmm[dst_idx];
            let mut words = [0u16; 8];
            for i in 0..4 { words[i] = ((d.lo >> (i * 16)) & 0xFFFF) as u16; }
            for i in 0..4 { words[4 + i] = ((d.hi >> (i * 16)) & 0xFFFF) as u16; }
            match modrm_reg {
                2 => { for w in &mut words { *w = if imm > 15 { 0 } else { *w >> imm }; } } // PSRLW
                4 => { for w in &mut words { *w = if imm > 15 { ((*w as i16) >> 15) as u16 } else { ((*w as i16) >> imm) as u16 }; } } // PSRAW
                6 => { for w in &mut words { *w = if imm > 15 { 0 } else { *w << imm }; } } // PSLLW
                _ => return Err(VmError::UndefinedOpcode(op2)),
            }
            d.lo = 0; d.hi = 0;
            for i in 0..4 { d.lo |= (words[i] as u64) << (i * 16); }
            for i in 0..4 { d.hi |= (words[4 + i] as u64) << (i * 16); }
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── Group 13: PSRLD/PSRAD/PSLLD (66 0F 72 /reg imm8) ──
        0x72 if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let imm = match &inst.operands[1] { Operand::Immediate(v) => *v as u32, _ => 0 };
            let modrm_reg = (inst.modrm.unwrap_or(0) >> 3) & 7;
            let d = &mut cpu.sse.xmm[dst_idx];
            let mut dwords = [0u32; 4];
            dwords[0] = d.lo as u32; dwords[1] = (d.lo >> 32) as u32;
            dwords[2] = d.hi as u32; dwords[3] = (d.hi >> 32) as u32;
            match modrm_reg {
                2 => { for dw in &mut dwords { *dw = if imm > 31 { 0 } else { *dw >> imm }; } } // PSRLD
                4 => { for dw in &mut dwords { *dw = if imm > 31 { ((*dw as i32) >> 31) as u32 } else { ((*dw as i32) >> imm) as u32 }; } } // PSRAD
                6 => { for dw in &mut dwords { *dw = if imm > 31 { 0 } else { *dw << imm }; } } // PSLLD
                _ => return Err(VmError::UndefinedOpcode(op2)),
            }
            d.lo = dwords[0] as u64 | ((dwords[1] as u64) << 32);
            d.hi = dwords[2] as u64 | ((dwords[3] as u64) << 32);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── SSE2 packed integer arithmetic ──
        // PADDD (66 0F FE), PSUBD (66 0F FA), PADDQ (66 0F D4), PSUBQ (66 0F FB)
        // PADDB (66 0F FC), PADDW (66 0F FD), PSUBB (66 0F F8), PSUBW (66 0F F9)
        // PMULLW (66 0F D5), PMULUDQ (66 0F F4), PMADDWD (66 0F F5)
        // PAND (66 0F DB), PANDN (66 0F DF), POR (66 0F EB)
        // PSLLW (66 0F F1), PSLLD (66 0F F2), PSLLQ (66 0F F3)
        // PSRLW (66 0F D1), PSRLD (66 0F D2), PSRLQ (66 0F D3)
        // PSRAW (66 0F E1), PSRAD (66 0F E2)
        // PAVGB (66 0F E0), PAVGW (66 0F E3)
        // PMINUB (66 0F DA), PMAXUB (66 0F DE)
        // PMULHUW (66 0F E4), PMULHW (66 0F E5)
        // PSADBW (66 0F F6)
        // PADDUSB (66 0F DC), PADDUSW (66 0F DD)
        // PSUBUSB (66 0F D8), PSUBUSW (66 0F D9)
        // PADDSB (66 0F EC), PADDSW (66 0F ED)
        // PSUBSB (66 0F E8), PSUBSW (66 0F E9)
        // PMINSW (66 0F EA), PMAXSW (66 0F EE)
        0xD4 | 0xD5 | 0xD8 | 0xD9 | 0xDA | 0xDB | 0xDC | 0xDD | 0xDE | 0xDF
        | 0xE0 | 0xE1 | 0xE2 | 0xE3 | 0xE4 | 0xE5
        | 0xE8 | 0xE9 | 0xEA | 0xEB | 0xEC | 0xED | 0xEE
        | 0xF1 | 0xF2 | 0xF3 | 0xF4 | 0xF5 | 0xF6
        | 0xF8 | 0xF9 | 0xFA | 0xFB | 0xFC | 0xFD | 0xFE
        if has_66 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let d = &mut cpu.sse.xmm[dst_idx];

            macro_rules! op_bytes {
                ($op:expr) => {{
                    let mut dl = d.lo.to_le_bytes();
                    let mut dh = d.hi.to_le_bytes();
                    let sl = src_lo.to_le_bytes();
                    let sh = src_hi.to_le_bytes();
                    for i in 0..8 { dl[i] = $op(dl[i], sl[i]); }
                    for i in 0..8 { dh[i] = $op(dh[i], sh[i]); }
                    d.lo = u64::from_le_bytes(dl);
                    d.hi = u64::from_le_bytes(dh);
                }};
            }
            macro_rules! op_words {
                ($op:expr) => {{
                    let mut dw = [0u16; 8];
                    let mut sw = [0u16; 8];
                    for i in 0..4 { dw[i] = ((d.lo >> (i*16)) & 0xFFFF) as u16; sw[i] = ((src_lo >> (i*16)) & 0xFFFF) as u16; }
                    for i in 0..4 { dw[4+i] = ((d.hi >> (i*16)) & 0xFFFF) as u16; sw[4+i] = ((src_hi >> (i*16)) & 0xFFFF) as u16; }
                    for i in 0..8 { dw[i] = $op(dw[i], sw[i]); }
                    d.lo = 0; d.hi = 0;
                    for i in 0..4 { d.lo |= (dw[i] as u64) << (i*16); }
                    for i in 0..4 { d.hi |= (dw[4+i] as u64) << (i*16); }
                }};
            }
            macro_rules! op_dwords {
                ($op:expr) => {{
                    let mut dd = [d.lo as u32, (d.lo >> 32) as u32, d.hi as u32, (d.hi >> 32) as u32];
                    let sd = [src_lo as u32, (src_lo >> 32) as u32, src_hi as u32, (src_hi >> 32) as u32];
                    for i in 0..4 { dd[i] = $op(dd[i], sd[i]); }
                    d.lo = dd[0] as u64 | ((dd[1] as u64) << 32);
                    d.hi = dd[2] as u64 | ((dd[3] as u64) << 32);
                }};
            }

            match op2 {
                // Byte ops
                0xFC => op_bytes!(|a: u8, b: u8| a.wrapping_add(b)),   // PADDB
                0xF8 => op_bytes!(|a: u8, b: u8| a.wrapping_sub(b)),   // PSUBB
                0xDA => op_bytes!(|a: u8, b: u8| a.min(b)),            // PMINUB
                0xDE => op_bytes!(|a: u8, b: u8| a.max(b)),            // PMAXUB
                0xE0 => op_bytes!(|a: u8, b: u8| ((a as u16 + b as u16 + 1) / 2) as u8), // PAVGB
                0xDC => op_bytes!(|a: u8, b: u8| (a as u16 + b as u16).min(255) as u8), // PADDUSB
                0xD8 => op_bytes!(|a: u8, b: u8| (a as i16 - b as i16).max(0) as u8),   // PSUBUSB
                0xEC => op_bytes!(|a: u8, b: u8| ((a as i8 as i16 + b as i8 as i16).clamp(-128, 127)) as u8), // PADDSB
                0xE8 => op_bytes!(|a: u8, b: u8| ((a as i8 as i16 - b as i8 as i16).clamp(-128, 127)) as u8), // PSUBSB

                // Word ops
                0xFD => op_words!(|a: u16, b: u16| a.wrapping_add(b)), // PADDW
                0xF9 => op_words!(|a: u16, b: u16| a.wrapping_sub(b)), // PSUBW
                0xD5 => op_words!(|a: u16, b: u16| (a as i16).wrapping_mul(b as i16) as u16), // PMULLW
                0xE4 => op_words!(|a: u16, b: u16| ((a as u32 * b as u32) >> 16) as u16), // PMULHUW
                0xE5 => op_words!(|a: u16, b: u16| (((a as i16 as i32) * (b as i16 as i32)) >> 16) as u16), // PMULHW
                0xE3 => op_words!(|a: u16, b: u16| ((a as u32 + b as u32 + 1) / 2) as u16), // PAVGW
                0xEA => op_words!(|a: u16, b: u16| (a as i16).min(b as i16) as u16), // PMINSW
                0xEE => op_words!(|a: u16, b: u16| (a as i16).max(b as i16) as u16), // PMAXSW
                0xDD => op_words!(|a: u16, b: u16| (a as u32 + b as u32).min(0xFFFF) as u16), // PADDUSW
                0xD9 => op_words!(|a: u16, b: u16| (a as i32 - b as i32).max(0) as u16), // PSUBUSW
                0xED => op_words!(|a: u16, b: u16| ((a as i16 as i32 + b as i16 as i32).clamp(-32768, 32767)) as u16), // PADDSW
                0xE9 => op_words!(|a: u16, b: u16| ((a as i16 as i32 - b as i16 as i32).clamp(-32768, 32767)) as u16), // PSUBSW

                // Dword ops
                0xFE => op_dwords!(|a: u32, b: u32| a.wrapping_add(b)), // PADDD
                0xFA => op_dwords!(|a: u32, b: u32| a.wrapping_sub(b)), // PSUBD

                // Qword ops
                0xD4 => { d.lo = d.lo.wrapping_add(src_lo); d.hi = d.hi.wrapping_add(src_hi); } // PADDQ
                0xFB => { d.lo = d.lo.wrapping_sub(src_lo); d.hi = d.hi.wrapping_sub(src_hi); } // PSUBQ

                // Bitwise
                0xDB => { d.lo &= src_lo; d.hi &= src_hi; } // PAND
                0xDF => { d.lo = !d.lo & src_lo; d.hi = !d.hi & src_hi; } // PANDN
                0xEB => { d.lo |= src_lo; d.hi |= src_hi; } // POR

                // Shifts by xmm count (low 64 bits = count)
                0xF1 => { // PSLLW by xmm
                    let cnt = src_lo as u32;
                    op_words!(|a: u16, _b: u16| if cnt > 15 { 0 } else { a << cnt });
                }
                0xF2 => { // PSLLD by xmm
                    let cnt = src_lo as u32;
                    op_dwords!(|a: u32, _b: u32| if cnt > 31 { 0 } else { a << cnt });
                }
                0xF3 => { // PSLLQ by xmm
                    let cnt = src_lo;
                    if cnt > 63 { d.lo = 0; d.hi = 0; } else { d.lo <<= cnt; d.hi <<= cnt; }
                }
                0xE1 => { // PSRAW by xmm
                    let cnt = src_lo as u32;
                    op_words!(|a: u16, _b: u16| if cnt > 15 { ((a as i16) >> 15) as u16 } else { ((a as i16) >> cnt) as u16 });
                }
                0xE2 => { // PSRAD by xmm
                    let cnt = src_lo as u32;
                    op_dwords!(|a: u32, _b: u32| if cnt > 31 { ((a as i32) >> 31) as u32 } else { ((a as i32) >> cnt) as u32 });
                }

                // PMULUDQ: multiply low dwords of each qword pair
                0xF4 => {
                    let a0 = d.lo as u32 as u64; let b0 = src_lo as u32 as u64;
                    let a1 = d.hi as u32 as u64; let b1 = src_hi as u32 as u64;
                    d.lo = a0.wrapping_mul(b0);
                    d.hi = a1.wrapping_mul(b1);
                }
                // PMADDWD: multiply words and add pairs
                0xF5 => {
                    let mut dw = [0i32; 4];
                    for i in 0..4 {
                        let qw = if i < 2 { d.lo } else { d.hi }; let sw = if i < 2 { src_lo } else { src_hi };
                        let j = (i % 2) * 2;
                        let a0 = ((qw >> (j * 16)) & 0xFFFF) as i16 as i32;
                        let a1 = ((qw >> ((j + 1) * 16)) & 0xFFFF) as i16 as i32;
                        let b0 = ((sw >> (j * 16)) & 0xFFFF) as i16 as i32;
                        let b1 = ((sw >> ((j + 1) * 16)) & 0xFFFF) as i16 as i32;
                        dw[i] = a0.wrapping_mul(b0).wrapping_add(a1.wrapping_mul(b1));
                    }
                    d.lo = dw[0] as u32 as u64 | ((dw[1] as u32 as u64) << 32);
                    d.hi = dw[2] as u32 as u64 | ((dw[3] as u32 as u64) << 32);
                }
                // PSADBW: sum of absolute differences of bytes
                0xF6 => {
                    let dl = d.lo.to_le_bytes(); let dh = d.hi.to_le_bytes();
                    let sl = src_lo.to_le_bytes(); let sh = src_hi.to_le_bytes();
                    let mut sum0: u64 = 0; let mut sum1: u64 = 0;
                    for i in 0..8 { sum0 += (dl[i] as i16 - sl[i] as i16).unsigned_abs() as u64; }
                    for i in 0..8 { sum1 += (dh[i] as i16 - sh[i] as i16).unsigned_abs() as u64; }
                    d.lo = sum0; d.hi = sum1;
                }
                _ => return Err(VmError::UndefinedOpcode(op2)),
            }
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── Everything else: #UD ──
        _ => Err(VmError::UndefinedOpcode(op2)),
    }
}

/// Dispatch minimal 0F 38 / 0F 3A instruction subset.
pub fn exec_sse_escape(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    match inst.opcode_map {
        OpcodeMap::Escape0F38 => exec_sse_0f38(cpu, inst, memory, mmu),
        OpcodeMap::Escape0F3A => exec_sse_0f3a(cpu, inst, memory, mmu),
        _ => Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }
}

fn exec_sse_0f38(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let op = inst.opcode as u8;
    match op {
        // 66 0F 38 00 /r -- PSHUFB xmm, xmm/m128
        0x00 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let mut dst = cpu.sse.xmm[dst_idx];
            pshufb_lane(&mut dst.lo, src_lo);
            pshufb_lane(&mut dst.hi, src_hi);
            cpu.sse.xmm[dst_idx] = dst;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 38 17 /r -- PTEST xmm, xmm/m128
        0x17 => {
            let dst_idx = xmm_dst_index(inst)?;
            let dst = cpu.sse.xmm[dst_idx];
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let and_lo = dst.lo & src_lo;
            let and_hi = dst.hi & src_hi;
            let andn_lo = (!dst.lo) & src_lo;
            let andn_hi = (!dst.hi) & src_hi;
            let zf = (and_lo | and_hi) == 0;
            let cf = (andn_lo | andn_hi) == 0;
            cpu.regs.rflags &= !(crate::flags::CF | crate::flags::ZF | crate::flags::AF
                | crate::flags::PF | crate::flags::SF | crate::flags::OF);
            if zf { cpu.regs.rflags |= crate::flags::ZF; }
            if cf { cpu.regs.rflags |= crate::flags::CF; }
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 38 29 /r -- PCMPEQQ xmm, xmm/m128
        0x29 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst = cpu.sse.xmm[dst_idx];
            let lo = if dst.lo == src_lo { u64::MAX } else { 0 };
            let hi = if dst.hi == src_hi { u64::MAX } else { 0 };
            cpu.sse.xmm[dst_idx] = Xmm { lo, hi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 38 39 /r -- PMINSD xmm, xmm/m128
        0x39 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst = cpu.sse.xmm[dst_idx];
            let d = dst.to_u32x4();
            let s = Xmm { lo: src_lo, hi: src_hi }.to_u32x4();
            let mut r = [0u32; 4];
            for i in 0..4 {
                r[i] = if (d[i] as i32) < (s[i] as i32) { d[i] } else { s[i] };
            }
            cpu.sse.xmm[dst_idx] = Xmm::from_u32x4(r);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 38 3B /r -- PMINUD xmm, xmm/m128
        0x3B => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst = cpu.sse.xmm[dst_idx];
            let d = dst.to_u32x4();
            let s = Xmm { lo: src_lo, hi: src_hi }.to_u32x4();
            let mut r = [0u32; 4];
            for i in 0..4 { r[i] = d[i].min(s[i]); }
            cpu.sse.xmm[dst_idx] = Xmm::from_u32x4(r);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 38 3D /r -- PMAXSD xmm, xmm/m128
        0x3D => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst = cpu.sse.xmm[dst_idx];
            let d = dst.to_u32x4();
            let s = Xmm { lo: src_lo, hi: src_hi }.to_u32x4();
            let mut r = [0u32; 4];
            for i in 0..4 {
                r[i] = if (d[i] as i32) > (s[i] as i32) { d[i] } else { s[i] };
            }
            cpu.sse.xmm[dst_idx] = Xmm::from_u32x4(r);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 38 3F /r -- PMAXUD xmm, xmm/m128
        0x3F => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst = cpu.sse.xmm[dst_idx];
            let d = dst.to_u32x4();
            let s = Xmm { lo: src_lo, hi: src_hi }.to_u32x4();
            let mut r = [0u32; 4];
            for i in 0..4 { r[i] = d[i].max(s[i]); }
            cpu.sse.xmm[dst_idx] = Xmm::from_u32x4(r);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 38 40 /r -- PMULLD xmm, xmm/m128
        0x40 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let dst = cpu.sse.xmm[dst_idx];
            let d = dst.to_u32x4();
            let s = Xmm { lo: src_lo, hi: src_hi }.to_u32x4();
            let mut r = [0u32; 4];
            for i in 0..4 {
                r[i] = d[i].wrapping_mul(s[i]);
            }
            cpu.sse.xmm[dst_idx] = Xmm::from_u32x4(r);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── SSE4.1 PBLENDVB (66 0F 38 10): byte blend with XMM0 mask ──
        0x10 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let mask = cpu.sse.xmm[0]; // implicit XMM0
            let d = &mut cpu.sse.xmm[dst_idx];
            let mut dl = d.lo.to_le_bytes(); let mut dh = d.hi.to_le_bytes();
            let sl = src_lo.to_le_bytes(); let sh = src_hi.to_le_bytes();
            let ml = mask.lo.to_le_bytes(); let mh = mask.hi.to_le_bytes();
            for i in 0..8 { if ml[i] & 0x80 != 0 { dl[i] = sl[i]; } }
            for i in 0..8 { if mh[i] & 0x80 != 0 { dh[i] = sh[i]; } }
            d.lo = u64::from_le_bytes(dl); d.hi = u64::from_le_bytes(dh);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // ── SSE4.1 BLENDVPS (66 0F 38 14): float blend with XMM0 mask ──
        0x14 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let mask = cpu.sse.xmm[0];
            let d = &mut cpu.sse.xmm[dst_idx];
            let dd = d.to_u32x4(); let sd = Xmm { lo: src_lo, hi: src_hi }.to_u32x4();
            let md = mask.to_u32x4();
            let mut r = [0u32; 4];
            for i in 0..4 { r[i] = if md[i] & 0x80000000 != 0 { sd[i] } else { dd[i] }; }
            *d = Xmm::from_u32x4(r);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // ── SSE4.1 BLENDVPD (66 0F 38 15): double blend with XMM0 mask ──
        0x15 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let mask = cpu.sse.xmm[0];
            let d = &mut cpu.sse.xmm[dst_idx];
            if mask.lo & (1u64 << 63) != 0 { d.lo = src_lo; }
            if mask.hi & (1u64 << 63) != 0 { d.hi = src_hi; }
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── SSSE3 PABSB/PABSW/PABSD ──
        0x1C => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let mut lo_bytes = src_lo.to_le_bytes();
            let mut hi_bytes = src_hi.to_le_bytes();
            for b in &mut lo_bytes { *b = (*b as i8).unsigned_abs(); }
            for b in &mut hi_bytes { *b = (*b as i8).unsigned_abs(); }
            cpu.sse.xmm[dst_idx] = Xmm { lo: u64::from_le_bytes(lo_bytes), hi: u64::from_le_bytes(hi_bytes) };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        0x1D => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let src = Xmm { lo: src_lo, hi: src_hi };
            let mut words = [0u16; 8];
            for i in 0..4 { words[i] = ((src.lo >> (i*16)) & 0xFFFF) as u16; }
            for i in 0..4 { words[4+i] = ((src.hi >> (i*16)) & 0xFFFF) as u16; }
            for w in &mut words { *w = (*w as i16).unsigned_abs(); }
            let mut lo = 0u64; let mut hi = 0u64;
            for i in 0..4 { lo |= (words[i] as u64) << (i*16); }
            for i in 0..4 { hi |= (words[4+i] as u64) << (i*16); }
            cpu.sse.xmm[dst_idx] = Xmm { lo, hi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        0x1E => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let src = Xmm { lo: src_lo, hi: src_hi };
            let s = src.to_u32x4();
            let r = [
                (s[0] as i32).unsigned_abs(),
                (s[1] as i32).unsigned_abs(),
                (s[2] as i32).unsigned_abs(),
                (s[3] as i32).unsigned_abs(),
            ];
            cpu.sse.xmm[dst_idx] = Xmm::from_u32x4(r);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // ── SSE4.1 PMOVZX variants ──
        // PMOVZXBW: byte→word (8 bytes → 8 words)
        0x30 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let bytes = src_lo.to_le_bytes();
            let mut r = [0u16; 8];
            for i in 0..8 { r[i] = bytes[i] as u16; }
            let lo = r[0] as u64 | ((r[1] as u64) << 16) | ((r[2] as u64) << 32) | ((r[3] as u64) << 48);
            let hi = r[4] as u64 | ((r[5] as u64) << 16) | ((r[6] as u64) << 32) | ((r[7] as u64) << 48);
            cpu.sse.xmm[dst_idx] = Xmm { lo, hi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVZXBD: byte→dword (4 bytes → 4 dwords)
        0x31 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let bytes = src_lo.to_le_bytes();
            let r = Xmm::from_u32x4([bytes[0] as u32, bytes[1] as u32, bytes[2] as u32, bytes[3] as u32]);
            cpu.sse.xmm[dst_idx] = r;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVZXBQ: byte→qword (2 bytes → 2 qwords)
        0x32 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let bytes = src_lo.to_le_bytes();
            cpu.sse.xmm[dst_idx] = Xmm { lo: bytes[0] as u64, hi: bytes[1] as u64 };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVZXWD: word→dword (4 words → 4 dwords)
        0x33 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let w = [src_lo as u16, (src_lo >> 16) as u16, (src_lo >> 32) as u16, (src_lo >> 48) as u16];
            let r = Xmm::from_u32x4([w[0] as u32, w[1] as u32, w[2] as u32, w[3] as u32]);
            cpu.sse.xmm[dst_idx] = r;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVZXWQ: word→qword (2 words → 2 qwords)
        0x34 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx] = Xmm { lo: (src_lo & 0xFFFF), hi: ((src_lo >> 16) & 0xFFFF) };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVZXDQ: dword→qword (2 dwords → 2 qwords)
        0x35 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx] = Xmm { lo: src_lo & 0xFFFFFFFF, hi: (src_lo >> 32) & 0xFFFFFFFF };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // ── SSE4.1 PMOVSX variants ──
        // PMOVSXBW: signed byte→word
        0x20 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let bytes = src_lo.to_le_bytes();
            let mut r = [0u16; 8];
            for i in 0..8 { r[i] = bytes[i] as i8 as i16 as u16; }
            let lo = r[0] as u64 | ((r[1] as u64) << 16) | ((r[2] as u64) << 32) | ((r[3] as u64) << 48);
            let hi = r[4] as u64 | ((r[5] as u64) << 16) | ((r[6] as u64) << 32) | ((r[7] as u64) << 48);
            cpu.sse.xmm[dst_idx] = Xmm { lo, hi };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVSXBD: signed byte→dword
        0x21 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let bytes = src_lo.to_le_bytes();
            let r = Xmm::from_u32x4([bytes[0] as i8 as i32 as u32, bytes[1] as i8 as i32 as u32,
                                       bytes[2] as i8 as i32 as u32, bytes[3] as i8 as i32 as u32]);
            cpu.sse.xmm[dst_idx] = r;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVSXBQ: signed byte→qword
        0x22 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let bytes = src_lo.to_le_bytes();
            cpu.sse.xmm[dst_idx] = Xmm { lo: bytes[0] as i8 as i64 as u64, hi: bytes[1] as i8 as i64 as u64 };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVSXWD: signed word→dword
        0x23 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let w = [src_lo as u16, (src_lo >> 16) as u16, (src_lo >> 32) as u16, (src_lo >> 48) as u16];
            let r = Xmm::from_u32x4([w[0] as i16 as i32 as u32, w[1] as i16 as i32 as u32,
                                       w[2] as i16 as i32 as u32, w[3] as i16 as i32 as u32]);
            cpu.sse.xmm[dst_idx] = r;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVSXWQ: signed word→qword
        0x24 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx] = Xmm {
                lo: (src_lo & 0xFFFF) as u16 as i16 as i64 as u64,
                hi: ((src_lo >> 16) & 0xFFFF) as u16 as i16 as i64 as u64,
            };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // PMOVSXDQ: signed dword→qword
        0x25 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, _) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            cpu.sse.xmm[dst_idx] = Xmm {
                lo: src_lo as u32 as i32 as i64 as u64,
                hi: (src_lo >> 32) as u32 as i32 as i64 as u64,
            };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // F2 0F 38 F0/F1 /r -- CRC32
        0xF0 | 0xF1 if inst.rep == crate::instruction::RepPrefix::Repne => {
            exec_crc32(cpu, inst, memory, mmu)
        }

        // 0F 38 F0/F1 /r -- MOVBE
        0xF0 | 0xF1 => exec_movbe(cpu, inst, memory, mmu),

        _ => Err(VmError::UndefinedOpcode(op)),
    }
}

fn exec_sse_0f3a(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let op = inst.opcode as u8;
    match op {
        // 66 0F 3A 0E /r ib -- PBLENDW xmm, xmm/m128, imm8
        0x0E => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let imm = inst.immediate as u8;
            let dst = cpu.sse.xmm[dst_idx];
            // XMM as 8 words: words 0-3 in lo, words 4-7 in hi
            let mut dst_bytes = [0u8; 16];
            dst_bytes[..8].copy_from_slice(&dst.lo.to_le_bytes());
            dst_bytes[8..16].copy_from_slice(&dst.hi.to_le_bytes());
            let mut src_bytes = [0u8; 16];
            src_bytes[..8].copy_from_slice(&src_lo.to_le_bytes());
            src_bytes[8..16].copy_from_slice(&src_hi.to_le_bytes());
            for i in 0..8u8 {
                if (imm >> i) & 1 != 0 {
                    let off = (i as usize) * 2;
                    dst_bytes[off] = src_bytes[off];
                    dst_bytes[off + 1] = src_bytes[off + 1];
                }
            }
            let mut lo_arr = [0u8; 8];
            let mut hi_arr = [0u8; 8];
            lo_arr.copy_from_slice(&dst_bytes[..8]);
            hi_arr.copy_from_slice(&dst_bytes[8..]);
            cpu.sse.xmm[dst_idx] = Xmm {
                lo: u64::from_le_bytes(lo_arr),
                hi: u64::from_le_bytes(hi_arr),
            };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 3A 0F /r ib -- PALIGNR xmm, xmm/m128, imm8
        0x0F => {
            let dst_idx = xmm_dst_index(inst)?;
            let dst = cpu.sse.xmm[dst_idx];
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let imm = (inst.immediate as u8) as usize;

            let mut cat = [0u8; 32];
            cat[..8].copy_from_slice(&dst.lo.to_le_bytes());
            cat[8..16].copy_from_slice(&dst.hi.to_le_bytes());
            cat[16..24].copy_from_slice(&src_lo.to_le_bytes());
            cat[24..32].copy_from_slice(&src_hi.to_le_bytes());

            let mut out = [0u8; 16];
            for i in 0..16 {
                out[i] = if imm + i < 32 { cat[imm + i] } else { 0 };
            }
            let mut lo_arr = [0u8; 8];
            let mut hi_arr = [0u8; 8];
            lo_arr.copy_from_slice(&out[..8]);
            hi_arr.copy_from_slice(&out[8..]);
            cpu.sse.xmm[dst_idx] = Xmm {
                lo: u64::from_le_bytes(lo_arr),
                hi: u64::from_le_bytes(hi_arr),
            };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }
        // 66 0F 3A 14 /r ib -- PEXTRB r/m8, xmm, imm8
        0x14 => {
            let src_idx = xmm_src_index(inst)?;
            let xmm = cpu.sse.xmm[src_idx];
            let mut bytes = [0u8; 16];
            bytes[..8].copy_from_slice(&xmm.lo.to_le_bytes());
            bytes[8..16].copy_from_slice(&xmm.hi.to_le_bytes());
            let sel = (inst.immediate as usize) & 0xF;
            let val = bytes[sel] as u64;
            super::write_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 3A 16 /r ib -- PEXTRD r/m32, xmm, imm8
        0x16 => {
            let src_idx = xmm_src_index(inst)?;
            let dwords = cpu.sse.xmm[src_idx].to_u32x4();
            let sel = (inst.immediate as usize) & 0x3;
            let val = dwords[sel] as u64;
            super::write_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 3A 17 /r ib -- EXTRACTPS r/m32, xmm, imm8
        0x17 => {
            let src_idx = xmm_src_index(inst)?;
            let dwords = cpu.sse.xmm[src_idx].to_u32x4();
            let sel = (inst.immediate as usize) & 0x3;
            let val = dwords[sel] as u64;
            super::write_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 3A 20 /r ib -- PINSRB xmm, r/m8, imm8
        0x20 => {
            let dst_idx = xmm_dst_index(inst)?;
            let val = super::read_operand(cpu, inst, &inst.operands[1], memory, mmu)? as u8;
            let mut bytes = [0u8; 16];
            let xmm = cpu.sse.xmm[dst_idx];
            bytes[..8].copy_from_slice(&xmm.lo.to_le_bytes());
            bytes[8..16].copy_from_slice(&xmm.hi.to_le_bytes());
            let sel = (inst.immediate as usize) & 0xF;
            bytes[sel] = val;
            let mut lo_arr = [0u8; 8];
            let mut hi_arr = [0u8; 8];
            lo_arr.copy_from_slice(&bytes[..8]);
            hi_arr.copy_from_slice(&bytes[8..]);
            cpu.sse.xmm[dst_idx] = Xmm {
                lo: u64::from_le_bytes(lo_arr),
                hi: u64::from_le_bytes(hi_arr),
            };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 3A 21 /r ib -- INSERTPS xmm, xmm/m32, imm8
        0x21 => {
            let dst_idx = xmm_dst_index(inst)?;
            let imm = inst.immediate as u8;
            // Source: if xmm, select dword by count_s; if memory, use m32 directly
            let src_val = match &inst.operands[1] {
                Operand::Register(RegOperand::Xmm(src)) => {
                    let count_s = ((imm >> 6) & 3) as usize;
                    let s = cpu.sse.xmm[*src as usize].to_u32x4();
                    s[count_s]
                }
                _ => {
                    super::read_operand(cpu, inst, &inst.operands[1], memory, mmu)? as u32
                }
            };
            let count_d = ((imm >> 4) & 3) as usize;
            let zmask = imm & 0xF;
            let mut dwords = cpu.sse.xmm[dst_idx].to_u32x4();
            dwords[count_d] = src_val;
            for i in 0..4u8 {
                if (zmask >> i) & 1 != 0 {
                    dwords[i as usize] = 0;
                }
            }
            cpu.sse.xmm[dst_idx] = Xmm::from_u32x4(dwords);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 3A 22 /r ib -- PINSRD xmm, r/m32, imm8
        0x22 => {
            let dst_idx = xmm_dst_index(inst)?;
            let val = super::read_operand(cpu, inst, &inst.operands[1], memory, mmu)? as u32;
            let sel = (inst.immediate as usize) & 0x3;
            let mut dwords = cpu.sse.xmm[dst_idx].to_u32x4();
            dwords[sel] = val;
            cpu.sse.xmm[dst_idx] = Xmm::from_u32x4(dwords);
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        // 66 0F 3A 44 /r ib -- PCLMULQDQ xmm, xmm/m128, imm8
        0x44 => {
            let dst_idx = xmm_dst_index(inst)?;
            let (src_lo, src_hi) = read_xmm_or_mem128(cpu, inst, &inst.operands[1], memory, mmu)?;
            let imm = inst.immediate as u8;
            let a = if imm & 0x01 != 0 { cpu.sse.xmm[dst_idx].hi } else { cpu.sse.xmm[dst_idx].lo };
            let b = if imm & 0x10 != 0 { src_hi } else { src_lo };
            // Carry-less multiply: 64×64 → 128
            let mut result = [0u64; 2]; // [lo, hi]
            for i in 0..64 {
                if (b >> i) & 1 != 0 {
                    // XOR a<<i into result
                    if i < 64 {
                        result[0] ^= a << i;
                        if i > 0 { result[1] ^= a >> (64 - i); }
                    } else {
                        result[1] ^= a << (i - 64);
                    }
                }
            }
            cpu.sse.xmm[dst_idx] = Xmm { lo: result[0], hi: result[1] };
            cpu.regs.rip += inst.length as u64;
            Ok(())
        }

        _ => Err(VmError::UndefinedOpcode(op)),
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

fn pshufb_lane(dst_lane: &mut u64, src_lane: u64) {
    let src = src_lane.to_le_bytes();
    let mut out = [0u8; 8];
    let ctrl = dst_lane.to_le_bytes();
    for i in 0..8 {
        let c = ctrl[i];
        if (c & 0x80) != 0 {
            out[i] = 0;
        } else {
            out[i] = src[(c & 0x07) as usize];
        }
    }
    *dst_lane = u64::from_le_bytes(out);
}

fn crc32_update(mut crc: u32, val: u64, bytes: usize) -> u32 {
    for i in 0..bytes {
        crc ^= ((val >> (i * 8)) as u8) as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0x82F6_3B78;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn read_crc_src(
    cpu: &Cpu,
    inst: &DecodedInst,
    memory: &GuestMemory,
    mmu: &Mmu,
) -> Result<(u64, usize)> {
    match &inst.operands[1] {
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            let val = translate_and_read(cpu, linear, mem_op.size, mmu, memory)?;
            Ok((val, mem_op.size.bytes() as usize))
        }
        Operand::Register(RegOperand::Gpr(idx)) => {
            let size = match inst.opcode as u8 {
                0xF0 => OperandSize::Byte,
                0xF1 => {
                    if inst.prefix.rex_w() {
                        OperandSize::Qword
                    } else if inst.prefix.operand_size_override {
                        OperandSize::Word
                    } else {
                        OperandSize::Dword
                    }
                }
                _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
            };
            let val = cpu.regs.read_gpr(*idx, size, inst.prefix.has_rex());
            Ok((val, size.bytes() as usize))
        }
        _ => Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }
}

fn exec_crc32(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst = match &inst.operands[0] {
        Operand::Register(RegOperand::Gpr(idx)) => *idx,
        _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    };
    let (src_val, src_bytes) = read_crc_src(cpu, inst, memory, mmu)?;

    if inst.prefix.rex_w() {
        let old = cpu.regs.read_gpr64(dst) as u32;
        let new_crc = crc32_update(old, src_val, src_bytes);
        cpu.regs.write_gpr64(dst, new_crc as u64);
    } else {
        let old = cpu.regs.read_gpr32(dst);
        let new_crc = crc32_update(old, src_val, src_bytes);
        cpu.regs.write_gpr32(dst, new_crc);
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

fn exec_movbe(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let sz = match &inst.operands[0] {
        Operand::Memory(mem) => mem.size,
        _ => inst.operand_size,
    };
    let mask = sz.mask();
    if (inst.opcode as u8) == 0xF0 {
        // MOVBE r, m
        let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)? & mask;
        let swapped = match sz {
            OperandSize::Word => (src as u16).swap_bytes() as u64,
            OperandSize::Dword => (src as u32).swap_bytes() as u64,
            OperandSize::Qword => src.swap_bytes(),
            OperandSize::Byte => src,
        };
        write_operand(cpu, inst, &inst.operands[0], swapped, memory, mmu)?;
    } else {
        // MOVBE m, r
        let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)? & mask;
        let swapped = match sz {
            OperandSize::Word => (src as u16).swap_bytes() as u64,
            OperandSize::Dword => (src as u32).swap_bytes() as u64,
            OperandSize::Qword => src.swap_bytes(),
            OperandSize::Byte => src,
        };
        write_operand(cpu, inst, &inst.operands[0], swapped, memory, mmu)?;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
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

/// FXSAVE: store x87/SSE state to a 512-byte memory region.
fn exec_fxsave(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let base = get_mem_linear(cpu, inst)?;

    translate_and_write(cpu, base, OperandSize::Word, cpu.fpu.fcw as u64, mmu, memory)?;
    translate_and_write(cpu, base.wrapping_add(2), OperandSize::Word, cpu.fpu.fsw as u64, mmu, memory)?;
    // Abridged FTW in FXSAVE: one bit per ST entry, 1 = valid
    let mut abridged_ftw: u8 = 0;
    for i in 0..8u8 {
        let tag = cpu.fpu.get_tag(i);
        if tag != crate::fpu_state::TAG_EMPTY {
            abridged_ftw |= 1 << i;
        }
    }
    translate_and_write(cpu, base.wrapping_add(4), OperandSize::Byte, abridged_ftw as u64, mmu, memory)?;
    translate_and_write(cpu, base.wrapping_add(6), OperandSize::Word, cpu.fpu.fop as u64, mmu, memory)?;
    translate_and_write(cpu, base.wrapping_add(8), OperandSize::Qword, cpu.fpu.fip, mmu, memory)?;
    translate_and_write(cpu, base.wrapping_add(16), OperandSize::Qword, cpu.fpu.fdp, mmu, memory)?;
    translate_and_write(cpu, base.wrapping_add(24), OperandSize::Dword, cpu.sse.mxcsr as u64, mmu, memory)?;
    translate_and_write(cpu, base.wrapping_add(28), OperandSize::Dword, 0x0000_FFFF, mmu, memory)?;

    // ST/MM registers (8 x 16 bytes). We store f64 bits in low qword.
    for i in 0..8u64 {
        let st = cpu.fpu.st(i as u8).to_bits();
        let off = base.wrapping_add(32 + i * 16);
        translate_and_write(cpu, off, OperandSize::Qword, st, mmu, memory)?;
        translate_and_write(cpu, off.wrapping_add(8), OperandSize::Qword, 0, mmu, memory)?;
    }

    // XMM registers (16 x 16 bytes)
    for i in 0..16u64 {
        let xmm = cpu.sse.xmm[i as usize];
        let off = base.wrapping_add(160 + i * 16);
        translate_and_write(cpu, off, OperandSize::Qword, xmm.lo, mmu, memory)?;
        translate_and_write(cpu, off.wrapping_add(8), OperandSize::Qword, xmm.hi, mmu, memory)?;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// FXRSTOR: load x87/SSE state from a 512-byte memory region.
fn exec_fxrstor(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let base = get_mem_linear(cpu, inst)?;

    cpu.fpu.fcw = translate_and_read(cpu, base, OperandSize::Word, mmu, memory)? as u16;
    cpu.fpu.fsw = translate_and_read(cpu, base.wrapping_add(2), OperandSize::Word, mmu, memory)? as u16;
    let abridged_ftw = translate_and_read(cpu, base.wrapping_add(4), OperandSize::Byte, mmu, memory)? as u8;
    cpu.fpu.fop = translate_and_read(cpu, base.wrapping_add(6), OperandSize::Word, mmu, memory)? as u16;
    cpu.fpu.fip = translate_and_read(cpu, base.wrapping_add(8), OperandSize::Qword, mmu, memory)?;
    cpu.fpu.fdp = translate_and_read(cpu, base.wrapping_add(16), OperandSize::Qword, mmu, memory)?;

    let mxcsr = translate_and_read(cpu, base.wrapping_add(24), OperandSize::Dword, mmu, memory)? as u32;
    if (mxcsr & !MXCSR_WRITE_MASK) != 0 {
        return Err(VmError::GeneralProtection(0));
    }
    cpu.sse.mxcsr = mxcsr;

    for i in 0..8u64 {
        let off = base.wrapping_add(32 + i * 16);
        let lo = translate_and_read(cpu, off, OperandSize::Qword, mmu, memory)?;
        cpu.fpu.set_st(i as u8, f64::from_bits(lo));
        if (abridged_ftw & (1 << i)) == 0 {
            cpu.fpu.set_tag(i as u8, crate::fpu_state::TAG_EMPTY);
        }
    }

    for i in 0..16u64 {
        let off = base.wrapping_add(160 + i * 16);
        let lo = translate_and_read(cpu, off, OperandSize::Qword, mmu, memory)?;
        let hi = translate_and_read(cpu, off.wrapping_add(8), OperandSize::Qword, mmu, memory)?;
        cpu.sse.xmm[i as usize] = Xmm { lo, hi };
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}
