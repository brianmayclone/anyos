//! Kernel syscall wrappers for libdb.
//!
//! Provides the minimal set of syscalls needed for file I/O and process control.
//! Uses the anyOS calling convention: RAX=num, RBX=arg1, R10=arg2, RDX=arg3.

use core::arch::asm;

// Syscall numbers (must match kernel/src/syscall/mod.rs)
const SYS_EXIT: u64 = 1;
const SYS_WRITE: u64 = 2;
const SYS_READ: u64 = 3;
const SYS_OPEN: u64 = 4;
const SYS_CLOSE: u64 = 5;
const SYS_SBRK: u64 = 9;
const SYS_MMAP: u64 = 14;
const SYS_MUNMAP: u64 = 15;
const SYS_LSEEK: u64 = 105;
const SYS_FSTAT: u64 = 106;

// Open flags (match kernel)
pub const O_WRITE: u32 = 1;
pub const O_CREATE: u32 = 4;
pub const O_TRUNC: u32 = 8;

// Seek whence
pub const SEEK_SET: u32 = 0;

// ── Raw syscall wrappers (SYSCALL instruction, anyOS convention) ─────────────

#[inline(always)]
fn syscall1(num: u64, a1: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[inline(always)]
fn syscall2(num: u64, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num => ret,
            in("r10") a2,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[inline(always)]
fn syscall3(num: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num => ret,
            in("r10") a2, in("rdx") a3,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

// ── High-level wrappers ──────────────────────────────────────────────────────

/// Exit the current process.
pub fn exit(code: u32) -> ! {
    syscall1(SYS_EXIT, code as u64);
    loop {}
}

/// Grow the heap. Returns new break address or u64::MAX on failure.
pub fn sbrk(increment: u32) -> u64 {
    syscall1(SYS_SBRK, increment as u64)
}

/// Map anonymous pages. Returns address or `u64::MAX` on failure.
pub fn mmap(size: u32) -> u64 {
    let ret = syscall1(SYS_MMAP, size as u64);
    if ret == u32::MAX as u64 { u64::MAX } else { ret }
}

/// Unmap pages previously mapped with `mmap`. Returns 0 on success.
pub fn munmap(addr: u64, size: u32) -> u64 {
    syscall2(SYS_MUNMAP, addr, size as u64)
}

/// Open a file. Returns fd or u32::MAX on error.
pub fn open(path: &str, flags: u32) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    let ret = syscall3(SYS_OPEN, buf.as_ptr() as u64, flags as u64, 0);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Close a file descriptor.
pub fn close(fd: u32) {
    syscall1(SYS_CLOSE, fd as u64);
}

/// Read from fd into buffer. Returns bytes read or u32::MAX.
pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    let ret = syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Write buffer to fd. Returns bytes written or u32::MAX.
pub fn write(fd: u32, buf: &[u8]) -> u32 {
    let ret = syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Seek within an open file. Returns new position or u32::MAX on error.
pub fn lseek(fd: u32, offset: i32, whence: u32) -> u32 {
    let ret = syscall3(SYS_LSEEK, fd as u64, offset as i64 as u64, whence as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Get file size via fstat. Returns file size or 0.
pub fn file_size(fd: u32) -> u32 {
    let mut stat_buf = [0u32; 4];
    let ret = syscall2(SYS_FSTAT, fd as u64, stat_buf.as_mut_ptr() as u64);
    if (ret as i64) < 0 { 0 } else { stat_buf[1] }
}

/// Write to stdout (fd=1) for debug logging.
pub fn log(msg: &[u8]) {
    write(1, msg);
}
