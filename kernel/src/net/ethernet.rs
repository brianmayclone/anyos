//! Ethernet frame handling: parse incoming frames and build outgoing ones.

use alloc::vec::Vec;
use super::types::MacAddr;

/// EtherType value for ARP frames.
pub const ETHERTYPE_ARP: u16  = 0x0806;
/// EtherType value for IPv4 frames.
pub const ETHERTYPE_IPV4: u16 = 0x0800;
/// EtherType value for IPv6 frames.
pub const ETHERTYPE_IPV6: u16 = 0x86DD;

const ETH_HEADER_LEN: usize = 14;

/// A parsed Ethernet frame with references into the original packet buffer.
pub struct EthFrame<'a> {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ethertype: u16,
    pub payload: &'a [u8],
}

/// Parse raw bytes into an Ethernet frame. Returns `None` if too short.
pub fn parse(data: &[u8]) -> Option<EthFrame<'_>> {
    if data.len() < ETH_HEADER_LEN {
        return None;
    }

    let dst = MacAddr([data[0], data[1], data[2], data[3], data[4], data[5]]);
    let src = MacAddr([data[6], data[7], data[8], data[9], data[10], data[11]]);
    let ethertype = ((data[12] as u16) << 8) | (data[13] as u16);
    let payload = &data[ETH_HEADER_LEN..];

    Some(EthFrame { dst, src, ethertype, payload })
}

/// Build an Ethernet frame: dst + src + ethertype + payload.
/// Uses a pre-sized Vec with direct header writes to avoid per-byte pushes.
pub fn build_frame(dst: MacAddr, src: MacAddr, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let total = (ETH_HEADER_LEN + payload.len()).max(60);
    let mut frame = alloc::vec![0u8; total];
    // Write header directly (14 bytes) — no push/extend overhead
    frame[0..6].copy_from_slice(&dst.0);
    frame[6..12].copy_from_slice(&src.0);
    frame[12] = (ethertype >> 8) as u8;
    frame[13] = (ethertype & 0xFF) as u8;
    // Copy payload after header (remaining bytes already zero = padding)
    frame[ETH_HEADER_LEN..ETH_HEADER_LEN + payload.len()].copy_from_slice(payload);
    frame
}

/// Dispatch an incoming Ethernet frame to the appropriate protocol handler
pub fn handle_frame(data: &[u8]) {
    let frame = match parse(data) {
        Some(f) => f,
        None => return,
    };

    match frame.ethertype {
        ETHERTYPE_ARP => super::arp::handle_arp(frame.payload),
        ETHERTYPE_IPV4 => super::ipv4::handle_ipv4(frame.payload),
        ETHERTYPE_IPV6 => super::ipv6::handle_ipv6(frame.payload),
        _ => {}
    }
}

/// Send a raw Ethernet frame.
pub fn send_frame(dst: MacAddr, ethertype: u16, payload: &[u8]) {
    let our_mac = super::config().mac;
    let frame = build_frame(dst, our_mac, ethertype, payload);
    // Use the registered network driver (E1000, VirtIO Net, etc.)
    crate::drivers::network::transmit(&frame);
}
