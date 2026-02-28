//! TCP input processing — incoming segment dispatch and state machine.
//!
//! Handles all incoming TCP segments: SYN (passive open), SYN-ACK
//! (active open), data/ACK in ESTABLISHED, FIN handling, and RST.
//! Implements fast retransmit on 3 duplicate ACKs.

use core::sync::atomic::Ordering;
use super::tcb::*;
use super::send::{send_segment, send_syn_segment};
use super::recv::accept_data_deferred;
use super::util::{is_seq_gt, is_seq_gte, is_seq_lte, send_rst};
use super::{TCP_CONNECTIONS, TCP_SEGMENTS_RECV, TCP_RETRANSMITS};

/// Handle an incoming TCP segment. Called from ipv4::handle_ipv4().
pub fn handle_tcp(pkt: &crate::net::ipv4::Ipv4Packet<'_>) {
    let seg = match parse_tcp(pkt) {
        Some(s) => s,
        None => return,
    };
    TCP_SEGMENTS_RECV.fetch_add(1, Ordering::Relaxed);

    // Process segment under lock, collect deferred sends and wake TIDs.
    let mut wake_tid: u32 = 0;
    let mut wake_listener_tid: u32 = 0;
    let deferred: Option<DeferredSend> = {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return,
        };

        // Find matching connection (exact match on 4-tuple)
        let slot_idx = table.iter().position(|slot| {
            if let Some(tcb) = slot {
                tcb.local_port == seg.dst_port
                    && tcb.remote_port == seg.src_port
                    && tcb.remote_ip == seg.src_ip
                    && tcb.state != TcpState::Listen
            } else {
                false
            }
        });

        let idx = match slot_idx {
            Some(i) => i,
            None => {
                // No exact match — check for a listening socket on this port
                if seg.flags & SYN != 0 && seg.flags & ACK == 0 {
                    let listener_idx = table.iter().position(|slot| {
                        if let Some(tcb) = slot {
                            tcb.state == TcpState::Listen && tcb.local_port == seg.dst_port
                        } else {
                            false
                        }
                    });

                    if let Some(lid) = listener_idx {
                        // Count pending connections for this listener
                        let pending_count = table.iter().filter(|s| {
                            s.as_ref().map(|t| {
                                t.parent_listener == Some(lid as u8) && !t.accepted
                            }).unwrap_or(false)
                        }).count();

                        if pending_count >= MAX_BACKLOG {
                            return; // Backlog full — silently drop SYN
                        }

                        let cfg = crate::net::config();
                        let mut new_slot = None;
                        for (i, slot) in table.iter_mut().enumerate() {
                            if slot.is_none() {
                                let mut tcb = Tcb::new(cfg.ip, seg.dst_port, seg.src_ip, seg.src_port);
                                tcb.state = TcpState::SynReceived;
                                tcb.rcv_irs = seg.seq;
                                tcb.rcv_nxt = seg.seq.wrapping_add(1);
                                tcb.snd_nxt = tcb.snd_iss.wrapping_add(1);
                                tcb.parent_listener = Some(lid as u8);
                                tcb.last_send_tick = crate::arch::x86::pit::get_ticks();
                                // Store peer's window scale if present (RFC 7323)
                                if let Some(shift) = seg.wscale {
                                    tcb.snd_wnd_shift = shift;
                                    tcb.rcv_wnd_shift = OUR_WINDOW_SHIFT;
                                }
                                *slot = Some(tcb);
                                new_slot = Some(i);
                                break;
                            }
                        }

                        if let Some(ns) = new_slot {
                            let tcb = table[ns].as_ref().unwrap();
                            let (lip, lp, rip, rp) = (tcb.local_ip, tcb.local_port,
                                tcb.remote_ip, tcb.remote_port);
                            let iss = tcb.snd_iss;
                            let rcv_nxt = tcb.rcv_nxt;
                            let use_wscale = tcb.rcv_wnd_shift > 0;
                            crate::serial_println!("TCP: SYN on listener {} -> new conn slot {} from {}:{} (wscale={})",
                                lid, ns, seg.src_ip, seg.src_port,
                                if use_wscale { tcb.snd_wnd_shift } else { 0 });
                            drop(conns);
                            if use_wscale {
                                send_syn_segment(lip, lp, rip, rp, iss, rcv_nxt, SYN | ACK);
                            } else {
                                send_segment(lip, lp, rip, rp, iss, rcv_nxt, SYN | ACK, 65535, &[]);
                            }
                            return;
                        }
                        return; // No free slots
                    }
                }
                // No matching connection and no listener — send RST
                drop(conns);
                send_rst(&seg);
                return;
            }
        };

        // RST handling — always process
        if seg.flags & RST != 0 {
            crate::serial_println!("TCP: RST received on socket {}", idx);
            let tcb = table[idx].as_mut().unwrap();
            tcb.reset_received = true;
            tcb.state = TcpState::Closed;
            wake_tid = tcb.waiting_tid;
            tcb.waiting_tid = 0;
            drop(conns);
            if wake_tid != 0 {
                crate::task::scheduler::try_wake_thread(wake_tid);
            }
            return;
        }

        // Read state before borrowing tcb mutably (needed for SynReceived
        // which also needs to access the listener in the table).
        let state = table[idx].as_ref().unwrap().state;
        let now = crate::arch::x86::pit::get_ticks();

        // SynReceived needs access to both tcb AND the parent listener in table,
        // so handle it separately to avoid double-mutable-borrow.
        let match_result = if state == TcpState::SynReceived {
            handle_syn_received(idx, &seg, table, &mut wake_listener_tid)
        } else {
            let tcb = table[idx].as_mut().unwrap();
            match tcb.state {
                TcpState::SynSent => {
                    handle_syn_sent(tcb, &seg)
                }

                TcpState::Established => {
                    handle_established(tcb, &seg)
                }

                TcpState::FinWait1 => {
                    if seg.flags & ACK != 0 {
                        if is_seq_gte(seg.ack, tcb.snd_nxt) {
                            tcb.snd_una = seg.ack;
                            tcb.state = TcpState::FinWait2;
                        }
                    }
                    let data_ack = accept_data_deferred(tcb, &seg);
                    if seg.flags & FIN != 0 {
                        tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(1);
                        tcb.fin_received = true;
                        tcb.state = TcpState::TimeWait;
                        tcb.time_wait_start = now;
                        Some(DeferredSend {
                            local_ip: tcb.local_ip, local_port: tcb.local_port,
                            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
                            seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
                            window: tcb.advertised_window(),
                        })
                    } else {
                        data_ack
                    }
                }

                TcpState::FinWait2 => {
                    let data_ack = accept_data_deferred(tcb, &seg);
                    if seg.flags & FIN != 0 {
                        tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(1);
                        tcb.fin_received = true;
                        tcb.state = TcpState::TimeWait;
                        tcb.time_wait_start = now;
                        Some(DeferredSend {
                            local_ip: tcb.local_ip, local_port: tcb.local_port,
                            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
                            seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
                            window: tcb.advertised_window(),
                        })
                    } else {
                        data_ack
                    }
                }

                TcpState::CloseWait => {
                    if seg.flags & ACK != 0 {
                        tcb.snd_una = seg.ack;
                    }
                    None
                }

                TcpState::LastAck => {
                    if seg.flags & ACK != 0 {
                        if is_seq_gte(seg.ack, tcb.snd_nxt) {
                            tcb.state = TcpState::Closed;
                        }
                    }
                    None
                }

                TcpState::TimeWait => {
                    if seg.flags & FIN != 0 {
                        tcb.time_wait_start = now;
                        Some(DeferredSend {
                            local_ip: tcb.local_ip, local_port: tcb.local_port,
                            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
                            seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
                            window: tcb.advertised_window(),
                        })
                    } else {
                        None
                    }
                }

                TcpState::SynReceived | TcpState::Listen | TcpState::Closed => None,
            }
        };

        // Collect waiting_tid — wake after lock drop
        let tcb = table[idx].as_mut().unwrap();
        if tcb.waiting_tid != 0 {
            wake_tid = tcb.waiting_tid;
            tcb.waiting_tid = 0;
        }

        match_result
    }; // lock dropped here

    // Wake blocked threads outside lock
    if wake_tid != 0 {
        crate::task::scheduler::try_wake_thread(wake_tid);
    }
    if wake_listener_tid != 0 {
        crate::task::scheduler::try_wake_thread(wake_listener_tid);
    }

    // Send deferred segment outside lock
    if let Some(ds) = deferred {
        send_segment(ds.local_ip, ds.local_port, ds.remote_ip, ds.remote_port,
                    ds.seq, ds.ack_num, ds.flags, ds.window, &[]);
    }
}

