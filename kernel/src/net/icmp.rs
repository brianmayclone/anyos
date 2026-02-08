/// ICMP (Internet Control Message Protocol) — ping echo request/reply.

use alloc::vec::Vec;
use super::types::Ipv4Addr;
use super::checksum;
use super::ipv4::Ipv4Packet;
use crate::sync::spinlock::Spinlock;

const ICMP_ECHO_REPLY: u8 = 0;
const ICMP_ECHO_REQUEST: u8 = 8;

/// Store for received ping replies
struct PingReply {
    src: Ipv4Addr,
    seq: u16,
    ttl: u8,
    data_len: usize,
    tick: u32,
}

static PING_REPLIES: Spinlock<Option<alloc::vec::Vec<PingReply>>> = Spinlock::new(None);

pub fn init() {
    let mut replies = PING_REPLIES.lock();
    *replies = Some(alloc::vec::Vec::new());
}

/// Send an ICMP echo request (ping)
pub fn send_echo_request(dst: Ipv4Addr, seq: u16, data: &[u8]) -> bool {
    let mut icmp = Vec::with_capacity(8 + data.len());
    icmp.push(ICMP_ECHO_REQUEST); // Type
    icmp.push(0);                  // Code
    icmp.push(0); icmp.push(0);    // Checksum (placeholder)
    icmp.push(0); icmp.push(1);    // Identifier (1)
    icmp.push((seq >> 8) as u8);   // Sequence (big-endian)
    icmp.push((seq & 0xFF) as u8);
    icmp.extend_from_slice(data);

    // Compute checksum
    let cksum = checksum::internet_checksum(&icmp);
    icmp[2] = (cksum >> 8) as u8;
    icmp[3] = (cksum & 0xFF) as u8;

    super::ipv4::send_ipv4(dst, super::ipv4::PROTO_ICMP, &icmp)
}

/// Ping a target with a sequence number and wait for reply.
/// Returns Some(rtt_in_ticks) on success, None on timeout.
pub fn ping(target: Ipv4Addr, seq: u16, timeout_ticks: u32) -> Option<(u32, u8)> {
    // Clear old replies for this seq
    {
        let mut replies = PING_REPLIES.lock();
        if let Some(list) = replies.as_mut() {
            list.retain(|r| r.seq != seq);
        }
    }

    let payload = b"anyOS ping";
    let start = crate::arch::x86::pit::get_ticks();

    if !send_echo_request(target, seq, payload) {
        return None;
    }

    loop {
        super::poll();

        {
            let mut replies = PING_REPLIES.lock();
            if let Some(list) = replies.as_mut() {
                if let Some(idx) = list.iter().position(|r| r.src == target && r.seq == seq) {
                    let reply = list.remove(idx);
                    let rtt = reply.tick.wrapping_sub(start);
                    return Some((rtt, reply.ttl));
                }
            }
        }

        let now = crate::arch::x86::pit::get_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            return None;
        }

        core::hint::spin_loop();
    }
}

/// Handle an incoming ICMP packet
pub fn handle_icmp(pkt: &Ipv4Packet<'_>) {
    let data = pkt.payload;
    if data.len() < 8 { return; }

    let icmp_type = data[0];
    let _code = data[1];

    match icmp_type {
        ICMP_ECHO_REQUEST => {
            // Build echo reply
            let mut reply = Vec::from(data);
            reply[0] = ICMP_ECHO_REPLY;
            reply[2] = 0; reply[3] = 0; // Clear checksum
            let cksum = checksum::internet_checksum(&reply);
            reply[2] = (cksum >> 8) as u8;
            reply[3] = (cksum & 0xFF) as u8;

            super::ipv4::send_ipv4(pkt.src, super::ipv4::PROTO_ICMP, &reply);
        }
        ICMP_ECHO_REPLY => {
            let seq = ((data[6] as u16) << 8) | data[7] as u16;
            let tick = crate::arch::x86::pit::get_ticks();

            let mut replies = PING_REPLIES.lock();
            if let Some(list) = replies.as_mut() {
                list.push(PingReply {
                    src: pkt.src,
                    seq,
                    ttl: pkt.ttl,
                    data_len: data.len() - 8,
                    tick,
                });
                // Keep list bounded
                if list.len() > 64 {
                    list.remove(0);
                }
            }
        }
        _ => {}
    }
}
