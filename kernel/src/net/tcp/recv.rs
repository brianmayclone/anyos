//! TCP receive path with out-of-order reassembly.
//!
//! Implements `recv()`, `recv_available()`, and `accept_data_deferred()`
//! with proper OOO segment buffering and delayed ACK support.

use super::send::send_segment;
use super::tcb::*;
use super::util::{is_seq_gt, is_seq_gte, is_seq_lte};
use super::TCP_CONNECTIONS;

// ── Out-of-order reassembly ─────────────────────────────────────────

/// Accept in-order data and buffer out-of-order segments for reassembly.
///
/// Returns a `DeferredSend` if an ACK should be sent (immediate or delayed).
/// Out-of-order segments are buffered in `tcb.ooo_buf` and reassembled
/// when the gap is filled.
pub(crate) fn accept_data_deferred(tcb: &mut Tcb, seg: &TcpSegment) -> Option<DeferredSend> {
    if seg.payload.is_empty() {
        return None;
    }

    let win = tcb.advertised_window();

    let mut seq = seg.seq;
    let mut payload = seg.payload.as_slice();

    // Retransmissions commonly overlap already accepted data. Trim the
    // duplicate prefix and keep any new tail instead of dropping the segment.
    if is_seq_gt(tcb.rcv_nxt, seq) {
        let overlap = tcb.rcv_nxt.wrapping_sub(seq) as usize;
        if overlap >= payload.len() {
            return Some(DeferredSend {
                local_ip: tcb.local_ip,
                local_port: tcb.local_port,
                remote_ip: tcb.remote_ip,
                remote_port: tcb.remote_port,
                seq: tcb.snd_nxt,
                ack_num: tcb.rcv_nxt,
                flags: ACK,
                window: win,
            });
        }
        payload = &payload[overlap..];
        seq = tcb.rcv_nxt;
    }

    if seq == tcb.rcv_nxt {
        // ── In-order segment ──
        let space = RECV_BUF_SIZE - tcb.recv_buf.len();
        if space == 0 {
            return Some(DeferredSend {
                local_ip: tcb.local_ip,
                local_port: tcb.local_port,
                remote_ip: tcb.remote_ip,
                remote_port: tcb.remote_port,
                seq: tcb.snd_nxt,
                ack_num: tcb.rcv_nxt,
                flags: ACK,
                window: tcb.advertised_window(),
            });
        }

        let take = payload.len().min(space);
        tcb.recv_buf.extend(&payload[..take]);
        tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(take as u32);
        tcb.ack_seg_count += 1;
        tcb.pending_ack = true;

        let had_ooo = !tcb.ooo_buf.is_empty();

        // Drain any now-contiguous OOO segments
        drain_ooo(tcb);

        // Decide whether to ACK now or delay
        let buf_pressure = tcb.recv_buf.len() > (RECV_BUF_SIZE * 3 / 4);
        let batch_full = tcb.ack_seg_count >= DELAYED_ACK_SEGMENTS;
        // Loopback: always ACK immediately (no timer to flush delayed ACKs)
        let is_loopback = tcb.remote_ip.0[0] == 127;

        if batch_full || buf_pressure || had_ooo || is_loopback {
            tcb.pending_ack = false;
            tcb.ack_seg_count = 0;
            tcb.last_ack_tick = crate::arch::hal::timer_current_ticks();
            Some(DeferredSend {
                local_ip: tcb.local_ip,
                local_port: tcb.local_port,
                remote_ip: tcb.remote_ip,
                remote_port: tcb.remote_port,
                seq: tcb.snd_nxt,
                ack_num: tcb.rcv_nxt,
                flags: ACK,
                window: tcb.advertised_window(),
            })
        } else {
            None
        }
    } else if is_seq_gt(seq, tcb.rcv_nxt) {
        // ── Out-of-order segment — buffer it ──
        // INSTRUMENTATION: log the FIRST gap of a loss event (ooo_buf empty ->
        // non-empty). A stall then shows as a [tcp-gap] with no matching
        // [tcp-recovered] — i.e. the missing segment at rcv_nxt was never
        // retransmitted/accepted. ooo_len lets us see if the buffer overflows.
        if tcb.ooo_buf.is_empty() {
            crate::serial_verbose_println!(
                "[tcp-gap] port={} rcv_nxt={} seg_seq={} ahead={} recv_buf={}",
                tcb.remote_port,
                tcb.rcv_nxt,
                seq,
                seq.wrapping_sub(tcb.rcv_nxt),
                tcb.recv_buf.len()
            );
        }
        insert_ooo(tcb, seq, payload);

        // Send duplicate ACK immediately (fast retransmit signal to sender)
        Some(DeferredSend {
            local_ip: tcb.local_ip,
            local_port: tcb.local_port,
            remote_ip: tcb.remote_ip,
            remote_port: tcb.remote_port,
            seq: tcb.snd_nxt,
            ack_num: tcb.rcv_nxt,
            flags: ACK,
            window: win,
        })
    } else {
        // ── Duplicate / overlapping — ACK immediately ──
        Some(DeferredSend {
            local_ip: tcb.local_ip,
            local_port: tcb.local_port,
            remote_ip: tcb.remote_ip,
            remote_port: tcb.remote_port,
            seq: tcb.snd_nxt,
            ack_num: tcb.rcv_nxt,
            flags: ACK,
            window: win,
        })
    }
}

