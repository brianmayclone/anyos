#![no_std]
#![no_main]

anyos_std::entry!(main);

fn to_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

fn parse_leading_int(s: &str) -> i64 {
    let bytes = s.as_bytes();
    let mut i = 0;
    let neg = if !bytes.is_empty() && bytes[0] == b'-' { i = 1; true } else { false };
    let mut n: i64 = 0;
    while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
        n = n * 10 + (bytes[i] - b'0') as i64;
        i += 1;
    }
    if neg { -n } else { n }
}

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

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let reverse = args.has(b'r');
    let numeric = args.has(b'n');
    let unique = args.has(b'u');
    let fold_case = args.has(b'f');

    let fd = if args.pos_count > 0 {
        let path = args.positional[0];
        let f = anyos_std::fs::open(path, 0);
        if f == u32::MAX {
            anyos_std::println!("sort: cannot open '{}'", path);
            return;
        }
        f
    } else {
        0 // stdin
    };

    let (file_buf, total) = read_all(fd);
    if fd != 0 { anyos_std::fs::close(fd); }

    let text = core::str::from_utf8(&file_buf[..total]).unwrap_or("");
    let mut lines: alloc::vec::Vec<&str> = text.lines().collect();

    if numeric {
        lines.sort_unstable_by(|a, b| {
            parse_leading_int(a).cmp(&parse_leading_int(b))
        });
    } else if fold_case {
        lines.sort_unstable_by(|a, b| {
            let ab = a.as_bytes();
            let bb = b.as_bytes();
            let min = if ab.len() < bb.len() { ab.len() } else { bb.len() };
            for i in 0..min {
                let la = to_lower(ab[i]);
                let lb = to_lower(bb[i]);
                if la < lb { return core::cmp::Ordering::Less; }
                if la > lb { return core::cmp::Ordering::Greater; }
            }
            ab.len().cmp(&bb.len())
        });
    } else {
        lines.sort_unstable();
    }

    if reverse {
        lines.reverse();
    }
    if unique {
        lines.dedup();
    }

    for line in &lines {
        anyos_std::println!("{}", line);
    }
}
