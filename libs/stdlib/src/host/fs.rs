//! Host-mode filesystem — delegates to std::fs.

use alloc::string::String;
use alloc::vec::Vec;

/// Open flags (compatible with anyOS API).
pub const O_WRITE: u32 = 1;
pub const O_APPEND: u32 = 2;
pub const O_CREATE: u32 = 4;
pub const O_TRUNC: u32 = 8;

pub fn open(_path: &str, _flags: u32) -> u32 {
    u32::MAX // Not implemented for host — use high-level functions
}

pub fn close(_fd: u32) -> u32 { 0 }

pub fn read(_fd: u32, _buf: &mut [u8]) -> u32 { 0 }

pub fn write(fd: u32, buf: &[u8]) -> u32 {
    use std::io::Write as IoWrite;
    if fd == 1 {
        let _ = std::io::stdout().write_all(buf);
    } else if fd == 2 {
        let _ = std::io::stderr().write_all(buf);
    }
    buf.len() as u32
}

pub fn read_to_string(path: &str) -> Result<String, ()> {
    std::fs::read_to_string(path).map_err(|_| ())
}

pub fn read_to_vec(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

pub fn write_bytes(path: &str, data: &[u8]) -> Result<(), ()> {
    std::fs::write(path, data).map_err(|_| ())
}

pub fn stat(_path: &str, _stat_buf: &mut [u32; 7]) -> u32 {
    u32::MAX
}

pub fn fstat(_fd: u32, _stat_buf: &mut [u32; 4]) -> u32 {
    u32::MAX
}

pub fn readdir(_path: &str, _buf: &mut [u8]) -> u32 {
    0
}

pub fn mkdir(path: &str) -> u32 {
    if std::fs::create_dir_all(path).is_ok() { 0 } else { u32::MAX }
}

pub fn unlink(path: &str) -> u32 {
    if std::fs::remove_file(path).is_ok() { 0 } else { u32::MAX }
}

pub fn getcwd(buf: &mut [u8]) -> u32 {
    if let Ok(cwd) = std::env::current_dir() {
        let s = cwd.to_string_lossy();
        let bytes = s.as_bytes();
        let len = bytes.len().min(buf.len() - 1);
        buf[..len].copy_from_slice(&bytes[..len]);
        buf[len] = 0;
        len as u32
    } else {
        u32::MAX
    }
}

pub fn isatty(fd: u32) -> u32 {
    if fd <= 2 { 1 } else { 0 }
}

pub fn rename(old: &str, new: &str) -> u32 {
    if std::fs::rename(old, new).is_ok() { 0 } else { u32::MAX }
}
