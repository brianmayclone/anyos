//! Process management — exit, spawn, wait, sleep, etc.

use core::arch::asm;
use crate::raw::*;

pub fn exit(code: u32) -> ! {
    unsafe {
        asm!("int 0x80", in("eax") SYS_EXIT, in("ebx") code, options(noreturn));
    }
}

pub fn getpid() -> u32 {
    syscall0(SYS_GETPID)
}

pub fn yield_cpu() {
    syscall0(SYS_YIELD);
}

pub fn sleep(ms: u32) {
    syscall1(SYS_SLEEP, ms);
}

pub fn sbrk(increment: i32) -> u32 {
    syscall1(SYS_SBRK, increment as u32)
}

pub fn waitpid(tid: u32) -> u32 {
    syscall1(SYS_WAITPID, tid)
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

    let args_ptr = if args.is_empty() { 0u32 } else { args_buf.as_ptr() as u32 };
    syscall3(SYS_SPAWN, path_buf.as_ptr() as u32, pipe_id, args_ptr)
}

/// Get command-line arguments. Returns the args length.
pub fn getargs(buf: &mut [u8]) -> usize {
    syscall2(SYS_GETARGS, buf.as_mut_ptr() as u32, buf.len() as u32) as usize
}
