/// ARP (Address Resolution Protocol) — resolves IPv4 addresses to MAC addresses.

use alloc::collections::BTreeMap;
use super::types::{MacAddr, Ipv4Addr};
use super::ethernet;
use crate::sync::spinlock::Spinlock;

const ARP_HW_ETHERNET: u16 = 1;
const ARP_PROTO_IPV4: u16 = 0x0800;
const ARP_OP_REQUEST: u16 = 1;
const ARP_OP_REPLY: u16 = 2;

// ARP table: maps IP (as u32) -> (MAC, tick when learned)
static ARP_TABLE: Spinlock<Option<BTreeMap<u32, (MacAddr, u32)>>> = Spinlock::new(None);

fn table() -> &'static Spinlock<Option<BTreeMap<u32, (MacAddr, u32)>>> {
    &ARP_TABLE
}

pub fn init() {
    let mut t = ARP_TABLE.lock();
    *t = Some(BTreeMap::new());
}

/// Look up a MAC for the given IP. Returns None if not cached.
pub fn lookup(ip: Ipv4Addr) -> Option<MacAddr> {
    let t = table().lock();
    t.as_ref().and_then(|map| map.get(&ip.to_u32()).map(|(mac, _)| *mac))
}

/// Insert an entry into the ARP table
pub fn insert(ip: Ipv4Addr, mac: MacAddr) {
    let mut t = table().lock();
    if let Some(map) = t.as_mut() {
        let ticks = crate::arch::x86::pit::get_ticks();
        map.insert(ip.to_u32(), (mac, ticks));
    }
}

/// Get all ARP table entries
pub fn entries() -> alloc::vec::Vec<(Ipv4Addr, MacAddr)> {
    let t = table().lock();
    let mut result = alloc::vec::Vec::new();
    if let Some(map) = t.as_ref() {
        for (&ip_u32, &(mac, _)) in map.iter() {
            result.push((Ipv4Addr::from_u32(ip_u32), mac));
        }
    }
    result
}

/// Send an ARP request for the given IP
pub fn request(target_ip: Ipv4Addr) {
    let cfg = super::config();
    let mut packet = [0u8; 28]; // ARP packet is 28 bytes

    // Hardware type: Ethernet
    packet[0] = 0; packet[1] = 1;
    // Protocol type: IPv4
    packet[2] = 0x08; packet[3] = 0x00;
    // Hardware addr len: 6
    packet[4] = 6;
    // Protocol addr len: 4
    packet[5] = 4;
    // Operation: request
    packet[6] = 0; packet[7] = 1;
    // Sender MAC
    packet[8..14].copy_from_slice(&cfg.mac.0);
    // Sender IP
    packet[14..18].copy_from_slice(&cfg.ip.0);
    // Target MAC: 00:00:00:00:00:00 (unknown)
    packet[18..24].copy_from_slice(&[0; 6]);
    // Target IP
    packet[24..28].copy_from_slice(&target_ip.0);

    ethernet::send_frame(MacAddr::BROADCAST, ethernet::ETHERTYPE_ARP, &packet);
}

/// Resolve an IP to MAC address, with timeout. Sends ARP request if needed.
pub fn resolve(ip: Ipv4Addr, timeout_ticks: u32) -> Option<MacAddr> {
    // Check cache first
    if let Some(mac) = lookup(ip) {
        return Some(mac);
    }

    // Broadcast address resolves to broadcast MAC
    if ip == Ipv4Addr::BROADCAST {
        return Some(MacAddr::BROADCAST);
    }

    // Send ARP request
    request(ip);

    let start = crate::arch::x86::pit::get_ticks();
    loop {
        // Poll for incoming packets
        super::poll();

        if let Some(mac) = lookup(ip) {
            return Some(mac);
        }

        let now = crate::arch::x86::pit::get_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            return None;
        }

        // Small delay to avoid spinning too fast
        core::hint::spin_loop();
    }
}

/// Handle an incoming ARP packet
pub fn handle_arp(data: &[u8]) {
    if data.len() < 28 { return; }

    let hw_type = ((data[0] as u16) << 8) | data[1] as u16;
    let proto = ((data[2] as u16) << 8) | data[3] as u16;
    let op = ((data[6] as u16) << 8) | data[7] as u16;

    if hw_type != ARP_HW_ETHERNET || proto != ARP_PROTO_IPV4 { return; }

    let sender_mac = MacAddr([data[8], data[9], data[10], data[11], data[12], data[13]]);
    let sender_ip = Ipv4Addr([data[14], data[15], data[16], data[17]]);
    let target_ip = Ipv4Addr([data[24], data[25], data[26], data[27]]);

    // Always learn the sender
    insert(sender_ip, sender_mac);

    let cfg = super::config();

    match op {
        ARP_OP_REQUEST => {
            // If they're asking for our IP, reply
            if target_ip == cfg.ip {
                let mut reply = [0u8; 28];
                reply[0] = 0; reply[1] = 1;   // HW type
                reply[2] = 0x08; reply[3] = 0; // Proto type
                reply[4] = 6; reply[5] = 4;    // Lengths
                reply[6] = 0; reply[7] = 2;    // Op: reply
                reply[8..14].copy_from_slice(&cfg.mac.0);  // Sender MAC (us)
                reply[14..18].copy_from_slice(&cfg.ip.0);  // Sender IP (us)
                reply[18..24].copy_from_slice(&sender_mac.0); // Target MAC
                reply[24..28].copy_from_slice(&sender_ip.0);  // Target IP

                ethernet::send_frame(sender_mac, ethernet::ETHERTYPE_ARP, &reply);
            }
        }
        ARP_OP_REPLY => {
            // Already inserted above
        }
        _ => {}
    }
}
