#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use anyos_std::sys;
use anyos_std::process;
use anyos_std::ui::window;
use anyos_std::fs;
use anyos_std::icons;

anyos_std::entry!(main);

use uisys_client::*;

// ─── Layout ──────────────────────────────────────────────────────────────────

const ROW_H: i32 = 22;
const PAD: i32 = 8;
const STATS_H: i32 = 70;
const TOOLBAR_H: i32 = 32;
const HEADER_Y_OFFSET: i32 = STATS_H + TOOLBAR_H;

const ICON_SIZE: u32 = 16;

// Column X positions (icon at COL_NAME, text at COL_NAME_TEXT)
const COL_TID: i32 = 10;
const COL_NAME: i32 = 60;
const COL_NAME_TEXT: i32 = COL_NAME + ICON_SIZE as i32 + 4;
const COL_STATE: i32 = 210;
const COL_ARCH: i32 = 300;
const COL_CPU: i32 = 360;
const COL_PRIO: i32 = 430;

// Selected row highlight color
const SEL_BG: u32 = 0xFF0A4A8A;
const MAX_CPUS: usize = 16;
const MAX_TASKS: usize = 64;

// ─── Data Structures ─────────────────────────────────────────────────────────

struct TaskEntry {
    tid: u32,
    name: [u8; 24],
    name_len: usize,
    state: u8,
    priority: u8,
    arch: u8,       // 0=x86_64, 1=x86
    cpu_pct_x10: u32, // CPU% × 10 (e.g. 125 = 12.5%)
}

/// Tracks previous cpu_ticks per TID for delta computation.
struct PrevTicks {
    entries: [(u32, u32); MAX_TASKS], // (tid, cpu_ticks)
    count: usize,
    prev_total: u32,
}

struct MemInfo {
    total_frames: u32,
    free_frames: u32,
    heap_used: u32,
    heap_total: u32,
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
    fn new() -> Self {
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

// ─── Icon Cache ──────────────────────────────────────────────────────────────

struct IconEntry {
    name: String,
    pixels: Vec<u32>, // ICON_SIZE * ICON_SIZE ARGB, empty = no icon found
}

fn load_app_icon(name: &str) -> Vec<u32> {
    let bin_path = alloc::format!("/bin/{}", name);
    let icon_path = icons::app_icon_path(&bin_path);

    let fd = fs::open(&icon_path, 0);
    if fd == u32::MAX { return Vec::new(); }

    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);

    if data.is_empty() { return Vec::new(); }

    let info = match libimage_client::probe_ico_size(&data, ICON_SIZE) {
        Some(i) => i,
        None => match libimage_client::probe(&data) {
            Some(i) => i,
            None => return Vec::new(),
        },
    };

    let src_w = info.width;
    let src_h = info.height;
    let mut pixels = vec![0u32; (src_w * src_h) as usize];
    let mut scratch = vec![0u8; info.scratch_needed as usize];

    let ok = if info.format == libimage_client::FMT_ICO {
        libimage_client::decode_ico_size(&data, ICON_SIZE, &mut pixels, &mut scratch).is_ok()
    } else {
        libimage_client::decode(&data, &mut pixels, &mut scratch).is_ok()
    };
    if !ok { return Vec::new(); }

    if src_w == ICON_SIZE && src_h == ICON_SIZE { return pixels; }

    let mut dst = vec![0u32; (ICON_SIZE * ICON_SIZE) as usize];
    libimage_client::scale_image(
        &pixels, src_w, src_h,
        &mut dst, ICON_SIZE, ICON_SIZE,
        libimage_client::MODE_SCALE,
    );
    dst
}

fn ensure_icon_cached(cache: &mut Vec<IconEntry>, name: &str) {
    if cache.iter().any(|e| e.name == name) { return; }
    let pixels = load_app_icon(name);
    cache.push(IconEntry { name: String::from(name), pixels });
}

fn find_icon<'a>(cache: &'a [IconEntry], name: &str) -> Option<&'a [u32]> {
    cache.iter()
        .find(|e| e.name == name)
        .map(|e| e.pixels.as_slice())
        .filter(|p| !p.is_empty())
}

// ─── Data Fetching ───────────────────────────────────────────────────────────

