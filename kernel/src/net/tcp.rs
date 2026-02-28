//! TCP (Transmission Control Protocol) -- connection-oriented, reliable transport.
//!
//! Supports both active open (connect) and passive open (listen/accept).
//! Sliding-window send with batched locking. 64-slot connection table with
//! retransmission, delayed ACKs, and TIME_WAIT cleanup.

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
const MAX_CONNECTIONS: usize = 64;
const RECV_BUF_SIZE: usize = 262144; // 256 KB — large enough for burst traffic
const WINDOW_SIZE: u16 = 65535;      // Raw window value in TCP header
const MSS: usize = 1460;
const RETRANSMIT_TICKS: u32 = 300; // 3 seconds at 100Hz
const MAX_RETRANSMITS: u32 = 5;
const TIME_WAIT_TICKS: u32 = 200; // 2 seconds at 100Hz
const MAX_BACKLOG: usize = 16;    // max pending connections per listener

/// Maximum bytes in flight (sliding window send limit).
/// 1 MB allows efficient large transfers with fewer ACK round-trips.
const MAX_IN_FLIGHT: usize = 1048576;

/// Our TCP Window Scale shift count (RFC 7323).
/// With shift=4, the effective receive window is `raw_window << 4`.
/// WINDOW_SIZE=65535 << 4 = 1,048,560 (~1 MB effective receive window).
const OUR_WINDOW_SHIFT: u8 = 4;

/// Maximum segments to batch per lock acquisition in send().
const SEND_BATCH_SIZE: usize = 64;

/// Delayed ACK: flush ACK after this many accepted data segments.
const DELAYED_ACK_SEGMENTS: u32 = 2;

/// Delayed ACK: flush ACK after this many ticks (20ms at 100Hz).
const DELAYED_ACK_TICKS: u32 = 2;

/// Yield the CPU briefly to avoid busy-waiting in poll loops.
fn poll_yield() {
    crate::task::scheduler::schedule();
}

// ── Global TCP statistics ────────────────────────────────────────────
use core::sync::atomic::{AtomicU64, Ordering};

static TCP_ACTIVE_OPENS: AtomicU64 = AtomicU64::new(0);   // connect() calls
static TCP_PASSIVE_OPENS: AtomicU64 = AtomicU64::new(0);  // accept() completions
static TCP_SEGMENTS_SENT: AtomicU64 = AtomicU64::new(0);
static TCP_SEGMENTS_RECV: AtomicU64 = AtomicU64::new(0);
static TCP_RETRANSMITS: AtomicU64 = AtomicU64::new(0);
static TCP_RESETS_SENT: AtomicU64 = AtomicU64::new(0);
static TCP_CONN_ERRORS: AtomicU64 = AtomicU64::new(0);

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

/// TCP connection state machine states per RFC 793.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpState {
    Closed,       // 0
    SynSent,      // 1
    Established,  // 2
    FinWait1,     // 3
    FinWait2,     // 4
    TimeWait,     // 5
    CloseWait,    // 6
    LastAck,      // 7
    Listen,       // 8
    SynReceived,  // 9
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
    snd_wnd: u32,   // send window (scaled by snd_wnd_shift)

    // Receive sequence variables
    rcv_irs: u32,   // initial receive sequence number
    rcv_nxt: u32,   // next expected

    // TCP Window Scaling (RFC 7323)
    snd_wnd_shift: u8,  // peer's scale factor: snd_wnd = raw_window << shift
    rcv_wnd_shift: u8,  // our scale factor (sent in SYN-ACK/SYN)

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

    // Delayed ACK support
    pending_ack: bool,         // true if we owe the sender an ACK
    ack_seg_count: u32,        // data segments received since last ACK sent
    last_ack_tick: u32,        // tick when last ACK was sent (for delayed ACK timer)

    // TIME_WAIT timer
    time_wait_start: u32,

    // Server socket support
    parent_listener: Option<u8>,  // index of the listener that spawned this connection
    accepted: bool,               // has accept() claimed this connection?

    // Ownership tracking for cleanup on process exit
    owner_tid: u32,               // thread ID that owns this connection (0 = unowned)

    // Blocking I/O: thread waiting for data/state change on this socket.
    // Set by recv()/accept()/connect() before blocking, cleared on wake.
    // 0 means no thread is waiting.
    waiting_tid: u32,
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
            snd_wnd_shift: 0,
            rcv_wnd_shift: 0,
            recv_buf: VecDeque::with_capacity(RECV_BUF_SIZE),
            last_sent_data: Vec::new(),
            last_sent_seq: 0,
            last_sent_flags: 0,
            retransmit_count: 0,
            last_send_tick: 0,
            fin_received: false,
            reset_received: false,
            pending_ack: false,
            ack_seg_count: 0,
            last_ack_tick: 0,
            time_wait_start: 0,
            parent_listener: None,
            accepted: false,
            owner_tid: 0,
            waiting_tid: 0,
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
    /// TCP Window Scale option (Kind=3), present only in SYN segments.
    wscale: Option<u8>,
    /// MSS option (Kind=2), present only in SYN segments.
    peer_mss: Option<u16>,
}