/// Insert a segment into the OOO buffer, merging overlaps.
/// Keeps the buffer sorted by sequence number for efficient drain_ooo.
fn insert_ooo(tcb: &mut Tcb, seq: u32, data: &[u8]) {
    if data.is_empty() {
        return;
    }

    let end = seq.wrapping_add(data.len() as u32);

    // Check for complete containment within an existing segment (skip duplicates)
    for existing in tcb.ooo_buf.iter() {
        let ex_end = existing.seq.wrapping_add(existing.data.len() as u32);
        // New segment entirely within existing → skip
        if is_seq_gte(seq, existing.seq) && is_seq_lte(end, ex_end) {
            return;
        }
    }

    // Drop oldest if buffer full
    if tcb.ooo_buf.len() >= MAX_OOO_SEGMENTS {
        return;
    }

    // Find insertion point to keep sorted by seq
    let pos = tcb
        .ooo_buf
        .iter()
        .position(|s| is_seq_gt(s.seq, seq))
        .unwrap_or(tcb.ooo_buf.len());

    tcb.ooo_buf.insert(
        pos,
        OooSegment {
            seq,
            data: data.to_vec(),
        },
    );

    // Merge adjacent/overlapping segments (single pass since buffer is sorted)
    let mut i = 0;
    while i + 1 < tcb.ooo_buf.len() {
        let a_end = tcb.ooo_buf[i]
            .seq
            .wrapping_add(tcb.ooo_buf[i].data.len() as u32);
        let b_seq = tcb.ooo_buf[i + 1].seq;

        // If segment i reaches into or past segment i+1, merge them
        if !is_seq_gt(b_seq, a_end) {
            let b_end = b_seq.wrapping_add(tcb.ooo_buf[i + 1].data.len() as u32);
            if is_seq_gt(b_end, a_end) {
                // Extend segment i with the non-overlapping tail of segment i+1
                let overlap = a_end.wrapping_sub(b_seq) as usize;
                if overlap < tcb.ooo_buf[i + 1].data.len() {
                    let tail = tcb.ooo_buf[i + 1].data[overlap..].to_vec();
                    tcb.ooo_buf[i].data.extend_from_slice(&tail);
                }
            }
            tcb.ooo_buf.remove(i + 1);
        } else {
            i += 1;
        }
    }
}

/// Drain contiguous segments from the OOO buffer into recv_buf.
fn drain_ooo(tcb: &mut Tcb) {
    // INSTRUMENTATION: pair with [tcp-gap]. If a gap opened but this never
    // logs [tcp-recovered] for that port, the lost segment at rcv_nxt was
    // never retransmitted/accepted — the stall point.
    let was_nonempty = !tcb.ooo_buf.is_empty();
    let entry_rcv_nxt = tcb.rcv_nxt;
    loop {
        if tcb.ooo_buf.is_empty() {
            if was_nonempty && tcb.rcv_nxt != entry_rcv_nxt {
                crate::serial_verbose_println!(
                    "[tcp-recovered] port={} rcv_nxt={}",
                    tcb.remote_port,
                    tcb.rcv_nxt
                );
            }
            break;
        }

        let next_seq = tcb.rcv_nxt;
        // Find a segment that starts at or before rcv_nxt
        let idx = tcb.ooo_buf.iter().position(|s| {
            // Segment starts at or before rcv_nxt and extends past it
            let s_end = s.seq.wrapping_add(s.data.len() as u32);
            !is_seq_gt(s.seq, next_seq) && is_seq_gt(s_end, next_seq)
        });

        match idx {
            Some(i) => {
                let mut seg = tcb.ooo_buf.remove(i);
                // Calculate how much of this segment is new data
                let overlap = next_seq.wrapping_sub(seg.seq) as usize;
                if overlap < seg.data.len() {
                    let new_data = &seg.data[overlap..];
                    let space = RECV_BUF_SIZE - tcb.recv_buf.len();
                    if space == 0 {
                        tcb.ooo_buf.insert(i, seg);
                        break;
                    }
                    let take = new_data.len().min(space);
                    tcb.recv_buf.extend(&new_data[..take]);
                    tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(take as u32);
                    let consumed = overlap + take;
                    if consumed < seg.data.len() {
                        seg.seq = seg.seq.wrapping_add(consumed as u32);
                        seg.data = seg.data[consumed..].to_vec();
                        tcb.ooo_buf.insert(i, seg);
                        break;
                    }
                }
            }
            None => break,
        }
    }
}

// ── Receive API ─────────────────────────────────────────────────────

