//! DLL loading support.

use crate::raw::*;

/// Load/map a DLL by filesystem path.
/// Returns the base virtual address, or 0 on failure.
pub fn dll_load(path: &str) -> u32 {
    let mut path_buf = [0u8; 257];
    let plen = path.len().min(256);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    path_buf[plen] = 0;
    syscall2(SYS_DLL_LOAD, path_buf.as_ptr() as u64, plen as u64)
}

/// Write a u32 value to a shared read-only DLIB page (kernel-mediated).
///
/// `dll_base` is the base virtual address of the DLIB (e.g. 0x04000000).
/// `offset` is the byte offset within the RO region.
/// Returns 0 on success, non-zero on error.
pub fn set_dll_u32(dll_base: u64, offset: u32, value: u32) -> u32 {
    syscall3(SYS_SET_DLL_U32, dll_base, offset as u64, value as u64)
}
