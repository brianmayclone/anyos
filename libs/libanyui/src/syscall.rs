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
const SYS_SBRK: u64 = 9;
const SYS_UPTIME_MS: u64 = 35;
const SYS_EVT_CHAN_EMIT: u64 = 65;
const SYS_EVT_CHAN_POLL: u64 = 66;
const SYS_EVT_CHAN_WAIT: u64 = 70;

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

const SYS_WRITE: u64 = 2;

const SYS_DLL_LOAD: u64 = 80;
const SYS_READDIR: u64 = 23;
const SYS_GETCWD: u64 = 25;
const SYS_MKDIR: u64 = 90;

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

pub fn uptime_ms() -> u32 {
    syscall0(SYS_UPTIME_MS) as u32
}

/// Load/map a shared library by path. Returns the base virtual address, or 0 on failure.
pub fn dll_load(path: &[u8]) -> u64 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path[..len]);
    buf[len] = 0;
    syscall2(SYS_DLL_LOAD, buf.as_ptr() as u64, len as u64)
}

/// Read directory entries. `path` must be null-terminated.
/// Each entry is 64 bytes: [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes]
/// Returns number of entries, or u32::MAX on error.
pub fn readdir(path: &[u8], buf: &mut [u8]) -> u32 {
    syscall3(SYS_READDIR, path.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64) as u32
}

/// Get current working directory. Returns length or u32::MAX on error.
pub fn getcwd(buf: &mut [u8]) -> u32 {
    syscall2(SYS_GETCWD, buf.as_mut_ptr() as u64, buf.len() as u64) as u32
}

/// Create a directory. `path` must be null-terminated. Returns 0 on success.
pub fn mkdir(path: &[u8]) -> u32 {
    syscall1(SYS_MKDIR, path.as_ptr() as u64) as u32
}

/// Write bytes to a file descriptor. fd=1 for stdout (serial).
pub fn write(fd: u32, buf: &[u8]) -> u32 {
    syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64) as u32
}

/// Non-blocking poll: dequeue one event from a channel subscription.
/// Returns true if an event was available.
pub fn evt_chan_poll(channel_id: u32, sub_id: u32, buf: &mut [u32; 5]) -> bool {
    syscall3(SYS_EVT_CHAN_POLL, channel_id as u64, sub_id as u64, buf.as_mut_ptr() as u64) != 0
}

/// Block until an event is available on a channel subscription, or timeout.
///
/// Returns 1 if events are available, 0 on timeout/spurious wake.
pub fn evt_chan_wait(channel_id: u32, sub_id: u32, timeout_ms: u32) -> u32 {
    syscall3(SYS_EVT_CHAN_WAIT, channel_id as u64, sub_id as u64, timeout_ms as u64) as u32
}

/// Emit an event to a channel. `event` is a 5×u32 event payload.
pub fn evt_chan_emit(channel_id: u32, event: &[u32; 5]) {
    syscall2(SYS_EVT_CHAN_EMIT, channel_id as u64, event.as_ptr() as u64);
}
