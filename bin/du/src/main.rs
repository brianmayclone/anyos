#![no_std]
#![no_main]

use anyos_std::Vec;

anyos_std::entry!(main);

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn fmt_u64(mut n: u64, buf: &mut [u8; 20]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap();
    }
    let mut tmp = [0u8; 20];
    let mut i = 0usize;
    while n > 0 {
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    core::str::from_utf8(&buf[..i]).unwrap()
}

fn format_human(bytes: u64, buf: &mut [u8; 20]) -> &str {
    if bytes >= 1024 * 1024 * 1024 {
        let v10 = (bytes * 10 / (1024 * 1024 * 1024)) as u64;
        let whole = v10 / 10;
        let frac = v10 % 10;
        let l = fmt_u64(whole, buf).len();
        buf[l] = b'.';
        buf[l + 1] = b'0' + frac as u8;
        buf[l + 2] = b'G';
        core::str::from_utf8(&buf[..l + 3]).unwrap()
    } else if bytes >= 1024 * 1024 {
        let v10 = (bytes * 10 / (1024 * 1024)) as u64;
        let whole = v10 / 10;
        let frac = v10 % 10;
        let l = fmt_u64(whole, buf).len();
        buf[l] = b'.';
        buf[l + 1] = b'0' + frac as u8;
        buf[l + 2] = b'M';
        core::str::from_utf8(&buf[..l + 3]).unwrap()
    } else if bytes >= 1024 {
        let kb = bytes / 1024;
        let l = fmt_u64(kb, buf).len();
        buf[l] = b'K';
        core::str::from_utf8(&buf[..l + 1]).unwrap()
    } else if bytes > 0 {
        let l = fmt_u64(bytes, buf).len();
        buf[l] = b'B';
        core::str::from_utf8(&buf[..l + 1]).unwrap()
    } else {
        buf[0] = b'0';
        core::str::from_utf8(&buf[..1]).unwrap()
    }
}

/// Returns size in display units (1K blocks or MB or human string).
fn size_str<'b>(bytes: u64, human: bool, use_mb: bool, buf: &'b mut [u8; 20]) -> &'b str {
    if human {
        format_human(bytes, buf)
    } else if use_mb {
        fmt_u64((bytes + 1024 * 1024 - 1) / (1024 * 1024), buf)
    } else {
        // default: 1K blocks, round up
        fmt_u64((bytes + 1023) / 1024, buf)
    }
}

// ─── Path helper ──────────────────────────────────────────────────────────────

fn build_child_path<'a>(buf: &'a mut [u8; 512], parent: &str, name: &str) -> &'a str {
    let pb = parent.as_bytes();
    let nb = name.as_bytes();
    let plen = pb.len().min(511);
    buf[..plen].copy_from_slice(&pb[..plen]);
    let mut pos = plen;
    if pos > 0 && buf[pos - 1] != b'/' && pos < 511 {
        buf[pos] = b'/';
        pos += 1;
    }
    let nlen = nb.len().min(511 - pos);
    buf[pos..pos + nlen].copy_from_slice(&nb[..nlen]);
    pos += nlen;
    core::str::from_utf8(&buf[..pos]).unwrap_or("")
}

// ─── Core walker ──────────────────────────────────────────────────────────────

struct Opts {
    all: bool,       // -a: show files too
    summarize: bool, // -s: only top-level total
    human: bool,     // -h: human-readable
    use_mb: bool,    // -m: megabytes
    max_depth: u32,  // -d N
}

/// A pending print entry: (path, bytes).
type Entry = (anyos_std::String, u64);

/// Recursively walk `path`, appending print entries to `out`.
/// Returns total byte count for the subtree.
fn walk(path: &str, depth: u32, opts: &Opts, out: &mut Vec<Entry>) -> u64 {
    // stat the target
    let mut st = [0u32; 7];
    if anyos_std::fs::stat(path, &mut st) != 0 {
        return 0;
    }
    let ftype = st[0]; // 0=file, 1=dir
    let fsize = st[1] as u64;

    if ftype != 1 {
        // Regular file (or anything that is not a directory)
        // Only emit if -a and within depth, and not summarize
        if opts.all && !opts.summarize && depth <= opts.max_depth {
            out.push((anyos_std::String::from(path), fsize));
        }
        return fsize;
    }

    // ── Directory ─────────────────────────────────────────────────────────────
    let mut dir_buf = [0u8; 64 * 512]; // up to 512 entries
    let count = anyos_std::fs::readdir(path, &mut dir_buf);
    if count == u32::MAX {
        return 0;
    }

    let mut total: u64 = 0;

    for i in 0..count as usize {
        let e = &dir_buf[i * 64..(i + 1) * 64];
        let etype = e[0]; // 0=file 1=dir
        let nlen = e[1] as usize;
        let name = core::str::from_utf8(&e[8..8 + nlen.min(56)]).unwrap_or("");
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }

        let mut cbuf = [0u8; 512];
        let child = build_child_path(&mut cbuf, path, name);

        if etype == 1 {
            // Subdirectory — recurse
            let sub = walk(child, depth + 1, opts, out);
            total += sub;
        } else {
            // File — size is in the directory entry at offset 4 (u32 LE)
            let sz = u32::from_le_bytes([e[4], e[5], e[6], e[7]]) as u64;
            total += sz;
            if opts.all && !opts.summarize && depth + 1 <= opts.max_depth {
                // We need the actual stat for a fresh path string
                out.push((anyos_std::String::from(child), sz));
            }
        }
    }

    // Emit this directory after its children (post-order), unless summarize
    // skips non-root levels.
    let print_this = if opts.summarize {
        depth == 0
    } else {
        depth <= opts.max_depth
    };
    if print_this {
        out.push((anyos_std::String::from(path), total));
    }

    total
}

