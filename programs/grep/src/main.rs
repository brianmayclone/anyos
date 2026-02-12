#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    // Parse: grep PATTERN [FILE]
    let mut parts = args.split_ascii_whitespace();
    let pattern = match parts.next() {
        Some(p) => p,
        None => {
            anyos_std::println!("Usage: grep PATTERN [FILE]");
            return;
        }
    };
    let file = parts.next().unwrap_or("");

    if file.is_empty() {
        anyos_std::println!("Usage: grep PATTERN FILE");
        return;
    }

    let fd = anyos_std::fs::open(file, 0);
    if fd == u32::MAX {
        anyos_std::println!("grep: cannot open '{}'", file);
        return;
    }

    // Read file and search line by line
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

    let data = &file_buf[..total];
    let pattern_bytes = pattern.as_bytes();
    let mut line_no: u32 = 0;

    for line in data.split(|&b| b == b'\n') {
        line_no += 1;
        // Simple substring search (case-sensitive)
        if contains(line, pattern_bytes) {
            if let Ok(s) = core::str::from_utf8(line) {
                anyos_std::println!("{}", s);
            }
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() { return true; }
    if needle.len() > haystack.len() { return false; }
    for i in 0..=haystack.len() - needle.len() {
        if &haystack[i..i + needle.len()] == needle {
            return true;
        }
    }
    false
}
