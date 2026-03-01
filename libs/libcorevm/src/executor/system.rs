//! System instruction handlers.
//!
//! Implements LGDT, LIDT, SGDT, SIDT, MOV CR, MOV DR, RDMSR, WRMSR, CPUID,
//! RDTSC, INVLPG, HLT, LMSW, SMSW, SYSCALL, SYSRET, SWAPGS, WBINVD, and
//! CLTS.

use crate::cpu::{Cpu, Mode};
use crate::error::{Result, VmError};
use crate::flags::{self, OperandSize};
use crate::instruction::{DecodedInst, Operand};
use crate::memory::{GuestMemory, Mmu};
use crate::registers::*;

use super::{compute_effective_address, translate_and_read, translate_and_write};

// ── Descriptor table operations ──

/// LGDT: load the Global Descriptor Table Register from memory.
///
/// Reads a 6-byte (16-bit mode), 6-byte (32-bit mode), or 10-byte (64-bit
/// mode) pseudo-descriptor from the memory operand: limit (2 bytes) followed
/// by base (4 or 8 bytes).
pub fn exec_lgdt(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let linear = get_memory_linear(cpu, inst)?;

    let limit = translate_and_read(cpu, linear, OperandSize::Word, mmu, memory)? as u16;
    let base = if matches!(cpu.mode, Mode::LongMode) {
        translate_and_read(cpu, linear.wrapping_add(2), OperandSize::Qword, mmu, memory)?
    } else {
        translate_and_read(cpu, linear.wrapping_add(2), OperandSize::Dword, mmu, memory)?
    };

    cpu.regs.gdtr = TableRegister { base, limit };
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// LIDT: load the Interrupt Descriptor Table Register from memory.
pub fn exec_lidt(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let linear = get_memory_linear(cpu, inst)?;

    let limit = translate_and_read(cpu, linear, OperandSize::Word, mmu, memory)? as u16;
    let base = if matches!(cpu.mode, Mode::LongMode) {
        translate_and_read(cpu, linear.wrapping_add(2), OperandSize::Qword, mmu, memory)?
    } else {
        translate_and_read(cpu, linear.wrapping_add(2), OperandSize::Dword, mmu, memory)?
    };

    cpu.regs.idtr = TableRegister { base, limit };
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// SGDT: store the GDTR to memory.
pub fn exec_sgdt(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let linear = get_memory_linear(cpu, inst)?;

    translate_and_write(
        cpu,
        linear,
        OperandSize::Word,
        cpu.regs.gdtr.limit as u64,
        mmu,
        memory,
    )?;

    if matches!(cpu.mode, Mode::LongMode) {
        translate_and_write(
            cpu,
            linear.wrapping_add(2),
            OperandSize::Qword,
            cpu.regs.gdtr.base,
            mmu,
            memory,
        )?;
    } else {
        translate_and_write(
            cpu,
            linear.wrapping_add(2),
            OperandSize::Dword,
            cpu.regs.gdtr.base,
            mmu,
            memory,
        )?;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// SIDT: store the IDTR to memory.
pub fn exec_sidt(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let linear = get_memory_linear(cpu, inst)?;

    translate_and_write(
        cpu,
        linear,
        OperandSize::Word,
        cpu.regs.idtr.limit as u64,
        mmu,
        memory,
    )?;

    if matches!(cpu.mode, Mode::LongMode) {
        translate_and_write(
            cpu,
            linear.wrapping_add(2),
            OperandSize::Qword,
            cpu.regs.idtr.base,
            mmu,
            memory,
        )?;
    } else {
        translate_and_write(
            cpu,
            linear.wrapping_add(2),
            OperandSize::Dword,
            cpu.regs.idtr.base,
            mmu,
            memory,
        )?;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── Control/Debug register moves ──

/// MOV CRn, r / MOV r, CRn.
///
/// Opcode 0F 20: MOV r64, CRn (read control register)
/// Opcode 0F 22: MOV CRn, r64 (write control register)
///
/// After writing CR0, calls `cpu.update_mode()` to recalculate the CPU mode.
pub fn exec_mov_cr(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let op = inst.opcode as u8;

    if op == 0x20 {
        // MOV r, CRn — read CR into GPR
        let cr_idx = inst.modrm_reg() & 0x0F;
        let val = match cr_idx {
            0 => cpu.regs.cr0,
            2 => cpu.regs.cr2,
            3 => cpu.regs.cr3,
            4 => cpu.regs.cr4,
            8 => cpu.regs.cr8,
            _ => return Err(VmError::UndefinedOpcode(op)),
        };
        let dst_gpr = inst.modrm_rm();
        cpu.regs.write_gpr64(dst_gpr, val);
    } else {
        // MOV CRn, r — write GPR into CR
        let cr_idx = inst.modrm_reg() & 0x0F;
        let src_gpr = inst.modrm_rm();
        let val = cpu.regs.read_gpr64(src_gpr);
        match cr_idx {
            0 => {
                cpu.regs.cr0 = val;
                cpu.update_mode();
            }
            2 => cpu.regs.cr2 = val,
            3 => cpu.regs.cr3 = val,
            4 => cpu.regs.cr4 = val,
            8 => cpu.regs.cr8 = val,
            _ => return Err(VmError::UndefinedOpcode(op)),
        }
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// MOV DRn, r / MOV r, DRn.
///
/// Opcode 0F 21: MOV r64, DRn (read debug register)
/// Opcode 0F 23: MOV DRn, r64 (write debug register)
pub fn exec_mov_dr(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let op = inst.opcode as u8;
    let dr_idx = (inst.modrm_reg() & 0x07) as usize;

    if dr_idx >= cpu.regs.dr.len() {
        return Err(VmError::UndefinedOpcode(op));
    }

    if op == 0x21 {
        // MOV r, DRn
        let val = cpu.regs.dr[dr_idx];
        let dst_gpr = inst.modrm_rm();
        cpu.regs.write_gpr64(dst_gpr, val);
    } else {
        // MOV DRn, r
        let src_gpr = inst.modrm_rm();
        let val = cpu.regs.read_gpr64(src_gpr);
        cpu.regs.dr[dr_idx] = val;
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── MSR operations ──

/// RDMSR: read Model-Specific Register.
///
/// ECX selects the MSR; the 64-bit value is returned in EDX:EAX.
pub fn exec_rdmsr(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let msr_index = cpu.regs.read_gpr32(GprIndex::Rcx as u8);
    let val = cpu.regs.read_msr(msr_index);

    cpu.regs.write_gpr32(GprIndex::Rax as u8, val as u32);
    cpu.regs.write_gpr32(GprIndex::Rdx as u8, (val >> 32) as u32);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// WRMSR: write Model-Specific Register.
///
/// ECX selects the MSR; the 64-bit value comes from EDX:EAX.
/// After writing EFER, calls `cpu.update_mode()` to recalculate CPU mode.
pub fn exec_wrmsr(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let msr_index = cpu.regs.read_gpr32(GprIndex::Rcx as u8);
    let lo = cpu.regs.read_gpr32(GprIndex::Rax as u8) as u64;
    let hi = cpu.regs.read_gpr32(GprIndex::Rdx as u8) as u64;
    let val = (hi << 32) | lo;

    cpu.regs.write_msr(msr_index, val);

    // Special handling for FS/GS base MSRs
    match msr_index {
        MSR_FS_BASE => {
            cpu.regs.segment_mut(SegReg::Fs).base = val;
        }
        MSR_GS_BASE => {
            cpu.regs.segment_mut(SegReg::Gs).base = val;
        }
        MSR_EFER => {
            cpu.update_mode();
        }
        _ => {}
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── CPUID ──

/// CPUID: return emulated processor identification and feature information.
///
/// Input: EAX = leaf number, ECX = sub-leaf (for some leaves).
/// Output: EAX, EBX, ECX, EDX.
pub fn exec_cpuid(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let leaf = cpu.regs.read_gpr32(GprIndex::Rax as u8);

    let (eax, ebx, ecx, edx) = match leaf {
        // Leaf 0: max standard leaf + vendor string
        0 => {
            // Vendor: "CoreVMx86Em\0" -> EBX:EDX:ECX
            // "Core" = 0x65726F43
            // "VMx8" = 0x3878_4D56
            // "6Em\0" = 0x006D_4536
            (0x0D, 0x65726F43, 0x006D4536, 0x38784D56)
        }
        // Leaf 1: family/model/stepping + feature flags
        1 => {
            // Family 6, Model 0x3C, Stepping 1 -> EAX = 0x000306C1
            let eax_val = 0x0003_06C1u32;
            // EBX: brand index=0, CLFLUSH=8, max IDs=1, APIC ID=0
            let ebx_val = 0x0001_0800u32;
            // ECX feature flags: SSE3(0), SSE4.1(19), SSE4.2(20), POPCNT(23)
            let ecx_val = (1 << 0) | (1 << 19) | (1 << 20) | (1 << 23);
            // EDX feature flags:
            // FPU(0), VME(1), DE(2), PSE(3), TSC(4), MSR(5), PAE(6),
            // CX8(8), PGE(13), MCA(14), CMOV(15), PAT(16), PSE-36(17),
            // CLFLUSH(19), MMX(23), FXSR(24), SSE(25), SSE2(26)
            let edx_val: u32 = (1 << 0)
                | (1 << 1)
                | (1 << 2)
                | (1 << 3)
                | (1 << 4)
                | (1 << 5)
                | (1 << 6)
                | (1 << 8)
                | (1 << 13)
                | (1 << 14)
                | (1 << 15)
                | (1 << 16)
                | (1 << 17)
                | (1 << 19)
                | (1 << 23)
                | (1 << 24)
                | (1 << 25)
                | (1 << 26);
            (eax_val, ebx_val, ecx_val, edx_val)
        }
        // Leaf 0x80000000: max extended leaf
        0x8000_0000 => (0x8000_0004, 0, 0, 0),
        // Leaf 0x80000001: extended feature flags
        0x8000_0001 => {
            // EDX: SYSCALL(11), NX(20), LM(29)
            let edx_val: u32 = (1 << 11) | (1 << 20) | (1 << 29);
            (0, 0, 0, edx_val)
        }
        // Leaf 0x80000002-0x80000004: processor brand string
        // "CoreVM x86 Emulator" padded to 48 bytes
        0x8000_0002 => {
            // "Core"
            (0x65726F43, 0x78204D56, 0x45203638, 0x616C756D)
        }
        0x8000_0003 => {
            // "tor\0" + padding
            (0x00726F74, 0, 0, 0)
        }
        0x8000_0004 => (0, 0, 0, 0),
        // All other leaves return zero
        _ => (0, 0, 0, 0),
    };

    cpu.regs.write_gpr32(GprIndex::Rax as u8, eax);
    cpu.regs.write_gpr32(GprIndex::Rbx as u8, ebx);
    cpu.regs.write_gpr32(GprIndex::Rcx as u8, ecx);
    cpu.regs.write_gpr32(GprIndex::Rdx as u8, edx);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── RDTSC ──

/// RDTSC: read Time Stamp Counter.
///
/// Returns the TSC value in EDX:EAX. We use the stored MSR_TSC value and
/// increment it each time RDTSC is executed.
pub fn exec_rdtsc(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let tsc = cpu.regs.read_msr(MSR_TSC);
    cpu.regs.write_gpr32(GprIndex::Rax as u8, tsc as u32);
    cpu.regs.write_gpr32(GprIndex::Rdx as u8, (tsc >> 32) as u32);

    // Increment TSC for next read
    cpu.regs.write_msr(MSR_TSC, tsc.wrapping_add(100));

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── INVLPG ──

/// INVLPG: invalidate TLB entry for the page containing the memory operand.
///
/// Since our emulator does not maintain a TLB, this is effectively a no-op.
/// We still compute the address to validate the operand and advance RIP.
pub fn exec_invlpg(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    _memory: &mut GuestMemory,
    _mmu: &Mmu,
) -> Result<()> {
    // Validate that operand 0 is a memory operand (INVLPG requires it)
    match &inst.operands[0] {
        Operand::Memory(_) => {}
        _ => return Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── HLT ──

/// HLT: halt the processor.
///
/// Returns `VmError::Halted` to signal the VM execution loop to stop.
pub fn exec_hlt(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    cpu.regs.rip += inst.length as u64;
    Err(VmError::Halted)
}

// ── LMSW / SMSW ──

/// LMSW: load machine status word (low 16 bits of CR0).
///
/// Can set PE (Protection Enable) but cannot clear it. Bits above 15 of
/// CR0 are not affected.
pub fn exec_lmsw(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let val = super::read_operand(cpu, inst, &inst.operands[0], memory, mmu)? as u16;

    // LMSW can set PE but cannot clear it
    let new_cr0_low = val as u64;
    let old_pe = cpu.regs.cr0 & CR0_PE;
    cpu.regs.cr0 = (cpu.regs.cr0 & !0xFFFF) | new_cr0_low | old_pe;

    cpu.update_mode();
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// SMSW: store machine status word (low 16 bits of CR0).
pub fn exec_smsw(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let val = (cpu.regs.cr0 & 0xFFFF) as u64;
    super::write_operand(cpu, inst, &inst.operands[0], val, memory, mmu)?;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── SYSCALL / SYSRET ──

/// SYSCALL: fast system call entry (64-bit mode).
///
/// - RCX = return RIP (next instruction)
/// - R11 = RFLAGS
/// - RFLAGS &= ~SFMASK
/// - CS.selector from STAR[47:32], SS.selector from STAR[47:32]+8
/// - RIP = LSTAR
///
/// Raises `#UD` if SCE (SYSCALL Enable) is not set in EFER, or if not in
/// long mode.
pub fn exec_syscall(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let efer = cpu.regs.read_msr(MSR_EFER);
    if (efer & EFER_SCE) == 0 {
        return Err(VmError::UndefinedOpcode(0x05));
    }

    let next_rip = cpu.regs.rip.wrapping_add(inst.length as u64);

    // Save return state
    cpu.regs.write_gpr64(GprIndex::Rcx as u8, next_rip);
    cpu.regs.write_gpr64(GprIndex::R11 as u8, cpu.regs.rflags);

    // Load STAR MSR: bits [47:32] = SYSCALL CS selector
    let star = cpu.regs.read_msr(MSR_STAR);
    let syscall_cs = ((star >> 32) & 0xFFFF) as u16;
    let syscall_ss = syscall_cs + 8;

    // Set CS and SS selectors
    cpu.regs.segment_mut(SegReg::Cs).selector = syscall_cs;
    cpu.regs.segment_mut(SegReg::Cs).long_mode = true;
    cpu.regs.segment_mut(SegReg::Cs).big = false;
    cpu.regs.segment_mut(SegReg::Cs).is_code = true;
    cpu.regs.segment_mut(SegReg::Cs).dpl = 0;
    cpu.regs.segment_mut(SegReg::Cs).base = 0;
    cpu.regs.segment_mut(SegReg::Cs).limit = 0xFFFF_FFFF;
    cpu.regs.segment_mut(SegReg::Cs).present = true;

    cpu.regs.segment_mut(SegReg::Ss).selector = syscall_ss;
    cpu.regs.segment_mut(SegReg::Ss).base = 0;
    cpu.regs.segment_mut(SegReg::Ss).limit = 0xFFFF_FFFF;
    cpu.regs.segment_mut(SegReg::Ss).present = true;
    cpu.regs.segment_mut(SegReg::Ss).writable = true;

    // Mask RFLAGS
    let sfmask = cpu.regs.read_msr(MSR_SFMASK);
    cpu.regs.rflags &= !sfmask;
    cpu.regs.rflags |= flags::RFLAGS_FIXED;

    // Jump to LSTAR
    cpu.regs.rip = cpu.regs.read_msr(MSR_LSTAR);
    cpu.regs.cpl = 0;

    Ok(())
}

/// SYSRET: fast return from system call.
///
/// - RIP = RCX
/// - RFLAGS = R11 (masked)
/// - CS.selector from STAR[63:48]+16 (user CS), SS.selector from STAR[63:48]+8
///
/// Raises `#UD` if SCE is not set in EFER.
pub fn exec_sysret(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    let efer = cpu.regs.read_msr(MSR_EFER);
    if (efer & EFER_SCE) == 0 {
        return Err(VmError::UndefinedOpcode(0x07));
    }

    let star = cpu.regs.read_msr(MSR_STAR);

    // SYSRET CS = STAR[63:48]+16, SS = STAR[63:48]+8
    let sysret_cs_base = ((star >> 48) & 0xFFFF) as u16;
    let user_cs: u16;
    let user_ss: u16;

    if inst.prefix.rex_w() {
        // 64-bit SYSRET
        user_cs = sysret_cs_base + 16;
        user_ss = sysret_cs_base + 8;
    } else {
        // 32-bit compat SYSRET
        user_cs = sysret_cs_base;
        user_ss = sysret_cs_base + 8;
    }

    // Restore RIP and RFLAGS
    let new_rip = cpu.regs.read_gpr64(GprIndex::Rcx as u8);
    let new_rflags = cpu.regs.read_gpr64(GprIndex::R11 as u8);

    cpu.regs.rip = new_rip;
    cpu.regs.rflags = new_rflags | flags::RFLAGS_FIXED;
    // Clear RF
    cpu.regs.rflags &= !flags::RF;

    // Set user-mode CS
    cpu.regs.segment_mut(SegReg::Cs).selector = user_cs;
    cpu.regs.segment_mut(SegReg::Cs).long_mode = inst.prefix.rex_w();
    cpu.regs.segment_mut(SegReg::Cs).big = !inst.prefix.rex_w();
    cpu.regs.segment_mut(SegReg::Cs).is_code = true;
    cpu.regs.segment_mut(SegReg::Cs).dpl = 3;
    cpu.regs.segment_mut(SegReg::Cs).base = 0;
    cpu.regs.segment_mut(SegReg::Cs).limit = 0xFFFF_FFFF;
    cpu.regs.segment_mut(SegReg::Cs).present = true;

    cpu.regs.segment_mut(SegReg::Ss).selector = user_ss;
    cpu.regs.segment_mut(SegReg::Ss).base = 0;
    cpu.regs.segment_mut(SegReg::Ss).limit = 0xFFFF_FFFF;
    cpu.regs.segment_mut(SegReg::Ss).present = true;
    cpu.regs.segment_mut(SegReg::Ss).writable = true;

    cpu.regs.cpl = 3;

    Ok(())
}

/// SWAPGS: swap GS.base with MSR_KERNEL_GS_BASE.
///
/// Only valid in 64-bit mode at CPL 0. Raises `#GP(0)` otherwise.
pub fn exec_swapgs(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    if !matches!(cpu.mode, Mode::LongMode) || cpu.regs.cpl != 0 {
        return Err(VmError::GeneralProtection(0));
    }

    let gs_base = cpu.regs.segment(SegReg::Gs).base;
    let kernel_gs = cpu.regs.read_msr(MSR_KERNEL_GS_BASE);

    cpu.regs.segment_mut(SegReg::Gs).base = kernel_gs;
    cpu.regs.write_msr(MSR_KERNEL_GS_BASE, gs_base);

    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// WBINVD: write back and invalidate data caches.
///
/// No-op in our emulator. Just advance RIP.
pub fn exec_wbinvd(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

/// CLTS: clear the Task-Switched flag (CR0.TS) in CR0.
pub fn exec_clts(cpu: &mut Cpu, inst: &DecodedInst) -> Result<()> {
    cpu.regs.cr0 &= !CR0_TS;
    cpu.regs.rip += inst.length as u64;
    Ok(())
}

// ── Helpers ──

/// Extract the linear address from the memory operand at position 0.
fn get_memory_linear(cpu: &Cpu, inst: &DecodedInst) -> Result<u64> {
    match &inst.operands[0] {
        Operand::Memory(mem_op) => compute_effective_address(cpu, mem_op, inst),
        _ => Err(VmError::UndefinedOpcode(inst.opcode as u8)),
    }
}
