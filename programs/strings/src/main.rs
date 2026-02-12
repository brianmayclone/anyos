#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    let mut min_len: usize = 4;
    let mut path = "";

    let parts: alloc::vec::Vec<&str> = args.split_ascii_whitespace().collect();
    let mut i = 0;
    while i < parts.len() {
        if parts[i] == "-n" && i + 1 < parts.len() {
            // Parse min length
            let mut val: usize = 0;
            for &b in parts[i + 1].as_bytes() {
                if b >= b'0' && b <= b'9' {
                    val = val * 10 + (b - b'0') as usize;
                }
            }
            if val > 0 { min_len = val; }
            i += 2;
        } else {
            path = parts[i];
            i += 1;
        }
    }

    if path.is_empty() {
        anyos_std::println!("Usage: strings [-n MIN] FILE");
        return;
    }

    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("strings: cannot open '{}'", path);
        return;
    }

    // Read file in chunks, extract printable ASCII runs
    let mut read_buf = [0u8; 512];
    let mut current = [0u8; 1024];
    let mut cur_len: usize = 0;

    loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        for i in 0..n as usize {
            let b = read_buf[i];
            if b >= 0x20 && b < 0x7F {
                if cur_len < current.len() {
                    current[cur_len] = b;
                    cur_len += 1;
                }
            } else {
                if cur_len >= min_len {
                    if let Ok(s) = core::str::from_utf8(&current[..cur_len]) {
                        anyos_std::println!("{}", s);
                    }
                }
                cur_len = 0;
            }
        }
    }
    // Flush remaining
    if cur_len >= min_len {
        if let Ok(s) = core::str::from_utf8(&current[..cur_len]) {
            anyos_std::println!("{}", s);
        }
    }

    anyos_std::fs::close(fd);
}
