//! Syscall wrappers for libini — delegates to libsyscall.

pub use libsyscall::{sbrk, mmap, munmap, exit};