// ─── main ─────────────────────────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);

    if raw.contains("--help") {
        anyos_std::println!("du - Estimate file space usage");
        anyos_std::println!("");
        anyos_std::println!("Usage: du [OPTION]... [FILE]...");
        anyos_std::println!("");
        anyos_std::println!("Options:");
        anyos_std::println!("  -a             All files, not just directories");
        anyos_std::println!("  -c             Show grand total");
        anyos_std::println!("  -d N           Max depth (0 = same as -s)");
        anyos_std::println!("  -h             Human-readable sizes (e.g. 1.2M)");
        anyos_std::println!("  -k             Sizes in 1K blocks (default)");
        anyos_std::println!("  -m             Sizes in megabytes");
        anyos_std::println!("  -s             Summary: only total for each argument");
        anyos_std::println!("  --max-depth=N  Same as -d N");
        anyos_std::println!("  --help         Show this help");
        return;
    }

    // ── Parse args manually (du has non-trivial flag set) ────────────────────
    let mut opt_all = false;
    let mut opt_total = false;
    let mut opt_human = false;
    let mut opt_mb = false;
    let mut opt_summarize = false;
    let mut max_depth: u32 = u32::MAX;
    let mut paths: Vec<&str> = Vec::new();

    let parts: Vec<&str> = raw.split_ascii_whitespace().collect();
    let mut i = 0usize;
    while i < parts.len() {
        let p = parts[i];
        if p == "--" {
            i += 1;
            while i < parts.len() {
                paths.push(parts[i]);
                i += 1;
            }
            break;
        }
        if p.starts_with("--max-depth=") {
            let val = &p["--max-depth=".len()..];
            max_depth = parse_u32(val).unwrap_or(u32::MAX);
            i += 1;
            continue;
        }
        if p == "-d" || p == "--max-depth" {
            if i + 1 < parts.len() {
                max_depth = parse_u32(parts[i + 1]).unwrap_or(u32::MAX);
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if p.starts_with('-') && p.len() > 1 && !p.starts_with("--") {
            // Short flags, possibly combined like -ahc
            for &b in &p.as_bytes()[1..] {
                match b {
                    b'a' => opt_all = true,
                    b'c' => opt_total = true,
                    b'h' => opt_human = true,
                    b'k' => {} // default, ignore
                    b'm' => opt_mb = true,
                    b's' => opt_summarize = true,
                    b'x' => {} // --one-file-system: not implemented, silently ignore
                    _ => {}
                }
            }
            i += 1;
            continue;
        }
        if !p.starts_with('-') {
            paths.push(p);
        }
        i += 1;
    }

    // Default path is "."
    if paths.is_empty() {
        paths.push(".");
    }

    // -s implies max_depth = 0
    if opt_summarize && max_depth == u32::MAX {
        max_depth = 0;
    }
    // -d 0 implies -s behaviour (no subdirs printed)
    if max_depth == 0 {
        opt_summarize = true;
    }

    let opts = Opts {
        all: opt_all,
        summarize: opt_summarize,
        human: opt_human,
        use_mb: opt_mb,
        max_depth,
    };

    let mut grand_total: u64 = 0;

    for path in &paths {
        let mut entries: Vec<Entry> = Vec::new();
        let total = walk(path, 0, &opts, &mut entries);
        grand_total += total;

        for (ref ep, bytes) in &entries {
            let mut nbuf = [0u8; 20];
            let sz = size_str(*bytes, opts.human, opts.use_mb, &mut nbuf);
            anyos_std::println!("{}\t{}", sz, ep.as_str());
        }
    }

    if opt_total {
        let mut nbuf = [0u8; 20];
        let sz = size_str(grand_total, opts.human, opts.use_mb, &mut nbuf);
        anyos_std::println!("{}\ttotal", sz);
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut n: u32 = 0;
    if s.is_empty() {
        return None;
    }
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}
