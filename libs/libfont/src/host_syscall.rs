//! Host-mode syscall shims for libfont — uses std::fs.

pub fn sbrk(_increment: u32) -> u64 {
    u64::MAX // Not used on host (system allocator)
}

pub fn mmap(_size: u32) -> u64 {
    u64::MAX
}

pub fn munmap(_addr: u64, _size: u32) -> u64 {
    0
}

pub fn exit(code: u32) -> ! {
    std::process::exit(code as i32);
}

pub fn close(_fd: u32) {}

pub fn gpu_has_accel() -> u32 {
    0 // No GPU accel on host — use greyscale AA
}

pub fn open(_path: &[u8], _flags: u32) -> u32 {
    u32::MAX // Not used directly; read_file handles everything
}

pub fn read(_fd: u32, _buf: *mut u8, _count: u32) -> u32 {
    0
}

pub fn fstat(_fd: u32, _stat_buf: *mut u8) -> u32 {
    u32::MAX
}

/// Read an entire file into a Vec<u8>. Uses std::fs on host.
pub fn read_file(path: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    let path_str = core::str::from_utf8(path).ok()?;
    // Strip null terminator if present
    let path_str = path_str.trim_end_matches('\0');
    std::fs::read(path_str).ok()
}
