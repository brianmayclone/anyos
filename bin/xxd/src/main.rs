#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"ls");

    let limit = args.opt_u32(b'l', 0);
    let skip = args.opt_u32(b's', 0);

    let fd = if args.pos_count > 0 {
        let path = args.positional[0];
        let f = anyos_std::fs::open(path, 0);
        if f == u32::MAX {
            anyos_std::println!("xxd: cannot open '{}'", path);
            return;
        }
        f
    } else {
        0 // stdin
    };

    // Skip bytes
    if skip > 0 && fd != 0 {
        anyos_std::fs::lseek(fd, skip as i32, 0);
    } else if skip > 0 {
        // stdin: read and discard
        let mut discard = [0u8; 512];
        let mut skipped: u32 = 0;
        while skipped < skip {
            let n = anyos_std::fs::read(fd, &mut discard);
            if n == 0 || n == u32::MAX { break; }
            skipped += n;
        }
    }

    let hex_chars: &[u8; 16] = b"0123456789abcdef";
    let mut offset: u32 = skip;
    let mut read_buf = [0u8; 16];
    let mut remaining = if limit > 0 { limit } else { u32::MAX };

    loop {
        if remaining == 0 { break; }
        let to_read = if remaining < 16 { remaining as usize } else { 16 };
        let n = anyos_std::fs::read(fd, &mut read_buf[..to_read]);
        if n == 0 || n == u32::MAX { break; }
        let n = n as usize;

        anyos_std::print!("{:08x}: ", offset);

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
        if remaining != u32::MAX {
            remaining -= n as u32;
        }
    }

    if fd != 0 { anyos_std::fs::close(fd); }
}
