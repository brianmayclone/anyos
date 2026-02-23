#![no_std]
#![no_main]

anyos_std::entry!(main);

fn to_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

fn matches_pattern(name: &str, pattern: &str, ignore_case: bool) -> bool {
    if pattern.is_empty() { return name.is_empty(); }
    let pat = pattern.as_bytes();
    let nam = name.as_bytes();
    let mut pi = 0;
    let mut ni = 0;
    let mut star_pi = usize::MAX;
    let mut star_ni = 0;
    while ni < nam.len() {
        if pi < pat.len() && pat[pi] == b'*' {
            star_pi = pi;
            star_ni = ni;
            pi += 1;
        } else if pi < pat.len() {
            let a = if ignore_case { to_lower(nam[ni]) } else { nam[ni] };
            let b = if ignore_case { to_lower(pat[pi]) } else { pat[pi] };
            if pat[pi] == b'?' || a == b {
                pi += 1;
                ni += 1;
            } else if star_pi != usize::MAX {
                pi = star_pi + 1;
                star_ni += 1;
                ni = star_ni;
            } else {
                return false;
            }
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

fn build_path(buf: &mut [u8; 512], parent: &str, name: &str) -> usize {
    let plen = parent.len();
    let has_slash = plen > 0 && parent.as_bytes()[plen - 1] == b'/';
    buf[..plen].copy_from_slice(parent.as_bytes());
    let mut pos = plen;
    if !has_slash && plen > 0 {
        buf[pos] = b'/';
        pos += 1;
    }
    let nlen = name.len().min(512 - pos);
    buf[pos..pos + nlen].copy_from_slice(&name.as_bytes()[..nlen]);
    pos + nlen
}

fn find_in(path: &str, pattern: &str, ignore_case: bool, type_filter: u8) {
    let mut buf = [0u8; 64 * 128]; // max 128 entries
    let count = anyos_std::fs::readdir(path, &mut buf);
    if count == u32::MAX { return; }

    for i in 0..count as usize {
        // 64-byte struct: [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes]
        let entry = &buf[i * 64..(i + 1) * 64];
        let entry_type = entry[0];
        let name_len = entry[1] as usize;
        let name = core::str::from_utf8(&entry[8..8 + name_len.min(56)]).unwrap_or("");

        if name.is_empty() || name == "." || name == ".." { continue; }

        let mut path_buf = [0u8; 512];
        let full_len = build_path(&mut path_buf, path, name);
        let full_str = core::str::from_utf8(&path_buf[..full_len]).unwrap_or("");

        // Check type filter: 0=no filter, 'f'=file(0), 'd'=dir(1)
        let type_ok = match type_filter {
            b'f' => entry_type == 0,
            b'd' => entry_type == 1,
            _ => true,
        };

        if type_ok && matches_pattern(name, pattern, ignore_case) {
            anyos_std::println!("{}", full_str);
        }

        // Recurse into directories
        if entry_type == 1 {
            find_in(full_str, pattern, ignore_case, type_filter);
        }
    }
}

/// Strip surrounding quotes (" or ') from a string if present.
fn strip_quotes(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 {
        if (b[0] == b'"' && b[b.len() - 1] == b'"')
            || (b[0] == b'\'' && b[b.len() - 1] == b'\'')
        {
            return &s[1..s.len() - 1];
        }
    }
    s
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);

    // Manual parsing for find's long-flag style: find [PATH] [-name PAT] [-iname PAT] [-type f/d]
    let mut path = ".";
    let mut pattern = "*";
    let mut ignore_case = false;
    let mut type_filter: u8 = 0;

    let parts: anyos_std::Vec<&str> = raw.split_ascii_whitespace().collect();
    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "-name" if i + 1 < parts.len() => {
                pattern = strip_quotes(parts[i + 1]);
                ignore_case = false;
                i += 2;
            }
            "-iname" if i + 1 < parts.len() => {
                pattern = strip_quotes(parts[i + 1]);
                ignore_case = true;
                i += 2;
            }
            "-type" if i + 1 < parts.len() => {
                let t = parts[i + 1];
                if !t.is_empty() {
                    type_filter = t.as_bytes()[0];
                }
                i += 2;
            }
            s if !s.starts_with('-') => {
                path = s;
                i += 1;
            }
            _ => { i += 1; }
        }
    }

    find_in(path, pattern, ignore_case, type_filter);
}
