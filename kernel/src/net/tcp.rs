//! TCP (Transmission Control Protocol) -- connection-oriented, reliable transport.
//!
//! Supports active open only (no listen/accept). Uses stop-and-wait for sending.
//! Fixed 16-slot connection table with retransmission and TIME_WAIT cleanup.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use super::types::Ipv4Addr;
use super::ipv4::Ipv4Packet;
use crate::sync::spinlock::Spinlock;

// TCP header flags
const FIN: u8 = 0x01;
const SYN: u8 = 0x02;
const RST: u8 = 0x04;
const PSH: u8 = 0x08;
const ACK: u8 = 0x10;

const TCP_HEADER_LEN: usize = 20;
const MAX_CONNECTIONS: usize = 16;
const RECV_BUF_SIZE: usize = 16384;
const WINDOW_SIZE: u16 = 8192;
const MSS: usize = 1460;
const RETRANSMIT_TICKS: u32 = 300; // 3 seconds at 100Hz
const MAX_RETRANSMITS: u32 = 5;
const TIME_WAIT_TICKS: u32 = 200; // 2 seconds at 100Hz

/// TCP connection state machine states per RFC 793.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    SynSent,
    Established,
    FinWait1,
    FinWait2,
    TimeWait,
    CloseWait,
    LastAck,
}

/// Transmission Control Block
struct Tcb {
    state: TcpState,
    local_ip: Ipv4Addr,
    local_port: u16,
    remote_ip: Ipv4Addr,
    remote_port: u16,

    // Send sequence variables
    snd_iss: u32,   // initial send sequence number
    snd_una: u32,   // oldest unacknowledged
    snd_nxt: u32,   // next to send
    snd_wnd: u16,   // send window

    // Receive sequence variables
    rcv_irs: u32,   // initial receive sequence number
    rcv_nxt: u32,   // next expected

    // Receive buffer
    recv_buf: VecDeque<u8>,

    // Retransmission
    last_sent_data: Vec<u8>,
    last_sent_seq: u32,
    last_sent_flags: u8,
    retransmit_count: u32,
    last_send_tick: u32,

    // State flags
    fin_received: bool,
    reset_received: bool,

    // TIME_WAIT timer
    time_wait_start: u32,
}

impl Tcb {
    fn new(local_ip: Ipv4Addr, local_port: u16, remote_ip: Ipv4Addr, remote_port: u16) -> Self {
        // Use PIT ticks as a simple ISS
        let iss = crate::arch::x86::pit::get_ticks().wrapping_mul(2654435761);
        Tcb {
            state: TcpState::Closed,
            local_ip,
            local_port,
            remote_ip,
            remote_port,
            snd_iss: iss,
            snd_una: iss,
            snd_nxt: iss,
            snd_wnd: 0,
            rcv_irs: 0,
            rcv_nxt: 0,
            recv_buf: VecDeque::with_capacity(RECV_BUF_SIZE),
            last_sent_data: Vec::new(),
            last_sent_seq: 0,
            last_sent_flags: 0,
            retransmit_count: 0,
            last_send_tick: 0,
            fin_received: false,
            reset_received: false,
            time_wait_start: 0,
        }
    }
}

struct TcpSegment {
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    payload: Vec<u8>,
    src_ip: Ipv4Addr,
}

static TCP_CONNECTIONS: Spinlock<Option<[Option<Tcb>; 16]>> = Spinlock::new(None);
static NEXT_PORT: Spinlock<u16> = Spinlock::new(49152);

/// Initialize the TCP connection table. Must be called before `connect()`.
pub fn init() {
    let mut conns = TCP_CONNECTIONS.lock();
    // Initialize array with const - can't use Default for Option<Tcb>
    *conns = Some([
        None, None, None, None, None, None, None, None,
        None, None, None, None, None, None, None, None,
    ]);
    crate::serial_println!("[OK] TCP initialized ({} slots)", MAX_CONNECTIONS);
}

fn alloc_ephemeral_port() -> u16 {
    let mut port = NEXT_PORT.lock();
    let p = *port;
    *port = if p >= 65535 { 49152 } else { p + 1 };
    p
}

