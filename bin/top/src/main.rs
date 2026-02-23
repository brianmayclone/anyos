#![no_std]
#![no_main]

anyos_std::entry!(main);

const MAX_TASKS: usize = 64;
const THREAD_ENTRY_SIZE: usize = 60;
/// Refresh interval in milliseconds.
const REFRESH_MS: u32 = 2000;

// ─── Per-thread previous tick tracking for CPU% delta ────────────────────────

struct PrevTicks {
    entries: [(u32, u32); MAX_TASKS], // (tid, cpu_ticks)
    count: usize,
    prev_total: u32,
}

struct CpuPrev {
    prev_total: u32,
    prev_idle: u32,
}

// ─── Parsed thread entry ─────────────────────────────────────────────────────

struct TaskEntry {
    tid: u32,
    name: [u8; 24],
    name_len: usize,
    state: u8,
    priority: u8,
    uid: u16,
    user_pages: u32,
    cpu_pct_x10: u32, // CPU% * 10 (e.g. 123 = 12.3%)
}

// ─── Formatting helpers ──────────────────────────────────────────────────────

/// Format a u32 into a decimal string, returning the str slice.
fn fmt_u32<'a>(buf: &'a mut [u8; 12], val: u32) -> &'a str {
    if val == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut v = val;
    let mut tmp = [0u8; 12];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

/// Format CPU percentage (value is pct*10, e.g. 123 → "12.3%").
fn fmt_pct<'a>(buf: &'a mut [u8; 12], pct_x10: u32) -> &'a str {
    let whole = pct_x10 / 10;
    let frac = pct_x10 % 10;
    let mut p = 0;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, whole);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p] = b'.';
    p += 1;
    buf[p] = b'0' + frac as u8;
    p += 1;
    buf[p] = b'%';
    p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

/// Format memory pages as human-readable (pages * 4 KiB).
fn fmt_mem<'a>(buf: &'a mut [u8; 16], pages: u32) -> &'a str {
    let kib = pages * 4;
    let mut t = [0u8; 12];
    let mut p = 0;
    if kib >= 1024 {
        let mib = kib / 1024;
        let frac = (kib % 1024) * 10 / 1024;
        let s = fmt_u32(&mut t, mib);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        buf[p] = b'.';
        p += 1;
        buf[p] = b'0' + frac as u8;
        p += 1;
        buf[p] = b'M';
        p += 1;
    } else {
        let s = fmt_u32(&mut t, kib);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        buf[p] = b'K';
        p += 1;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

// ─── Data fetching ───────────────────────────────────────────────────────────

/// Fetch thread list and compute per-thread CPU% delta.
fn fetch_tasks(
    raw: &mut [u8; THREAD_ENTRY_SIZE * MAX_TASKS],
    prev: &mut PrevTicks,
    total_sched_ticks: u32,
    out: &mut [TaskEntry; MAX_TASKS],
) -> usize {
    let count = anyos_std::sys::sysinfo(1, raw);
    if count == u32::MAX || count == 0 {
        return 0;
    }
    let n = (count as usize).min(MAX_TASKS);
    let dt = total_sched_ticks.wrapping_sub(prev.prev_total);

    for i in 0..n {
        let off = i * THREAD_ENTRY_SIZE;
        let tid = u32::from_le_bytes([raw[off], raw[off + 1], raw[off + 2], raw[off + 3]]);
        let prio = raw[off + 4];
        let state = raw[off + 5];
        let mut name = [0u8; 24];
        name.copy_from_slice(&raw[off + 8..off + 32]);
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(24);
        let user_pages = u32::from_le_bytes([raw[off + 32], raw[off + 33], raw[off + 34], raw[off + 35]]);
        let cpu_ticks = u32::from_le_bytes([raw[off + 36], raw[off + 37], raw[off + 38], raw[off + 39]]);
        let uid = u16::from_le_bytes([raw[off + 56], raw[off + 57]]);

        let prev_ticks = prev.entries[..prev.count]
            .iter()
            .find(|e| e.0 == tid)
            .map(|e| e.1)
            .unwrap_or(cpu_ticks);

        let d_ticks = cpu_ticks.wrapping_sub(prev_ticks);
        let cpu_pct_x10 = if dt > 0 && d_ticks > 0 {
            (d_ticks as u64 * 1000 / dt as u64).min(1000) as u32
        } else {
            0
        };

        out[i] = TaskEntry { tid, name, name_len, state, priority: prio, uid, user_pages, cpu_pct_x10 };
    }

    // Save current ticks for next delta
    prev.count = n;
    for i in 0..n {
        let off = i * THREAD_ENTRY_SIZE;
        let tid = u32::from_le_bytes([raw[off], raw[off + 1], raw[off + 2], raw[off + 3]]);
        let cpu_ticks = u32::from_le_bytes([raw[off + 36], raw[off + 37], raw[off + 38], raw[off + 39]]);
        prev.entries[i] = (tid, cpu_ticks);
    }
    prev.prev_total = total_sched_ticks;

    n
}

/// Fetch overall CPU load. Returns (overall_pct, total_sched_ticks).
fn fetch_cpu(cpu_prev: &mut CpuPrev) -> (u32, u32) {
    let mut buf = [0u8; 16 + 8 * 16];
    anyos_std::sys::sysinfo(3, &mut buf);

    let total = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let idle = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);

    let dt = total.wrapping_sub(cpu_prev.prev_total);
    let di = idle.wrapping_sub(cpu_prev.prev_idle);
    let pct = if dt > 0 {
        100u32.saturating_sub(di.saturating_mul(100) / dt)
    } else {
        0
    };

    cpu_prev.prev_total = total;
    cpu_prev.prev_idle = idle;

    (pct, total)
}

