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

use super::{compute_effective_address, read_operand, write_operand};

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
    // Source is smaller than destination; read at source size, zero-extend implicitly
    let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
    // The value is already zero-extended by read_operand (read_gpr8/read_gpr16 return u64)
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
    let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;

    // Determine source size from the source operand
    let src_size = match &inst.operands[1] {
        Operand::Memory(mem_op) => mem_op.size,
        _ => {
            // For register source, infer from opcode:
            // 0FBE = r/m8 -> r, 0FBF = r/m16 -> r
            let op2 = inst.opcode as u8;
            if op2 == 0xBE { OperandSize::Byte } else { OperandSize::Word }
        }
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
    let src = read_operand(cpu, inst, &inst.operands[1], memory, mmu)?;
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
