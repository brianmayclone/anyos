//! Logic, bitwise, shift, and rotate instruction handlers.
//!
//! Implements AND, OR, XOR, NOT, TEST, SHL, SHR, SAR, ROL, ROR, RCL, RCR,
//! BT, BTS, BTR, BTC, BSF, and BSR.

use crate::cpu::Cpu;
use crate::error::Result;
use crate::flags::{self, OperandSize};
use crate::instruction::DecodedInst;
use crate::memory::{GuestMemory, Mmu};

use super::{read_operand, write_operand};

// ── Logic operations ──

/// AND: `dst = dst & src`, CF=OF=0, PF/ZF/SF from result.
pub fn exec_and(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let size = inst.operand_size;
    let result = (dst_val & src_val) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_logic(result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// OR: `dst = dst | src`, CF=OF=0, PF/ZF/SF from result.
pub fn exec_or(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let size = inst.operand_size;
    let result = (dst_val | src_val) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_logic(result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// XOR: `dst = dst ^ src`, CF=OF=0, PF/ZF/SF from result.
pub fn exec_xor(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let size = inst.operand_size;
    let result = (dst_val ^ src_val) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_logic(result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// NOT: `dst = ~dst` (bitwise complement). No flags are modified.
pub fn exec_not(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let size = inst.operand_size;
    let result = (!dst_val) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// TEST: `dst & src` without writing; CF=OF=0, PF/ZF/SF from result.
pub fn exec_test(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let size = inst.operand_size;
    let result = (dst_val & src_val) & size.mask();

    let f = flags::flags_logic(result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── Shift operations ──

/// Read the shift count from the instruction.
///
/// For opcodes D0/D1 (shift by 1), the count is 1. For D2/D3 (shift by CL),
/// the count comes from CL. For C0/C1 (shift by imm8), the count is the
/// immediate. The count is masked to 5 bits (32-bit) or 6 bits (64-bit).
fn get_shift_count(cpu: &Cpu, inst: &DecodedInst) -> u32 {
    let raw = if inst.operand_count >= 2 {
        match inst.operands[1] {
            crate::instruction::Operand::Immediate(imm) => imm as u32,
            crate::instruction::Operand::Register(crate::instruction::RegOperand::Gpr(1)) => {
                // CL register
                cpu.regs.read_gpr8(1, false) as u32
            }
            _ => 1, // fallback for D0/D1
        }
    } else {
        1
    };

    // Mask count: 5 bits for 8/16/32-bit, 6 bits for 64-bit
    let mask = if inst.operand_size == OperandSize::Qword { 0x3F } else { 0x1F };
    raw & mask
}

/// SHL/SAL: logical/arithmetic shift left.
///
/// CF = last bit shifted out. OF = sign change on count=1.
pub fn exec_shl(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let count = get_shift_count(cpu, inst);
    let size = inst.operand_size;

    if count == 0 {
        cpu.regs.rip += inst.length as u64;
        return Ok(());
    }

    let bits = size.bits();
    let mask = size.mask();

    let result = if count >= bits {
        0u64
    } else {
        (dst_val << count) & mask
    };

    // CF = last bit shifted out
    let cf = if count <= bits {
        ((dst_val >> (bits - count)) & 1) != 0
    } else {
        false
    };

    // OF = MSB of result XOR CF (defined only for count=1)
    let of = if count == 1 {
        (((result >> (bits - 1)) & 1) != 0) != cf
    } else {
        false
    };

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_shift(result, cf, of, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// SHR: logical shift right.
///
/// CF = last bit shifted out. OF = MSB of original value (for count=1).
pub fn exec_shr(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)? & inst.operand_size.mask();
    let count = get_shift_count(cpu, inst);
    let size = inst.operand_size;

    if count == 0 {
        cpu.regs.rip += inst.length as u64;
        return Ok(());
    }

    let bits = size.bits();

    let result = if count >= bits {
        0u64
    } else {
        dst_val >> count
    };

    // CF = last bit shifted out
    let cf = if count <= bits {
        ((dst_val >> (count - 1)) & 1) != 0
    } else {
        false
    };

    // OF = MSB of original operand (for count=1)
    let of = if count == 1 {
        ((dst_val >> (bits - 1)) & 1) != 0
    } else {
        false
    };

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_shift(result, cf, of, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// SAR: arithmetic shift right (preserves sign bit).
///
/// CF = last bit shifted out. OF = 0 for count=1 (sign doesn't change).
pub fn exec_sar(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)? & inst.operand_size.mask();
    let count = get_shift_count(cpu, inst);
    let size = inst.operand_size;

    if count == 0 {
        cpu.regs.rip += inst.length as u64;
        return Ok(());
    }

    let bits = size.bits();
    let mask = size.mask();

    // Sign-extend the value to perform arithmetic shift
    let signed_val = sign_extend(dst_val, size);

    let result = if count >= bits {
        // All bits shifted out; result is all-zeros or all-ones depending on sign
        if (signed_val as i64) < 0 {
            mask
        } else {
            0u64
        }
    } else {
        ((signed_val as i64) >> count) as u64 & mask
    };

    // CF = last bit shifted out
    let cf = if count >= bits {
        ((signed_val >> (bits - 1)) & 1) != 0
    } else {
        ((dst_val >> (count - 1)) & 1) != 0
    };

    // OF = 0 for count=1 (SAR never changes sign)
    let of = false;

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_shift(result, cf, of, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── Rotate operations ──

/// ROL: rotate left.
///
/// CF = LSB of result (last bit rotated to position 0). OF = MSB xor CF (for count=1).
pub fn exec_rol(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)? & inst.operand_size.mask();
    let count = get_shift_count(cpu, inst);
    let size = inst.operand_size;

    if count == 0 {
        cpu.regs.rip += inst.length as u64;
        return Ok(());
    }

    let bits = size.bits();
    let mask = size.mask();
    let effective_count = count % bits;

    let result = if effective_count == 0 {
        dst_val
    } else {
        ((dst_val << effective_count) | (dst_val >> (bits - effective_count))) & mask
    };

    let cf = (result & 1) != 0;
    let of = if count == 1 {
        (((result >> (bits - 1)) & 1) != 0) != cf
    } else {
        // OF undefined for count != 1; preserve existing
        (cpu.regs.rflags & flags::OF) != 0
    };

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    // Only CF and OF are modified; PF/ZF/SF/AF unchanged for rotates
    if cf {
        cpu.regs.rflags |= flags::CF;
    } else {
        cpu.regs.rflags &= !flags::CF;
    }
    if of {
        cpu.regs.rflags |= flags::OF;
    } else {
        cpu.regs.rflags &= !flags::OF;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// ROR: rotate right.
///
/// CF = MSB of result. OF = MSB xor second-MSB of result (for count=1).
pub fn exec_ror(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)? & inst.operand_size.mask();
    let count = get_shift_count(cpu, inst);
    let size = inst.operand_size;

    if count == 0 {
        cpu.regs.rip += inst.length as u64;
        return Ok(());
    }

    let bits = size.bits();
    let mask = size.mask();
    let effective_count = count % bits;

    let result = if effective_count == 0 {
        dst_val
    } else {
        ((dst_val >> effective_count) | (dst_val << (bits - effective_count))) & mask
    };

    let cf = ((result >> (bits - 1)) & 1) != 0;
    let of = if count == 1 {
        let msb = ((result >> (bits - 1)) & 1) != 0;
        let second_msb = ((result >> (bits - 2)) & 1) != 0;
        msb != second_msb
    } else {
        (cpu.regs.rflags & flags::OF) != 0
    };

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    if cf {
        cpu.regs.rflags |= flags::CF;
    } else {
        cpu.regs.rflags &= !flags::CF;
    }
    if of {
        cpu.regs.rflags |= flags::OF;
    } else {
        cpu.regs.rflags &= !flags::OF;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// RCL: rotate left through carry.
///
/// Rotates (bits+1)-wide value including CF.
pub fn exec_rcl(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)? & inst.operand_size.mask();
    let count = get_shift_count(cpu, inst);
    let size = inst.operand_size;

    if count == 0 {
        cpu.regs.rip += inst.length as u64;
        return Ok(());
    }

    let bits = size.bits();
    let mask = size.mask();
    let old_cf = if (cpu.regs.rflags & flags::CF) != 0 { 1u64 } else { 0u64 };

    // Effective rotation modulo (bits+1) since CF is included
    let effective = count % (bits + 1);

    let mut val = dst_val;
    let mut cf = old_cf;
    for _ in 0..effective {
        let new_cf = (val >> (bits - 1)) & 1;
        val = ((val << 1) | cf) & mask;
        cf = new_cf;
    }

    write_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;

    if cf != 0 {
        cpu.regs.rflags |= flags::CF;
    } else {
        cpu.regs.rflags &= !flags::CF;
    }
    if count == 1 {
        let msb = ((val >> (bits - 1)) & 1) != 0;
        if msb != (cf != 0) {
            cpu.regs.rflags |= flags::OF;
        } else {
            cpu.regs.rflags &= !flags::OF;
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// RCR: rotate right through carry.
///
/// Rotates (bits+1)-wide value including CF.
pub fn exec_rcr(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)? & inst.operand_size.mask();
    let count = get_shift_count(cpu, inst);
    let size = inst.operand_size;

    if count == 0 {
        cpu.regs.rip += inst.length as u64;
        return Ok(());
    }

    let bits = size.bits();
    let mask = size.mask();
    let old_cf = if (cpu.regs.rflags & flags::CF) != 0 { 1u64 } else { 0u64 };

    // OF is defined for count=1 BEFORE the rotate
    if count == 1 {
        let msb = ((dst_val >> (bits - 1)) & 1) != 0;
        if msb != (old_cf != 0) {
            cpu.regs.rflags |= flags::OF;
        } else {
            cpu.regs.rflags &= !flags::OF;
        }
    }

    let effective = count % (bits + 1);

    let mut val = dst_val;
    let mut cf = old_cf;
    for _ in 0..effective {
        let new_cf = val & 1;
        val = ((cf << (bits - 1)) | (val >> 1)) & mask;
        cf = new_cf;
    }

    write_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;

    if cf != 0 {
        cpu.regs.rflags |= flags::CF;
    } else {
        cpu.regs.rflags &= !flags::CF;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── Bit test operations ──

/// Read the bit index from the source operand and compute the bit position.
///
/// For register sources, the bit index is masked to the operand size.
/// For immediate sources, the immediate value is used directly.
fn get_bit_index(cpu: &Cpu, inst: &DecodedInst, memory: &GuestMemory, mmu: &Mmu) -> Result<u32> {
    let src = super::read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let bits = inst.operand_size.bits();
    Ok((src as u32) % bits)
}

/// BT: test bit at position. CF = selected bit.
pub fn exec_bt(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let bit_idx = get_bit_index(cpu, inst, memory, mmu)?;

    if ((dst_val >> bit_idx) & 1) != 0 {
        cpu.regs.rflags |= flags::CF;
    } else {
        cpu.regs.rflags &= !flags::CF;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// BTS: test bit and set. CF = selected bit, then set the bit.
pub fn exec_bts(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let bit_idx = get_bit_index(cpu, inst, memory, mmu)?;

    if ((dst_val >> bit_idx) & 1) != 0 {
        cpu.regs.rflags |= flags::CF;
    } else {
        cpu.regs.rflags &= !flags::CF;
    }

    let result = dst_val | (1u64 << bit_idx);
    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// BTR: test bit and reset. CF = selected bit, then clear the bit.
pub fn exec_btr(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let bit_idx = get_bit_index(cpu, inst, memory, mmu)?;

    if ((dst_val >> bit_idx) & 1) != 0 {
        cpu.regs.rflags |= flags::CF;
    } else {
        cpu.regs.rflags &= !flags::CF;
    }

    let result = dst_val & !(1u64 << bit_idx);
    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// BTC: test bit and complement. CF = selected bit, then flip the bit.
pub fn exec_btc(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let bit_idx = get_bit_index(cpu, inst, memory, mmu)?;

    if ((dst_val >> bit_idx) & 1) != 0 {
        cpu.regs.rflags |= flags::CF;
    } else {
        cpu.regs.rflags &= !flags::CF;
    }

    let result = dst_val ^ (1u64 << bit_idx);
    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── Bit scan operations ──

/// BSF: bit scan forward. Find the lowest set bit.
///
/// If src is zero, ZF=1 and dst is undefined. Otherwise, ZF=0 and dst
/// is the index of the lowest set bit.
pub fn exec_bsf(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)? & inst.operand_size.mask();

    if src == 0 {
        cpu.regs.rflags |= flags::ZF;
    } else {
        cpu.regs.rflags &= !flags::ZF;
        let bit_index = src.trailing_zeros() as u64;
        write_operand(cpu, inst, &inst.operands[0], bit_index, memory, mmu)?;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// BSR: bit scan reverse. Find the highest set bit.
///
/// If src is zero, ZF=1 and dst is undefined. Otherwise, ZF=0 and dst
/// is the index of the highest set bit.
pub fn exec_bsr(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)? & inst.operand_size.mask();

    if src == 0 {
        cpu.regs.rflags |= flags::ZF;
    } else {
        cpu.regs.rflags &= !flags::ZF;
        let bit_index = (63 - src.leading_zeros()) as u64;
        write_operand(cpu, inst, &inst.operands[0], bit_index, memory, mmu)?;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── Utility ──

/// Sign-extend a value from the given operand size to 64 bits.
fn sign_extend(val: u64, size: OperandSize) -> u64 {
    match size {
        OperandSize::Byte => val as u8 as i8 as i64 as u64,
        OperandSize::Word => val as u16 as i16 as i64 as u64,
        OperandSize::Dword => val as u32 as i32 as i64 as u64,
        OperandSize::Qword => val,
    }
}
