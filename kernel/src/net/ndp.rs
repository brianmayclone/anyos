//! NDP (Neighbor Discovery Protocol, RFC 4861) — resolves IPv6 addresses to
//! MAC addresses using ICMPv6 Neighbor Solicitation / Advertisement.
//! Equivalent to ARP for IPv6.

use super::icmpv6;
use super::ipv6::Ipv6Packet;
use super::types::{Ipv6Addr, MacAddr};
use crate::sync::spinlock::Spinlock;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// NDP neighbor cache: maps IPv6 address (as [u8; 16]) to (MAC, tick learned).
/// Using a Vec of entries since BTreeMap needs Ord on [u8;16].
struct NdpEntry {
    ip: Ipv6Addr,
    mac: MacAddr,
    tick: u32,
}

static NDP_TABLE: Spinlock<Vec<NdpEntry>> = Spinlock::new(Vec::new());

/// Look up a MAC for the given IPv6 address. Returns None if not cached.
pub fn lookup(ip: Ipv6Addr) -> Option<MacAddr> {
    let table = NDP_TABLE.lock();
    for entry in table.iter() {
        if entry.ip == ip {
            return Some(entry.mac);
        }
    }
    None
}

/// Insert or update an entry in the NDP neighbor cache.
pub fn insert(ip: Ipv6Addr, mac: MacAddr) {
    let mut table = NDP_TABLE.lock();
    let ticks = crate::arch::hal::timer_current_ticks();
    for entry in table.iter_mut() {
        if entry.ip == ip {
            entry.mac = mac;
            entry.tick = ticks;
            return;
        }
    }
    // Limit cache size
    if table.len() >= 256 {
        table.remove(0);
    }
    table.push(NdpEntry {
        ip,
        mac,
        tick: ticks,
    });
}

/// Get all NDP neighbor cache entries.
pub fn entries() -> Vec<(Ipv6Addr, MacAddr)> {
    let table = NDP_TABLE.lock();
    table.iter().map(|e| (e.ip, e.mac)).collect()
}

/// Send a Neighbor Solicitation for the given IPv6 address.
///
/// ICMPv6 Neighbor Solicitation (type 135):
///   [type:1, code:1, checksum:2, reserved:4, target:16, options...]
///   Option: Source Link-Layer Address (type 1, len 1 = 8 bytes)
fn send_neighbor_solicitation(target: Ipv6Addr) {
    let cfg = super::config();
    let src = cfg.ipv6_link_local;
    if src.is_unspecified() {
        return;
    }

    // Destination: solicited-node multicast of target
    let dst = target.solicited_node_multicast();

    let mut icmp = Vec::with_capacity(32);
    // Type: Neighbor Solicitation (135)
    icmp.push(icmpv6::ICMPV6_NEIGHBOR_SOLICITATION);
    // Code: 0
    icmp.push(0);
    // Checksum placeholder
    icmp.push(0);
    icmp.push(0);
    // Reserved (4 bytes)
    icmp.push(0);
    icmp.push(0);
    icmp.push(0);
    icmp.push(0);
    // Target address (16 bytes)
    icmp.extend_from_slice(&target.0);

    // Option: Source Link-Layer Address (type=1, len=1 unit=8 bytes)
    icmp.push(1); // Type: Source LLA
    icmp.push(1); // Length: 1 (in units of 8 bytes)
    icmp.extend_from_slice(&cfg.mac.0);

    // Compute ICMPv6 checksum
    icmpv6::compute_icmpv6_checksum(&mut icmp, &src, &dst);

    super::ipv6::send_ipv6(dst, super::ipv6::PROTO_ICMPV6, &icmp);
}

/// Resolve an IPv6 address to a MAC address, with timeout.
/// Sends Neighbor Solicitation if not cached.
pub fn resolve(ip: Ipv6Addr, timeout_ticks: u32) -> Option<MacAddr> {
    // Check cache first
    if let Some(mac) = lookup(ip) {
        return Some(mac);
    }

    // Multicast addresses map directly to MAC
    if ip.is_multicast() {
        return Some(ip.multicast_mac());
    }

    // Send Neighbor Solicitation
    send_neighbor_solicitation(ip);

    let start = crate::arch::hal::timer_current_ticks();
    loop {
        super::poll();

        if let Some(mac) = lookup(ip) {
            return Some(mac);
        }

        let now = crate::arch::hal::timer_current_ticks();
        if now.wrapping_sub(start) >= timeout_ticks {
            return None;
        }

        super::wait_for_poll_progress();
    }
}

