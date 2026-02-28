//! Kernel syscall wrappers for libhttp.
//!
//! Provides raw syscall access for networking (DNS, TCP), file I/O,
//! memory management (sbrk), sleep, and random number generation.

use core::arch::asm;

const SYS_EXIT: u64 = 1;
const SYS_WRITE: u64 = 2;
const SYS_READ: u64 = 3;
const SYS_OPEN: u64 = 4;
const SYS_CLOSE: u64 = 5;
const SYS_SLEEP: u64 = 8;
const SYS_SBRK: u64 = 9;
const SYS_NET_DNS: u64 = 43;
const SYS_TCP_CONNECT: u64 = 100;
const SYS_TCP_SEND: u64 = 101;
const SYS_TCP_RECV: u64 = 102;
const SYS_TCP_CLOSE: u64 = 103;
const SYS_FSTAT: u64 = 106;
const SYS_RANDOM: u64 = 210;

pub const O_WRITE: u32 = 1;
pub const O_CREATE: u32 = 4;
pub const O_TRUNC: u32 = 8;

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

// ── Process ──────────────────────────────────────────────────────────────────

/// Terminate the process with the given exit code.
pub fn exit(code: u32) -> ! {
    syscall1(SYS_EXIT, code as u64);
    loop {}
}

/// Sleep for `ms` milliseconds.
pub fn sleep(ms: u32) {
    syscall1(SYS_SLEEP, ms as u64);
}

// ── Memory ───────────────────────────────────────────────────────────────────

/// Extend the process heap by `increment` bytes.
/// Returns the new break address, or `u64::MAX` on failure.
pub fn sbrk(increment: u32) -> u64 {
    syscall1(SYS_SBRK, increment as u64)
}

// ── File I/O ─────────────────────────────────────────────────────────────────

/// Open a file. Returns fd on success, `u32::MAX` on error.
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

/// Read from a file descriptor. Returns bytes read, or `u32::MAX` on error.
pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    let ret = syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Write to a file descriptor. Returns bytes written, or `u32::MAX` on error.
pub fn write(fd: u32, buf: &[u8]) -> u32 {
    let ret = syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Get file size via fstat. Returns size or 0 on error.
pub fn file_size(fd: u32) -> u32 {
    let mut stat_buf = [0u32; 4];
    let ret = syscall2(SYS_FSTAT, fd as u64, stat_buf.as_mut_ptr() as u64);
    if (ret as i64) < 0 { 0 } else { stat_buf[1] }
}

// ── Networking ───────────────────────────────────────────────────────────────

/// Resolve a hostname to an IPv4 address.
/// Returns 0 on success, non-zero on failure.
pub fn dns_resolve(hostname: &str, result: &mut [u8; 4]) -> u32 {
    let mut host_buf = [0u8; 257];
    let len = hostname.len().min(256);
    host_buf[..len].copy_from_slice(&hostname.as_bytes()[..len]);
    host_buf[len] = 0;
    let ret = syscall2(SYS_NET_DNS, host_buf.as_ptr() as u64, result.as_mut_ptr() as u64);
    ret as u32
}

/// Connect to a TCP server. Returns socket id, or `u32::MAX` on error.
pub fn tcp_connect(ip: &[u8; 4], port: u16, timeout_ms: u32) -> u32 {
    #[repr(C, packed)]
    struct TcpConnectParams {
        ip: [u8; 4],
        port: u16,
        _pad: u16,
        timeout: u32,
    }
    let params = TcpConnectParams {
        ip: *ip,
        port,
        _pad: 0,
        timeout: timeout_ms,
    };
    let ret = syscall1(SYS_TCP_CONNECT, &params as *const _ as u64);
    if (ret as i64) < 0 || ret == u64::MAX { u32::MAX } else { ret as u32 }
}

/// Send data over a TCP socket. Returns bytes sent, or `u32::MAX` on error.
pub fn tcp_send(sock: u32, data: &[u8]) -> u32 {
    let ret = syscall3(SYS_TCP_SEND, sock as u64, data.as_ptr() as u64, data.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Receive data from a TCP socket. Returns bytes received, 0 on EOF, `u32::MAX` on error.
pub fn tcp_recv(sock: u32, buf: &mut [u8]) -> u32 {
    let ret = syscall3(SYS_TCP_RECV, sock as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Close a TCP socket.
pub fn tcp_close(sock: u32) {
    syscall1(SYS_TCP_CLOSE, sock as u64);
}

// ── Random ───────────────────────────────────────────────────────────────────

/// Fill buffer with random bytes. Returns bytes written.
pub fn random(buf: &mut [u8]) -> u32 {
    let len = buf.len().min(256);
    let ret = syscall2(SYS_RANDOM, buf.as_mut_ptr() as u64, len as u64);
    ret as u32
}

// ── Debug logging ────────────────────────────────────────────────────────────

/// Write a message to stdout (fd 1).
pub fn log(msg: &[u8]) {
    write(1, msg);
}
