//! Kernel syscall wrappers for libgl.
//!
//! Minimal syscall set: sbrk for heap allocation, exit for panic handler.

use core::arch::asm;

const SYS_EXIT: u64 = 1;
const SYS_SBRK: u64 = 9;

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

/// Exit the current process.
pub fn exit(code: u32) -> ! {
    syscall1(SYS_EXIT, code as u64);
    loop {}
}

/// Grow the heap. Returns new break address or u64::MAX on failure.
pub fn sbrk(increment: u32) -> u64 {
    syscall1(SYS_SBRK, increment as u64)
}
