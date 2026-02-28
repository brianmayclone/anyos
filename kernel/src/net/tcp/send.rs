//! TCP segment construction and data sending.
//!
//! Provides low-level segment building (`send_segment`, `send_syn_segment`)
//! with dynamic receive window advertisement, and the high-level `send()`
//! function with sliding window, batched locking, and send buffer tracking.

use core::sync::atomic::Ordering;
use super::tcb::*;
use super::{TCP_CONNECTIONS, TCP_SEGMENTS_SENT, TCP_RESETS_SENT};
use crate::net::types::Ipv4Addr;

// ── Low-level segment construction ──────────────────────────────────

/// Build and send a TCP segment with dynamic window advertisement.
///
/// The `window` parameter is the raw value to place in the TCP header
/// (already shifted by the caller if window scaling is in use).
pub(crate) fn send_segment(
    local_ip: Ipv4Addr,
    local_port: u16,
    remote_ip: Ipv4Addr,
    remote_port: u16,
    seq: u32,
    ack_num: u32,
    flags: u8,
    window: u16,
    payload: &[u8],
) -> bool {
    let tcp_len = TCP_HEADER_LEN + payload.len();
    let mut segment = [0u8; 1536]; // stack buffer, fits MTU
    if tcp_len > segment.len() { return false; }

    // Source port
    segment[0] = (local_port >> 8) as u8;
    segment[1] = (local_port & 0xFF) as u8;
    // Dest port
    segment[2] = (remote_port >> 8) as u8;
    segment[3] = (remote_port & 0xFF) as u8;
    // Sequence number
    segment[4] = (seq >> 24) as u8;
    segment[5] = (seq >> 16) as u8;
    segment[6] = (seq >> 8) as u8;
    segment[7] = seq as u8;
    // Ack number
    segment[8] = (ack_num >> 24) as u8;
    segment[9] = (ack_num >> 16) as u8;
    segment[10] = (ack_num >> 8) as u8;
    segment[11] = ack_num as u8;
    // Data offset (5 = 20 bytes / 4) + reserved
    segment[12] = 0x50;
    // Flags
    segment[13] = flags;
    // Window (dynamic — reflects actual recv buffer space)
    segment[14] = (window >> 8) as u8;
    segment[15] = (window & 0xFF) as u8;
    // Checksum placeholder (already 0)
    // Urgent pointer (already 0)

    // Payload
    if !payload.is_empty() {
        segment[TCP_HEADER_LEN..tcp_len].copy_from_slice(payload);
    }

    tcp_checksum_and_send(local_ip, remote_ip, &mut segment[..tcp_len], flags)
}

/// Build and send a SYN or SYN-ACK segment with TCP options (MSS + Window Scale).
///
/// SYN segments advertise the unscaled window per RFC 7323.
pub(crate) fn send_syn_segment(
    local_ip: Ipv4Addr,
    local_port: u16,
    remote_ip: Ipv4Addr,
    remote_port: u16,
    seq: u32,
    ack_num: u32,
    flags: u8,
) -> bool {
    // Options: MSS (4 bytes) + NOP (1) + Window Scale (3 bytes) = 8 bytes
    // Total header: 20 + 8 = 28 bytes, data_offset = 7
    const OPT_LEN: usize = 8;
    let tcp_len = TCP_HEADER_LEN + OPT_LEN;
    let mut segment = [0u8; 28];

    // Source port
    segment[0] = (local_port >> 8) as u8;
    segment[1] = (local_port & 0xFF) as u8;
    // Dest port
    segment[2] = (remote_port >> 8) as u8;
    segment[3] = (remote_port & 0xFF) as u8;
    // Sequence number
    segment[4] = (seq >> 24) as u8;
    segment[5] = (seq >> 16) as u8;
    segment[6] = (seq >> 8) as u8;
    segment[7] = seq as u8;
    // Ack number
    segment[8] = (ack_num >> 24) as u8;
    segment[9] = (ack_num >> 16) as u8;
    segment[10] = (ack_num >> 8) as u8;
    segment[11] = ack_num as u8;
    // Data offset: 7 (28 bytes / 4) + reserved
    segment[12] = 0x70;
    // Flags
    segment[13] = flags;
    // Window (SYN window is NOT scaled per RFC 7323)
    let window: u16 = 65535;
    segment[14] = (window >> 8) as u8;
    segment[15] = (window & 0xFF) as u8;
    // Checksum placeholder (bytes 16-17, already 0)
    // Urgent pointer (bytes 18-19, already 0)

    // TCP Options (8 bytes):
    // MSS option: Kind=2, Len=4, MSS=1460
    segment[20] = 2;
    segment[21] = 4;
    segment[22] = (MSS >> 8) as u8;
    segment[23] = (MSS & 0xFF) as u8;
    // NOP padding
    segment[24] = 1;
    // Window Scale option: Kind=3, Len=3, Shift=OUR_WINDOW_SHIFT
    segment[25] = 3;
    segment[26] = 3;
    segment[27] = OUR_WINDOW_SHIFT;

    tcp_checksum_and_send(local_ip, remote_ip, &mut segment[..tcp_len], flags)
}