fn parse_tcp(pkt: &Ipv4Packet<'_>) -> Option<TcpSegment> {
    let data = pkt.payload;
    if data.len() < TCP_HEADER_LEN {
        return None;
    }

    let src_port = ((data[0] as u16) << 8) | data[1] as u16;
    let dst_port = ((data[2] as u16) << 8) | data[3] as u16;
    let seq = ((data[4] as u32) << 24) | ((data[5] as u32) << 16)
        | ((data[6] as u32) << 8) | data[7] as u32;
    let ack = ((data[8] as u32) << 24) | ((data[9] as u32) << 16)
        | ((data[10] as u32) << 8) | data[11] as u32;
    let data_offset = ((data[12] >> 4) as usize) * 4;
    let flags = data[13] & 0x3F;
    let window = ((data[14] as u16) << 8) | data[15] as u16;

    if data_offset < TCP_HEADER_LEN || data_offset > data.len() {
        return None;
    }

    let payload = if data_offset < data.len() {
        Vec::from(&data[data_offset..])
    } else {
        Vec::new()
    };

    Some(TcpSegment {
        src_port,
        dst_port,
        seq,
        ack,
        flags,
        window,
        payload,
        src_ip: pkt.src,
    })
}

/// Build and send a TCP segment.
fn send_segment(
    local_ip: Ipv4Addr,
    local_port: u16,
    remote_ip: Ipv4Addr,
    remote_port: u16,
    seq: u32,
    ack_num: u32,
    flags: u8,
    payload: &[u8],
) -> bool {
    let tcp_len = TCP_HEADER_LEN + payload.len();
    let mut segment = Vec::with_capacity(tcp_len);

    // Source port
    segment.push((local_port >> 8) as u8);
    segment.push((local_port & 0xFF) as u8);
    // Dest port
    segment.push((remote_port >> 8) as u8);
    segment.push((remote_port & 0xFF) as u8);
    // Sequence number
    segment.push((seq >> 24) as u8);
    segment.push((seq >> 16) as u8);
    segment.push((seq >> 8) as u8);
    segment.push(seq as u8);
    // Ack number
    segment.push((ack_num >> 24) as u8);
    segment.push((ack_num >> 16) as u8);
    segment.push((ack_num >> 8) as u8);
    segment.push(ack_num as u8);
    // Data offset (5 = 20 bytes / 4) + reserved
    segment.push(0x50);
    // Flags
    segment.push(flags);
    // Window
    segment.push((WINDOW_SIZE >> 8) as u8);
    segment.push((WINDOW_SIZE & 0xFF) as u8);
    // Checksum placeholder
    segment.push(0);
    segment.push(0);
    // Urgent pointer
    segment.push(0);
    segment.push(0);

    // Payload
    segment.extend_from_slice(payload);

    // Compute checksum with pseudo-header
    let pseudo_sum = super::checksum::pseudo_header_checksum(
        local_ip.as_bytes(),
        remote_ip.as_bytes(),
        super::ipv4::PROTO_TCP,
        tcp_len as u16,
    );

    // Add segment data to checksum
    let mut sum = pseudo_sum;
    let mut i = 0;
    while i + 1 < segment.len() {
        sum += ((segment[i] as u32) << 8) | (segment[i + 1] as u32);
        i += 2;
    }
    if i < segment.len() {
        sum += (segment[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let cksum = !(sum as u16);
    segment[16] = (cksum >> 8) as u8;
    segment[17] = (cksum & 0xFF) as u8;

    super::ipv4::send_ipv4(remote_ip, super::ipv4::PROTO_TCP, &segment)
}

/// Send a RST for an unexpected segment (no connection found).
fn send_rst(seg: &TcpSegment) {
    let cfg = super::config();
    if seg.flags & RST != 0 {
        return; // Never RST a RST
    }
    if seg.flags & ACK != 0 {
        send_segment(cfg.ip, seg.dst_port, seg.src_ip, seg.src_port,
                     seg.ack, 0, RST, &[]);
    } else {
        let ack_val = seg.seq.wrapping_add(seg.payload.len() as u32)
            .wrapping_add(if seg.flags & SYN != 0 { 1 } else { 0 })
            .wrapping_add(if seg.flags & FIN != 0 { 1 } else { 0 });
        send_segment(cfg.ip, seg.dst_port, seg.src_ip, seg.src_port,
                     0, ack_val, RST | ACK, &[]);
    }
}

/// Active open: connect to a remote host. Returns socket ID or u32::MAX on error.
pub fn connect(remote_ip: Ipv4Addr, remote_port: u16, timeout_ticks: u32) -> u32 {
    let cfg = super::config();
    let local_port = alloc_ephemeral_port();

    // Find a free slot and insert TCB
    let slot_id = {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };
        let mut found = None;
        for (i, slot) in table.iter_mut().enumerate() {
            if slot.is_none() {
                let mut tcb = Tcb::new(cfg.ip, local_port, remote_ip, remote_port);
                tcb.state = TcpState::SynSent;
                tcb.snd_nxt = tcb.snd_iss.wrapping_add(1);
                tcb.last_sent_flags = SYN;
                tcb.last_send_tick = crate::arch::x86::pit::get_ticks();
                *slot = Some(tcb);
                found = Some(i);
                break;
            }
        }
        match found {
            Some(id) => id,
            None => return u32::MAX,
        }
    };

    // Send SYN (outside lock!)
    let iss = {
        let conns = TCP_CONNECTIONS.lock();
        let table = conns.as_ref().unwrap();
        table[slot_id].as_ref().unwrap().snd_iss
    };

    crate::serial_println!("TCP: connecting to {}:{} from port {}", remote_ip, remote_port, local_port);
    send_segment(cfg.ip, local_port, remote_ip, remote_port, iss, 0, SYN, &[]);

    // Wait for connection to establish
    let start = crate::arch::x86::pit::get_ticks();
    loop {
        // Poll network (outside lock!)
        super::poll();

        // Check state
        {
            let conns = TCP_CONNECTIONS.lock();
            let table = conns.as_ref().unwrap();
            if let Some(tcb) = &table[slot_id] {
                match tcb.state {
                    TcpState::Established => {
                        crate::serial_println!("TCP: connected socket {}", slot_id);
                        return slot_id as u32;
                    }
                    TcpState::Closed => {
                        crate::serial_println!("TCP: connection refused");
                        return u32::MAX;
                    }
                    _ => {}
                }
                if tcb.reset_received {
                    // Clean up
                    drop(conns);
                    let mut conns = TCP_CONNECTIONS.lock();
                    let table = conns.as_mut().unwrap();
                    table[slot_id] = None;
                    return u32::MAX;
                }
            } else {
                return u32::MAX;
            }
        }

        let now = crate::arch::x86::pit::get_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            // Timeout - clean up
            crate::serial_println!("TCP: connect timeout");
            let mut conns = TCP_CONNECTIONS.lock();
            let table = conns.as_mut().unwrap();
            table[slot_id] = None;
            return u32::MAX;
        }

        core::hint::spin_loop();
    }
}

/// Send data on an established connection. Returns bytes sent or u32::MAX on error.
/// Uses stop-and-wait: sends one MSS-sized segment, waits for ACK, repeats.
pub fn send(socket_id: u32, data: &[u8], timeout_ticks: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }

    let mut offset = 0usize;
    while offset < data.len() {
        let chunk_end = (offset + MSS).min(data.len());
        let chunk = &data[offset..chunk_end];

        // Get connection info and send (with lock)
        let (local_ip, local_port, remote_ip, remote_port, seq, ack_num) = {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return u32::MAX,
            };
            let tcb = match table[id].as_mut() {
                Some(t) => t,
                None => return u32::MAX,
            };
            if tcb.state != TcpState::Established {
                return if offset > 0 { offset as u32 } else { u32::MAX };
            }
            let info = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                       tcb.snd_nxt, tcb.rcv_nxt);
            tcb.last_sent_data = Vec::from(chunk);
            tcb.last_sent_seq = tcb.snd_nxt;
            tcb.last_sent_flags = PSH | ACK;
            tcb.last_send_tick = crate::arch::x86::pit::get_ticks();
            tcb.retransmit_count = 0;
            tcb.snd_nxt = tcb.snd_nxt.wrapping_add(chunk.len() as u32);
            info
        };

        // Send segment (outside lock)
        send_segment(local_ip, local_port, remote_ip, remote_port,
                     seq, ack_num, PSH | ACK, chunk);

        // Wait for ACK
        let start = crate::arch::x86::pit::get_ticks();
        loop {
            super::poll();

            let acked = {
                let conns = TCP_CONNECTIONS.lock();
                let table = conns.as_ref().unwrap();
                if let Some(tcb) = &table[id] {
                    if tcb.reset_received || tcb.state == TcpState::Closed {
                        return if offset > 0 { offset as u32 } else { u32::MAX };
                    }
                    // Check if our data was acknowledged
                    let expected_ack = seq.wrapping_add(chunk.len() as u32);
                    is_seq_gte(tcb.snd_una, expected_ack)
                } else {
                    return u32::MAX;
                }
            };

            if acked {
                break;
            }

            let now = crate::arch::x86::pit::get_ticks();
            if now.wrapping_sub(start) >= timeout_ticks {
                crate::serial_println!("TCP: send timeout on socket {}", socket_id);
                return if offset > 0 { offset as u32 } else { u32::MAX };
            }

            core::hint::spin_loop();
        }

        offset = chunk_end;
    }

    offset as u32
}