// ── State-specific handlers ─────────────────────────────────────────

/// Handle segment in SYN_SENT state (active open, waiting for SYN-ACK).
fn handle_syn_sent(tcb: &mut Tcb, seg: &TcpSegment) -> Option<DeferredSend> {
    if seg.flags & SYN != 0 && seg.flags & ACK != 0 {
        if seg.ack == tcb.snd_nxt {
            tcb.rcv_irs = seg.seq;
            tcb.rcv_nxt = seg.seq.wrapping_add(1);
            tcb.snd_una = seg.ack;
            // SYN-ACK window is NOT scaled (RFC 7323)
            tcb.snd_wnd = seg.window as u32;
            if let Some(shift) = seg.wscale {
                tcb.snd_wnd_shift = shift;
                tcb.rcv_wnd_shift = OUR_WINDOW_SHIFT;
            }
            tcb.state = TcpState::Established;
            tcb.send_buf.clear();
            tcb.retransmit_count = 0;
            tcb.dup_ack_count = 0;
            Some(DeferredSend {
                local_ip: tcb.local_ip, local_port: tcb.local_port,
                remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
                seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
                window: tcb.advertised_window(),
            })
        } else {
            crate::serial_println!("TCP: SYN-ACK bad ACK {} expected {}", seg.ack, tcb.snd_nxt);
            None
        }
    } else if seg.flags & ACK != 0 {
        Some(DeferredSend {
            local_ip: tcb.local_ip, local_port: tcb.local_port,
            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
            seq: seg.ack, ack_num: 0, flags: RST,
            window: 0,
        })
    } else {
        None
    }
}

