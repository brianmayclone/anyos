//! Device management and DLL loading syscall handlers.
//!
//! Covers device listing, open/close/read/write/ioctl, IRQ waiting,
//! and dynamic library loading.

use super::helpers::{is_valid_user_ptr, read_user_str};

/// sys_devlist - List devices. Each entry is 64 bytes:
///   [0..32]  path (null-terminated)
///   [32..56] driver name (null-terminated, 24 bytes)
///   [56]     driver_type (0=Block,1=Char,2=Network,3=Display,4=Input,5=Audio,6=Output,7=Sensor,8=Bus,9=Unknown)
///   [57..64] padding (zeroed)
pub fn sys_devlist(buf_ptr: u32, buf_size: u32) -> u32 {
    let devices = crate::drivers::hal::list_devices();
    let count = devices.len();
    if buf_ptr != 0 && buf_size > 0 && is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        let entry_size = 64usize;
        let max_entries = buf_size as usize / entry_size;
        for (i, (path, name, dtype)) in devices.iter().enumerate().take(max_entries.min(count)) {
            let offset = i * entry_size;
            // Zero the entry first
            for b in &mut buf[offset..offset + entry_size] { *b = 0; }
            // Path [0..32]
            let path_bytes = path.as_bytes();
            let plen = path_bytes.len().min(31);
            buf[offset..offset + plen].copy_from_slice(&path_bytes[..plen]);
            // Driver name [32..56]
            let name_bytes = name.as_bytes();
            let nlen = name_bytes.len().min(23);
            buf[offset + 32..offset + 32 + nlen].copy_from_slice(&name_bytes[..nlen]);
            // Driver type [56]
            buf[offset + 56] = match dtype {
                crate::drivers::hal::DriverType::Block => 0,
                crate::drivers::hal::DriverType::Char => 1,
                crate::drivers::hal::DriverType::Network => 2,
                crate::drivers::hal::DriverType::Display => 3,
                crate::drivers::hal::DriverType::Input => 4,
                crate::drivers::hal::DriverType::Audio => 5,
                crate::drivers::hal::DriverType::Output => 6,
                crate::drivers::hal::DriverType::Sensor => 7,
                crate::drivers::hal::DriverType::Bus => 8,
                crate::drivers::hal::DriverType::Unknown => 9,
            };
        }
    }
    count as u32
}

pub fn sys_devopen(path_ptr: u32, _flags: u32) -> u32 {
    let path = unsafe { read_user_str(path_ptr) };
    let devices = crate::drivers::hal::list_devices();
    if devices.iter().any(|(p, _, _)| p == path) { 0 } else { u32::MAX }
}

pub fn sys_devclose(_handle: u32) -> u32 { 0 }
pub fn sys_devread(_handle: u32, _buf_ptr: u32, _len: u32) -> u32 { u32::MAX }
pub fn sys_devwrite(_handle: u32, _buf_ptr: u32, _len: u32) -> u32 { u32::MAX }
/// sys_devioctl - Send ioctl to a device by driver type.
/// handle = DriverType as u32 (0=Block,1=Char,2=Network,3=Display,4=Input,5=Audio,6=Output,7=Sensor)
pub fn sys_devioctl(dtype: u32, cmd: u32, arg: u32) -> u32 {
    use crate::drivers::hal::{DriverType, device_ioctl_by_type};
    let driver_type = match dtype {
        0 => DriverType::Block,
        1 => DriverType::Char,
        2 => DriverType::Network,
        3 => DriverType::Display,
        4 => DriverType::Input,
        5 => DriverType::Audio,
        6 => DriverType::Output,
        7 => DriverType::Sensor,
        _ => return u32::MAX,
    };
    match device_ioctl_by_type(driver_type, cmd, arg) {
        Ok(val) => val,
        Err(_) => u32::MAX,
    }
}
pub fn sys_irqwait(_irq: u32) -> u32 { 0 }

// =========================================================================
// DLL (SYS_DLL_LOAD)
// =========================================================================

/// sys_dll_load - Load/map a DLL into the current process.
/// arg1=path_ptr (null-terminated), arg2=path_len (unused, null-terminated).
/// Returns base virtual address of the DLL, or 0 on failure.
pub fn sys_dll_load(path_ptr: u32, _path_len: u32) -> u32 {
    if path_ptr == 0 { return 0; }
    let path = unsafe { read_user_str(path_ptr) };
    // Try existing loaded DLLs first
    if let Some(base) = crate::task::dll::get_dll_base(path) {
        return base as u32;
    }
    // Try loading from filesystem (dload)
    match crate::task::dll::load_dll_dynamic(path) {
        Some(base) => base as u32,
        None => 0,
    }
}

/// Write a u32 value to a shared DLIB page.
/// arg1 = dll_base_vaddr (lower 32 bits), arg2 = offset, arg3 = value.
/// Used by compositor to write theme field to uisys shared RO pages.
pub fn sys_set_dll_u32(dll_base_lo: u32, offset: u32, value: u32) -> u32 {
    let dll_base = dll_base_lo as u64;
    if crate::task::dll::set_dll_u32(dll_base, offset as u64, value) {
        0
    } else {
        u32::MAX
    }
}
