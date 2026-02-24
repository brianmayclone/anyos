#![no_std]
#![no_main]

anyos_std::entry!(main);

struct Entry {
    name: [u8; 56],
    name_len: usize,
    size: u32,
    entry_type: u8,
    is_symlink: bool,
    uid: u16,
    gid: u16,
    mode: u32,
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

/// Build a child path from parent + name.
fn build_path(parent: &str, name: &str) -> anyos_std::String {
    if parent == "/" {
        anyos_std::format!("/{}", name)
    } else {
        anyos_std::format!("{}/{}", parent, name)
    }
}

/// List the contents of a single directory, optionally recursing into subdirectories.
fn list_directory(path: &str, long: bool, all: bool, one_per_line: bool,
                  human: bool, sort_size: bool, reverse: bool, recursive: bool) {
    let mut buf = [0u8; 64 * 128];
    let count = anyos_std::fs::readdir(path, &mut buf);

    if count == u32::MAX {
        anyos_std::println!("ls: cannot access '{}': No such file or directory", path);
        return;
    }

    let mut entries = anyos_std::Vec::new();
    let max_entries = buf.len() / 64;
    let actual = (count as usize).min(max_entries);
    for i in 0..actual {
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

        // Get uid/gid/mode via stat if long format
        let (uid, gid, mode) = if long {
            let name_str = core::str::from_utf8(&name[..nlen]).unwrap_or("");
            let full_path = build_path(path, name_str);
            let mut stat_buf = [0u32; 7];
            if anyos_std::fs::stat(&full_path, &mut stat_buf) == 0 {
                (stat_buf[3] as u16, stat_buf[4] as u16, stat_buf[5])
            } else {
                (0u16, 0u16, 0u32)
            }
        } else {
            (0, 0, 0)
        };

        entries.push(Entry { name, name_len: nlen, size, entry_type, is_symlink, uid, gid, mode });
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

    print_entries(&entries, path, long, one_per_line, human);

    // Recurse into subdirectories
    if recursive {
        for e in &entries {
            if e.entry_type == 1 {
                let name_str = core::str::from_utf8(&e.name[..e.name_len]).unwrap_or("");
                if name_str == "." || name_str == ".." { continue; }
                let child = build_path(path, name_str);
                anyos_std::println!("\n{}:", child);
                list_directory(&child, long, all, one_per_line, human, sort_size, reverse, recursive);
            }
        }
    }
}

/// Resolve a uid to a username string. Falls back to the numeric uid.
fn uid_to_name(uid: u16, buf: &mut [u8; 32]) -> &str {
    let n = anyos_std::process::getusername(uid, buf);
    if n != u32::MAX && n > 0 {
        let len = (n as usize).min(32);
        core::str::from_utf8(&buf[..len]).unwrap_or("?")
    } else {
        // Fallback: format uid as number
        let len = format_u32(buf, uid as u32, 0);
        core::str::from_utf8(&buf[..len]).unwrap_or("?")
    }
}

/// Resolve a gid to a group name by searching the listgroups output.
/// Format: "gid:groupname\ngid:groupname\n..."
/// Writes result to out_buf and returns the length.
fn gid_to_name(gid: u16, groups_buf: &[u8], groups_len: usize, out_buf: &mut [u8; 32]) -> usize {
    let data = &groups_buf[..groups_len];
    let mut start = 0;
    while start < data.len() {
        let mut end = start;
        while end < data.len() && data[end] != b'\n' {
            end += 1;
        }
        let line = &data[start..end];
        if let Some(colon) = line.iter().position(|&b| b == b':') {
            let gid_part = &line[..colon];
            let name_part = &line[colon + 1..];
            let mut g: u32 = 0;
            let mut valid = !gid_part.is_empty();
            for &b in gid_part {
                if b >= b'0' && b <= b'9' {
                    g = g * 10 + (b - b'0') as u32;
                } else {
                    valid = false;
                    break;
                }
            }
            if valid && g == gid as u32 {
                let nlen = name_part.len().min(32);
                out_buf[..nlen].copy_from_slice(&name_part[..nlen]);
                return nlen;
            }
        }
        start = end + 1;
    }
    // Fallback: format gid as number
    format_u32(out_buf, gid as u32, 0)
}

/// Format permission bits as rwxrwxrwx string.
fn format_mode(buf: &mut [u8; 9], mode: u32) {
    buf[0] = if mode & 0o400 != 0 { b'r' } else { b'-' };
    buf[1] = if mode & 0o200 != 0 { b'w' } else { b'-' };
    buf[2] = if mode & 0o100 != 0 { b'x' } else { b'-' };
    buf[3] = if mode & 0o040 != 0 { b'r' } else { b'-' };
    buf[4] = if mode & 0o020 != 0 { b'w' } else { b'-' };
    buf[5] = if mode & 0o010 != 0 { b'x' } else { b'-' };
    buf[6] = if mode & 0o004 != 0 { b'r' } else { b'-' };
    buf[7] = if mode & 0o002 != 0 { b'w' } else { b'-' };
    buf[8] = if mode & 0o001 != 0 { b'x' } else { b'-' };
}

/// Print a list of entries in the requested format.
fn print_entries(entries: &[Entry], base_path: &str, long: bool, one_per_line: bool, human: bool) {
    if long {
        // Pre-load group list once for all entries
        let mut groups_raw = [0u8; 1024];
        let groups_len = anyos_std::users::listgroups(&mut groups_raw) as usize;
        let groups_len = groups_len.min(groups_raw.len());

        for e in entries {
            let type_char = if e.is_symlink { 'l' } else { match e.entry_type { 1 => 'd', 2 => 'c', _ => '-' } };
            let name_str = core::str::from_utf8(&e.name[..e.name_len]).unwrap_or("???");

            // Permission string
            let mut mode_buf = [b'-'; 9];
            format_mode(&mut mode_buf, e.mode);
            let mode_str = core::str::from_utf8(&mode_buf).unwrap_or("---------");

            // Resolve user and group names
            let mut ubuf = [0u8; 32];
            let user_name = uid_to_name(e.uid, &mut ubuf);
            let mut gbuf = [0u8; 32];
            let glen = gid_to_name(e.gid, &groups_raw, groups_len, &mut gbuf);
            let group_name = core::str::from_utf8(&gbuf[..glen]).unwrap_or("?");

            // For symlinks, try to read target
            let mut link_target = [0u8; 256];
            let mut link_len = 0usize;
            if e.is_symlink {
                let mut full = [0u8; 512];
                let mut full_len = 0usize;
                let path_bytes = base_path.as_bytes();
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
                    anyos_std::println!("{}{} {:<8} {:<8} {:>6}  {} -> {}", type_char, mode_str, user_name, group_name, size_str, name_str, tgt);
                } else {
                    anyos_std::println!("{}{} {:<8} {:<8} {:>6}  {}", type_char, mode_str, user_name, group_name, size_str, name_str);
                }
            } else {
                if link_len > 0 {
                    let tgt = core::str::from_utf8(&link_target[..link_len]).unwrap_or("?");
                    anyos_std::println!("{}{} {:<8} {:<8} {:>8}  {} -> {}", type_char, mode_str, user_name, group_name, e.size, name_str, tgt);
                } else {
                    anyos_std::println!("{}{} {:<8} {:<8} {:>8}  {}", type_char, mode_str, user_name, group_name, e.size, name_str);
                }
            }
        }
    } else if one_per_line {
        for e in entries {
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

/// List paths as entries without descending into directories (for -d flag).
fn list_as_entries(paths: &[&str], long: bool, one_per_line: bool,
                   human: bool, sort_size: bool, reverse: bool) {
    let mut entries = anyos_std::Vec::new();
    for &p in paths {
        let mut stat_buf = [0u32; 7];
        let ret = anyos_std::fs::stat(p, &mut stat_buf);
        let (etype, size, is_sym, uid, gid, mode) = if ret == 0 {
            (stat_buf[0] as u8, stat_buf[1], stat_buf[2] & 1 != 0,
             stat_buf[3] as u16, stat_buf[4] as u16, stat_buf[5])
        } else {
            anyos_std::println!("ls: cannot access '{}': No such file or directory", p);
            continue;
        };
        // Use the path as-is for display (e.g. "." or "/etc")
        let mut name = [0u8; 56];
        let nlen = p.len().min(56);
        name[..nlen].copy_from_slice(&p.as_bytes()[..nlen]);
        entries.push(Entry { name, name_len: nlen, size, entry_type: etype, is_symlink: is_sym, uid, gid, mode });
    }
    if sort_size {
        entries.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    } else {
        entries.sort_unstable_by(|a, b| cmp_name_ci(&a.name, a.name_len, &b.name, b.name_len));
    }
    if reverse { entries.reverse(); }
    print_entries(&entries, ".", long, one_per_line, human);
}

/// List individual files (from glob expansion or explicit file args).
fn list_files(args: &anyos_std::args::ParsedArgs, long: bool, one_per_line: bool,
              human: bool, sort_size: bool, reverse: bool) {
    let mut entries = anyos_std::Vec::new();

    for idx in 0..args.pos_count {
        let name_str = args.positional[idx];
        // Stat the file to get type/size info
        let mut stat_buf = [0u32; 7];
        let ret = anyos_std::fs::stat(name_str, &mut stat_buf);
        let (etype, size, is_sym, uid, gid, mode) = if ret == 0 {
            (stat_buf[0] as u8, stat_buf[1], stat_buf[2] & 1 != 0,
             stat_buf[3] as u16, stat_buf[4] as u16, stat_buf[5])
        } else {
            (0u8, 0u32, false, 0u16, 0u16, 0u32)
        };

        // Extract just the filename part for display
        let display_name = if let Some(pos) = name_str.rfind('/') {
            &name_str[pos + 1..]
        } else {
            name_str
        };

        let mut name = [0u8; 56];
        let nlen = display_name.len().min(56);
        name[..nlen].copy_from_slice(&display_name.as_bytes()[..nlen]);

        if ret != 0 {
            anyos_std::println!("ls: cannot access '{}': No such file or directory", name_str);
            continue;
        }

        entries.push(Entry { name, name_len: nlen, size, entry_type: etype, is_symlink: is_sym, uid, gid, mode });
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

    print_entries(&entries, ".", long, one_per_line, human);
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
    let dir_itself = args.has(b'd'); // -d: list directories themselves, not contents
    let recursive = args.has(b'R');  // -R: list subdirectories recursively

    if args.pos_count == 0 {
        if dir_itself {
            list_as_entries(&["."], long, one_per_line, human, sort_size, reverse);
        } else {
            if recursive { anyos_std::println!(".:");}
            list_directory(".", long, all, one_per_line, human, sort_size, reverse, recursive);
        }
    } else if dir_itself {
        let mut paths: anyos_std::Vec<&str> = anyos_std::Vec::new();
        for idx in 0..args.pos_count {
            paths.push(args.positional[idx]);
        }
        list_as_entries(&paths, long, one_per_line, human, sort_size, reverse);
    } else if args.pos_count == 1 {
        let path = args.positional[0];
        let mut buf = [0u8; 64 * 4];
        let count = anyos_std::fs::readdir(path, &mut buf);
        if count != u32::MAX {
            if recursive { anyos_std::println!("{}:", path); }
            list_directory(path, long, all, one_per_line, human, sort_size, reverse, recursive);
        } else {
            list_files(&args, long, one_per_line, human, sort_size, reverse);
        }
    } else {
        let mut files: anyos_std::Vec<&str> = anyos_std::Vec::new();
        let mut dirs: anyos_std::Vec<&str> = anyos_std::Vec::new();

        for idx in 0..args.pos_count {
            let path = args.positional[idx];
            let mut stat_buf = [0u32; 7];
            let ret = anyos_std::fs::stat(path, &mut stat_buf);
            if ret == 0 && stat_buf[0] == 1 {
                dirs.push(path);
            } else {
                files.push(path);
            }
        }

        if !files.is_empty() {
            let mut entries = anyos_std::Vec::new();
            for name_str in &files {
                let mut stat_buf = [0u32; 7];
                let ret = anyos_std::fs::stat(name_str, &mut stat_buf);
                let (etype, size, is_sym, uid, gid, mode) = if ret == 0 {
                    (stat_buf[0] as u8, stat_buf[1], stat_buf[2] & 1 != 0,
                     stat_buf[3] as u16, stat_buf[4] as u16, stat_buf[5])
                } else {
                    anyos_std::println!("ls: cannot access '{}': No such file or directory", name_str);
                    continue;
                };
                let display = if let Some(pos) = name_str.rfind('/') {
                    &name_str[pos + 1..]
                } else {
                    name_str
                };
                let mut name = [0u8; 56];
                let nlen = display.len().min(56);
                name[..nlen].copy_from_slice(&display.as_bytes()[..nlen]);
                entries.push(Entry { name, name_len: nlen, size, entry_type: etype, is_symlink: is_sym, uid, gid, mode });
            }
            if sort_size {
                entries.sort_unstable_by(|a, b| b.size.cmp(&a.size));
            } else {
                entries.sort_unstable_by(|a, b| cmp_name_ci(&a.name, a.name_len, &b.name, b.name_len));
            }
            if reverse { entries.reverse(); }
            print_entries(&entries, ".", long, one_per_line, human);
        }

        for (i, dir) in dirs.iter().enumerate() {
            if !files.is_empty() || i > 0 {
                anyos_std::println!("");
            }
            if dirs.len() > 1 || !files.is_empty() || recursive {
                anyos_std::println!("{}:", dir);
            }
            list_directory(dir, long, all, one_per_line, human, sort_size, reverse, recursive);
        }
    }
}
