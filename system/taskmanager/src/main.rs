#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use anyos_std::sys;
use anyos_std::process;
use anyos_std::fs;
use anyos_std::icons;

anyos_std::entry!(main);

use libanyui_client as ui;
use ui::{ColumnDef, ALIGN_RIGHT};

// ─── Constants ───────────────────────────────────────────────────────────────

const MAX_CPUS: usize = 16;
const MAX_TASKS: usize = 64;
const THREAD_ENTRY_SIZE: usize = 60;
const ICON_SIZE: u32 = 16;
const GRAPH_SAMPLES: usize = 60;

// ─── Data Structures ─────────────────────────────────────────────────────────

struct TaskEntry {
    tid: u32,
    name: [u8; 24],
    name_len: usize,
    state: u8,
    priority: u8,
    arch: u8,
    uid: u16,
    user_pages: u32,
    cpu_pct_x10: u32,
    io_read_bytes: u64,
    io_write_bytes: u64,
}

struct PrevTicks {
    entries: [(u32, u32); MAX_TASKS],
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

// ─── CPU History (ring buffer for line graphs) ───────────────────────────────

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

    fn get(&self, core: usize, age: usize) -> u8 {
        if age >= self.count { return 0; }
        let idx = (self.pos + GRAPH_SAMPLES - 1 - age) % GRAPH_SAMPLES;
        self.samples[core][idx]
    }
}

// ─── Icon Cache ──────────────────────────────────────────────────────────────

struct IconEntry {
    name: String,
    pixels: Vec<u32>,
}

fn load_app_icon(name: &str) -> Vec<u32> {
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
    if count == u32::MAX { return result; }

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

        let uid = u16::from_le_bytes([buf[off + 56], buf[off + 57]]);

        result.push(TaskEntry { tid, name, name_len, state, priority: prio, arch, uid, user_pages, cpu_pct_x10, io_read_bytes, io_write_bytes });
    }

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
    if sys::sysinfo(0, &mut buf) != 0 { return None; }
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
        brand, vendor,
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

fn isqrt_ceil(n: usize) -> usize {
    if n <= 1 { return 1; }
    let mut x = 1;
    while x * x < n { x += 1; }
    x
}

fn trim_leading_spaces(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|&c| c != b' ').unwrap_or(b.len());
    &b[start..]
}

// ─── Global mutable state (accessed from timer + button callbacks) ───────────