/// Receive data from an established connection.
/// Returns bytes received, 0 if connection closed (FIN), u32::MAX on error.
pub fn recv(socket_id: u32, buf: &mut [u8], timeout_ticks: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS || buf.is_empty() {
        return u32::MAX;
    }

    let start = crate::arch::x86::pit::get_ticks();
    loop {
        super::poll();

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
                return u32::MAX;
            }

            // Drain data from recv buffer
            if !tcb.recv_buf.is_empty() {
                let n = tcb.recv_buf.len().min(buf.len());
                for i in 0..n {
                    buf[i] = tcb.recv_buf.pop_front().unwrap();
                }
                return n as u32;
            }

            // If FIN received and no more data, signal EOF
            if tcb.fin_received {
                return 0;
            }

            // If connection is not in a receivable state
            match tcb.state {
                TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2 => {}
                TcpState::CloseWait => {
                    // Remote already closed, no more data coming
                    return 0;
                }
                _ => return u32::MAX,
            }
        }

        let now = crate::arch::x86::pit::get_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            return u32::MAX;
        }

        core::hint::spin_loop();
    }
}

/// Close a TCP connection. Sends FIN, waits for ACK.
pub fn close(socket_id: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }

    // Get info and update state
    let send_info = {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };
        let tcb = match table[id].as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };

        match tcb.state {
            TcpState::Established => {
                tcb.state = TcpState::FinWait1;
                let info = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                           tcb.snd_nxt, tcb.rcv_nxt);
                tcb.last_sent_flags = FIN | ACK;
                tcb.last_send_tick = crate::arch::x86::pit::get_ticks();
                tcb.retransmit_count = 0;
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(1);
                Some(info)
            }
            TcpState::CloseWait => {
                tcb.state = TcpState::LastAck;
                let info = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                           tcb.snd_nxt, tcb.rcv_nxt);
                tcb.last_sent_flags = FIN | ACK;
                tcb.last_send_tick = crate::arch::x86::pit::get_ticks();
                tcb.retransmit_count = 0;
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(1);
                Some(info)
            }
            TcpState::Closed => {
                table[id] = None;
                return 0;
            }
            _ => {
                // Already closing or time-waiting, just force close
                table[id] = None;
                return 0;
            }
        }
    };

    if let Some((local_ip, local_port, remote_ip, remote_port, seq, ack_num)) = send_info {
        send_segment(local_ip, local_port, remote_ip, remote_port, seq, ack_num, FIN | ACK, &[]);
    }

    // Wait for close to complete (with timeout)
    let start = crate::arch::x86::pit::get_ticks();
    let timeout = 500; // 5 seconds
    loop {
        super::poll();

        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return 0,
            };
            if let Some(tcb) = &table[id] {
                match tcb.state {
                    TcpState::Closed | TcpState::TimeWait => {
                        if tcb.state == TcpState::Closed {
                            table[id] = None;
                        }
                        return 0;
                    }
                    _ => {}
                }
                if tcb.reset_received {
                    table[id] = None;
                    return 0;
                }
            } else {
                return 0;
            }
        }

        let now = crate::arch::x86::pit::get_ticks();
        if now.wrapping_sub(start) >= timeout {
            // Force close with RST — extract info then drop lock before sending
            let rst_info = {
                let mut conns = TCP_CONNECTIONS.lock();
                let table = conns.as_mut().unwrap();
                let info = table[id].as_ref().map(|tcb| {
                    (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                     tcb.snd_nxt, tcb.rcv_nxt)
                });
                table[id] = None;
                info
            };
            if let Some((lip, lp, rip, rp, sn, rn)) = rst_info {
                send_segment(lip, lp, rip, rp, sn, rn, RST, &[]);
            }
            return 0;
        }

        core::hint::spin_loop();
    }
}

