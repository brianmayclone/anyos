//! x87 FPU instruction handlers.
//!
//! Implements the most commonly used x87 floating-point instructions.
//! Unrecognized FPU opcodes are silently skipped (RIP advanced) rather
//! than raising `#UD`, matching the behavior expected by guest code that
//! may probe FPU capabilities.
//!
//! The x87 instruction set uses opcodes D8-DF as escape bytes. The ModR/M
//! byte further distinguishes the specific operation. When ModR/M mod < 3,
//! the instruction has a memory operand. When mod == 3, the instruction
//! operates on FPU registers.

use crate::cpu::Cpu;
use crate::error::Result;
use crate::flags::OperandSize;
use crate::instruction::DecodedInst;
use crate::memory::{GuestMemory, Mmu};
use crate::registers::GprIndex;

use super::{translate_and_read, translate_and_write};

/// Dispatch an x87 FPU instruction based on the escape byte (D8-DF) and ModR/M.
///
/// For unrecognized opcodes, advances RIP without side effects. This prevents
/// crashes on rarely-used FPU instructions that the emulator does not model.
pub fn exec_fpu(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let escape = inst.opcode as u8; // D8-DF
    let modrm = inst.modrm.unwrap_or(0);
    let modrm_mod = (modrm >> 6) & 3;
    let modrm_reg = (modrm >> 3) & 7;
    let modrm_rm = modrm & 7;

    match escape {
        // ── D8: arithmetic with memory float32 or ST(i) ──
        0xD8 => {
            if modrm_mod != 3 {
                // Memory operand: 32-bit float
                let val = load_f32_memory(cpu, inst, memory, mmu)?;
                match modrm_reg {
                    0 => {
                        // FADD ST(0), m32
                        let st0 = cpu.fpu.st(0);
                        cpu.fpu.set_st(0, st0 + val);
                    }
                    1 => {
                        // FMUL ST(0), m32
                        let st0 = cpu.fpu.st(0);
                        cpu.fpu.set_st(0, st0 * val);
                    }
                    2 => {
                        // FCOM m32 — compare ST(0) with m32
                        fpu_compare(cpu, cpu.fpu.st(0), val);
                    }
                    3 => {
                        // FCOMP m32 — compare and pop
                        let st0 = cpu.fpu.st(0);
                        fpu_compare(cpu, st0, val);
                        cpu.fpu.pop();
                    }
                    4 => {
                        // FSUB ST(0), m32
                        let st0 = cpu.fpu.st(0);
                        cpu.fpu.set_st(0, st0 - val);
                    }
                    5 => {
                        // FSUBR ST(0), m32
                        let st0 = cpu.fpu.st(0);
                        cpu.fpu.set_st(0, val - st0);
                    }
                    6 => {
                        // FDIV ST(0), m32
                        let st0 = cpu.fpu.st(0);
                        cpu.fpu.set_st(0, st0 / val);
                    }
                    7 => {
                        // FDIVR ST(0), m32
                        let st0 = cpu.fpu.st(0);
                        cpu.fpu.set_st(0, val / st0);
                    }
                    _ => {}
                }
            } else {
                // Register-register: D8 C0+i..D8 FF
                let st_i = cpu.fpu.st(modrm_rm);
                let st0 = cpu.fpu.st(0);
                match modrm_reg {
                    0 => cpu.fpu.set_st(0, st0 + st_i),  // FADD ST(0), ST(i)
                    1 => cpu.fpu.set_st(0, st0 * st_i),  // FMUL ST(0), ST(i)
                    2 => fpu_compare(cpu, st0, st_i),     // FCOM ST(i)
                    3 => {
                        fpu_compare(cpu, st0, st_i);
                        cpu.fpu.pop();
                    } // FCOMP ST(i)
                    4 => cpu.fpu.set_st(0, st0 - st_i),  // FSUB ST(0), ST(i)
                    5 => cpu.fpu.set_st(0, st_i - st0),   // FSUBR
                    6 => cpu.fpu.set_st(0, st0 / st_i),  // FDIV ST(0), ST(i)
                    7 => cpu.fpu.set_st(0, st_i / st0),   // FDIVR
                    _ => {}
                }
            }
        }

        // ── D9: load/store/misc ──
        0xD9 => {
            if modrm_mod != 3 {
                match modrm_reg {
                    0 => {
                        // FLD m32
                        let val = load_f32_memory(cpu, inst, memory, mmu)?;
                        cpu.fpu.push(val);
                    }
                    2 => {
                        // FST m32
                        let val = cpu.fpu.st(0);
                        store_f32_memory(cpu, inst, val, memory, mmu)?;
                    }
                    3 => {
                        // FSTP m32
                        let val = cpu.fpu.st(0);
                        store_f32_memory(cpu, inst, val, memory, mmu)?;
                        cpu.fpu.pop();
                    }
                    5 => {
                        // FLDCW m16
                        let linear = get_mem_linear(cpu, inst)?;
                        let val = translate_and_read(cpu, linear, OperandSize::Word, mmu, memory)?;
                        cpu.fpu.fcw = val as u16;
                    }
                    7 => {
                        // FSTCW / FNSTCW m16
                        let linear = get_mem_linear(cpu, inst)?;
                        translate_and_write(
                            cpu,
                            linear,
                            OperandSize::Word,
                            cpu.fpu.fcw as u64,
                            mmu,
                            memory,
                        )?;
                    }
                    _ => {} // Skip unrecognized
                }
            } else {
                // Register forms
                match modrm {
                    // D9 C0+i: FLD ST(i)
                    0xC0..=0xC7 => {
                        let val = cpu.fpu.st(modrm_rm);
                        cpu.fpu.push(val);
                    }
                    // D9 C8+i: FXCH ST(i)
                    0xC8..=0xCF => {
                        let st0 = cpu.fpu.st(0);
                        let sti = cpu.fpu.st(modrm_rm);
                        cpu.fpu.set_st(0, sti);
                        cpu.fpu.set_st(modrm_rm, st0);
                    }
                    // D9 D0: FNOP
                    0xD0 => {}
                    // D9 E0: FCHS — negate ST(0)
                    0xE0 => {
                        let val = cpu.fpu.st(0);
                        cpu.fpu.set_st(0, -val);
                    }
                    // D9 E1: FABS — absolute value of ST(0)
                    0xE1 => {
                        let val = cpu.fpu.st(0);
                        // no_std: use bit manipulation for abs
                        let abs_val = f64::from_bits(val.to_bits() & !(1u64 << 63));
                        cpu.fpu.set_st(0, abs_val);
                    }
                    // D9 E4: FTST — compare ST(0) with 0.0
                    0xE4 => {
                        fpu_compare(cpu, cpu.fpu.st(0), 0.0);
                    }
                    // D9 E8: FLD1 — push 1.0
                    0xE8 => {
                        cpu.fpu.push(1.0);
                    }
                    // D9 E9: FLDL2T — push log2(10) (approx)
                    0xE9 => {
                        cpu.fpu.push(3.321928094887362);
                    }
                    // D9 EA: FLDL2E — push log2(e) (approx)
                    0xEA => {
                        cpu.fpu.push(1.4426950408889634);
                    }
                    // D9 EB: FLDPI — push pi (approx)
                    0xEB => {
                        cpu.fpu.push(core::f64::consts::PI);
                    }
                    // D9 EC: FLDLG2 — push log10(2) (approx)
                    0xEC => {
                        cpu.fpu.push(0.30102999566398114);
                    }
                    // D9 ED: FLDLN2 — push ln(2) (approx)
                    0xED => {
                        cpu.fpu.push(core::f64::consts::LN_2);
                    }
                    // D9 EE: FLDZ — push 0.0
                    0xEE => {
                        cpu.fpu.push(0.0);
                    }
                    _ => {} // Skip unrecognized
                }
            }
        }

        // ── DA: arithmetic with memory int32 ──
        0xDA => {
            if modrm_mod != 3 {
                let linear = get_mem_linear(cpu, inst)?;
                let raw = translate_and_read(cpu, linear, OperandSize::Dword, mmu, memory)?;
                let val = (raw as i32) as f64;
                let st0 = cpu.fpu.st(0);
                match modrm_reg {
                    0 => cpu.fpu.set_st(0, st0 + val),
                    1 => cpu.fpu.set_st(0, st0 * val),
                    2 => fpu_compare(cpu, st0, val),
                    3 => {
                        fpu_compare(cpu, st0, val);
                        cpu.fpu.pop();
                    }
                    4 => cpu.fpu.set_st(0, st0 - val),
                    5 => cpu.fpu.set_st(0, val - st0),
                    6 => cpu.fpu.set_st(0, st0 / val),
                    7 => cpu.fpu.set_st(0, val / st0),
                    _ => {}
                }
            } else {
                // DA E9: FUCOMPP — compare and pop twice
                if modrm == 0xE9 {
                    let st0 = cpu.fpu.st(0);
                    let st1 = cpu.fpu.st(1);
                    fpu_compare(cpu, st0, st1);
                    cpu.fpu.pop();
                    cpu.fpu.pop();
                }
                // Other register forms: skip
            }
        }

        // ── DB: misc / FINIT / FCLEX ──
        0xDB => {
            if modrm_mod != 3 {
                match modrm_reg {
                    0 => {
                        // FILD m32int
                        let linear = get_mem_linear(cpu, inst)?;
                        let raw = translate_and_read(cpu, linear, OperandSize::Dword, mmu, memory)?;
                        cpu.fpu.push((raw as i32) as f64);
                    }
                    2 => {
                        // FIST m32int
                        let val = cpu.fpu.st(0) as i32;
                        let linear = get_mem_linear(cpu, inst)?;
                        translate_and_write(cpu, linear, OperandSize::Dword, val as u32 as u64, mmu, memory)?;
                    }
                    3 => {
                        // FISTP m32int
                        let val = cpu.fpu.st(0) as i32;
                        let linear = get_mem_linear(cpu, inst)?;
                        translate_and_write(cpu, linear, OperandSize::Dword, val as u32 as u64, mmu, memory)?;
                        cpu.fpu.pop();
                    }
                    5 => {
                        // FLD m80 — load 80-bit extended (approximate as f64)
                        let linear = get_mem_linear(cpu, inst)?;
                        let lo = translate_and_read(cpu, linear, OperandSize::Qword, mmu, memory)?;
                        let hi = translate_and_read(
                            cpu,
                            linear.wrapping_add(8),
                            OperandSize::Word,
                            mmu,
                            memory,
                        )? as u16;
                        let val = decode_f80(lo, hi);
                        cpu.fpu.push(val);
                    }
                    7 => {
                        // FSTP m80 — store 80-bit extended
                        let val = cpu.fpu.st(0);
                        let (lo, hi) = encode_f80(val);
                        let linear = get_mem_linear(cpu, inst)?;
                        translate_and_write(cpu, linear, OperandSize::Qword, lo, mmu, memory)?;
                        translate_and_write(
                            cpu,
                            linear.wrapping_add(8),
                            OperandSize::Word,
                            hi as u64,
                            mmu,
                            memory,
                        )?;
                        cpu.fpu.pop();
                    }
                    _ => {}
                }
            } else {
                match modrm {
                    // DB E2: FCLEX/FNCLEX — clear exception flags
                    0xE2 => {
                        cpu.fpu.fsw &= !0x00FF; // Clear exception flags
                    }
                    // DB E3: FINIT/FNINIT — initialize FPU
                    0xE3 => {
                        cpu.fpu.reset();
                    }
                    _ => {} // Skip
                }
            }
        }

        // ── DC: arithmetic with memory float64 or ST(i), ST(0) ──
        0xDC => {
            if modrm_mod != 3 {
                let val = load_f64_memory(cpu, inst, memory, mmu)?;
                let st0 = cpu.fpu.st(0);
                match modrm_reg {
                    0 => cpu.fpu.set_st(0, st0 + val),
                    1 => cpu.fpu.set_st(0, st0 * val),
                    2 => fpu_compare(cpu, st0, val),
                    3 => {
                        fpu_compare(cpu, st0, val);
                        cpu.fpu.pop();
                    }
                    4 => cpu.fpu.set_st(0, st0 - val),
                    5 => cpu.fpu.set_st(0, val - st0),
                    6 => cpu.fpu.set_st(0, st0 / val),
                    7 => cpu.fpu.set_st(0, val / st0),
                    _ => {}
                }
            } else {
                // DC C0+i..DC FF: arithmetic ST(i), ST(0)
                let st0 = cpu.fpu.st(0);
                let sti = cpu.fpu.st(modrm_rm);
                match modrm_reg {
                    0 => cpu.fpu.set_st(modrm_rm, sti + st0),
                    1 => cpu.fpu.set_st(modrm_rm, sti * st0),
                    4 => cpu.fpu.set_st(modrm_rm, sti - st0),
                    5 => cpu.fpu.set_st(modrm_rm, st0 - sti),
                    6 => cpu.fpu.set_st(modrm_rm, sti / st0),
                    7 => cpu.fpu.set_st(modrm_rm, st0 / sti),
                    _ => {}
                }
            }
        }

        // ── DD: load/store float64, FFREE, FST/FSTP ──
        0xDD => {
            if modrm_mod != 3 {
                match modrm_reg {
                    0 => {
                        // FLD m64
                        let val = load_f64_memory(cpu, inst, memory, mmu)?;
                        cpu.fpu.push(val);
                    }
                    2 => {
                        // FST m64
                        let val = cpu.fpu.st(0);
                        store_f64_memory(cpu, inst, val, memory, mmu)?;
                    }
                    3 => {
                        // FSTP m64
                        let val = cpu.fpu.st(0);
                        store_f64_memory(cpu, inst, val, memory, mmu)?;
                        cpu.fpu.pop();
                    }
                    7 => {
                        // FNSTSW m16 — store status word to memory
                        let linear = get_mem_linear(cpu, inst)?;
                        translate_and_write(
                            cpu,
                            linear,
                            OperandSize::Word,
                            cpu.fpu.fsw as u64,
                            mmu,
                            memory,
                        )?;
                    }
                    _ => {}
                }
            } else {
                match modrm_reg {
                    0 => {
                        // DD C0+i: FFREE ST(i)
                        cpu.fpu.set_tag(
                            (cpu.fpu.top.wrapping_add(modrm_rm)) & 7,
                            crate::fpu_state::TAG_EMPTY,
                        );
                    }
                    2 => {
                        // DD D0+i: FST ST(i)
                        let val = cpu.fpu.st(0);
                        cpu.fpu.set_st(modrm_rm, val);
                    }
                    3 => {
                        // DD D8+i: FSTP ST(i)
                        let val = cpu.fpu.st(0);
                        cpu.fpu.set_st(modrm_rm, val);
                        cpu.fpu.pop();
                    }
                    _ => {}
                }
            }
        }

        // ── DE: arithmetic with int16 memory or register+pop ──
        0xDE => {
            if modrm_mod != 3 {
                let linear = get_mem_linear(cpu, inst)?;
                let raw = translate_and_read(cpu, linear, OperandSize::Word, mmu, memory)?;
                let val = (raw as i16) as f64;
                let st0 = cpu.fpu.st(0);
                match modrm_reg {
                    0 => cpu.fpu.set_st(0, st0 + val),
                    1 => cpu.fpu.set_st(0, st0 * val),
                    2 => fpu_compare(cpu, st0, val),
                    3 => {
                        fpu_compare(cpu, st0, val);
                        cpu.fpu.pop();
                    }
                    4 => cpu.fpu.set_st(0, st0 - val),
                    5 => cpu.fpu.set_st(0, val - st0),
                    6 => cpu.fpu.set_st(0, st0 / val),
                    7 => cpu.fpu.set_st(0, val / st0),
                    _ => {}
                }
            } else {
                // Register-register with pop
                let st0 = cpu.fpu.st(0);
                let sti = cpu.fpu.st(modrm_rm);
                match modrm_reg {
                    0 => cpu.fpu.set_st(modrm_rm, sti + st0),
                    1 => cpu.fpu.set_st(modrm_rm, sti * st0),
                    4 => cpu.fpu.set_st(modrm_rm, sti - st0),
                    5 => cpu.fpu.set_st(modrm_rm, st0 - sti),
                    6 => cpu.fpu.set_st(modrm_rm, sti / st0),
                    7 => cpu.fpu.set_st(modrm_rm, st0 / sti),
                    _ => {}
                }
                // Pop ST(0) after the operation
                if modrm_reg <= 7 {
                    cpu.fpu.pop();
                }
            }
        }

        // ── DF: misc integer load/store, FNSTSW AX ──
        0xDF => {
            if modrm_mod != 3 {
                match modrm_reg {
                    0 => {
                        // FILD m16int
                        let linear = get_mem_linear(cpu, inst)?;
                        let raw = translate_and_read(cpu, linear, OperandSize::Word, mmu, memory)?;
                        cpu.fpu.push((raw as i16) as f64);
                    }
                    2 => {
                        // FIST m16int
                        let val = cpu.fpu.st(0) as i16;
                        let linear = get_mem_linear(cpu, inst)?;
                        translate_and_write(cpu, linear, OperandSize::Word, val as u16 as u64, mmu, memory)?;
                    }
                    3 => {
                        // FISTP m16int
                        let val = cpu.fpu.st(0) as i16;
                        let linear = get_mem_linear(cpu, inst)?;
                        translate_and_write(cpu, linear, OperandSize::Word, val as u16 as u64, mmu, memory)?;
                        cpu.fpu.pop();
                    }
                    5 => {
                        // FILD m64int
                        let linear = get_mem_linear(cpu, inst)?;
                        let raw = translate_and_read(cpu, linear, OperandSize::Qword, mmu, memory)?;
                        cpu.fpu.push((raw as i64) as f64);
                    }
                    7 => {
                        // FISTP m64int
                        let val = cpu.fpu.st(0) as i64;
                        let linear = get_mem_linear(cpu, inst)?;
                        translate_and_write(cpu, linear, OperandSize::Qword, val as u64, mmu, memory)?;
                        cpu.fpu.pop();
                    }
                    _ => {}
                }
            } else {
                // DF E0: FNSTSW AX — store FPU status word to AX
                if modrm == 0xE0 {
                    cpu.regs.write_gpr16(GprIndex::Rax as u8, cpu.fpu.fsw);
                }
                // Other register forms: skip
            }
        }

        _ => {} // Should not reach here, but be safe
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── FPU helpers ──

/// Set the FPU condition codes (C0, C2, C3) in FSW based on a comparison.
///
/// C3:C2:C0 encoding:
/// - ST(0) > val: 0:0:0
/// - ST(0) < val: 0:0:1
/// - ST(0) = val: 1:0:0
/// - Unordered:   1:1:1
fn fpu_compare(cpu: &mut Cpu, a: f64, b: f64) {
    // Clear C0 (bit 8), C2 (bit 10), C3 (bit 14)
    cpu.fpu.fsw &= !(0x4500);

    if a.is_nan() || b.is_nan() {
        // Unordered
        cpu.fpu.fsw |= 0x4500; // C3=1, C2=1, C0=1
    } else if a > b {
        // C3=0, C2=0, C0=0 (already cleared)
    } else if a < b {
        cpu.fpu.fsw |= 0x0100; // C0=1
    } else {
        // Equal
        cpu.fpu.fsw |= 0x4000; // C3=1
    }
}

/// Load a 32-bit float from the memory operand, returning it as f64.
fn load_f32_memory(
    cpu: &Cpu,
    inst: &DecodedInst,
    memory: &GuestMemory,
    mmu: &Mmu,
) -> Result<f64> {
    let linear = get_mem_linear(cpu, inst)?;
    let raw = translate_and_read(cpu, linear, OperandSize::Dword, mmu, memory)?;
    Ok(f32::from_bits(raw as u32) as f64)
}

/// Store an f64 value as a 32-bit float to the memory operand.
fn store_f32_memory(
    cpu: &Cpu,
    inst: &DecodedInst,
    val: f64,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let linear = get_mem_linear(cpu, inst)?;
    let bits = (val as f32).to_bits();
    translate_and_write(cpu, linear, OperandSize::Dword, bits as u64, mmu, memory)
}

/// Load a 64-bit float from the memory operand.
fn load_f64_memory(
    cpu: &Cpu,
    inst: &DecodedInst,
    memory: &GuestMemory,
    mmu: &Mmu,
) -> Result<f64> {
    let linear = get_mem_linear(cpu, inst)?;
    let raw = translate_and_read(cpu, linear, OperandSize::Qword, mmu, memory)?;
    Ok(f64::from_bits(raw))
}

/// Store an f64 value as a 64-bit float to the memory operand.
fn store_f64_memory(
    cpu: &Cpu,
    inst: &DecodedInst,
    val: f64,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let linear = get_mem_linear(cpu, inst)?;
    translate_and_write(cpu, linear, OperandSize::Qword, val.to_bits(), mmu, memory)
}

/// Get the linear address of the memory operand at position 0.
fn get_mem_linear(cpu: &Cpu, inst: &DecodedInst) -> Result<u64> {
    match &inst.operands[0] {
        crate::instruction::Operand::Memory(mem_op) => {
            super::compute_effective_address(cpu, mem_op, inst)
        }
        _ => {
            // Try operand 1 for instructions like FIST where the memory
            // operand might be in a different slot
            match &inst.operands[1] {
                crate::instruction::Operand::Memory(mem_op) => {
                    super::compute_effective_address(cpu, mem_op, inst)
                }
                _ => Err(crate::error::VmError::UndefinedOpcode(inst.opcode as u8)),
            }
        }
    }
}

/// Approximate decode of an 80-bit extended precision float to f64.
///
/// This is a simplified conversion that handles the common cases (normal
/// numbers, zero, infinity, NaN) but may lose precision for denormals
/// and values outside the f64 range.
fn decode_f80(significand: u64, exponent_sign: u16) -> f64 {
    let sign = (exponent_sign >> 15) & 1;
    let exponent = exponent_sign & 0x7FFF;

    if exponent == 0 && significand == 0 {
        return if sign != 0 { -0.0 } else { 0.0 };
    }

    if exponent == 0x7FFF {
        if significand == 0x8000_0000_0000_0000 {
            return if sign != 0 {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }
        return f64::NAN;
    }

    // Convert 80-bit bias (16383) to 64-bit bias (1023)
    let exp64 = exponent as i32 - 16383 + 1023;

    if exp64 <= 0 || exp64 >= 2047 {
        // Out of f64 range: return infinity or zero
        if exp64 >= 2047 {
            return if sign != 0 {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }
        return if sign != 0 { -0.0 } else { 0.0 };
    }

    // Build f64 bits: sign(1) + exponent(11) + mantissa(52)
    // The 80-bit format has an explicit integer bit; f64 has an implicit one.
    // Drop the integer bit and take the top 52 mantissa bits.
    let mantissa52 = (significand << 1) >> 12; // shift out integer bit, take top 52

    let bits = ((sign as u64) << 63) | ((exp64 as u64) << 52) | mantissa52;
    f64::from_bits(bits)
}

/// Approximate encode of an f64 to 80-bit extended precision.
///
/// Returns (significand: u64, exponent_sign: u16).
fn encode_f80(val: f64) -> (u64, u16) {
    let bits = val.to_bits();
    let sign = ((bits >> 63) & 1) as u16;
    let exp64 = ((bits >> 52) & 0x7FF) as i32;
    let mantissa52 = bits & 0x000F_FFFF_FFFF_FFFF;

    if exp64 == 0 && mantissa52 == 0 {
        // Zero
        return (0, sign << 15);
    }

    if exp64 == 0x7FF {
        if mantissa52 == 0 {
            // Infinity
            return (0x8000_0000_0000_0000, (sign << 15) | 0x7FFF);
        }
        // NaN
        return (0xC000_0000_0000_0000, (sign << 15) | 0x7FFF);
    }

    // Convert exponent bias: f64 bias=1023 -> f80 bias=16383
    let exp80 = (exp64 - 1023 + 16383) as u16;

    // Reconstruct significand with explicit integer bit
    let significand = 0x8000_0000_0000_0000 | (mantissa52 << 11);

    (significand, (sign << 15) | exp80)
}
