//! Terminal rendering for htop — bars, header, process rows, fnbar.

use anyos_std::fmt::{fmt_u32, fmt_pct};
use crate::data::{CpuState, TaskEntry};

// ─── ANSI color constants ─────────────────────────────────────────────────────

pub const RESET:     &str = "\x1B[0m";
pub const BOLD:      &str = "\x1B[1m";
pub const FG_BLACK:  &str = "\x1B[30m";
pub const FG_BBLACK: &str = "\x1B[90m";
pub const FG_BRED:   &str = "\x1B[91m";
pub const FG_BGREEN: &str = "\x1B[92m";
pub const FG_BYELLOW:&str = "\x1B[93m";
pub const FG_BWHITE: &str = "\x1B[97m";
pub const BG_GREEN:  &str = "\x1B[42m";
pub const BG_BGREEN: &str = "\x1B[102m";
pub const BG_BCYAN:  &str = "\x1B[106m";

// ─── Memory formatting ────────────────────────────────────────────────────────

/// Format KB with one decimal place and G/M/K suffix: "8.3G", "512M", "64K".
pub fn fmt_mem_kb<'a>(buf: &'a mut [u8; 12], kb: u32) -> &'a str {
    let (whole, frac, suffix) = if kb >= 1024 * 1024 {
        let g10 = (kb as u64 * 10 / (1024 * 1024)) as u32;
        (g10 / 10, g10 % 10, b'G')
    } else if kb >= 1024 {
        let m10 = (kb as u64 * 10 / 1024) as u32;
        (m10 / 10, m10 % 10, b'M')
    } else {
        (kb, 0, b'K')
    };
    let mut tmp = [0u8; 10];
    let mut pos = 10usize;
    if whole == 0 { pos -= 1; tmp[pos] = b'0'; } else {
        let mut v = whole;
        while v > 0 { pos -= 1; tmp[pos] = b'0' + (v % 10) as u8; v /= 10; }
    }
    let dl = 10 - pos;
    buf[..dl].copy_from_slice(&tmp[pos..]);
    let mut out_len = dl;
    if frac > 0 { buf[out_len] = b'.'; out_len += 1; buf[out_len] = b'0' + frac as u8; out_len += 1; }
    buf[out_len] = suffix; out_len += 1;
    core::str::from_utf8(&buf[..out_len]).unwrap_or("?")
}

/// Format KB as short string for process list: "123M", "4G", "56K".
pub fn fmt_mem_short<'a>(buf: &'a mut [u8; 12], kb: u32) -> &'a str {
    let (val, suffix) = if kb >= 1024 * 1024 {
        (kb / (1024 * 1024), b'G')
    } else if kb >= 1024 {
        (kb / 1024, b'M')
    } else {
        (kb, b'K')
    };
    let mut tmp = [0u8; 10];
    let mut pos = 10usize;
    if val == 0 { pos -= 1; tmp[pos] = b'0'; } else {
        let mut v = val;
        while v > 0 { pos -= 1; tmp[pos] = b'0' + (v % 10) as u8; v /= 10; }
    }
    let dl = 10 - pos;
    buf[..dl].copy_from_slice(&tmp[pos..]);
    buf[dl] = suffix;
    core::str::from_utf8(&buf[..dl + 1]).unwrap_or("?")
}

// ─── Bar rendering ────────────────────────────────────────────────────────────

/// Print a fixed-width bar: `[||||   XX%]` — exactly `width` columns.
/// Percentage is always shown as " X%" (3 chars, space-padded), so the
/// suffix `" X%]"` is always 4 chars → `fill_area = width - 5`.
pub fn print_bar_fixed(pct: u32, width: usize, fill_bg: &str, fill_fg: &str) {
    // Clamp to 99 so the suffix " X%]" / "XX%]" never exceeds 4 chars (100% → "99%")
    let pct = pct.min(99);
    if width < 6 { anyos_std::print!("[{:>2}%]", pct); return; }
    let fill_area = width - 5; // '[' + fill + ' X%]' (4)
    let filled = ((pct as usize) * fill_area / 100).min(fill_area);
    let empty  = fill_area - filled;

    anyos_std::print!("[");
    if filled > 0 {
        anyos_std::print!("{}{}", fill_bg, fill_fg);
        for _ in 0..filled { anyos_std::print!("|"); }
        anyos_std::print!("{}", RESET);
    }
    for _ in 0..empty { anyos_std::print!(" "); }
    let mut t = [0u8; 12];
    anyos_std::print!("{:>3}%]", fmt_u32(&mut t, pct));
}

