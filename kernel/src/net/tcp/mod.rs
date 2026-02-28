//! TCP (Transmission Control Protocol) — connection-oriented, reliable transport.
//!
//! Supports both active open (connect) and passive open (listen/accept).
//! Sliding-window send with batched locking. 64-slot connection table with
//! retransmission (exponential backoff), delayed ACKs, out-of-order reassembly,
//! fast retransmit, dynamic window advertisement, and TIME_WAIT cleanup.
//!
//! ## Module structure
//!
//! - `tcb` — Transmission Control Block, types, constants, parsing
//! - `send` — Segment construction and data sending
//! - `recv` — Receive path with OOO reassembly
//! - `input` — Incoming segment dispatch and state machine
//! - `connect` — Connection lifecycle (connect/listen/accept/close)
//! - `timer` — Retransmission, delayed ACK flush, TIME_WAIT cleanup
//! - `util` — Sequence helpers, RST, port allocation, cleanup, netstat

pub(crate) mod tcb;
pub(crate) mod send;
pub(crate) mod recv;
pub(crate) mod input;
pub(crate) mod connect;
pub(crate) mod timer;
pub(crate) mod util;

use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;
use tcb::{Tcb, MAX_CONNECTIONS};
use core::sync::atomic::{AtomicU64, Ordering};

// ── Re-exports (public API — must match old tcp.rs signatures) ──────

pub use tcb::TcpState;
pub use util::{cleanup_for_thread, list_connections, TcpConnInfo};

// ── Global TCP connection table ─────────────────────────────────────

pub(crate) static TCP_CONNECTIONS: Spinlock<Option<Vec<Option<Tcb>>>> = Spinlock::new(None);

// ── Global TCP statistics ───────────────────────────────────────────

pub(crate) static TCP_ACTIVE_OPENS: AtomicU64 = AtomicU64::new(0);
pub(crate) static TCP_PASSIVE_OPENS: AtomicU64 = AtomicU64::new(0);
pub(crate) static TCP_SEGMENTS_SENT: AtomicU64 = AtomicU64::new(0);
pub(crate) static TCP_SEGMENTS_RECV: AtomicU64 = AtomicU64::new(0);
pub(crate) static TCP_RETRANSMITS: AtomicU64 = AtomicU64::new(0);
pub(crate) static TCP_RESETS_SENT: AtomicU64 = AtomicU64::new(0);
pub(crate) static TCP_CONN_ERRORS: AtomicU64 = AtomicU64::new(0);

// ── Initialization ──────────────────────────────────────────────────

/// Initialize the TCP connection table. Must be called before `connect()`.
pub fn init() {
    let mut conns = TCP_CONNECTIONS.lock();
    let mut table = Vec::with_capacity(MAX_CONNECTIONS);
    for _ in 0..MAX_CONNECTIONS {
        table.push(None);
    }
    *conns = Some(table);
    crate::serial_println!("[OK] TCP initialized ({} slots, OOO buffering, fast retransmit)", MAX_CONNECTIONS);
}

// ── Statistics ───────────────────────────────────────────────────────

/// Snapshot of TCP protocol statistics.
pub struct TcpStats {
    pub active_opens: u64,
    pub passive_opens: u64,
    pub segments_sent: u64,
    pub segments_recv: u64,
    pub retransmits: u64,
    pub resets_sent: u64,
    pub conn_errors: u64,
    pub curr_established: u32,
}

/// Get a snapshot of TCP protocol statistics.
pub fn get_stats() -> TcpStats {
    let conns = TCP_CONNECTIONS.lock();
    let established = match conns.as_ref() {
        Some(table) => table.iter()
            .filter(|s| s.as_ref().map(|t| t.state == TcpState::Established).unwrap_or(false))
            .count() as u32,
        None => 0,
    };
    TcpStats {
        active_opens: TCP_ACTIVE_OPENS.load(Ordering::Relaxed),
        passive_opens: TCP_PASSIVE_OPENS.load(Ordering::Relaxed),
        segments_sent: TCP_SEGMENTS_SENT.load(Ordering::Relaxed),
        segments_recv: TCP_SEGMENTS_RECV.load(Ordering::Relaxed),
        retransmits: TCP_RETRANSMITS.load(Ordering::Relaxed),
        resets_sent: TCP_RESETS_SENT.load(Ordering::Relaxed),
        conn_errors: TCP_CONN_ERRORS.load(Ordering::Relaxed),
        curr_established: established,
    }
}

// ── Public API delegations ──────────────────────────────────────────
// These maintain the same function signatures as the old tcp.rs so that
// all callers (syscall handlers, net::poll, etc.) work unchanged.

/// Active open: connect to a remote host.
pub fn connect(remote_ip: crate::net::types::Ipv4Addr, remote_port: u16, timeout_ticks: u32) -> u32 {
    connect::connect(remote_ip, remote_port, timeout_ticks)
}

/// Passive open: listen on a local port.
pub fn listen(port: u16, backlog: u16) -> u32 {
    connect::listen(port, backlog)
}

/// Accept a connection from a listening socket.
pub fn accept(listener_id: u32, timeout_ticks: u32) -> (u32, crate::net::types::Ipv4Addr, u16) {
    connect::accept(listener_id, timeout_ticks)
}

/// Send data on an established connection.
pub fn send(socket_id: u32, data: &[u8], timeout_ticks: u32) -> u32 {
    send::send(socket_id, data, timeout_ticks)
}

/// Receive data from an established connection.
pub fn recv(socket_id: u32, buf: &mut [u8], timeout_ticks: u32) -> u32 {
    recv::recv(socket_id, buf, timeout_ticks)
}

/// Check bytes available to read.
pub fn recv_available(socket_id: u32) -> u32 {
    recv::recv_available(socket_id)
}

/// Close a TCP connection.
pub fn close(socket_id: u32) -> u32 {
    connect::close(socket_id)
}

/// Close a listening socket.
pub fn close_listener(socket_id: u32) -> u32 {
    connect::close_listener(socket_id)
}

/// Get connection state.
pub fn status(socket_id: u32) -> u32 {
    connect::status(socket_id)
}

/// Half-close (SHUT_WR).
pub fn shutdown_write(socket_id: u32) -> u32 {
    connect::shutdown_write(socket_id)
}

/// Handle an incoming TCP segment.
pub fn handle_tcp(pkt: &crate::net::ipv4::Ipv4Packet<'_>) {
    input::handle_tcp(pkt)
}

/// Check retransmissions, flush delayed ACKs, TIME_WAIT cleanup.
pub fn check_retransmissions() {
    timer::check_retransmissions();
    timer::check_fin_retransmissions();
}