/// Get connection state. Returns TcpState as u32, or u32::MAX if not found.
pub fn status(socket_id: u32) -> u32 {
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
        Some(tcb) => tcb.state as u32,
        None => u32::MAX,
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

/// Half-close (SHUT_WR): send FIN but don't block. Connection can still receive.
pub fn shutdown_write(socket_id: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }

    let send_info = {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };
        let tcb = match table[id].as_mut() {
            Some(t) => t,
            None => return u32::MAX,
        };

        match tcb.state {
            TcpState::Established => {
                tcb.state = TcpState::FinWait1;
                let info = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                           tcb.snd_nxt, tcb.rcv_nxt);
                tcb.last_sent_flags = FIN | ACK;
                tcb.last_send_tick = crate::arch::x86::pit::get_ticks();
                tcb.retransmit_count = 0;
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(1);
                Some(info)
            }
            _ => None,
        }
    };

    if let Some((local_ip, local_port, remote_ip, remote_port, seq, ack_num)) = send_info {
        send_segment(local_ip, local_port, remote_ip, remote_port, seq, ack_num, FIN | ACK, &[]);
    }

    0
}

/// Deferred send info — collected while holding the lock, sent after dropping it.
struct DeferredSend {
    local_ip: Ipv4Addr,
    local_port: u16,
    remote_ip: Ipv4Addr,
    remote_port: u16,
    seq: u32,
    ack_num: u32,
    flags: u8,
}

