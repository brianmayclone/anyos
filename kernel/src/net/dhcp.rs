//! DHCP client -- obtains IP configuration from a DHCP server.
//! Implements the 4-step handshake: DISCOVER -> OFFER -> REQUEST -> ACK.

use alloc::vec;
use alloc::vec::Vec;
use super::types::{Ipv4Addr, MacAddr};

const DHCP_SERVER_PORT: u16 = 67;
const DHCP_CLIENT_PORT: u16 = 68;

const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;

const DHCP_MAGIC: [u8; 4] = [99, 130, 83, 99];

/// The network configuration obtained from a successful DHCP exchange.
pub struct DhcpResult {
    pub ip: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns: Ipv4Addr,
    pub server_ip: Ipv4Addr,
}

/// Run DHCP discovery and return the assigned configuration.
pub fn discover() -> Result<DhcpResult, &'static str> {
    let mac = super::config().mac;

    // Bind to DHCP client port
    super::udp::bind(DHCP_CLIENT_PORT);

    // --- Send DISCOVER ---
    let discover = build_dhcp_packet(DHCP_DISCOVER, mac, Ipv4Addr::ZERO, Ipv4Addr::ZERO);
    super::udp::send_raw(
        Ipv4Addr::ZERO, Ipv4Addr::BROADCAST, MacAddr::BROADCAST,
        DHCP_CLIENT_PORT, DHCP_SERVER_PORT, &discover,
    );

    crate::serial_println!("  DHCP: DISCOVER sent");

    // --- Wait for OFFER ---
    let offer = wait_dhcp_response(DHCP_OFFER, 500)?;
    crate::serial_println!("  DHCP: OFFER received - IP {}", offer.ip);

    // --- Send REQUEST ---
    let request = build_dhcp_packet(DHCP_REQUEST, mac, offer.ip, offer.server_ip);
    super::udp::send_raw(
        Ipv4Addr::ZERO, Ipv4Addr::BROADCAST, MacAddr::BROADCAST,
        DHCP_CLIENT_PORT, DHCP_SERVER_PORT, &request,
    );

    crate::serial_println!("  DHCP: REQUEST sent for {}", offer.ip);

    // --- Wait for ACK ---
    let ack = wait_dhcp_response(DHCP_ACK, 500)?;
    crate::serial_println!("  DHCP: ACK received");

    super::udp::unbind(DHCP_CLIENT_PORT);

    Ok(ack)
}

fn wait_dhcp_response(expected_type: u8, timeout_ticks: u32) -> Result<DhcpResult, &'static str> {
    let start = crate::arch::hal::timer_current_ticks();

    loop {
        super::poll();

        if let Some(dgram) = super::udp::recv(DHCP_CLIENT_PORT) {
            if let Some(result) = parse_dhcp_response(&dgram.data, expected_type) {
                return Ok(result);
            }
        }

        let now = crate::arch::hal::timer_current_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            return Err("DHCP timeout");
        }

        core::hint::spin_loop();
    }
}

fn build_dhcp_packet(msg_type: u8, mac: MacAddr, requested_ip: Ipv4Addr, server_ip: Ipv4Addr) -> Vec<u8> {
    let mut pkt = vec![0u8; 300]; // DHCP minimum packet

    pkt[0] = 1;    // Op: BOOTREQUEST
    pkt[1] = 1;    // HType: Ethernet
    pkt[2] = 6;    // HLen: MAC length
    pkt[3] = 0;    // Hops

    // XID (transaction ID) - use a simple constant
    pkt[4] = 0x39; pkt[5] = 0x03; pkt[6] = 0xF3; pkt[7] = 0x26;

    // Secs
    pkt[8] = 0; pkt[9] = 0;
    // Flags: broadcast
    pkt[10] = 0x80; pkt[11] = 0x00;

    // Client MAC
    pkt[28..34].copy_from_slice(&mac.0);

    // Magic cookie
    pkt[236..240].copy_from_slice(&DHCP_MAGIC);

    // DHCP options
    let mut off = 240;

    // Option 53: DHCP Message Type
    pkt[off] = 53; pkt[off + 1] = 1; pkt[off + 2] = msg_type; off += 3;

    if msg_type == DHCP_REQUEST {
        // Option 50: Requested IP
        pkt[off] = 50; pkt[off + 1] = 4;
        pkt[off + 2..off + 6].copy_from_slice(&requested_ip.0);
        off += 6;

        // Option 54: Server Identifier
        pkt[off] = 54; pkt[off + 1] = 4;
        pkt[off + 2..off + 6].copy_from_slice(&server_ip.0);
        off += 6;
    }

    // Option 55: Parameter Request List
    pkt[off] = 55; pkt[off + 1] = 3;
    pkt[off + 2] = 1;  // Subnet mask
    pkt[off + 3] = 3;  // Router
    pkt[off + 4] = 6;  // DNS
    off += 5;

    // End option
    pkt[off] = 255;

    pkt.truncate(off + 1);
    pkt
}

fn parse_dhcp_response(data: &[u8], expected_type: u8) -> Option<DhcpResult> {
    if data.len() < 240 { return None; }

    // Verify magic cookie
    if data[236..240] != DHCP_MAGIC { return None; }

    // Check op is BOOTREPLY
    if data[0] != 2 { return None; }

    // Your IP address (yiaddr)
    let ip = Ipv4Addr([data[16], data[17], data[18], data[19]]);

    // Server IP (siaddr)
    let mut server_ip = Ipv4Addr([data[20], data[21], data[22], data[23]]);

    // Parse options
    let mut mask = Ipv4Addr::new(255, 255, 255, 0);
    let mut gateway = Ipv4Addr::ZERO;
    let mut dns = Ipv4Addr::ZERO;
    let mut msg_type: u8 = 0;

    let mut off = 240;
    while off < data.len() {
        let opt = data[off];
        if opt == 255 { break; }      // End
        if opt == 0 { off += 1; continue; } // Pad
        if off + 1 >= data.len() { break; }
        let len = data[off + 1] as usize;
        if off + 2 + len > data.len() { break; }

        match opt {
            53 if len >= 1 => msg_type = data[off + 2],
            1 if len >= 4 => mask = Ipv4Addr([data[off+2], data[off+3], data[off+4], data[off+5]]),
            3 if len >= 4 => gateway = Ipv4Addr([data[off+2], data[off+3], data[off+4], data[off+5]]),
            6 if len >= 4 => dns = Ipv4Addr([data[off+2], data[off+3], data[off+4], data[off+5]]),
            54 if len >= 4 => server_ip = Ipv4Addr([data[off+2], data[off+3], data[off+4], data[off+5]]),
            _ => {}
        }

        off += 2 + len;
    }

    if msg_type != expected_type { return None; }

    Some(DhcpResult { ip, mask, gateway, dns, server_ip })
}
