//! DNS resolver -- resolves hostnames to IPv4 addresses via UDP queries to port 53.
//! Supports A record lookups with a fixed transaction ID and 5-second timeout.

use alloc::vec::Vec;
use super::types::Ipv4Addr;

const DNS_PORT: u16 = 53;

/// Resolve a hostname to an IPv4 address using the configured DNS server.
pub fn resolve(hostname: &str) -> Result<Ipv4Addr, &'static str> {
    let cfg = super::config();
    if cfg.dns == Ipv4Addr::ZERO {
        return Err("No DNS server configured");
    }

    // Build DNS query
    let query = build_query(hostname);

    // Bind a temporary port
    let src_port: u16 = 49152; // Use a fixed ephemeral port
    super::udp::bind(src_port);

    // Send query
    super::udp::send(cfg.dns, src_port, DNS_PORT, &query);

    // Wait for response
    let result = match super::udp::recv_timeout(src_port, 500) {
        Some(dgram) => parse_response(&dgram.data),
        None => Err("DNS timeout"),
    };

    super::udp::unbind(src_port);
    result
}

fn build_query(hostname: &str) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(64);

    // Transaction ID
    pkt.push(0xAA); pkt.push(0xBB);
    // Flags: standard query, recursion desired
    pkt.push(0x01); pkt.push(0x00);
    // Questions: 1
    pkt.push(0x00); pkt.push(0x01);
    // Answers: 0
    pkt.push(0x00); pkt.push(0x00);
    // Authority: 0
    pkt.push(0x00); pkt.push(0x00);
    // Additional: 0
    pkt.push(0x00); pkt.push(0x00);

    // Question: encode hostname as DNS labels
    for label in hostname.split('.') {
        let bytes = label.as_bytes();
        if bytes.len() > 63 { continue; }
        pkt.push(bytes.len() as u8);
        pkt.extend_from_slice(bytes);
    }
    pkt.push(0); // Root label

    // Type: A (host address)
    pkt.push(0x00); pkt.push(0x01);
    // Class: IN (Internet)
    pkt.push(0x00); pkt.push(0x01);

    pkt
}

fn parse_response(data: &[u8]) -> Result<Ipv4Addr, &'static str> {
    if data.len() < 12 {
        return Err("DNS response too short");
    }

    // Check transaction ID
    if data[0] != 0xAA || data[1] != 0xBB {
        return Err("DNS transaction ID mismatch");
    }

    // Check response flag
    if data[2] & 0x80 == 0 {
        return Err("DNS: not a response");
    }

    // Check RCODE (bits 0-3 of byte 3)
    let rcode = data[3] & 0x0F;
    if rcode != 0 {
        return Err("DNS query failed");
    }

    let answer_count = ((data[6] as u16) << 8) | data[7] as u16;
    if answer_count == 0 {
        return Err("DNS: no answers");
    }

    // Skip question section
    let mut off = 12;
    // Skip QNAME
    off = skip_name(data, off)?;
    // Skip QTYPE + QCLASS
    off += 4;
    if off > data.len() { return Err("DNS: truncated"); }

    // Parse answers
    for _ in 0..answer_count {
        if off >= data.len() { break; }
        // Skip NAME (may be a pointer)
        off = skip_name(data, off)?;
        if off + 10 > data.len() { return Err("DNS: truncated answer"); }

        let rtype = ((data[off] as u16) << 8) | data[off + 1] as u16;
        let rdlength = ((data[off + 8] as u16) << 8) | data[off + 9] as u16;
        off += 10;

        if rtype == 1 && rdlength == 4 {
            // A record
            if off + 4 > data.len() { return Err("DNS: truncated A record"); }
            return Ok(Ipv4Addr([data[off], data[off+1], data[off+2], data[off+3]]));
        }

        off += rdlength as usize;
    }

    Err("DNS: no A record found")
}

fn skip_name(data: &[u8], mut off: usize) -> Result<usize, &'static str> {
    // DNS names can be either labels or pointers (or mix)
    let mut jumps = 0;
    loop {
        if off >= data.len() { return Err("DNS: name overflow"); }
        let b = data[off];
        if b == 0 {
            // End of name
            return Ok(off + 1);
        }
        if b & 0xC0 == 0xC0 {
            // Pointer — skip 2 bytes, done
            return Ok(off + 2);
        }
        // Label: skip length + label bytes
        off += 1 + (b as usize);
        jumps += 1;
        if jumps > 128 { return Err("DNS: name too long"); }
    }
}
