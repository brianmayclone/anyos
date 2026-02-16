#![no_std]
#![no_main]

anyos_std::entry!(main);

fn read_all(fd: u32) -> (anyos_std::Vec<u8>, usize) {
    let mut file_buf = anyos_std::vec![0u8; 64 * 1024];
    let mut total: usize = 0;
    let mut read_buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        let n = n as usize;
        if total + n > file_buf.len() { break; }
        file_buf[total..total + n].copy_from_slice(&read_buf[..n]);
        total += n;
    }
    (file_buf, total)
}

fn parse_num(s: &str) -> u32 {
    let mut n: u32 = 0;
    for &b in s.as_bytes() {
        if b >= b'0' && b <= b'9' {
            n = n * 10 + (b - b'0') as u32;
        }
    }
    n
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
            anyos_std::println!("tail: cannot open '{}'", path);
            return;
        }
        f
    } else {
        0 // stdin
    };

    let (file_buf, total) = read_all(fd);
    if fd != 0 { anyos_std::fs::close(fd); }

    let data = &file_buf[..total];

    if let Some(c_val) = byte_mode {
        let max_bytes = parse_num(c_val);
        let n = if max_bytes == 0 { 512 } else { max_bytes as usize };
        let start = if total > n { total - n } else { 0 };
        if let Ok(s) = core::str::from_utf8(&data[start..]) {
            anyos_std::print!("{}", s);
        }
        return;
    }

    // Count newlines from end to find start position
    let mut line_count: u32 = 0;
    let mut start = 0;
    for i in (0..total).rev() {
        if data[i] == b'\n' {
            line_count += 1;
            if line_count >= max_lines + 1 {
                start = i + 1;
                break;
            }
        }
    }

    if let Ok(s) = core::str::from_utf8(&data[start..]) {
        anyos_std::print!("{}", s);
    }
}
