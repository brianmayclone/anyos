#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    // Read interface configs (up to 8 interfaces, 64 bytes each)
    let mut iface_buf = [0u8; 512];
    let iface_count = anyos_std::net::get_interfaces(&mut iface_buf);

    // Read active NIC config: 24 bytes [ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1]
    let mut nic_buf = [0u8; 24];
    let nic_ok = anyos_std::net::get_config(&mut nic_buf) == 0;

    let nic_ip = &nic_buf[0..4];
    let nic_mask = &nic_buf[4..8];
    let nic_gw = &nic_buf[8..12];
    let nic_dns = &nic_buf[12..16];
    let nic_mac = &nic_buf[16..22];
    let nic_link = nic_buf[22];

    if iface_count > 0 && iface_count != u32::MAX {
        for i in 0..iface_count as usize {
            let off = i * 64;
            let method = iface_buf[off];
            let name_len = (iface_buf[off + 1] as usize).min(16);
            let name = core::str::from_utf8(&iface_buf[off + 2..off + 2 + name_len])
                .unwrap_or("?");

            if i > 0 {
                anyos_std::println!("");
            }

            if method == 2 {
                // Loopback interface
                let ip = &iface_buf[off + 18..off + 22];
                let mask = &iface_buf[off + 22..off + 26];
                anyos_std::println!("{}:", name);
                anyos_std::println!("  Link     : UP");
                anyos_std::println!(
                    "  IPv4     : {}.{}.{}.{}",
                    ip[0], ip[1], ip[2], ip[3]
                );
                anyos_std::println!(
                    "  Netmask  : {}.{}.{}.{}",
                    mask[0], mask[1], mask[2], mask[3]
                );
            } else if nic_ok {
                // Physical interface — use active NIC config
                anyos_std::println!("{}:", name);
                anyos_std::println!(
                    "  MAC      : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    nic_mac[0], nic_mac[1], nic_mac[2], nic_mac[3], nic_mac[4], nic_mac[5]
                );
                anyos_std::println!("  Link     : {}", if nic_link != 0 { "UP" } else { "DOWN" });
                anyos_std::println!(
                    "  IPv4     : {}.{}.{}.{}",
                    nic_ip[0], nic_ip[1], nic_ip[2], nic_ip[3]
                );
                anyos_std::println!(
                    "  Netmask  : {}.{}.{}.{}",
                    nic_mask[0], nic_mask[1], nic_mask[2], nic_mask[3]
                );
                anyos_std::println!(
                    "  Gateway  : {}.{}.{}.{}",
                    nic_gw[0], nic_gw[1], nic_gw[2], nic_gw[3]
                );
                anyos_std::println!(
                    "  DNS      : {}.{}.{}.{}",
                    nic_dns[0], nic_dns[1], nic_dns[2], nic_dns[3]
                );
            }
        }
    } else if nic_ok {
        // Fallback: no interfaces file, just show NIC config
        anyos_std::println!("eth0:");
        anyos_std::println!(
            "  MAC      : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            nic_mac[0], nic_mac[1], nic_mac[2], nic_mac[3], nic_mac[4], nic_mac[5]
        );
        anyos_std::println!("  Link     : {}", if nic_link != 0 { "UP" } else { "DOWN" });
        anyos_std::println!(
            "  IPv4     : {}.{}.{}.{}",
            nic_ip[0], nic_ip[1], nic_ip[2], nic_ip[3]
        );
        anyos_std::println!(
            "  Netmask  : {}.{}.{}.{}",
            nic_mask[0], nic_mask[1], nic_mask[2], nic_mask[3]
        );
        anyos_std::println!(
            "  Gateway  : {}.{}.{}.{}",
            nic_gw[0], nic_gw[1], nic_gw[2], nic_gw[3]
        );
        anyos_std::println!(
            "  DNS      : {}.{}.{}.{}",
            nic_dns[0], nic_dns[1], nic_dns[2], nic_dns[3]
        );
    } else {
        anyos_std::println!("ifconfig: Failed to get network config");
    }
}