/// Print `Mem[...fill... X.XG/Y.YG]` — exactly `total_width` columns.
pub fn print_mem_bar(used_kb: u32, total_kb: u32, total_width: usize) {
    let pct = if total_kb > 0 { (used_kb as u64 * 100 / total_kb as u64) as u32 } else { 0 };
    let mut sa = [0u8; 12]; let mut sb = [0u8; 12];
    let used_s  = fmt_mem_kb(&mut sa, used_kb);
    let total_s = fmt_mem_kb(&mut sb, total_kb);
    let size_len = used_s.len() + 1 + total_s.len();
    // "Mem[" (4) + fill_area + size_len + "]" (1)
    let overhead = 5 + size_len;
    let fill_area = if total_width > overhead { total_width - overhead } else { 0 };
    let filled = ((pct as usize) * fill_area / 100).min(fill_area);
    let empty  = fill_area - filled;

    anyos_std::print!("Mem[");
    if filled > 0 {
        anyos_std::print!("{}{}", BG_BGREEN, FG_BLACK);
        for _ in 0..filled { anyos_std::print!("|"); }
        anyos_std::print!("{}", RESET);
    }
    for _ in 0..empty { anyos_std::print!(" "); }
    anyos_std::print!("{}{}/{}{}", FG_BWHITE, used_s, total_s, RESET);
    anyos_std::print!("]");
}

/// Print `Swp[   0K/0K]` — exactly `total_width` columns (anyOS has no swap).
pub fn print_swp_bar(total_width: usize) {
    // "Swp[" (4) + inner + "]" (1) = total_width  → inner = total_width - 5
    let inner = if total_width > 5 { total_width - 5 } else { 0 };
    // "0K/0K" = 5 chars; pad the rest
    let pad = if inner > 5 { inner - 5 } else { 0 };
    anyos_std::print!("Swp[");
    for _ in 0..pad { anyos_std::print!(" "); }
    anyos_std::print!("{}0K/0K{}", FG_BWHITE, RESET);
    anyos_std::print!("]");
}

// ─── Full frame render ────────────────────────────────────────────────────────

pub struct FrameData<'a> {
    pub cpu:        &'a CpuState,
    pub tasks:      &'a [TaskEntry],
    pub task_count: usize,
    pub uid_cache:  &'a [(u16, [u8; 16], u8)],
    pub uid_cache_len: usize,
    pub n_running:  u32,
    pub n_sleeping: u32,
    pub used_kb:    u32,
    pub total_kb:   u32,
    pub hours:      u32,
    pub mins:       u32,
    pub secs:       u32,
    pub term_rows:  usize,
    pub term_cols:  usize,
}

pub fn render_frame(f: &FrameData) {
    let ncpu  = (f.cpu.num_cpus as usize).min(crate::MAX_CPUS);
    let half  = f.term_cols / 2;
    // Each half: "NN " (3 chars label) + bar
    let bar_w = half.saturating_sub(3);
    let cpu_rows = (ncpu + 1) / 2;
    let mut t = [0u8; 12];

    // ── CPU bars ─────────────────────────────────────────────────────────────
    for row in 0..cpu_rows {
        let lc = row * 2;
        let rc = lc + 1;

        anyos_std::print!("{:<2} ", fmt_u32(&mut t, lc as u32));
        print_bar_fixed(f.cpu.core_pct[lc], bar_w, BG_GREEN, FG_BWHITE);

        if rc < ncpu {
            anyos_std::print!("{:<2} ", fmt_u32(&mut t, rc as u32));
            print_bar_fixed(f.cpu.core_pct[rc], bar_w, BG_GREEN, FG_BWHITE);
        } else {
            for _ in 0..half { anyos_std::print!(" "); }
        }
        anyos_std::print!("\x1B[K\n");
    }

    // ── Mem / Swp bars ───────────────────────────────────────────────────────
    print_mem_bar(f.used_kb, f.total_kb, f.term_cols);
    anyos_std::print!("\x1B[K\n");
    print_swp_bar(f.term_cols);
    anyos_std::print!("\x1B[K\n");

    // ── Summary ───────────────────────────────────────────────────────────────
    anyos_std::print!("  {}Tasks:{} ", BOLD, RESET);
    anyos_std::print!("{}, ", f.task_count);
    anyos_std::print!("{}{}{} running, ", FG_BGREEN, f.n_running, RESET);
    anyos_std::print!("{}{}{} sleeping", FG_BBLACK, f.n_sleeping, RESET);
    anyos_std::print!("\x1B[K\n");
    anyos_std::print!("  {}Uptime:{} {:02}:{:02}:{:02}\x1B[K\n",
        BOLD, RESET, f.hours, f.mins, f.secs);

    // ── Tab bar ───────────────────────────────────────────────────────────────
    anyos_std::print!("{}{}  Main  {}{}  I/O  {}\x1B[K\n",
        BG_BCYAN, FG_BLACK, RESET, FG_BBLACK, RESET);

    // ── Column header ─────────────────────────────────────────────────────────
    // Fixed part: "  PID USER      PRI  NI  VIRT   RES   SHR S  CPU%  MEM%   TIME+ "
    // Widths:      7+1 + 8+1 + 3+1 + 3+1 + 5+1 + 5+1 + 5+1 + 1+1 + 5+1 + 5+1 + 7+1 = 65 chars
    // cmd_w fills the rest → total = 65 + cmd_w = term_cols → cmd_w = term_cols - 65
    let cmd_w = f.term_cols.saturating_sub(65).max(4);
    anyos_std::print!("{}{}", BG_BCYAN, FG_BLACK);
    anyos_std::print!("{:>7} {:<8} {:>3} {:>3} {:>5} {:>5} {:>5} {:1} {:>5} {:>5} {:>7} ",
        "PID", "USER", "PRI", "NI", "VIRT", "RES", "SHR", "S", "CPU%", "MEM%", "TIME+");
    anyos_std::print!("{:<width$}", "Command", width = cmd_w);
    anyos_std::print!("{}\x1B[K\n", RESET);

    // ── Process rows ──────────────────────────────────────────────────────────
    // Rows used so far: cpu_rows + 2(mem/swp) + 2(summary) + 1(tab) + 1(col) = cpu_rows+6
    let header_rows = cpu_rows + 6;
    let max_rows = f.term_rows.saturating_sub(header_rows + 1); // +1 for fnbar
    let visible  = f.task_count.min(max_rows);

    for i in 0..visible {
        render_task_row(&f.tasks[i], f.uid_cache, f.uid_cache_len, f.total_kb, cmd_w);
    }

    anyos_std::print!("\x1B[J"); // clear to end of screen

    // ── Function key bar ──────────────────────────────────────────────────────
    let mut row_buf = [0u8; 12];
    anyos_std::print!("\x1B[{};1H", fmt_u32(&mut row_buf, f.term_rows as u32));
    const FNKEYS: [(&str, &str); 10] = [
        ("1","Help  "),("2","Setup "),("3","Search"),("4","Filter"),("5","Tree  "),
        ("6","SortBy"),("7","Nice -"),("8","Nice +"),("9","Kill  "),("10","Quit  "),
    ];
    for (num, lbl) in &FNKEYS {
        anyos_std::print!("{}{}F{}{}{}{}{}", BOLD, FG_BBLACK, num, RESET, BG_BCYAN, FG_BLACK, lbl);
    }
    anyos_std::print!("{}\x1B[K", RESET);
}

