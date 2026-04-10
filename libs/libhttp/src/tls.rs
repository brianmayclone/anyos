//! TLS support for libhttp using libtls (pure Rust).
//!
//! Replaces the former BearSSL C wrapper with the native Rust TLS library.
//! Provides the same public API: connect, send, recv, close, last_error.

use crate::syscall;

pub type TlsHandle = u32;

/// Initialize libtls transport callbacks. Called lazily on first use.
fn ensure_initialized() {
    use core::sync::atomic::{AtomicBool, Ordering};
    static INITIALIZED: AtomicBool = AtomicBool::new(false);
    if !INITIALIZED.swap(true, Ordering::SeqCst) {
        libtls::set_transport(tcp_send, tcp_recv, sleep, random);
    }
}

fn tcp_send(fd: u32, data: &[u8]) -> i32 {
    // Single attempt — libtls handles partial writes via send_all().
    let n = syscall::tcp_send(fd, data);
    if n == u32::MAX { -1 } else { n as i32 }
}

fn tcp_recv(fd: u32, buf: &mut [u8]) -> i32 {
    // First try: blocking recv (kernel has 30s timeout).
    let n = syscall::tcp_recv(fd, buf);
    if n == 0 { return 0; }          // EOF
    if n != u32::MAX { return n as i32; } // Got data

    // Timeout — check if connection is still alive.
    let avail = syscall::tcp_recv_available(fd);
    match avail {
        u32::MAX => -1,          // Connection error
        0xFFFFFFFE => 0,         // EOF (remote closed)
        _ => {
            // Connection alive but no data yet — one more try after brief delay.
            syscall::sleep(50);
            let n = syscall::tcp_recv(fd, buf);
            if n == 0 { 0 }
            else if n != u32::MAX { n as i32 }
            else { -1 }
        }
    }
}

fn sleep(ms: u32) {
    syscall::sleep(ms);
}

fn random(buf: &mut [u8]) -> u32 {
    syscall::random(buf) as u32
}

/// Establish a TLS connection over an existing TCP socket.
pub fn connect(fd: u32, host: &str) -> i32 {
    ensure_initialized();
    libtls::connect(fd, host)
}

/// Send data over the TLS connection.
pub fn send(handle: TlsHandle, data: &[u8]) -> i32 {
    libtls::send(handle, data)
}

/// Receive data from the TLS connection.
pub fn recv(handle: TlsHandle, buf: &mut [u8]) -> i32 {
    libtls::recv(handle, buf)
}

/// Close the TLS connection.
pub fn close(handle: TlsHandle) {
    libtls::close(handle);
}

/// Get the last error code.
pub fn last_error(handle: TlsHandle) -> i32 {
    libtls::last_error(handle)
}
