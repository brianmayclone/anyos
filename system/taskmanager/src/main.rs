#![no_std]
#![no_main]

mod types;
mod data;
mod format;
mod icon_cache;
mod graph;

use alloc::vec::Vec;

use anyos_std::sys;
use anyos_std::process;

anyos_std::entry!(main);

use libanyui_client as ui;
use ui::{ColumnDef, ALIGN_RIGHT};

use types::*;
use data::*;
use format::*;
use icon_cache::*;
use graph::*;

// ─── Global mutable state (accessed from timer + button callbacks) ───────────

static mut THREAD_BUF: [u8; THREAD_ENTRY_SIZE * 64] = [0u8; THREAD_ENTRY_SIZE * 64];
static mut PREV_TICKS: Option<*mut PrevTicks> = None;
static mut CPU_STATE: Option<*mut CpuState> = None;
static mut CPU_HISTORY: Option<*mut CpuHistory> = None;
static mut ICON_CACHE: Option<*mut Vec<IconEntry>> = None;

// Change tracking for incremental updates
static mut PREV_PROC_COUNT: usize = 0;
static mut PREV_TASK_TIDS: Option<*mut Vec<u32>> = None;
static mut PREV_TASK_STATES: Option<*mut Vec<u8>> = None;
static mut PREV_DISK_COUNT: usize = 0;