/// Sort tasks by CPU% descending (simple insertion sort — max 64 entries).
fn sort_by_cpu_desc(tasks: &mut [TaskEntry], n: usize) {
    for i in 1..n {
        let mut j = i;
        while j > 0 && tasks[j].cpu_pct_x10 > tasks[j - 1].cpu_pct_x10 {
            tasks.swap(j, j - 1);
            j -= 1;
        }
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let mut raw_buf = [0u8; THREAD_ENTRY_SIZE * MAX_TASKS];
    let mut prev = PrevTicks {
        entries: [(0, 0); MAX_TASKS],
        count: 0,
        prev_total: 0,
    };
    let mut cpu_prev = CpuPrev { prev_total: 0, prev_idle: 0 };

    // Default task entry for array init
    const EMPTY_TASK: TaskEntry = TaskEntry {
        tid: 0, name: [0; 24], name_len: 0, state: 0,
        priority: 0, uid: 0, user_pages: 0, cpu_pct_x10: 0,
    };
    let mut tasks = [EMPTY_TASK; MAX_TASKS];

    // Pre-resolve uid→username cache (max 16 unique users)
    let mut uid_cache: [(u16, [u8; 16], u8); 16] = [(0, [0u8; 16], 0); 16];

    // Track how many lines we wrote last frame so we can clear leftover rows
    let mut prev_line_count: usize = 0;
    let mut first_frame = true;

    // Prime the CPU delta counters (first frame will show 0% CPU)
    fetch_cpu(&mut cpu_prev);

    loop {
        let (cpu_pct, total_sched_ticks) = fetch_cpu(&mut cpu_prev);
        let task_count = fetch_tasks(&mut raw_buf, &mut prev, total_sched_ticks, &mut tasks);
        sort_by_cpu_desc(&mut tasks, task_count);

        // Resolve usernames
        let mut uid_cache_len = 0usize;
        for i in 0..task_count {
            let uid = tasks[i].uid;
            let found = uid_cache[..uid_cache_len].iter().any(|e| e.0 == uid);
            if !found && uid_cache_len < 16 {
                let mut name_buf = [0u8; 16];
                let nlen = anyos_std::process::getusername(uid, &mut name_buf);
                let len = if nlen != u32::MAX && nlen > 0 { (nlen as u8).min(15) } else { 0 };
                uid_cache[uid_cache_len] = (uid, name_buf, len);
                uid_cache_len += 1;
            }
        }

        // Count states
        let mut running = 0u32;
        let mut blocked = 0u32;
        for i in 0..task_count {
            match tasks[i].state {
                1 => running += 1,
                2 => blocked += 1,
                _ => {}
            }
        }

        // Fetch memory info
        let mut mem_buf = [0u8; 16];
        anyos_std::sys::sysinfo(0, &mut mem_buf);
        let total_frames = u32::from_le_bytes([mem_buf[0], mem_buf[1], mem_buf[2], mem_buf[3]]);
        let free_frames = u32::from_le_bytes([mem_buf[4], mem_buf[5], mem_buf[6], mem_buf[7]]);
        let heap_used = u32::from_le_bytes([mem_buf[8], mem_buf[9], mem_buf[10], mem_buf[11]]);
        let heap_total = u32::from_le_bytes([mem_buf[12], mem_buf[13], mem_buf[14], mem_buf[15]]);
        let used_kb = total_frames.saturating_sub(free_frames) * 4;
        let total_kb = total_frames * 4;
        let mem_pct = if total_kb > 0 { used_kb as u64 * 100 / total_kb as u64 } else { 0 };

        // Uptime
        let ticks = anyos_std::sys::uptime();
        let hz = anyos_std::sys::tick_hz();
        let total_secs = if hz > 0 { ticks / hz } else { 0 };
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;

        // ── Render ──
        // Flicker-free: on first frame clear screen, then only cursor-home +
        // overwrite each line with \x1B[K (erase to EOL) to avoid blanking.

        if first_frame {
            anyos_std::print!("\x1B[2J");
            first_frame = false;
        }
        // Cursor home (top-left)
        anyos_std::print!("\x1B[H");

        let mut line_count: usize = 0;

        // Line 1: top header
        let mut t = [0u8; 12];
        anyos_std::print!("top - up ");
        if hours > 0 {
            anyos_std::print!("{}h ", fmt_u32(&mut t, hours as u32));
        }
        anyos_std::print!("{}m ", fmt_u32(&mut t, mins as u32));
        anyos_std::print!("{}s", fmt_u32(&mut t, secs as u32));
        anyos_std::print!("   Tasks: {}", fmt_u32(&mut t, task_count as u32));
        anyos_std::print!(", {} run", fmt_u32(&mut t, running));
        anyos_std::print!(", {} blk", fmt_u32(&mut t, blocked));
        anyos_std::println!("\x1B[K");
        line_count += 1;

        // Line 2: CPU + Memory
        anyos_std::print!("CPU: {}%", fmt_u32(&mut t, cpu_pct));
        anyos_std::print!("   Mem: {}", fmt_u32(&mut t, used_kb / 1024));
        anyos_std::print!("/{}M", fmt_u32(&mut t, total_kb / 1024));
        anyos_std::print!(" ({}%)", fmt_u32(&mut t, mem_pct as u32));
        anyos_std::print!("   Heap: {}", fmt_u32(&mut t, heap_used / 1024));
        anyos_std::print!("/{}K", fmt_u32(&mut t, heap_total / 1024));
        anyos_std::println!("\x1B[K");
        line_count += 1;

        // Blank line
        anyos_std::println!("\x1B[K");
        line_count += 1;

        // Table header
        anyos_std::print!("{:>5} {:<10} {:>3} {:<8} {:>5} {:>6} {}", "TID", "USER", "PRI", "STATE", "CPU%", "MEM", "NAME");
        anyos_std::println!("\x1B[K");
        line_count += 1;
        anyos_std::print!("------------------------------------------------------");
        anyos_std::println!("\x1B[K");
        line_count += 1;

        // Table rows
        for i in 0..task_count {
            let task = &tasks[i];
            let name = core::str::from_utf8(&task.name[..task.name_len]).unwrap_or("???");

            let state_str = match task.state {
                0 => "Ready",
                1 => "Running",
                2 => "Blocked",
                3 => "Dead",
                _ => "?",
            };

            let username = uid_cache[..uid_cache_len]
                .iter()
                .find(|e| e.0 == task.uid)
                .and_then(|e| {
                    if e.2 > 0 {
                        core::str::from_utf8(&e.1[..e.2 as usize]).ok()
                    } else {
                        None
                    }
                })
                .unwrap_or("?");

            let mut cbuf = [0u8; 12];
            let cpu_str = if task.cpu_pct_x10 > 0 {
                fmt_pct(&mut cbuf, task.cpu_pct_x10)
            } else {
                "0.0%"
            };

            let mut mbuf = [0u8; 16];
            let mem_str = fmt_mem(&mut mbuf, task.user_pages);

            anyos_std::print!("{:>5} {:<10} {:>3} {:<8} {:>5} {:>6} {}",
                task.tid, username, task.priority, state_str, cpu_str, mem_str, name);
            anyos_std::println!("\x1B[K");
            line_count += 1;
        }

        // Clear any leftover lines from previous frame (e.g. a process exited)
        while line_count < prev_line_count {
            anyos_std::println!("\x1B[K");
            line_count += 1;
        }
        prev_line_count = line_count;

        anyos_std::process::sleep(REFRESH_MS);
    }
}
