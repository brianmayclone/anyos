#[cfg(not(target_os = "linux"))]
use crate::errors::AsldError;

#[cfg(not(target_os = "linux"))]
use super::VmInstance;
use super::{exit_reason, VmExitInfo};

#[derive(Clone, Copy)]
pub(super) struct IoStringInfo {
    pub(super) rep: bool,
    pub(super) address_size: u8,
}

pub(super) fn io_string_info(exit: &VmExitInfo) -> Option<IoStringInfo> {
    if exit.reason != exit_reason::IO_INSTRUCTION {
        return None;
    }

    let (is_string, rep, address_code) = match exit.hw_reason {
        30 => (
            (exit.qualification & (1 << 4)) != 0,
            (exit.qualification & (1 << 5)) != 0,
            (exit.qualification >> 7) & 0x7,
        ),
        0x7b => (
            (exit.qualification & (1 << 2)) != 0,
            (exit.qualification & (1 << 3)) != 0,
            (exit.qualification >> 7) & 0x7,
        ),
        _ => (false, false, 0),
    };
    if !is_string {
        return None;
    }

    Some(IoStringInfo {
        rep,
        address_size: match address_code {
            0 => 2,
            1 => 4,
            2 => 8,
            _ => 2,
        },
    })
}

pub(super) fn address_register_value(value: u64, address_size: u8) -> u64 {
    match address_size {
        2 => value & 0xffff,
        4 => value & 0xffff_ffff,
        _ => value,
    }
}

pub(super) fn update_address_register(original: u64, address_size: u8, value: u64) -> u64 {
    match address_size {
        2 => (original & !0xffff) | (value & 0xffff),
        4 => value & 0xffff_ffff,
        _ => value,
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read_guest_bytes(
    instance: &VmInstance,
    vcpu: &libavm::AvmVcpu,
    guest_linear: u64,
    bytes: &mut [u8],
) -> Result<(), AsldError> {
    let gpa = vcpu
        .translate(guest_linear)
        .map_err(|_| AsldError::BackendUnavailable("avm translate failed"))?
        .unwrap_or(guest_linear);
    read_guest_physical_checked(
        instance.guest_memory_addr,
        instance.guest_memory_size,
        gpa,
        bytes,
    )
}

#[cfg(not(target_os = "linux"))]
pub(super) fn write_guest_bytes(
    instance: &VmInstance,
    vcpu: &libavm::AvmVcpu,
    guest_linear: u64,
    bytes: &[u8],
) -> Result<(), AsldError> {
    let gpa = vcpu
        .translate(guest_linear)
        .map_err(|_| AsldError::BackendUnavailable("avm translate failed"))?
        .unwrap_or(guest_linear);
    write_guest_physical_checked(
        instance.guest_memory_addr,
        instance.guest_memory_size,
        gpa,
        bytes,
    )
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read_guest_physical(
    guest_memory_addr: usize,
    guest_memory_size: usize,
    guest_phys: u64,
    dest: &mut [u8],
) -> bool {
    read_guest_physical_checked(guest_memory_addr, guest_memory_size, guest_phys, dest).is_ok()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn write_guest_physical(
    guest_memory_addr: usize,
    guest_memory_size: usize,
    guest_phys: u64,
    bytes: &[u8],
) -> bool {
    write_guest_physical_checked(guest_memory_addr, guest_memory_size, guest_phys, bytes).is_ok()
}

#[cfg(not(target_os = "linux"))]
fn read_guest_physical_checked(
    guest_memory_addr: usize,
    guest_memory_size: usize,
    guest_phys: u64,
    dest: &mut [u8],
) -> Result<(), AsldError> {
    let start = guest_phys as usize;
    let end = start
        .checked_add(dest.len())
        .ok_or(AsldError::InvalidState("guest I/O buffer overflow"))?;
    if guest_memory_addr == 0 || end > guest_memory_size {
        return Err(AsldError::InvalidState("guest I/O buffer out of bounds"));
    }
    unsafe {
        let src = core::slice::from_raw_parts((guest_memory_addr + start) as *const u8, dest.len());
        dest.copy_from_slice(src);
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn write_guest_physical_checked(
    guest_memory_addr: usize,
    guest_memory_size: usize,
    guest_phys: u64,
    bytes: &[u8],
) -> Result<(), AsldError> {
    let start = guest_phys as usize;
    let end = start
        .checked_add(bytes.len())
        .ok_or(AsldError::InvalidState("guest I/O buffer overflow"))?;
    if guest_memory_addr == 0 || end > guest_memory_size {
        return Err(AsldError::InvalidState("guest I/O buffer out of bounds"));
    }
    unsafe {
        let dest =
            core::slice::from_raw_parts_mut((guest_memory_addr + start) as *mut u8, bytes.len());
        dest.copy_from_slice(bytes);
    }
    Ok(())
}
