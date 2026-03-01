//! Syscall wrappers for libzip — delegates to libsyscall.

pub use libsyscall::{
    sbrk, mmap, munmap, exit, close, lseek, file_size, mkdir, stat,
    O_WRITE, O_CREATE, O_TRUNC, SEEK_SET,
};

pub fn open(path: &str, flags: u32) -> u32 {
    libsyscall::open(path, flags)
}

pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    libsyscall::read(fd, buf)
}

pub fn write(fd: u32, buf: &[u8]) -> u32 {
    libsyscall::write(fd, buf)
}

pub fn readdir(path: &str, buf: &mut [u8]) -> u32 {
    let mut path_buf = [0u8; 257];
    let len = path.len().min(256);
    path_buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    path_buf[len] = 0;
    libsyscall::readdir(&path_buf[..len + 1], buf)
}

pub fn log(msg: &[u8]) {
    libsyscall::write(1, msg);
}
