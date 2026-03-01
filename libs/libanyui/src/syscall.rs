//! Syscall wrappers for libanyui — delegates to libsyscall.

pub use libsyscall::{
    exit, yield_cpu, sleep, sbrk, mmap, munmap, uptime_ms,
    dll_load, readdir, getcwd, write, open, read, close,
    evt_chan_poll, evt_chan_wait, evt_chan_emit,
};

/// Create a directory (accepts &[u8] path).
pub fn mkdir(path: &[u8]) -> u32 {
    libsyscall::mkdir_bytes(path)
}