/// Handle an incoming Neighbor Solicitation (ICMPv6 type 135).
///
/// If the target is our address, reply with a Neighbor Advertisement.
pub fn handle_neighbor_solicitation(pkt: &Ipv6Packet<'_>) {
    let data = pkt.payload;
    // NS: type(1) + code(1) + checksum(2) + reserved(4) + target(16) = 24 min
    if data.len() < 24 {
        return;
    }

    let mut target = [0u8; 16];
    target.copy_from_slice(&data[8..24]);
    let target_addr = Ipv6Addr(target);

    let cfg = super::config();

    // Check if the target is one of our addresses
    let is_ours = target_addr == cfg.ipv6_link_local || target_addr == cfg.ipv6_addr;
    if !is_ours {
        return;
    }

    // Parse options to learn source MAC
    if data.len() > 24 {
        parse_ndp_options(&data[24..], |opt_type, opt_data| {
            if opt_type == 1 && opt_data.len() >= 6 {
                // Source Link-Layer Address
                let mac = MacAddr([
                    opt_data[0],
                    opt_data[1],
                    opt_data[2],
                    opt_data[3],
                    opt_data[4],
                    opt_data[5],
                ]);
                if !pkt.src.is_unspecified() {
                    insert(pkt.src, mac);
                }
            }
        });
    }

    // Send Neighbor Advertisement
    let dst = if pkt.src.is_unspecified() {
        Ipv6Addr::ALL_NODES // DAD — reply to all-nodes multicast
    } else {
        pkt.src
    };

    let mut reply = Vec::with_capacity(32);
    // Type: Neighbor Advertisement (136)
    reply.push(icmpv6::ICMPV6_NEIGHBOR_ADVERTISEMENT);
    // Code: 0
    reply.push(0);
    // Checksum placeholder
    reply.push(0);
    reply.push(0);
    // Flags: Solicited (S=1), Override (O=1)
    // Bits: R(0) S(1) O(1) Reserved(29) → 0x60000000
    reply.push(0x60);
    reply.push(0x00);
    reply.push(0x00);
    reply.push(0x00);
    // Target address
    reply.extend_from_slice(&target_addr.0);

    // Option: Target Link-Layer Address (type=2, len=1)
    reply.push(2); // Type: Target LLA
    reply.push(1); // Length: 1 (8 bytes)
    reply.extend_from_slice(&cfg.mac.0);

    icmpv6::compute_icmpv6_checksum(&mut reply, &target_addr, &dst);
    super::ipv6::send_ipv6(dst, super::ipv6::PROTO_ICMPV6, &reply);
}

/// Handle an incoming Neighbor Advertisement (ICMPv6 type 136).
///
/// Learn the target's MAC address from the Target Link-Layer Address option.
pub fn handle_neighbor_advertisement(pkt: &Ipv6Packet<'_>) {
    let data = pkt.payload;
    // NA: type(1) + code(1) + checksum(2) + flags(4) + target(16) = 24 min
    if data.len() < 24 {
        return;
    }

    let mut target = [0u8; 16];
    target.copy_from_slice(&data[8..24]);
    let target_addr = Ipv6Addr(target);

    // Parse options for Target Link-Layer Address
    if data.len() > 24 {
        parse_ndp_options(&data[24..], |opt_type, opt_data| {
            if opt_type == 2 && opt_data.len() >= 6 {
                // Target Link-Layer Address
                let mac = MacAddr([
                    opt_data[0],
                    opt_data[1],
                    opt_data[2],
                    opt_data[3],
                    opt_data[4],
                    opt_data[5],
                ]);
                insert(target_addr, mac);
            }
        });
    }
}

/// Handle a Router Advertisement (ICMPv6 type 134).
///
/// Extracts prefix information for SLAAC and learns the router's link-local
/// address as default gateway.
pub fn handle_router_advertisement(pkt: &Ipv6Packet<'_>) {
    let data = pkt.payload;
    // RA: type(1) + code(1) + checksum(2) + cur_hop_limit(1) + flags(1)
    //   + router_lifetime(2) + reachable_time(4) + retrans_timer(4) = 16 min
    if data.len() < 16 {
        return;
    }

    let router_lifetime = ((data[6] as u16) << 8) | data[7] as u16;

    // Learn the router's MAC from Source Link-Layer Address option
    let router_ip = pkt.src;
    if data.len() > 16 {
        parse_ndp_options(&data[16..], |opt_type, opt_data| {
            match opt_type {
                1 => {
                    // Source Link-Layer Address
                    if opt_data.len() >= 6 {
                        let mac = MacAddr([
                            opt_data[0],
                            opt_data[1],
                            opt_data[2],
                            opt_data[3],
                            opt_data[4],
                            opt_data[5],
                        ]);
                        insert(router_ip, mac);
                    }
                }
                3 => {
                    // Prefix Information option (30 bytes data after type+len)
                    handle_prefix_info(opt_data, &router_ip);
                }
                _ => {}
            }
        });
    }

    // Set default gateway if router lifetime > 0
    if router_lifetime > 0 && router_ip.is_link_local() {
        let set_gw = {
            let mut cfg = super::NET_CONFIG.lock();
            if cfg.ipv6_gateway.is_unspecified() {
                cfg.ipv6_gateway = router_ip;
                true
            } else {
                false
            }
        };
        if set_gw {
            crate::serial_verbose_println!("[IPv6] Default gateway set to {}", router_ip);
        }
    }
}

