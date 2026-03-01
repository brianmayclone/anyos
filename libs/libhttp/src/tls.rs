//! TLS support for libhttp using BearSSL.
//!
//! Wraps the C-side `anyos_tls.c` which provides a high-level API:
//! `tls_connect`, `tls_send`, `tls_recv`, `tls_close`.
//!
//! The Rust side provides the TCP I/O callbacks (`anyos_tcp_send`,
//! `anyos_tcp_recv`, `anyos_sleep`, `anyos_random`) that the C wrapper calls.

use crate::syscall;

// ---------------------------------------------------------------------------
// FFI: C → Rust callbacks (called by BearSSL I/O layer)
// ---------------------------------------------------------------------------

/// TCP send callback for BearSSL.
#[no_mangle]
extern "C" fn anyos_tcp_send(fd: i32, data: *const u8, len: i32) -> i32 {
    if data.is_null() || len <= 0 {
        return -1;
    }
    let buf = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let n = syscall::tcp_send(fd as u32, buf);
    if n == u32::MAX { -1 } else { n as i32 }
}

/// TCP receive callback for BearSSL.
///
/// Retries on timeout (tcp_recv returns u32::MAX) by checking whether
/// the connection is still alive via `tcp_recv_available`. This prevents
/// a transient TCP timeout from killing the entire TLS session, which
/// would make large HTTPS downloads fail mid-stream.
#[no_mangle]
extern "C" fn anyos_tcp_recv(fd: i32, data: *mut u8, len: i32) -> i32 {
    if data.is_null() || len <= 0 {
        return -1;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(data, len as usize) };
    let sock = fd as u32;

    for _attempt in 0..3 {
        let n = syscall::tcp_recv(sock, buf);
        if n == 0 {
            return 0; // EOF (FIN received)
        }
        if n != u32::MAX {
            return n as i32; // Got data
        }

        // tcp_recv returned u32::MAX — could be timeout or real error.
        // Check connection state to decide whether to retry.
        let avail = syscall::tcp_recv_available(sock);
        match avail {
            u32::MAX => return -1,       // True error (reset, invalid socket)
            0xFFFFFFFE => return 0,      // EOF (u32::MAX - 1)
            _ => {
                // Connection still alive (avail == 0 or >0). Retry.
                // Brief sleep to let more data arrive.
                syscall::sleep(100);
            }
        }
    }

    // All retries exhausted
    -1
}

/// Sleep callback for BearSSL.
#[no_mangle]
extern "C" fn anyos_sleep(ms: i32) {
    syscall::sleep(ms as u32);
}

/// Random bytes callback for BearSSL.
#[no_mangle]
extern "C" fn anyos_random(buf: *mut u8, len: i32) -> i32 {
    if buf.is_null() || len <= 0 {
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, len as usize) };
    syscall::random(slice) as i32
}

// ---------------------------------------------------------------------------
// FFI: Rust → C wrapper functions
// ---------------------------------------------------------------------------

extern "C" {
    fn tls_connect(fd: i32, host: *const u8) -> i32;
    fn tls_send(data: *const u8, len: i32) -> i32;
    fn tls_recv(data: *mut u8, len: i32) -> i32;
    fn tls_close();
    fn tls_last_error() -> i32;
}

// ---------------------------------------------------------------------------
// Public Rust API
// ---------------------------------------------------------------------------

/// Establish a TLS connection over an existing TCP socket.
/// The socket must already be connected. `host` is used for SNI.
/// Returns 0 on success, negative error code on failure.
pub fn connect(fd: u32, host: &str) -> i32 {
    let mut host_buf = [0u8; 256];
    let len = host.len().min(host_buf.len() - 1);
    host_buf[..len].copy_from_slice(&host.as_bytes()[..len]);
    host_buf[len] = 0;
    unsafe { tls_connect(fd as i32, host_buf.as_ptr()) }
}

/// Send data over the TLS connection.
/// Returns bytes sent on success, negative on error.
pub fn send(data: &[u8]) -> i32 {
    unsafe { tls_send(data.as_ptr(), data.len() as i32) }
}

/// Receive data from the TLS connection.
/// Returns bytes read, 0 on EOF, negative on error.
pub fn recv(buf: &mut [u8]) -> i32 {
    unsafe { tls_recv(buf.as_mut_ptr(), buf.len() as i32) }
}

/// Close the TLS connection (sends close_notify).
pub fn close() {
    unsafe { tls_close(); }
}

/// Get the last BearSSL error code.
pub fn last_error() -> i32 {
    unsafe { tls_last_error() }
}
