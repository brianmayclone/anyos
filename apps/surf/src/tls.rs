//! TLS support for Surf using BearSSL.
//!
//! Wraps the C-side `anyos_tls.c` which provides a high-level API:
//! `tls_connect`, `tls_send`, `tls_recv`, `tls_close`.
//!
//! The Rust side provides the TCP I/O callbacks (`anyos_tcp_send`,
//! `anyos_tcp_recv`, `anyos_sleep`) that the C wrapper calls.

use anyos_std::net;

// ---------------------------------------------------------------------------
// FFI: C → Rust callbacks (called by BearSSL I/O layer)
// ---------------------------------------------------------------------------

#[no_mangle]
extern "C" fn anyos_tcp_send(fd: i32, data: *const u8, len: i32) -> i32 {
    if data.is_null() || len <= 0 {
        return -1;
    }
    let buf = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let n = net::tcp_send(fd as u32, buf);
    if n == u32::MAX {
        -1
    } else {
        n as i32
    }
}

#[no_mangle]
extern "C" fn anyos_tcp_recv(fd: i32, data: *mut u8, len: i32) -> i32 {
    if data.is_null() || len <= 0 {
        return -1;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(data, len as usize) };
    let sock = fd as u32;

    // Match the more robust BearSSL callback path used by libhttp:
    // a transient tcp_recv timeout must not look like a hard TLS failure.
    // Retry a few times when the socket still appears alive.
    for _attempt in 0..3 {
        let n = net::tcp_recv(sock, buf);
        if n == 0 {
            return 0;
        }
        if n != u32::MAX {
            return n as i32;
        }

        let avail = net::tcp_recv_available(sock);
        match avail {
            u32::MAX => return -1,
            0xFFFF_FFFE => return 0,
            _ => anyos_std::process::sleep(100),
        }
    }

    -1
}

#[no_mangle]
extern "C" fn anyos_sleep(ms: i32) {
    anyos_std::process::sleep(ms as u32);
}

#[no_mangle]
extern "C" fn anyos_random(buf: *mut u8, len: i32) -> i32 {
    if buf.is_null() || len <= 0 {
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, len as usize) };
    anyos_std::sys::random(slice) as i32
}

// ---------------------------------------------------------------------------
// FFI: Rust → C wrapper functions
// ---------------------------------------------------------------------------

extern "C" {
    fn tls_connect(fd: i32, host: *const u8) -> i32;
    fn tls_send(handle: i32, data: *const u8, len: i32) -> i32;
    fn tls_recv(handle: i32, data: *mut u8, len: i32) -> i32;
    fn tls_close(handle: i32);
    fn tls_last_error(handle: i32) -> i32;
}

// ---------------------------------------------------------------------------
// Public Rust API
// ---------------------------------------------------------------------------

pub type TlsHandle = u32;

/// Establish a TLS connection over an existing TCP socket.
/// The socket must already be connected. `host` is used for SNI.
/// Returns a positive TLS handle on success, negative error code on failure.
pub fn connect(fd: u32, host: &str) -> i32 {
    // Create null-terminated host string on the stack.
    let mut host_buf = [0u8; 256];
    let len = host.len().min(host_buf.len() - 1);
    host_buf[..len].copy_from_slice(&host.as_bytes()[..len]);
    host_buf[len] = 0;
    unsafe { tls_connect(fd as i32, host_buf.as_ptr()) }
}

/// Send data over the TLS connection identified by `handle`.
/// Returns bytes sent on success, negative on error.
pub fn send(handle: TlsHandle, data: &[u8]) -> i32 {
    unsafe { tls_send(handle as i32, data.as_ptr(), data.len() as i32) }
}

/// Receive data from the TLS connection identified by `handle`.
/// Returns bytes read, 0 on EOF, negative on error.
pub fn recv(handle: TlsHandle, buf: &mut [u8]) -> i32 {
    unsafe { tls_recv(handle as i32, buf.as_mut_ptr(), buf.len() as i32) }
}

/// Close the TLS connection (sends close_notify).
pub fn close(handle: TlsHandle) {
    unsafe {
        tls_close(handle as i32);
    }
}

/// Get the last BearSSL error code.
pub fn last_error(handle: TlsHandle) -> i32 {
    unsafe { tls_last_error(handle as i32) }
}
