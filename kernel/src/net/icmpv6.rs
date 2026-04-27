//! ICMPv6 (Internet Control Message Protocol for IPv6) — echo request/reply
//! and Neighbor Discovery Protocol messages (RFC 4443, RFC 4861).

use super::checksum;
use super::ipv6::Ipv6Packet;
use super::types::{Ipv6Addr, MacAddr};
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

// ── ICMPv6 message types ────────────────────────────────────────────

/// Destination Unreachable.
pub const ICMPV6_DEST_UNREACHABLE: u8 = 1;
/// Packet Too Big.
pub const ICMPV6_PACKET_TOO_BIG: u8 = 2;
/// Time Exceeded.
pub const ICMPV6_TIME_EXCEEDED: u8 = 3;
/// Echo Request (ping).
pub const ICMPV6_ECHO_REQUEST: u8 = 128;
/// Echo Reply (pong).
pub const ICMPV6_ECHO_REPLY: u8 = 129;

// ── NDP message types (RFC 4861) ────────────────────────────────────

/// Router Solicitation.
pub const ICMPV6_ROUTER_SOLICITATION: u8 = 133;
/// Router Advertisement.
pub const ICMPV6_ROUTER_ADVERTISEMENT: u8 = 134;
/// Neighbor Solicitation.
pub const ICMPV6_NEIGHBOR_SOLICITATION: u8 = 135;
/// Neighbor Advertisement.
pub const ICMPV6_NEIGHBOR_ADVERTISEMENT: u8 = 136;

// ── NDP option types ────────────────────────────────────────────────

/// Source Link-Layer Address option.
const NDP_OPT_SOURCE_LLA: u8 = 1;
/// Target Link-Layer Address option.
const NDP_OPT_TARGET_LLA: u8 = 2;
/// Prefix Information option.
const NDP_OPT_PREFIX_INFO: u8 = 3;

// ── Ping reply storage ──────────────────────────────────────────────

struct Ping6Reply {
    src: Ipv6Addr,
    seq: u16,
    hop_limit: u8,
    data_len: usize,
    tick: u32,
}

static PING6_REPLIES: Spinlock<Vec<Ping6Reply>> = Spinlock::new(Vec::new());

/// Send an ICMPv6 echo request (ping6).
pub fn send_echo_request(dst: Ipv6Addr, seq: u16, data: &[u8]) -> bool {
    let cfg = super::config();
    let src = if dst.is_link_local() {
        cfg.ipv6_link_local
    } else if !cfg.ipv6_addr.is_unspecified() {
        cfg.ipv6_addr
    } else {
        cfg.ipv6_link_local
    };

    if src.is_unspecified() {
        return false;
    }

    let mut icmp = Vec::with_capacity(8 + data.len());
    icmp.push(ICMPV6_ECHO_REQUEST); // Type
    icmp.push(0); // Code
    icmp.push(0);
    icmp.push(0); // Checksum placeholder
    icmp.push(0);
    icmp.push(1); // Identifier (1)
    icmp.push((seq >> 8) as u8); // Sequence (big-endian)
    icmp.push((seq & 0xFF) as u8);
    icmp.extend_from_slice(data);

    // Compute ICMPv6 checksum (mandatory, uses IPv6 pseudo-header)
    compute_icmpv6_checksum(&mut icmp, &src, &dst);

    super::ipv6::send_ipv6(dst, super::ipv6::PROTO_ICMPV6, &icmp)
}

/// Ping6 with timeout. Returns Some((rtt_ticks, hop_limit)) on success.
pub fn ping6(target: Ipv6Addr, seq: u16, timeout_ticks: u32) -> Option<(u32, u8)> {
    {
        let mut replies = PING6_REPLIES.lock();
        replies.retain(|r| r.seq != seq);
    }

    let payload = b"anyOS ping6";
    let start = crate::arch::hal::timer_current_ticks();

    if !send_echo_request(target, seq, payload) {
        return None;
    }

    loop {
        super::poll();
        {
            let mut replies = PING6_REPLIES.lock();
            if let Some(idx) = replies.iter().position(|r| r.src == target && r.seq == seq) {
                let reply = replies.remove(idx);
                let rtt = reply.tick.wrapping_sub(start);
                return Some((rtt, reply.hop_limit));
            }
        }
        let now = crate::arch::hal::timer_current_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            return None;
        }
        core::hint::spin_loop();
    }
}

/// Handle an incoming ICMPv6 packet.
pub fn handle_icmpv6(pkt: &Ipv6Packet<'_>) {
    let data = pkt.payload;
    if data.len() < 4 {
        return;
    }

    let icmp_type = data[0];
    let _code = data[1];

    match icmp_type {
        ICMPV6_ECHO_REQUEST => {
            if data.len() < 8 {
                return;
            }
            // Build echo reply
            let cfg = super::config();
            let src = if pkt.dst.is_multicast() {
                cfg.ipv6_link_local
            } else {
                pkt.dst
            };
            let mut reply = Vec::from(data);
            reply[0] = ICMPV6_ECHO_REPLY;
            reply[2] = 0;
            reply[3] = 0; // Clear checksum
            compute_icmpv6_checksum(&mut reply, &src, &pkt.src);
            super::ipv6::send_ipv6(pkt.src, super::ipv6::PROTO_ICMPV6, &reply);
        }
        ICMPV6_ECHO_REPLY => {
            if data.len() < 8 {
                return;
            }
            let seq = ((data[6] as u16) << 8) | data[7] as u16;
            let tick = crate::arch::hal::timer_current_ticks();
            let mut replies = PING6_REPLIES.lock();
            replies.push(Ping6Reply {
                src: pkt.src,
                seq,
                hop_limit: pkt.hop_limit,
                data_len: data.len() - 8,
                tick,
            });
            if replies.len() > 64 {
                replies.remove(0);
            }
        }
        ICMPV6_NEIGHBOR_SOLICITATION => {
            super::ndp::handle_neighbor_solicitation(pkt);
        }
        ICMPV6_NEIGHBOR_ADVERTISEMENT => {
            super::ndp::handle_neighbor_advertisement(pkt);
        }
        ICMPV6_ROUTER_ADVERTISEMENT => {
            super::ndp::handle_router_advertisement(pkt);
        }
        _ => {}
    }
}

/// Compute and set the ICMPv6 checksum field (bytes 2-3).
/// ICMPv6 checksum is mandatory and includes the IPv6 pseudo-header.
pub fn compute_icmpv6_checksum(icmp: &mut [u8], src: &Ipv6Addr, dst: &Ipv6Addr) {
    // Clear checksum field
    if icmp.len() < 4 {
        return;
    }
    icmp[2] = 0;
    icmp[3] = 0;

    let pseudo_sum = checksum::pseudo_header_checksum_v6(
        src.as_bytes(),
        dst.as_bytes(),
        super::ipv6::PROTO_ICMPV6,
        icmp.len() as u32,
    );

    let mut sum = pseudo_sum;
    let mut i = 0;
    while i + 1 < icmp.len() {
        sum += ((icmp[i] as u32) << 8) | (icmp[i + 1] as u32);
        i += 2;
    }
    if i < icmp.len() {
        sum += (icmp[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let cksum = !(sum as u16);
    icmp[2] = (cksum >> 8) as u8;
    icmp[3] = (cksum & 0xFF) as u8;
}
