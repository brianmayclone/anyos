//! Kernel syscall wrappers for libanyui.
//!
//! Only actual kernel syscalls live here (exit, yield, sleep, sbrk).
//! Window/compositor operations use libcompositor.dlib via compositor.rs.
//!
//! Convention (matches kernel):
//!   RAX = syscall number
//!   RBX = arg1, R10 = arg2, RDX = arg3, RSI = arg4, RDI = arg5
//!   Return in RAX. Clobbers RCX, R11.

use core::arch::asm;

// Syscall numbers (must match kernel/src/syscall/mod.rs)
const SYS_EXIT: u64 = 1;
const SYS_YIELD: u64 = 7;
const SYS_SLEEP: u64 = 8;
const SYS_SBRK: u64 = 10;

#[inline(always)]
fn syscall0(num: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") num => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

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

// ── High-level wrappers ──────────────────────────────────────────────

pub fn exit(code: u32) -> ! {
    syscall1(SYS_EXIT, code as u64);
    loop {}
}

pub fn yield_cpu() {
    syscall0(SYS_YIELD);
}

pub fn sleep(ms: u32) {
    syscall1(SYS_SLEEP, ms as u64);
}

pub fn sbrk(increment: u32) -> u64 {
    syscall1(SYS_SBRK, increment as u64)
}
