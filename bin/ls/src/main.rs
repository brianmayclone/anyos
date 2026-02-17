#![no_std]
#![no_main]

anyos_std::entry!(main);

struct Entry {
    name: [u8; 56],
    name_len: usize,
    size: u32,
    entry_type: u8,
    is_symlink: bool,
}

fn format_size_human(buf: &mut [u8], size: u32) -> usize {
    const UNITS: &[u8] = b"BKMGT";
    let mut val = size;
    let mut unit = 0;
    // Find the right unit: shift by 1024 while >= 1024
    while val >= 1024 && unit < 4 {
        unit += 1;
        val /= 1024; // integer division — we'll compute one decimal below
    }
    if unit == 0 {
        // Bytes — no decimal
        return format_u32(buf, val, 0);
    }
    // Compute one decimal place: (size * 10 / 1024^unit) % 10
    let mut denom: u64 = 1;
    for _ in 0..unit {
        denom *= 1024;
    }
    let tenths = ((size as u64) * 10 / denom) % 10;
    let whole = format_u32(buf, val, 0);
    let mut pos = whole;
    buf[pos] = b'.';
    pos += 1;
    buf[pos] = b'0' + tenths as u8;
    pos += 1;
    buf[pos] = UNITS[unit];
    pos += 1;
    pos
}

fn format_u32(buf: &mut [u8], val: u32, min_width: usize) -> usize {
    if val == 0 {
        let pad = if min_width > 1 { min_width - 1 } else { 0 };
        for i in 0..pad {
            buf[i] = b' ';
        }
        buf[pad] = b'0';
        return pad + 1;
    }
    let mut tmp = [0u8; 10];
    let mut n = val;
    let mut len = 0;
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    let pad = if min_width > len { min_width - len } else { 0 };
    for i in 0..pad {
        buf[i] = b' ';
    }
    for i in 0..len {
        buf[pad + i] = tmp[len - 1 - i];
    }
    pad + len
}

fn to_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

fn cmp_name_ci(a: &[u8], a_len: usize, b: &[u8], b_len: usize) -> core::cmp::Ordering {
    let min = if a_len < b_len { a_len } else { b_len };
    for i in 0..min {
        let la = to_lower(a[i]);
        let lb = to_lower(b[i]);
        if la < lb { return core::cmp::Ordering::Less; }
        if la > lb { return core::cmp::Ordering::Greater; }
    }
    a_len.cmp(&b_len)
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let long = args.has(b'l');
    let all = args.has(b'a');
    let one_per_line = args.has(b'1');
    let human = args.has(b'h');
    let sort_size = args.has(b'S');
    let reverse = args.has(b'r');

    let path = args.first_or(".");

    // Read directory entries
    let mut buf = [0u8; 64 * 128]; // max 128 entries
    let count = anyos_std::fs::readdir(path, &mut buf);

    if count == u32::MAX {
        anyos_std::println!("ls: cannot access '{}': No such file or directory", path);
        return;
    }

    // Parse into entries, filtering hidden
    let mut entries = anyos_std::Vec::new();
    for i in 0..count as usize {
        let raw_entry = &buf[i * 64..(i + 1) * 64];
        let entry_type = raw_entry[0];
        let name_len = raw_entry[1] as usize;
        let flags = raw_entry[2];
        let is_symlink = flags & 1 != 0;
        let size = u32::from_le_bytes([raw_entry[4], raw_entry[5], raw_entry[6], raw_entry[7]]);
        let mut name = [0u8; 56];
        let nlen = name_len.min(56);
        name[..nlen].copy_from_slice(&raw_entry[8..8 + nlen]);

        if !all && nlen > 0 && name[0] == b'.' {
            continue;
        }

        entries.push(Entry { name, name_len: nlen, size, entry_type, is_symlink });
    }

    // Sort
    if sort_size {
        entries.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    } else {
        entries.sort_unstable_by(|a, b| {
            cmp_name_ci(&a.name, a.name_len, &b.name, b.name_len)
        });
    }
    if reverse {
        entries.reverse();
    }

    // Print
    if long {
        for e in &entries {
            let type_char = if e.is_symlink { 'l' } else { match e.entry_type { 1 => 'd', 2 => 'c', _ => '-' } };
            let name_str = core::str::from_utf8(&e.name[..e.name_len]).unwrap_or("???");

            // For symlinks, try to read target
            let mut link_target = [0u8; 256];
            let mut link_len = 0usize;
            if e.is_symlink {
                // Build full path for readlink
                let mut full = [0u8; 512];
                let mut full_len = 0usize;
                let path_bytes = path.as_bytes();
                for &b in path_bytes {
                    if full_len < 511 { full[full_len] = b; full_len += 1; }
                }
                if full_len > 0 && full[full_len - 1] != b'/' {
                    if full_len < 511 { full[full_len] = b'/'; full_len += 1; }
                }
                for j in 0..e.name_len {
                    if full_len < 511 { full[full_len] = e.name[j]; full_len += 1; }
                }
                full[full_len] = 0;
                let full_str = core::str::from_utf8(&full[..full_len]).unwrap_or("");
                let n = anyos_std::fs::readlink(full_str, &mut link_target);
                if n != u32::MAX { link_len = n as usize; }
            }

            if human {
                let mut sbuf = [0u8; 16];
                let slen = format_size_human(&mut sbuf, e.size);
                let size_str = core::str::from_utf8(&sbuf[..slen]).unwrap_or("?");
                if link_len > 0 {
                    let tgt = core::str::from_utf8(&link_target[..link_len]).unwrap_or("?");
                    anyos_std::println!("{}  {:>6}  {} -> {}", type_char, size_str, name_str, tgt);
                } else {
                    anyos_std::println!("{}  {:>6}  {}", type_char, size_str, name_str);
                }
            } else {
                if link_len > 0 {
                    let tgt = core::str::from_utf8(&link_target[..link_len]).unwrap_or("?");
                    anyos_std::println!("{}  {:>8}  {} -> {}", type_char, e.size, name_str, tgt);
                } else {
                    anyos_std::println!("{}  {:>8}  {}", type_char, e.size, name_str);
                }
            }
        }
    } else if one_per_line {
        for e in &entries {
            let name_str = core::str::from_utf8(&e.name[..e.name_len]).unwrap_or("???");
            anyos_std::println!("{}", name_str);
        }
    } else {
        // Columnar output: names separated by spaces
        for (i, e) in entries.iter().enumerate() {
            let name_str = core::str::from_utf8(&e.name[..e.name_len]).unwrap_or("???");
            if i > 0 {
                anyos_std::print!("  ");
            }
            anyos_std::print!("{}", name_str);
        }
        if !entries.is_empty() {
            anyos_std::println!("");
        }
    }
}