/// Handle data/ACK/FIN in ESTABLISHED state with fast retransmit.
fn handle_established(tcb: &mut Tcb, seg: &TcpSegment) -> Option<DeferredSend> {
    // ── Process ACK ──
    if seg.flags & ACK != 0 {
        if is_seq_gt(seg.ack, tcb.snd_una) && is_seq_lte(seg.ack, tcb.snd_nxt) {
            // New ACK — advances snd_una
            let acked = seg.ack.wrapping_sub(tcb.snd_una) as usize;
            tcb.snd_una = seg.ack;
            tcb.snd_wnd = (seg.window as u32) << tcb.snd_wnd_shift;

            // Drain acknowledged bytes from send buffer
            let drain = acked.min(tcb.send_buf.len());
            if drain > 0 {
                tcb.send_buf.drain(..drain);
            }

            // Reset fast retransmit state
            tcb.dup_ack_count = 0;

            if tcb.snd_una == tcb.snd_nxt {
                // All data acknowledged
                tcb.send_buf.clear();
                tcb.retransmit_count = 0;
            }
        } else if seg.ack == tcb.snd_una && !seg.payload.is_empty() {
            // Data segment with same ACK — not a dup ACK, just piggybacked
            tcb.snd_wnd = (seg.window as u32) << tcb.snd_wnd_shift;
        } else if seg.ack == tcb.snd_una && seg.payload.is_empty()
            && seg.flags & (SYN | FIN | RST) == 0
            && tcb.snd_una != tcb.snd_nxt
        {
            // ── Duplicate ACK (RFC 5681) ──
            tcb.dup_ack_count += 1;

            if tcb.dup_ack_count == 3 {
                // Fast retransmit: resend from snd_una
                fast_retransmit(tcb);
            }
        }
    }

    // ── Accept data ──
    let data_ack = accept_data_deferred(tcb, seg);

    // ── Handle FIN ──
    if seg.flags & FIN != 0 {
        tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(1);
        tcb.fin_received = true;
        tcb.state = TcpState::CloseWait;
        Some(DeferredSend {
            local_ip: tcb.local_ip, local_port: tcb.local_port,
            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
            seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
            window: tcb.advertised_window(),
        })
    } else {
        data_ack
    }
}

