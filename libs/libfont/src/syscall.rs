//! Raw syscall wrappers for libfont.dll.
//!
//! Uses the SYSCALL instruction directly (x86-64 fast path).
//! Convention: RAX=num, RBX=arg1, R10=arg2, RDX=arg3. Return in RAX.
//! Clobbers: RCX (user RIP), R11 (user RFLAGS).

use core::arch::asm;

// Syscall numbers (must match kernel/src/syscall/mod.rs)
const SYS_READ: u32 = 3;
const SYS_OPEN: u32 = 4;
const SYS_CLOSE: u32 = 5;
const SYS_SBRK: u32 = 9;
const SYS_FSTAT: u32 = 106;
const SYS_GPU_HAS_ACCEL: u32 = 135;

#[inline(always)]
fn syscall0(num: u32) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") num as u64 => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[inline(always)]
fn syscall1(num: u32, a1: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num as u64 => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[inline(always)]
fn syscall2(num: u32, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num as u64 => ret,
            in("r10") a2,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[inline(always)]
fn syscall3(num: u32, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num as u64 => ret,
            in("r10") a2,
            in("rdx") a3,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

/// Extend the process heap by `increment` bytes. Returns previous break address.
/// Returns `usize::MAX` on failure.
pub fn sbrk(increment: i32) -> usize {
    syscall1(SYS_SBRK, increment as u64) as usize
}

/// Open a file. Returns file descriptor or u32::MAX on error.
/// flags: 0 = read-only. Path is copied to a stack buffer and null-terminated
/// (kernel expects a C string, not a length-prefixed string).
pub fn open(path: &[u8], flags: u32) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path[..len]);
    buf[len] = 0;
    syscall2(SYS_OPEN, buf.as_ptr() as u64, flags as u64) as u32
}

/// Read from a file descriptor into buffer. Returns bytes read.
pub fn read(fd: u32, buf: *mut u8, count: u32) -> u32 {
    syscall3(SYS_READ, fd as u64, buf as u64, count as u64) as u32
}

/// Close a file descriptor.
pub fn close(fd: u32) {
    syscall1(SYS_CLOSE, fd as u64);
}

/// Get file status. Returns 0 on success.
/// stat_buf layout: [u32 file_type, u32 size, u32 position]
pub fn fstat(fd: u32, stat_buf: *mut u8) -> u32 {
    syscall2(SYS_FSTAT, fd as u64, stat_buf as u64) as u32
}

/// Query whether GPU acceleration is available.
pub fn gpu_has_accel() -> u32 {
    syscall0(SYS_GPU_HAS_ACCEL) as u32
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
    // Stat buffer layout: [0..4] = file_type (u32), [4..8] = size (u32), [8..12] = position (u32)
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
