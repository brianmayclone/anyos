#![no_std]
#![no_main]

anyos_std::entry!(main);

fn matches_pattern(name: &str, pattern: &str) -> bool {
    // Simple wildcard matching: * matches any sequence
    if pattern.is_empty() { return name.is_empty(); }
    let pat = pattern.as_bytes();
    let nam = name.as_bytes();
    let mut pi = 0;
    let mut ni = 0;
    let mut star_pi = usize::MAX;
    let mut star_ni = 0;
    while ni < nam.len() {
        if pi < pat.len() && (pat[pi] == b'?' || pat[pi] == nam[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star_pi = pi;
            star_ni = ni;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ni += 1;
            ni = star_ni;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' { pi += 1; }
    pi == pat.len()
}

fn find_in(path: &str, pattern: &str, buf: &mut [u8]) {
    let count = anyos_std::fs::readdir(path, buf);
    if count == u32::MAX { return; }

    let mut offset = 0;
    for _ in 0..count {
        // Each entry: name (null-terminated), then next entry
        let end = buf[offset..].iter().position(|&b| b == 0).unwrap_or(0);
        let name = core::str::from_utf8(&buf[offset..offset + end]).unwrap_or("");
        offset += end + 1;

        if name.is_empty() || name == "." || name == ".." { continue; }

        // Build full path
        let mut full = [0u8; 256];
        let plen = path.len();
        full[..plen].copy_from_slice(path.as_bytes());
        if plen > 0 && full[plen - 1] != b'/' {
            full[plen] = b'/';
            let nlen = name.len().min(256 - plen - 2);
            full[plen + 1..plen + 1 + nlen].copy_from_slice(&name.as_bytes()[..nlen]);
            let full_str = core::str::from_utf8(&full[..plen + 1 + nlen]).unwrap_or("");

            if matches_pattern(name, pattern) {
                anyos_std::println!("{}", full_str);
            }

            // Recurse into directories
            let mut stat_buf = [0u32; 2];
            if anyos_std::fs::stat(full_str, &mut stat_buf) == 0 && stat_buf[0] == 1 {
                let mut sub_buf = [0u8; 4096];
                find_in(full_str, pattern, &mut sub_buf);
            }
        } else {
            let nlen = name.len().min(256 - plen - 1);
            full[plen..plen + nlen].copy_from_slice(&name.as_bytes()[..nlen]);
            let full_str = core::str::from_utf8(&full[..plen + nlen]).unwrap_or("");

            if matches_pattern(name, pattern) {
                anyos_std::println!("{}", full_str);
            }

            let mut stat_buf = [0u32; 2];
            if anyos_std::fs::stat(full_str, &mut stat_buf) == 0 && stat_buf[0] == 1 {
                let mut sub_buf = [0u8; 4096];
                find_in(full_str, pattern, &mut sub_buf);
            }
        }
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    // Parse: find [PATH] -name PATTERN
    let mut path = "/";
    let mut pattern = "*";

    let parts: alloc::vec::Vec<&str> = args.split_ascii_whitespace().collect();
    let mut i = 0;
    while i < parts.len() {
        if parts[i] == "-name" && i + 1 < parts.len() {
            pattern = parts[i + 1];
            i += 2;
        } else {
            path = parts[i];
            i += 1;
        }
    }

    let mut buf = [0u8; 4096];
    find_in(path, pattern, &mut buf);
}
