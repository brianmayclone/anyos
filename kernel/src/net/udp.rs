//! UDP (User Datagram Protocol) -- connectionless datagram transport.
//! Supports port binding, non-blocking and blocking receive, raw sends for DHCP,
//! per-port options (broadcast, receive timeout), and multicast/broadcast destinations.

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use super::types::Ipv4Addr;
use super::ipv4::Ipv4Packet;
use crate::sync::spinlock::Spinlock;

const UDP_HEADER_LEN: usize = 8;
const MAX_QUEUE_LEN: usize = 128;

// Socket option constants (match stdlib)
pub const SO_BROADCAST: u32 = 1;
pub const SO_RCVTIMEO: u32 = 2;

/// A received UDP datagram with source address/port and payload.
pub struct UdpDatagram {
    pub src_ip: Ipv4Addr,
    pub src_port: u16,
    pub data: Vec<u8>,
}

/// Per-port configuration and receive queue.
struct PortConfig {
    queue: VecDeque<UdpDatagram>,
    broadcast: bool,
    timeout_ms: u32,
}

impl PortConfig {
    fn new() -> Self {
        PortConfig {
            queue: VecDeque::new(),
            broadcast: false,
            timeout_ms: 0,
        }
    }
}

/// Port bindings: port -> config + queue of received datagrams
static UDP_PORTS: Spinlock<Option<BTreeMap<u16, PortConfig>>> = Spinlock::new(None);

/// Initialize the UDP subsystem. Must be called before binding ports.
pub fn init() {
    let mut ports = UDP_PORTS.lock();
    *ports = Some(BTreeMap::new());
}

/// Bind to a UDP port (creates a receive queue). Returns true if newly bound.
pub fn bind(port: u16) -> bool {
    let mut ports = UDP_PORTS.lock();
    if let Some(map) = ports.as_mut() {
        if map.contains_key(&port) {
            return false; // already bound
        }
        map.insert(port, PortConfig::new());
        true
    } else {
        false
    }
}

/// Unbind a UDP port
pub fn unbind(port: u16) {
    let mut ports = UDP_PORTS.lock();
    if let Some(map) = ports.as_mut() {
        map.remove(&port);
    }
}

/// Set a per-port option. Returns true on success.
pub fn set_opt(port: u16, opt: u32, val: u32) -> bool {
    let mut ports = UDP_PORTS.lock();
    if let Some(map) = ports.as_mut() {
        if let Some(cfg) = map.get_mut(&port) {
            match opt {
                SO_BROADCAST => { cfg.broadcast = val != 0; true }
                SO_RCVTIMEO => { cfg.timeout_ms = val; true }
                _ => false,
            }
        } else {
            false
        }
    } else {
        false
    }
}

/// Get the receive timeout for a bound port (ms). Returns 0 if not bound or non-blocking.
pub fn get_timeout_ms(port: u16) -> u32 {
    let ports = UDP_PORTS.lock();
    if let Some(map) = ports.as_ref() {
        if let Some(cfg) = map.get(&port) {
            return cfg.timeout_ms;
        }
    }
    0
}

/// Check if broadcast is enabled on a port.
pub fn is_broadcast_enabled(port: u16) -> bool {
    let ports = UDP_PORTS.lock();
    if let Some(map) = ports.as_ref() {
        if let Some(cfg) = map.get(&port) {
            return cfg.broadcast;
        }
    }
    false
}

/// Send a UDP datagram. For broadcast destinations, the source port must have
/// SO_BROADCAST enabled (or `force_broadcast` must be true for internal callers).
pub fn send(dst_ip: Ipv4Addr, src_port: u16, dst_port: u16, data: &[u8]) -> bool {
    // Check broadcast permission
    if dst_ip == Ipv4Addr::BROADCAST || dst_ip.is_broadcast_for(super::config().mask) {
        if !is_broadcast_enabled(src_port) {
            return false;
        }
    }
    send_unchecked(dst_ip, src_port, dst_port, data)
}

/// Internal send without broadcast permission check (for kernel-internal callers like DHCP).
pub fn send_unchecked(dst_ip: Ipv4Addr, src_port: u16, dst_port: u16, data: &[u8]) -> bool {
    let total_len = UDP_HEADER_LEN + data.len();
    let mut udp = Vec::with_capacity(total_len);

    udp.push((src_port >> 8) as u8);
    udp.push((src_port & 0xFF) as u8);
    udp.push((dst_port >> 8) as u8);
    udp.push((dst_port & 0xFF) as u8);
    udp.push((total_len >> 8) as u8);
    udp.push((total_len & 0xFF) as u8);
    udp.push(0); udp.push(0); // Checksum (0 = disabled for now)
    udp.extend_from_slice(data);

    super::ipv4::send_ipv4(dst_ip, super::ipv4::PROTO_UDP, &udp)
}

/// Send a UDP datagram with raw IP (for DHCP before config)
pub fn send_raw(src_ip: Ipv4Addr, dst_ip: Ipv4Addr, dst_mac: super::types::MacAddr,
                src_port: u16, dst_port: u16, data: &[u8]) -> bool {
    let total_len = UDP_HEADER_LEN + data.len();
    let mut udp = Vec::with_capacity(total_len);

    udp.push((src_port >> 8) as u8);
    udp.push((src_port & 0xFF) as u8);
    udp.push((dst_port >> 8) as u8);
    udp.push((dst_port & 0xFF) as u8);
    udp.push((total_len >> 8) as u8);
    udp.push((total_len & 0xFF) as u8);
    udp.push(0); udp.push(0);
    udp.extend_from_slice(data);

    super::ipv4::send_ipv4_raw(src_ip, dst_ip, dst_mac, super::ipv4::PROTO_UDP, &udp)
}

/// Receive a UDP datagram on a bound port (non-blocking)
pub fn recv(port: u16) -> Option<UdpDatagram> {
    let mut ports = UDP_PORTS.lock();
    if let Some(map) = ports.as_mut() {
        if let Some(cfg) = map.get_mut(&port) {
            return cfg.queue.pop_front();
        }
    }
    None
}

/// Receive a UDP datagram with timeout (blocking with polling)
pub fn recv_timeout(port: u16, timeout_ticks: u32) -> Option<UdpDatagram> {
    let start = crate::arch::x86::pit::get_ticks();
    loop {
        super::poll();

        if let Some(dgram) = recv(port) {
            return Some(dgram);
        }

        let now = crate::arch::x86::pit::get_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            return None;
        }

        core::hint::spin_loop();
    }
}

/// Handle an incoming UDP packet
pub fn handle_udp(pkt: &Ipv4Packet<'_>) {
    let data = pkt.payload;
    if data.len() < UDP_HEADER_LEN { return; }

    let src_port = ((data[0] as u16) << 8) | data[1] as u16;
    let dst_port = ((data[2] as u16) << 8) | data[3] as u16;
    let length = ((data[4] as u16) << 8) | data[5] as u16;

    if (length as usize) > data.len() { return; }

    let payload = &data[UDP_HEADER_LEN..(length as usize)];

    let mut ports = UDP_PORTS.lock();
    if let Some(map) = ports.as_mut() {
        if let Some(cfg) = map.get_mut(&dst_port) {
            if cfg.queue.len() < MAX_QUEUE_LEN {
                cfg.queue.push_back(UdpDatagram {
                    src_ip: pkt.src,
                    src_port,
                    data: Vec::from(payload),
                });
            }
        }
    }
}
