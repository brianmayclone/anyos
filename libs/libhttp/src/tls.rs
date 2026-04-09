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
    let n = syscall::tcp_send(fd, data);
    if n == u32::MAX { -1 } else { n as i32 }
}

fn tcp_recv(fd: u32, buf: &mut [u8]) -> i32 {
    let sock = fd;
    for _attempt in 0..3 {
        let n = syscall::tcp_recv(sock, buf);
        if n == 0 {
            return 0;
        }
        if n != u32::MAX {
            return n as i32;
        }
        let avail = syscall::tcp_recv_available(sock);
        match avail {
            u32::MAX => return -1,
            0xFFFFFFFE => return 0,
            _ => syscall::sleep(100),
        }
    }
    -1
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
