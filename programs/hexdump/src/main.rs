#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    if args.is_empty() {
        anyos_std::println!("Usage: hexdump <file>");
        return;
    }

    let path = args.trim();
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("hexdump: cannot open '{}'", path);
        return;
    }

    let mut read_buf = [0u8; 16];
    let mut offset: u32 = 0;

    loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        let n = n as usize;

        // Print offset
        anyos_std::print!("{:08X}  ", offset);

        // Print hex bytes
        for i in 0..16 {
            if i < n {
                anyos_std::print!("{:02X} ", read_buf[i]);
            } else {
                anyos_std::print!("   ");
            }
            if i == 7 { anyos_std::print!(" "); }
        }

        // Print ASCII
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
    }

    anyos_std::fs::close(fd);
    anyos_std::println!("{:08X}", offset);
}