static TCP_CONNECTIONS: Spinlock<Option<Vec<Option<Tcb>>>> = Spinlock::new(None);
static NEXT_PORT: Spinlock<u16> = Spinlock::new(49152);

/// Initialize the TCP connection table. Must be called before `connect()`.
pub fn init() {
    let mut conns = TCP_CONNECTIONS.lock();
    let mut table = Vec::with_capacity(MAX_CONNECTIONS);
    for _ in 0..MAX_CONNECTIONS {
        table.push(None);
    }
    *conns = Some(table);
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

    // Parse TCP options (between fixed header and payload).
    let mut wscale = None;
    let mut peer_mss = None;
    if data_offset > TCP_HEADER_LEN {
        let opts = &data[TCP_HEADER_LEN..data_offset];
        let mut i = 0;
        while i < opts.len() {
            match opts[i] {
                0 => break,        // End of Options
                1 => { i += 1; }   // NOP
                2 => {             // MSS (Kind=2, Len=4)
                    if i + 4 <= opts.len() && opts[i + 1] == 4 {
                        peer_mss = Some(((opts[i + 2] as u16) << 8) | opts[i + 3] as u16);
                    }
                    i += if i + 1 < opts.len() { opts[i + 1] as usize } else { 2 };
                }
                3 => {             // Window Scale (Kind=3, Len=3)
                    if i + 3 <= opts.len() && opts[i + 1] == 3 {
                        wscale = Some(opts[i + 2].min(14)); // RFC 7323: max shift is 14
                    }
                    i += if i + 1 < opts.len() { opts[i + 1] as usize } else { 2 };
                }
                _ => {             // Unknown option — skip using length field
                    if i + 1 < opts.len() && opts[i + 1] >= 2 {
                        i += opts[i + 1] as usize;
                    } else {
                        break;
                    }
                }
            }
        }
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
        wscale,
        peer_mss,
    })
}

/// Build and send a TCP segment.
/// Build and send a TCP segment using a stack-allocated buffer (no heap alloc).
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
    // Window
    segment[14] = (WINDOW_SIZE >> 8) as u8;
    segment[15] = (WINDOW_SIZE & 0xFF) as u8;
    // Checksum placeholder (already 0)
    // Urgent pointer (already 0)

    // Payload
    if !payload.is_empty() {
        segment[TCP_HEADER_LEN..tcp_len].copy_from_slice(payload);
    }

    tcp_checksum_and_send(local_ip, remote_ip, &mut segment[..tcp_len], flags)
}

