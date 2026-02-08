/// UDP (User Datagram Protocol) — connectionless transport.

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use super::types::Ipv4Addr;
use super::ipv4::Ipv4Packet;
use crate::sync::spinlock::Spinlock;

const UDP_HEADER_LEN: usize = 8;

pub struct UdpDatagram {
    pub src_ip: Ipv4Addr,
    pub src_port: u16,
    pub data: Vec<u8>,
}

/// Port bindings: port -> queue of received datagrams
static UDP_PORTS: Spinlock<Option<BTreeMap<u16, VecDeque<UdpDatagram>>>> = Spinlock::new(None);

pub fn init() {
    let mut ports = UDP_PORTS.lock();
    *ports = Some(BTreeMap::new());
}

/// Bind to a UDP port (creates a receive queue)
pub fn bind(port: u16) {
    let mut ports = UDP_PORTS.lock();
    if let Some(map) = ports.as_mut() {
        map.entry(port).or_insert_with(VecDeque::new);
    }
}

/// Unbind a UDP port
pub fn unbind(port: u16) {
    let mut ports = UDP_PORTS.lock();
    if let Some(map) = ports.as_mut() {
        map.remove(&port);
    }
}

/// Send a UDP datagram
pub fn send(dst_ip: Ipv4Addr, src_port: u16, dst_port: u16, data: &[u8]) -> bool {
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
        if let Some(queue) = map.get_mut(&port) {
            return queue.pop_front();
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
        if let Some(queue) = map.get_mut(&dst_port) {
            if queue.len() < 128 {
                queue.push_back(UdpDatagram {
                    src_ip: pkt.src,
                    src_port,
                    data: Vec::from(payload),
                });
            }
        }
    }
}
