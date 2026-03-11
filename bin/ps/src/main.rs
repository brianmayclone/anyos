#![no_std]
#![no_main]

anyos_std::entry!(main);

/// Thread entry size from sysinfo(1).
const ENTRY_SIZE: usize = 64;
/// Max threads we can list.
const MAX_THREADS: usize = 128;

fn main() {
    anyos_std::i18n::init();
    let t = anyos_std::i18n::t;

    // Parse args: BSD-style "ps vax" (no dash)
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let mut flag_v = false; // verbose (memory, CPU%, I/O)
    let mut flag_a = false; // all users
    let mut flag_x = false; // include system/kernel threads

    // BSD style: "ps vax" or "ps -vax" — parse each char as a flag
    for i in 0..args.pos_count {
        let arg = args.positional[i];
        for &ch in arg.as_bytes() {
            match ch {
                b'v' | b'V' => flag_v = true,
                b'a' | b'A' => flag_a = true,
                b'x' | b'X' => flag_x = true,
                b'-' => {}
                _ => {}
            }
        }
    }
    // Also check if flags were passed with dash (e.g. -v -a -x)
    if args.has(b'v') { flag_v = true; }
    if args.has(b'a') { flag_a = true; }
    if args.has(b'x') { flag_x = true; }

    // Get own PID and UID
    let my_pid = anyos_std::process::getpid();
    let my_uid = anyos_std::process::getuid();

    // Fetch thread list
    let mut buf = [0u8; ENTRY_SIZE * MAX_THREADS];
    let count = anyos_std::sys::sysinfo(1, &mut buf);
    if count == u32::MAX || count == 0 {
        anyos_std::println!("{}", t("No threads running."));
        return;
    }
    let n = (count as usize).min(MAX_THREADS);

    // Parse all entries into a compact array for lookups
    let mut tids = [0u32; MAX_THREADS];
    let mut ppids = [0u32; MAX_THREADS];
    let mut uids = [0u16; MAX_THREADS];
    for i in 0..n {
        let off = i * ENTRY_SIZE;
        if off + ENTRY_SIZE > buf.len() { break; }
        tids[i] = u32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]);
        uids[i] = u16::from_le_bytes([buf[off + 56], buf[off + 57]]);
        ppids[i] = u32::from_le_bytes([buf[off+60], buf[off+61], buf[off+62], buf[off+63]]);
    }

    // For default mode: find the terminal session root.
    // Walk parent chain from ps (my_pid) upward to find the "Terminal" process.
    // Strategy: walk up parents, stop at "Terminal" (by name) or when parent
    // is not found in the process list / has PPID=0 (system process).
    let mut session_root = my_pid;
    if !flag_a && !flag_x {
        let mut current = my_pid;
        for _ in 0..16 {
            // Find current's entry in the list
            let mut cur_idx = usize::MAX;
            for j in 0..n {
                if tids[j] == current {
                    cur_idx = j;
                    break;
                }
            }
            if cur_idx == usize::MAX { break; } // current not in list

            // Check if current process is named "Terminal" — that's our session root
            let off = cur_idx * ENTRY_SIZE;
            let name_bytes = &buf[off + 8..off + 32];
            let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(24);
            let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("");
            if name == "Terminal" || name == "terminal" {
                session_root = current;
                break;
            }

            let parent = ppids[cur_idx];
            if parent == 0 || parent == current { break; }

            // Check if parent exists in the process list
            let mut parent_found = false;
            for j in 0..n {
                if tids[j] == parent {
                    parent_found = true;
                    break;
                }
            }
            if !parent_found { break; } // parent gone, stop here

            session_root = parent;
            current = parent;
        }
    }

    // Build set of TIDs to display (for default mode: session_root + all descendants)
    let mut visible = [false; MAX_THREADS];
    if !flag_a && !flag_x {
        // Mark session root
        for i in 0..n {
            if tids[i] == session_root { visible[i] = true; }
        }
        // Iteratively mark all descendants (breadth-first, multiple passes)
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..n {
                if visible[i] { continue; }
                // Check if parent is visible
                for j in 0..n {
                    if visible[j] && tids[j] == ppids[i] {
                        visible[i] = true;
                        changed = true;
                        break;
                    }
                }
            }
        }
    }

    // Pre-resolve uid→username cache
    let mut uid_cache: [(u16, [u8; 16], u8); 16] = [(0, [0u8; 16], 0); 16];
    let mut uid_cache_len = 0usize;

    for i in 0..n {
        let uid = uids[i];
        let found = uid_cache[..uid_cache_len].iter().any(|e| e.0 == uid);
        if !found && uid_cache_len < 16 {
            let mut name_buf = [0u8; 16];
            let nlen = anyos_std::process::getusername(uid, &mut name_buf);
            let len = if nlen != u32::MAX && nlen > 0 { (nlen as u8).min(15) } else { 0 };
            uid_cache[uid_cache_len] = (uid, name_buf, len);
            uid_cache_len += 1;
        }
    }

    // Print header
    if flag_v {
        anyos_std::println!(
            "{:<6} {:<6} {:<10} {:<6} {:<8} {:<8} {:<10} {:<10} {}",
            "TID", "PPID", "USER", "PRI", "STATE", "PAGES", "IO-R", "IO-W", "NAME"
        );
        anyos_std::println!("{}", "------------------------------------------------------------------------------------");
    } else {
        anyos_std::println!("{:<6} {:<6} {:<10} {:<6} {:<8} {}", "TID", "PPID", "USER", "PRI", "STATE", "NAME");
        anyos_std::println!("{}", "------------------------------------------------------");
    }

    let mut shown = 0u32;

    for i in 0..n {
        let off = i * ENTRY_SIZE;
        if off + ENTRY_SIZE > buf.len() { break; }

        let tid = tids[i];
        let prio = buf[off + 4];
        let state = buf[off + 5];
        let name_bytes = &buf[off + 8..off + 32];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(24);
        let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("???");
        let user_pages = u32::from_le_bytes([buf[off+32], buf[off+33], buf[off+34], buf[off+35]]);
        let _cpu_ticks = u32::from_le_bytes([buf[off+36], buf[off+37], buf[off+38], buf[off+39]]);
        let io_read = u64::from_le_bytes([
            buf[off+40], buf[off+41], buf[off+42], buf[off+43],
            buf[off+44], buf[off+45], buf[off+46], buf[off+47],
        ]);
        let io_write = u64::from_le_bytes([
            buf[off+48], buf[off+49], buf[off+50], buf[off+51],
            buf[off+52], buf[off+53], buf[off+54], buf[off+55],
        ]);
        let uid = uids[i];
        let parent_tid = ppids[i];

        // Filtering:
        // - Default (no flags): only terminal session (session_root + descendants)
        // - 'a': all users' processes (exclude system threads)
        // - 'x': own UID + system/kernel threads
        // - 'ax': everything

        if !flag_a && !flag_x {
            // Default: only processes in our terminal session tree
            if !visible[i] { continue; }
        } else if flag_a && !flag_x {
            // 'a': all users, but exclude system threads (uid=0, no parent)
            let is_system = uid == 0 && parent_tid == 0 && tid <= 4;
            if is_system { continue; }
        } else if !flag_a && flag_x {
            // 'x': own UID + system/kernel threads
            let is_system = uid == 0 && parent_tid == 0 && tid <= 4;
            if uid != my_uid && !is_system { continue; }
        }
        // 'ax': show everything (no filter)

        let state_str = match state {
            0 => t("Ready"),
            1 => t("Running"),
            2 => t("Blocked"),
            3 => t("Dead"),
            4 => t("Stopped"),
            _ => t("Unknown"),
        };

        // Username lookup
        let username = uid_cache[..uid_cache_len]
            .iter()
            .find(|e| e.0 == uid)
            .and_then(|e| {
                if e.2 > 0 {
                    core::str::from_utf8(&e.1[..e.2 as usize]).ok()
                } else {
                    None
                }
            })
            .unwrap_or("?");

        if flag_v {
            anyos_std::println!(
                "{:<6} {:<6} {:<10} {:<6} {:<8} {:<8} {:<10} {:<10} {}",
                tid, parent_tid, username, prio, state_str,
                user_pages, format_bytes(io_read), format_bytes(io_write), name
            );
        } else {
            anyos_std::println!(
                "{:<6} {:<6} {:<10} {:<6} {:<8} {}",
                tid, parent_tid, username, prio, state_str, name
            );
        }
        shown += 1;
    }

    anyos_std::println!("\n{}: {} {}", t("Total"), shown, t("thread(s)"));
}

