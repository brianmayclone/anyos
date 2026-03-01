//! Syscall wrappers for libfont.dlib — delegates to libsyscall.

pub use libsyscall::{sbrk, mmap, munmap, exit, close};

/// Query whether GPU acceleration is available.
pub fn gpu_has_accel() -> u32 {
    libsyscall::syscall0(libsyscall::SYS_GPU_HAS_ACCEL) as u32
}

/// Open a file (byte slice path). Returns fd or u32::MAX on error.
pub fn open(path: &[u8], flags: u32) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path[..len]);
    buf[len] = 0;
    libsyscall::syscall2(libsyscall::SYS_OPEN, buf.as_ptr() as u64, flags as u64) as u32
}

/// Read from a file descriptor into buffer. Returns bytes read.
pub fn read(fd: u32, buf: *mut u8, count: u32) -> u32 {
    libsyscall::syscall3(libsyscall::SYS_READ, fd as u64, buf as u64, count as u64) as u32
}

/// Get file status. Returns 0 on success.
pub fn fstat(fd: u32, stat_buf: *mut u8) -> u32 {
    libsyscall::syscall2(libsyscall::SYS_FSTAT, fd as u64, stat_buf as u64) as u32
}

/// Read an entire file into a Vec<u8>. Returns None on error.
pub fn read_file(path: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    let fd = open(path, 0);
    if fd == u32::MAX {
        return None;
    }

    // Get file size via fstat
    let mut stat_buf = [0u8; 32];
    if fstat(fd, stat_buf.as_mut_ptr()) != 0 {
        close(fd);
        return None;
    }
    let file_size = u32::from_le_bytes([stat_buf[4], stat_buf[5], stat_buf[6], stat_buf[7]]) as usize;
    if file_size == 0 {
        close(fd);
        return None;
    }

    let mut data = alloc::vec![0u8; file_size];
    let mut total_read = 0usize;
    while total_read < file_size {
        let chunk = (file_size - total_read).min(4096);
        let n = read(fd, data[total_read..].as_mut_ptr(), chunk as u32) as usize;
        if n == 0 || n > chunk {
            break;
        }
        total_read += n;
    }
    close(fd);

    if total_read > 0 {
        data.truncate(total_read);
        Some(data)
    } else {
        None
    }
}