/// Build and send a SYN or SYN-ACK segment with TCP options (MSS + Window Scale).
fn send_syn_segment(
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
    segment[14] = (WINDOW_SIZE >> 8) as u8;
    segment[15] = (WINDOW_SIZE & 0xFF) as u8;
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

/// Compute TCP checksum and send via IPv4. Shared by send_segment and send_syn_segment.
fn tcp_checksum_and_send(local_ip: Ipv4Addr, remote_ip: Ipv4Addr, segment: &mut [u8], flags: u8) -> bool {
    let tcp_len = segment.len();

    // Compute checksum with pseudo-header
    let pseudo_sum = super::checksum::pseudo_header_checksum(
        local_ip.as_bytes(),
        remote_ip.as_bytes(),
        super::ipv4::PROTO_TCP,
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
    super::ipv4::send_ipv4(remote_ip, super::ipv4::PROTO_TCP, segment)
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
    let tid = crate::task::scheduler::current_tid();

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
                tcb.owner_tid = tid;
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
    TCP_ACTIVE_OPENS.fetch_add(1, Ordering::Relaxed);
    send_syn_segment(cfg.ip, local_port, remote_ip, remote_port, iss, 0, SYN);

    // Wait for connection to establish (blocking — zero CPU while waiting)
    let start = crate::arch::x86::pit::get_ticks();

    // Poll once eagerly to process any pending packets.
    super::poll();

    loop {
        // Check state
        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = conns.as_mut().unwrap();
            if let Some(tcb) = table[slot_id].as_mut() {
                match tcb.state {
                    TcpState::Established => {
                        tcb.waiting_tid = 0;
                        crate::serial_println!("TCP: connected socket {}", slot_id);
                        return slot_id as u32;
                    }
                    TcpState::Closed => {
                        tcb.waiting_tid = 0;
                        crate::serial_println!("TCP: connection refused");
                        return u32::MAX;
                    }
                    _ => {}
                }
                if tcb.reset_received {
                    table[slot_id] = None;
                    return u32::MAX;
                }

                // Check timeout before blocking
                let now = crate::arch::x86::pit::get_ticks();
                if now.wrapping_sub(start) >= timeout_ticks {
                    crate::serial_println!("TCP: connect timeout");
                    table[slot_id] = None;
                    return u32::MAX;
                }

                // Register ourselves as waiting and block
                tcb.waiting_tid = tid;
            } else {
                return u32::MAX;
            }
        }
        // Lock is dropped here — safe to block.

        // Sleep briefly (2 ticks = 20ms). Woken early by try_wake_thread()
        // when SYN-ACK arrives, or by timer for timeout check.
        let wake_at = crate::arch::x86::pit::get_ticks() + 2;
        crate::task::scheduler::sleep_until(wake_at);

        // After waking, poll to process pending packets.
        super::poll();
    }
}

/// Passive open: listen on a local port. Returns listener socket ID or u32::MAX.
pub fn listen(port: u16, _backlog: u16) -> u32 {
    let cfg = super::config();
    let tid = crate::task::scheduler::current_tid();
    let mut conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_mut() {
        Some(t) => t,
        None => return u32::MAX,
    };

    // Check if port is already in use (listen or active connection)
    for (i, slot) in table.iter().enumerate() {
        if let Some(tcb) = slot {
            if tcb.local_port == port && tcb.state == TcpState::Listen {
                crate::serial_println!("TCP: port {} already listening (slot {} owner_tid={})",
                    port, i, tcb.owner_tid);
                return u32::MAX;
            }
        }
    }

    // Find a free slot
    let mut found = None;
    for (i, slot) in table.iter_mut().enumerate() {
        if slot.is_none() {
            let mut tcb = Tcb::new(cfg.ip, port, Ipv4Addr([0, 0, 0, 0]), 0);
            tcb.state = TcpState::Listen;
            tcb.owner_tid = tid;
            *slot = Some(tcb);
            found = Some(i);
            break;
        }
    }

    match found {
        Some(id) => {
            crate::serial_println!("TCP: listening on port {} (socket {})", port, id);
            id as u32
        }
        None => {
            crate::serial_println!("TCP: no free slots for listen");
            u32::MAX
        }
    }
}

/// Accept a connection from a listening socket. Blocks until a connection is
/// established or timeout. Returns (socket_id, remote_ip, remote_port) packed
/// into the result buffer, or u32::MAX on error.
///
/// result_ptr: user buffer of 12 bytes: [socket_id:u32, remote_ip:[u8;4], remote_port:u16, pad:u16]
/// Accept a connection on a listening socket.
/// Blocks the calling thread until a connection is established or timeout.
/// Zero CPU usage while waiting.
pub fn accept(listener_id: u32, timeout_ticks: u32) -> (u32, Ipv4Addr, u16) {
    let lid = listener_id as usize;
    if lid >= MAX_CONNECTIONS {
        return (u32::MAX, Ipv4Addr([0; 4]), 0);
    }

    let tid = crate::task::scheduler::current_tid();
    let start = crate::arch::x86::pit::get_ticks();

    // Poll once eagerly to process any pending packets before blocking.
    super::poll();

    loop {
        {
            let mut conns = TCP_CONNECTIONS.lock();
            let table = match conns.as_mut() {
                Some(t) => t,
                None => return (u32::MAX, Ipv4Addr([0; 4]), 0),
            };

            // Verify listener is still valid
            let listen_valid = table[lid].as_ref()
                .map(|t| t.state == TcpState::Listen)
                .unwrap_or(false);
            if !listen_valid {
                if let Some(tcb) = table[lid].as_mut() {
                    tcb.waiting_tid = 0;
                }
                return (u32::MAX, Ipv4Addr([0; 4]), 0);
            }

            // Find a ready connection spawned by this listener
            for i in 0..table.len() {
                let ready = table[i].as_ref().map(|tcb| {
                    tcb.parent_listener == Some(lid as u8)
                        && tcb.state == TcpState::Established
                        && !tcb.accepted
                }).unwrap_or(false);

                if ready {
                    let tcb = table[i].as_mut().unwrap();
                    tcb.accepted = true;
                    tcb.parent_listener = None;
                    tcb.owner_tid = crate::task::scheduler::current_tid();
                    let rip = tcb.remote_ip;
                    let rport = tcb.remote_port;
                    // Clear waiting_tid on the listener
                    if let Some(listener) = table[lid].as_mut() {
                        listener.waiting_tid = 0;
                    }
                    crate::serial_println!("TCP: accepted socket {} from {}:{}", i, rip, rport);
                    TCP_PASSIVE_OPENS.fetch_add(1, Ordering::Relaxed);
                    return (i as u32, rip, rport);
                }
            }

            // Check timeout before blocking
            let now = crate::arch::x86::pit::get_ticks();
            if now.wrapping_sub(start) >= timeout_ticks {
                if let Some(tcb) = table[lid].as_mut() {
                    tcb.waiting_tid = 0;
                }
                return (u32::MAX, Ipv4Addr([0; 4]), 0);
            }

            // Register ourselves as the waiting thread on the listener socket
            if let Some(tcb) = table[lid].as_mut() {
                tcb.waiting_tid = tid;
            }
        }
        // Lock is dropped here — safe to block.

        // Sleep briefly (2 ticks = 20ms). Woken early by try_wake_thread()
        // when a connection completes the handshake, or by timer for timeout.
        let wake_at = crate::arch::x86::pit::get_ticks() + 2;
        crate::task::scheduler::sleep_until(wake_at);

        // After waking, poll network to process any pending packets.
        super::poll();
    }
}

/// Close a listening socket. Also cleans up any pending (unaccepted) connections.
pub fn close_listener(socket_id: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }

    let mut conns = TCP_CONNECTIONS.lock();
    let table = match conns.as_mut() {
        Some(t) => t,
        None => return u32::MAX,
    };

    // Verify it's a listener
    let is_listener = table[id].as_ref()
        .map(|t| t.state == TcpState::Listen)
        .unwrap_or(false);

    if !is_listener {
        // Not a listener — fall through to normal close
        return u32::MAX;
    }

    // Clean up pending connections spawned by this listener
    for i in 0..table.len() {
        let is_pending = table[i].as_ref().map(|tcb| {
            tcb.parent_listener == Some(id as u8)
                && !tcb.accepted
        }).unwrap_or(false);
        if is_pending {
            table[i] = None;
        }
    }

    // Remove the listener itself
    table[id] = None;
    crate::serial_println!("TCP: listener socket {} closed", id);
    0
}

