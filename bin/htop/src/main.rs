#![no_std]
#![no_main]

anyos_std::entry!(main);

const MAX_TASKS: usize = 64;
const MAX_CPUS: usize = 16;
const THREAD_ENTRY_SIZE: usize = 60;
const REFRESH_MS: u32 = 2000;
const BAR_WIDTH: usize = 20;
/// Maximum visible rows in terminal (safe default — prevents scrolling).
const MAX_VISIBLE_ROWS: usize = 24;

// ANSI color codes
const RESET: &str = "\x1B[0m";
const BOLD: &str = "\x1B[1m";
const DIM: &str = "\x1B[90m";    // bright black = dim gray
const RED: &str = "\x1B[31m";
const GREEN: &str = "\x1B[32m";
const YELLOW: &str = "\x1B[33m";
const CYAN: &str = "\x1B[36m";
const BRIGHT_WHITE: &str = "\x1B[97m";

// ─── Data Structures ─────────────────────────────────────────────────────────

struct PrevTicks {
    entries: [(u32, u32); MAX_TASKS],
    count: usize,
    prev_total: u32,
}

struct CpuState {
    num_cpus: u32,
    total_sched_ticks: u32,
    overall_pct: u32,
    core_pct: [u32; MAX_CPUS],
    prev_total: u32,
    prev_idle: u32,
    prev_core_total: [u32; MAX_CPUS],
    prev_core_idle: [u32; MAX_CPUS],
}

impl CpuState {
    const fn new() -> Self {
        CpuState {
            num_cpus: 1,
            total_sched_ticks: 0,
            overall_pct: 0,
            core_pct: [0; MAX_CPUS],
            prev_total: 0,
            prev_idle: 0,
            prev_core_total: [0; MAX_CPUS],
            prev_core_idle: [0; MAX_CPUS],
        }
    }
}

#[derive(Clone, Copy)]
struct TaskEntry {
    tid: u32,
    name: [u8; 24],
    name_len: usize,
    state: u8,
    priority: u8,
    uid: u16,
    user_pages: u32,
    cpu_pct_x10: u32,
}

// ─── Formatting Helpers ──────────────────────────────────────────────────────

/// Format u32 to decimal string.
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

/// Format CPU% (pct_x10: 123 → "12.3%").
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

/// Format memory pages as human-readable.
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

// ─── Data Fetching ───────────────────────────────────────────────────────────

/// Fetch per-core CPU load and overall stats.
fn fetch_cpu(state: &mut CpuState) {
    let mut buf = [0u8; 16 + 8 * MAX_CPUS];
    anyos_std::sys::sysinfo(3, &mut buf);

    let total = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let idle = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ncpu = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    state.num_cpus = ncpu.max(1).min(MAX_CPUS as u32);
    state.total_sched_ticks = total;

    let dt = total.wrapping_sub(state.prev_total);
    let di = idle.wrapping_sub(state.prev_idle);
    state.overall_pct = if dt > 0 {
        100u32.saturating_sub(di.saturating_mul(100) / dt)
    } else {
        0
    };
    state.prev_total = total;
    state.prev_idle = idle;

    for i in 0..(state.num_cpus as usize).min(MAX_CPUS) {
        let off = 16 + i * 8;
        if off + 8 > buf.len() { break; }
        let ct = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let ci = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);
        let dct = ct.wrapping_sub(state.prev_core_total[i]);
        let dci = ci.wrapping_sub(state.prev_core_idle[i]);
        state.core_pct[i] = if dct > 0 {
            100u32.saturating_sub(dci.saturating_mul(100) / dct)
        } else {
            0
        };
        state.prev_core_total[i] = ct;
        state.prev_core_idle[i] = ci;
    }
}

