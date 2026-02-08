#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let args_buf = &mut [0u8; 256];
    let args_len = anyos_std::process::getargs(args_buf);
    let args = core::str::from_utf8(&args_buf[..args_len]).unwrap_or("");

    if args.is_empty() {
        anyos_std::println!("Usage: ping <ip>");
        anyos_std::println!("  Example: ping 10.0.2.2");
        return;
    }

    // Parse IP address from args
    let ip = match parse_ipv4(args.trim()) {
        Some(ip) => ip,
        None => {
            anyos_std::println!("Invalid IP address: {}", args);
            return;
        }
    };

    anyos_std::println!(
        "PING {}.{}.{}.{} — 4 packets",
        ip[0], ip[1], ip[2], ip[3]
    );

    let mut sent = 0u32;
    let mut received = 0u32;

    for seq in 0..4u32 {
        sent += 1;
        let rtt = anyos_std::net::ping(&ip, seq, 500);
        if rtt == u32::MAX {
            anyos_std::println!(
                "  seq={}: Request timed out",
                seq
            );
        } else {
            received += 1;
            // RTT is in PIT ticks (100 Hz = 10ms per tick)
            let ms = rtt * 10;
            anyos_std::println!(
                "  seq={}: Reply from {}.{}.{}.{} time={}ms",
                seq, ip[0], ip[1], ip[2], ip[3], ms
            );
        }
        // Wait between pings
        if seq < 3 {
            anyos_std::process::sleep(1000);
        }
    }

    let lost = sent - received;
    anyos_std::println!(
        "--- {} packets transmitted, {} received, {} lost ---",
        sent, received, lost
    );
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut ip = [0u8; 4];
    let mut idx = 0;
    for part in s.split('.') {
        if idx >= 4 {
            return None;
        }
        let val: u32 = {
            let mut n = 0u32;
            for b in part.bytes() {
                if b < b'0' || b > b'9' {
                    return None;
                }
                n = n * 10 + (b - b'0') as u32;
            }
            n
        };
        if val > 255 {
            return None;
        }
        ip[idx] = val as u8;
        idx += 1;
    }
    if idx == 4 { Some(ip) } else { None }
}
