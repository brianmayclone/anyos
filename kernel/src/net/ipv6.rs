//! IPv6 packet handling: build and parse IPv6 headers, route outgoing packets.
//! Performs next-hop resolution via NDP before handing frames to the Ethernet layer.

use super::ethernet;
use super::types::{Ipv6Addr, MacAddr};
use alloc::vec::Vec;

/// IPv6 fixed header length (40 bytes).
const IPV6_HEADER_LEN: usize = 40;
/// ICMPv6 next-header value.
pub const PROTO_ICMPV6: u8 = 58;
/// TCP next-header value (same as IPv4 protocol number).
pub const PROTO_TCP: u8 = 6;
/// UDP next-header value (same as IPv4 protocol number).
pub const PROTO_UDP: u8 = 17;

/// A parsed IPv6 packet with a reference to its payload.
#[derive(Debug)]
pub struct Ipv6Packet<'a> {
    pub src: Ipv6Addr,
    pub dst: Ipv6Addr,
    pub next_header: u8,
    pub hop_limit: u8,
    pub payload: &'a [u8],
    pub payload_len: u16,
}

/// Parse an IPv6 packet from raw bytes.
pub fn parse(data: &[u8]) -> Option<Ipv6Packet<'_>> {
    if data.len() < IPV6_HEADER_LEN {
        return None;
    }

    let version = data[0] >> 4;
    if version != 6 {
        return None;
    }

    let payload_len = ((data[4] as u16) << 8) | data[5] as u16;
    let next_header = data[6];
    let hop_limit = data[7];

    let mut src = [0u8; 16];
    let mut dst = [0u8; 16];
    src.copy_from_slice(&data[8..24]);
    dst.copy_from_slice(&data[24..40]);

    let total_len = IPV6_HEADER_LEN + payload_len as usize;
    if data.len() < total_len {
        return None;
    }

    let payload = &data[IPV6_HEADER_LEN..total_len];

    Some(Ipv6Packet {
        src: Ipv6Addr(src),
        dst: Ipv6Addr(dst),
        next_header,
        hop_limit,
        payload,
        payload_len,
    })
}

/// Build and send an IPv6 packet.
pub fn send_ipv6(dst: Ipv6Addr, next_header: u8, payload: &[u8]) -> bool {
    let cfg = super::config();

    // Determine source address
    let src_ip = if dst.is_link_local() || dst.is_multicast() {
        cfg.ipv6_link_local
    } else if !cfg.ipv6_addr.is_unspecified() {
        cfg.ipv6_addr
    } else {
        cfg.ipv6_link_local
    };

    if src_ip.is_unspecified() {
        return false; // No IPv6 address configured
    }

    // Loopback shortcut: ::1 or own address
    let is_loopback = dst.is_loopback()
        || (dst == cfg.ipv6_link_local && !cfg.ipv6_link_local.is_unspecified())
        || (dst == cfg.ipv6_addr && !cfg.ipv6_addr.is_unspecified());

    if is_loopback {
        let lo_src = if dst.is_loopback() {
            Ipv6Addr::LOOPBACK
        } else {
            dst
        };
        let packet = build_packet(lo_src, dst, next_header, 64, payload);
        handle_ipv6(&packet);
        return true;
    }

    let payload_len = payload.len();
    if payload_len > 65535 {
        return false;
    }
    // MTU check (1500 ethernet - 40 IPv6 header = 1460 max payload)
    if IPV6_HEADER_LEN + payload_len > 1500 {
        return false;
    }

    let packet = build_packet(src_ip, dst, next_header, 64, payload);

    // Resolve destination MAC
    let dst_mac = if dst.is_multicast() {
        dst.multicast_mac()
    } else {
        // Determine next-hop
        let next_hop = if dst.is_link_local() || cfg.is_local_v6(dst) {
            dst
        } else if !cfg.ipv6_gateway.is_unspecified() {
            cfg.ipv6_gateway
        } else {
            dst // Assume on-link if no gateway
        };

        match super::ndp::resolve(next_hop, 200) {
            Some(mac) => mac,
            None => {
                crate::serial_verbose_println!("  IPv6: NDP resolve failed for {}", next_hop);
                return false;
            }
        }
    };

    ethernet::send_frame(dst_mac, ethernet::ETHERTYPE_IPV6, &packet)
}

/// Build an IPv6 packet (header + payload).
fn build_packet(
    src: Ipv6Addr,
    dst: Ipv6Addr,
    next_header: u8,
    hop_limit: u8,
    payload: &[u8],
) -> Vec<u8> {
    let payload_len = payload.len() as u16;
    let total = IPV6_HEADER_LEN + payload.len();
    let mut packet = Vec::with_capacity(total);

    // Version (6), Traffic Class (0), Flow Label (0)
    packet.push(0x60); // version=6, TC high bits=0
    packet.push(0x00); // TC low bits + flow label high
    packet.push(0x00); // flow label
    packet.push(0x00); // flow label low

    // Payload length (big-endian)
    packet.push((payload_len >> 8) as u8);
    packet.push((payload_len & 0xFF) as u8);

    // Next header
    packet.push(next_header);

    // Hop limit
    packet.push(hop_limit);

    // Source address (16 bytes)
    packet.extend_from_slice(&src.0);

    // Destination address (16 bytes)
    packet.extend_from_slice(&dst.0);

    // Payload
    packet.extend_from_slice(payload);

    packet
}

/// Handle an incoming IPv6 packet.
pub fn handle_ipv6(data: &[u8]) {
    let pkt = match parse(data) {
        Some(p) => p,
        None => return,
    };

    // Check if this packet is addressed to us
    let cfg = super::config();
    let for_us = pkt.dst.is_multicast()
        || pkt.dst.is_loopback()
        || pkt.dst == cfg.ipv6_link_local
        || pkt.dst == cfg.ipv6_addr
        // Solicited-node multicast for our link-local
        || (!cfg.ipv6_link_local.is_unspecified()
            && pkt.dst == cfg.ipv6_link_local.solicited_node_multicast())
        // Solicited-node multicast for our global
        || (!cfg.ipv6_addr.is_unspecified()
            && pkt.dst == cfg.ipv6_addr.solicited_node_multicast());

    if !for_us {
        return;
    }

    match pkt.next_header {
        PROTO_ICMPV6 => super::icmpv6::handle_icmpv6(&pkt),
        PROTO_TCP => super::tcp::handle_tcp_v6(&pkt),
        PROTO_UDP => super::udp::handle_udp_v6(&pkt),
        _ => {}
    }
}