fn render_task_row(
    task:          &TaskEntry,
    uid_cache:     &[(u16, [u8; 16], u8)],
    uid_cache_len: usize,
    total_kb:      u32,
    cmd_w:         usize,
) {
    let name = core::str::from_utf8(&task.name[..task.name_len]).unwrap_or("?");

    let (state_ch, state_color) = match task.state {
        0 => ('S', FG_BBLACK),
        1 => ('R', FG_BGREEN),
        2 => ('D', FG_BRED),
        3 => ('Z', FG_BBLACK),
        _ => ('?', RESET),
    };

    let username = uid_cache[..uid_cache_len]
        .iter()
        .find(|e| e.0 == task.uid)
        .and_then(|e| if e.2 > 0 { core::str::from_utf8(&e.1[..e.2 as usize]).ok() } else { None })
        .unwrap_or("?");

    let mut vbuf = [0u8; 12];
    let virt_s = fmt_mem_short(&mut vbuf, task.user_pages * 4);

    let mut cbuf = [0u8; 12];
    let cpu_s = fmt_pct(&mut cbuf, task.cpu_pct_x10);

    let mem_pct10 = if total_kb > 0 {
        (task.user_pages as u64 * 4 * 1000 / total_kb as u64) as u32
    } else { 0 };
    let mut mbuf = [0u8; 12];
    let mem_s = fmt_pct(&mut mbuf, mem_pct10);

    let pid_color = if task.state == 1 { FG_BGREEN } else { RESET };
    anyos_std::print!("{}{:>7}{} ", pid_color, task.tid, RESET);

    let uname = if username.len() > 8 { &username[..8] } else { username };
    anyos_std::print!("{}{:<8}{} ", FG_BYELLOW, uname, RESET);

    anyos_std::print!("{:>3} {:>3} ", task.priority as i32, 0i32);
    anyos_std::print!("{:>5} {:>5} {:>5} ", virt_s, virt_s, "0");
    anyos_std::print!("{}{}{} ", state_color, state_ch, RESET);

    let cpu_color = if task.cpu_pct_x10 > 500 { FG_BRED } else { RESET };
    anyos_std::print!("{}{:>5}{} ", cpu_color, cpu_s, RESET);
    anyos_std::print!("{:>5} {:>7} ", mem_s, "0:00.00");

    let name_trunc = if name.len() > cmd_w { &name[..cmd_w] } else { name };
    anyos_std::print!("{}\x1B[K\n", name_trunc);
}