/// Format byte count as human-readable (B, KB, MB).
fn format_bytes(bytes: u64) -> &'static str {
    // Use a static buffer ring for formatting (max 4 concurrent)
    static mut BUFS: [[u8; 16]; 4] = [[0u8; 16]; 4];
    static mut IDX: usize = 0;
    unsafe {
        let i = IDX;
        IDX = (IDX + 1) % 4;
        let buf = &mut BUFS[i];
        buf.fill(0);
        let len = if bytes >= 1024 * 1024 {
            let mb = bytes / (1024 * 1024);
            let frac = (bytes % (1024 * 1024)) * 10 / (1024 * 1024);
            write_u64_unit(buf, mb, frac as u32, b'M')
        } else if bytes >= 1024 {
            let kb = bytes / 1024;
            let frac = (bytes % 1024) * 10 / 1024;
            write_u64_unit(buf, kb, frac as u32, b'K')
        } else {
            write_u64_plain(buf, bytes, b'B')
        };
        core::str::from_utf8(&buf[..len]).unwrap_or("?")
    }
}

fn write_u64_unit(buf: &mut [u8; 16], val: u64, frac: u32, unit: u8) -> usize {
    let mut pos = write_u64_digits(buf, val, 0);
    if frac > 0 && pos < 14 {
        buf[pos] = b'.';
        pos += 1;
        buf[pos] = b'0' + (frac as u8).min(9);
        pos += 1;
    }
    if pos < 15 {
        buf[pos] = unit;
        pos += 1;
    }
    pos
}

fn write_u64_plain(buf: &mut [u8; 16], val: u64, unit: u8) -> usize {
    let mut pos = write_u64_digits(buf, val, 0);
    if pos < 15 {
        buf[pos] = unit;
        pos += 1;
    }
    pos
}

fn write_u64_digits(buf: &mut [u8; 16], val: u64, start: usize) -> usize {
    if val == 0 {
        buf[start] = b'0';
        return start + 1;
    }
    let mut tmp = [0u8; 20];
    let mut n = val;
    let mut len = 0;
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    let mut pos = start;
    for i in (0..len).rev() {
        if pos >= 16 { break; }
        buf[pos] = tmp[i];
        pos += 1;
    }
    pos
}