// Reusable buffers (avoid per-tick heap allocations)
static mut TASKS_BUF: Option<*mut Vec<TaskEntry>> = None;
static mut COLORS_BUF: Option<*mut Vec<u32>> = None;

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    ui::init();

    let win = ui::Window::new("Activity Monitor", 100, 60, 580, 420);

    // ── Header (DOCK_TOP, 76px) ──
    let header = ui::View::new();
    header.set_size(0, 76);
    header.set_dock(ui::DOCK_TOP);
    header.set_color(0xFF2A2A2A);
    header.set_padding(0, 0, 0, 4);
    win.add(&header);

    // Header top row (DOCK_TOP, 30px): segmented control + uptime
    let header_top = ui::View::new();
    header_top.set_size(0, 30);
    header_top.set_dock(ui::DOCK_TOP);
    header_top.set_color(0xFF2A2A2A);
    header.add(&header_top);

    let seg = ui::SegmentedControl::new("Processes|Graphs|Disk|System");
    seg.set_size(380, 24);
    seg.set_dock(ui::DOCK_LEFT);
    seg.set_margin(8, 3, 0, 0);
    header_top.add(&seg);

    let uptime_label = ui::Label::new("");
    uptime_label.set_size(170, 24);
    uptime_label.set_dock(ui::DOCK_RIGHT);
    uptime_label.set_margin(0, 5, 8, 0);
    uptime_label.set_text_color(0xFF8E8E93); // dimmed gray
    uptime_label.set_font_size(11);
    uptime_label.set_text_align(ui::TEXT_ALIGN_RIGHT);
    header_top.add(&uptime_label);

    // Memory info line (DOCK_TOP, 18px)
    let mem_label = ui::Label::new("");
    mem_label.set_size(0, 18);
    mem_label.set_dock(ui::DOCK_TOP);
    mem_label.set_margin(8, 0, 8, 0);
    header.add(&mem_label);

    // Memory bar (DOCK_TOP, 6px)
    let mem_bar = ui::ProgressBar::new(0);
    mem_bar.set_size(0, 6);
    mem_bar.set_dock(ui::DOCK_TOP);
    mem_bar.set_margin(8, 4, 8, 0);
    header.add(&mem_bar);

    // ── Panel: Processes (DOCK_FILL) ──
    let panel_procs = ui::View::new();
    panel_procs.set_dock(ui::DOCK_FILL);
    win.add(&panel_procs);

    // Process toolbar (DOCK_TOP, 32px)
    let proc_toolbar = ui::View::new();
    proc_toolbar.set_size(0, 32);
    proc_toolbar.set_dock(ui::DOCK_TOP);
    panel_procs.add(&proc_toolbar);

    let kill_btn = ui::Button::new("Kill Process");
    kill_btn.set_position(8, 4);
    kill_btn.set_size(100, 24);
    proc_toolbar.add(&kill_btn);

    let proc_info_label = ui::Label::new("");
    proc_info_label.set_position(120, 6);
    proc_info_label.set_size(440, 20);
    proc_toolbar.add(&proc_info_label);

    // Process grid (DOCK_FILL)
    let proc_grid = ui::DataGrid::new(400, 200);
    proc_grid.set_dock(ui::DOCK_FILL);
    proc_grid.set_font_size(11);
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
    proc_grid.set_row_height(20);
    panel_procs.add(&proc_grid);

    // ── Panel: Graphs (DOCK_FILL, initially hidden) ──
    let panel_graphs = ui::View::new();
    panel_graphs.set_dock(ui::DOCK_FILL);
    panel_graphs.set_visible(false);
    win.add(&panel_graphs);

    // Graph info area (DOCK_TOP, 34px)
    let graph_info = ui::View::new();
    graph_info.set_size(0, 34);
    graph_info.set_dock(ui::DOCK_TOP);
    panel_graphs.add(&graph_info);

    let graph_label = ui::Label::new("");
    graph_label.set_size(0, 20);
    graph_label.set_dock(ui::DOCK_TOP);
    graph_label.set_margin(8, 4, 8, 0);
    graph_info.add(&graph_label);

    let graph_bar = ui::ProgressBar::new(0);
    graph_bar.set_size(0, 6);
    graph_bar.set_dock(ui::DOCK_TOP);
    graph_bar.set_margin(8, 2, 8, 0);
    graph_info.add(&graph_bar);

    // CPU canvas (DOCK_FILL — buffer auto-resizes)
    let cpu_canvas = ui::Canvas::new(400, 200);
    cpu_canvas.set_dock(ui::DOCK_FILL);
    cpu_canvas.set_margin(4, 2, 4, 4);
    panel_graphs.add(&cpu_canvas);

    // ── Panel: Disk (DOCK_FILL, initially hidden) ──
    let panel_disk = ui::View::new();
    panel_disk.set_dock(ui::DOCK_FILL);
    panel_disk.set_visible(false);
    win.add(&panel_disk);

    let disk_summary = ui::Label::new("");
    disk_summary.set_size(0, 28);
    disk_summary.set_dock(ui::DOCK_TOP);
    disk_summary.set_margin(8, 4, 8, 0);
    panel_disk.add(&disk_summary);

    let disk_grid = ui::DataGrid::new(400, 200);
    disk_grid.set_dock(ui::DOCK_FILL);
    disk_grid.set_font_size(11);
    disk_grid.set_columns(&[
        ColumnDef::new("TID").width(50).align(ALIGN_RIGHT),
        ColumnDef::new("Process").width(160),
        ColumnDef::new("Read").width(130).align(ALIGN_RIGHT),
        ColumnDef::new("Written").width(130).align(ALIGN_RIGHT),
    ]);
    disk_grid.set_row_height(20);
    panel_disk.add(&disk_grid);

    // ── Panel: System (DOCK_FILL, initially hidden) ──
    let panel_system = ui::View::new();
    panel_system.set_dock(ui::DOCK_FILL);
    panel_system.set_visible(false);
    win.add(&panel_system);

    // ScrollView → StackPanel → Cards
    let sys_scroll = ui::ScrollView::new();
    sys_scroll.set_dock(ui::DOCK_FILL);
    panel_system.add(&sys_scroll);

    let sys_stack = ui::StackPanel::vertical();
    sys_stack.set_size(580, 0); // auto-height
    sys_stack.set_padding(8, 8, 8, 8);
    sys_scroll.add(&sys_stack);

    // -- Processor card --
    let cpu_card = ui::Card::new();
    cpu_card.set_size(560, 110);
    cpu_card.set_margin(0, 0, 0, 8);
    cpu_card.set_padding(12, 8, 12, 8);
    sys_stack.add(&cpu_card);

    let cpu_card_stack = ui::StackPanel::vertical();
    cpu_card_stack.set_dock(ui::DOCK_FILL);
    cpu_card.add(&cpu_card_stack);

    let cpu_title = ui::Label::new("Processor");
    cpu_title.set_size(536, 20);
    cpu_title.set_font_size(13);
    cpu_title.set_text_color(0xFF0A84FF);
    cpu_card_stack.add(&cpu_title);

    let cpu_brand_label = ui::Label::new("");
    cpu_brand_label.set_size(536, 18);
    cpu_brand_label.set_margin(0, 2, 0, 0);
    cpu_card_stack.add(&cpu_brand_label);

    let cpu_vendor_label = ui::Label::new("");
    cpu_vendor_label.set_size(536, 18);
    cpu_card_stack.add(&cpu_vendor_label);

    let cpu_cores_label = ui::Label::new("");
    cpu_cores_label.set_size(536, 18);
    cpu_card_stack.add(&cpu_cores_label);

    let cpu_speed_label = ui::Label::new("");
    cpu_speed_label.set_size(536, 18);
    cpu_card_stack.add(&cpu_speed_label);

    // -- Memory card --
    let mem_card = ui::Card::new();
    mem_card.set_size(560, 72);
    mem_card.set_margin(0, 0, 0, 8);
    mem_card.set_padding(12, 8, 12, 8);
    sys_stack.add(&mem_card);

    let mem_card_stack = ui::StackPanel::vertical();
    mem_card_stack.set_dock(ui::DOCK_FILL);
    mem_card.add(&mem_card_stack);

    let mem_title = ui::Label::new("Memory");
    mem_title.set_size(536, 20);
    mem_title.set_font_size(13);
    mem_title.set_text_color(0xFF0A84FF);
    mem_card_stack.add(&mem_title);

    let mem_total_label = ui::Label::new("");
    mem_total_label.set_size(536, 18);
    mem_total_label.set_margin(0, 2, 0, 0);
    mem_card_stack.add(&mem_total_label);

    let mem_free_label = ui::Label::new("");
    mem_free_label.set_size(536, 18);
    mem_card_stack.add(&mem_free_label);

    // -- System card --
    let sys_card = ui::Card::new();
    sys_card.set_size(560, 72);
    sys_card.set_margin(0, 0, 0, 8);
    sys_card.set_padding(12, 8, 12, 8);
    sys_stack.add(&sys_card);

    let sys_card_stack = ui::StackPanel::vertical();
    sys_card_stack.set_dock(ui::DOCK_FILL);
    sys_card.add(&sys_card_stack);

    let sys_title = ui::Label::new("System");
    sys_title.set_size(536, 20);
    sys_title.set_font_size(13);
    sys_title.set_text_color(0xFF0A84FF);
    sys_card_stack.add(&sys_title);

    let boot_label = ui::Label::new("");
    boot_label.set_size(536, 18);
    boot_label.set_margin(0, 2, 0, 0);
    sys_card_stack.add(&boot_label);

    let uptime_sys_label = ui::Label::new("");
    uptime_sys_label.set_size(536, 18);
    sys_card_stack.add(&uptime_sys_label);

    // -- Display card --
    let disp_card = ui::Card::new();
    disp_card.set_size(560, 54);
    disp_card.set_margin(0, 0, 0, 8);
    disp_card.set_padding(12, 8, 12, 8);
    sys_stack.add(&disp_card);

    let disp_card_stack = ui::StackPanel::vertical();
    disp_card_stack.set_dock(ui::DOCK_FILL);
    disp_card.add(&disp_card_stack);

    let disp_title = ui::Label::new("Display");
    disp_title.set_size(536, 20);
    disp_title.set_font_size(13);
    disp_title.set_text_color(0xFF0A84FF);
    disp_card_stack.add(&disp_title);

    let disp_res_label = ui::Label::new("");
    disp_res_label.set_size(536, 18);
    disp_res_label.set_margin(0, 2, 0, 0);
    disp_card_stack.add(&disp_res_label);

    // ── Connect segmented control to panels ──
    seg.connect_panels(&[&panel_procs, &panel_graphs, &panel_disk, &panel_system]);

    // ── Allocate state on heap (accessed from callbacks) ──
    let prev_ticks = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(
        PrevTicks { entries: [(0, 0); MAX_TASKS], count: 0, prev_total: 0 }
    ));
    let cpu_state = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(CpuState::new()));
    let cpu_history = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(CpuHistory::new()));
    let icon_cache = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Vec::<IconEntry>::new()));

    let prev_task_tids = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Vec::<u32>::new()));
    let prev_task_states = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Vec::<u8>::new()));
    let tasks_buf = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Vec::<TaskEntry>::new()));
    let colors_buf = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Vec::<u32>::new()));

    unsafe {
        PREV_TICKS = Some(prev_ticks);
        CPU_STATE = Some(cpu_state);
        CPU_HISTORY = Some(cpu_history);
        ICON_CACHE = Some(icon_cache);
        PREV_TASK_TIDS = Some(prev_task_tids);
        PREV_TASK_STATES = Some(prev_task_states);
        TASKS_BUF = Some(tasks_buf);
        COLORS_BUF = Some(colors_buf);
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
        let prev_tids = unsafe { &mut *PREV_TASK_TIDS.unwrap() };
        let prev_states = unsafe { &mut *PREV_TASK_STATES.unwrap() };
        let tasks = unsafe { &mut *TASKS_BUF.unwrap() };
        let colors = unsafe { &mut *COLORS_BUF.unwrap() };

        // Fetch data (always needed for CPU history + task tracking)
        fetch_cpu(cpu_st);
        hist.push(cpu_st);
        fetch_tasks(tbuf, prev, cpu_st.total_sched_ticks, tasks);

        let active_tab = seg.get_state();

        // ── Update uptime label ──
        {
            let ticks = sys::uptime();
            let hz = sys::tick_hz();
            let total_secs = if hz > 0 { ticks / hz } else { 0 };
            let hours = total_secs / 3600;
            let mins = (total_secs % 3600) / 60;
            let secs = total_secs % 60;
            let mut ubuf = [0u8; 40];
            let mut p = 0;
            let mut t = [0u8; 12];
            ubuf[p..p + 8].copy_from_slice(b"Uptime: "); p += 8;
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

        // ── Update processes tab (incremental) ──
        if active_tab == 0 {
            // Status bar
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

            let old_count = unsafe { PREV_PROC_COUNT };
            let new_count = tasks.len();
            if new_count != old_count {
                proc_grid.set_row_count(new_count as u32);
            }

            let mut colors_dirty = false;
            let col_count = 8usize;
            let needed = new_count * col_count;
            colors.clear();
            colors.resize(needed, 0u32);

            for (ri, task) in tasks.iter().enumerate() {
                let old_tid = prev_tids.get(ri).copied().unwrap_or(u32::MAX);
                let old_state = prev_states.get(ri).copied().unwrap_or(255);
                let is_new = ri >= old_count || old_tid != task.tid;

                // Static columns: only when process is new or TID changed
                if is_new {
                    let mut t = [0u8; 12];
                    let s = fmt_u32(&mut t, task.tid);
                    proc_grid.set_cell(ri as u32, 0, s);
                    proc_grid.set_cell(ri as u32, 1, core::str::from_utf8(&task.name[..task.name_len]).unwrap_or(""));
                    {
                        let mut ubuf = [0u8; 16];
                        let nlen = process::getusername(task.uid, &mut ubuf);
                        if nlen != u32::MAX && nlen > 0 {
                            if let Ok(s) = core::str::from_utf8(&ubuf[..nlen as usize]) {
                                proc_grid.set_cell(ri as u32, 2, s);
                            }
                        } else {
                            proc_grid.set_cell(ri as u32, 2, "?");
                        }
                    }
                    let arch_str = if task.arch == 1 { "x86" } else { "x86_64" };
                    proc_grid.set_cell(ri as u32, 4, arch_str);
                    {
                        let mut t = [0u8; 12];
                        let s = fmt_u32(&mut t, task.priority as u32);
                        proc_grid.set_cell(ri as u32, 7, s);
                    }

                    // Icon only for new processes
                    if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
                        ensure_icon_cached(cache, name);
                        if let Some(pixels) = find_icon(cache, name) {
                            proc_grid.set_cell_icon(ri as u32, 1, pixels, ICON_SIZE, ICON_SIZE);
                        }
                    }
                }

                // State: only when changed
                if is_new || old_state != task.state {
                    let state_str = match task.state {
                        0 => "Ready",
                        1 => "Running",
                        2 => "Blocked",
                        3 => "Terminated",
                        _ => "Unknown",
                    };
                    proc_grid.set_cell(ri as u32, 3, state_str);
                    colors_dirty = true;
                }

                // Volatile columns: always update (but set_cell checks for changes)
                {
                    let mut cbuf = [0u8; 12];
                    let s = if task.cpu_pct_x10 > 0 {
                        fmt_pct(&mut cbuf, task.cpu_pct_x10)
                    } else {
                        "0.0%"
                    };
                    proc_grid.set_cell(ri as u32, 5, s);
                }
                {
                    let mut mbuf = [0u8; 16];
                    let s = fmt_mem_pages(&mut mbuf, task.user_pages);
                    proc_grid.set_cell(ri as u32, 6, s);
                }

                // Build colors row
                let state_color = match task.state {
                    0 => 0xFFFFD60A,
                    1 => 0xFF30D158,
                    2 => 0xFFFF3B30,
                    3 => 0xFF8E8E93,
                    _ => 0xFF8E8E93,
                };
                colors[ri * col_count + 3] = state_color;
                if task.arch == 1 {
                    colors[ri * col_count + 4] = 0xFFFF9500;
                }
            }

            // Colors: only update when something changed (set_cell_colors checks equality)
            if colors_dirty || new_count != old_count {
                proc_grid.set_cell_colors(&colors);
            }

            // Update tracking state
            prev_tids.clear();
            prev_states.clear();
            for task in tasks.iter() {
                prev_tids.push(task.tid);
                prev_states.push(task.state);
            }
            unsafe { PREV_PROC_COUNT = new_count; }
        }

        // ── Update graphs tab ──
        if active_tab == 1 {
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

            let cw = cpu_canvas.get_stride();
            let ch = cpu_canvas.get_height();
            cpu_canvas.clear(0xFF1A1A1A);
            if cw >= 20 && ch >= 20 {
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
        }

        // ── Update disk tab (incremental) ──
        if active_tab == 2 {
            let mut total_read: u64 = 0;
            let mut total_write: u64 = 0;
            for t in tasks.iter() {
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

            // Incremental disk grid update (no Vec allocation)
            let new_disk_count = tasks.iter().filter(|t| t.io_read_bytes > 0 || t.io_write_bytes > 0).count();
            let old_disk_count = unsafe { PREV_DISK_COUNT };
            if new_disk_count != old_disk_count {
                disk_grid.set_row_count(new_disk_count as u32);
            }
            let mut ri = 0u32;
            for task in tasks.iter() {
                if task.io_read_bytes == 0 && task.io_write_bytes == 0 { continue; }
                let mut t = [0u8; 12];
                let s = fmt_u32(&mut t, task.tid);
                disk_grid.set_cell(ri, 0, s);
                disk_grid.set_cell(ri, 1, core::str::from_utf8(&task.name[..task.name_len]).unwrap_or(""));
                {
                    let mut rb = [0u8; 20];
                    let rs = fmt_bytes(&mut rb, task.io_read_bytes);
                    disk_grid.set_cell(ri, 2, rs);
                }
                {
                    let mut wb = [0u8; 20];
                    let ws = fmt_bytes(&mut wb, task.io_write_bytes);
                    disk_grid.set_cell(ri, 3, ws);
                }
                ri += 1;
            }
            unsafe { PREV_DISK_COUNT = new_disk_count; }
        }

        // ── Update system tab ──
        if active_tab == 3 {
            let hw = fetch_hwinfo();
            let mut t = [0u8; 12];

            // Processor card
            {
                let brand_len = hw.brand.iter().position(|&b| b == 0).unwrap_or(48);
                let brand_trimmed = trim_leading_spaces(&hw.brand[..brand_len]);
                if let Ok(s) = core::str::from_utf8(brand_trimmed) {
                    cpu_brand_label.set_text(s);
                }

                let vendor_len = hw.vendor.iter().position(|&b| b == 0).unwrap_or(16);
                if let Ok(s) = core::str::from_utf8(&hw.vendor[..vendor_len]) {
                    let mut buf = [0u8; 32];
                    let mut p = 0;
                    buf[p..p + 8].copy_from_slice(b"Vendor: "); p += 8;
                    let vl = s.len().min(16);
                    buf[p..p + vl].copy_from_slice(&s.as_bytes()[..vl]); p += vl;
                    if let Ok(vs) = core::str::from_utf8(&buf[..p]) {
                        cpu_vendor_label.set_text(vs);
                    }
                }

                let mut buf = [0u8; 24];
                let mut p = 0;
                buf[p..p + 8].copy_from_slice(b"Cores:  "); p += 8;
                let s = fmt_u32(&mut t, hw.cpu_count); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                if let Ok(s) = core::str::from_utf8(&buf[..p]) { cpu_cores_label.set_text(s); }

                let mut buf = [0u8; 32];
                let mut p = 0;
                buf[p..p + 8].copy_from_slice(b"Speed:  "); p += 8;
                let s = fmt_u32(&mut t, hw.tsc_mhz); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                buf[p..p + 4].copy_from_slice(b" MHz"); p += 4;
                if let Ok(s) = core::str::from_utf8(&buf[..p]) { cpu_speed_label.set_text(s); }
            }

            // Memory card
            {
                let mut buf = [0u8; 32];
                let mut p = 0;
                buf[p..p + 8].copy_from_slice(b"Total:  "); p += 8;
                let s = fmt_u32(&mut t, hw.total_mem_mib); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                buf[p..p + 4].copy_from_slice(b" MiB"); p += 4;
                if let Ok(s) = core::str::from_utf8(&buf[..p]) { mem_total_label.set_text(s); }

                let mut buf = [0u8; 32];
                let mut p = 0;
                buf[p..p + 8].copy_from_slice(b"Free:   "); p += 8;
                let s = fmt_u32(&mut t, hw.free_mem_mib); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                buf[p..p + 4].copy_from_slice(b" MiB"); p += 4;
                if let Ok(s) = core::str::from_utf8(&buf[..p]) { mem_free_label.set_text(s); }
            }

            // System card
            {
                let boot = if hw.boot_mode == 1 { "UEFI" } else { "BIOS" };
                let mut buf = [0u8; 24];
                let mut p = 0;
                buf[p..p + 6].copy_from_slice(b"Boot: "); p += 6;
                buf[p..p + boot.len()].copy_from_slice(boot.as_bytes()); p += boot.len();
                if let Ok(s) = core::str::from_utf8(&buf[..p]) { boot_label.set_text(s); }

                let ticks = sys::uptime();
                let hz = sys::tick_hz();
                let total_secs = if hz > 0 { ticks / hz } else { 0 };
                let hours = total_secs / 3600;
                let mins = (total_secs % 3600) / 60;
                let secs = total_secs % 60;
                let mut buf = [0u8; 40];
                let mut p = 0;
                buf[p..p + 8].copy_from_slice(b"Uptime: "); p += 8;
                let s = fmt_u32(&mut t, hours as u32); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                buf[p..p + 2].copy_from_slice(b"h "); p += 2;
                let s = fmt_u32(&mut t, mins as u32); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                buf[p..p + 2].copy_from_slice(b"m "); p += 2;
                let s = fmt_u32(&mut t, secs as u32); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                buf[p] = b's'; p += 1;
                if let Ok(s) = core::str::from_utf8(&buf[..p]) { uptime_sys_label.set_text(s); }
            }

            // Display card
            if hw.fb_width > 0 && hw.fb_height > 0 {
                let mut buf = [0u8; 40];
                let mut p = 0;
                let s = fmt_u32(&mut t, hw.fb_width); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                buf[p] = b'x'; p += 1;
                let s = fmt_u32(&mut t, hw.fb_height); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                buf[p..p + 2].copy_from_slice(b" ("); p += 2;
                let s = fmt_u32(&mut t, hw.fb_bpp); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
                buf[p..p + 5].copy_from_slice(b"-bit)"); p += 5;
                if let Ok(s) = core::str::from_utf8(&buf[..p]) { disp_res_label.set_text(s); }
            }
        }
    });

    ui::run();
}
