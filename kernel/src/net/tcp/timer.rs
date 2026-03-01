//! TCP timer processing — retransmission, delayed ACK flush, TIME_WAIT cleanup.
//!
//! `check_retransmissions()` is called from `net::poll()` and handles:
//! 1. Flushing delayed ACKs that have been pending too long.
//! 2. Retransmitting unACKed data with exponential backoff.
//! 3. Cleaning up TIME_WAIT and Closed connections.

use core::sync::atomic::Ordering;
use super::tcb::*;
use super::send::{send_segment, send_syn_segment};
use super::{TCP_CONNECTIONS, TCP_RETRANSMITS};
use crate::net::types::Ipv4Addr;

/// Check retransmissions, flush delayed ACKs, and perform TIME_WAIT cleanup.
/// Called from net::poll().
pub fn check_retransmissions() {
    let now = crate::arch::hal::timer_current_ticks();

    // ── Pass 1: flush any delayed ACKs that have been pending too long ──
    let mut delayed_acks: [(Ipv4Addr, u16, Ipv4Addr, u16, u32, u32, u16); 8] =
        [(Ipv4Addr([0; 4]), 0, Ipv4Addr([0; 4]), 0, 0, 0, 0); 8];
    let mut delayed_ack_count = 0usize;

    {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return,
        };
        for i in 0..table.len() {
            if delayed_ack_count >= delayed_acks.len() { break; }
            if let Some(tcb) = table[i].as_mut() {
                if tcb.pending_ack && now.wrapping_sub(tcb.last_ack_tick) >= DELAYED_ACK_TICKS {
                    tcb.pending_ack = false;
                    tcb.ack_seg_count = 0;
                    tcb.last_ack_tick = now;
                    let win = tcb.advertised_window();
                    delayed_acks[delayed_ack_count] =
                        (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                         tcb.snd_nxt, tcb.rcv_nxt, win);
                    delayed_ack_count += 1;
                }
            }
        }
    }

    // Send delayed ACKs outside the lock
    for i in 0..delayed_ack_count {
        let (lip, lp, rip, rp, seq, ack_num, win) = delayed_acks[i];
        send_segment(lip, lp, rip, rp, seq, ack_num, ACK, win, &[]);
    }

    // ── Pass 2: retransmissions and cleanup ──
    let mut conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_mut() {
        Some(t) => t,
        None => return,
    };

    for i in 0..table.len() {
        let should_retransmit;
        let should_cleanup;
        let should_syn_retransmit;

        if let Some(tcb) = &table[i] {
            // TIME_WAIT cleanup
            should_cleanup = tcb.state == TcpState::TimeWait
                && now.wrapping_sub(tcb.time_wait_start) >= TIME_WAIT_TICKS;

            // Closed cleanup
            if tcb.state == TcpState::Closed {
                table[i] = None;
                continue;
            }

            // SynReceived cleanup: if max retransmits exceeded, drop
            if tcb.state == TcpState::SynReceived
                && tcb.retransmit_count >= MAX_RETRANSMITS
            {
                table[i] = None;
                continue;
            }

            // Compute retransmit timeout with exponential backoff
            let rto = RETRANSMIT_TICKS << tcb.retransmit_count.min(5);

            // Data retransmit for Established (from send_buf)
            should_retransmit = !tcb.send_buf.is_empty()
                && now.wrapping_sub(tcb.last_send_tick) >= rto
                && tcb.retransmit_count < MAX_RETRANSMITS
                && tcb.state == TcpState::Established;

            // SYN / SYN-ACK retransmit
            should_syn_retransmit = (tcb.state == TcpState::SynReceived || tcb.state == TcpState::SynSent)
                && now.wrapping_sub(tcb.last_send_tick) >= rto
                && tcb.retransmit_count < MAX_RETRANSMITS;
        } else {
            continue;
        }

        if should_cleanup {
            table[i] = None;
            continue;
        }

        if should_syn_retransmit {
            let tcb = table[i].as_mut().unwrap();
            tcb.retransmit_count += 1;
            TCP_RETRANSMITS.fetch_add(1, Ordering::Relaxed);
            tcb.last_send_tick = now;
            let (lip, lp, rip, rp) = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port);

            if tcb.state == TcpState::SynReceived {
                let iss = tcb.snd_iss;
                let rcv_nxt = tcb.rcv_nxt;
                let use_wscale = tcb.rcv_wnd_shift > 0;
                drop(conns);
                if use_wscale {
                    send_syn_segment(lip, lp, rip, rp, iss, rcv_nxt, SYN | ACK);
                } else {
                    send_segment(lip, lp, rip, rp, iss, rcv_nxt, SYN | ACK, 65535, &[]);
                }
            } else {
                // SynSent — retransmit SYN
                let iss = tcb.snd_iss;
                drop(conns);
                send_syn_segment(lip, lp, rip, rp, iss, 0, SYN);
            }
            return; // Can't continue after dropping lock
        }

        if should_retransmit {
            let tcb = table[i].as_mut().unwrap();
            tcb.retransmit_count += 1;
            TCP_RETRANSMITS.fetch_add(1, Ordering::Relaxed);
            tcb.last_send_tick = now;

            // Retransmit from send_buf starting at snd_una
            let len = tcb.send_buf.len().min(MSS);
            if len == 0 {
                continue;
            }

            let mut data = [0u8; 1460];
            let (front, back) = tcb.send_buf.as_slices();
            let front_n = front.len().min(len);
            data[..front_n].copy_from_slice(&front[..front_n]);
            if front_n < len {
                data[front_n..len].copy_from_slice(&back[..len - front_n]);
            }

            let seq = tcb.snd_una;
            let ack_num = tcb.rcv_nxt;
            let win = tcb.advertised_window();
            let (lip, lp, rip, rp) = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port);

            crate::serial_println!("TCP: retransmit #{} socket {} seq={} len={}", tcb.retransmit_count, i, seq, len);

            drop(conns);
            send_segment(lip, lp, rip, rp, seq, ack_num, PSH | ACK, win, &data[..len]);
            return; // Can't continue after dropping lock
        }
    }
}

/// Retransmit FIN for FinWait1 / LastAck states.
/// This is handled separately since FIN doesn't use the send_buf.
pub fn check_fin_retransmissions() {
    let now = crate::arch::hal::timer_current_ticks();

    let mut conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_mut() {
        Some(t) => t,
        None => return,
    };

    for i in 0..table.len() {
        let should_retransmit_fin = if let Some(tcb) = &table[i] {
            let rto = RETRANSMIT_TICKS << tcb.retransmit_count.min(5);
            (tcb.state == TcpState::FinWait1 || tcb.state == TcpState::LastAck)
                && now.wrapping_sub(tcb.last_send_tick) >= rto
                && tcb.retransmit_count < MAX_RETRANSMITS
        } else {
            false
        };

        if should_retransmit_fin {
            let tcb = table[i].as_mut().unwrap();
            tcb.retransmit_count += 1;
            TCP_RETRANSMITS.fetch_add(1, Ordering::Relaxed);
            tcb.last_send_tick = now;

            let seq = tcb.snd_nxt.wrapping_sub(1); // FIN consumes one seq
            let ack_num = tcb.rcv_nxt;
            let win = tcb.advertised_window();
            let (lip, lp, rip, rp) = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port);

            drop(conns);
            send_segment(lip, lp, rip, rp, seq, ack_num, FIN | ACK, win, &[]);
            return;
        }
    }
}
