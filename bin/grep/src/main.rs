#![no_std]
#![no_main]

anyos_std::entry!(main);

fn to_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

fn find_needle(haystack: &[u8], needle: &[u8], ignore_case: bool) -> Option<usize> {
    if needle.is_empty() { return Some(0); }
    if needle.len() > haystack.len() { return None; }
    for i in 0..=haystack.len() - needle.len() {
        let mut ok = true;
        for j in 0..needle.len() {
            let a = if ignore_case { to_lower(haystack[i + j]) } else { haystack[i + j] };
            let b = if ignore_case { to_lower(needle[j]) } else { needle[j] };
            if a != b { ok = false; break; }
        }
        if ok { return Some(i); }
    }
    None
}

fn is_word_char(b: u8) -> bool {
    (b >= b'a' && b <= b'z') || (b >= b'A' && b <= b'Z') || (b >= b'0' && b <= b'9') || b == b'_'
}

fn matches_line(line: &[u8], needle: &[u8], ignore_case: bool, whole_word: bool) -> bool {
    if !whole_word {
        return find_needle(line, needle, ignore_case).is_some();
    }
    // Word boundary check
    let mut start = 0;
    loop {
        match find_needle(&line[start..], needle, ignore_case) {
            None => return false,
            Some(pos) => {
                let abs = start + pos;
                let before_ok = abs == 0 || !is_word_char(line[abs - 1]);
                let after_ok = abs + needle.len() >= line.len() || !is_word_char(line[abs + needle.len()]);
                if before_ok && after_ok { return true; }
                start = abs + 1;
                if start + needle.len() > line.len() { return false; }
            }
        }
    }
}

fn read_file(fd: u32) -> anyos_std::Vec<u8> {
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
    file_buf.truncate(total);
    file_buf
}

fn grep_data(data: &[u8], needle: &[u8], ignore_case: bool, invert: bool,
             show_num: bool, count_only: bool, list_only: bool, whole_word: bool,
             prefix: &str) -> bool {
    let mut match_count: u32 = 0;
    let mut line_no: u32 = 0;

    for line in data.split(|&b| b == b'\n') {
        line_no += 1;
        let mut hit = matches_line(line, needle, ignore_case, whole_word);
        if invert { hit = !hit; }
        if hit {
            match_count += 1;
            if !count_only && !list_only {
                let s = core::str::from_utf8(line).unwrap_or("");
                if !prefix.is_empty() && show_num {
                    anyos_std::println!("{}:{}:{}", prefix, line_no, s);
                } else if !prefix.is_empty() {
                    anyos_std::println!("{}:{}", prefix, s);
                } else if show_num {
                    anyos_std::println!("{}:{}", line_no, s);
                } else {
                    anyos_std::println!("{}", s);
                }
            }
        }
    }

    if count_only {
        if !prefix.is_empty() {
            anyos_std::println!("{}:{}", prefix, match_count);
        } else {
            anyos_std::println!("{}", match_count);
        }
    }
    match_count > 0
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let ignore_case = args.has(b'i');
    let invert = args.has(b'v');
    let show_num = args.has(b'n');
    let count_only = args.has(b'c');
    let list_only = args.has(b'l');
    let whole_word = args.has(b'w');

    if args.pos_count == 0 {
        anyos_std::println!("Usage: grep [-ivnclw] PATTERN [FILE...]");
        return;
    }

    let pattern = args.positional[0];
    let needle = pattern.as_bytes();
    let multi = args.pos_count > 2;

    if args.pos_count == 1 {
        // Read from stdin
        let data = read_file(0);
        grep_data(&data, needle, ignore_case, invert, show_num, count_only, list_only, whole_word, "");
        return;
    }

    for i in 1..args.pos_count {
        let path = args.positional[i];
        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX {
            anyos_std::println!("grep: {}: No such file or directory", path);
            continue;
        }
        let data = read_file(fd);
        anyos_std::fs::close(fd);

        let prefix = if multi { path } else { "" };
        let found = grep_data(&data, needle, ignore_case, invert, show_num, count_only, list_only, whole_word, prefix);
        if list_only && found {
            anyos_std::println!("{}", path);
        }
    }
}
