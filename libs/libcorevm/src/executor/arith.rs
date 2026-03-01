//! Arithmetic instruction handlers.
//!
//! Implements ADD, ADC, SUB, SBB, CMP, INC, DEC, NEG, MUL, IMUL (1/2/3-op),
//! DIV, and IDIV. Each function reads operands, computes the result, writes
//! back (except CMP), updates RFLAGS, and advances RIP.

use crate::cpu::Cpu;
use crate::error::{Result, VmError};
use crate::flags::{self, OperandSize};
use crate::instruction::DecodedInst;
use crate::memory::{GuestMemory, Mmu};

use super::{read_operand, write_operand};

/// ADD: `dst = dst + src`, updating all arithmetic flags.
pub fn exec_add(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let size = inst.operand_size;
    let result = dst_val.wrapping_add(src_val) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_add(dst_val, src_val, result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// ADC: `dst = dst + src + CF`, updating all arithmetic flags.
pub fn exec_adc(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let carry = if (cpu.regs.rflags & flags::CF) != 0 { 1u64 } else { 0u64 };
    let size = inst.operand_size;
    let result = dst_val.wrapping_add(src_val).wrapping_add(carry) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_add(dst_val, src_val.wrapping_add(carry), result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// SUB: `dst = dst - src`, updating all arithmetic flags.
pub fn exec_sub(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let size = inst.operand_size;
    let result = dst_val.wrapping_sub(src_val) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_sub(dst_val, src_val, result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// SBB: `dst = dst - src - CF`, updating all arithmetic flags.
pub fn exec_sbb(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let borrow = if (cpu.regs.rflags & flags::CF) != 0 { 1u64 } else { 0u64 };
    let size = inst.operand_size;
    let result = dst_val.wrapping_sub(src_val).wrapping_sub(borrow) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_sub(dst_val, src_val.wrapping_add(borrow), result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// CMP: compute `dst - src`, update flags, but do NOT write the result.
pub fn exec_cmp(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let size = inst.operand_size;
    let result = dst_val.wrapping_sub(src_val) & size.mask();

    let f = flags::flags_sub(dst_val, src_val, result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// INC: `dst += 1`, preserving CF.
pub fn exec_inc(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let size = inst.operand_size;
    let result = dst_val.wrapping_add(1) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_inc(dst_val, result, size);
    flags::update_flags_preserve_cf(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// DEC: `dst -= 1`, preserving CF.
pub fn exec_dec(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let size = inst.operand_size;
    let result = dst_val.wrapping_sub(1) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_dec(dst_val, result, size);
    flags::update_flags_preserve_cf(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// NEG: `dst = 0 - dst` (two's complement negation).
///
/// CF is set if the original value was non-zero. Other flags are set as for
/// `SUB 0, dst`.
pub fn exec_neg(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let size = inst.operand_size;
    let result = (0u64).wrapping_sub(dst_val) & size.mask();

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    let f = flags::flags_sub(0, dst_val, result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    // CF is set if the original operand was non-zero
    if dst_val & size.mask() != 0 {
        cpu.regs.rflags |= flags::CF;
    } else {
        cpu.regs.rflags &= !flags::CF;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// MUL: unsigned multiply.
///
/// - 8-bit:  AX = AL * src8
/// - 16-bit: DX:AX = AX * src16
/// - 32-bit: EDX:EAX = EAX * src32
/// - 64-bit: RDX:RAX = RAX * src64
///
/// OF=CF=(high half != 0). SF, ZF, AF, PF are undefined.
pub fn exec_mul(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let src = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let size = inst.operand_size;

    match size {
        OperandSize::Byte => {
            let al = cpu.regs.read_gpr8(0, false) as u16;
            let product = al * (src as u16);
            cpu.regs.write_gpr16(0, product); // AX = result
            let high = (product >> 8) as u8;
            if high != 0 {
                cpu.regs.rflags |= flags::CF | flags::OF;
            } else {
                cpu.regs.rflags &= !(flags::CF | flags::OF);
            }
        }
        OperandSize::Word => {
            let ax = cpu.regs.read_gpr16(0) as u32;
            let product = ax * (src as u32);
            cpu.regs.write_gpr16(0, product as u16); // AX = low
            cpu.regs.write_gpr16(2, (product >> 16) as u16); // DX = high
            if (product >> 16) != 0 {
                cpu.regs.rflags |= flags::CF | flags::OF;
            } else {
                cpu.regs.rflags &= !(flags::CF | flags::OF);
            }
        }
        OperandSize::Dword => {
            let eax = cpu.regs.read_gpr32(0) as u64;
            let product = eax * (src as u64);
            cpu.regs.write_gpr32(0, product as u32); // EAX = low
            cpu.regs.write_gpr32(2, (product >> 32) as u32); // EDX = high
            if (product >> 32) != 0 {
                cpu.regs.rflags |= flags::CF | flags::OF;
            } else {
                cpu.regs.rflags &= !(flags::CF | flags::OF);
            }
        }
        OperandSize::Qword => {
            let rax = cpu.regs.read_gpr64(0) as u128;
            let product = rax * (src as u128);
            cpu.regs.write_gpr64(0, product as u64); // RAX = low
            cpu.regs.write_gpr64(2, (product >> 64) as u64); // RDX = high
            if (product >> 64) != 0 {
                cpu.regs.rflags |= flags::CF | flags::OF;
            } else {
                cpu.regs.rflags &= !(flags::CF | flags::OF);
            }
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// IMUL (single operand): signed multiply, same layout as MUL but signed.
///
/// OF=CF=(sign extension of low half != full result).
pub fn exec_imul_1op(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let src = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let size = inst.operand_size;

    match size {
        OperandSize::Byte => {
            let al = cpu.regs.read_gpr8(0, false) as i8 as i16;
            let s = src as i8 as i16;
            let product = al.wrapping_mul(s);
            cpu.regs.write_gpr16(0, product as u16); // AX
            let low = product as i8;
            if low as i16 != product {
                cpu.regs.rflags |= flags::CF | flags::OF;
            } else {
                cpu.regs.rflags &= !(flags::CF | flags::OF);
            }
        }
        OperandSize::Word => {
            let ax = cpu.regs.read_gpr16(0) as i16 as i32;
            let s = src as i16 as i32;
            let product = ax.wrapping_mul(s);
            cpu.regs.write_gpr16(0, product as u16); // AX
            cpu.regs.write_gpr16(2, (product >> 16) as u16); // DX
            let low = product as i16;
            if low as i32 != product {
                cpu.regs.rflags |= flags::CF | flags::OF;
            } else {
                cpu.regs.rflags &= !(flags::CF | flags::OF);
            }
        }
        OperandSize::Dword => {
            let eax = cpu.regs.read_gpr32(0) as i32 as i64;
            let s = src as i32 as i64;
            let product = eax.wrapping_mul(s);
            cpu.regs.write_gpr32(0, product as u32); // EAX
            cpu.regs.write_gpr32(2, (product >> 32) as u32); // EDX
            let low = product as i32;
            if low as i64 != product {
                cpu.regs.rflags |= flags::CF | flags::OF;
            } else {
                cpu.regs.rflags &= !(flags::CF | flags::OF);
            }
        }
        OperandSize::Qword => {
            let rax = cpu.regs.read_gpr64(0) as i64 as i128;
            let s = src as i64 as i128;
            let product = rax.wrapping_mul(s);
            cpu.regs.write_gpr64(0, product as u64); // RAX
            cpu.regs.write_gpr64(2, (product >> 64) as u64); // RDX
            let low = product as i64;
            if low as i128 != product {
                cpu.regs.rflags |= flags::CF | flags::OF;
            } else {
                cpu.regs.rflags &= !(flags::CF | flags::OF);
            }
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// IMUL (two-operand): `dst = dst * src` (truncated to operand size).
///
/// OF=CF=(sign extension of truncated result != full result).
pub fn exec_imul_2op(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let size = inst.operand_size;

    let (result, overflow) = imul_truncated(dst_val, src_val, size);

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    if overflow {
        cpu.regs.rflags |= flags::CF | flags::OF;
    } else {
        cpu.regs.rflags &= !(flags::CF | flags::OF);
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// IMUL (three-operand): `dst = src1 * imm` (truncated).
///
/// OF=CF=(sign extension of truncated result != full result).
pub fn exec_imul_3op(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let imm_val = read_operand(cpu, inst, &inst.operands[2], memory, mmu)?;
    let size = inst.operand_size;

    let (result, overflow) = imul_truncated(src_val, imm_val, size);

    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    if overflow {
        cpu.regs.rflags |= flags::CF | flags::OF;
    } else {
        cpu.regs.rflags &= !(flags::CF | flags::OF);
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// Signed multiply with truncation and overflow detection.
///
/// Returns (truncated_result, overflow_flag). Overflow is set when the full
/// signed product cannot be represented in the given operand size.
fn imul_truncated(a: u64, b: u64, size: OperandSize) -> (u64, bool) {
    let mask = size.mask();
    match size {
        OperandSize::Byte => {
            let sa = a as i8 as i16;
            let sb = b as i8 as i16;
            let product = sa.wrapping_mul(sb);
            let truncated = (product as u16 & 0xFF) as u64;
            let overflow = (truncated as i8 as i16) != product;
            (truncated, overflow)
        }
        OperandSize::Word => {
            let sa = a as i16 as i32;
            let sb = b as i16 as i32;
            let product = sa.wrapping_mul(sb);
            let truncated = (product as u32 & 0xFFFF) as u64;
            let overflow = (truncated as i16 as i32) != product;
            (truncated, overflow)
        }
        OperandSize::Dword => {
            let sa = a as i32 as i64;
            let sb = b as i32 as i64;
            let product = sa.wrapping_mul(sb);
            let truncated = (product as u64) & mask;
            let overflow = (truncated as i32 as i64) != product;
            (truncated, overflow)
        }
        OperandSize::Qword => {
            let sa = a as i64 as i128;
            let sb = b as i64 as i128;
            let product = sa.wrapping_mul(sb);
            let truncated = product as u64;
            let overflow = (truncated as i64 as i128) != product;
            (truncated, overflow)
        }
    }
}

/// DIV: unsigned division.
///
/// - 8-bit:  AL = AX / src8, AH = AX % src8
/// - 16-bit: AX = DX:AX / src16, DX = DX:AX % src16
/// - 32-bit: EAX = EDX:EAX / src32, EDX = EDX:EAX % src32
/// - 64-bit: RAX = RDX:RAX / src64, RDX = RDX:RAX % src64
///
/// Raises `#DE` if the divisor is zero or the quotient overflows.
pub fn exec_div(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let divisor = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let size = inst.operand_size;

    if divisor == 0 {
        return Err(VmError::DivideByZero);
    }

    match size {
        OperandSize::Byte => {
            let dividend = cpu.regs.read_gpr16(0) as u16; // AX
            let d = divisor as u8 as u16;
            let quotient = dividend / d;
            let remainder = dividend % d;
            if quotient > 0xFF {
                return Err(VmError::DivideByZero);
            }
            // AL = quotient, AH = remainder
            let ax = (quotient & 0xFF) | ((remainder & 0xFF) << 8);
            cpu.regs.write_gpr16(0, ax);
        }
        OperandSize::Word => {
            let dx = cpu.regs.read_gpr16(2) as u32;
            let ax = cpu.regs.read_gpr16(0) as u32;
            let dividend = (dx << 16) | ax;
            let d = divisor as u16 as u32;
            let quotient = dividend / d;
            let remainder = dividend % d;
            if quotient > 0xFFFF {
                return Err(VmError::DivideByZero);
            }
            cpu.regs.write_gpr16(0, quotient as u16);
            cpu.regs.write_gpr16(2, remainder as u16);
        }
        OperandSize::Dword => {
            let edx = cpu.regs.read_gpr32(2) as u64;
            let eax = cpu.regs.read_gpr32(0) as u64;
            let dividend = (edx << 32) | eax;
            let d = divisor as u32 as u64;
            let quotient = dividend / d;
            let remainder = dividend % d;
            if quotient > 0xFFFF_FFFF {
                return Err(VmError::DivideByZero);
            }
            cpu.regs.write_gpr32(0, quotient as u32);
            cpu.regs.write_gpr32(2, remainder as u32);
        }
        OperandSize::Qword => {
            let rdx = cpu.regs.read_gpr64(2) as u128;
            let rax = cpu.regs.read_gpr64(0) as u128;
            let dividend = (rdx << 64) | rax;
            let d = divisor as u128;
            let quotient = dividend / d;
            let remainder = dividend % d;
            if quotient > u64::MAX as u128 {
                return Err(VmError::DivideByZero);
            }
            cpu.regs.write_gpr64(0, quotient as u64);
            cpu.regs.write_gpr64(2, remainder as u64);
        }
    }

    // Flags are undefined after DIV
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// IDIV: signed division.
///
/// Same register layout as DIV but using signed arithmetic. Raises `#DE`
/// on zero divisor or quotient overflow.
pub fn exec_idiv(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let divisor = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let size = inst.operand_size;

    if divisor == 0 {
        return Err(VmError::DivideByZero);
    }

    match size {
        OperandSize::Byte => {
            let dividend = cpu.regs.read_gpr16(0) as i16;
            let d = divisor as i8 as i16;
            if d == 0 {
                return Err(VmError::DivideByZero);
            }
            let quotient = dividend / d;
            let remainder = dividend % d;
            if quotient > 127 || quotient < -128 {
                return Err(VmError::DivideByZero);
            }
            let ax = ((quotient as u8) as u16) | (((remainder as u8) as u16) << 8);
            cpu.regs.write_gpr16(0, ax);
        }
        OperandSize::Word => {
            let dx = cpu.regs.read_gpr16(2) as u16;
            let ax = cpu.regs.read_gpr16(0) as u16;
            let dividend = (((dx as u32) << 16) | (ax as u32)) as i32;
            let d = divisor as i16 as i32;
            if d == 0 {
                return Err(VmError::DivideByZero);
            }
            let quotient = dividend / d;
            let remainder = dividend % d;
            if quotient > 0x7FFF || quotient < -0x8000 {
                return Err(VmError::DivideByZero);
            }
            cpu.regs.write_gpr16(0, quotient as u16);
            cpu.regs.write_gpr16(2, remainder as u16);
        }
        OperandSize::Dword => {
            let edx = cpu.regs.read_gpr32(2) as u32;
            let eax = cpu.regs.read_gpr32(0) as u32;
            let dividend = (((edx as u64) << 32) | (eax as u64)) as i64;
            let d = divisor as i32 as i64;
            if d == 0 {
                return Err(VmError::DivideByZero);
            }
            let quotient = dividend / d;
            let remainder = dividend % d;
            if quotient > 0x7FFF_FFFF || quotient < -0x8000_0000 {
                return Err(VmError::DivideByZero);
            }
            cpu.regs.write_gpr32(0, quotient as u32);
            cpu.regs.write_gpr32(2, remainder as u32);
        }
        OperandSize::Qword => {
            let rdx = cpu.regs.read_gpr64(2) as u64;
            let rax = cpu.regs.read_gpr64(0) as u64;
            let dividend = (((rdx as u128) << 64) | (rax as u128)) as i128;
            let d = divisor as i64 as i128;
            if d == 0 {
                return Err(VmError::DivideByZero);
            }
            let quotient = dividend / d;
            let remainder = dividend % d;
            if quotient > i64::MAX as i128 || quotient < i64::MIN as i128 {
                return Err(VmError::DivideByZero);
            }
            cpu.regs.write_gpr64(0, quotient as u64);
            cpu.regs.write_gpr64(2, remainder as u64);
        }
    }

    // Flags are undefined after IDIV
    cpu.regs.rip += inst.length as u64;
    Ok(())
}
