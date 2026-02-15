#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    anyos_std::println!("DHCP: Sending DISCOVER...");

    let mut result = [0u8; 16];
    let ret = anyos_std::net::dhcp(&mut result);

    if ret != 0 {
        anyos_std::println!("DHCP: Failed (error {})", ret);
        return;
    }

    let ip = &result[0..4];
    let mask = &result[4..8];
    let gw = &result[8..12];
    let dns = &result[12..16];

    anyos_std::println!("DHCP: Configuration received:");
    anyos_std::println!("  IP Address : {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    anyos_std::println!("  Subnet Mask: {}.{}.{}.{}", mask[0], mask[1], mask[2], mask[3]);
    anyos_std::println!("  Gateway    : {}.{}.{}.{}", gw[0], gw[1], gw[2], gw[3]);
    anyos_std::println!("  DNS Server : {}.{}.{}.{}", dns[0], dns[1], dns[2], dns[3]);
}
