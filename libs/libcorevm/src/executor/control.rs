//! Control flow instruction handlers.
//!
//! Implements JMP (rel/rm/far), Jcc, CALL (rel/rm/far), RET (near/far),
//! LOOP/LOOPE/LOOPNE, JCXZ/JECXZ/JRCXZ, INT, INT3, INTO, and IRET.

use crate::cpu::{Cpu, Mode};
use crate::error::{Result, VmError};
use crate::flags::{self, OperandSize};
use crate::instruction::{DecodedInst, Operand};
use crate::interrupts::InterruptController;
use crate::memory::{GuestMemory, Mmu};
use crate::registers::{GprIndex, SegReg};

use super::{pop_val, push_val, read_operand, stack_operand_size};

// ── JMP ──

/// JMP rel8/rel16/rel32: relative jump.
///
/// `RIP = RIP + inst.length + signed_offset`. Does NOT modify flags.
pub fn exec_jmp_rel(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let offset = match inst.operands[0] {
        Operand::RelativeOffset(off) => off,
        Operand::Immediate(imm) => imm as i64,
        _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    };

    let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);
    let target = next_rip.wrapping_add(offset as u64);

    cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), target);
    Ok(())
}

/// JMP r/m: indirect jump (RIP = operand value).
pub fn exec_jmp_rm(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let target = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), target);
    Ok(())
}

/// JMP far: load new CS:RIP from a far pointer.
pub fn exec_jmp_far(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let (segment, offset) = read_far_pointer(cpu, inst, memory, mmu)?;
    load_cs(cpu, segment, memory, mmu)?;
    cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), offset);
    Ok(())
}

// ── Jcc ──

/// Jcc: conditional jump. If condition is met, RIP += signed offset.
///
/// The condition code is encoded in the low 4 bits of the opcode (for both
/// short 0x7x and near 0x0F 8x forms).
pub fn exec_jcc(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let cc = (inst.opcode as u8) & 0x0F;
    let condition_met = flags::eval_cc(cc, cpu.regs.rflags);

    let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);

    if condition_met {
        let offset = match inst.operands[0] {
            Operand::RelativeOffset(off) => off,
            Operand::Immediate(imm) => imm as i64,
            _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
        };
        let target = next_rip.wrapping_add(offset as u64);
        cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), target);
    } else {
        cpu.regs.rip = next_rip;
    }

    Ok(())
}

// ── CALL ──

/// CALL rel16/rel32: push return address, then jump to relative target.
pub fn exec_call_rel(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let offset = match inst.operands[0] {
        Operand::RelativeOffset(off) => off,
        Operand::Immediate(imm) => imm as i64,
        _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    };

    let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);
    let size = stack_operand_size(cpu, inst);

    push_val(cpu, next_rip, size, mmu, memory)?;

    let target = next_rip.wrapping_add(offset as u64);
    cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), target);
    Ok(())
}

/// CALL r/m: push return address, then jump to operand value.
pub fn exec_call_rm(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let target = read_operand(cpu, inst, &inst.operands[0], memory, mmu)?;
    let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);
    let size = stack_operand_size(cpu, inst);

    push_val(cpu, next_rip, size, mmu, memory)?;
    cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), target);
    Ok(())
}

/// CALL far: push CS:RIP, then load new CS:RIP from far pointer.
pub fn exec_call_far(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let (new_seg, new_offset) = read_far_pointer(cpu, inst, memory, mmu)?;
    let offset_size = control_offset_size(cpu, inst);
    let stack_size = stack_operand_size(cpu, inst);
    let old_cs = cpu.regs.segment(SegReg::Cs).selector as u64;
    let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);

    if matches!(cpu.mode, Mode::RealMode) {
        push_val(cpu, old_cs, OperandSize::Word, mmu, memory)?;
        push_val(cpu, next_rip, offset_size, mmu, memory)?;
    } else {
        push_val(cpu, old_cs, stack_size, mmu, memory)?;
        push_val(cpu, next_rip, stack_size, mmu, memory)?;
    }

    load_cs(cpu, new_seg, memory, mmu)?;
    cpu.regs.rip = mask_control_offset(cpu, offset_size, new_offset);
    Ok(())
}

