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
const COL_NAME: i32 = 50;
const COL_NAME_TEXT: i32 = COL_NAME + ICON_SIZE as i32 + 4;
const COL_USER: i32 = 185;
const COL_STATE: i32 = 250;
const COL_ARCH: i32 = 330;
const COL_CPU: i32 = 385;
const COL_MEM: i32 = 440;
const COL_PRIO: i32 = 510;

// Selected row highlight color
const SEL_BG: u32 = 0xFF0A4A8A;
const MAX_CPUS: usize = 16;
const MAX_TASKS: usize = 64;
const THREAD_ENTRY_SIZE: usize = 60;

// ─── Data Structures ─────────────────────────────────────────────────────────

struct TaskEntry {
    tid: u32,
    name: [u8; 24],
    name_len: usize,
    state: u8,
    priority: u8,
    arch: u8,       // 0=x86_64, 1=x86
    uid: u16,       // user ID
    user_pages: u32, // number of user-space pages (× 4 = KiB)
    cpu_pct_x10: u32, // CPU% × 10 (e.g. 125 = 12.5%)
    io_read_bytes: u64,
    io_write_bytes: u64,
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

struct HwInfo {
    brand: [u8; 48],
    vendor: [u8; 16],
    tsc_mhz: u32,
    cpu_count: u32,
    boot_mode: u32,
    total_mem_mib: u32,
    free_mem_mib: u32,
    fb_width: u32,
    fb_height: u32,
    fb_bpp: u32,
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

// ─── CPU History (ring buffer for line graphs) ──────────────────────────────

const GRAPH_SAMPLES: usize = 60; // 60 × 500ms = 30 seconds

struct CpuHistory {
    samples: [[u8; GRAPH_SAMPLES]; MAX_CPUS],
    pos: usize,
    count: usize,
}

impl CpuHistory {
    fn new() -> Self {
        CpuHistory { samples: [[0; GRAPH_SAMPLES]; MAX_CPUS], pos: 0, count: 0 }
    }

    fn push(&mut self, cpu: &CpuState) {
        for i in 0..(cpu.num_cpus as usize).min(MAX_CPUS) {
            self.samples[i][self.pos] = cpu.core_pct[i].min(100) as u8;
        }
        self.pos = (self.pos + 1) % GRAPH_SAMPLES;
        if self.count < GRAPH_SAMPLES { self.count += 1; }
    }

