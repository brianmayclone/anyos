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

    // Parse: head [-n N] <file>
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
        anyos_std::println!("Usage: head [-n N] <file>");
        return;
    }

    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("head: cannot open '{}'", path);
        return;
    }

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

    anyos_std::fs::close(fd);
}