// ── RET ──

/// RET near: pop RIP from stack.
pub fn exec_ret_near(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    let rip = pop_val(cpu, size, mmu, memory)?;
    cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), rip);
    Ok(())
}

/// RET near imm16: pop RIP, then add imm16 to RSP.
pub fn exec_ret_near_imm(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let size = stack_operand_size(cpu, inst);
    let rip = pop_val(cpu, size, mmu, memory)?;
    let imm = inst.immediate as u64;

    // Add immediate to stack pointer
    let sp = cpu.regs.sp().wrapping_add(imm);
    cpu.regs.set_sp(sp);

    cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), rip);
    Ok(())
}

/// RETF: pop RIP and CS from stack.
///
/// For RETF with imm16 (opcode 0xCA), the immediate is added to RSP after
/// popping. For RETF without immediate (opcode 0xCB), nothing extra is done.
pub fn exec_ret_far(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let offset_size = control_offset_size(cpu, inst);
    let stack_size = stack_operand_size(cpu, inst);

    let rip = pop_val(cpu, offset_size, mmu, memory)?;
    let cs = if matches!(cpu.mode, Mode::RealMode) {
        pop_val(cpu, OperandSize::Word, mmu, memory)? as u16
    } else {
        pop_val(cpu, stack_size, mmu, memory)? as u16
    };

    // If opcode is 0xCA, add immediate to RSP
    if inst.opcode as u8 == 0xCA {
        let imm = inst.immediate as u64;
        let sp = cpu.regs.sp().wrapping_add(imm);
        cpu.regs.set_sp(sp);
    }

    load_cs(cpu, cs, memory, mmu)?;
    cpu.regs.rip = mask_control_offset(cpu, offset_size, rip);
    Ok(())
}

// ── LOOP ──

/// LOOP/LOOPE/LOOPNE: decrement counter register, conditional jump.
///
/// - 0xE2 = LOOP (jump if CX/ECX/RCX != 0)
/// - 0xE1 = LOOPE/LOOPZ (jump if CX != 0 AND ZF=1)
/// - 0xE0 = LOOPNE/LOOPNZ (jump if CX != 0 AND ZF=0)
pub fn exec_loop(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let op = inst.opcode as u8;
    let addr_size = inst.address_size;

    // Decrement counter
    let counter_idx = GprIndex::Rcx as u8;
    let count = cpu.regs.read_gpr(counter_idx, addr_size, inst.prefix.has_rex());
    let new_count = count.wrapping_sub(1) & addr_size.mask();
    cpu.regs.write_gpr(counter_idx, addr_size, inst.prefix.has_rex(), new_count);

    let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);

    let should_jump = match op {
        0xE2 => new_count != 0,                                        // LOOP
        0xE1 => new_count != 0 && (cpu.regs.rflags & flags::ZF) != 0, // LOOPE
        0xE0 => new_count != 0 && (cpu.regs.rflags & flags::ZF) == 0, // LOOPNE
        _ => false,
    };

    if should_jump {
        let offset = match inst.operands[0] {
            Operand::RelativeOffset(off) => off,
            Operand::Immediate(imm) => imm as i64,
            _ => return Err(VmError::UndefinedOpcode(op)),
        };
        let target = next_rip.wrapping_add(offset as u64);
        cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), target);
    } else {
        cpu.regs.rip = next_rip;
    }

    Ok(())
}

/// JCXZ/JECXZ/JRCXZ: jump if counter register is zero.
pub fn exec_jcxz(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let addr_size = inst.address_size;
    let counter = cpu.regs.read_gpr(GprIndex::Rcx as u8, addr_size, inst.prefix.has_rex());
    let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);

    if counter == 0 {
        let offset = match inst.operands[0] {
            Operand::RelativeOffset(off) => off,
            Operand::Immediate(imm) => imm as i64,
            _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
        };
        let target = next_rip.wrapping_add(offset as u64);
        cpu.regs.rip = mask_control_offset(cpu, control_offset_size(cpu, inst), target);
    } else {
        cpu.regs.rip = next_rip;
    }

    Ok(())
}