/// Handle a Prefix Information option from a Router Advertisement.
/// Performs SLAAC (RFC 4862) to generate a global IPv6 address.
fn handle_prefix_info(data: &[u8], _router: &Ipv6Addr) {
    // Prefix Info option data (after type+len bytes already stripped):
    //   prefix_len(1), flags(1), valid_lifetime(4), preferred_lifetime(4),
    //   reserved(4), prefix(16) = 30 bytes
    if data.len() < 30 {
        return;
    }

    let prefix_len = data[0];
    let flags = data[1];
    let _valid_lifetime = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
    let _preferred_lifetime = u32::from_be_bytes([data[6], data[7], data[8], data[9]]);

    // A flag (autonomous address-configuration)
    let autonomous = flags & 0x40 != 0;
    if !autonomous {
        return;
    }

    // Only handle /64 prefixes for SLAAC
    if prefix_len != 64 {
        return;
    }

    let mut prefix = [0u8; 16];
    prefix.copy_from_slice(&data[14..30]);

    // Generate address via EUI-64 from MAC
    let cfg = super::config();
    let mac = cfg.mac;

    let mut addr = prefix;
    // EUI-64: insert ff:fe, flip U/L bit
    addr[8] = mac.0[0] ^ 0x02;
    addr[9] = mac.0[1];
    addr[10] = mac.0[2];
    addr[11] = 0xff;
    addr[12] = 0xfe;
    addr[13] = mac.0[3];
    addr[14] = mac.0[4];
    addr[15] = mac.0[5];

    let new_addr = Ipv6Addr(addr);

    // Set as our global address if we don't have one yet
    let configured = {
        let mut net_cfg = super::NET_CONFIG.lock();
        if net_cfg.ipv6_addr.is_unspecified() {
            net_cfg.ipv6_addr = new_addr;
            net_cfg.ipv6_prefix_len = prefix_len;
            true
        } else {
            false
        }
    };
    if configured {
        crate::serial_verbose_println!("[IPv6] SLAAC: configured global address {}", new_addr);
    }
}

/// Send a Router Solicitation to discover routers on the link.
/// Used during IPv6 initialization to trigger Router Advertisements.
pub fn send_router_solicitation() {
    let cfg = super::config();
    let src = cfg.ipv6_link_local;
    if src.is_unspecified() {
        return;
    }

    let mut icmp = Vec::with_capacity(16);
    // Type: Router Solicitation (133)
    icmp.push(icmpv6::ICMPV6_ROUTER_SOLICITATION);
    // Code: 0
    icmp.push(0);
    // Checksum placeholder
    icmp.push(0);
    icmp.push(0);
    // Reserved (4 bytes)
    icmp.push(0);
    icmp.push(0);
    icmp.push(0);
    icmp.push(0);

    // Option: Source Link-Layer Address
    icmp.push(1); // Type
    icmp.push(1); // Length (8 bytes)
    icmp.extend_from_slice(&cfg.mac.0);

    let dst = Ipv6Addr::ALL_ROUTERS;
    icmpv6::compute_icmpv6_checksum(&mut icmp, &src, &dst);
    super::ipv6::send_ipv6(dst, super::ipv6::PROTO_ICMPV6, &icmp);
}

/// Parse NDP options (TLV format). Calls `callback(type, value_data)` for each option.
fn parse_ndp_options(data: &[u8], mut callback: impl FnMut(u8, &[u8])) {
    let mut off = 0;
    while off + 2 <= data.len() {
        let opt_type = data[off];
        let opt_len = data[off + 1] as usize; // In units of 8 bytes
        if opt_len == 0 {
            break;
        } // Prevent infinite loop
        let opt_total = opt_len * 8;
        if off + opt_total > data.len() {
            break;
        }
        // Value starts after type(1) + len(1), length is opt_total - 2
        callback(opt_type, &data[off + 2..off + opt_total]);
        off += opt_total;
    }
}