    /// Get sample for a core at a given age (0 = newest, count-1 = oldest).
    fn get(&self, core: usize, age: usize) -> u8 {
        if age >= self.count { return 0; }
        let idx = (self.pos + GRAPH_SAMPLES - 1 - age) % GRAPH_SAMPLES;
        self.samples[core][idx]
    }
}

// ─── Icon Cache ──────────────────────────────────────────────────────────────

struct IconEntry {
    name: String,
    pixels: Vec<u32>, // ICON_SIZE * ICON_SIZE ARGB, empty = no icon found
}

fn load_app_icon(name: &str) -> Vec<u32> {
    // Check /Applications/{Name}.app first, then fall back to /bin/{name}
    let bin_path = {
        let app_path = alloc::format!("/Applications/{}.app", name);
        let mut stat_buf = [0u32; 7];
        if fs::stat(&app_path, &mut stat_buf) == 0 && stat_buf[0] == 1 {
            app_path
        } else {
            alloc::format!("/System/bin/{}", name)
        }
    };
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

fn fetch_tasks(buf: &mut [u8; THREAD_ENTRY_SIZE * 64], prev: &mut PrevTicks, total_sched_ticks: u32) -> Vec<TaskEntry> {
    let mut result = Vec::new();
    let count = sys::sysinfo(1, buf);
    if count == u32::MAX {
        return result;
    }

    let dt = total_sched_ticks.wrapping_sub(prev.prev_total);

    for i in 0..count as usize {
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() { break; }
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let prio = buf[off + 4];
        let state = buf[off + 5];
        let arch = buf[off + 6];
        let mut name = [0u8; 24];
        name.copy_from_slice(&buf[off + 8..off + 32]);
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(24);
        let user_pages = u32::from_le_bytes([buf[off + 32], buf[off + 33], buf[off + 34], buf[off + 35]]);
        let cpu_ticks = u32::from_le_bytes([buf[off + 36], buf[off + 37], buf[off + 38], buf[off + 39]]);
        let io_read_bytes = u64::from_le_bytes([
            buf[off + 40], buf[off + 41], buf[off + 42], buf[off + 43],
            buf[off + 44], buf[off + 45], buf[off + 46], buf[off + 47],
        ]);
        let io_write_bytes = u64::from_le_bytes([
            buf[off + 48], buf[off + 49], buf[off + 50], buf[off + 51],
            buf[off + 52], buf[off + 53], buf[off + 54], buf[off + 55],
        ]);

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

        let uid = u16::from_le_bytes([buf[off + 56], buf[off + 57]]);

        result.push(TaskEntry { tid, name, name_len, state, priority: prio, arch, uid, user_pages, cpu_pct_x10, io_read_bytes, io_write_bytes });
    }

    // Save current snapshot for next delta
    prev.count = 0;
    for i in 0..count as usize {
        if prev.count >= MAX_TASKS { break; }
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() { break; }
        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let cpu_ticks = u32::from_le_bytes([buf[off + 36], buf[off + 37], buf[off + 38], buf[off + 39]]);
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

fn fetch_hwinfo() -> HwInfo {
    let mut buf = [0u8; 96];
    sys::sysinfo(4, &mut buf);
    let mut brand = [0u8; 48];
    let mut vendor = [0u8; 16];
    brand.copy_from_slice(&buf[0..48]);
    vendor.copy_from_slice(&buf[48..64]);
    HwInfo {
        brand,
        vendor,
        tsc_mhz: u32::from_le_bytes([buf[64], buf[65], buf[66], buf[67]]),
        cpu_count: u32::from_le_bytes([buf[68], buf[69], buf[70], buf[71]]),
        boot_mode: u32::from_le_bytes([buf[72], buf[73], buf[74], buf[75]]),
        total_mem_mib: u32::from_le_bytes([buf[76], buf[77], buf[78], buf[79]]),
        free_mem_mib: u32::from_le_bytes([buf[80], buf[81], buf[82], buf[83]]),
        fb_width: u32::from_le_bytes([buf[84], buf[85], buf[86], buf[87]]),
        fb_height: u32::from_le_bytes([buf[88], buf[89], buf[90], buf[91]]),
        fb_bpp: u32::from_le_bytes([buf[92], buf[93], buf[94], buf[95]]),
    }
}

// ─── Rendering ───────────────────────────────────────────────────────────────

fn render(
    win_id: u32,
    active_tab: usize,
    tasks: &[TaskEntry],
    mem: &Option<MemInfo>,
    cpu: &CpuState,
    cpu_history: &CpuHistory,
    selected: Option<usize>,
    kill_btn: &UiToolbarButton,
    icon_cache: &[IconEntry],
    scroll_offset: usize,
    hw_info: &HwInfo,
    win_w: u32,
    win_h: u32,
) {
    // Clear background
    window::fill_rect(win_id, 0, 0, win_w as u16, win_h as u16, colors::WINDOW_BG());

    // ── Header with Segmented Control ──
    card(win_id, 0, 0, win_w, STATS_H as u32);

    segmented(win_id, PAD, 6, 380, 24, &["Processes", "Graphs", "Disk", "System"], active_tab);

    // Uptime
    let ticks = sys::uptime();
    let hz = sys::tick_hz();
    let secs = if hz > 0 { ticks / hz } else { 0 };
    let mins = secs / 60;
    let mut ubuf = [0u8; 24];
    let ustr = fmt_uptime(&mut ubuf, mins, secs % 60);
    label(win_id, win_w as i32 - (ustr.len() as i32 * 7) - PAD, 6, ustr, colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    if active_tab == 0 {
        // ── Processes Tab ──

        // Memory + CPU info text
        if let Some(ref mem) = mem {
            let total_kb = mem.total_frames * 4;
            let free_kb = mem.free_frames * 4;
            let used_kb = total_kb - free_kb;
            let heap_kb = mem.heap_used / 1024;
            let heap_total_kb = mem.heap_total / 1024;

            let mut mbuf = [0u8; 80];
            let ms = fmt_mem_line(&mut mbuf, used_kb / 1024, total_kb / 1024, heap_kb, heap_total_kb);
            label(win_id, PAD, 34, ms, colors::TEXT(), FontSize::Normal, TextAlign::Left);

            let bar_w = (win_w as i32 - PAD * 2) as u32;
            if total_kb > 0 {
                let mem_pct = (used_kb as u64 * 100 / total_kb as u64) as u32;
                progress(win_id, PAD, 50, bar_w, 6, mem_pct);
            }
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
        label(win_id, COL_USER, y + 3, "User", colors::TEXT(), FontSize::Small, TextAlign::Left);
        label(win_id, COL_STATE, y + 3, "State", colors::TEXT(), FontSize::Small, TextAlign::Left);
        label(win_id, COL_ARCH, y + 3, "Arch", colors::TEXT(), FontSize::Small, TextAlign::Left);
        label(win_id, COL_CPU, y + 3, "CPU", colors::TEXT(), FontSize::Small, TextAlign::Left);
        label(win_id, COL_MEM, y + 3, "Memory", colors::TEXT(), FontSize::Small, TextAlign::Left);
        label(win_id, COL_PRIO, y + 3, "Priority", colors::TEXT(), FontSize::Small, TextAlign::Left);
        y += ROW_H;

        // ── Task Rows (with scroll) ──
        for (global_i, task) in tasks.iter().enumerate().skip(scroll_offset) {
            if y + ROW_H > win_h as i32 {
                break;
            }

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

            let mut tbuf = [0u8; 12];
            label(win_id, COL_TID, y + 3, fmt_u32(&mut tbuf, task.tid), text_color, FontSize::Small, TextAlign::Left);

            if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
                if let Some(pixels) = find_icon(icon_cache, name) {
                    let icon_y = y + (ROW_H - ICON_SIZE as i32) / 2;
                    window::blit(win_id, COL_NAME as i16, icon_y as i16, ICON_SIZE as u16, ICON_SIZE as u16, pixels);
                }
                label(win_id, COL_NAME_TEXT, y + 3, name, text_color, FontSize::Small, TextAlign::Left);
            }

            // User column
            {
                let mut ubuf = [0u8; 16];
                let nlen = anyos_std::process::getusername(task.uid, &mut ubuf);
                let uname = if nlen != u32::MAX && nlen > 0 {
                    core::str::from_utf8(&ubuf[..nlen as usize]).unwrap_or("?")
                } else {
                    "?"
                };
                label(win_id, COL_USER, y + 3, uname, colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
            }

            status_indicator_sized(win_id, COL_STATE, y + 3, state_kind, state_text, FontSize::Small);

            let arch_str = if task.arch == 1 { "x86" } else { "x86_64" };
            let arch_color = if task.arch == 1 { 0xFFFF9500 } else { colors::TEXT_SECONDARY() };
            label(win_id, COL_ARCH, y + 3, arch_str, arch_color, FontSize::Small, TextAlign::Left);

            let mut cpubuf = [0u8; 12];
            let cpu_str = if task.cpu_pct_x10 > 0 {
                fmt_pct(&mut cpubuf, task.cpu_pct_x10)
            } else {
                "0.0%"
            };
            label(win_id, COL_CPU, y + 3, cpu_str, colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);

            let mut membuf = [0u8; 16];
            let mem_str = fmt_mem_pages(&mut membuf, task.user_pages);
            label(win_id, COL_MEM, y + 3, mem_str, colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);

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
    } else if active_tab == 1 {
        // ── Graphs Tab ──
        render_graphs_tab(win_id, cpu, cpu_history, win_w, win_h);
    } else if active_tab == 2 {
        // ── Disk Tab ──
        render_disk_tab(win_id, tasks, scroll_offset, win_w, win_h);
    } else {
        // ── System Tab ──
        render_system_tab(win_id, hw_info, win_w, win_h);
    }

    window::present(win_id);
}

fn render_system_tab(win_id: u32, hw: &HwInfo, win_w: u32, _win_h: u32) {
    let lx = PAD + 8;        // label x
    let vx = 120i32;         // value x
    let section_w = (win_w as i32 - PAD * 2) as u32;
    let mut y = STATS_H + PAD;

    // ── Processor Section ──
    label(win_id, lx, y, "Processor", 0xFF00C8FF, FontSize::Normal, TextAlign::Left);
    y += 20;
    divider_h(win_id, lx, y, section_w - 16);
    y += 8;

    // Brand
    let brand_len = hw.brand.iter().position(|&b| b == 0).unwrap_or(48);
    let brand_trimmed = trim_leading_spaces(&hw.brand[..brand_len]);
    if let Ok(brand_str) = core::str::from_utf8(brand_trimmed) {
        label(win_id, lx, y, "Name", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
        label(win_id, vx, y, brand_str, colors::TEXT(), FontSize::Small, TextAlign::Left);
    }
    y += 18;

    // Vendor
    let vendor_len = hw.vendor.iter().position(|&b| b == 0).unwrap_or(16);
    if let Ok(vendor_str) = core::str::from_utf8(&hw.vendor[..vendor_len]) {
        label(win_id, lx, y, "Vendor", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
        label(win_id, vx, y, vendor_str, colors::TEXT(), FontSize::Small, TextAlign::Left);
    }
    y += 18;

    // Cores
    let mut t = [0u8; 12];
    label(win_id, lx, y, "Cores", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
    label(win_id, vx, y, fmt_u32(&mut t, hw.cpu_count), colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += 18;

    // Speed
    let mut sbuf = [0u8; 24];
    let speed_str = fmt_mhz(&mut sbuf, hw.tsc_mhz);
    label(win_id, lx, y, "Speed", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
    label(win_id, vx, y, speed_str, colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += 28;

    // ── Memory Section ──
    label(win_id, lx, y, "Memory", 0xFF00C8FF, FontSize::Normal, TextAlign::Left);
    y += 20;
    divider_h(win_id, lx, y, section_w - 16);
    y += 8;

    let mut mbuf = [0u8; 16];
    label(win_id, lx, y, "Total", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
    let ms = fmt_mib(&mut mbuf, hw.total_mem_mib);
    label(win_id, vx, y, ms, colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += 18;

    label(win_id, lx, y, "Free", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
    let ms = fmt_mib(&mut mbuf, hw.free_mem_mib);
    label(win_id, vx, y, ms, colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += 18;

    // Memory usage bar
    let used_mib = hw.total_mem_mib.saturating_sub(hw.free_mem_mib);
    let mem_pct = if hw.total_mem_mib > 0 {
        (used_mib as u64 * 100 / hw.total_mem_mib as u64) as u32
    } else {
        0
    };
    let mut pbuf = [0u8; 16];
    let ps = fmt_usage(&mut pbuf, used_mib, hw.total_mem_mib);
    label(win_id, lx, y, "Used", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
    label(win_id, vx, y, ps, colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += 18;
    progress(win_id, lx, y, section_w - 16, 8, mem_pct);
    y += 20;

    // ── System Section ──
    label(win_id, lx, y, "System", 0xFF00C8FF, FontSize::Normal, TextAlign::Left);
    y += 20;
    divider_h(win_id, lx, y, section_w - 16);
    y += 8;

    label(win_id, lx, y, "Boot", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
    let boot_str = if hw.boot_mode == 1 { "UEFI" } else { "BIOS" };
    label(win_id, vx, y, boot_str, colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += 18;

    label(win_id, lx, y, "Kernel", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
    label(win_id, vx, y, "anyOS v0.1", colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += 18;

    // Display
    if hw.fb_width > 0 && hw.fb_height > 0 {
        let mut dbuf = [0u8; 32];
        let ds = fmt_display(&mut dbuf, hw.fb_width, hw.fb_height, hw.fb_bpp);
        label(win_id, lx, y, "Display", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
        label(win_id, vx, y, ds, colors::TEXT(), FontSize::Small, TextAlign::Left);
        y += 18;
    }

    // Uptime
    let ticks = sys::uptime();
    let hz = sys::tick_hz();
    let total_secs = if hz > 0 { ticks / hz } else { 0 };
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let mut ut = [0u8; 24];
    let us = fmt_hms(&mut ut, hours, mins, secs);
    label(win_id, lx, y, "Uptime", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
    label(win_id, vx, y, us, colors::TEXT(), FontSize::Small, TextAlign::Left);
}

// ─── Graphs Tab ──────────────────────────────────────────────────────────────

fn render_graphs_tab(win_id: u32, cpu: &CpuState, history: &CpuHistory, win_w: u32, win_h: u32) {
    let ncpu = (cpu.num_cpus as usize).max(1).min(MAX_CPUS);

    // Show overall CPU% in header area
    let mut cbuf = [0u8; 24];
    let cs = fmt_overall_cpu(&mut cbuf, cpu.overall_pct, ncpu as u32);
    label(win_id, PAD, 34, cs, colors::TEXT(), FontSize::Normal, TextAlign::Left);

    // Overall CPU progress bar
    let bar_w = (win_w as i32 - PAD * 2) as u32;
    progress(win_id, PAD, 50, bar_w, 6, cpu.overall_pct);

    // Compute grid layout: cols × rows
    let cols = isqrt_ceil(ncpu);
    let rows = (ncpu + cols - 1) / cols;

    let area_x = PAD;
    let area_y = STATS_H + PAD;
    let area_w = win_w as i32 - PAD * 2;
    let area_h = win_h as i32 - area_y - PAD;
    if area_w <= 0 || area_h <= 0 { return; }

    let gap = 6i32;
    let cell_w = (area_w - gap * (cols as i32 - 1).max(0)) / cols as i32;
    let cell_h = (area_h - gap * (rows as i32 - 1).max(0)) / rows as i32;
    if cell_w < 20 || cell_h < 20 { return; }

    for core in 0..ncpu {
        let col = core % cols;
        let row = core / cols;
        let cx = area_x + col as i32 * (cell_w + gap);
        let cy = area_y + row as i32 * (cell_h + gap);
        draw_cpu_graph(win_id, cx, cy, cell_w as u32, cell_h as u32, core, cpu.core_pct[core], history);
    }
}

fn draw_cpu_graph(
    win_id: u32, x: i32, y: i32, w: u32, h: u32,
    core: usize, current_pct: u32, history: &CpuHistory,
) {
    let graph_bg = 0xFF1A1A2E;
    let grid_color = 0xFF2A2A3E;
    let line_color = 0xFF00C8FF; // macOS accent cyan
    let fill_color = 0xFF0D2840; // dark teal fill (opaque)

    // Background
    window::fill_rect(win_id, x as i16, y as i16, w as u16, h as u16, graph_bg);

    // Title: "CPU N: XX%"
    let label_h: u32 = 16;
    let mut lbuf = [0u8; 16];
    let ls = fmt_core_label(&mut lbuf, core as u32, current_pct);
    label(win_id, x + 4, y + 1, ls, 0xFFCCCCCC, FontSize::Small, TextAlign::Left);

    // Graph area below label
    let gy = y + label_h as i32;
    let gh = h.saturating_sub(label_h);
    if gh < 8 { return; }

    // Horizontal grid lines at 25%, 50%, 75%
    for pct in [25u32, 50, 75] {
        let ly = gy + (gh as i32 - (pct as i32 * gh as i32 / 100));
        window::fill_rect(win_id, x as i16, ly as i16, w as u16, 1, grid_color);
    }

    // Draw line graph
    let sample_count = history.count;
    if sample_count < 2 { return; }

    let num_pts = (w as usize).min(sample_count);
    let mut prev_vy: i32 = -1;

    for px in 0..w {
        // Map pixel to sample age (px=0 → oldest visible, px=w-1 → newest)
        let age = if w > 1 {
            ((w - 1 - px) as usize * (num_pts - 1)) / (w as usize - 1).max(1)
        } else {
            0
        };
        let pct = history.get(core, age) as i32;
        let val_h = pct * gh as i32 / 100;
        let vy = gy + gh as i32 - val_h;

        // Fill area below the line
        if val_h > 0 {
            window::fill_rect(win_id, (x + px as i32) as i16, vy as i16, 1, val_h as u16, fill_color);
        }

        // Draw connecting line between previous and current point
        if prev_vy >= 0 {
            let y0 = prev_vy.min(vy);
            let y1 = prev_vy.max(vy);
            let seg_h = (y1 - y0 + 1).max(1);
            window::fill_rect(win_id, (x + px as i32) as i16, y0 as i16, 1, seg_h as u16, line_color);
        } else {
            // First point: single pixel
            window::fill_rect(win_id, (x + px as i32) as i16, vy as i16, 1, 1, line_color);
        }

        prev_vy = vy;
    }

    // Border
    let border = 0xFF3A3A4E;
    window::fill_rect(win_id, x as i16, y as i16, w as u16, 1, border);
    window::fill_rect(win_id, x as i16, (y + h as i32 - 1) as i16, w as u16, 1, border);
    window::fill_rect(win_id, x as i16, y as i16, 1, h as u16, border);
    window::fill_rect(win_id, (x + w as i32 - 1) as i16, y as i16, 1, h as u16, border);
}

fn isqrt_ceil(n: usize) -> usize {
    if n <= 1 { return 1; }
    let mut x = 1;
    while x * x < n { x += 1; }
    x
}

// Disk tab column positions
const DISK_COL_TID: i32 = 10;
const DISK_COL_NAME: i32 = 60;
const DISK_COL_READ: i32 = 220;
const DISK_COL_WRITE: i32 = 350;

fn render_disk_tab(win_id: u32, tasks: &[TaskEntry], scroll_offset: usize, win_w: u32, win_h: u32) {
    // Compute totals
    let mut total_read: u64 = 0;
    let mut total_write: u64 = 0;
    for t in tasks {
        total_read += t.io_read_bytes;
        total_write += t.io_write_bytes;
    }

    // Summary in card area (below segmented control)
    let mut sbuf = [0u8; 48];
    let ss = fmt_io_summary(&mut sbuf, total_read, total_write);
    label(win_id, PAD, 34, ss, colors::TEXT(), FontSize::Normal, TextAlign::Left);

    // Column headers
    let mut y = STATS_H + PAD;
    window::fill_rect(win_id, 0, y as i16, win_w as u16, ROW_H as u16, 0xFF4A4A4A);
    label(win_id, DISK_COL_TID, y + 3, "TID", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, DISK_COL_NAME, y + 3, "Process", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, DISK_COL_READ, y + 3, "Bytes Read", colors::TEXT(), FontSize::Small, TextAlign::Left);
    label(win_id, DISK_COL_WRITE, y + 3, "Bytes Written", colors::TEXT(), FontSize::Small, TextAlign::Left);
    y += ROW_H;

    // Rows — only show processes with I/O activity, sorted implicitly by task order
    let mut row_idx = 0usize;
    for task in tasks.iter() {
        if task.io_read_bytes == 0 && task.io_write_bytes == 0 { continue; }
        if row_idx < scroll_offset { row_idx += 1; continue; }
        if y + ROW_H > win_h as i32 { break; }

        if row_idx % 2 == 1 {
            window::fill_rect(win_id, 0, y as i16, win_w as u16, ROW_H as u16, 0xFF333333);
        }

        let mut tbuf = [0u8; 12];
        label(win_id, DISK_COL_TID, y + 3, fmt_u32(&mut tbuf, task.tid), colors::TEXT(), FontSize::Small, TextAlign::Left);

        if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
            label(win_id, DISK_COL_NAME, y + 3, name, colors::TEXT(), FontSize::Small, TextAlign::Left);
        }

        let mut rbuf = [0u8; 20];
        let rs = fmt_bytes(&mut rbuf, task.io_read_bytes);
        label(win_id, DISK_COL_READ, y + 3, rs, colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);

        let mut wbuf = [0u8; 20];
        let ws = fmt_bytes(&mut wbuf, task.io_write_bytes);
        label(win_id, DISK_COL_WRITE, y + 3, ws, colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);

        y += ROW_H;
        row_idx += 1;
    }

    // If no processes have I/O, show a message
    if total_read == 0 && total_write == 0 {
        label(win_id, PAD + 8, y + 20, "No disk activity", colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
    }
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

fn trim_leading_spaces(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|&c| c != b' ').unwrap_or(b.len());
    &b[start..]
}

fn fmt_mhz<'a>(buf: &'a mut [u8; 24], mhz: u32) -> &'a str {
    let mut p = 0;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, mhz);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 4].copy_from_slice(b" MHz"); p += 4;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_mib<'a>(buf: &'a mut [u8; 16], mib: u32) -> &'a str {
    let mut p = 0;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, mib);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 4].copy_from_slice(b" MiB"); p += 4;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_usage<'a>(buf: &'a mut [u8; 16], used: u32, total: u32) -> &'a str {
    let mut p = 0;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, used); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'/'; p += 1;
    let s = fmt_u32(&mut t, total); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'M'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_display<'a>(buf: &'a mut [u8; 32], w: u32, h: u32, bpp: u32) -> &'a str {
    let mut p = 0;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, w); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b'x'; p += 1;
    let s = fmt_u32(&mut t, h); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 2].copy_from_slice(b" ("); p += 2;
    let s = fmt_u32(&mut t, bpp); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 5].copy_from_slice(b"-bit)"); p += 5;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_hms<'a>(buf: &'a mut [u8; 24], h: u32, m: u32, s: u32) -> &'a str {
    let mut p = 0;
    let mut t = [0u8; 12];
    let vs = fmt_u32(&mut t, h); buf[p..p + vs.len()].copy_from_slice(vs.as_bytes()); p += vs.len();
    buf[p..p + 2].copy_from_slice(b"h "); p += 2;
    let vs = fmt_u32(&mut t, m); buf[p..p + vs.len()].copy_from_slice(vs.as_bytes()); p += vs.len();
    buf[p..p + 2].copy_from_slice(b"m "); p += 2;
    let vs = fmt_u32(&mut t, s); buf[p..p + vs.len()].copy_from_slice(vs.as_bytes()); p += vs.len();
    buf[p] = b's'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_bytes<'a>(buf: &'a mut [u8; 20], bytes: u64) -> &'a str {
    let mut t = [0u8; 12];
    let mut p = 0;
    if bytes >= 1024 * 1024 {
        let mib = bytes / (1024 * 1024);
        let frac = ((bytes % (1024 * 1024)) * 10 / (1024 * 1024)) as u32;
        let s = fmt_u32(&mut t, mib as u32); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p] = b'.'; p += 1;
        buf[p] = b'0' + frac as u8; p += 1;
        buf[p..p + 4].copy_from_slice(b" MiB"); p += 4;
    } else if bytes >= 1024 {
        let kib = bytes / 1024;
        let s = fmt_u32(&mut t, kib as u32); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p..p + 4].copy_from_slice(b" KiB"); p += 4;
    } else {
        let s = fmt_u32(&mut t, bytes as u32); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p..p + 2].copy_from_slice(b" B"); p += 2;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_io_summary<'a>(buf: &'a mut [u8; 48], read: u64, write: u64) -> &'a str {
    let mut p = 0;
    let mut rb = [0u8; 20];
    let rs = fmt_bytes(&mut rb, read);
    buf[p..p + 3].copy_from_slice(b"R: "); p += 3;
    buf[p..p + rs.len()].copy_from_slice(rs.as_bytes()); p += rs.len();
    buf[p..p + 6].copy_from_slice(b"   W: "); p += 6;
    let mut wb = [0u8; 20];
    let ws = fmt_bytes(&mut wb, write);
    buf[p..p + ws.len()].copy_from_slice(ws.as_bytes()); p += ws.len();
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_overall_cpu<'a>(buf: &'a mut [u8; 24], pct: u32, ncpu: u32) -> &'a str {
    let mut p = 0;
    let mut t = [0u8; 12];
    buf[p..p + 5].copy_from_slice(b"CPU: "); p += 5;
    let s = fmt_u32(&mut t, pct); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 4].copy_from_slice(b"%  ("); p += 4;
    let s = fmt_u32(&mut t, ncpu); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 2].copy_from_slice(b"C)"); p += 2;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_mem_pages<'a>(buf: &'a mut [u8; 16], pages: u32) -> &'a str {
    let kib = pages * 4;
    let mut t = [0u8; 12];
    let mut p = 0;
    if kib >= 1024 {
        let mib = kib / 1024;
        let frac = (kib % 1024) * 10 / 1024;
        let s = fmt_u32(&mut t, mib); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p] = b'.'; p += 1;
        buf[p] = b'0' + frac as u8; p += 1;
        buf[p] = b'M'; p += 1;
    } else {
        let s = fmt_u32(&mut t, kib); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
        buf[p] = b'K'; p += 1;
    }
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
    let win_id = window::create("Activity Monitor", 100, 60, 580, 420);
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

    let (mut win_w, mut win_h) = window::get_size(win_id).unwrap_or((580, 420));
    let kill_btn = UiToolbarButton::new(PAD, STATS_H + 4, 100, 24);

    let mut thread_buf = [0u8; THREAD_ENTRY_SIZE * 64];
    let mut event = [0u32; 5];
    let mut last_update: u32 = 0;
    let mut selected: Option<usize> = None;
    let mut cpu_state = CpuState::new();
    let mut prev_ticks = PrevTicks { entries: [(0, 0); MAX_TASKS], count: 0, prev_total: 0 };
    let mut scroll_offset: usize = 0;
    let mut icon_cache: Vec<IconEntry> = Vec::new();
    let mut active_tab: usize = 0;
    let mut hw_info = fetch_hwinfo();
    let mut cpu_history = CpuHistory::new();

    fetch_cpu(&mut cpu_state);
    cpu_history.push(&cpu_state);

    // Initial fetch and render
    let tasks = fetch_tasks(&mut thread_buf, &mut prev_ticks, cpu_state.total_sched_ticks);
    for task in &tasks {
        if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
            ensure_icon_cached(&mut icon_cache, name);
        }
    }
    let mem = fetch_memory();
    render(win_id, active_tab, &tasks, &mem, &cpu_state, &cpu_history, selected, &kill_btn, &icon_cache, scroll_offset, &hw_info, win_w, win_h);

    loop {
        let t0 = sys::uptime_ms();
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

            // Scroll (processes and disk tabs)
            if (active_tab == 0 || active_tab == 2) && event[0] == window::EVENT_MOUSE_SCROLL {
                let dz = event[1] as i32;
                if dz < 0 {
                    scroll_offset = scroll_offset.saturating_sub(3);
                } else if dz > 0 {
                    scroll_offset = scroll_offset.saturating_add(3);
                }
                last_update = 0;
            }

            // Mouse click
            if ev.is_mouse_down() {
                let (mx, my) = ev.mouse_pos();

                // Segmented control hit test
                if let Some(tab) = segmented_hit_test(PAD, 6, 380, 24, 4, mx, my) {
                    if tab != active_tab {
                        active_tab = tab;
                        if active_tab == 3 {
                            hw_info = fetch_hwinfo();
                        }
                        last_update = 0;
                    }
                } else if active_tab == 0 && kill_btn.handle_event(&ev) {
                    if let Some(sel_idx) = selected {
                        let tasks = fetch_tasks(&mut thread_buf, &mut prev_ticks, cpu_state.total_sched_ticks);
                        if sel_idx < tasks.len() && tasks[sel_idx].tid > 3 {
                            process::kill(tasks[sel_idx].tid);
                            selected = None;
                            window::disable_menu_item(win_id, 10);
                            last_update = 0;
                        }
                    }
                } else if active_tab == 0 {
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
            cpu_history.push(&cpu_state);
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

            if active_tab == 3 {
                hw_info = fetch_hwinfo();
            }
            let mem = fetch_memory();
            render(win_id, active_tab, &tasks, &mem, &cpu_state, &cpu_history, selected, &kill_btn, &icon_cache, scroll_offset, &hw_info, win_w, win_h);
            last_update = now;
        }

        let elapsed = sys::uptime_ms().wrapping_sub(t0);
        if elapsed < 16 { process::sleep(16 - elapsed); }
    }

    window::destroy(win_id);
}