/// Fast retransmit: resend one MSS from the send buffer at snd_una.
///
/// Called when 3 duplicate ACKs are received, indicating a lost segment.
/// Sends outside the critical section would require collecting deferred data,
/// but since we're already inside handle_tcp's lock scope, we record the
/// retransmit and let the timer path handle the actual send.
fn fast_retransmit(tcb: &mut Tcb) {
    if tcb.send_buf.is_empty() {
        return;
    }

    TCP_RETRANSMITS.fetch_add(1, Ordering::Relaxed);
    tcb.retransmit_count = 0; // Reset — fast retransmit is not a timeout
    tcb.last_send_tick = crate::arch::x86::pit::get_ticks();

    // Extract first MSS of send_buf for retransmission
    let len = tcb.send_buf.len().min(MSS);
    let mut data = [0u8; 1460];
    let (front, back) = tcb.send_buf.as_slices();
    let front_n = front.len().min(len);
    data[..front_n].copy_from_slice(&front[..front_n]);
    if front_n < len {
        data[front_n..len].copy_from_slice(&back[..len - front_n]);
    }

    let win = tcb.advertised_window();

    // We cannot call send_segment here (would need to drop the lock first).
    // Instead, we use the deferred retransmit path: set a flag so the
    // next check_retransmissions() call sends immediately.
    // Actually — handle_tcp's caller sends the deferred segment, but that's
    // only for ACKs. For fast retransmit data, we need to send now.
    // Since the lock is held by our caller, and send_segment doesn't need
    // TCP_CONNECTIONS, we can call it directly here.
    send_segment(tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                 tcb.snd_una, tcb.rcv_nxt, PSH | ACK, win, &data[..len]);

    crate::serial_println!("TCP: fast retransmit seq={} len={}", tcb.snd_una, len);
}

/// Handle SynReceived state (server-side 3-way handshake completion).
///
/// Takes `idx` and indexes into `table` directly to avoid the double-mutable-borrow
/// problem (needs to update both the connection TCB and the parent listener).
fn handle_syn_received(
    idx: usize,
    seg: &TcpSegment,
    table: &mut [Option<Tcb>],
    wake_listener_tid: &mut u32,
) -> Option<DeferredSend> {
    let tcb = table[idx].as_mut().unwrap();

    if seg.flags & ACK != 0 {
        if seg.ack == tcb.snd_nxt {
            tcb.snd_una = seg.ack;
            // ACK completing handshake: window IS scaled (RFC 7323)
            tcb.snd_wnd = (seg.window as u32) << tcb.snd_wnd_shift;
            tcb.state = TcpState::Established;
            tcb.send_buf.clear();
            tcb.retransmit_count = 0;
            tcb.dup_ack_count = 0;
            let parent = tcb.parent_listener;
            crate::serial_println!("TCP: SynReceived -> Established on socket {}", idx);
            // Wake the accept() thread on the parent listener
            if let Some(lid) = parent {
                if let Some(listener) = table[lid as usize].as_mut() {
                    *wake_listener_tid = listener.waiting_tid;
                    listener.waiting_tid = 0;
                }
            }
            None
        } else {
            crate::serial_println!("TCP: SynReceived bad ACK {} expected {}", seg.ack, tcb.snd_nxt);
            None
        }
    } else if seg.flags & SYN != 0 {
        // Duplicate SYN — retransmit SYN-ACK
        Some(DeferredSend {
            local_ip: tcb.local_ip, local_port: tcb.local_port,
            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
            seq: tcb.snd_iss, ack_num: tcb.rcv_nxt,
            flags: SYN | ACK,
            window: tcb.advertised_window(),
        })
    } else {
        None
    }
}
