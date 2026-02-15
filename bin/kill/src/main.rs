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

    if args.is_empty() {
        anyos_std::println!("Usage: kill <tid>");
        return;
    }

    match parse_u32(args.trim()) {
        Some(tid) => {
            let result = anyos_std::process::kill(tid);
            if result == 0 {
                anyos_std::println!("Killed thread {}", tid);
            } else {
                anyos_std::println!("Failed to kill thread {} (not found or permission denied)", tid);
            }
        }
        None => {
            anyos_std::println!("Invalid TID: {}", args);
        }
    }
}