/// Metadata for a segment prepared under lock, sent after lock is released.
struct BatchSegment {
    local_ip: Ipv4Addr,
    local_port: u16,
    remote_ip: Ipv4Addr,
    remote_port: u16,
    seq: u32,
    ack_num: u32,
    data_start: usize,  // offset into the caller's `data` slice
    data_end: usize,     // exclusive end offset
}

/// Send data on an established connection. Returns bytes sent or u32::MAX on error.
///
/// Uses sliding window with batched locking: acquires TCP_CONNECTIONS once per
/// batch of up to SEND_BATCH_SIZE segments, prepares all segment metadata under
/// the lock, then sends them all after releasing the lock.
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
    let mut send_offset = 0usize; // next byte index to segment (relative to data)

    // Stack-allocated batch buffer — avoids heap alloc per segment.
    let mut batch: [core::mem::MaybeUninit<BatchSegment>; SEND_BATCH_SIZE] =
        unsafe { core::mem::MaybeUninit::uninit().assume_init() };

    loop {
        // ── Single lock acquisition: compute ack_offset, prepare batch of segments ──
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

            let acked_bytes = tcb.snd_una.wrapping_sub(send_base_seq) as usize;
            let ack_offset = acked_bytes.min(data.len());

            // All data acknowledged?
            if ack_offset >= data.len() {
                return data.len() as u32;
            }

            let snd_wnd = (tcb.snd_wnd as usize).max(MSS);
            let window = snd_wnd.min(MAX_IN_FLIGHT);
            let now = crate::arch::x86::pit::get_ticks();

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
                    data_start: send_offset,
                    data_end: chunk_end,
                };

                // Update TCB for this segment.
                let chunk_len = chunk_end - send_offset;
                tcb.snd_nxt = tcb.snd_nxt.wrapping_add(chunk_len as u32);

                batch[count].write(seg);
                count += 1;
                send_offset = chunk_end;
            }

            // Track the last segment for retransmission.
            if count > 0 {
                let last = unsafe { batch[count - 1].assume_init_ref() };
                let last_chunk = &data[last.data_start..last.data_end];
                tcb.last_sent_data.clear();
                tcb.last_sent_data.extend_from_slice(last_chunk);
                tcb.last_sent_seq = last.seq;
                tcb.last_sent_flags = PSH | ACK;
                tcb.last_send_tick = now;
                tcb.retransmit_count = 0;
            }

            (ack_offset, count)
        }; // lock released here

        // ── Send all batched segments outside the lock ──
        for i in 0..batch_count {
            let seg = unsafe { batch[i].assume_init_ref() };
            send_segment(seg.local_ip, seg.local_port, seg.remote_ip, seg.remote_port,
                         seg.seq, seg.ack_num, PSH | ACK, &data[seg.data_start..seg.data_end]);
        }

        // All data acknowledged?
        if ack_offset >= data.len() {
            return data.len() as u32;
        }

        // Poll network for incoming ACKs.
        super::poll();

        // Check timeout.
        let now = crate::arch::x86::pit::get_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            crate::serial_println!("TCP: send timeout on socket {}", socket_id);
            return if ack_offset > 0 { ack_offset as u32 } else { u32::MAX };
        }

        // If no segments were sent (window full), sleep briefly to let ACKs arrive.
        if batch_count == 0 {
            let wake_at = crate::arch::x86::pit::get_ticks() + 1;
            crate::task::scheduler::sleep_until(wake_at);
            super::poll();
        }
    }
}

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
    let start = crate::arch::x86::pit::get_ticks();

    // Poll once eagerly to process any pending packets before blocking.
    super::poll();

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
                let n = tcb.recv_buf.len().min(buf.len());
                let (front, back) = tcb.recv_buf.as_slices();
                let front_n = front.len().min(n);
                buf[..front_n].copy_from_slice(&front[..front_n]);
                if front_n < n {
                    buf[front_n..n].copy_from_slice(&back[..n - front_n]);
                }
                tcb.recv_buf.drain(..n);
                tcb.waiting_tid = 0;
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
            let now = crate::arch::x86::pit::get_ticks();
            if now.wrapping_sub(start) >= timeout_ticks {
                tcb.waiting_tid = 0;
                return u32::MAX;
            }

            // Register ourselves as the waiting thread and block
            tcb.waiting_tid = tid;
        }
        // Lock is dropped here — safe to block.

        // Sleep briefly (2 ticks = 20ms). The thread will be woken early by
        // try_wake_thread() when data arrives (from E1000 IRQ → net::poll →
        // handle_tcp), or by the timer when the sleep expires (for timeout checks).
        let wake_at = crate::arch::x86::pit::get_ticks() + 2;
        crate::task::scheduler::sleep_until(wake_at);

        // After waking, poll network to process any pending packets.
        super::poll();
    }
}