/// Handle an incoming TCP segment. Called from ipv4::handle_ipv4().
pub fn handle_tcp(pkt: &Ipv4Packet<'_>) {
    let seg = match parse_tcp(pkt) {
        Some(s) => s,
        None => return,
    };

    // Process segment under lock, collect deferred sends
    let deferred: Option<DeferredSend> = {
        let mut conns = TCP_CONNECTIONS.lock();
        let table = match conns.as_mut() {
            Some(t) => t,
            None => return,
        };

        // Find matching connection
        let slot_idx = table.iter().position(|slot| {
            if let Some(tcb) = slot {
                tcb.local_port == seg.dst_port
                    && tcb.remote_port == seg.src_port
                    && tcb.remote_ip == seg.src_ip
            } else {
                false
            }
        });

        let idx = match slot_idx {
            Some(i) => i,
            None => {
                // No matching connection - send RST after dropping lock
                drop(conns);
                send_rst(&seg);
                return;
            }
        };

        // RST handling — always process
        if seg.flags & RST != 0 {
            crate::serial_println!("TCP: RST received on socket {}", idx);
            table[idx].as_mut().unwrap().reset_received = true;
            table[idx].as_mut().unwrap().state = TcpState::Closed;
            return;
        }

        let tcb = table[idx].as_mut().unwrap();
        let now = crate::arch::x86::pit::get_ticks();

        match tcb.state {
            TcpState::SynSent => {
                if seg.flags & SYN != 0 && seg.flags & ACK != 0 {
                    if seg.ack == tcb.snd_nxt {
                        tcb.rcv_irs = seg.seq;
                        tcb.rcv_nxt = seg.seq.wrapping_add(1);
                        tcb.snd_una = seg.ack;
                        tcb.snd_wnd = seg.window;
                        tcb.state = TcpState::Established;
                        tcb.last_sent_data.clear();
                        tcb.retransmit_count = 0;
                        Some(DeferredSend {
                            local_ip: tcb.local_ip, local_port: tcb.local_port,
                            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
                            seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
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
                    })
                } else {
                    None
                }
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
                // Accept data (deferred ACK handled by accept_data_deferred)
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
                    })
                } else {
                    None
                }
            }

            TcpState::Closed => None,
        }
    }; // lock dropped here

    // Send deferred segment outside lock
    if let Some(ds) = deferred {
        send_segment(ds.local_ip, ds.local_port, ds.remote_ip, ds.remote_port,
                    ds.seq, ds.ack_num, ds.flags, &[]);
    }
}

