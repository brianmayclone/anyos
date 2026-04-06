//! Syscall wrappers for libanyui — delegates to libsyscall.

pub use libsyscall::{
    close, dll_load, evt_chan_emit, evt_chan_poll, evt_chan_wait, exit, get_tid, getcwd, mmap,
    munmap, open, read, readdir, sbrk, sleep, uptime_ms, write, yield_cpu,
};

/// Create a directory (accepts &[u8] path).
pub fn mkdir(path: &[u8]) -> u32 {
    libsyscall::mkdir_bytes(path)
}
