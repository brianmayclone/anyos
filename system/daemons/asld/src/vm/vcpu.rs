use crate::errors::AsldError;

use super::msr::{MsrState, MSR_IA32_EFER, MSR_IA32_FS_BASE, MSR_IA32_GS_BASE};

pub(super) fn write_io_read_value(
    vcpu: &libavm::AvmVcpu,
    access_size: u8,
    value: u32,
) -> Result<(), AsldError> {
    let mut regs = vcpu
        .regs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_regs failed"))?;
    let mask = match access_size {
        1 => 0xffu64,
        2 => 0xffffu64,
        4 => 0xffff_ffffu64,
        _ => 0xffu64,
    };
    regs.rax = (regs.rax & !mask) | ((value as u64) & mask);
    vcpu.set_regs(&regs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_regs failed"))
}

pub(super) fn write_msr_read_value(vcpu: &libavm::AvmVcpu, value: u64) -> Result<(), AsldError> {
    let mut regs = vcpu
        .regs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_regs failed"))?;
    regs.rax = (regs.rax & !0xffff_ffffu64) | (value & 0xffff_ffffu64);
    regs.rdx = (regs.rdx & !0xffff_ffffu64) | ((value >> 32) & 0xffff_ffffu64);
    vcpu.set_regs(&regs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_regs failed"))
}

pub(super) fn sync_guest_msr_side_effects(
    vcpu: &libavm::AvmVcpu,
    msr: u32,
    state: &MsrState,
) -> Result<(), AsldError> {
    if !matches!(msr, MSR_IA32_EFER | MSR_IA32_FS_BASE | MSR_IA32_GS_BASE) {
        return Ok(());
    }
    let mut sregs = vcpu
        .sregs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_sregs failed"))?;
    match msr {
        MSR_IA32_EFER => sregs.efer = state.efer,
        MSR_IA32_FS_BASE => sregs.fs_base = state.fs_base,
        MSR_IA32_GS_BASE => sregs.gs_base = state.gs_base,
        _ => {}
    }
    vcpu.set_sregs(&sregs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_sregs failed"))
}

pub(super) fn advance_guest_rip(
    vcpu: &libavm::AvmVcpu,
    instruction_len: u32,
) -> Result<(), AsldError> {
    let mut sregs = vcpu
        .sregs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_sregs failed"))?;
    sregs.rip = sregs.rip.wrapping_add(if instruction_len == 0 {
        1
    } else {
        instruction_len as u64
    });
    vcpu.set_sregs(&sregs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_sregs failed"))
}
