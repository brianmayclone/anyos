//! Filesystem operations — open, close, read, write, readdir, stat.

use crate::raw::*;

pub fn write(fd: u32, buf: &[u8]) -> u32 {
    syscall3(SYS_WRITE, fd, buf.as_ptr() as u32, buf.len() as u32)
}

pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_READ, fd, buf.as_mut_ptr() as u32, buf.len() as u32)
}

pub fn open(path: &str, flags: u32) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    syscall3(SYS_OPEN, buf.as_ptr() as u32, flags, 0)
}

pub fn close(fd: u32) -> u32 {
    syscall1(SYS_CLOSE, fd)
}

/// Read directory entries. Returns number of entries (or u32::MAX on error).
/// Each entry is 64 bytes: [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes]
pub fn readdir(path: &str, buf: &mut [u8]) -> u32 {
    let mut path_buf = [0u8; 257];
    let len = path.len().min(256);
    path_buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    path_buf[len] = 0;
    syscall3(SYS_READDIR, path_buf.as_ptr() as u32, buf.as_mut_ptr() as u32, buf.len() as u32)
}

/// Get file status. Returns 0 on success. Writes [type:u32, size:u32] to buf.
pub fn stat(path: &str, stat_buf: &mut [u32; 2]) -> u32 {
    let mut path_buf = [0u8; 257];
    let len = path.len().min(256);
    path_buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    path_buf[len] = 0;
    syscall2(SYS_STAT, path_buf.as_ptr() as u32, stat_buf.as_mut_ptr() as u32)
}
