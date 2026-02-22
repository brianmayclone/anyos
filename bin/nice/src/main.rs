#![no_std]
#![no_main]

anyos_std::entry!(main);

fn parse_u32(s: &str) -> Option<u32> {
    let mut n: u32 = 0;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' { return None; }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

fn main() {
    let mut buf = [0u8; 256];
    let args = anyos_std::process::args(&mut buf);

    let mut parts = args.trim().split_whitespace();
    let prio_str = match parts.next() {
        Some(s) => s,
        None => {
            anyos_std::println!("Usage: nice <priority> <tid>");
            anyos_std::println!("  priority: 0-127 (0 = lowest, 127 = highest)");
            anyos_std::println!("  tid:      thread ID (use ps to find)");
            return;
        }
    };
    let tid_str = match parts.next() {
        Some(s) => s,
        None => {
            anyos_std::println!("Usage: nice <priority> <tid>");
            return;
        }
    };

    let priority = match parse_u32(prio_str) {
        Some(p) if p <= 127 => p,
        _ => {
            anyos_std::println!("Invalid priority '{}' (must be 0-127)", prio_str);
            return;
        }
    };

    let tid = match parse_u32(tid_str) {
        Some(t) => t,
        None => {
            anyos_std::println!("Invalid TID '{}'", tid_str);
            return;
        }
    };

    let result = anyos_std::process::set_priority(tid, priority as u8);
    if result == 0 {
        anyos_std::println!("Set thread {} priority to {}", tid, priority);
    } else {
        anyos_std::println!("Failed to set priority for thread {}", tid);
    }
}
