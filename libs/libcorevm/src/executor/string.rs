//! String operation instruction handlers.
//!
//! Implements MOVS, CMPS, STOS, LODS, SCAS, INS, and OUTS. All string
//! operations respect the DF (direction flag): if DF=0, index registers
//! increment; if DF=1, they decrement.
//!
//! REP/REPE/REPNE prefix handling is built into each function:
//! - REP (MOVS/STOS/LODS/INS/OUTS): repeat while RCX != 0
//! - REPE (CMPS/SCAS): repeat while RCX != 0 AND ZF=1
//! - REPNE (CMPS/SCAS): repeat while RCX != 0 AND ZF=0
//! - Without REP: execute once

use crate::cpu::Cpu;
use crate::error::Result;
use crate::flags::{self, OperandSize};
use crate::instruction::{DecodedInst, RepPrefix};
use crate::io::IoDispatch;
use crate::memory::{GuestMemory, Mmu};
use crate::registers::{GprIndex, SegReg};

use super::{translate_and_read, translate_and_write};

/// Determine the element size for string operations from the opcode.
///
/// Byte variants use the low-bit-clear opcode, word/dword/qword variants
/// use the operand size from the instruction.
fn string_element_size(inst: &DecodedInst) -> OperandSize {
    let op = inst.opcode as u8;
    // Byte variants: A4, A6, AA, AC, AE (even opcodes)
    if (op & 1) == 0 {
        OperandSize::Byte
    } else {
        inst.operand_size
    }
}

/// Compute the index register step (positive or negative) based on DF and element size.
fn step(cpu: &Cpu, elem_size: OperandSize) -> i64 {
    let bytes = elem_size.bytes() as i64;
    if (cpu.regs.rflags & flags::DF) != 0 {
        -bytes
    } else {
        bytes
    }
}

/// Read the counter register (CX/ECX/RCX) at the instruction's address size.
fn read_counter(cpu: &Cpu, inst: &DecodedInst) -> u64 {
    cpu.regs.read_gpr(GprIndex::Rcx as u8, inst.address_size, inst.prefix.has_rex())
}

/// Write the counter register at the instruction's address size.
fn write_counter(cpu: &mut Cpu, inst: &DecodedInst, val: u64) {
    cpu.regs.write_gpr(
        GprIndex::Rcx as u8,
        inst.address_size,
        inst.prefix.has_rex(),
        val,
    );
}

/// Read RSI/ESI/SI at the instruction's address size.
fn read_si(cpu: &Cpu, inst: &DecodedInst) -> u64 {
    cpu.regs.read_gpr(GprIndex::Rsi as u8, inst.address_size, inst.prefix.has_rex())
}

/// Write RSI/ESI/SI at the instruction's address size.
fn write_si(cpu: &mut Cpu, inst: &DecodedInst, val: u64) {
    cpu.regs.write_gpr(
        GprIndex::Rsi as u8,
        inst.address_size,
        inst.prefix.has_rex(),
        val,
    );
}

/// Read RDI/EDI/DI at the instruction's address size.
fn read_di(cpu: &Cpu, inst: &DecodedInst) -> u64 {
    cpu.regs.read_gpr(GprIndex::Rdi as u8, inst.address_size, inst.prefix.has_rex())
}

/// Write RDI/EDI/DI at the instruction's address size.
fn write_di(cpu: &mut Cpu, inst: &DecodedInst, val: u64) {
    cpu.regs.write_gpr(
        GprIndex::Rdi as u8,
        inst.address_size,
        inst.prefix.has_rex(),
        val,
    );
}

/// Compute the linear address for DS:[RSI] with optional segment override.
fn src_linear(cpu: &Cpu, inst: &DecodedInst) -> u64 {
    let seg = inst.prefix.seg_override.unwrap_or(SegReg::Ds);
    let base = cpu.regs.segment(seg).base;
    base.wrapping_add(read_si(cpu, inst))
}

/// Compute the linear address for ES:[RDI] (always ES, no override allowed).
fn dst_linear(cpu: &Cpu, inst: &DecodedInst) -> u64 {
    let base = cpu.regs.segment(SegReg::Es).base;
    base.wrapping_add(read_di(cpu, inst))
}