// ── Interrupts ──

/// INT n: software interrupt.
///
/// Pushes FLAGS, CS, and RIP onto the stack, clears IF and TF, then vectors
/// through the IDT/IVT to the handler for vector `n`.
pub fn exec_int(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &mut Mmu,
    interrupts: &mut InterruptController,
) -> Result<()> {
    let vector = inst.immediate as u8;
    dispatch_software_interrupt(cpu, inst, vector, memory, mmu, interrupts)
}

/// INT 3: breakpoint interrupt (vector 3).
pub fn exec_int3(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &mut Mmu,
    interrupts: &mut InterruptController,
) -> Result<()> {
    dispatch_software_interrupt(cpu, inst, 3, memory, mmu, interrupts)
}

/// INTO: interrupt if OF=1 (vector 4).
///
/// In 64-bit mode, INTO is invalid (#UD).
pub fn exec_into(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &mut Mmu,
    interrupts: &mut InterruptController,
) -> Result<()> {
    if matches!(cpu.mode, Mode::LongMode) {
        return Err(VmError::UndefinedOpcode(0xCE));
    }

    if (cpu.regs.rflags & flags::OF) != 0 {
        dispatch_software_interrupt(cpu, inst, 4, memory, mmu, interrupts)
    } else {
        cpu.regs.rip += inst.length as u64;
        Ok(())
    }
}

/// Common software interrupt dispatch.
///
/// Pushes the appropriate state for the current CPU mode and jumps to the
/// interrupt handler. In real mode, uses the IVT at address 0. In protected
/// and long mode, reads the IDT gate descriptor.
fn dispatch_software_interrupt(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    vector: u8,
    memory: &mut GuestMemory,
    mmu: &mut Mmu,
    interrupts: &mut InterruptController,
) -> Result<()> {
    let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);

    match cpu.mode {
        Mode::RealMode => {
            // Real mode: push FLAGS (16-bit), CS, IP; load from IVT
            let rflags16 = cpu.regs.rflags as u16;
            let cs = cpu.regs.segment(SegReg::Cs).selector;
            let ip = next_rip as u16;

            push_val(cpu, rflags16 as u64, OperandSize::Word, mmu, memory)?;
            push_val(cpu, cs as u64, OperandSize::Word, mmu, memory)?;
            push_val(cpu, ip as u64, OperandSize::Word, mmu, memory)?;

            // Clear IF and TF
            cpu.regs.rflags &= !(flags::IF | flags::TF);

            let (seg, off) = interrupts.read_idt_entry_real(vector, memory)?;
            cpu.regs.load_segment_real(SegReg::Cs, seg);
            cpu.regs.rip = off as u64;
        }
        Mode::ProtectedMode => {
            // In V86 mode, software INTs trap to the monitor if IOPL < 3.
            if (cpu.regs.rflags & flags::VM) != 0 {
                let iopl = ((cpu.regs.rflags & flags::IOPL_MASK) >> flags::IOPL_SHIFT) as u8;
                if iopl < 3 {
                    return Err(VmError::GeneralProtection(0));
                }
                // IOPL=3: deliver through IDT with full V86 frame.
                cpu.regs.rip = next_rip;
                cpu.deliver_interrupt(vector, false, None, memory, mmu, interrupts)?;
                return Ok(());
            }

            let entry = interrupts.read_idt_entry_protected(
                vector,
                cpu.regs.idtr.base,
                cpu.regs.idtr.limit,
                cpu.regs.cr3,
                cpu.regs.cpl,
                mmu,
                memory,
            )?;

            if !entry.present {
                return Err(VmError::SegmentNotPresent(vector as u32 * 8 + 2));
            }

            let size = OperandSize::Dword;
            let eflags = cpu.regs.rflags as u32;
            let cs = cpu.regs.segment(SegReg::Cs).selector;
            let eip = next_rip as u32;

            push_val(cpu, eflags as u64, size, mmu, memory)?;
            push_val(cpu, cs as u64, size, mmu, memory)?;
            push_val(cpu, eip as u64, size, mmu, memory)?;

            // Clear IF for interrupt gates, keep for trap gates
            cpu.regs.rflags &= !flags::TF;
            match entry.gate_type {
                crate::interrupts::GateType::Interrupt16
                | crate::interrupts::GateType::Interrupt32 => {
                    cpu.regs.rflags &= !flags::IF;
                }
                _ => {}
            }

            // Load CS from GDT using the gate's selector.
            cpu.load_segment_from_gdt(SegReg::Cs, entry.selector, memory, mmu)?;
            cpu.update_mode();
            cpu.regs.rip = entry.offset;
        }
        Mode::LongMode => {
            let entry = interrupts.read_idt_entry_long(
                vector,
                cpu.regs.idtr.base,
                cpu.regs.idtr.limit,
                cpu.regs.cr3,
                cpu.regs.cpl,
                mmu,
                memory,
            )?;

            if !entry.present {
                return Err(VmError::SegmentNotPresent(vector as u32 * 16 + 2));
            }

            let size = OperandSize::Qword;
            let ss = cpu.regs.segment(SegReg::Ss).selector as u64;
            let rsp = cpu.regs.sp();

            // In 64-bit mode: push SS, RSP, RFLAGS, CS, RIP
            push_val(cpu, ss, size, mmu, memory)?;
            push_val(cpu, rsp, size, mmu, memory)?;
            push_val(cpu, cpu.regs.rflags, size, mmu, memory)?;
            push_val(
                cpu,
                cpu.regs.segment(SegReg::Cs).selector as u64,
                size,
                mmu,
                memory,
            )?;
            push_val(cpu, next_rip, size, mmu, memory)?;

            cpu.regs.rflags &= !flags::TF;
            match entry.gate_type {
                crate::interrupts::GateType::Interrupt64 => {
                    cpu.regs.rflags &= !flags::IF;
                }
                _ => {}
            }

            cpu.load_segment_from_gdt(SegReg::Cs, entry.selector, memory, mmu)?;
            cpu.update_mode();
            cpu.regs.rip = entry.offset;
        }
    }

    Ok(())
}

