// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Host-mode process functions — delegates to std.

use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_FAKE_PID: AtomicU32 = AtomicU32::new(10_000);

pub const STILL_RUNNING: u32 = u32::MAX - 1;

pub fn exit(code: u32) -> ! {
    std::process::exit(code as i32);
}

pub fn getpid() -> u32 {
    std::process::id()
}

pub fn sleep(ms: u32) {
    std::thread::sleep(std::time::Duration::from_millis(ms as u64));
}

pub fn sleep_us(us: u32) {
    std::thread::sleep(std::time::Duration::from_micros(us as u64));
}

pub fn sbrk(_increment: i32) -> usize {
    0 // Not meaningful on host
}

pub fn mmap(size: usize) -> *mut u8 {
    let layout = std::alloc::Layout::from_size_align(size, 4096).unwrap();
    unsafe { std::alloc::alloc_zeroed(layout) }
}

pub fn munmap(addr: *mut u8, size: usize) -> bool {
    let layout = std::alloc::Layout::from_size_align(size, 4096).unwrap();
    unsafe {
        std::alloc::dealloc(addr, layout);
    }
    true
}

pub fn yield_cpu() {
    std::thread::yield_now();
}

pub fn spawn(_path: &str, _args: &str) -> u32 {
    u32::MAX // Not supported on host
}

pub fn spawn_piped(_path: &str, _args: &str, _pipe_fd: u32) -> u32 {
    NEXT_FAKE_PID.fetch_add(1, Ordering::Relaxed)
}

pub fn spawn_piped_full(_path: &str, _args: &str, _stdout_pipe: u32, _stdin_pipe: u32) -> u32 {
    NEXT_FAKE_PID.fetch_add(1, Ordering::Relaxed)
}

pub fn launch_app(_path: &str, _args: &str) -> u32 {
    u32::MAX
}

pub fn fork() -> u32 {
    u32::MAX
}

pub fn exec(_path: &str, _args: &str) -> u32 {
    u32::MAX
}

pub fn waitpid(_tid: u32) -> u32 {
    u32::MAX
}

pub fn try_waitpid(_tid: u32) -> u32 {
    u32::MAX
}

pub fn kill(_tid: u32) -> u32 {
    u32::MAX
}

pub fn send_signal(_tid: u32, _sig: u32) -> u32 {
    u32::MAX
}

pub fn getargs(buf: &mut [u8]) -> usize {
    let args: Vec<std::string::String> = std::env::args().collect();
    let joined = quote_join_args(&args);
    let bytes = joined.as_bytes();
    let len = bytes.len().min(buf.len());
    buf[..len].copy_from_slice(&bytes[..len]);
    len
}

pub fn args(buf: &mut [u8]) -> &str {
    let args: Vec<std::string::String> = std::env::args().skip(1).collect();
    let joined = quote_join_args(&args);
    let bytes = joined.as_bytes();
    let len = bytes.len().min(buf.len().saturating_sub(1));
    buf[..len].copy_from_slice(&bytes[..len]);
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}

fn quote_join_args(args: &[std::string::String]) -> std::string::String {
    let mut out = std::string::String::new();
    for (idx, arg) in args.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        if arg
            .as_bytes()
            .iter()
            .any(|b| matches!(*b, b' ' | b'\t' | b'"' | b'\'' | b'\\'))
        {
            out.push('"');
            for ch in arg.chars() {
                if ch == '"' || ch == '\\' {
                    out.push('\\');
                }
                out.push(ch);
            }
            out.push('"');
        } else {
            out.push_str(arg);
        }
    }
    out
}

pub fn shutdown() -> ! {
    std::process::exit(0);
}

pub fn reboot() -> ! {
    std::process::exit(0);
}

pub const SIGTSTP: u32 = 20;
pub const SIGCONT: u32 = 18;
pub const STOPPED: u32 = u32::MAX - 2;
