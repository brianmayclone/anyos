//! Raw syscall wrappers for libimage.dlib.

use core::arch::asm;

const SYS_SBRK: u32 = 9;
const SYS_EXIT: u32 = 1;
const SYS_WRITE: u32 = 2;

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
pub fn sbrk(increment: i32) -> usize {
    syscall1(SYS_SBRK, increment as u64) as usize
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
