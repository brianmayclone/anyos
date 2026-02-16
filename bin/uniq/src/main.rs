#![no_std]
#![no_main]

anyos_std::entry!(main);

fn to_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

fn eq_ci(a: &str, b: &str) -> bool {
    if a.len() != b.len() { return false; }
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    for i in 0..ab.len() {
        if to_lower(ab[i]) != to_lower(bb[i]) { return false; }
    }
    true
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

    let count_mode = args.has(b'c');
    let dups_only = args.has(b'd');
    let ignore_case = args.has(b'i');

    let fd = if args.pos_count > 0 {
        let path = args.positional[0];
        let f = anyos_std::fs::open(path, 0);
        if f == u32::MAX {
            anyos_std::println!("uniq: cannot open '{}'", path);
            return;
        }
        f
    } else {
        0 // stdin
    };

    let (file_buf, total) = read_all(fd);
    if fd != 0 { anyos_std::fs::close(fd); }

    let text = core::str::from_utf8(&file_buf[..total]).unwrap_or("");
    let mut prev = "";
    let mut count: u32 = 0;

    for line in text.lines() {
        let same = if ignore_case { eq_ci(line, prev) } else { line == prev };
        if same {
            count += 1;
        } else {
            if count > 0 {
                let show = !dups_only || count > 1;
                if show {
                    if count_mode {
                        anyos_std::println!("{:>7} {}", count, prev);
                    } else {
                        anyos_std::println!("{}", prev);
                    }
                }
            }
            prev = line;
            count = 1;
        }
    }
    if count > 0 {
        let show = !dups_only || count > 1;
        if show {
            if count_mode {
                anyos_std::println!("{:>7} {}", count, prev);
            } else {
                anyos_std::println!("{}", prev);
            }
        }
    }
}
