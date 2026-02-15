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