/// Close a TCP connection. Sends FIN, waits for ACK.
/// For listener sockets, closes all pending connections too.
pub fn close(socket_id: u32) -> u32 {
    let id = socket_id as usize;
    if id >= MAX_CONNECTIONS {
        return u32::MAX;
    }

    // Check if it's a listener — if so, use close_listener
    {
        let conns = TCP_CONNECTIONS.lock();
        if let Some(table) = conns.as_ref() {
            if let Some(tcb) = &table[id] {
                if tcb.state == TcpState::Listen {
                    drop(conns);
                    return close_listener(socket_id);
                }
            }
        }
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

        let wake_at = crate::arch::x86::pit::get_ticks() + 5;
        crate::task::scheduler::sleep_until(wake_at);
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
    TCP_SEGMENTS_RECV.fetch_add(1, Ordering::Relaxed);

    // Process segment under lock, collect deferred sends and wake TIDs.
    // wake_tid: thread blocked on this connection (recv/connect).
    // wake_listener_tid: thread blocked on the parent listener (accept).
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
                    // Incoming SYN — look for a listener
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
                                t.parent_listener == Some(lid as u8)
                                    && !t.accepted
                            }).unwrap_or(false)
                        }).count();

                        if pending_count >= MAX_BACKLOG {
                            // Backlog full — silently drop SYN (don't RST)
                            return;
                        }

                        let cfg = super::config();
                        // Allocate a new slot for SynReceived
                        let mut new_slot = None;
                        for (i, slot) in table.iter_mut().enumerate() {
                            if slot.is_none() {
                                let mut tcb = Tcb::new(cfg.ip, seg.dst_port, seg.src_ip, seg.src_port);
                                tcb.state = TcpState::SynReceived;
                                tcb.rcv_irs = seg.seq;
                                tcb.rcv_nxt = seg.seq.wrapping_add(1);
                                tcb.snd_nxt = tcb.snd_iss.wrapping_add(1);
                                tcb.parent_listener = Some(lid as u8);
                                tcb.last_sent_flags = SYN | ACK;
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
                            // Send SYN-ACK with options if peer supports window scaling
                            if use_wscale {
                                send_syn_segment(lip, lp, rip, rp, iss, rcv_nxt, SYN | ACK);
                            } else {
                                send_segment(lip, lp, rip, rp, iss, rcv_nxt, SYN | ACK, &[]);
                            }
                            return;
                        }
                        // No free slots
                        return;
                    }
                }
                // No matching connection and no listener — send RST after dropping lock
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

        let tcb = table[idx].as_mut().unwrap();
        let now = crate::arch::x86::pit::get_ticks();

        let match_result = match tcb.state {
            TcpState::SynSent => {
                if seg.flags & SYN != 0 && seg.flags & ACK != 0 {
                    if seg.ack == tcb.snd_nxt {
                        tcb.rcv_irs = seg.seq;
                        tcb.rcv_nxt = seg.seq.wrapping_add(1);
                        tcb.snd_una = seg.ack;
                        // SYN-ACK window is NOT scaled (RFC 7323)
                        tcb.snd_wnd = seg.window as u32;
                        // If peer included window scale, enable scaling
                        if let Some(shift) = seg.wscale {
                            tcb.snd_wnd_shift = shift;
                            tcb.rcv_wnd_shift = OUR_WINDOW_SHIFT;
                        }
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

            TcpState::SynReceived => {
                // Waiting for ACK to complete the 3-way handshake
                if seg.flags & ACK != 0 {
                    if seg.ack == tcb.snd_nxt {
                        tcb.snd_una = seg.ack;
                        // ACK completing handshake: window IS scaled (RFC 7323)
                        tcb.snd_wnd = (seg.window as u32) << tcb.snd_wnd_shift;
                        tcb.state = TcpState::Established;
                        tcb.last_sent_data.clear();
                        tcb.retransmit_count = 0;
                        crate::serial_println!("TCP: SynReceived -> Established on socket {}", idx);
                        // Wake the accept() thread on the parent listener
                        if let Some(lid) = tcb.parent_listener {
                            if let Some(listener) = table[lid as usize].as_mut() {
                                wake_listener_tid = listener.waiting_tid;
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
                    })
                } else {
                    None
                }
            }

            TcpState::Listen => None, // Handled earlier in listener lookup

            TcpState::Closed => None,
        };

        // Collect waiting_tid from the connection — wake after lock drop.
        // Any event (data, state change, FIN) should wake the blocked thread
        // so it can re-evaluate its condition.
        let tcb = table[idx].as_mut().unwrap();
        if tcb.waiting_tid != 0 {
            wake_tid = tcb.waiting_tid;
            tcb.waiting_tid = 0;
        }

        match_result
    }; // lock dropped here

    // Wake blocked threads outside lock.
    // Use try_wake_thread (non-blocking, IRQ-safe) since handle_tcp may be
    // called from the E1000 IRQ handler via net::poll().
    if wake_tid != 0 {
        crate::task::scheduler::try_wake_thread(wake_tid);
    }
    if wake_listener_tid != 0 {
        crate::task::scheduler::try_wake_thread(wake_listener_tid);
    }

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
            tcb.snd_wnd = (seg.window as u32) << tcb.snd_wnd_shift;
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

/// Accept in-order data with delayed ACK support.
///
/// Instead of ACKing every segment immediately, we delay the ACK and only send
/// it after DELAYED_ACK_SEGMENTS data segments have been accepted, or when the
/// recv buffer is >75% full. This halves the number of ACK packets on the wire.
///
/// Pending delayed ACKs are flushed by `flush_delayed_acks()` on a timer.
fn accept_data_deferred(tcb: &mut Tcb, seg: &TcpSegment) -> Option<DeferredSend> {
    if seg.payload.is_empty() {
        return None;
    }

    if seg.seq == tcb.rcv_nxt {
        let space = RECV_BUF_SIZE - tcb.recv_buf.len();
        let take = seg.payload.len().min(space);
        tcb.recv_buf.extend(&seg.payload[..take]);
        tcb.rcv_nxt = tcb.rcv_nxt.wrapping_add(take as u32);
        tcb.ack_seg_count += 1;
        tcb.pending_ack = true;

        // Decide whether to ACK now or delay.
        let buf_pressure = tcb.recv_buf.len() > (RECV_BUF_SIZE * 3 / 4);
        let batch_full = tcb.ack_seg_count >= DELAYED_ACK_SEGMENTS;

        if batch_full || buf_pressure {
            // Send ACK immediately.
            tcb.pending_ack = false;
            tcb.ack_seg_count = 0;
            tcb.last_ack_tick = crate::arch::x86::pit::get_ticks();
            Some(DeferredSend {
                local_ip: tcb.local_ip, local_port: tcb.local_port,
                remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
                seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
            })
        } else {
            // Delay ACK — will be flushed by timer or next segment.
            None
        }
    } else if is_seq_gt(tcb.rcv_nxt, seg.seq) {
        // Duplicate — send duplicate ACK immediately (fast retransmit signal).
        Some(DeferredSend {
            local_ip: tcb.local_ip, local_port: tcb.local_port,
            remote_ip: tcb.remote_ip, remote_port: tcb.remote_port,
            seq: tcb.snd_nxt, ack_num: tcb.rcv_nxt, flags: ACK,
        })
    } else {
        None
    }
}

/// Check retransmissions, flush delayed ACKs, and perform TIME_WAIT cleanup.
/// Called from net::poll().
pub fn check_retransmissions() {
    let now = crate::arch::x86::pit::get_ticks();

    // ── Pass 1: flush any delayed ACKs that have been pending too long ──
    // Collect up to 8 pending ACKs under lock, send outside.
    let mut delayed_acks: [(Ipv4Addr, u16, Ipv4Addr, u16, u32, u32); 8] =
        [(Ipv4Addr([0;4]), 0, Ipv4Addr([0;4]), 0, 0, 0); 8];
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
                    delayed_acks[delayed_ack_count] =
                        (tcb.local_ip, tcb.local_port, tcb.remote_ip, tcb.remote_port,
                         tcb.snd_nxt, tcb.rcv_nxt);
                    delayed_ack_count += 1;
                }
            }
        }
    }

    // Send delayed ACKs outside the lock.
    for i in 0..delayed_ack_count {
        let (lip, lp, rip, rp, seq, ack_num) = delayed_acks[i];
        send_segment(lip, lp, rip, rp, seq, ack_num, ACK, &[]);
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

            // SynReceived cleanup: if max retransmits exceeded, drop connection
            if tcb.state == TcpState::SynReceived
                && tcb.retransmit_count >= MAX_RETRANSMITS
            {
                table[i] = None;
                continue;
            }

            // Retransmission check (data retransmit for Established/SynSent)
            should_retransmit = !tcb.last_sent_data.is_empty()
                && now.wrapping_sub(tcb.last_send_tick) >= RETRANSMIT_TICKS
                && tcb.retransmit_count < MAX_RETRANSMITS
                && (tcb.state == TcpState::Established || tcb.state == TcpState::SynSent);

            // SynReceived SYN-ACK retransmit
            should_syn_retransmit = tcb.state == TcpState::SynReceived
                && now.wrapping_sub(tcb.last_send_tick) >= RETRANSMIT_TICKS
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
            let iss = tcb.snd_iss;
            let rcv_nxt = tcb.rcv_nxt;
            let use_wscale = tcb.rcv_wnd_shift > 0;

            drop(conns);
            if use_wscale {
                send_syn_segment(lip, lp, rip, rp, iss, rcv_nxt, SYN | ACK);
            } else {
                send_segment(lip, lp, rip, rp, iss, rcv_nxt, SYN | ACK, &[]);
            }
            return;
        }

        if should_retransmit {
            let tcb = table[i].as_mut().unwrap();
            tcb.retransmit_count += 1;
            TCP_RETRANSMITS.fetch_add(1, Ordering::Relaxed);
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

/// Clean up all TCP connections owned by a specific thread.
/// Called from sys_exit() when a process terminates.
/// Sends RST for established connections and frees listener slots + pending connections.
pub fn cleanup_for_thread(tid: u32) {
    // Collect RST info under lock, send RSTs outside lock
    let mut rst_list: [(Ipv4Addr, u16, Ipv4Addr, u16, u32, u32); 16] =
        [(Ipv4Addr([0;4]), 0, Ipv4Addr([0;4]), 0, 0, 0); 16];
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
                // Clean up pending connections spawned by this listener
                let lid = i as u8;
                for j in 0..table.len() {
                    let is_pending = table[j].as_ref().map(|tcb| {
                        tcb.parent_listener == Some(lid) && !tcb.accepted
                    }).unwrap_or(false);
                    if is_pending {
                        // Collect RST info for SynReceived/Established pending connections
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
            send_segment(lip, lp, rip, rp, seq, ack, RST | ACK, &[]);
        }
    }
}

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
/// Returns a Vec of connection info structs.
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
