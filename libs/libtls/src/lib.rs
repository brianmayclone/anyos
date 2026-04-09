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
// Connection table — dynamic, thread-safe, unlimited connections.
//
// Each connection gets a unique handle (1-based u32). Connections are stored
// in a Vec protected by an atomic spinlock. Handles are never reused within
// the same process lifetime to prevent use-after-free races.
// ---------------------------------------------------------------------------

pub type TlsHandle = u32;

use core::sync::atomic::{AtomicBool, Ordering};

/// Spinlock for the connection table. Fine-grained: only held during
/// insert/remove/lookup, never during I/O or handshake.
static TABLE_LOCK: AtomicBool = AtomicBool::new(false);

fn lock_table() {
    while TABLE_LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}

fn unlock_table() {
    TABLE_LOCK.store(false, Ordering::Release);
}

struct ConnEntry {
    handle: TlsHandle,
    conn: TlsConnection,
}

/// The connection table. Entries are never moved; removal sets the slot to None.
/// New connections append or fill a None gap.
static mut CONN_TABLE: Option<Vec<Option<ConnEntry>>> = None;
/// Monotonically increasing handle counter. Never wraps in practice.
static mut NEXT_HANDLE: TlsHandle = 1;

/// Ensure the connection table is initialized.
fn ensure_table() {
    unsafe {
        if CONN_TABLE.is_none() {
            CONN_TABLE = Some(Vec::with_capacity(32));
        }
    }
}

/// Allocate a handle and insert a connection. Returns the handle.
fn insert_conn(conn: TlsConnection) -> TlsHandle {
    lock_table();
    ensure_table();
    let handle = unsafe { NEXT_HANDLE };
    unsafe { NEXT_HANDLE += 1; }

    let entry = ConnEntry { handle, conn };
    let table = unsafe { CONN_TABLE.as_mut().unwrap() };

    // Try to fill a None gap first
    for slot in table.iter_mut() {
        if slot.is_none() {
            *slot = Some(entry);
            unlock_table();
            return handle;
        }
    }
    // No gap — append
    table.push(Some(entry));
    unlock_table();
    handle
}

/// Get a mutable reference to a connection by handle.
/// The caller must NOT hold the table lock while using the connection
/// (I/O operations can block and must not prevent other threads from
/// allocating/freeing connections).
fn with_conn<F, R>(handle: TlsHandle, f: F) -> Option<R>
where
    F: FnOnce(&mut TlsConnection) -> R,
{
    lock_table();
    ensure_table();
    let table = unsafe { CONN_TABLE.as_mut().unwrap() };
    // Find the entry with matching handle and get a raw pointer.
    // We release the lock BEFORE calling f() so other threads can
    // alloc/free while this connection does I/O.
    let conn_ptr: *mut TlsConnection = {
        let mut found = core::ptr::null_mut();
        for slot in table.iter_mut() {
            if let Some(entry) = slot {
                if entry.handle == handle {
                    found = &mut entry.conn as *mut TlsConnection;
                    break;
                }
            }
        }
        found
    };
    unlock_table();

    if conn_ptr.is_null() {
        None
    } else {
        // SAFETY: The connection is only removed by close() which the caller
        // must not call concurrently with send/recv on the same handle.
        // Different handles can be used concurrently without issue.
        Some(f(unsafe { &mut *conn_ptr }))
    }
}

/// Remove a connection by handle. Returns the connection for cleanup.
fn remove_conn(handle: TlsHandle) -> Option<TlsConnection> {
    lock_table();
    ensure_table();
    let table = unsafe { CONN_TABLE.as_mut().unwrap() };
    let mut removed = None;
    for slot in table.iter_mut() {
        if let Some(entry) = slot {
            if entry.handle == handle {
                let entry = slot.take().unwrap();
                removed = Some(entry.conn);
                break;
            }
        }
    }
    unlock_table();
    removed
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Establish a TLS connection over an existing TCP socket.
/// The socket must already be connected. `host` is used for SNI.
/// Returns a positive TLS handle on success, negative error code on failure.
///
/// Thread-safe: multiple threads can call connect() concurrently.
/// Each connection gets its own handle and independent state.
pub fn connect(fd: u32, host: &str) -> i32 {
    match TlsConnection::connect(fd, host) {
        Ok(conn) => insert_conn(conn) as i32,
        Err(e) => -(e as i32),
    }
}

/// Send data over the TLS connection identified by `handle`.
/// Returns bytes sent on success, negative on error.
///
/// Thread-safe: different handles can be used concurrently.
/// The same handle must not be used from multiple threads simultaneously.
pub fn send(handle: TlsHandle, data: &[u8]) -> i32 {
    with_conn(handle, |conn| conn.send(data)).unwrap_or(-1)
}

/// Receive data from the TLS connection identified by `handle`.
/// Returns bytes read, 0 on EOF, negative on error.
pub fn recv(handle: TlsHandle, buf: &mut [u8]) -> i32 {
    with_conn(handle, |conn| conn.recv(buf)).unwrap_or(-1)
}

/// Close the TLS connection (sends close_notify).
pub fn close(handle: TlsHandle) {
    if let Some(mut conn) = remove_conn(handle) {
        conn.close();
    }
}

/// Get the last error code for a connection.
pub fn last_error(handle: TlsHandle) -> i32 {
    with_conn(handle, |conn| conn.last_error() as i32).unwrap_or(-1)
}
