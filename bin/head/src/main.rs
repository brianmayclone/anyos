#![no_std]
#![no_main]

anyos_std::entry!(main);

fn head_lines(fd: u32, max_lines: u32) {
    let mut lines_printed: u32 = 0;
    let mut read_buf = [0u8; 512];
    'outer: loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        for &b in &read_buf[..n as usize] {
            anyos_std::print!("{}", b as char);
            if b == b'\n' {
                lines_printed += 1;
                if lines_printed >= max_lines { break 'outer; }
            }
        }
    }
}

fn head_bytes(fd: u32, max_bytes: u32) {
    let mut printed: u32 = 0;
    let mut read_buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        for &b in &read_buf[..n as usize] {
            anyos_std::print!("{}", b as char);
            printed += 1;
            if printed >= max_bytes { return; }
        }
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"nc");

    let max_lines = args.opt_u32(b'n', 10);
    let byte_mode = args.opt(b'c');

    let fd = if args.pos_count > 0 {
        let path = args.positional[0];
        let f = anyos_std::fs::open(path, 0);
        if f == u32::MAX {
            anyos_std::println!("head: cannot open '{}'", path);
            return;
        }
        f
    } else {
        0 // stdin
    };

    if let Some(c_val) = byte_mode {
        let max_bytes = args.opt_u32(b'c', 512);
        head_bytes(fd, max_bytes);
    } else {
        head_lines(fd, max_lines);
    }

    if fd != 0 {
        anyos_std::fs::close(fd);
    }
}
