#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{net, process, println};

const ENTRY_SIZE: usize = 128;

fn main() {
    let mut args_buf = [0u8; 256];
    let args = process::args(&mut args_buf);

    if args.is_empty() || args == "show" {
        show_routes();
        return;
    }
    if args == "--help" || args == "-h" {
        print_usage();
        return;
    }

    let parts: anyos_std::Vec<&str> = args.split_ascii_whitespace().collect();
    if parts.len() == 3 && parts[0] == "dns" {
        if let Some(ip) = parse_ipv4(parts[2]) {
            set_dns(ip);
            return;
        }
    }
    if parts.len() == 3 && parts[0] == "default" && parts[1] == "via" {
        if let Some(ip) = parse_ipv4(parts[2]) {
            set_default_gateway(ip);
            return;
        }
    }

    print_usage();
}

fn show_routes() {
    let mut cfg = [0u8; 24];
    if net::get_config(&mut cfg) != 0 {
        println!("route: no active network configuration");
        return;
    }

    let ip = &cfg[0..4];
    let mask = &cfg[4..8];
    let gw = &cfg[8..12];
    let dns = &cfg[12..16];

    println!("Kernel IPv4 routing state");
    println!("Destination      Gateway          Netmask          Iface");
    println!(
        "{}.{}.{}.{}      0.0.0.0          {}.{}.{}.{}      eth0",
        ip[0] & mask[0], ip[1] & mask[1], ip[2] & mask[2], ip[3] & mask[3],
        mask[0], mask[1], mask[2], mask[3]
    );
    println!(
        "0.0.0.0          {}.{}.{}.{}      0.0.0.0          eth0",
        gw[0], gw[1], gw[2], gw[3]
    );
    println!(
        "DNS server       {}.{}.{}.{}",
        dns[0], dns[1], dns[2], dns[3]
    );
}

fn set_default_gateway(ip: [u8; 4]) {
    let mut cfg = [0u8; 24];
    if net::get_config(&mut cfg) != 0 {
        println!("route: failed to read active config");
        return;
    }
    cfg[8..12].copy_from_slice(&ip);
    let mut live = [0u8; 16];
    live.copy_from_slice(&cfg[..16]);
    net::set_config(&live);

    let persisted = update_interface_field(26, &ip);
    println!(
        "route: default gateway set to {}.{}.{}.{}{}",
        ip[0], ip[1], ip[2], ip[3],
        if persisted { "" } else { " (live only; DHCP interface not persisted)" }
    );
}

fn set_dns(ip: [u8; 4]) {
    let mut cfg = [0u8; 24];
    if net::get_config(&mut cfg) != 0 {
        println!("route: failed to read active config");
        return;
    }
    cfg[12..16].copy_from_slice(&ip);
    let mut live = [0u8; 16];
    live.copy_from_slice(&cfg[..16]);
    net::set_config(&live);

    let persisted = update_interface_field(30, &ip);
    println!(
        "route: DNS server set to {}.{}.{}.{}{}",
        ip[0], ip[1], ip[2], ip[3],
        if persisted { "" } else { " (live only; DHCP interface not persisted)" }
    );
}

fn update_interface_field(offset: usize, value: &[u8; 4]) -> bool {
    let mut iface_buf = [0u8; 1024];
    let count = net::get_interfaces(&mut iface_buf) as usize;
    if count == 0 || count == usize::MAX {
        return false;
    }

    let mut target = None;
    for i in 0..count {
        let off = i * ENTRY_SIZE;
        if iface_buf[off] == 2 {
            continue;
        }
        target = Some(off);
        break;
    }

    let off = match target {
        Some(v) => v,
        None => return false,
    };

    if iface_buf[off] != 1 {
        return false;
    }

    iface_buf[off + offset..off + offset + 4].copy_from_slice(value);

    let total = 4 + count * ENTRY_SIZE;
    let mut syscall_buf = [0u8; 4 + 8 * ENTRY_SIZE];
    syscall_buf[0..4].copy_from_slice(&(count as u32).to_le_bytes());
    syscall_buf[4..total].copy_from_slice(&iface_buf[..count * ENTRY_SIZE]);
    net::set_interfaces(&syscall_buf[..total]) == 0
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for octet in &mut out {
        *octet = parse_u8(parts.next()?)?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

fn parse_u8(s: &str) -> Option<u8> {
    let mut val = 0u32;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
        if val > 255 {
            return None;
        }
    }
    Some(val as u8)
}

fn print_usage() {
    println!("route - show or update IPv4 routing");
    println!("");
    println!("Usage: route");
    println!("       route default via <ip>");
    println!("       route dns set <ip>");
}
