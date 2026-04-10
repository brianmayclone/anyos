#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("ifconfig - Display network interface configuration\n");
        anyos_std::println!("Usage: ifconfig [interface]");
        return;
    }

    // Optional: filter by interface name (positional[0] is program name)
    let filter = {
        let args = anyos_std::args::parse(raw, b"");
        if args.pos_count > 1 {
            let mut buf = [0u8; 32];
            let len = args.positional[1].len().min(32);
            buf[..len].copy_from_slice(&args.positional[1].as_bytes()[..len]);
            Some((buf, len))
        } else {
            None
        }
    };

    // Read interface configs (up to 8 interfaces, 128 bytes each)
    let mut iface_buf = [0u8; 1024];
    let iface_count = anyos_std::net::get_interfaces(&mut iface_buf);

    // Read active NIC config: 24 bytes [ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1]
    let mut nic_buf = [0u8; 24];
    let nic_ok = anyos_std::net::get_config(&mut nic_buf) == 0;

    let nic_ip = [nic_buf[0], nic_buf[1], nic_buf[2], nic_buf[3]];
    let nic_mask = [nic_buf[4], nic_buf[5], nic_buf[6], nic_buf[7]];
    let nic_gw = [nic_buf[8], nic_buf[9], nic_buf[10], nic_buf[11]];
    let nic_dns = [nic_buf[12], nic_buf[13], nic_buf[14], nic_buf[15]];
    let nic_mac = [nic_buf[16], nic_buf[17], nic_buf[18], nic_buf[19], nic_buf[20], nic_buf[21]];
    let nic_link = nic_buf[22];

    // Read IPv6 config
    let mut ipv6_buf = [0u8; 80];
    let ipv6_ok = anyos_std::net::get_ipv6_config(&mut ipv6_buf) == 0;

    // Read NIC driver name
    let mut driver_buf = [0u8; 64];
    let driver_len = anyos_std::net::nic_driver_name(&mut driver_buf);

    // Read network statistics
    let stats = anyos_std::net::net_stats();

    if iface_count > 0 && iface_count != u32::MAX {
        let mut printed = false;
        for i in 0..iface_count as usize {
            let off = i * 128;
            let method = iface_buf[off];
            let name_len = (iface_buf[off + 1] as usize).min(16);
            let name = core::str::from_utf8(&iface_buf[off + 2..off + 2 + name_len])
                .unwrap_or("?");

            // Apply filter if specified
            if let Some((ref fbuf, flen)) = filter {
                let fstr = core::str::from_utf8(&fbuf[..flen]).unwrap_or("");
                if name != fstr {
                    continue;
                }
            }

            if printed {
                anyos_std::println!("");
            }
            printed = true;

            if method == 2 {
                // Loopback interface
                print_loopback(name, &iface_buf[off..]);
            } else if nic_ok {
                // Physical interface
                let driver_name = if driver_len > 0 {
                    core::str::from_utf8(&driver_buf[..driver_len as usize]).unwrap_or("")
                } else {
                    ""
                };
                print_physical(
                    name,
                    driver_name,
                    method,
                    &nic_mac,
                    nic_link,
                    &nic_ip,
                    &nic_mask,
                    &nic_gw,
                    &nic_dns,
                    if ipv6_ok { Some(&ipv6_buf) } else { None },
                    stats.as_ref(),
                );
            }
        }
        if !printed {
            if let Some((ref fbuf, flen)) = filter {
                let fstr = core::str::from_utf8(&fbuf[..flen]).unwrap_or("");
                anyos_std::println!("ifconfig: interface '{}' not found", fstr);
            }
        }
    } else if nic_ok {
        // Fallback: no interfaces file, show NIC config as eth0
        let driver_name = if driver_len > 0 {
            core::str::from_utf8(&driver_buf[..driver_len as usize]).unwrap_or("")
        } else {
            ""
        };
        print_physical(
            "eth0",
            driver_name,
            0,
            &nic_mac,
            nic_link,
            &nic_ip,
            &nic_mask,
            &nic_gw,
            &nic_dns,
            if ipv6_ok { Some(&ipv6_buf) } else { None },
            stats.as_ref(),
        );
    } else {
        anyos_std::println!("ifconfig: no network interfaces found");
    }
}

fn print_loopback(name: &str, entry: &[u8]) {
    let ip = &entry[18..22];
    let mask = &entry[22..26];
    let ipv6_prefix = entry[34];
    let ipv6_addr = &entry[37..53];

    anyos_std::println!("{}: flags=73<UP,LOOPBACK,RUNNING>  mtu 65536", name);
    anyos_std::println!(
        "        inet {}.{}.{}.{}  netmask {}.{}.{}.{}",
        ip[0], ip[1], ip[2], ip[3],
        mask[0], mask[1], mask[2], mask[3],
    );

    // IPv6 loopback (::1)
    if is_ipv6_set(ipv6_addr) {
        anyos_std::print!("        inet6 ");
        print_ipv6(ipv6_addr);
        anyos_std::println!("  prefixlen {}", ipv6_prefix);
    } else {
        anyos_std::println!("        inet6 ::1  prefixlen 128  scope host");
    }

    anyos_std::println!("        loop  txqueuelen 1000");
}

