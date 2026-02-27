#![no_std]
#![no_main]

use anyos_std::net;
use anyos_std::println;

anyos_std::entry!(main);

/// Read interface configs from the kernel and configure the network accordingly.
/// For each interface: DHCP runs discovery, static applies the saved addresses.
fn main() {
    let mut iface_buf = [0u8; 512];
    let count = net::get_interfaces(&mut iface_buf);

    if count == 0 || count == u32::MAX {
        // No interfaces file or empty — fall back to plain DHCP
        run_dhcp();
        return;
    }

    for i in 0..count as usize {
        let off = i * 64;
        let method = iface_buf[off];
        let name_len = (iface_buf[off + 1] as usize).min(16);
        let name = core::str::from_utf8(&iface_buf[off + 2..off + 2 + name_len]).unwrap_or("?");

        match method {
            0 => {
                // DHCP
                println!("NET: {} — DHCP discovery...", name);
                run_dhcp();
            }
            1 => {
                // Static
                let ip = &iface_buf[off + 18..off + 22];
                let mask = &iface_buf[off + 22..off + 26];
                let gw = &iface_buf[off + 26..off + 30];
                let dns = &iface_buf[off + 30..off + 34];

                println!("NET: {} — static configuration", name);

                // Apply via set_config syscall
                let mut cfg = [0u8; 16];
                cfg[0..4].copy_from_slice(ip);
                cfg[4..8].copy_from_slice(mask);
                cfg[8..12].copy_from_slice(gw);
                cfg[12..16].copy_from_slice(dns);
                net::set_config(&cfg);

                println!("  IP Address : {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                println!("  Subnet Mask: {}.{}.{}.{}", mask[0], mask[1], mask[2], mask[3]);
                println!("  Gateway    : {}.{}.{}.{}", gw[0], gw[1], gw[2], gw[3]);
                println!("  DNS Server : {}.{}.{}.{}", dns[0], dns[1], dns[2], dns[3]);
            }
            _ => {
                println!("NET: {} — unknown method {}, skipping", name, method);
            }
        }
    }
}

/// Run DHCP discovery and print the result.
fn run_dhcp() {
    let mut result = [0u8; 16];
    let ret = net::dhcp(&mut result);

    if ret != 0 {
        println!("DHCP: Failed (error {})", ret);
        return;
    }

    let ip = &result[0..4];
    let mask = &result[4..8];
    let gw = &result[8..12];
    let dns = &result[12..16];

    println!("DHCP: Configuration received:");
    println!("  IP Address : {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    println!("  Subnet Mask: {}.{}.{}.{}", mask[0], mask[1], mask[2], mask[3]);
    println!("  Gateway    : {}.{}.{}.{}", gw[0], gw[1], gw[2], gw[3]);
    println!("  DNS Server : {}.{}.{}.{}", dns[0], dns[1], dns[2], dns[3]);
}
