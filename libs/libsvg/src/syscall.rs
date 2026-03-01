//! Syscall wrappers for libsvg.so — delegates to libsyscall.

pub use libsyscall::{sbrk, mmap, munmap, exit};

/// Write bytes to a file descriptor (fire-and-forget, no return value).
pub fn write(fd: u32, data: &[u8]) {
    libsyscall::syscall3(libsyscall::SYS_WRITE, fd as u64, data.as_ptr() as u64, data.len() as u64);
}
