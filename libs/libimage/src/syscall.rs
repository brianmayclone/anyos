//! Raw syscall wrappers for libimage.dlib.

use core::arch::asm;

const SYS_EXIT: u32 = 1;
const SYS_WRITE: u32 = 2;
const SYS_SBRK: u32 = 9;
const SYS_MMAP: u32 = 14;
const SYS_MUNMAP: u32 = 15;

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
/// Returns `u64::MAX` on failure.
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

/// Write bytes to a file descriptor.
pub fn write(fd: u32, buf: &[u8]) {
    syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64);
}

/// Exit the current process.
pub fn exit(code: u32) -> ! {
    syscall1(SYS_EXIT, code as u64);
    loop {}
}

// ── File I/O (for pak cache) ─────────────────────────

const SYS_OPEN: u32 = 4;
const SYS_READ: u32 = 3;
const SYS_CLOSE: u32 = 5;
const SYS_FSTAT: u32 = 106;

/// Open a file. Returns fd or u32::MAX on error.
pub fn open(path: &[u8]) -> u32 {
    // Path must be null-terminated in a buffer
    let mut buf = [0u8; 128];
    let len = path.len().min(buf.len() - 1);
    buf[..len].copy_from_slice(&path[..len]);
    buf[len] = 0;
    let ret = syscall3(SYS_OPEN, buf.as_ptr() as u64, 0, 0);
    ret as u32
}

/// Read from fd into buf. Returns bytes read.
pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64) as u32
}

/// Close an fd.
pub fn close(fd: u32) {
    syscall1(SYS_CLOSE, fd as u64);
}

/// Get file stats. stat_buf: [type, size, flags, ...]. Returns 0 on success.
pub fn fstat(fd: u32, stat_buf: &mut [u32; 4]) -> u32 {
    let ret = syscall2(SYS_FSTAT, fd as u64, stat_buf.as_mut_ptr() as u64);
    ret as u32
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
            out("rdx") _,
        );
    }
    ret
}
