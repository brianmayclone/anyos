//! Syscall wrappers for libdb — delegates to libsyscall.

pub use libsyscall::{
    exit, sbrk, mmap, munmap, open, close, read, write, lseek, file_size, log,
    fsync, O_WRITE, O_CREATE, O_TRUNC, SEEK_SET,
};
