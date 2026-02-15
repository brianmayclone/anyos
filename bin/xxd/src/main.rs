#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let path = args.trim();

    if path.is_empty() {
        anyos_std::println!("Usage: xxd FILE");
        return;
    }

    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("xxd: cannot open '{}'", path);
        return;
    }

    let hex_chars: &[u8; 16] = b"0123456789abcdef";
    let mut offset: u32 = 0;
    let mut read_buf = [0u8; 16];

    loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        let n = n as usize;

        // Format: XXXXXXXX: XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX  ................
        // Print offset
        anyos_std::print!("{:08x}: ", offset);

        // Hex dump (groups of 2)
        for i in 0..16 {
            if i < n {
                let b = read_buf[i];
                let hi = hex_chars[(b >> 4) as usize] as char;
                let lo = hex_chars[(b & 0x0F) as usize] as char;
                anyos_std::print!("{}{}", hi, lo);
            } else {
                anyos_std::print!("  ");
            }
            if i % 2 == 1 {
                anyos_std::print!(" ");
            }
        }

        // ASCII
        anyos_std::print!(" ");
        for i in 0..n {
            let b = read_buf[i];
            if b >= 0x20 && b < 0x7F {
                anyos_std::print!("{}", b as char);
            } else {
                anyos_std::print!(".");
            }
        }
        anyos_std::println!("");

        offset += n as u32;
    }

    anyos_std::fs::close(fd);
}