fn print_physical(
    name: &str,
    driver: &str,
    method: u8,
    mac: &[u8; 6],
    link: u8,
    ip: &[u8; 4],
    mask: &[u8; 4],
    gw: &[u8; 4],
    dns: &[u8; 4],
    ipv6: Option<&[u8; 80]>,
    stats: Option<&anyos_std::net::NetStats>,
) {
    let up = link != 0;
    let method_str = match method {
        0 => "DHCP",
        1 => "STATIC",
        _ => "UNKNOWN",
    };

    // Flags line
    if up {
        anyos_std::println!(
            "{}: flags=4163<UP,BROADCAST,RUNNING,MULTICAST>  mtu 1500",
            name,
        );
    } else {
        anyos_std::println!(
            "{}: flags=4098<BROADCAST,MULTICAST>  mtu 1500",
            name,
        );
    }

    // IPv4
    let prefix = netmask_to_prefix(mask);
    anyos_std::println!(
        "        inet {}.{}.{}.{}  netmask {}.{}.{}.{}  broadcast {}.{}.{}.{}",
        ip[0], ip[1], ip[2], ip[3],
        mask[0], mask[1], mask[2], mask[3],
        broadcast_addr(ip, mask, 0), broadcast_addr(ip, mask, 1),
        broadcast_addr(ip, mask, 2), broadcast_addr(ip, mask, 3),
    );

    // IPv6
    if let Some(v6) = ipv6 {
        let link_local = &v6[0..16];
        let global = &v6[16..32];
        let prefix_len = v6[32];
        let _gw6 = &v6[35..51];
        let _dns6 = &v6[51..67];

        if is_ipv6_set(global) {
            anyos_std::print!("        inet6 ");
            print_ipv6(global);
            anyos_std::println!("  prefixlen {}  scope global", prefix_len);
        }
        if is_ipv6_set(link_local) {
            anyos_std::print!("        inet6 ");
            print_ipv6(link_local);
            anyos_std::println!("  prefixlen 64  scope link");
        }
    }

    // Ethernet / MAC
    anyos_std::println!(
        "        ether {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  txqueuelen 1000",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
    );

    // RX/TX statistics
    if let Some(s) = stats {
        let (rx_val, rx_unit) = human_bytes(s.rx_bytes);
        let (tx_val, tx_unit) = human_bytes(s.tx_bytes);
        anyos_std::println!(
            "        RX packets {}  bytes {} ({} {})",
            s.rx_packets, s.rx_bytes, rx_val, rx_unit,
        );
        anyos_std::println!(
            "        RX errors {}  dropped 0  overruns 0",
            s.rx_errors,
        );
        anyos_std::println!(
            "        TX packets {}  bytes {} ({} {})",
            s.tx_packets, s.tx_bytes, tx_val, tx_unit,
        );
        anyos_std::println!(
            "        TX errors {}  dropped 0  overruns 0",
            s.tx_errors,
        );
    }

    // Extra info
    if !driver.is_empty() {
        anyos_std::println!("        device {}  method {}", driver, method_str);
    }
    if ip[0] != 0 {
        anyos_std::println!(
            "        gateway {}.{}.{}.{}  dns {}.{}.{}.{}",
            gw[0], gw[1], gw[2], gw[3],
            dns[0], dns[1], dns[2], dns[3],
        );
    }
}

fn is_ipv6_set(addr: &[u8]) -> bool {
    addr.iter().any(|&b| b != 0)
}

fn print_ipv6(addr: &[u8]) {
    // Find longest run of zero 16-bit groups for :: compression
    let mut groups = [0u16; 8];
    for i in 0..8 {
        groups[i] = ((addr[i * 2] as u16) << 8) | (addr[i * 2 + 1] as u16);
    }

    // Find best zero-run for :: compression
    let mut best_start = 8usize;
    let mut best_len = 0usize;
    let mut cur_start = 0usize;
    let mut cur_len = 0usize;
    for i in 0..8 {
        if groups[i] == 0 {
            if cur_len == 0 {
                cur_start = i;
            }
            cur_len += 1;
        } else {
            if cur_len > best_len && cur_len >= 2 {
                best_start = cur_start;
                best_len = cur_len;
            }
            cur_len = 0;
        }
    }
    if cur_len > best_len && cur_len >= 2 {
        best_start = cur_start;
        best_len = cur_len;
    }

    let mut first = true;
    let mut i = 0;
    while i < 8 {
        if i == best_start && best_len > 0 {
            if i == 0 {
                anyos_std::print!("::");
            } else {
                anyos_std::print!("::");
            }
            i += best_len;
            first = i >= 8;
            continue;
        }
        if !first {
            anyos_std::print!(":");
        }
        anyos_std::print!("{:x}", groups[i]);
        first = false;
        i += 1;
    }
}

fn netmask_to_prefix(mask: &[u8; 4]) -> u32 {
    let val = ((mask[0] as u32) << 24) | ((mask[1] as u32) << 16)
        | ((mask[2] as u32) << 8) | (mask[3] as u32);
    val.count_ones()
}

fn broadcast_addr(ip: &[u8; 4], mask: &[u8; 4], idx: usize) -> u8 {
    ip[idx] | !mask[idx]
}

fn human_bytes(bytes: u64) -> (u64, &'static str) {
    if bytes >= 1_073_741_824 {
        (bytes / 1_073_741_824, "GiB")
    } else if bytes >= 1_048_576 {
        (bytes / 1_048_576, "MiB")
    } else if bytes >= 1024 {
        (bytes / 1024, "KiB")
    } else {
        (bytes, "B")
    }
}