fn fetch_tasks(buf: &mut [u8; 36 * 64], prev: &mut PrevTicks, total_sched_ticks: u32) -> Vec<TaskEntry> {
    let mut result = Vec::new();
    let count = sys::sysinfo(1, buf);
    if count == u32::MAX {
        return result;
    }

    let dt = total_sched_ticks.wrapping_sub(prev.prev_total);

    for i in 0..count as usize {
        let off = i * 36;
        if off + 36 > buf.len() { break; }
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let prio = buf[off + 4];
        let state = buf[off + 5];
        let arch = buf[off + 6];
        let mut name = [0u8; 24];
        name.copy_from_slice(&buf[off + 8..off + 32]);
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(24);
        let cpu_ticks = u32::from_le_bytes([buf[off + 32], buf[off + 33], buf[off + 34], buf[off + 35]]);

        // Find previous ticks for this TID
        let prev_ticks = prev.entries[..prev.count]
            .iter()
            .find(|e| e.0 == tid)
            .map(|e| e.1)
            .unwrap_or(cpu_ticks); // First time: delta = 0

        let d_ticks = cpu_ticks.wrapping_sub(prev_ticks);
        let cpu_pct_x10 = if dt > 0 && d_ticks > 0 {
            (d_ticks as u64 * 1000 / dt as u64).min(1000) as u32
        } else {
            0
        };

        result.push(TaskEntry { tid, name, name_len, state, priority: prio, arch, cpu_pct_x10 });
    }

    // Save current snapshot for next delta
    prev.count = 0;
    for i in 0..count as usize {
        if prev.count >= MAX_TASKS { break; }
        let off = i * 36;
        if off + 36 > buf.len() { break; }
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let cpu_ticks = u32::from_le_bytes([buf[off + 32], buf[off + 33], buf[off + 34], buf[off + 35]]);
        prev.entries[prev.count] = (tid, cpu_ticks);
        prev.count += 1;
    }
    prev.prev_total = total_sched_ticks;

    result
}

fn fetch_memory() -> Option<MemInfo> {
    let mut buf = [0u8; 16];
    if sys::sysinfo(0, &mut buf) != 0 {
        return None;
    }
    Some(MemInfo {
        total_frames: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
        free_frames: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        heap_used: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
        heap_total: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
    })
}