/// MOVS: copy from DS:[RSI] to ES:[RDI].
///
/// REP prefix: repeat while RCX != 0.
pub fn exec_movs(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let elem = string_element_size(inst);
    let delta = step(cpu, elem);

    if inst.rep == RepPrefix::Rep {
        // REP MOVS
        loop {
            let count = read_counter(cpu, inst);
            if count == 0 {
                break;
            }

            let s = src_linear(cpu, inst);
            let d = dst_linear(cpu, inst);
            let val = translate_and_read(cpu, s, elem, mmu, memory)?;
            translate_and_write(cpu, d, elem, val, mmu, memory)?;

            write_si(cpu, inst, read_si(cpu, inst).wrapping_add(delta as u64));
            write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
            write_counter(cpu, inst, count - 1);
        }
    } else {
        // Single MOVS
        let s = src_linear(cpu, inst);
        let d = dst_linear(cpu, inst);
        let val = translate_and_read(cpu, s, elem, mmu, memory)?;
        translate_and_write(cpu, d, elem, val, mmu, memory)?;

        write_si(cpu, inst, read_si(cpu, inst).wrapping_add(delta as u64));
        write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// CMPS: compare DS:[RSI] with ES:[RDI], update flags.
///
/// REPE prefix: repeat while RCX != 0 AND ZF=1.
/// REPNE prefix: repeat while RCX != 0 AND ZF=0.
pub fn exec_cmps(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let elem = string_element_size(inst);
    let delta = step(cpu, elem);

    match inst.rep {
        RepPrefix::Rep => {
            // REPE CMPS
            loop {
                let count = read_counter(cpu, inst);
                if count == 0 {
                    break;
                }

                let s = src_linear(cpu, inst);
                let d = dst_linear(cpu, inst);
                let src_val = translate_and_read(cpu, s, elem, mmu, memory)?;
                let dst_val = translate_and_read(cpu, d, elem, mmu, memory)?;

                let result = src_val.wrapping_sub(dst_val) & elem.mask();
                let f = flags::flags_sub(src_val, dst_val, result, elem);
                flags::update_flags(&mut cpu.regs.rflags, f);

                write_si(cpu, inst, read_si(cpu, inst).wrapping_add(delta as u64));
                write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
                write_counter(cpu, inst, count - 1);

                // REPE: stop if ZF=0 (mismatch found)
                if (cpu.regs.rflags & flags::ZF) == 0 {
                    break;
                }
            }
        }
        RepPrefix::Repne => {
            // REPNE CMPS
            loop {
                let count = read_counter(cpu, inst);
                if count == 0 {
                    break;
                }

                let s = src_linear(cpu, inst);
                let d = dst_linear(cpu, inst);
                let src_val = translate_and_read(cpu, s, elem, mmu, memory)?;
                let dst_val = translate_and_read(cpu, d, elem, mmu, memory)?;

                let result = src_val.wrapping_sub(dst_val) & elem.mask();
                let f = flags::flags_sub(src_val, dst_val, result, elem);
                flags::update_flags(&mut cpu.regs.rflags, f);

                write_si(cpu, inst, read_si(cpu, inst).wrapping_add(delta as u64));
                write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
                write_counter(cpu, inst, count - 1);

                // REPNE: stop if ZF=1 (match found)
                if (cpu.regs.rflags & flags::ZF) != 0 {
                    break;
                }
            }
        }
        RepPrefix::None => {
            // Single CMPS
            let s = src_linear(cpu, inst);
            let d = dst_linear(cpu, inst);
            let src_val = translate_and_read(cpu, s, elem, mmu, memory)?;
            let dst_val = translate_and_read(cpu, d, elem, mmu, memory)?;

            let result = src_val.wrapping_sub(dst_val) & elem.mask();
            let f = flags::flags_sub(src_val, dst_val, result, elem);
            flags::update_flags(&mut cpu.regs.rflags, f);

            write_si(cpu, inst, read_si(cpu, inst).wrapping_add(delta as u64));
            write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// STOS: store AL/AX/EAX/RAX to ES:[RDI].
///
/// REP prefix: repeat while RCX != 0.
pub fn exec_stos(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let elem = string_element_size(inst);
    let delta = step(cpu, elem);
    let acc = cpu.regs.read_gpr(GprIndex::Rax as u8, elem, inst.prefix.has_rex());

    if inst.rep == RepPrefix::Rep {
        loop {
            let count = read_counter(cpu, inst);
            if count == 0 {
                break;
            }

            let d = dst_linear(cpu, inst);
            translate_and_write(cpu, d, elem, acc, mmu, memory)?;

            write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
            write_counter(cpu, inst, count - 1);
        }
    } else {
        let d = dst_linear(cpu, inst);
        translate_and_write(cpu, d, elem, acc, mmu, memory)?;

        write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// LODS: load from DS:[RSI] into AL/AX/EAX/RAX.
///
/// REP prefix: repeat while RCX != 0 (rarely used but valid).
pub fn exec_lods(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let elem = string_element_size(inst);
    let delta = step(cpu, elem);

    if inst.rep == RepPrefix::Rep {
        loop {
            let count = read_counter(cpu, inst);
            if count == 0 {
                break;
            }

            let s = src_linear(cpu, inst);
            let val = translate_and_read(cpu, s, elem, mmu, memory)?;
            cpu.regs.write_gpr(GprIndex::Rax as u8, elem, inst.prefix.has_rex(), val);

            write_si(cpu, inst, read_si(cpu, inst).wrapping_add(delta as u64));
            write_counter(cpu, inst, count - 1);
        }
    } else {
        let s = src_linear(cpu, inst);
        let val = translate_and_read(cpu, s, elem, mmu, memory)?;
        cpu.regs.write_gpr(GprIndex::Rax as u8, elem, inst.prefix.has_rex(), val);

        write_si(cpu, inst, read_si(cpu, inst).wrapping_add(delta as u64));
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// SCAS: compare AL/AX/EAX/RAX with ES:[RDI], update flags.
///
/// REPE prefix: repeat while RCX != 0 AND ZF=1.
/// REPNE prefix: repeat while RCX != 0 AND ZF=0.
pub fn exec_scas(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let elem = string_element_size(inst);
    let delta = step(cpu, elem);
    let acc = cpu.regs.read_gpr(GprIndex::Rax as u8, elem, inst.prefix.has_rex());

    match inst.rep {
        RepPrefix::Rep => {
            // REPE SCAS
            loop {
                let count = read_counter(cpu, inst);
                if count == 0 {
                    break;
                }

                let d = dst_linear(cpu, inst);
                let mem_val = translate_and_read(cpu, d, elem, mmu, memory)?;

                let result = acc.wrapping_sub(mem_val) & elem.mask();
                let f = flags::flags_sub(acc, mem_val, result, elem);
                flags::update_flags(&mut cpu.regs.rflags, f);

                write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
                write_counter(cpu, inst, count - 1);

                if (cpu.regs.rflags & flags::ZF) == 0 {
                    break;
                }
            }
        }
        RepPrefix::Repne => {
            // REPNE SCAS
            loop {
                let count = read_counter(cpu, inst);
                if count == 0 {
                    break;
                }

                let d = dst_linear(cpu, inst);
                let mem_val = translate_and_read(cpu, d, elem, mmu, memory)?;

                let result = acc.wrapping_sub(mem_val) & elem.mask();
                let f = flags::flags_sub(acc, mem_val, result, elem);
                flags::update_flags(&mut cpu.regs.rflags, f);

                write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
                write_counter(cpu, inst, count - 1);

                if (cpu.regs.rflags & flags::ZF) != 0 {
                    break;
                }
            }
        }
        RepPrefix::None => {
            // Single SCAS
            let d = dst_linear(cpu, inst);
            let mem_val = translate_and_read(cpu, d, elem, mmu, memory)?;

            let result = acc.wrapping_sub(mem_val) & elem.mask();
            let f = flags::flags_sub(acc, mem_val, result, elem);
            flags::update_flags(&mut cpu.regs.rflags, f);

            write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// Determine element size for INS/OUTS from the opcode.
///
/// 0x6C/0x6E = byte, 0x6D/0x6F = word/dword per operand size.
fn ins_outs_element_size(inst: &DecodedInst) -> OperandSize {
    let op = inst.opcode as u8;
    if (op & 1) == 0 {
        OperandSize::Byte
    } else {
        inst.operand_size
    }
}

/// INS: read from I/O port DX, write to ES:[RDI].
///
/// REP prefix: repeat while RCX != 0.
pub fn exec_ins(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
    io: &mut IoDispatch,
) -> Result<()> {
    let elem = ins_outs_element_size(inst);
    let delta = step(cpu, elem);
    let port = cpu.regs.read_gpr16(GprIndex::Rdx as u8);
    let size_io = elem.bytes() as u8;
    // Cap at 4 bytes for I/O port access.
    let size_io = if size_io > 4 { 4 } else { size_io };

    if inst.rep == RepPrefix::Rep {
        loop {
            let count = read_counter(cpu, inst);
            if count == 0 {
                break;
            }

            let val = io.port_in(port, size_io)? as u64;
            let d = dst_linear(cpu, inst);
            translate_and_write(cpu, d, elem, val, mmu, memory)?;

            write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
            write_counter(cpu, inst, count - 1);
        }
    } else {
        let val = io.port_in(port, size_io)? as u64;
        let d = dst_linear(cpu, inst);
        translate_and_write(cpu, d, elem, val, mmu, memory)?;

        write_di(cpu, inst, read_di(cpu, inst).wrapping_add(delta as u64));
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// OUTS: read from DS:[RSI], write to I/O port DX.
///
/// REP prefix: repeat while RCX != 0.
pub fn exec_outs(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
    io: &mut IoDispatch,
) -> Result<()> {
    let elem = ins_outs_element_size(inst);
    let delta = step(cpu, elem);
    let port = cpu.regs.read_gpr16(GprIndex::Rdx as u8);
    let size_io = elem.bytes() as u8;
    let size_io = if size_io > 4 { 4 } else { size_io };

    if inst.rep == RepPrefix::Rep {
        loop {
            let count = read_counter(cpu, inst);
            if count == 0 {
                break;
            }

            let s = src_linear(cpu, inst);
            let val = translate_and_read(cpu, s, elem, mmu, memory)?;
            io.port_out(port, size_io, val as u32)?;

            write_si(cpu, inst, read_si(cpu, inst).wrapping_add(delta as u64));
            write_counter(cpu, inst, count - 1);
        }
    } else {
        let s = src_linear(cpu, inst);
        let val = translate_and_read(cpu, s, elem, mmu, memory)?;
        io.port_out(port, size_io, val as u32)?;

        write_si(cpu, inst, read_si(cpu, inst).wrapping_add(delta as u64));
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}