static mut THREAD_BUF: [u8; THREAD_ENTRY_SIZE * 64] = [0u8; THREAD_ENTRY_SIZE * 64];
static mut PREV_TICKS: Option<*mut PrevTicks> = None;
static mut CPU_STATE: Option<*mut CpuState> = None;
static mut CPU_HISTORY: Option<*mut CpuHistory> = None;
static mut ICON_CACHE: Option<*mut Vec<IconEntry>> = None;

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    ui::init();

    let win = ui::Window::new("Activity Monitor", 100, 60, 580, 420);

    // ── Header card (always visible, top 70px) ──
    let header = ui::View::new();
    header.set_position(0, 0);
    header.set_size(580, 70);
    header.set_color(0xFF2A2A2A);
    win.add(&header);

    let seg = ui::SegmentedControl::new("Processes|Graphs|Disk|System");
    seg.set_position(8, 6);
    seg.set_size(380, 24);
    header.add(&seg);

    let uptime_label = ui::Label::new("");
    uptime_label.set_position(400, 6);
    uptime_label.set_size(170, 20);
    header.add(&uptime_label);

    let mem_label = ui::Label::new("");
    mem_label.set_position(8, 34);
    mem_label.set_size(400, 16);
    header.add(&mem_label);

    let mem_bar = ui::ProgressBar::new(0);
    mem_bar.set_position(8, 52);
    mem_bar.set_size(564, 6);
    header.add(&mem_bar);

    // ── Panel: Processes ──
    let panel_procs = ui::View::new();
    panel_procs.set_position(0, 70);
    panel_procs.set_size(580, 350);
    win.add(&panel_procs);

    let kill_btn = ui::Button::new("Kill Process");
    kill_btn.set_position(8, 4);
    kill_btn.set_size(100, 24);
    panel_procs.add(&kill_btn);

    let proc_info_label = ui::Label::new("");
    proc_info_label.set_position(120, 6);
    proc_info_label.set_size(440, 20);
    panel_procs.add(&proc_info_label);

    let proc_grid = ui::DataGrid::new(580, 318);
    proc_grid.set_position(0, 32);
    proc_grid.set_columns(&[
        ColumnDef::new("TID").width(45).align(ALIGN_RIGHT),
        ColumnDef::new("Process").width(130),
        ColumnDef::new("User").width(65),
        ColumnDef::new("State").width(70),
        ColumnDef::new("Arch").width(50),
        ColumnDef::new("CPU%").width(55).align(ALIGN_RIGHT),
        ColumnDef::new("Memory").width(65).align(ALIGN_RIGHT),
        ColumnDef::new("Priority").width(50).align(ALIGN_RIGHT),
    ]);
    proc_grid.set_row_height(24);
    panel_procs.add(&proc_grid);

    // ── Panel: Graphs ──
    let panel_graphs = ui::View::new();
    panel_graphs.set_position(0, 70);
    panel_graphs.set_size(580, 350);
    panel_graphs.set_visible(false);
    win.add(&panel_graphs);

    let graph_label = ui::Label::new("");
    graph_label.set_position(8, 4);
    graph_label.set_size(400, 20);
    panel_graphs.add(&graph_label);

    let graph_bar = ui::ProgressBar::new(0);
    graph_bar.set_position(8, 24);
    graph_bar.set_size(564, 6);
    panel_graphs.add(&graph_bar);

    let cpu_canvas = ui::Canvas::new(564, 306);
    cpu_canvas.set_position(8, 38);
    panel_graphs.add(&cpu_canvas);

    // ── Panel: Disk ──
    let panel_disk = ui::View::new();
    panel_disk.set_position(0, 70);
    panel_disk.set_size(580, 350);
    panel_disk.set_visible(false);
    win.add(&panel_disk);

    let disk_summary = ui::Label::new("");
    disk_summary.set_position(8, 4);
    disk_summary.set_size(400, 20);
    panel_disk.add(&disk_summary);

    let disk_grid = ui::DataGrid::new(580, 322);
    disk_grid.set_position(0, 28);
    disk_grid.set_columns(&[
        ColumnDef::new("TID").width(50).align(ALIGN_RIGHT),
        ColumnDef::new("Process").width(160),
        ColumnDef::new("Read").width(130).align(ALIGN_RIGHT),
        ColumnDef::new("Written").width(130).align(ALIGN_RIGHT),
    ]);
    disk_grid.set_row_height(24);
    panel_disk.add(&disk_grid);

    // ── Panel: System ──
    let panel_system = ui::View::new();
    panel_system.set_position(0, 70);
    panel_system.set_size(580, 350);
    panel_system.set_visible(false);
    win.add(&panel_system);

    // System tab is text labels — we'll fill them in the timer
    let sys_text = ui::Label::new("");
    sys_text.set_position(8, 8);
    sys_text.set_size(560, 330);
    panel_system.add(&sys_text);

    // ── Connect segmented control to panels ──
    seg.connect_panels(&[&panel_procs, &panel_graphs, &panel_disk, &panel_system]);

    // ── Allocate state on heap (accessed from callbacks) ──
    let prev_ticks = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(
        PrevTicks { entries: [(0, 0); MAX_TASKS], count: 0, prev_total: 0 }
    ));
    let cpu_state = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(CpuState::new()));
    let cpu_history = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(CpuHistory::new()));
    let icon_cache = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Vec::<IconEntry>::new()));

    unsafe {
        PREV_TICKS = Some(prev_ticks);
        CPU_STATE = Some(cpu_state);
        CPU_HISTORY = Some(cpu_history);
        ICON_CACHE = Some(icon_cache);
    }

    // Initial CPU fetch
    unsafe { fetch_cpu(&mut *cpu_state); }
    unsafe { (*cpu_history).push(&*cpu_state); }

    // ── Kill button handler ──
    kill_btn.on_click(move |_| {
        let sel = proc_grid.selected_row();
        if sel != u32::MAX {
            let mut tid_buf = [0u8; 12];
            let len = proc_grid.get_cell(sel, 0, &mut tid_buf);
            if len > 0 {
                let tid = parse_u32_bytes(&tid_buf[..len as usize]).unwrap_or(0);
                if tid > 3 {
                    process::kill(tid);
                }
            }
        }
    });

    // ── Timer: refresh every 500ms ──
    // All controls are Copy — captured directly by value.
    ui::set_timer(500, move || {
        let cpu_st = unsafe { &mut *CPU_STATE.unwrap() };
        let hist = unsafe { &mut *CPU_HISTORY.unwrap() };
        let prev = unsafe { &mut *PREV_TICKS.unwrap() };
        let cache = unsafe { &mut *ICON_CACHE.unwrap() };
        let tbuf = unsafe { &mut THREAD_BUF };

        // Fetch data
        fetch_cpu(cpu_st);
        hist.push(cpu_st);
        let tasks = fetch_tasks(tbuf, prev, cpu_st.total_sched_ticks);

        // Cache icons
        for task in &tasks {
            if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
                ensure_icon_cached(cache, name);
            }
        }

        // ── Update uptime label ──
        {
            let ticks = sys::uptime();
            let hz = sys::tick_hz();
            let total_secs = if hz > 0 { ticks / hz } else { 0 };
            let hours = total_secs / 3600;
            let mins = (total_secs % 3600) / 60;
            let secs = total_secs % 60;
            let mut ubuf = [0u8; 32];
            let mut p = 0;
            let mut t = [0u8; 12];
            if hours > 0 {
                let s = fmt_u32(&mut t, hours as u32); ubuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                ubuf[p..p + 2].copy_from_slice(b"h "); p += 2;
            }
            let s = fmt_u32(&mut t, mins as u32); ubuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            ubuf[p..p + 2].copy_from_slice(b"m "); p += 2;
            let s = fmt_u32(&mut t, secs as u32); ubuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            ubuf[p] = b's'; p += 1;
            if let Ok(s) = core::str::from_utf8(&ubuf[..p]) {
                uptime_label.set_text(s);
            }
        }

        // ── Update memory info ──
        if let Some(mem) = fetch_memory() {
            let total_kb = mem.total_frames * 4;
            let free_kb = mem.free_frames * 4;
            let used_kb = total_kb - free_kb;
            let heap_kb = mem.heap_used / 1024;
            let heap_total_kb = mem.heap_total / 1024;

            let mut mbuf = [0u8; 80];
            let mut p = 0;
            let mut t = [0u8; 12];
            mbuf[p..p + 5].copy_from_slice(b"Mem: "); p += 5;
            let s = fmt_u32(&mut t, used_kb / 1024); mbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            mbuf[p] = b'/'; p += 1;
            let s = fmt_u32(&mut t, total_kb / 1024); mbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            mbuf[p..p + 8].copy_from_slice(b"M  Heap:"); p += 8;
            let s = fmt_u32(&mut t, heap_kb); mbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            mbuf[p] = b'/'; p += 1;
            let s = fmt_u32(&mut t, heap_total_kb); mbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            mbuf[p] = b'K'; p += 1;
            if let Ok(s) = core::str::from_utf8(&mbuf[..p]) {
                mem_label.set_text(s);
            }

            if total_kb > 0 {
                mem_bar.set_state((used_kb as u64 * 100 / total_kb as u64) as u32);
            }
        }

        // ── Update processes tab ──
        {
            let mut ibuf = [0u8; 32];
            let mut p = 0;
            let mut t = [0u8; 12];
            let s = fmt_u32(&mut t, tasks.len() as u32); ibuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            ibuf[p..p + 11].copy_from_slice(b" proc  CPU:"); p += 11;
            let s = fmt_u32(&mut t, cpu_st.overall_pct); ibuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            ibuf[p] = b'%'; p += 1;
            if let Ok(s) = core::str::from_utf8(&ibuf[..p]) {
                proc_info_label.set_text(s);
            }
        }

        // Build row data into a flat encoded buffer (0x1E = row sep, 0x1F = col sep)
        let mut buf = Vec::new();
        for (ri, task) in tasks.iter().enumerate() {
            if ri > 0 { buf.push(0x1E); }
            // TID
            let mut t = [0u8; 12];
            let s = fmt_u32(&mut t, task.tid);
            buf.extend_from_slice(s.as_bytes());
            buf.push(0x1F);
            // Process name
            buf.extend_from_slice(&task.name[..task.name_len]);
            buf.push(0x1F);
            // User
            {
                let mut ubuf = [0u8; 16];
                let nlen = process::getusername(task.uid, &mut ubuf);
                if nlen != u32::MAX && nlen > 0 {
                    buf.extend_from_slice(&ubuf[..nlen as usize]);
                } else {
                    buf.push(b'?');
                }
            }
            buf.push(0x1F);
            // State
            let state_str = match task.state {
                0 => b"Ready" as &[u8],
                1 => b"Running",
                2 => b"Blocked",
                3 => b"Terminated",
                _ => b"Unknown",
            };
            buf.extend_from_slice(state_str);
            buf.push(0x1F);
            // Arch
            let arch_str = if task.arch == 1 { b"x86" as &[u8] } else { b"x86_64" };
            buf.extend_from_slice(arch_str);
            buf.push(0x1F);
            // CPU%
            {
                let mut cbuf = [0u8; 12];
                let s = if task.cpu_pct_x10 > 0 {
                    fmt_pct(&mut cbuf, task.cpu_pct_x10)
                } else {
                    "0.0%"
                };
                buf.extend_from_slice(s.as_bytes());
            }
            buf.push(0x1F);
            // Memory
            {
                let mut mbuf = [0u8; 16];
                let s = fmt_mem_pages(&mut mbuf, task.user_pages);
                buf.extend_from_slice(s.as_bytes());
            }
            buf.push(0x1F);
            // Priority
            {
                let mut pbuf = [0u8; 12];
                let s = fmt_u32(&mut pbuf, task.priority as u32);
                buf.extend_from_slice(s.as_bytes());
            }
        }

        proc_grid.set_data_raw(&buf);

        // Set icons for Process column (column 1)
        for (ri, task) in tasks.iter().enumerate() {
            if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
                if let Some(pixels) = find_icon(cache, name) {
                    proc_grid.set_cell_icon(ri as u32, 1, pixels, ICON_SIZE, ICON_SIZE);
                }
            }
        }

        // State colors
        let col_count = 8u32;
        let mut colors = vec![0u32; tasks.len() * col_count as usize];
        for (ri, task) in tasks.iter().enumerate() {
            let state_color = match task.state {
                0 => 0xFFFFD60A,  // Ready = yellow
                1 => 0xFF30D158,  // Running = green
                2 => 0xFFFF3B30,  // Blocked = red
                3 => 0xFF8E8E93,  // Terminated = gray
                _ => 0xFF8E8E93,
            };
            colors[ri * col_count as usize + 3] = state_color; // State column
            if task.arch == 1 {
                colors[ri * col_count as usize + 4] = 0xFFFF9500; // Arch column for x86
            }
        }
        proc_grid.set_cell_colors(&colors);

        // ── Update graphs tab ──
        {
            let ncpu = (cpu_st.num_cpus as usize).max(1).min(MAX_CPUS);
            let mut gbuf = [0u8; 24];
            let mut p = 0;
            let mut t = [0u8; 12];
            gbuf[p..p + 5].copy_from_slice(b"CPU: "); p += 5;
            let s = fmt_u32(&mut t, cpu_st.overall_pct); gbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            gbuf[p..p + 4].copy_from_slice(b"%  ("); p += 4;
            let s = fmt_u32(&mut t, ncpu as u32); gbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            gbuf[p..p + 2].copy_from_slice(b"C)"); p += 2;
            if let Ok(s) = core::str::from_utf8(&gbuf[..p]) {
                graph_label.set_text(s);
            }
            graph_bar.set_state(cpu_st.overall_pct);

            // Draw CPU graphs on canvas
            let cw = 564u32;
            let ch = 306u32;
            cpu_canvas.clear(0xFF1A1A1A);

            let cols = isqrt_ceil(ncpu);
            let rows = (ncpu + cols - 1) / cols;
            let gap = 6i32;
            let cell_w = ((cw as i32) - gap * (cols as i32 - 1).max(0)) / cols as i32;
            let cell_h = ((ch as i32) - gap * (rows as i32 - 1).max(0)) / rows as i32;

            if cell_w >= 20 && cell_h >= 20 {
                for core in 0..ncpu {
                    let col = core % cols;
                    let row = core / cols;
                    let cx = col as i32 * (cell_w + gap);
                    let cy = row as i32 * (cell_h + gap);
                    draw_cpu_graph(&cpu_canvas, cx, cy, cell_w as u32, cell_h as u32, core, cpu_st.core_pct[core], hist);
                }
            }
        }

        // ── Update disk tab ──
        {
            let mut total_read: u64 = 0;
            let mut total_write: u64 = 0;
            for t in &tasks {
                total_read += t.io_read_bytes;
                total_write += t.io_write_bytes;
            }

            let mut sbuf = [0u8; 48];
            let mut p = 0;
            let mut rb = [0u8; 20];
            let rs = fmt_bytes(&mut rb, total_read);
            sbuf[p..p + 3].copy_from_slice(b"R: "); p += 3;
            sbuf[p..p + rs.len()].copy_from_slice(rs.as_bytes()); p += rs.len();
            sbuf[p..p + 6].copy_from_slice(b"   W: "); p += 6;
            let mut wb = [0u8; 20];
            let ws = fmt_bytes(&mut wb, total_write);
            sbuf[p..p + ws.len()].copy_from_slice(ws.as_bytes()); p += ws.len();
            if let Ok(s) = core::str::from_utf8(&sbuf[..p]) {
                disk_summary.set_text(s);
            }

            // Build disk grid data (only processes with I/O)
            let mut dbuf = Vec::new();
            let mut first = true;
            for task in &tasks {
                if task.io_read_bytes == 0 && task.io_write_bytes == 0 { continue; }
                if !first { dbuf.push(0x1E); }
                first = false;
                // TID
                let mut t = [0u8; 12];
                let s = fmt_u32(&mut t, task.tid);
                dbuf.extend_from_slice(s.as_bytes());
                dbuf.push(0x1F);
                // Process
                dbuf.extend_from_slice(&task.name[..task.name_len]);
                dbuf.push(0x1F);
                // Read
                let mut rb = [0u8; 20];
                let rs = fmt_bytes(&mut rb, task.io_read_bytes);
                dbuf.extend_from_slice(rs.as_bytes());
                dbuf.push(0x1F);
                // Written
                let mut wb = [0u8; 20];
                let ws = fmt_bytes(&mut wb, task.io_write_bytes);
                dbuf.extend_from_slice(ws.as_bytes());
            }
            disk_grid.set_data_raw(&dbuf);
        }

        // ── Update system tab ──
        {
            let hw = fetch_hwinfo();
            let mut sbuf = [0u8; 512];
            let mut p = 0;

            // Processor section
            let brand_len = hw.brand.iter().position(|&b| b == 0).unwrap_or(48);
            let brand_trimmed = trim_leading_spaces(&hw.brand[..brand_len]);
            if let Ok(brand_str) = core::str::from_utf8(brand_trimmed) {
                sbuf[p..p + 6].copy_from_slice(b"CPU:  "); p += 6;
                let bl = brand_str.len().min(80);
                sbuf[p..p + bl].copy_from_slice(&brand_str.as_bytes()[..bl]); p += bl;
                sbuf[p] = b'\n'; p += 1;
            }

            let vendor_len = hw.vendor.iter().position(|&b| b == 0).unwrap_or(16);
            if let Ok(vendor_str) = core::str::from_utf8(&hw.vendor[..vendor_len]) {
                sbuf[p..p + 10].copy_from_slice(b"Vendor:   "); p += 10;
                let vl = vendor_str.len().min(16);
                sbuf[p..p + vl].copy_from_slice(&vendor_str.as_bytes()[..vl]); p += vl;
                sbuf[p] = b'\n'; p += 1;
            }

            let mut t = [0u8; 12];
            sbuf[p..p + 10].copy_from_slice(b"Cores:    "); p += 10;
            let s = fmt_u32(&mut t, hw.cpu_count); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            sbuf[p] = b'\n'; p += 1;

            sbuf[p..p + 10].copy_from_slice(b"Speed:    "); p += 10;
            let s = fmt_u32(&mut t, hw.tsc_mhz); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            sbuf[p..p + 4].copy_from_slice(b" MHz"); p += 4;
            sbuf[p] = b'\n'; p += 1;

            sbuf[p..p + 10].copy_from_slice(b"Memory:   "); p += 10;
            let s = fmt_u32(&mut t, hw.total_mem_mib); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            sbuf[p..p + 4].copy_from_slice(b" MiB"); p += 4;
            sbuf[p] = b'\n'; p += 1;

            sbuf[p..p + 10].copy_from_slice(b"Free:     "); p += 10;
            let s = fmt_u32(&mut t, hw.free_mem_mib); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            sbuf[p..p + 4].copy_from_slice(b" MiB"); p += 4;
            sbuf[p] = b'\n'; p += 1;

            sbuf[p..p + 10].copy_from_slice(b"Boot:     "); p += 10;
            let boot = if hw.boot_mode == 1 { b"UEFI" as &[u8] } else { b"BIOS" };
            sbuf[p..p + boot.len()].copy_from_slice(boot); p += boot.len();
            sbuf[p] = b'\n'; p += 1;

            if hw.fb_width > 0 && hw.fb_height > 0 {
                sbuf[p..p + 10].copy_from_slice(b"Display:  "); p += 10;
                let s = fmt_u32(&mut t, hw.fb_width); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                sbuf[p] = b'x'; p += 1;
                let s = fmt_u32(&mut t, hw.fb_height); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                sbuf[p..p + 2].copy_from_slice(b" ("); p += 2;
                let s = fmt_u32(&mut t, hw.fb_bpp); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                sbuf[p..p + 5].copy_from_slice(b"-bit)"); p += 5;
                sbuf[p] = b'\n'; p += 1;
            }

            // Uptime
            let ticks = sys::uptime();
            let hz = sys::tick_hz();
            let total_secs = if hz > 0 { ticks / hz } else { 0 };
            let hours = total_secs / 3600;
            let mins = (total_secs % 3600) / 60;
            let secs = total_secs % 60;
            sbuf[p..p + 10].copy_from_slice(b"Uptime:   "); p += 10;
            let s = fmt_u32(&mut t, hours as u32); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            sbuf[p..p + 2].copy_from_slice(b"h "); p += 2;
            let s = fmt_u32(&mut t, mins as u32); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            sbuf[p..p + 2].copy_from_slice(b"m "); p += 2;
            let s = fmt_u32(&mut t, secs as u32); sbuf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
            sbuf[p] = b's'; p += 1;

            if let Ok(s) = core::str::from_utf8(&sbuf[..p]) {
                sys_text.set_text(s);
            }
        }
    });

    ui::run();
}