/// IRET/IRETD/IRETQ: return from interrupt.
///
/// Restores RIP, CS, and FLAGS from the stack. In 64-bit mode, also
/// restores RSP and SS.
pub fn exec_iret(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    match cpu.mode {
        Mode::RealMode => {
            let size = if inst.prefix.operand_size_override {
                OperandSize::Dword
            } else {
                OperandSize::Word
            };
            let ip = pop_val(cpu, size, mmu, memory)?;
            let cs = pop_val(cpu, size, mmu, memory)? as u16;
            let flags_val = pop_val(cpu, size, mmu, memory)?;

            cpu.regs.load_segment_real(SegReg::Cs, cs);
            cpu.regs.rip = if size == OperandSize::Word {
                ip & 0xFFFF
            } else {
                ip & 0xFFFF_FFFF
            };
            cpu.regs.rflags = if size == OperandSize::Word {
                // Preserve upper bits of RFLAGS, replace low 16 bits.
                (cpu.regs.rflags & !0xFFFF) | (flags_val & 0xFFFF)
            } else {
                // IRETD restores the low 32 bits of EFLAGS in real mode.
                (cpu.regs.rflags & !0xFFFF_FFFF) | (flags_val & 0xFFFF_FFFF)
            };
            cpu.regs.rflags |= flags::RFLAGS_FIXED;
        }
        Mode::ProtectedMode => {
            // IRET while in V86 mode: behave like real-mode IRET.
            if (cpu.regs.rflags & flags::VM) != 0 {
                let iopl = ((cpu.regs.rflags & flags::IOPL_MASK) >> flags::IOPL_SHIFT) as u8;
                if iopl < 3 {
                    return Err(VmError::GeneralProtection(0));
                }
                // V86 IRET with IOPL=3: pop IP, CS, FLAGS (16-bit).
                let ip = pop_val(cpu, OperandSize::Word, mmu, memory)?;
                let cs = pop_val(cpu, OperandSize::Word, mmu, memory)? as u16;
                let flags_val = pop_val(cpu, OperandSize::Word, mmu, memory)?;

                cpu.regs.load_segment_real(SegReg::Cs, cs);
                cpu.regs.rip = ip & 0xFFFF;
                // Restore lower 16 bits of FLAGS but preserve VM, IOPL.
                let preserve = flags::VM | flags::IOPL_MASK;
                let new_flags = (cpu.regs.rflags & !(0xFFFF & !preserve))
                    | (flags_val & (0xFFFF & !preserve));
                cpu.regs.rflags = new_flags | flags::RFLAGS_FIXED;
                return Ok(());
            }

            let size = stack_operand_size(cpu, inst);
            let is_16 = size == OperandSize::Word;
            let ip = pop_val(cpu, size, mmu, memory)?;
            let cs = pop_val(cpu, size, mmu, memory)? as u16;
            let flags_val = pop_val(cpu, size, mmu, memory)?;
            let old_cpl = cpu.regs.cpl;
            let new_cpl = (cs & 0x3) as u8;
            let old_flags = cpu.regs.rflags;

            // V86 return: when CPL=0, 32-bit operand, and popped EFLAGS has VM=1,
            // switch to Virtual 8086 mode. Pop ESP, SS, ES, DS, FS, GS and load
            // all segments as real-mode segments.
            if old_cpl == 0 && !is_16 && (flags_val & flags::VM) != 0 {
                let new_esp = pop_val(cpu, size, mmu, memory)?;
                let new_ss = pop_val(cpu, size, mmu, memory)? as u16;
                let new_es = pop_val(cpu, size, mmu, memory)? as u16;
                let new_ds = pop_val(cpu, size, mmu, memory)? as u16;
                let new_fs = pop_val(cpu, size, mmu, memory)? as u16;
                let new_gs = pop_val(cpu, size, mmu, memory)? as u16;

                cpu.regs.rip = ip & 0xFFFF;
                cpu.regs.rflags = (flags_val & 0xFFFF_FFFF) | flags::RFLAGS_FIXED;
                cpu.regs.set_sp(new_esp & 0xFFFF);
                cpu.regs.load_segment_real(SegReg::Cs, cs);
                cpu.regs.load_segment_real(SegReg::Ss, new_ss);
                cpu.regs.load_segment_real(SegReg::Es, new_es);
                cpu.regs.load_segment_real(SegReg::Ds, new_ds);
                cpu.regs.load_segment_real(SegReg::Fs, new_fs);
                cpu.regs.load_segment_real(SegReg::Gs, new_gs);
                cpu.regs.cpl = 3;
                cpu.mode = Mode::ProtectedMode; // still PM, but VM flag set
                cpu.update_mode();
            } else {
                cpu.load_segment_from_gdt(SegReg::Cs, cs, memory, mmu)?;
                cpu.update_mode();
                cpu.regs.rip = if is_16 { ip & 0xFFFF } else { ip & 0xFFFF_FFFF };

                // Restore EFLAGS (preserve IOPL and IF based on CPL)
                let mut new_flags = if is_16 {
                    (cpu.regs.rflags & !0xFFFF) | (flags_val & 0xFFFF)
                } else {
                    flags_val & 0xFFFF_FFFF
                };
                new_flags |= flags::RFLAGS_FIXED;
                // Intel SDM: IRET clears RF in protected mode.
                new_flags &= !flags::RF;
                if new_cpl > 0 {
                    // CPL > 0 cannot modify IOPL.
                    new_flags = (new_flags & !flags::IOPL_MASK) | (old_flags & flags::IOPL_MASK);
                    let iopl = ((old_flags & flags::IOPL_MASK) >> flags::IOPL_SHIFT) as u8;
                    if new_cpl > iopl {
                        // CPL > IOPL cannot modify IF.
                        new_flags = (new_flags & !flags::IF) | (old_flags & flags::IF);
                    }
                }
                cpu.regs.rflags = new_flags;

                // Outer-privilege return pops ESP and SS.
                if new_cpl > old_cpl {
                    let sp = pop_val(cpu, size, mmu, memory)?;
                    let ss = pop_val(cpu, size, mmu, memory)? as u16;
                    if is_16 {
                        cpu.regs.set_sp(sp & 0xFFFF);
                    } else {
                        cpu.regs.set_sp(sp & 0xFFFF_FFFF);
                    }
                    cpu.load_segment_from_gdt(SegReg::Ss, ss, memory, mmu)?;
                }
                cpu.regs.cpl = new_cpl;
            }
        }
        Mode::LongMode => {
            let size = OperandSize::Qword;
            let rip = pop_val(cpu, size, mmu, memory)?;
            let cs = pop_val(cpu, size, mmu, memory)? as u16;
            let rflags = pop_val(cpu, size, mmu, memory)?;
            let rsp = pop_val(cpu, size, mmu, memory)?;
            let ss = pop_val(cpu, size, mmu, memory)? as u16;

            cpu.load_segment_from_gdt(SegReg::Cs, cs, memory, mmu)?;
            cpu.update_mode();
            cpu.regs.rip = rip;

            let mut new_flags = rflags;
            new_flags |= flags::RFLAGS_FIXED;
            // Clear VM and RF on IRETQ
            new_flags &= !(flags::VM | flags::RF);
            cpu.regs.rflags = new_flags;

            cpu.regs.set_sp(rsp);
            cpu.load_segment_from_gdt(SegReg::Ss, ss, memory, mmu)?;
        }
    }

    Ok(())
}

