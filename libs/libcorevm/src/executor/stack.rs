//! Stack operation instruction handlers.
//!
//! Implements PUSH (reg/imm/rm/seg), POP (reg/rm/seg), PUSHF/POPF,
//! PUSHA/POPA, ENTER, and LEAVE.

use crate::cpu::{Cpu, Mode};
use crate::error::{Result, VmError};
use crate::flags::{self, OperandSize};
use crate::instruction::{DecodedInst, Operand, RegOperand};
use crate::memory::{GuestMemory, Mmu};
use crate::registers::{GprIndex, SegReg};

use super::{pop_val, push_val, read_operand, stack_operand_size, write_operand};

/// PUSH r16/r32/r64: push a general-purpose register.
pub fn exec_push_reg(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    let val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    push_val(cpu, val, size, mmu, memory)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// POP r16/r32/r64: pop a general-purpose register.
pub fn exec_pop_reg(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    let val = pop_val(cpu, size, mmu, memory)?;
    write_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// PUSH imm8/imm16/imm32: push an immediate value (sign-extended to stack width).
pub fn exec_push_imm(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    // The immediate is already sign-extended by the decoder
    let val = inst.immediate;
    push_val(cpu, val, size, mmu, memory)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// PUSH r/m16-64: push a memory or register operand.
pub fn exec_push_rm(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    let val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    push_val(cpu, val, size, mmu, memory)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// POP r/m16-64: pop to a memory or register operand.
pub fn exec_pop_rm(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    let val = pop_val(cpu, size, mmu, memory)?;
    write_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// PUSH seg: push a segment register selector.
pub fn exec_push_seg(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    let val = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    push_val(cpu, val, size, mmu, memory)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// POP seg: pop a segment register selector.
pub fn exec_pop_seg(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    let selector = pop_val(cpu, size, mmu, memory)? as u16;

    let seg_reg = match &inst.operands[0] {
        Operand::Register(RegOperand::Seg(sr)) => *sr,
        _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    };

    match cpu.mode {
        Mode::RealMode => cpu.regs.load_segment_real(seg_reg, selector),
        _ => {
            // In protected/long mode a full descriptor load is needed.
            // Simplified: update the selector.
            cpu.regs.segment_mut(seg_reg).selector = selector;
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// PUSHF/PUSHFD/PUSHFQ: push FLAGS/EFLAGS/RFLAGS.
///
/// The pushed value has VM and RF cleared.
pub fn exec_pushf(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    // Clear VM and RF in the pushed value
    let mut val = cpu.regs.rflags & !(flags::VM | flags::RF);
    // Mask to push size
    val &= size.mask();
    push_val(cpu, val, size, mmu, memory)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// POPF/POPFD/POPFQ: pop FLAGS/EFLAGS/RFLAGS.
///
/// Respects IOPL for IF modification and CPL for IOPL modification.
pub fn exec_popf(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    let popped = pop_val(cpu, size, mmu, memory)?;

    let mut new_flags = popped;

    // Always set bit 1 (reserved, must be 1)
    new_flags |= flags::RFLAGS_FIXED;

    // In 64-bit mode, clear VM and RF
    if matches!(cpu.mode, Mode::LongMode) {
        new_flags &= !(flags::VM | flags::RF);
    }

    // CPL > 0: cannot change IOPL
    if cpu.regs.cpl > 0 {
        new_flags = (new_flags & !flags::IOPL_MASK) | (cpu.regs.rflags & flags::IOPL_MASK);
    }

    // CPL > IOPL: cannot change IF
    let iopl = ((cpu.regs.rflags & flags::IOPL_MASK) >> flags::IOPL_SHIFT) as u8;
    if cpu.regs.cpl > iopl {
        new_flags = (new_flags & !flags::IF) | (cpu.regs.rflags & flags::IF);
    }

    // Merge: replace bits within the pop size, preserve upper bits
    let pop_mask = size.mask();
    cpu.regs.rflags = (cpu.regs.rflags & !pop_mask) | (new_flags & pop_mask);
    cpu.regs.rflags |= flags::RFLAGS_FIXED;

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// PUSHA/PUSHAD: push all general-purpose registers (16/32-bit mode only).
///
/// Pushes in order: AX/EAX, CX/ECX, DX/EDX, BX/EBX, original SP/ESP,
/// BP/EBP, SI/ESI, DI/EDI.
pub fn exec_pusha(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    if matches!(cpu.mode, Mode::LongMode) {
        return Err(VmError::UndefinedOpcode(0x60));
    }

    let size = inst.operand_size;
    let original_sp = cpu.regs.read_gpr(GprIndex::Rsp as u8, size, false);

    // Order: AX, CX, DX, BX, SP (original), BP, SI, DI
    let regs_order = [
        GprIndex::Rax,
        GprIndex::Rcx,
        GprIndex::Rdx,
        GprIndex::Rbx,
    ];

    for &r in &regs_order {
        let val = cpu.regs.read_gpr(r as u8, size, false);
        push_val(cpu, val, size, mmu, memory)?;
    }

    // Push original SP (before any pushes in this instruction)
    push_val(cpu, original_sp, size, mmu, memory)?;

    let regs_order2 = [GprIndex::Rbp, GprIndex::Rsi, GprIndex::Rdi];

    for &r in &regs_order2 {
        let val = cpu.regs.read_gpr(r as u8, size, false);
        push_val(cpu, val, size, mmu, memory)?;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// POPA/POPAD: pop all general-purpose registers (16/32-bit mode only).
///
/// Pops in reverse order of PUSHA. SP/ESP is discarded (not loaded).
pub fn exec_popa(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    if matches!(cpu.mode, Mode::LongMode) {
        return Err(VmError::UndefinedOpcode(0x61));
    }

    let size = inst.operand_size;

    // Pop order: DI, SI, BP, (skip SP), BX, DX, CX, AX
    let di = pop_val(cpu, size, mmu, memory)?;
    let si = pop_val(cpu, size, mmu, memory)?;
    let bp = pop_val(cpu, size, mmu, memory)?;
    let _sp_discarded = pop_val(cpu, size, mmu, memory)?;
    let bx = pop_val(cpu, size, mmu, memory)?;
    let dx = pop_val(cpu, size, mmu, memory)?;
    let cx = pop_val(cpu, size, mmu, memory)?;
    let ax = pop_val(cpu, size, mmu, memory)?;

    cpu.regs.write_gpr(GprIndex::Rdi as u8, size, false, di);
    cpu.regs.write_gpr(GprIndex::Rsi as u8, size, false, si);
    cpu.regs.write_gpr(GprIndex::Rbp as u8, size, false, bp);
    // SP is NOT restored
    cpu.regs.write_gpr(GprIndex::Rbx as u8, size, false, bx);
    cpu.regs.write_gpr(GprIndex::Rdx as u8, size, false, dx);
    cpu.regs.write_gpr(GprIndex::Rcx as u8, size, false, cx);
    cpu.regs.write_gpr(GprIndex::Rax as u8, size, false, ax);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// ENTER imm16, imm8: create a stack frame.
///
/// - `imm16` is the number of bytes to allocate for local variables.
/// - `imm8` is the nesting level (0 for most code).
///
/// The basic (level=0) behavior is: push RBP, RBP=RSP, RSP-=imm16.
pub fn exec_enter(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let alloc_size = inst.immediate as u16 as u64;
    let nesting = (inst.immediate2 as u8) & 0x1F;
    let size = stack_operand_size(cpu, inst);

    // Push BP/EBP/RBP
    let bp = cpu.regs.read_gpr(GprIndex::Rbp as u8, size, inst.prefix.has_rex());
    push_val(cpu, bp, size, mmu, memory)?;

    let frame_ptr = cpu.regs.sp();

    if nesting > 0 {
        // Nested levels: push previous frame pointers
        for i in 1..nesting as u64 {
            let bytes = size.bytes() as u64;
            let prev_bp = bp.wrapping_sub(i * bytes);
            let val = super::translate_and_read(
                cpu,
                cpu.regs.segment(SegReg::Ss).base.wrapping_add(prev_bp),
                size,
                mmu,
                memory,
            )?;
            push_val(cpu, val, size, mmu, memory)?;
        }
        // Push the new frame pointer
        push_val(cpu, frame_ptr, size, mmu, memory)?;
    }

    // Set BP/EBP/RBP to the frame pointer
    cpu.regs.write_gpr(GprIndex::Rbp as u8, size, inst.prefix.has_rex(), frame_ptr);

    // Allocate space for locals
    let new_sp = cpu.regs.sp().wrapping_sub(alloc_size);
    cpu.regs.set_sp(new_sp);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// LEAVE: destroy stack frame. RSP = RBP, then pop RBP.
pub fn exec_leave(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);

    // RSP/ESP/SP = RBP/EBP/BP
    let bp = cpu.regs.read_gpr(GprIndex::Rbp as u8, size, inst.prefix.has_rex());
    match size {
        OperandSize::Word => cpu.regs.write_gpr16(GprIndex::Rsp as u8, bp as u16),
        OperandSize::Dword => cpu.regs.write_gpr32(GprIndex::Rsp as u8, bp as u32),
        _ => cpu.regs.set_sp(bp),
    }

    // Pop RBP/EBP/BP
    let val = pop_val(cpu, size, mmu, memory)?;
    cpu.regs.write_gpr(GprIndex::Rbp as u8, size, inst.prefix.has_rex(), val);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}
