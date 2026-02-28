//! Transmission Control Block (TCB) and related types.
//!
//! Contains the per-connection state, TCP segment representation,
//! out-of-order buffer entry, and protocol constants.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::net::types::Ipv4Addr;

// ── TCP header flags ─────────────────────────────────────────────────
pub(crate) const FIN: u8 = 0x01;
pub(crate) const SYN: u8 = 0x02;
pub(crate) const RST: u8 = 0x04;
pub(crate) const PSH: u8 = 0x08;
pub(crate) const ACK: u8 = 0x10;

// ── Protocol constants ──────────────────────────────────────────────
pub(crate) const TCP_HEADER_LEN: usize = 20;
pub(crate) const MAX_CONNECTIONS: usize = 64;
/// Receive buffer size per connection (256 KB).
pub(crate) const RECV_BUF_SIZE: usize = 262144;
/// Maximum segment size (standard Ethernet MTU minus IP+TCP headers).
pub(crate) const MSS: usize = 1460;
/// Base retransmission timeout in ticks (3 seconds at 100 Hz).
pub(crate) const RETRANSMIT_TICKS: u32 = 300;
/// Maximum retransmission attempts before giving up.
pub(crate) const MAX_RETRANSMITS: u32 = 5;
/// TIME_WAIT duration in ticks (2 seconds at 100 Hz).
pub(crate) const TIME_WAIT_TICKS: u32 = 200;
/// Maximum pending connections per listener.
pub(crate) const MAX_BACKLOG: usize = 16;
/// Maximum bytes in flight (sliding window send limit, 1 MB).
pub(crate) const MAX_IN_FLIGHT: usize = 1_048_576;
/// Our TCP Window Scale shift count (RFC 7323).
/// WINDOW_SIZE << 4 = ~1 MB effective receive window.
pub(crate) const OUR_WINDOW_SHIFT: u8 = 4;
/// Maximum segments to batch per lock acquisition in send().
pub(crate) const SEND_BATCH_SIZE: usize = 64;
/// Delayed ACK: flush after this many accepted data segments.
pub(crate) const DELAYED_ACK_SEGMENTS: u32 = 2;
/// Delayed ACK: flush after this many ticks (20 ms at 100 Hz).
pub(crate) const DELAYED_ACK_TICKS: u32 = 2;
/// Maximum out-of-order segments buffered per connection.
pub(crate) const MAX_OOO_SEGMENTS: usize = 32;
/// Maximum send buffer size (matches MAX_IN_FLIGHT).
pub(crate) const MAX_SEND_BUF: usize = MAX_IN_FLIGHT;

// ── TCP connection state machine (RFC 793) ──────────────────────────

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

// ── Out-of-order segment ────────────────────────────────────────────

/// A buffered out-of-order TCP segment awaiting reassembly.
pub(crate) struct OooSegment {
    pub seq: u32,
    pub data: Vec<u8>,
}

// ── Parsed TCP segment ──────────────────────────────────────────────

/// A parsed incoming TCP segment with header fields and payload.
pub(crate) struct TcpSegment {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    pub payload: Vec<u8>,
    pub src_ip: Ipv4Addr,
    /// TCP Window Scale option (Kind=3), present only in SYN segments.
    pub wscale: Option<u8>,
    /// MSS option (Kind=2), present only in SYN segments.
    pub peer_mss: Option<u16>,
}

// ── Deferred send info ──────────────────────────────────────────────

/// Segment metadata collected while holding the lock, sent after releasing it.
pub(crate) struct DeferredSend {
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,
    pub seq: u32,
    pub ack_num: u32,
    pub flags: u8,
    pub window: u16,
}

/// Metadata for a data segment prepared under lock (used by send batching).
pub(crate) struct BatchSegment {
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,
    pub seq: u32,
    pub ack_num: u32,
    pub window: u16,
    pub data_start: usize,
    pub data_end: usize,
}

// ── Transmission Control Block ──────────────────────────────────────

/// Per-connection TCP state (Transmission Control Block).
pub(crate) struct Tcb {
    pub state: TcpState,
    pub local_ip: Ipv4Addr,
    pub local_port: u16,
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,

    // ── Send sequence variables ──
    pub snd_iss: u32,     // initial send sequence number
    pub snd_una: u32,     // oldest unacknowledged
    pub snd_nxt: u32,     // next to send
    pub snd_wnd: u32,     // send window (scaled by snd_wnd_shift)

    // ── Receive sequence variables ──
    pub rcv_irs: u32,     // initial receive sequence number
    pub rcv_nxt: u32,     // next expected

    // ── TCP Window Scaling (RFC 7323) ──
    pub snd_wnd_shift: u8,   // peer's scale factor
    pub rcv_wnd_shift: u8,   // our scale factor

    // ── Receive buffer ──
    pub recv_buf: VecDeque<u8>,

    // ── Out-of-order reassembly buffer ──
    pub ooo_buf: Vec<OooSegment>,

    // ── Send buffer (all unACKed data from snd_una to snd_nxt) ──
    pub send_buf: VecDeque<u8>,

    // ── Retransmission state ──
    pub retransmit_count: u32,
    pub last_send_tick: u32,

    // ── Fast retransmit ──
    pub dup_ack_count: u32,

    // ── State flags ──
    pub fin_received: bool,
    pub reset_received: bool,

    // ── Delayed ACK support ──
    pub pending_ack: bool,
    pub ack_seg_count: u32,
    pub last_ack_tick: u32,

    // ── TIME_WAIT timer ──
    pub time_wait_start: u32,

    // ── Server socket support ──
    pub parent_listener: Option<u8>,
    pub accepted: bool,

    // ── Ownership tracking ──
    pub owner_tid: u32,

    // ── Blocking I/O ──
    pub waiting_tid: u32,
}

impl Tcb {
    /// Create a new TCB with initial sequence number derived from PIT ticks.
    pub fn new(local_ip: Ipv4Addr, local_port: u16, remote_ip: Ipv4Addr, remote_port: u16) -> Self {
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
            ooo_buf: Vec::new(),
            send_buf: VecDeque::new(),
            retransmit_count: 0,
            last_send_tick: 0,
            dup_ack_count: 0,
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

    /// Compute the receive window value to advertise (scaled down by rcv_wnd_shift).
    pub fn advertised_window(&self) -> u16 {
        let available = RECV_BUF_SIZE.saturating_sub(self.recv_buf.len());
        let scaled = available >> self.rcv_wnd_shift;
        scaled.min(65535) as u16
    }
}

// ── Parsing ─────────────────────────────────────────────────────────

/// Parse a TCP segment from an IPv4 packet payload.
pub(crate) fn parse_tcp(pkt: &crate::net::ipv4::Ipv4Packet<'_>) -> Option<TcpSegment> {
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