fn fetch_cpu(state: &mut CpuState) {
    let mut buf = [0u8; 16 + 8 * MAX_CPUS];
    sys::sysinfo(3, &mut buf);

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

// ─── Rendering ───────────────────────────────────────────────────────────────

fn render(
    win_id: u32,
    tasks: &[TaskEntry],
    mem: &Option<MemInfo>,
    cpu: &CpuState,
    selected: Option<usize>,
    kill_btn: &UiToolbarButton,
    icon_cache: &[IconEntry],
    scroll_offset: usize,
    win_w: u32,
    win_h: u32,
) {
    // Clear background
    window::fill_rect(win_id, 0, 0, win_w as u16, win_h as u16, colors::WINDOW_BG());

    // ── System Stats Header ──
    card(win_id, 0, 0, win_w, STATS_H as u32);

    label(win_id, PAD, 4, "Activity Monitor", 0xFF00C8FF, FontSize::Normal, TextAlign::Left);

    // Uptime
    let ticks = sys::uptime();
    let hz = sys::tick_hz();
    let secs = if hz > 0 { ticks / hz } else { 0 };
    let mins = secs / 60;
    let mut ubuf = [0u8; 24];
    let ustr = fmt_uptime(&mut ubuf, mins, secs % 60);
    label(win_id, win_w as i32 - (ustr.len() as i32 * 7) - PAD, 4, ustr, colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    // Memory + CPU info text
    if let Some(ref mem) = mem {
        let total_kb = mem.total_frames * 4;
        let free_kb = mem.free_frames * 4;
        let used_kb = total_kb - free_kb;
        let heap_kb = mem.heap_used / 1024;
        let heap_total_kb = mem.heap_total / 1024;

        let mut mbuf = [0u8; 80];
        let ms = fmt_mem_line(&mut mbuf, used_kb / 1024, total_kb / 1024, heap_kb, heap_total_kb);
        label(win_id, PAD, 22, ms, colors::TEXT(), FontSize::Normal, TextAlign::Left);

        let bar_w = (win_w as i32 - PAD * 2) as u32;
        if total_kb > 0 {
            let mem_pct = (used_kb as u64 * 100 / total_kb as u64) as u32;
            progress(win_id, PAD, 38, bar_w, 6, mem_pct);
        }
    }

    // Per-core CPU bars
    let ncpu = (cpu.num_cpus as usize).max(1).min(MAX_CPUS);
    let bar_area_w = win_w as i32 - PAD * 2;
    let gap = 6i32;
    let per_core_w = if ncpu > 1 {
        ((bar_area_w - gap * (ncpu as i32 - 1)) / ncpu as i32).max(20) as u32
    } else {
        bar_area_w as u32
    };

    for i in 0..ncpu {
        let bx = PAD + i as i32 * (per_core_w as i32 + gap);
        let mut lbuf = [0u8; 16];
        let ls = fmt_core_label(&mut lbuf, i as u32, cpu.core_pct[i]);
        label(win_id, bx, 48, ls, colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
        progress(win_id, bx, 60, per_core_w, 5, cpu.core_pct[i]);
    }

    // ── Toolbar ──
    toolbar(win_id, 0, STATS_H, win_w, TOOLBAR_H as u32);
    kill_btn.render(win_id, "Kill Process");

    let mut cbuf = [0u8; 32];
    let cs = fmt_process_cpu(&mut cbuf, tasks.len(), cpu.overall_pct);
    label(win_id, win_w as i32 - (cs.len() as i32 * 7) - PAD, STATS_H + 8, cs, colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let mut y = HEADER_Y_OFFSET;

    // ── Column Headers ──
    window::fill_rect(win_id, 0, y as i16, win_w as u16, ROW_H as u16, 0xFF4A4A4A);
    label(win_id, COL_TID, y + 3, "TID", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, COL_NAME_TEXT, y + 3, "Process", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, COL_STATE, y + 3, "State", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, COL_ARCH, y + 3, "Arch", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, COL_CPU, y + 3, "CPU", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, COL_PRIO, y + 3, "Priority", colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += ROW_H;

    // ── Task Rows (with scroll) ──
    for (global_i, task) in tasks.iter().enumerate().skip(scroll_offset) {
        if y + ROW_H > win_h as i32 {
            break;
        }

        // Selection highlight or alternating background
        if selected == Some(global_i) {
            window::fill_rect(win_id, 0, y as i16, win_w as u16, ROW_H as u16, SEL_BG);
        } else if global_i % 2 == 1 {
            window::fill_rect(win_id, 0, y as i16, win_w as u16, ROW_H as u16, 0xFF333333);
        }

        let text_color = if selected == Some(global_i) { 0xFFFFFFFF } else { colors::TEXT() };

        let (state_text, state_kind) = match task.state {
            0 => ("Ready", StatusKind::Warning),
            1 => ("Running", StatusKind::Online),
            2 => ("Blocked", StatusKind::Error),
            3 => ("Terminated", StatusKind::Offline),
            _ => ("Unknown", StatusKind::Offline),
        };

        // TID
        let mut tbuf = [0u8; 12];
        label(win_id, COL_TID, y + 3, fmt_u32(&mut tbuf, task.tid), text_color, FontSize::Small, TextAlign::Left);

        // Icon + Name
        if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
            if let Some(pixels) = find_icon(icon_cache, name) {
                let icon_y = y + (ROW_H - ICON_SIZE as i32) / 2;
                window::blit(win_id, COL_NAME as i16, icon_y as i16, ICON_SIZE as u16, ICON_SIZE as u16, pixels);
            }
            label(win_id, COL_NAME_TEXT, y + 3, name, text_color, FontSize::Small, TextAlign::Left);
        }

        // State (using sized variant to match table font)
        status_indicator_sized(win_id, COL_STATE, y + 3, state_kind, state_text, FontSize::Small);

        // Architecture
        let arch_str = if task.arch == 1 { "x86" } else { "x86_64" };
        let arch_color = if task.arch == 1 { 0xFFFF9500 } else { colors::TEXT_SECONDARY() };
        label(win_id, COL_ARCH, y + 3, arch_str, arch_color, FontSize::Small, TextAlign::Left);

        // CPU%
        let mut cpubuf = [0u8; 12];
        let cpu_str = if task.cpu_pct_x10 > 0 {
            fmt_pct(&mut cpubuf, task.cpu_pct_x10)
        } else {
            "0.0%"
        };
        label(win_id, COL_CPU, y + 3, cpu_str, colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);

        // Priority
        let mut pbuf = [0u8; 12];
        label(win_id, COL_PRIO, y + 3, fmt_u32(&mut pbuf, task.priority as u32), colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);

        y += ROW_H;
    }

    // ── Scroll indicator ──
    let table_h = win_h as i32 - HEADER_Y_OFFSET - ROW_H;
    let max_visible = (table_h / ROW_H).max(1) as usize;
    if tasks.len() > max_visible {
        let bar_x = win_w as i32 - 4;
        let bar_top = HEADER_Y_OFFSET + ROW_H;
        let thumb_h = ((max_visible as i32 * table_h) / tasks.len() as i32).max(12);
        let thumb_y = bar_top + (scroll_offset as i32 * table_h / tasks.len() as i32);
        window::fill_rect(win_id, bar_x as i16, thumb_y as i16, 3, thumb_h as u16, 0x60FFFFFF);
    }

    window::present(win_id);
}

// ─── Formatting ──────────────────────────────────────────────────────────────

fn fmt_u32<'a>(buf: &'a mut [u8; 12], val: u32) -> &'a str {
    if val == 0 { buf[0] = b'0'; return unsafe { core::str::from_utf8_unchecked(&buf[..1]) }; }
    let mut v = val; let mut tmp = [0u8; 12]; let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    for i in 0..n { buf[i] = tmp[n - 1 - i]; }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

fn fmt_pct<'a>(buf: &'a mut [u8; 12], pct_x10: u32) -> &'a str {
    let whole = pct_x10 / 10;
    let frac = pct_x10 % 10;
    let mut p = 0;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, whole);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'.'; p += 1;
    buf[p] = b'0' + frac as u8; p += 1;
    buf[p] = b'%'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_uptime<'a>(buf: &'a mut [u8; 24], mins: u32, secs: u32) -> &'a str {
    let mut p = 0;
    buf[p..p + 8].copy_from_slice(b"Uptime: "); p += 8;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, mins); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 2].copy_from_slice(b"m "); p += 2;
    let s = fmt_u32(&mut t, secs); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b's'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_mem_line<'a>(buf: &'a mut [u8; 80], used_mb: u32, total_mb: u32, heap_kb: u32, heap_total_kb: u32) -> &'a str {
    let mut p = 0;
    let mut t = [0u8; 12];
    buf[p..p + 5].copy_from_slice(b"Mem: "); p += 5;
    let s = fmt_u32(&mut t, used_mb); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'/'; p += 1;
    let s = fmt_u32(&mut t, total_mb); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 8].copy_from_slice(b"M  Heap:"); p += 8;
    let s = fmt_u32(&mut t, heap_kb); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'/'; p += 1;
    let s = fmt_u32(&mut t, heap_total_kb); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'K'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_core_label<'a>(buf: &'a mut [u8; 16], core_id: u32, pct: u32) -> &'a str {
    let mut p = 0;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, core_id);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 2].copy_from_slice(b": "); p += 2;
    let s = fmt_u32(&mut t, pct);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'%'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_process_cpu<'a>(buf: &'a mut [u8; 32], count: usize, cpu_pct: u32) -> &'a str {
    let mut t = [0u8; 12];
    let mut p = 0;
    let s = fmt_u32(&mut t, count as u32);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 5].copy_from_slice(b" proc"); p += 5;
    buf[p..p + 6].copy_from_slice(b"  CPU:"); p += 6;
    let s = fmt_u32(&mut t, cpu_pct);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'%'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let win_id = window::create("Activity Monitor", 100, 60, 540, 420);
    if win_id == u32::MAX {
        return;
    }

    let mut mb = window::MenuBarBuilder::new()
        .menu("File")
            .item(1, "Close", 0)
        .end_menu()
        .menu("Process")
            .item(10, "Kill Process", 0)
            .item(11, "Refresh", 0)
        .end_menu();
    let data = mb.build();
    window::set_menu(win_id, data);
    window::disable_menu_item(win_id, 10);

    let (mut win_w, mut win_h) = window::get_size(win_id).unwrap_or((540, 420));
    let kill_btn = UiToolbarButton::new(PAD, STATS_H + 4, 100, 24);

    let mut thread_buf = [0u8; 36 * 64];
    let mut event = [0u32; 5];
    let mut last_update: u32 = 0;
    let mut selected: Option<usize> = None;
    let mut cpu_state = CpuState::new();
    let mut prev_ticks = PrevTicks { entries: [(0, 0); MAX_TASKS], count: 0, prev_total: 0 };
    let mut scroll_offset: usize = 0;
    let mut icon_cache: Vec<IconEntry> = Vec::new();

    fetch_cpu(&mut cpu_state);

    // Initial fetch and render
    let tasks = fetch_tasks(&mut thread_buf, &mut prev_ticks, cpu_state.total_sched_ticks);
    for task in &tasks {
        if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
            ensure_icon_cached(&mut icon_cache, name);
        }
    }
    let mem = fetch_memory();
    render(win_id, &tasks, &mem, &cpu_state, selected, &kill_btn, &icon_cache, scroll_offset, win_w, win_h);

    loop {
        if window::get_event(win_id, &mut event) == 1 {
            let ev = UiEvent::from_raw(&event);

            if ev.is_key_down() && ev.key_code() == KEY_ESCAPE {
                break;
            }

            if event[0] == window::EVENT_MENU_ITEM {
                match event[2] {
                    1 => { break; }
                    10 => {
                        if let Some(sel_idx) = selected {
                            let tasks = fetch_tasks(&mut thread_buf, &mut prev_ticks, cpu_state.total_sched_ticks);
                            if sel_idx < tasks.len() && tasks[sel_idx].tid > 3 {
                                process::kill(tasks[sel_idx].tid);
                                selected = None;
                                window::disable_menu_item(win_id, 10);
                                last_update = 0;
                            }
                        }
                    }
                    11 => { last_update = 0; }
                    _ => {}
                }
            }

            if event[0] == EVENT_WINDOW_CLOSE { break; }

            if event[0] == EVENT_RESIZE {
                win_w = event[1];
                win_h = event[2];
                last_update = 0;
            }

            if event[0] == 0x0050 { last_update = 0; }

            // Scroll
            if event[0] == window::EVENT_MOUSE_SCROLL {
                let dz = event[1] as i32;
                if dz < 0 {
                    scroll_offset = scroll_offset.saturating_sub(3);
                } else if dz > 0 {
                    scroll_offset = scroll_offset.saturating_add(3);
                }
                last_update = 0;
            }

            // Mouse click on task rows
            if ev.is_mouse_down() {
                let (_mx, my) = ev.mouse_pos();

                if kill_btn.handle_event(&ev) {
                    if let Some(sel_idx) = selected {
                        let tasks = fetch_tasks(&mut thread_buf, &mut prev_ticks, cpu_state.total_sched_ticks);
                        if sel_idx < tasks.len() && tasks[sel_idx].tid > 3 {
                            process::kill(tasks[sel_idx].tid);
                            selected = None;
                            window::disable_menu_item(win_id, 10);
                            last_update = 0;
                        }
                    }
                } else {
                    let row_start_y = HEADER_Y_OFFSET + ROW_H;
                    if my >= row_start_y {
                        let vis_row = ((my - row_start_y) / ROW_H) as usize;
                        let global_row = vis_row + scroll_offset;
                        let tasks = fetch_tasks(&mut thread_buf, &mut prev_ticks, cpu_state.total_sched_ticks);
                        let old_sel = selected;
                        if global_row < tasks.len() {
                            selected = Some(global_row);
                        } else {
                            selected = None;
                        }
                        if old_sel != selected {
                            if selected.is_some() {
                                window::enable_menu_item(win_id, 10);
                            } else {
                                window::disable_menu_item(win_id, 10);
                            }
                        }
                        last_update = 0;
                    }
                }
            }
        }

        // Auto-refresh every ~500ms
        let refresh_ticks = sys::tick_hz() / 2;
        let now = sys::uptime();
        if now.wrapping_sub(last_update) >= refresh_ticks {
            fetch_cpu(&mut cpu_state);
            let tasks = fetch_tasks(&mut thread_buf, &mut prev_ticks, cpu_state.total_sched_ticks);

            // Ensure icons are cached for all tasks
            for task in &tasks {
                if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
                    ensure_icon_cached(&mut icon_cache, name);
                }
            }

            if let Some(sel) = selected {
                if sel >= tasks.len() {
                    selected = if tasks.is_empty() { None } else { Some(tasks.len() - 1) };
                }
            }

            // Clamp scroll offset
            let table_h = win_h as i32 - HEADER_Y_OFFSET - ROW_H;
            let max_visible = (table_h / ROW_H).max(1) as usize;
            if tasks.len() > max_visible {
                scroll_offset = scroll_offset.min(tasks.len() - max_visible);
            } else {
                scroll_offset = 0;
            }

            let mem = fetch_memory();
            render(win_id, &tasks, &mem, &cpu_state, selected, &kill_btn, &icon_cache, scroll_offset, win_w, win_h);
            last_update = now;
        }

        process::sleep(16);
    }

    window::destroy(win_id);
}
