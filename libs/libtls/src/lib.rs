//! libtls -- Pure Rust TLS 1.3/1.2 implementation for anyOS.
//!
//! Replaces BearSSL with a fully Rust-based TLS stack supporting:
//! - TLS 1.3 (RFC 8446) -- preferred
//! - TLS 1.2 (RFC 5246) -- fallback
//!
//! Cipher suites:
//! - TLS_AES_128_GCM_SHA256 (0x1301)
//! - TLS_AES_256_GCM_SHA384 (0x1302)
//! - TLS_CHACHA20_POLY1305_SHA256 (0x1303)
//! - TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 (0xC02F) -- TLS 1.2
//!
//! Key exchange: X25519 (primary), ECDHE-P256 (fallback)

#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

pub mod crypto;
pub mod error;
pub mod record;
pub mod cipher_suite;
pub mod connection;
pub mod x509;
pub mod handshake;

use alloc::vec::Vec;
use error::TlsError;
use connection::TlsConnection;

// ---------------------------------------------------------------------------
// Transport callbacks (set by consumer)
// ---------------------------------------------------------------------------

/// Send data over TCP. Returns bytes sent, negative on error.
pub type SendFn = fn(fd: u32, data: &[u8]) -> i32;
/// Receive data from TCP. Returns bytes read, 0=EOF, negative=error.
pub type RecvFn = fn(fd: u32, buf: &mut [u8]) -> i32;
/// Sleep for the given number of milliseconds.
pub type SleepFn = fn(ms: u32);
/// Fill buffer with random bytes. Returns bytes written.
pub type RandomFn = fn(buf: &mut [u8]) -> u32;

struct TransportCallbacks {
    send: SendFn,
    recv: RecvFn,
    sleep: SleepFn,
    random: RandomFn,
}

// Default stubs that return errors (must be replaced before use)
fn default_send(_fd: u32, _data: &[u8]) -> i32 { -1 }
fn default_recv(_fd: u32, _buf: &mut [u8]) -> i32 { -1 }
fn default_sleep(_ms: u32) {}
fn default_random(buf: &mut [u8]) -> u32 {
    // Insecure fallback -- must be replaced by consumer
    for b in buf.iter_mut() { *b = 0; }
    buf.len() as u32
}

static mut TRANSPORT: TransportCallbacks = TransportCallbacks {
    send: default_send,
    recv: default_recv,
    sleep: default_sleep,
    random: default_random,
};

/// Register transport callbacks. Must be called once before any TLS operation.
pub fn set_transport(send: SendFn, recv: RecvFn, sleep: SleepFn, random: RandomFn) {
    unsafe {
        TRANSPORT = TransportCallbacks { send, recv, sleep, random };
    }
}

pub(crate) fn transport_send(fd: u32, data: &[u8]) -> i32 {
    unsafe { (TRANSPORT.send)(fd, data) }
}

pub(crate) fn transport_recv(fd: u32, buf: &mut [u8]) -> i32 {
    unsafe { (TRANSPORT.recv)(fd, buf) }
}

pub(crate) fn transport_sleep(ms: u32) {
    unsafe { (TRANSPORT.sleep)(ms) }
}

pub(crate) fn transport_random(buf: &mut [u8]) -> u32 {
    unsafe { (TRANSPORT.random)(buf) }
}

// ---------------------------------------------------------------------------
// Slot table (matches BearSSL's 16-connection model)
// ---------------------------------------------------------------------------

const MAX_TLS_CONTEXTS: usize = 16;

pub type TlsHandle = u32;

struct TlsSlot {
    conn: TlsConnection,
}

static mut TLS_SLOTS: [Option<TlsSlot>; MAX_TLS_CONTEXTS] = [
    None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None,
];

fn alloc_slot() -> Option<usize> {
    unsafe {
        for i in 0..MAX_TLS_CONTEXTS {
            if TLS_SLOTS[i].is_none() {
                return Some(i);
            }
        }
    }
    None
}

fn get_slot(handle: TlsHandle) -> Option<&'static mut TlsSlot> {
    if handle == 0 || handle as usize > MAX_TLS_CONTEXTS {
        return None;
    }
    unsafe { TLS_SLOTS[(handle - 1) as usize].as_mut() }
}

fn free_slot(handle: TlsHandle) {
    if handle > 0 && (handle as usize) <= MAX_TLS_CONTEXTS {
        unsafe { TLS_SLOTS[(handle - 1) as usize] = None; }
    }
}

// ---------------------------------------------------------------------------
// Public API (compatible with BearSSL wrapper)
// ---------------------------------------------------------------------------

/// Establish a TLS connection over an existing TCP socket.
/// The socket must already be connected. `host` is used for SNI.
/// Returns a positive TLS handle on success, negative error code on failure.
pub fn connect(fd: u32, host: &str) -> i32 {
    let idx = match alloc_slot() {
        Some(i) => i,
        None => return -(TlsError::NoSlotsAvailable as i32),
    };

    match TlsConnection::connect(fd, host) {
        Ok(conn) => {
            unsafe {
                TLS_SLOTS[idx] = Some(TlsSlot { conn });
            }
            (idx + 1) as i32
        }
        Err(e) => {
            -(e as i32)
        }
    }
}

/// Send data over the TLS connection identified by `handle`.
/// Returns bytes sent on success, negative on error.
pub fn send(handle: TlsHandle, data: &[u8]) -> i32 {
    match get_slot(handle) {
        Some(slot) => slot.conn.send(data),
        None => -1,
    }
}

/// Receive data from the TLS connection identified by `handle`.
/// Returns bytes read, 0 on EOF, negative on error.
pub fn recv(handle: TlsHandle, buf: &mut [u8]) -> i32 {
    match get_slot(handle) {
        Some(slot) => slot.conn.recv(buf),
        None => -1,
    }
}

/// Close the TLS connection (sends close_notify).
pub fn close(handle: TlsHandle) {
    if let Some(slot) = get_slot(handle) {
        slot.conn.close();
    }
    free_slot(handle);
}

/// Get the last error code for a connection.
pub fn last_error(handle: TlsHandle) -> i32 {
    match get_slot(handle) {
        Some(slot) => slot.conn.last_error() as i32,
        None => -1,
    }
}
