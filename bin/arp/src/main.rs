#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    // Each ARP entry is 12 bytes: [ip:4, mac:6, pad:2]
    let mut buf = [0u8; 12 * 32]; // max 32 entries
    let count = anyos_std::net::arp(&mut buf);

    if count == u32::MAX {
        anyos_std::println!("arp: Failed to get ARP table");
        return;
    }

    if count == 0 {
        anyos_std::println!("ARP table is empty");
        return;
    }

    anyos_std::println!("{:<18} {}", "IP Address", "MAC Address");
    anyos_std::println!("{}", "--------------------------------------");

    for i in 0..count as usize {
        let entry = &buf[i * 12..(i + 1) * 12];
        let ip = &entry[0..4];
        let mac = &entry[4..10];

        anyos_std::println!(
            "{:<3}.{:<3}.{:<3}.{:<3}    {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            ip[0], ip[1], ip[2], ip[3],
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
    }

    anyos_std::println!("\n{} entries", count);
}
