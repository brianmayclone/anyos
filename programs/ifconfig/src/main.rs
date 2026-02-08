#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    // Net config: 24 bytes [ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1]
    let mut buf = [0u8; 24];
    let ret = anyos_std::net::get_config(&mut buf);

    if ret != 0 {
        anyos_std::println!("ifconfig: Failed to get network config");
        return;
    }

    let ip = &buf[0..4];
    let mask = &buf[4..8];
    let gw = &buf[8..12];
    let dns = &buf[12..16];
    let mac = &buf[16..22];
    let link = buf[22];

    anyos_std::println!("eth0:");
    anyos_std::println!(
        "  MAC      : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
    anyos_std::println!("  Link     : {}", if link != 0 { "UP" } else { "DOWN" });
    anyos_std::println!(
        "  IPv4     : {}.{}.{}.{}",
        ip[0], ip[1], ip[2], ip[3]
    );
    anyos_std::println!(
        "  Netmask  : {}.{}.{}.{}",
        mask[0], mask[1], mask[2], mask[3]
    );
    anyos_std::println!(
        "  Gateway  : {}.{}.{}.{}",
        gw[0], gw[1], gw[2], gw[3]
    );
    anyos_std::println!(
        "  DNS      : {}.{}.{}.{}",
        dns[0], dns[1], dns[2], dns[3]
    );
}
