#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    if args.is_empty() {
        anyos_std::println!("Usage: ping <host>");
        anyos_std::println!("  Example: ping 10.0.2.2");
        anyos_std::println!("  Example: ping google.com");
        return;
    }

    let target = args.trim();

    // Try parsing as IP address first, then fall back to DNS resolution
    let ip = match parse_ipv4(target) {
        Some(ip) => ip,
        None => {
            // Not a valid IP — try DNS resolution
            let mut resolved = [0u8; 4];
            let ret = anyos_std::net::dns(target, &mut resolved);
            if ret != 0 {
                anyos_std::println!("ping: cannot resolve {}: DNS lookup failed", target);
                return;
            }
            anyos_std::println!(
                "PING {} ({}.{}.{}.{})",
                target, resolved[0], resolved[1], resolved[2], resolved[3]
            );
            resolved
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
            // RTT is in PIT ticks; convert to milliseconds
            let hz = anyos_std::sys::tick_hz();
            let ms = if hz > 0 { rtt * 1000 / hz } else { 0 };
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
