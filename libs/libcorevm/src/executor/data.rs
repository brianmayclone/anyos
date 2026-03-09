//! Data movement instruction handlers.
//!
//! Implements MOV, MOVZX, MOVSX, MOVSXD, LEA, XCHG, BSWAP, CBW/CWDE/CDQE,
//! CWD/CDQ/CQO, CMOVcc, CMPXCHG, XADD, XLAT, MOV seg, SAHF, and LAHF.

use crate::cpu::{Cpu, Mode};
use crate::error::{Result, VmError};
use crate::flags::{self, OperandSize};
use crate::instruction::{DecodedInst, Operand, RegOperand};
use crate::memory::{GuestMemory, Mmu};
use crate::registers::{GprIndex, SegReg};

use super::{
    compute_effective_address, read_operand, translate_and_read, translate_and_write, write_operand,
};

/// MOV: simple data transfer with no flags modification.
pub fn exec_mov(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    write_operand(cpu, inst, &inst.operands[0], src, memory, mmu)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// MOVZX: zero-extend source to destination size.
pub fn exec_movzx(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    // Determine source size from opcode: 0xB6 = r/m8, 0xB7 = r/m16
    let src_size = if (inst.opcode as u8) == 0xB6 {
        OperandSize::Byte
    } else {
        OperandSize::Word
    };

    // Read source at the *source* size (not inst.operand_size which is the dest size).
    let src = match &inst.operands[1] {
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            translate_and_read(cpu, linear, mem_op.size, mmu, memory)?
        }
        Operand::Register(RegOperand::Gpr(idx)) => {
            cpu.regs.read_gpr(*idx, src_size, inst.prefix.has_rex())
        }
        _ => return Err(VmError::UndefinedOpcode(0)),
    };

    // Write to destination at destination size (inst.operand_size).
    write_operand(cpu, inst, &inst.operands[0], src, memory, mmu)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// MOVSX: sign-extend source to destination size.
pub fn exec_movsx(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    // Determine source size from opcode: 0xBE = r/m8, 0xBF = r/m16
    let src_size = if (inst.opcode as u8) == 0xBE {
        OperandSize::Byte
    } else {
        OperandSize::Word
    };

    // Read source at the *source* size (not inst.operand_size which is the dest size).
    let src = match &inst.operands[1] {
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            translate_and_read(cpu, linear, mem_op.size, mmu, memory)?
        }
        Operand::Register(RegOperand::Gpr(idx)) => {
            cpu.regs.read_gpr(*idx, src_size, inst.prefix.has_rex())
        }
        _ => return Err(VmError::UndefinedOpcode(0)),
    };

    let sign_extended = match src_size {
        OperandSize::Byte => src as u8 as i8 as i64 as u64,
        OperandSize::Word => src as u16 as i16 as i64 as u64,
        OperandSize::Dword => src as u32 as i32 as i64 as u64,
        OperandSize::Qword => src,
    };

    // Mask to destination size
    let result = sign_extended & inst.operand_size.mask();
    write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// MOVSXD: sign-extend 32-bit to 64-bit (64-bit mode only).
pub fn exec_movsxd(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    // Read source at Dword size (not inst.operand_size which is Qword).
    let src = match &inst.operands[1] {
        Operand::Memory(mem_op) => {
            let linear = compute_effective_address(cpu, mem_op, inst)?;
            translate_and_read(cpu, linear, mem_op.size, mmu, memory)?
        }
        Operand::Register(RegOperand::Gpr(idx)) => {
            cpu.regs.read_gpr(*idx, OperandSize::Dword, inst.prefix.has_rex())
        }
        _ => return Err(VmError::UndefinedOpcode(0)),
    };
    let sign_extended = src as u32 as i32 as i64 as u64;
    write_operand(cpu, inst, &inst.operands[0], sign_extended, memory, mmu)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// LEA: compute effective address and store in register (no memory access).
pub fn exec_lea(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let addr = match &inst.operands[1] {
        Operand::Memory(mem_op) => compute_effective_address(cpu, mem_op, inst)?,
        _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    };

    // LEA does not use segment base; subtract it back out
    let linear = match &inst.operands[1] {
        Operand::Memory(mem_op) => {
            let seg_base = cpu.regs.segment(mem_op.segment).base;
            addr.wrapping_sub(seg_base)
        }
        _ => addr,
    };

    // Mask to operand size
    let result = linear & inst.operand_size.mask();

    match &inst.operands[0] {
        Operand::Register(RegOperand::Gpr(idx)) => {
            cpu.regs.write_gpr(*idx, inst.operand_size, inst.prefix.has_rex(), result);
        }
        _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// XCHG: swap two operands atomically. No flags modified.
pub fn exec_xchg(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let val0 = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let val1 = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    write_operand(cpu, inst, &inst.operands[0], val1, memory, mmu)?;
    write_operand(cpu, inst, &inst.operands[1], val0, memory, mmu)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// BSWAP: byte-swap a 32-bit or 64-bit register.
pub fn exec_bswap(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    match &inst.operands[0] {
        Operand::Register(RegOperand::Gpr(idx)) => {
            let idx = *idx;
            match inst.operand_size {
                OperandSize::Dword => {
                    let val = cpu.regs.read_gpr32(idx);
                    cpu.regs.write_gpr32(idx, val.swap_bytes());
                }
                OperandSize::Qword => {
                    let val = cpu.regs.read_gpr64(idx);
                    cpu.regs.write_gpr64(idx, val.swap_bytes());
                }
                _ => {
                    // BSWAP with 16-bit operand is undefined; result is zero on real HW
                    cpu.regs.write_gpr16(idx, 0);
                }
            }
        }
        _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// CBW/CWDE/CDQE: sign-extend AL->AX, AX->EAX, or EAX->RAX.
pub fn exec_cbw_cwde_cdqe(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    match inst.operand_size {
        OperandSize::Word => {
            // CBW: AL -> AX
            let al = cpu.regs.read_gpr8(0, false) as i8 as i16;
            cpu.regs.write_gpr16(0, al as u16);
        }
        OperandSize::Dword => {
            // CWDE: AX -> EAX
            let ax = cpu.regs.read_gpr16(0) as i16 as i32;
            cpu.regs.write_gpr32(0, ax as u32);
        }
        OperandSize::Qword => {
            // CDQE: EAX -> RAX
            let eax = cpu.regs.read_gpr32(0) as i32 as i64;
            cpu.regs.write_gpr64(0, eax as u64);
        }
        _ => {}
    }
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// CWD/CDQ/CQO: sign-extend AX->DX:AX, EAX->EDX:EAX, or RAX->RDX:RAX.
pub fn exec_cwd_cdq_cqo(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    match inst.operand_size {
        OperandSize::Word => {
            // CWD: sign of AX -> DX
            let ax = cpu.regs.read_gpr16(0) as i16;
            let dx = if ax < 0 { 0xFFFFu16 } else { 0u16 };
            cpu.regs.write_gpr16(GprIndex::Rdx as u8, dx);
        }
        OperandSize::Dword => {
            // CDQ: sign of EAX -> EDX
            let eax = cpu.regs.read_gpr32(0) as i32;
            let edx = if eax < 0 { 0xFFFF_FFFFu32 } else { 0u32 };
            cpu.regs.write_gpr32(GprIndex::Rdx as u8, edx);
        }
        OperandSize::Qword => {
            // CQO: sign of RAX -> RDX
            let rax = cpu.regs.read_gpr64(0) as i64;
            let rdx = if rax < 0 { u64::MAX } else { 0u64 };
            cpu.regs.write_gpr64(GprIndex::Rdx as u8, rdx);
        }
        _ => {}
    }
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// CMOVcc: conditional move. Move src to dst if condition code is met.
pub fn exec_cmov(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let cc = (inst.opcode as u8) & 0x0F;
    let condition_met = flags::eval_cc(cc, cpu.regs.rflags);

    if condition_met {
        let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
        write_operand(cpu, inst, &inst.operands[0], src, memory, mmu)?;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// CMPXCHG: compare and exchange.
///
/// Compares the accumulator (AL/AX/EAX/RAX) with dst. If equal, ZF=1 and
/// src is loaded into dst. Otherwise, ZF=0 and dst is loaded into accumulator.
pub fn exec_cmpxchg(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = inst.operand_size;
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let acc = cpu.regs.read_gpr(GprIndex::Rax as u8, size, inst.prefix.has_rex());

    let cmp_result = acc.wrapping_sub(dst_val) & size.mask();
    let f = flags::flags_sub(acc, dst_val, cmp_result, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    if (acc & size.mask()) == (dst_val & size.mask()) {
        // Equal: store src into dst
        write_operand(cpu, inst, &inst.operands[0], src_val, memory, mmu)?;
    } else {
        // Not equal: load dst into accumulator
        cpu.regs.write_gpr(GprIndex::Rax as u8, size, inst.prefix.has_rex(), dst_val);
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// CMPXCHG8B/CMPXCHG16B: compare and exchange 64/128-bit memory operand.
///
/// - 0F C7 /1                => CMPXCHG8B m64
/// - REX.W + 0F C7 /1 (64b)  => CMPXCHG16B m128
///
/// Only ZF is architecturally defined; other flags are left unchanged.
pub fn exec_cmpxchg8b16b(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let mem = match &inst.operands[0] {
        Operand::Memory(mem_op) => mem_op,
        _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    };
    let linear = compute_effective_address(cpu, mem, inst)?;

    if inst.prefix.rex_w() && matches!(cpu.mode, Mode::LongMode) {
        // CMPXCHG16B m128
        let mem_lo = translate_and_read(cpu, linear, OperandSize::Qword, mmu, memory)?;
        let mem_hi = translate_and_read(cpu, linear.wrapping_add(8), OperandSize::Qword, mmu, memory)?;

        let acc_lo = cpu.regs.read_gpr64(GprIndex::Rax as u8);
        let acc_hi = cpu.regs.read_gpr64(GprIndex::Rdx as u8);

        if mem_lo == acc_lo && mem_hi == acc_hi {
            let new_lo = cpu.regs.read_gpr64(GprIndex::Rbx as u8);
            let new_hi = cpu.regs.read_gpr64(GprIndex::Rcx as u8);
            translate_and_write(cpu, linear, OperandSize::Qword, new_lo, mmu, memory)?;
            translate_and_write(cpu, linear.wrapping_add(8), OperandSize::Qword, new_hi, mmu, memory)?;
            cpu.regs.rflags |= flags::ZF;
        } else {
            cpu.regs.write_gpr64(GprIndex::Rax as u8, mem_lo);
            cpu.regs.write_gpr64(GprIndex::Rdx as u8, mem_hi);
            cpu.regs.rflags &= !flags::ZF;
        }
    } else {
        // CMPXCHG8B m64
        let mem_lo = translate_and_read(cpu, linear, OperandSize::Dword, mmu, memory)? as u32;
        let mem_hi = translate_and_read(cpu, linear.wrapping_add(4), OperandSize::Dword, mmu, memory)? as u32;
        let mem_u64 = (mem_hi as u64) << 32 | (mem_lo as u64);

        let acc_u64 = ((cpu.regs.read_gpr32(GprIndex::Rdx as u8) as u64) << 32)
            | (cpu.regs.read_gpr32(GprIndex::Rax as u8) as u64);

        if mem_u64 == acc_u64 {
            let new_lo = cpu.regs.read_gpr32(GprIndex::Rbx as u8) as u64;
            let new_hi = cpu.regs.read_gpr32(GprIndex::Rcx as u8) as u64;
            translate_and_write(cpu, linear, OperandSize::Dword, new_lo, mmu, memory)?;
            translate_and_write(cpu, linear.wrapping_add(4), OperandSize::Dword, new_hi, mmu, memory)?;
            cpu.regs.rflags |= flags::ZF;
        } else {
            cpu.regs.write_gpr32(GprIndex::Rax as u8, mem_lo);
            cpu.regs.write_gpr32(GprIndex::Rdx as u8, mem_hi);
            cpu.regs.rflags &= !flags::ZF;
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// RDRAND / RDSEED: store hardware random number in register.
///
/// CF=1 on success (always for us), OF=SF=ZF=AF=PF=0.
pub fn exec_rdrand(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    // LCG PRNG using instruction count as evolving state — not cryptographically
    // secure, but sufficient for guest OS entropy needs in emulation.
    // Mixing RIP + instruction_count ensures different values on repeated calls.
    let seed = cpu.instruction_count
        .wrapping_mul(6364136223846793005)
        .wrapping_add(cpu.regs.rip)
        .wrapping_add(1442695040888963407);
    let rand_val = seed;

    // RDRAND/RDSEED destination is always a GPR register (enforced by decoder)
    match &inst.operands[0] {
        Operand::Register(RegOperand::Gpr(idx)) => {
            cpu.regs.write_gpr(*idx, inst.operand_size, inst.prefix.has_rex(), rand_val);
        }
        _ => return Err(VmError::UndefinedOpcode(0xC7)),
    }

    // CF=1 (success), all other arithmetic flags cleared
    cpu.regs.rflags &= !flags::ARITH_MASK;
    cpu.regs.rflags |= flags::CF;

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// XADD: exchange and add.
///
/// Swaps dst and src, then stores (original_dst + original_src) in dst.
/// Flags are updated as for ADD.
pub fn exec_xadd(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = inst.operand_size;
    let dst_val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let src_val = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let sum = dst_val.wrapping_add(src_val) & size.mask();

    // src <- original dst
    write_operand(cpu, inst, &inst.operands[1], dst_val, memory, mmu)?;
    // dst <- sum
    write_operand(cpu, inst, &inst.operands[0], sum, memory, mmu)?;

    let f = flags::flags_add(dst_val, src_val, sum, size);
    flags::update_flags(&mut cpu.regs.rflags, f);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// POPCNT: population count.
///
/// Counts bits set in source and writes count to destination.
pub fn exec_popcnt(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    if inst.rep != crate::instruction::RepPrefix::Rep {
        return Err(VmError::UndefinedOpcode(inst.opcode as u8));
    }

    let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    let src = src & inst.operand_size.mask();
    let count = src.count_ones() as u64;
    write_operand(cpu, inst, &inst.operands[0], count, memory, mmu)?;

    // POPCNT defines: ZF = (src==0), CF/OF/SF/AF/PF cleared.
    cpu.regs.rflags &= !(flags::CF | flags::OF | flags::SF | flags::AF | flags::PF | flags::ZF);
    if src == 0 {
        cpu.regs.rflags |= flags::ZF;
    }
    cpu.regs.rflags |= flags::RFLAGS_FIXED;

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// XLAT: table lookup byte — `AL = [RBX + unsigned AL]`.
pub fn exec_xlat(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let al = cpu.regs.read_gpr8(GprIndex::Rax as u8, false) as u64;
    let bx = cpu.regs.read_gpr64(GprIndex::Rbx as u8);
    let offset = bx.wrapping_add(al);

    // Use DS segment by default (or override)
    let seg = inst.prefix.seg_override.unwrap_or(SegReg::Ds);
    let seg_desc = cpu.regs.segment(seg);
    let linear = seg_desc.base.wrapping_add(offset);

    let val = super::translate_and_read(cpu, linear, OperandSize::Byte, mmu, memory)?;
    cpu.regs.write_gpr8(GprIndex::Rax as u8, false, val as u8);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// MOV to/from segment register (opcodes 8C and 8E).
///
/// 8C: MOV r/m16, Sreg
/// 8E: MOV Sreg, r/m16
pub fn exec_mov_seg(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let op = inst.opcode as u8;

    if op == 0x8C {
        // MOV r/m, Sreg — operand[0] is dst (r/m), operand[1] is src (seg)
        let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
        write_operand(cpu, inst, &inst.operands[0], src, memory, mmu)?;
    } else {
        // MOV Sreg, r/m — operand[0] is dst (seg), operand[1] is src (r/m)
        let selector = read_operand(cpu, inst, &inst.operands[1], memory, mmu)? as u16;
        let seg_reg = match &inst.operands[0] {
            Operand::Register(RegOperand::Seg(sr)) => *sr,
            _ => return Err(VmError::UndefinedOpcode(op)),
        };

        match cpu.mode {
            Mode::RealMode => {
                cpu.regs.load_segment_real(seg_reg, selector);
            }
            Mode::LongMode => {
                // In long mode, CS/DS/ES/SS bases are forced to 0.
                // FS/GS retain their base (set via MSR).
                let desc = cpu.regs.segment_mut(seg_reg);
                desc.selector = selector;
                match seg_reg {
                    SegReg::Fs | SegReg::Gs => {} // keep existing base from MSR
                    _ => desc.base = 0,
                }
            }
            Mode::ProtectedMode => {
                // Load full descriptor from the GDT.
                cpu.load_segment_from_gdt(seg_reg, selector, memory, mmu)?;
                if matches!(seg_reg, SegReg::Cs) {
                    cpu.update_mode();
                }
            }
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// LES/LDS/LSS/LFS/LGS: load a far pointer (seg:offset) from memory.
///
/// Reads `operand_size` bytes for the offset followed by a 16-bit selector
/// from the memory operand. Stores the offset in the GP register (operand 0)
/// and loads the selector into the implicit segment register determined by
/// the opcode.
pub fn exec_load_far_ptr(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    use crate::instruction::OpcodeMap;

    let op = inst.opcode as u8;

    // Determine which segment register to load.
    let seg_reg = match inst.opcode_map {
        OpcodeMap::Primary => match op {
            0xC4 => SegReg::Es,
            0xC5 => SegReg::Ds,
            _ => return Err(VmError::UndefinedOpcode(op)),
        },
        OpcodeMap::Secondary => match op {
            0xB2 => SegReg::Ss,
            0xB4 => SegReg::Fs,
            0xB5 => SegReg::Gs,
            _ => return Err(VmError::UndefinedOpcode(op)),
        },
        _ => return Err(VmError::UndefinedOpcode(op)),
    };

    // operand[1] is the memory source (r/m). Compute its linear address.
    let mem_op = match &inst.operands[1] {
        Operand::Memory(m) => m,
        _ => return Err(VmError::UndefinedOpcode(op)),
    };
    let linear = compute_effective_address(cpu, mem_op, inst)?;
    let off_size = inst.operand_size;

    // Read the offset (16 or 32 bits).
    let offset = translate_and_read(cpu, linear, off_size, mmu, memory)?;

    // Read the selector (16 bits) immediately following the offset.
    let sel_offset = match off_size {
        OperandSize::Word => 2u64,
        OperandSize::Dword => 4u64,
        _ => 4u64,
    };
    let selector = translate_and_read(cpu, linear.wrapping_add(sel_offset), OperandSize::Word, mmu, memory)? as u16;

    // Write the offset to the destination GP register (operand 0).
    write_operand(cpu, inst, &inst.operands[0], offset, memory, mmu)?;

    // Load the segment register.
    match cpu.mode {
        Mode::RealMode => {
            cpu.regs.load_segment_real(seg_reg, selector);
        }
        Mode::ProtectedMode => {
            cpu.load_segment_from_gdt(seg_reg, selector, memory, mmu)?;
        }
        Mode::LongMode => {
            let desc = cpu.regs.segment_mut(seg_reg);
            desc.selector = selector;
            match seg_reg {
                SegReg::Fs | SegReg::Gs => {}
                _ => desc.base = 0,
            }
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// SAHF: store AH into the low byte of FLAGS (SF, ZF, AF, PF, CF).
///
/// Bits 1, 3, 5 of FLAGS are reserved and handled accordingly:
/// bit 1 is always 1, bits 3 and 5 are always 0.
pub fn exec_sahf(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let ah = cpu.regs.read_gpr8(4, false); // AH (index 4 without REX = AH)
    // SAHF loads SF:ZF:0:AF:0:PF:1:CF from AH
    let mask = flags::SF | flags::ZF | flags::AF | flags::PF | flags::CF;
    cpu.regs.rflags = (cpu.regs.rflags & !mask) | ((ah as u64) & mask);
    // Ensure bit 1 is always set
    cpu.regs.rflags |= flags::RFLAGS_FIXED;

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// LAHF: load the low byte of FLAGS into AH.
pub fn exec_lahf(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let flags_byte = (cpu.regs.rflags & 0xFF) as u8;
    cpu.regs.write_gpr8(4, false, flags_byte); // AH
    cpu.regs.rip += inst.length as u64;
    Ok(())
}
