/// IPv4 packet handling: build and parse IPv4 headers, route outgoing packets.

use alloc::vec::Vec;
use super::types::{Ipv4Addr, MacAddr};
use super::checksum;
use super::ethernet;

const IPV4_HEADER_LEN: usize = 20;
pub const PROTO_ICMP: u8 = 1;
pub const PROTO_UDP: u8 = 17;
pub const PROTO_TCP: u8 = 6;

pub struct Ipv4Packet<'a> {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub protocol: u8,
    pub ttl: u8,
    pub payload: &'a [u8],
    pub total_len: u16,
    pub header_len: usize,
}

/// Parse an IPv4 packet
pub fn parse(data: &[u8]) -> Option<Ipv4Packet<'_>> {
    if data.len() < IPV4_HEADER_LEN { return None; }

    let version = data[0] >> 4;
    if version != 4 { return None; }

    let ihl = (data[0] & 0x0F) as usize;
    let header_len = ihl * 4;
    if data.len() < header_len { return None; }

    let total_len = ((data[2] as u16) << 8) | data[3] as u16;
    if (total_len as usize) > data.len() { return None; }

    let ttl = data[8];
    let protocol = data[9];
    let src = Ipv4Addr([data[12], data[13], data[14], data[15]]);
    let dst = Ipv4Addr([data[16], data[17], data[18], data[19]]);

    let payload = &data[header_len..(total_len as usize)];

    Some(Ipv4Packet { src, dst, protocol, ttl, payload, total_len, header_len })
}

/// Build and send an IPv4 packet
pub fn send_ipv4(dst: Ipv4Addr, protocol: u8, payload: &[u8]) -> bool {
    let cfg = super::config();
    let total_len = IPV4_HEADER_LEN + payload.len();
    if total_len > 1500 { return false; }

    // Build IPv4 header
    let mut header = [0u8; IPV4_HEADER_LEN];
    header[0] = 0x45; // Version 4, IHL 5
    header[1] = 0;    // DSCP/ECN
    header[2] = (total_len >> 8) as u8;
    header[3] = (total_len & 0xFF) as u8;
    // Identification (static counter)
    static mut IP_ID: u16 = 0;
    let id = unsafe { IP_ID += 1; IP_ID };
    header[4] = (id >> 8) as u8;
    header[5] = (id & 0xFF) as u8;
    header[6] = 0x40; // Don't Fragment flag
    header[7] = 0;
    header[8] = 64;   // TTL
    header[9] = protocol;
    // Checksum = 0 initially
    header[10] = 0;
    header[11] = 0;
    // Source IP
    header[12..16].copy_from_slice(&cfg.ip.0);
    // Destination IP
    header[16..20].copy_from_slice(&dst.0);

    // Compute header checksum
    let cksum = checksum::internet_checksum(&header);
    header[10] = (cksum >> 8) as u8;
    header[11] = (cksum & 0xFF) as u8;

    // Build full packet
    let mut packet = Vec::with_capacity(total_len);
    packet.extend_from_slice(&header);
    packet.extend_from_slice(payload);

    // Resolve destination MAC
    let next_hop = if cfg.is_local(dst) || dst == Ipv4Addr::BROADCAST {
        dst
    } else {
        cfg.gateway
    };

    let dst_mac = if dst == Ipv4Addr::BROADCAST {
        MacAddr::BROADCAST
    } else {
        match super::arp::resolve(next_hop, 200) { // 2 second timeout at 100Hz
            Some(mac) => mac,
            None => {
                crate::serial_println!("  IPv4: ARP resolve failed for {}", next_hop);
                return false;
            }
        }
    };

    ethernet::send_frame(dst_mac, ethernet::ETHERTYPE_IPV4, &packet);
    true
}

/// Build and send an IPv4 packet with a specific source IP (for DHCP before config)
pub fn send_ipv4_raw(src_ip: Ipv4Addr, dst_ip: Ipv4Addr, dst_mac: MacAddr, protocol: u8, payload: &[u8]) -> bool {
    let total_len = IPV4_HEADER_LEN + payload.len();
    if total_len > 1500 { return false; }

    let mut header = [0u8; IPV4_HEADER_LEN];
    header[0] = 0x45;
    header[2] = (total_len >> 8) as u8;
    header[3] = (total_len & 0xFF) as u8;
    static mut RAW_IP_ID: u16 = 0x1000;
    let id = unsafe { RAW_IP_ID += 1; RAW_IP_ID };
    header[4] = (id >> 8) as u8;
    header[5] = (id & 0xFF) as u8;
    header[6] = 0x40;
    header[8] = 64;
    header[9] = protocol;
    header[12..16].copy_from_slice(&src_ip.0);
    header[16..20].copy_from_slice(&dst_ip.0);

    let cksum = checksum::internet_checksum(&header);
    header[10] = (cksum >> 8) as u8;
    header[11] = (cksum & 0xFF) as u8;

    let mut packet = Vec::with_capacity(total_len);
    packet.extend_from_slice(&header);
    packet.extend_from_slice(payload);

    ethernet::send_frame(dst_mac, ethernet::ETHERTYPE_IPV4, &packet);
    true
}

/// Handle an incoming IPv4 packet
pub fn handle_ipv4(data: &[u8]) {
    let pkt = match parse(data) {
        Some(p) => p,
        None => return,
    };

    match pkt.protocol {
        PROTO_ICMP => super::icmp::handle_icmp(&pkt),
        PROTO_UDP => super::udp::handle_udp(&pkt),
        _ => {}
    }
}