/// Fetch thread list with per-thread CPU% delta.
fn fetch_tasks(
    raw: &mut [u8; THREAD_ENTRY_SIZE * MAX_TASKS],
    prev: &mut PrevTicks,
    total_sched_ticks: u32,
    out: &mut [TaskEntry; MAX_TASKS],
) -> usize {
    let count = anyos_std::sys::sysinfo(1, raw);
    if count == u32::MAX || count == 0 { return 0; }
    let n = (count as usize).min(MAX_TASKS);
    let dt = total_sched_ticks.wrapping_sub(prev.prev_total);

    for i in 0..n {
        let off = i * THREAD_ENTRY_SIZE;
        let tid = u32::from_le_bytes([raw[off], raw[off+1], raw[off+2], raw[off+3]]);
        let prio = raw[off + 4];
        let state = raw[off + 5];
        let mut name = [0u8; 24];
        name.copy_from_slice(&raw[off + 8..off + 32]);
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(24);
        let user_pages = u32::from_le_bytes([raw[off+32], raw[off+33], raw[off+34], raw[off+35]]);
        let cpu_ticks = u32::from_le_bytes([raw[off+36], raw[off+37], raw[off+38], raw[off+39]]);
        let uid = u16::from_le_bytes([raw[off + 56], raw[off + 57]]);

        let prev_ticks = prev.entries[..prev.count]
            .iter().find(|e| e.0 == tid).map(|e| e.1).unwrap_or(cpu_ticks);
        let d_ticks = cpu_ticks.wrapping_sub(prev_ticks);
        let cpu_pct_x10 = if dt > 0 && d_ticks > 0 {
            (d_ticks as u64 * 1000 / dt as u64).min(1000) as u32
        } else { 0 };

        out[i] = TaskEntry { tid, name, name_len, state, priority: prio, uid, user_pages, cpu_pct_x10 };
    }

    prev.count = n;
    for i in 0..n {
        let off = i * THREAD_ENTRY_SIZE;
        let tid = u32::from_le_bytes([raw[off], raw[off+1], raw[off+2], raw[off+3]]);
        let cpu_ticks = u32::from_le_bytes([raw[off+36], raw[off+37], raw[off+38], raw[off+39]]);
        prev.entries[i] = (tid, cpu_ticks);
    }
    prev.prev_total = total_sched_ticks;
    n
}

/// Sort by CPU% descending (insertion sort).
fn sort_by_cpu_desc(tasks: &mut [TaskEntry], n: usize) {
    for i in 1..n {
        let mut j = i;
        while j > 0 && tasks[j].cpu_pct_x10 > tasks[j - 1].cpu_pct_x10 {
            tasks.swap(j, j - 1);
            j -= 1;
        }
    }
}

// ─── Bar Rendering ───────────────────────────────────────────────────────────

/// Pick ANSI color code for a percentage (green/yellow/red thresholds).
fn pct_color(pct: u32, yellow_thresh: u32, red_thresh: u32) -> &'static str {
    if pct >= red_thresh { RED }
    else if pct >= yellow_thresh { YELLOW }
    else { GREEN }
}