// ─── CPU Graph Drawing ───────────────────────────────────────────────────────

fn draw_cpu_graph(
    cv: &ui::Canvas,
    x: i32, y: i32, w: u32, h: u32,
    core: usize, current_pct: u32, history: &CpuHistory,
) {
    let graph_bg = 0xFF1A1A2E;
    let grid_color = 0xFF2A2A3E;
    let line_color = 0xFF00C8FF;
    let fill_color = 0xFF0D2840;

    cv.fill_rect(x, y, w, h, graph_bg);

    // Title: "N: XX%"
    let label_h = 16u32;
    // Draw graph area below label
    let gy = y + label_h as i32;
    let gh = h.saturating_sub(label_h);
    if gh < 8 { return; }

    // Grid lines at 25%, 50%, 75%
    for pct in [25u32, 50, 75] {
        let ly = gy + (gh as i32 - (pct as i32 * gh as i32 / 100));
        cv.fill_rect(x, ly, w, 1, grid_color);
    }

    // Line graph
    let sample_count = history.count;
    if sample_count < 2 { return; }
    let num_pts = (w as usize).min(sample_count);

    let mut prev_vy: i32 = -1;
    for px in 0..w {
        let age = if w > 1 {
            ((w - 1 - px) as usize * (num_pts - 1)) / (w as usize - 1).max(1)
        } else {
            0
        };
        let pct = history.get(core, age) as i32;
        let val_h = pct * gh as i32 / 100;
        let vy = gy + gh as i32 - val_h;

        if val_h > 0 {
            cv.fill_rect(x + px as i32, vy, 1, val_h as u32, fill_color);
        }

        if prev_vy >= 0 {
            let y0 = prev_vy.min(vy);
            let y1 = prev_vy.max(vy);
            let seg_h = (y1 - y0 + 1).max(1);
            cv.fill_rect(x + px as i32, y0, 1, seg_h as u32, line_color);
        } else {
            cv.fill_rect(x + px as i32, vy, 1, 1, line_color);
        }
        prev_vy = vy;
    }

    // Border
    let border = 0xFF3A3A4E;
    cv.fill_rect(x, y, w, 1, border);
    cv.fill_rect(x, y + h as i32 - 1, w, 1, border);
    cv.fill_rect(x, y, 1, h, border);
    cv.fill_rect(x + w as i32 - 1, y, 1, h, border);
}

fn parse_u32_bytes(s: &[u8]) -> Option<u32> {
    if s.is_empty() { return None; }
    let mut val = 0u32;
    for &b in s {
        if b < b'0' || b > b'9' { return None; }
        val = val * 10 + (b - b'0') as u32;
    }
    Some(val)
}
