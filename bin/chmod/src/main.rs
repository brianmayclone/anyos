#![no_std]
#![no_main]

anyos_std::entry!(main);

fn parse_mode(s: &str) -> Option<u16> {
    if s.starts_with("0x") || s.starts_with("0X") {
        u16::from_str_radix(&s[2..], 16).ok()
    } else {
        // Try decimal
        let mut val: u16 = 0;
        for b in s.bytes() {
            if b < b'0' || b > b'9' {
                return None;
            }
            val = val.checked_mul(10)?.checked_add((b - b'0') as u16)?;
        }
        Some(val)
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count < 2 {
        anyos_std::println!("Usage: chmod <mode> <path>");
        anyos_std::println!("  mode: decimal or hex (0x...) permission value");
        return;
    }

    let mode_str = args.positional[0];
    let path = args.positional[1];

    let mode = match parse_mode(mode_str) {
        Some(m) => m,
        None => {
            anyos_std::println!("chmod: invalid mode '{}'", mode_str);
            return;
        }
    };

    let ret = anyos_std::fs::chmod(path, mode);
    if ret == u32::MAX {
        anyos_std::println!("chmod: failed to change mode of '{}'", path);
    } else {
        anyos_std::println!("chmod: mode of '{}' changed to 0x{:03X}", path, mode);
    }
}
