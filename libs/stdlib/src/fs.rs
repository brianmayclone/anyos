//! Filesystem operations — open, close, read, write, readdir, stat, mkdir, unlink.

use crate::raw::*;

// Open flags (bit flags, match kernel/src/syscall/handlers.rs sys_open)
pub const O_WRITE: u32 = 1;
pub const O_APPEND: u32 = 2;
pub const O_CREATE: u32 = 4;
pub const O_TRUNC: u32 = 8;

pub fn write(fd: u32, buf: &[u8]) -> u32 {
    syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64)
}

pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

pub fn open(path: &str, flags: u32) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    syscall3(SYS_OPEN, buf.as_ptr() as u64, flags as u64, 0)
}

pub fn close(fd: u32) -> u32 {
    syscall1(SYS_CLOSE, fd as u64)
}

/// Read directory entries. Returns number of entries (or u32::MAX on error).
/// Each entry is 64 bytes: [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes]
pub fn readdir(path: &str, buf: &mut [u8]) -> u32 {
    let mut path_buf = [0u8; 257];
    let len = path.len().min(256);
    path_buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    path_buf[len] = 0;
    syscall3(SYS_READDIR, path_buf.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Get file status. Returns 0 on success. Writes [type:u32, size:u32] to buf.
pub fn stat(path: &str, stat_buf: &mut [u32; 2]) -> u32 {
    let mut path_buf = [0u8; 257];
    let len = path.len().min(256);
    path_buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    path_buf[len] = 0;
    syscall2(SYS_STAT, path_buf.as_ptr() as u64, stat_buf.as_mut_ptr() as u64)
}

/// Create a directory. Returns 0 on success, u32::MAX on error.
pub fn mkdir(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    syscall1(SYS_MKDIR, buf.as_ptr() as u64)
}

/// Delete a file. Returns 0 on success, u32::MAX on error.
pub fn unlink(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    syscall1(SYS_UNLINK, buf.as_ptr() as u64)
}

/// Truncate a file to zero length. Returns 0 on success, u32::MAX on error.
pub fn truncate(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    syscall1(SYS_TRUNCATE, buf.as_ptr() as u64)
}

// Seek whence constants
pub const SEEK_SET: u32 = 0;
pub const SEEK_CUR: u32 = 1;
pub const SEEK_END: u32 = 2;

/// Seek within an open file. Returns new position or u32::MAX on error.
pub fn lseek(fd: u32, offset: i32, whence: u32) -> u32 {
    syscall3(SYS_LSEEK, fd as u64, offset as i64 as u64, whence as u64)
}

/// Get file information by fd. Returns 0 on success.
/// Writes [type:u32, size:u32, position:u32] to stat_buf.
pub fn fstat(fd: u32, stat_buf: &mut [u32; 3]) -> u32 {
    syscall2(SYS_FSTAT, fd as u64, stat_buf.as_mut_ptr() as u64)
}

/// Get current working directory. Returns length or u32::MAX on error.
pub fn getcwd(buf: &mut [u8]) -> u32 {
    syscall2(SYS_GETCWD, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Check if fd refers to a terminal. Returns 1 for tty, 0 otherwise.
pub fn isatty(fd: u32) -> u32 {
    syscall1(SYS_ISATTY, fd as u64)
}
