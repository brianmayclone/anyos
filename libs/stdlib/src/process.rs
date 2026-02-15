//! Process management — exit, spawn, wait, sleep, etc.

use core::arch::asm;
use crate::raw::*;

pub fn exit(code: u32) -> ! {
    unsafe {
        asm!(
            "mov rbx, {code}",
            "int 0x80",
            code = in(reg) code as u64,
            in("rax") SYS_EXIT as u64,
            options(noreturn)
        );
    }
}

pub fn getpid() -> u32 {
    syscall0(SYS_GETPID)
}

pub fn yield_cpu() {
    syscall0(SYS_YIELD);
}

pub fn sleep(ms: u32) {
    syscall1(SYS_SLEEP, ms as u64);
}

pub fn sbrk(increment: i32) -> usize {
    syscall1(SYS_SBRK, increment as i64 as u64) as usize
}

pub fn waitpid(tid: u32) -> u32 {
    syscall1(SYS_WAITPID, tid as u64)
}

/// Non-blocking check if process exited.
/// Returns exit code if terminated, u32::MAX if not found, STILL_RUNNING if alive.
pub const STILL_RUNNING: u32 = u32::MAX - 1;
pub fn try_waitpid(tid: u32) -> u32 {
    syscall1(SYS_TRY_WAITPID, tid as u64)
}

/// Kill a thread by TID. Returns 0 on success, u32::MAX on failure.
pub fn kill(tid: u32) -> u32 {
    syscall1(SYS_KILL, tid as u64)
}

/// Spawn a new process. Returns TID or u32::MAX on error.
pub fn spawn(path: &str, args: &str) -> u32 {
    spawn_piped(path, args, 0)
}

/// Spawn a new process with stdout redirected to a pipe.
/// pipe_id=0 means no pipe (stdout goes to serial only).
/// Returns TID or u32::MAX on error.
pub fn spawn_piped(path: &str, args: &str, pipe_id: u32) -> u32 {
    let mut path_buf = [0u8; 257];
    let plen = path.len().min(256);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    path_buf[plen] = 0;

    let mut args_buf = [0u8; 257];
    let alen = args.len().min(256);
    args_buf[..alen].copy_from_slice(&args.as_bytes()[..alen]);
    args_buf[alen] = 0;

    let args_ptr = if args.is_empty() { 0u64 } else { args_buf.as_ptr() as u64 };
    syscall3(SYS_SPAWN, path_buf.as_ptr() as u64, pipe_id as u64, args_ptr)
}

/// Spawn a new process with both stdout and stdin redirected to pipes.
/// stdout_pipe=0 means no stdout pipe, stdin_pipe=0 means no stdin pipe.
/// Returns TID or u32::MAX on error.
pub fn spawn_piped_full(path: &str, args: &str, stdout_pipe: u32, stdin_pipe: u32) -> u32 {
    let mut path_buf = [0u8; 257];
    let plen = path.len().min(256);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    path_buf[plen] = 0;

    let mut args_buf = [0u8; 257];
    let alen = args.len().min(256);
    args_buf[..alen].copy_from_slice(&args.as_bytes()[..alen]);
    args_buf[alen] = 0;

    let args_ptr = if args.is_empty() { 0u64 } else { args_buf.as_ptr() as u64 };
    syscall4(SYS_SPAWN, path_buf.as_ptr() as u64, stdout_pipe as u64, args_ptr, stdin_pipe as u64)
}

/// Create a new thread in the current process, sharing the same address space.
///
/// `entry` is a function pointer for the new thread's entry point.
/// `stack_top` is the top of a user-allocated stack (must be 8-byte aligned,
/// and the caller should subtract 8 from the true top for ABI alignment).
/// `name` is a human-readable thread name (max 31 chars, shown in task manager/logs).
///
/// Returns the TID of the new thread, or 0 on error.
/// The new thread inherits the parent's priority.
pub fn thread_create(entry: fn(), stack_top: usize, name: &str) -> u32 {
    thread_create_with_priority(entry, stack_top, name, 0)
}

/// Create a new thread with an explicit priority (1-255, higher = more CPU time).
/// Priority 0 inherits from the parent thread.
pub fn thread_create_with_priority(entry: fn(), stack_top: usize, name: &str, priority: u8) -> u32 {
    syscall5(
        SYS_THREAD_CREATE,
        entry as u64,
        stack_top as u64,
        name.as_ptr() as u64,
        name.len() as u64,
        priority as u64,
    )
}

/// Set the priority of a thread (1-255, higher = more CPU time).
/// `tid` = 0 means the calling thread itself.
/// Returns 0 on success.
pub fn set_priority(tid: u32, priority: u8) -> u32 {
    syscall2(SYS_SET_PRIORITY, tid as u64, priority as u64)
}

/// Get command-line arguments (raw). Returns the args length.
/// The raw args string includes argv[0] (the program name).
pub fn getargs(buf: &mut [u8]) -> usize {
    syscall2(SYS_GETARGS, buf.as_mut_ptr() as u64, buf.len() as u64) as usize
}

/// Get command-line arguments, skipping argv[0] (the program name).
/// Returns the argument portion of the args string (after the first space).
pub fn args(buf: &mut [u8; 256]) -> &str {
    let len = getargs(buf);
    let all = core::str::from_utf8(&buf[..len]).unwrap_or("");
    match all.find(' ') {
        Some(idx) => all[idx + 1..].trim_start(),
        None => "",
    }
}
