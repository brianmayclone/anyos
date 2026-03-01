//! DNS resolver -- resolves hostnames to IPv4 addresses via UDP queries to port 53.
//! Supports A record lookups with caching, 500ms timeout, and static hosts file.
//!
//! Resolution order: hosts file → cache → DNS query.

use alloc::string::String;
use alloc::vec::Vec;
use super::types::Ipv4Addr;
use crate::sync::spinlock::Spinlock;

const DNS_PORT: u16 = 53;
const DNS_CACHE_SIZE: usize = 64;
const HOSTS_PATH: &str = "/System/etc/network/hosts";
const MAX_HOSTS_ENTRIES: usize = 128;

// ── Hosts file ──────────────────────────────────────────────────────────────

/// A static hostname→IP mapping from the hosts file.
struct HostEntry {
    hostname: String,
    addr: Ipv4Addr,
}

/// Static hosts table loaded from `/System/etc/network/hosts`.
static HOSTS_TABLE: Spinlock<Vec<HostEntry>> = Spinlock::new(Vec::new());

/// Load (or reload) the hosts file from disk into the in-memory table.
/// Called at boot after VFS init, and on demand via syscall when the file changes.
pub fn load_hosts() {
    let data = match crate::fs::vfs::read_file_to_vec(HOSTS_PATH) {
        Ok(d) => d,
        Err(_) => {
            crate::serial_println!("[NET] hosts file not found: {}", HOSTS_PATH);
            return;
        }
    };

    let text = match core::str::from_utf8(&data) {
        Ok(t) => t,
        Err(_) => {
            crate::serial_println!("[NET] hosts file is not valid UTF-8");
            return;
        }
    };

    let mut entries: Vec<HostEntry> = Vec::new();

    for line in text.split('\n') {
        let line = line.trim();
        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split on whitespace: first token is IP, rest are hostnames/aliases
        let mut parts = line.split_whitespace();
        let ip_str = match parts.next() {
            Some(s) => s,
            None => continue,
        };

        let ip = match Ipv4Addr::parse(ip_str) {
            Some(a) => a,
            None => continue,
        };

        // Each remaining token is a hostname or alias
        for name in parts {
            if entries.len() >= MAX_HOSTS_ENTRIES {
                break;
            }
            entries.push(HostEntry {
                hostname: String::from(name),
                addr: ip,
            });
        }
    }

    let count = entries.len();
    {
        let mut table = HOSTS_TABLE.lock();
        *table = entries;
    }
    crate::serial_println!("[OK] Loaded {} hosts entries from {}", count, HOSTS_PATH);
}

/// Look up a hostname in the static hosts table.
fn hosts_lookup(hostname: &str) -> Option<Ipv4Addr> {
    let table = HOSTS_TABLE.lock();
    for entry in table.iter() {
        if entry.hostname.eq_ignore_ascii_case(hostname) {
            return Some(entry.addr);
        }
    }
    None
}

/// A cached DNS entry.
struct DnsCacheEntry {
    hostname: String,
    addr: Ipv4Addr,
    /// PIT tick when this entry was cached.
    cached_at: u32,
}

/// Simple DNS cache — stores up to DNS_CACHE_SIZE entries, evicts oldest on overflow.
static DNS_CACHE: Spinlock<Vec<DnsCacheEntry>> = Spinlock::new(Vec::new());

/// Look up a cached DNS entry. Returns None if not cached or expired (5 min TTL).
fn cache_lookup(hostname: &str) -> Option<Ipv4Addr> {
    let cache = DNS_CACHE.lock();
    let now = crate::arch::hal::timer_current_ticks();
    let ttl_ticks = 300 * crate::arch::hal::timer_frequency_hz() as u32; // 5 minutes
    for entry in cache.iter() {
        if entry.hostname == hostname {
            if now.wrapping_sub(entry.cached_at) < ttl_ticks {
                return Some(entry.addr);
            }
        }
    }
    None
}

/// Insert or update a DNS cache entry.
fn cache_insert(hostname: &str, addr: Ipv4Addr) {
    let mut cache = DNS_CACHE.lock();
    let now = crate::arch::hal::timer_current_ticks();
    // Update existing entry if present.
    for entry in cache.iter_mut() {
        if entry.hostname == hostname {
            entry.addr = addr;
            entry.cached_at = now;
            return;
        }
    }
    // Evict oldest if at capacity.
    if cache.len() >= DNS_CACHE_SIZE {
        cache.remove(0);
    }
    cache.push(DnsCacheEntry {
        hostname: String::from(hostname),
        addr,
        cached_at: now,
    });
}

/// Resolve a hostname to an IPv4 address.
///
/// Resolution order: hosts file → cache → DNS query.
/// DNS results are cached for 5 minutes.
pub fn resolve(hostname: &str) -> Result<Ipv4Addr, &'static str> {
    // 1. Check static hosts file.
    if let Some(addr) = hosts_lookup(hostname) {
        return Ok(addr);
    }

    // 2. Check DNS cache.
    if let Some(addr) = cache_lookup(hostname) {
        return Ok(addr);
    }

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

    // Cache successful results.
    if let Ok(addr) = result {
        cache_insert(hostname, addr);
    }

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
