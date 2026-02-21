//! Self-contained syscall wrappers for libanyui.
//!
//! libanyui is a shared library (.so) and cannot depend on stdlib at link time.
//! These wrappers use inline assembly to invoke the SYSCALL instruction directly.
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
const SYS_WIN_CREATE: u64 = 50;
const SYS_WIN_DESTROY: u64 = 51;
const SYS_WIN_GET_EVENT: u64 = 53;
const SYS_WIN_FILL_RECT: u64 = 54;
const SYS_WIN_PRESENT: u64 = 56;
const SYS_WIN_BLIT: u64 = 59;

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
            in("r10") a2,
            in("rdx") a3,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[inline(always)]
fn syscall5(num: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
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
            in("rdx") a3,
            in("rsi") a4,
            in("rdi") a5,
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

/// Create a compositor window. Returns window handle.
pub fn win_create(title: &[u8], x: u32, y: u32, w: u32, h: u32) -> u32 {
    // SYS_WIN_CREATE: arg1=title_ptr, arg2=title_len, arg3=packed(x,y), arg4=w, arg5=h
    let packed_xy = ((x as u64) << 32) | (y as u64);
    syscall5(
        SYS_WIN_CREATE,
        title.as_ptr() as u64,
        title.len() as u64,
        packed_xy,
        w as u64,
        h as u64,
    ) as u32
}

pub fn win_destroy(win: u32) {
    syscall1(SYS_WIN_DESTROY, win as u64);
}

/// Poll one event from a window. Returns 1 if event available.
pub fn win_get_event(win: u32, buf: &mut [u32; 5]) -> u32 {
    syscall2(SYS_WIN_GET_EVENT, win as u64, buf.as_mut_ptr() as u64) as u32
}

/// Fill a rectangle in the window.
pub fn win_fill_rect(win: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    // SYS_WIN_FILL_RECT: arg1=win, arg2=packed(x,y), arg3=packed(w,h), arg4=color
    let packed_xy = ((x as u32 as u64) << 32) | (y as u32 as u64);
    let packed_wh = ((w as u64) << 32) | (h as u64);
    syscall5(
        SYS_WIN_FILL_RECT,
        win as u64,
        packed_xy,
        packed_wh,
        color as u64,
        0,
    );
}

/// Present (flip) the window to the screen.
pub fn win_present(win: u32) {
    syscall1(SYS_WIN_PRESENT, win as u64);
}

/// Blit ARGB pixel data to window content surface.
pub fn win_blit(win: u32, x: i32, y: i32, w: u32, h: u32, data: &[u32]) {
    // SYS_WIN_BLIT: arg1=win, arg2=packed(x,y), arg3=packed(w,h), arg4=data_ptr, arg5=data_len
    let packed_xy = ((x as u32 as u64) << 32) | (y as u32 as u64);
    let packed_wh = ((w as u64) << 32) | (h as u64);
    syscall5(
        SYS_WIN_BLIT,
        win as u64,
        packed_xy,
        packed_wh,
        data.as_ptr() as u64,
        data.len() as u64,
    );
}
