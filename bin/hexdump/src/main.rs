#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"ns");

    // -C is accepted but already default format
    let limit = args.opt_u32(b'n', 0);
    let skip = args.opt_u32(b's', 0);

    let path = args.first_or("");
    if path.is_empty() {
        anyos_std::println!("Usage: hexdump [-C] [-n LEN] [-s SKIP] FILE");
        return;
    }

    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("hexdump: cannot open '{}'", path);
        return;
    }

    // Skip bytes
    if skip > 0 {
        anyos_std::fs::lseek(fd, skip as i32, 0); // SEEK_SET=0
    }

    let mut read_buf = [0u8; 16];
    let mut offset: u32 = skip;
    let mut remaining = if limit > 0 { limit } else { u32::MAX };

    loop {
        if remaining == 0 { break; }
        let to_read = if remaining < 16 { remaining as usize } else { 16 };
        let n = anyos_std::fs::read(fd, &mut read_buf[..to_read]);
        if n == 0 || n == u32::MAX { break; }
        let n = n as usize;

        anyos_std::print!("{:08X}  ", offset);

        for i in 0..16 {
            if i < n {
                anyos_std::print!("{:02X} ", read_buf[i]);
            } else {
                anyos_std::print!("   ");
            }
            if i == 7 { anyos_std::print!(" "); }
        }

        anyos_std::print!(" |");
        for i in 0..n {
            let b = read_buf[i];
            if b >= 0x20 && b < 0x7F {
                anyos_std::print!("{}", b as char);
            } else {
                anyos_std::print!(".");
            }
        }
        anyos_std::println!("|");

        offset += n as u32;
        if remaining != u32::MAX {
            remaining -= n as u32;
        }
    }

    anyos_std::fs::close(fd);
    anyos_std::println!("{:08X}", offset);
}