/// Compute TCP checksum and send via IPv4.
fn tcp_checksum_and_send(local_ip: Ipv4Addr, remote_ip: Ipv4Addr, segment: &mut [u8], flags: u8) -> bool {
    let tcp_len = segment.len();

    // Compute checksum with pseudo-header
    let pseudo_sum = crate::net::checksum::pseudo_header_checksum(
        local_ip.as_bytes(),
        remote_ip.as_bytes(),
        crate::net::ipv4::PROTO_TCP,
        tcp_len as u16,
    );

    // Ensure checksum field is zero before computing
    segment[16] = 0;
    segment[17] = 0;

    let mut sum = pseudo_sum;
    let mut i = 0;
    while i + 1 < tcp_len {
        sum += ((segment[i] as u32) << 8) | (segment[i + 1] as u32);
        i += 2;
    }
    if i < tcp_len {
        sum += (segment[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let cksum = !(sum as u16);
    segment[16] = (cksum >> 8) as u8;
    segment[17] = (cksum & 0xFF) as u8;

    TCP_SEGMENTS_SENT.fetch_add(1, Ordering::Relaxed);
    if flags & RST != 0 {
        TCP_RESETS_SENT.fetch_add(1, Ordering::Relaxed);
    }
    crate::net::ipv4::send_ipv4(remote_ip, crate::net::ipv4::PROTO_TCP, segment)
}

// ── High-level send ─────────────────────────────────────────────────

/// Send data on an established connection. Returns bytes sent or u32::MAX on error.
///
/// Uses sliding window with batched locking. Data is appended to the
/// connection's send buffer for retransmission support.
pub fn send(socket_id: u32, data: &[u8], timeout_ticks: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS || data.is_empty() {
        return if data.is_empty() { 0 } else { u32::MAX };
    }

    // Record the base sequence number at the start of our data.
    let send_base_seq = {
        let conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_ref() {
            Some(t) => t,
            None => return u32::MAX,
        };
        match &table[id] {
            Some(tcb) if tcb.state == TcpState::Established => tcb.snd_nxt,
            _ => return u32::MAX,
        }
    };

    let start = crate::arch::x86::pit::get_ticks();
    let mut send_offset = 0usize;

    // Stack-allocated batch buffer — avoids heap alloc per segment.
    let mut batch: [core::mem::MaybeUninit<BatchSegment>; SEND_BATCH_SIZE] =
        unsafe { core::mem::MaybeUninit::uninit().assume_init() };

    loop {
        // ── Single lock acquisition: compute ack_offset, prepare batch ──
        let (ack_offset, batch_count) = {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return u32::MAX,
            };
            let tcb = match table[id].as_mut() {
                Some(t) => t,
                None => return u32::MAX,
            };

            if tcb.reset_received || tcb.state == TcpState::Closed {
                return if send_offset > 0 { send_offset as u32 } else { u32::MAX };
            }
            if tcb.state != TcpState::Established {
                return if send_offset > 0 { send_offset as u32 } else { u32::MAX };
            }

            // Drain acknowledged bytes from send buffer
            let acked_bytes = tcb.snd_una.wrapping_sub(send_base_seq) as usize;
            let ack_offset = acked_bytes.min(data.len());

            // Trim send_buf: remove data that has been ACKed
            let buf_acked = acked_bytes.min(tcb.send_buf.len());
            if buf_acked > 0 {
                tcb.send_buf.drain(..buf_acked);
            }

            // All data acknowledged?
            if ack_offset >= data.len() {
                return data.len() as u32;
            }

            let snd_wnd = (tcb.snd_wnd as usize).max(MSS);
            let window = snd_wnd.min(MAX_IN_FLIGHT);
            let now = crate::arch::x86::pit::get_ticks();
            let win_val = tcb.advertised_window();

            // Prepare up to SEND_BATCH_SIZE segments under this single lock.
            let mut count = 0usize;
            while send_offset < data.len() && count < SEND_BATCH_SIZE {
                let in_flight = send_offset - ack_offset;
                if in_flight >= window {
                    break; // window full
                }
                let remaining_window = window - in_flight;
                let chunk_end = (send_offset + MSS).min(data.len()).min(send_offset + remaining_window);

                let seg = BatchSegment {
                    local_ip: tcb.local_ip,
                    local_port: tcb.local_port,
                    remote_ip: tcb.remote_ip,
                    remote_port: tcb.remote_port,
                    seq: tcb.snd_nxt,
                    ack_num: tcb.rcv_nxt,
                    window: win_val,
                    data_start: send_offset,
                    data_end: chunk_end,
                };

                // Append to send buffer for retransmission
                let chunk = &data[send_offset..chunk_end];
                let chunk_len = chunk.len();
                if tcb.send_buf.len() + chunk_len <= MAX_SEND_BUF {
                    tcb.send_buf.extend(chunk.iter());
                }

                // Update TCB for this segment
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(chunk_len as u32);
                tcb.last_send_tick = now;
                tcb.retransmit_count = 0;

                batch[count].write(seg);
                count += 1;
                send_offset = chunk_end;
            }

            (ack_offset, count)
        }; // lock released here

        // ── Send all batched segments outside the lock ──
        for i in 0..batch_count {
            let seg = unsafe { batch[i].assume_init_ref() };
            send_segment(seg.local_ip, seg.local_port, seg.remote_ip, seg.remote_port,
                         seg.seq, seg.ack_num, PSH | ACK, seg.window,
                         &data[seg.data_start..seg.data_end]);
        }

        // All data acknowledged?
        if ack_offset >= data.len() {
            return data.len() as u32;
        }

        // Poll network for incoming ACKs (fast path).
        crate::net::poll_rx();

        // Check timeout.
        let now = crate::arch::x86::pit::get_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            crate::serial_println!("TCP: send timeout on socket {}", socket_id);
            return if ack_offset > 0 { ack_offset as u32 } else { u32::MAX };
        }

        // If no segments were sent (window full), sleep briefly.
        if batch_count == 0 {
            let wake_at = crate::arch::x86::pit::get_ticks() + 1;
            crate::task::scheduler::sleep_until(wake_at);
            crate::net::poll_rx();
        }
    }
}