/// Handle data/ACK/FIN in ESTABLISHED state. Returns deferred send if needed.
fn handle_established(tcb: &mut Tcb, seg: &TcpSegment) -> Option<DeferredSend> {
    // Process ACK
    if seg.flags & ACK != 0 {
        if is_seq_gt(seg.ack, tcb.snd_una) && is_seq_lte(seg.ack, tcb.snd_nxt) {
            tcb.snd_una = seg.ack;
            tcb.snd_wnd = seg.window;
            if tcb.snd_una == tcb.snd_nxt {
                tcb.last_sent_data.clear();
                tcb.retransmit_count = 0;
            }
        }
    }

    // Accept data
    let data_ack = accept_data_deferred(tcb, seg);

    // Handle FIN
    if seg.flags & FIN != 0 {
        tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(1);
        tcb.fin_received = true;
        tcb.state = TcpState::CloseWait;
        Some(DeferredSend {
            local_ip: tcb.local_ip, local_port: tcb.local_port,
            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
            seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
        })
    } else {
        data_ack
    }
}

/// Accept in-order data. Returns deferred ACK send if data was accepted.
fn accept_data_deferred(tcb: &mut Tcb, seg: &TcpSegment) -> Option<DeferredSend> {
    if seg.payload.is_empty() {
        return None;
    }

    if seg.seq == tcb.rcv_nxt {
        let space = RECV_BUF_SIZE - tcb.recv_buf.len();
        let take = seg.payload.len().min(space);
        for i in 0..take {
            tcb.recv_buf.push_back(seg.payload[i]);
        }
        tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(take as u32);
        Some(DeferredSend {
            local_ip: tcb.local_ip, local_port: tcb.local_port,
            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
            seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
        })
    } else if is_seq_gt(tcb.rcv_nxt, seg.seq) {
        // Duplicate — send duplicate ACK
        Some(DeferredSend {
            local_ip: tcb.local_ip, local_port: tcb.local_port,
            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
            seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
        })
    } else {
        None
    }
}

/// Check retransmissions and TIME_WAIT cleanup. Called from net::poll().
pub fn check_retransmissions() {
    let now = crate::arch::x86::pit::get_ticks();

    let mut conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_mut() {
        Some(t) => t,
        None => return,
    };

    for i in 0..MAX_CONNECTIONS {
        let should_retransmit;
        let should_cleanup;

        if let Some(tcb) = &table[i] {
            // TIME_WAIT cleanup
            should_cleanup = tcb.state == TcpState::TimeWait
                && now.wrapping_sub(tcb.time_wait_start) >= TIME_WAIT_TICKS;

            // Closed cleanup
            if tcb.state == TcpState::Closed {
                table[i] = None;
                continue;
            }

            // Retransmission check
            should_retransmit = !tcb.last_sent_data.is_empty()
                && now.wrapping_sub(tcb.last_send_tick) >= RETRANSMIT_TICKS
                && tcb.retransmit_count < MAX_RETRANSMITS
                && (tcb.state == TcpState::Established || tcb.state == TcpState::SynSent);
        } else {
            continue;
        }

        if should_cleanup {
            table[i] = None;
            continue;
        }

        if should_retransmit {
            let tcb = table[i].as_mut().unwrap();
            tcb.retransmit_count += 1;
            tcb.last_send_tick = now;
            let data = tcb.last_sent_data.clone();
            let flags = tcb.last_sent_flags;
            let seq = tcb.last_sent_seq;
            let ack_num = tcb.rcv_nxt;
            let (lip, lp, rip, rp) = (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port);

            crate::serial_println!("TCP: retransmit #{} socket {} seq={}", tcb.retransmit_count, i, seq);

            // Must drop lock before sending
            drop(conns);
            send_segment(lip, lp, rip, rp, seq, ack_num, flags, &data);
            // Can't continue iterating after dropping lock, so return
            return;
        }
    }
}

// Sequence number comparison helpers (wrapping-safe)
fn is_seq_gt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

fn is_seq_gte(a: u32, b: u32) -> bool {
    a == b || is_seq_gt(a, b)
}

fn is_seq_lte(a: u32, b: u32) -> bool {
    a == b || !is_seq_gt(a, b)
}
