//! Syscall wrappers for libimage.dlib — delegates to libsyscall.

pub use libsyscall::{sbrk, mmap, munmap, exit, close};

/// Write bytes to a file descriptor.
pub fn write(fd: u32, buf: &[u8]) {
    libsyscall::syscall3(libsyscall::SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64);
}

/// Open a file (byte slice path). Returns fd or u32::MAX on error.
pub fn open(path: &[u8]) -> u32 {
    libsyscall::open_bytes(path)
}

/// Read from fd into buf. Returns bytes read.
pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    libsyscall::read(fd, buf)
}

/// Get file stats. stat_buf: [type, size, flags, ...]. Returns 0 on success.
pub fn fstat(fd: u32, stat_buf: &mut [u32; 4]) -> u32 {
    libsyscall::fstat(fd, stat_buf)
}