/// Print a colored bar: `label[||||       XX%]`
/// `label_width` is the fixed-width label area (right-aligned).
fn print_bar(label: &str, pct: u32, color: &str) {
    let filled = (pct as usize * BAR_WIDTH / 100).min(BAR_WIDTH);
    let empty = BAR_WIDTH - filled;

    anyos_std::print!("{}[", label);
    anyos_std::print!("{}", color);
    for _ in 0..filled {
        anyos_std::print!("|");
    }
    anyos_std::print!("{}", RESET);
    for _ in 0..empty {
        anyos_std::print!(" ");
    }

    let mut t = [0u8; 12];
    let pct_s = fmt_u32(&mut t, pct);
    // Right-align percentage: pad to 3 chars
    if pct < 10 {
        anyos_std::print!("  {}%]", pct_s);
    } else if pct < 100 {
        anyos_std::print!(" {}%]", pct_s);
    } else {
        anyos_std::print!("{}%]", pct_s);
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let mut raw_buf = [0u8; THREAD_ENTRY_SIZE * MAX_TASKS];
    let mut prev = PrevTicks { entries: [(0, 0); MAX_TASKS], count: 0, prev_total: 0 };
    let mut cpu_state = CpuState::new();

    const EMPTY_TASK: TaskEntry = TaskEntry {
        tid: 0, name: [0; 24], name_len: 0, state: 0,
        priority: 0, uid: 0, user_pages: 0, cpu_pct_x10: 0,
    };
    let mut tasks = [EMPTY_TASK; MAX_TASKS];

    let mut uid_cache: [(u16, [u8; 16], u8); 16] = [(0, [0u8; 16], 0); 16];
    let mut prev_line_count: usize = 0;
    let mut first_frame = true;

    // Prime CPU delta counters
    fetch_cpu(&mut cpu_state);

    loop {
        fetch_cpu(&mut cpu_state);
        let task_count = fetch_tasks(&mut raw_buf, &mut prev, cpu_state.total_sched_ticks, &mut tasks);
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

        // Memory info
        let mut mem_buf = [0u8; 16];
        anyos_std::sys::sysinfo(0, &mut mem_buf);
        let total_frames = u32::from_le_bytes([mem_buf[0], mem_buf[1], mem_buf[2], mem_buf[3]]);
        let free_frames = u32::from_le_bytes([mem_buf[4], mem_buf[5], mem_buf[6], mem_buf[7]]);
        let heap_used = u32::from_le_bytes([mem_buf[8], mem_buf[9], mem_buf[10], mem_buf[11]]);
        let heap_total = u32::from_le_bytes([mem_buf[12], mem_buf[13], mem_buf[14], mem_buf[15]]);
        let used_kb = total_frames.saturating_sub(free_frames) * 4;
        let total_kb = total_frames * 4;
        let mem_pct = if total_kb > 0 { (used_kb as u64 * 100 / total_kb as u64) as u32 } else { 0 };
        let heap_pct = if heap_total > 0 { (heap_used as u64 * 100 / heap_total as u64) as u32 } else { 0 };

        // Uptime
        let ticks = anyos_std::sys::uptime();
        let hz = anyos_std::sys::tick_hz();
        let total_secs = if hz > 0 { ticks / hz } else { 0 };
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;

        // ── Render ──
        if first_frame {
            anyos_std::print!("\x1B[2J");
            first_frame = false;
        }
        anyos_std::print!("\x1B[H");
        let mut line_count: usize = 0;
        let mut t = [0u8; 12];

        // ── Per-core CPU bars (2-column layout) ──
        let ncpu = (cpu_state.num_cpus as usize).min(MAX_CPUS);
        let rows = (ncpu + 1) / 2; // ceil(ncpu / 2)
        for row in 0..rows {
            let left = row * 2;
            let right = left + 1;

            // Left bar
            // Label: core number, padded to 2 chars
            if left < 10 {
                anyos_std::print!("{}", fmt_u32(&mut t, left as u32));
            } else {
                anyos_std::print!("{}", fmt_u32(&mut t, left as u32));
            }
            let left_pct = cpu_state.core_pct[left];
            print_bar("", left_pct, pct_color(left_pct, 50, 75));

            // Right bar (if exists)
            if right < ncpu {
                anyos_std::print!("  ");
                anyos_std::print!("{}", fmt_u32(&mut t, right as u32));
                let right_pct = cpu_state.core_pct[right];
                print_bar("", right_pct, pct_color(right_pct, 50, 75));
            }

            anyos_std::println!("\x1B[K");
            line_count += 1;
        }

        // ── Memory bar ──
        anyos_std::print!("Mem ");
        print_bar("", mem_pct, pct_color(mem_pct, 60, 80));
        anyos_std::print!("  {}", fmt_u32(&mut t, used_kb / 1024));
        anyos_std::print!("/{}M", fmt_u32(&mut t, total_kb / 1024));
        anyos_std::println!("\x1B[K");
        line_count += 1;

        // ── Heap bar ──
        anyos_std::print!("Heap");
        print_bar("", heap_pct, CYAN);
        anyos_std::print!("  {}", fmt_u32(&mut t, heap_used / 1024));
        anyos_std::print!("/{}K", fmt_u32(&mut t, heap_total / 1024));
        anyos_std::println!("\x1B[K");
        line_count += 1;

        // ── Summary line ──
        anyos_std::print!("{}Tasks:{} {}", BOLD, RESET, fmt_u32(&mut t, task_count as u32));
        anyos_std::print!(", {}{}run{}", GREEN, fmt_u32(&mut t, running), RESET);
        anyos_std::print!(", {}{}blk{}", RED, fmt_u32(&mut t, blocked), RESET);
        anyos_std::print!("        {}Uptime:{} ", BOLD, RESET);
        if hours > 0 {
            anyos_std::print!("{}h ", fmt_u32(&mut t, hours as u32));
        }
        anyos_std::print!("{}m ", fmt_u32(&mut t, mins as u32));
        anyos_std::print!("{}s", fmt_u32(&mut t, secs as u32));
        anyos_std::println!("\x1B[K");
        line_count += 1;

        // ── Blank separator ──
        anyos_std::println!("\x1B[K");
        line_count += 1;

        // ── Table header ──
        anyos_std::print!("{}{}", BOLD, CYAN);
        anyos_std::print!("{:>5} {:<10} {:>3} {:<8} {:>5} {:>6} {}", "TID", "USER", "PRI", "STATE", "CPU%", "MEM", "NAME");
        anyos_std::print!("{}", RESET);
        anyos_std::println!("\x1B[K");
        line_count += 1;

        anyos_std::print!("{}", DIM);
        anyos_std::print!("------------------------------------------------------");
        anyos_std::print!("{}", RESET);
        anyos_std::println!("\x1B[K");
        line_count += 1;

        // ── Process rows (limited to fit terminal height) ──
        let max_proc_rows = MAX_VISIBLE_ROWS.saturating_sub(line_count);
        let visible_tasks = task_count.min(max_proc_rows);
        for i in 0..visible_tasks {
            let task = &tasks[i];
            let name = core::str::from_utf8(&task.name[..task.name_len]).unwrap_or("???");

            let (state_str, state_color) = match task.state {
                0 => ("Ready", YELLOW),
                1 => ("Running", GREEN),
                2 => ("Blocked", RED),
                3 => ("Dead", DIM),
                _ => ("?", RESET),
            };

            let username = uid_cache[..uid_cache_len]
                .iter()
                .find(|e| e.0 == task.uid)
                .and_then(|e| {
                    if e.2 > 0 { core::str::from_utf8(&e.1[..e.2 as usize]).ok() } else { None }
                })
                .unwrap_or("?");

            let mut cbuf = [0u8; 12];
            let cpu_str = if task.cpu_pct_x10 > 0 { fmt_pct(&mut cbuf, task.cpu_pct_x10) } else { "0.0%" };

            let mut mbuf = [0u8; 16];
            let mem_str = fmt_mem(&mut mbuf, task.user_pages);

            // TID in cyan
            anyos_std::print!("{}{:>5}{} ", CYAN, task.tid, RESET);
            // User
            anyos_std::print!("{:<10} ", username);
            // Priority
            anyos_std::print!("{:>3} ", task.priority);
            // State colored
            anyos_std::print!("{}{:<8}{} ", state_color, state_str, RESET);
            // CPU% — red if high
            if task.cpu_pct_x10 > 50 {
                anyos_std::print!("{}{:>5}{} ", RED, cpu_str, RESET);
            } else {
                anyos_std::print!("{:>5} ", cpu_str);
            }
            // MEM
            anyos_std::print!("{:>6} ", mem_str);
            // Name
            anyos_std::print!("{}", name);

            anyos_std::println!("\x1B[K");
            line_count += 1;
        }

        // Clear leftover lines from previous frame
        while line_count < prev_line_count {
            anyos_std::println!("\x1B[K");
            line_count += 1;
        }
        prev_line_count = line_count;

        anyos_std::process::sleep(REFRESH_MS);
    }
}