// ── Helpers ──

/// Determine the code offset width used by near/far control transfers.
fn control_offset_size(cpu: &Cpu, inst: &DecodedInst) -> OperandSize {
    match cpu.mode {
        Mode::LongMode => OperandSize::Qword,
        _ => inst.operand_size,
    }
}

/// Mask a control-transfer target to the active IP/EIP width.
fn mask_control_offset(cpu: &Cpu, size: OperandSize, rip: u64) -> u64 {
    match cpu.mode {
        Mode::RealMode | Mode::ProtectedMode => match size {
            OperandSize::Word => rip & 0xFFFF,
            OperandSize::Dword => rip & 0xFFFF_FFFF,
            OperandSize::Byte | OperandSize::Qword => rip,
        },
        Mode::LongMode => rip, // Full 64-bit
    }
}

/// Read a far pointer (segment:offset) from the instruction operands.
fn read_far_pointer(
    cpu: &Cpu,
    inst: &DecodedInst,
    memory: &GuestMemory,
    mmu: &Mmu,
) -> Result<(u16, u64)> {
    match inst.operands[0] {
        Operand::FarPointer { segment, offset } => Ok((segment, offset)),
        Operand::Memory(ref mem_op) => {
            // Read offset followed by segment from memory
            let linear = super::compute_effective_address(cpu, mem_op, inst)?;
            let offset_size = inst.operand_size;
            let offset = super::translate_and_read(cpu, linear, offset_size, mmu, memory)?;
            let seg_addr = linear.wrapping_add(offset_size.bytes() as u64);
            let segment = super::translate_and_read(cpu, seg_addr, OperandSize::Word, mmu, memory)? as u16;
            Ok((segment, offset))
        }
        _ => Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }
}

/// Load a new CS selector from the GDT and update the CPU mode.
///
/// In real mode, sets base = selector << 4 (the real-mode convention).
/// In protected/long mode, reads the full segment descriptor from the GDT
/// and caches it in the segment register, then updates the decoder mode
/// to reflect any changes in the D/B or L flags.
fn load_cs(
    cpu: &mut Cpu,
    selector: u16,
    memory: &GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    match cpu.mode {
        Mode::RealMode => {
            cpu.regs.load_segment_real(SegReg::Cs, selector);
        }
        _ => {
            cpu.load_segment_from_gdt(SegReg::Cs, selector, memory, mmu)?;
            cpu.update_mode();
        }
    }
    Ok(())
}
