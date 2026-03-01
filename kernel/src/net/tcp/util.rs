//! TCP utility functions.
//!
//! Sequence number comparison helpers (wrapping-safe per RFC 793),
//! RST generation, ephemeral port allocation, connection cleanup,
//! and connection listing for `netstat`.

use alloc::vec::Vec;
use crate::net::types::Ipv4Addr;
use crate::sync::spinlock::Spinlock;
use super::tcb::*;
use super::send::send_segment;
use super::TCP_CONNECTIONS;

// ── Sequence number comparison (wrapping-safe) ──────────────────────

/// Returns true if `a` is strictly after `b` in sequence space.
pub(crate) fn is_seq_gt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

/// Returns true if `a` is at or after `b` in sequence space.
pub(crate) fn is_seq_gte(a: u32, b: u32) -> bool {
    a == b || is_seq_gt(a, b)
}

/// Returns true if `a` is at or before `b` in sequence space.
pub(crate) fn is_seq_lte(a: u32, b: u32) -> bool {
    a == b || !is_seq_gt(a, b)
}

// ── Ephemeral port allocation ───────────────────────────────────────

static NEXT_PORT: Spinlock<u16> = Spinlock::new(49152);

/// Allocate the next ephemeral port (49152–65535 range, wrapping).
pub(crate) fn alloc_ephemeral_port() -> u16 {
    let mut port = NEXT_PORT.lock();
    let p = *port;
    *port = if p >= 65535 { 49152 } else { p + 1 };
    p
}

// ── RST generation ──────────────────────────────────────────────────

/// Send a RST for an unexpected segment (no matching connection).
pub(crate) fn send_rst(seg: &TcpSegment) {
    let cfg = crate::net::config();
    if seg.flags & RST != 0 {
        return; // Never RST a RST
    }
    if seg.flags & ACK != 0 {
        send_segment(cfg.ip, seg.dst_port, seg.src_ip, seg.src_port,
                     seg.ack, 0, RST, 0, &[]);
    } else {
        let ack_val = seg.seq.wrapping_add(seg.payload.len() as u32)
            .wrapping_add(if seg.flags & SYN != 0 { 1 } else { 0 })
            .wrapping_add(if seg.flags & FIN != 0 { 1 } else { 0 });
        send_segment(cfg.ip, seg.dst_port, seg.src_ip, seg.src_port,
                     0, ack_val, RST | ACK, 0, &[]);
    }
}

// ── Thread cleanup ──────────────────────────────────────────────────

/// Clean up all TCP connections owned by a specific thread.
/// Called from sys_exit() when a process terminates.
/// Sends RST for established connections and frees listener slots + pending connections.
pub fn cleanup_for_thread(tid: u32) {
    let mut rst_list: [(Ipv4Addr, u16, Ipv4Addr, u16, u32, u32); 16] =
        [(Ipv4Addr([0; 4]), 0, Ipv4Addr([0; 4]), 0, 0, 0); 16];
    let mut rst_count = 0usize;

    {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return,
        };

        // First pass: close listeners and their pending (unaccepted) connections
        for i in 0..table.len() {
            let is_owned_listener = table[i].as_ref().map(|tcb| {
                tcb.owner_tid == tid && tcb.state == TcpState::Listen
            }).unwrap_or(false);

            if is_owned_listener {
                let lid = i as u8;
                for j in 0..table.len() {
                    let is_pending = table[j].as_ref().map(|tcb| {
                        tcb.parent_listener == Some(lid) && !tcb.accepted
                    }).unwrap_or(false);
                    if is_pending {
                        if let Some(tcb) = &table[j] {
                            if tcb.state != TcpState::Closed && rst_count < rst_list.len() {
                                rst_list[rst_count] = (tcb.local_ip, tcb.local_port,
                                    tcb.remote_ip, tcb.remote_port, tcb.snd_nxt, tcb.rcv_nxt);
                                rst_count += 1;
                            }
                        }
                        table[j] = None;
                    }
                }
                table[i] = None;
                crate::serial_println!("TCP: cleanup listener socket {} for TID {}", i, tid);
            }
        }

        // Second pass: close active connections owned by this thread
        for i in 0..table.len() {
            let is_owned = table[i].as_ref().map(|tcb| {
                tcb.owner_tid == tid
            }).unwrap_or(false);

            if is_owned {
                if let Some(tcb) = &table[i] {
                    match tcb.state {
                        TcpState::Established | TcpState::SynSent | TcpState::SynReceived
                        | TcpState::FinWait1 | TcpState::FinWait2 | TcpState::CloseWait => {
                            if rst_count < rst_list.len() {
                                rst_list[rst_count] = (tcb.local_ip, tcb.local_port,
                                    tcb.remote_ip, tcb.remote_port, tcb.snd_nxt, tcb.rcv_nxt);
                                rst_count += 1;
                            }
                        }
                        _ => {}
                    }
                }
                table[i] = None;
                crate::serial_println!("TCP: cleanup socket {} for TID {}", i, tid);
            }
        }
    }

    // Send RSTs outside the lock
    for k in 0..rst_count {
        let (lip, lp, rip, rp, seq, ack) = rst_list[k];
        if rp != 0 {
            send_segment(lip, lp, rip, rp, seq, ack, RST | ACK, 0, &[]);
        }
    }
}

// ── Connection listing (netstat) ────────────────────────────────────

/// Connection info for netstat display.
pub struct TcpConnInfo {
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,
    pub state: TcpState,
    pub owner_tid: u32,
    pub recv_buf_len: usize,
}

/// List all active TCP connections and listeners.
pub fn list_connections() -> Vec<TcpConnInfo> {
    let mut result = Vec::new();
    let conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_ref() {
        Some(t) => t,
        None => return result,
    };

    for slot in table.iter() {
        if let Some(tcb) = slot {
            result.push(TcpConnInfo {
                local_ip: tcb.local_ip,
                local_port: tcb.local_port,
                remote_ip: tcb.remote_ip,
                remote_port: tcb.remote_port,
                state: tcb.state,
                owner_tid: tcb.owner_tid,
                recv_buf_len: tcb.recv_buf.len(),
            });
        }
    }

    result
}
