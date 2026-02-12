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
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    let mut max_lines: u32 = 10;
    let mut path = "";

    let parts: [&str; 4] = {
        let mut p = [""; 4];
        let mut idx = 0;
        for word in args.split_ascii_whitespace() {
            if idx < 4 { p[idx] = word; idx += 1; }
        }
        p
    };

    if parts[0] == "-n" && !parts[1].is_empty() {
        if let Some(n) = parse_u32(parts[1]) { max_lines = n; }
        path = parts[2];
    } else {
        path = parts[0];
    }

    if path.is_empty() {
        anyos_std::println!("Usage: tail [-n N] <file>");
        return;
    }

    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("tail: cannot open '{}'", path);
        return;
    }

    // Read entire file into memory (limited to 64 KiB)
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
    anyos_std::fs::close(fd);

    // Count newlines from end
    let data = &file_buf[..total];
    let mut line_count: u32 = 0;
    let mut start = total;
    for i in (0..total).rev() {
        if data[i] == b'\n' {
            line_count += 1;
            if line_count >= max_lines + 1 {
                start = i + 1;
                break;
            }
        }
    }
    if line_count < max_lines + 1 {
        start = 0;
    }

    // Print from start to end
    let output = core::str::from_utf8(&data[start..total]).unwrap_or("");
    anyos_std::print!("{}", output);
}