/// Receive data from an established connection.
/// Returns bytes received, 0 if connection closed (FIN), u32::MAX on error.
///
/// Blocks the calling thread until data arrives, the connection closes,
/// or the timeout expires. Zero CPU usage while waiting.
pub fn recv(socket_id: u32, buf: &mut [u8], timeout_ticks: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS || buf.is_empty() {
        return u32::MAX;
    }

    let tid = crate::task::scheduler::current_tid();
    let start = crate::arch::hal::timer_current_ticks();

    // Poll once eagerly to process any pending packets before blocking.
    crate::net::poll();

    loop {
        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return u32::MAX,
            };
            let tcb = match table[id].as_mut() {
                Some(t) => t,
                None => return u32::MAX,
            };

            if tcb.reset_received {
                tcb.waiting_tid = 0;
                return u32::MAX;
            }

            // Copy directly from VecDeque slices — no intermediate Vec allocation.
            if !tcb.recv_buf.is_empty() {
                let old_window = tcb.advertised_window();
                let n = tcb.recv_buf.len().min(buf.len());
                let (front, back) = tcb.recv_buf.as_slices();
                let front_n = front.len().min(n);
                buf[..front_n].copy_from_slice(&front[..front_n]);
                if front_n < n {
                    buf[front_n..n].copy_from_slice(&back[..n - front_n]);
                }
                tcb.recv_buf.drain(..n);
                tcb.waiting_tid = 0;

                let new_window = tcb.advertised_window();

                // Send window update ACK if window opened significantly.
                // RFC 1122 §4.2.3.3: send when window opens by >= 1 MSS
                // (scaled). This unblocks the sender after a zero-window stall.
                let mss_scaled = (MSS >> tcb.rcv_wnd_shift as usize) as u16;
                let window_update = if new_window >= mss_scaled
                    && new_window.saturating_sub(old_window) >= mss_scaled
                {
                    Some(DeferredSend {
                        local_ip: tcb.local_ip,
                        local_port: tcb.local_port,
                        remote_ip: tcb.remote_ip,
                        remote_port: tcb.remote_port,
                        seq: tcb.snd_nxt,
                        ack_num: tcb.rcv_nxt,
                        flags: ACK,
                        window: new_window,
                    })
                } else {
                    None
                };

                // Drop lock before sending
                drop(conns);

                // Send window update outside the lock
                if let Some(ref wu) = window_update {
                    send_segment(
                        wu.local_ip,
                        wu.local_port,
                        wu.remote_ip,
                        wu.remote_port,
                        wu.seq,
                        wu.ack_num,
                        wu.flags,
                        wu.window,
                        &[],
                    );
                }

                return n as u32;
            }

            // If FIN received and no more data, signal EOF
            if tcb.fin_received {
                tcb.waiting_tid = 0;
                return 0;
            }

            // If connection is not in a receivable state
            match tcb.state {
                TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2 => {}
                TcpState::CloseWait => {
                    tcb.waiting_tid = 0;
                    return 0;
                }
                _ => {
                    tcb.waiting_tid = 0;
                    return u32::MAX;
                }
            }

            // Check timeout before blocking
            let now = crate::arch::hal::timer_current_ticks();
            if now.wrapping_sub(start) >= timeout_ticks {
                tcb.waiting_tid = 0;
                return u32::MAX;
            }

            // Race-free publish-state-then-block:
            //
            //   1. Set waiting_tid so the input path knows whom to wake.
            //   2. Transition to Blocked while still holding TCP_CONNECTIONS.
            //
            // The input path acquires TCP_CONNECTIONS before reading
            // waiting_tid and calling try_wake_thread(), so it cannot run
            // between (1) and (2). When we drop the lock and call schedule(),
            // any subsequent wake observes state==Blocked and succeeds.
            tcb.waiting_tid = tid;
            let wake_at = now.wrapping_add(1);
            crate::task::scheduler::prepare_to_block_until(wake_at);
        }
        // Lock is dropped here — safe to block.

        // Sleep at most 1 tick (1 ms with TICK_HZ=1000). Woken earlier by
        // try_wake_thread() — see prepare_to_block_until() above for the
        // race-free publish-state-then-block sequence.
        crate::task::scheduler::schedule();

        // After waking, process incoming packets and TCP timers. The timer
        // pass is needed here too: long blocking reads must still flush
        // delayed ACKs and window updates promptly.
        crate::net::poll();
    }
}

/// Check bytes available to read on a TCP connection.
/// Returns: >0 = bytes in recv_buf, 0 = no data yet, u32::MAX-1 = EOF/FIN, u32::MAX = error.
pub fn recv_available(socket_id: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }
    let conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_ref() {
        Some(t) => t,
        None => return u32::MAX,
    };
    match &table[id] {
        Some(tcb) => {
            if tcb.reset_received {
                return u32::MAX;
            }
            if !tcb.recv_buf.is_empty() {
                return tcb.recv_buf.len() as u32;
            }
            if tcb.fin_received {
                return u32::MAX - 1;
            }
            match tcb.state {
                TcpState::CloseWait | TcpState::Closed => u32::MAX - 1,
                _ => 0,
            }
        }
        None => u32::MAX,
    }
}
