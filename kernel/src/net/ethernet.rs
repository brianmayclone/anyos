//! Ethernet frame handling: parse incoming frames and build outgoing ones.

use alloc::vec::Vec;
use super::types::MacAddr;

/// EtherType value for ARP frames.
pub const ETHERTYPE_ARP: u16  = 0x0806;
/// EtherType value for IPv4 frames.
pub const ETHERTYPE_IPV4: u16 = 0x0800;

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

/// Build an Ethernet frame: dst + src + ethertype + payload
pub fn build_frame(dst: MacAddr, src: MacAddr, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(ETH_HEADER_LEN + payload.len());
    frame.extend_from_slice(&dst.0);
    frame.extend_from_slice(&src.0);
    frame.push((ethertype >> 8) as u8);
    frame.push((ethertype & 0xFF) as u8);
    frame.extend_from_slice(payload);
    // Pad to minimum Ethernet frame size (60 bytes without FCS)
    while frame.len() < 60 {
        frame.push(0);
    }
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
        _ => {}
    }
}

/// Send a raw Ethernet frame
pub fn send_frame(dst: MacAddr, ethertype: u16, payload: &[u8]) {
    let our_mac = super::config().mac;
    let frame = build_frame(dst, our_mac, ethertype, payload);
    crate::drivers::network::e1000::transmit(&frame);
}
