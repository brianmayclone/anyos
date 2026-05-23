#![no_std]
#![no_main]

mod data;
mod format;
mod graph;
mod icon_cache;
mod types;

use alloc::vec::Vec;

use anyos_std::i18n;
use anyos_std::process;

anyos_std::entry!(main);

use data::*;
use format::*;
use graph::*;
use icon_cache::*;
use libanyui_client as ui;
use types::*;
use ui::{ColumnDef, Widget, ALIGN_RIGHT};

const WIN_W: u32 = 1000;
const WIN_H: u32 = 700;
const SIDEBAR_W: u32 = 260;
const SIDEBAR_CONTENT_W: u32 = SIDEBAR_W - 14;
const HEADER_H: u32 = 68;
const FOOTER_H: u32 = 32;
const SIDEBAR_ITEM_H: i32 = 112;
const SIDEBAR_GAP: i32 = 10;
const INVALID_TID: u32 = u32::MAX;
const PROCESS_COLS: usize = 11;

struct AppState {
    win: ui::Window,
    sidebar_canvas: ui::Canvas,
    detail_panel: ui::View,
    detail_canvas: ui::Canvas,
    process_panel: ui::View,
    proc_grid: ui::DataGrid,
    kill_btn: ui::Button,
    focus_btn: ui::Button,
    title_label: ui::Label,
    subtitle_label: ui::Label,
    status_left: ui::Label,
    status_mid: ui::Label,
    status_right: ui::Label,

    selected: ResourceView,
    selected_tid: u32,

    thread_buf: [u8; THREAD_ENTRY_SIZE * MAX_TASKS],
    prev: PrevTicks,
    cpu: CpuState,
    cpu_history: CpuHistory,
    history: ActivityHistory,
    mem: MemInfo,
    hw: HwInfo,
    net_totals: Option<NetTotals>,
    prev_net_totals: Option<NetTotals>,

    tasks: Vec<TaskEntry>,
    display_rows: Vec<DisplayRow>,
    expanded_leaders: Vec<u32>,
    icon_cache: Vec<IconEntry>,
    grid_data: Vec<u8>,
    colors: Vec<u32>,
    indents: Vec<u16>,

    disk_read_bps: u32,
    disk_write_bps: u32,
    net_rx_bps: u32,
    net_tx_bps: u32,
    total_io_read: u64,
    total_io_write: u64,
    running_count: u32,
    ready_count: u32,
    blocked_count: u32,
}

anyos_std::global_app_state!(AppState);

fn main() {
    if !ui::init() {
        return;
    }
    i18n::init();

    let win = ui::Window::new("Activity Monitor", 80, 50, WIN_W, WIN_H);
    win.set_min_size(1000, 650);

    let sidebar = ui::View::new();
    sidebar.set_dock(ui::DOCK_LEFT);
    sidebar.set_size(SIDEBAR_W, 0);
    sidebar.set_color(SIDEBAR_BG);
    win.add(&sidebar);

    let sidebar_header = ui::View::new();
    sidebar_header.set_dock(ui::DOCK_TOP);
    sidebar_header.set_size(SIDEBAR_W, 56);
    sidebar_header.set_color(SIDEBAR_BG);
    sidebar.add(&sidebar_header);

    let sidebar_title = ui::Label::new("Resources");
    sidebar_title.set_position(22, 17);
    sidebar_title.set_size(180, 24);
    sidebar_title.set_font_size(17);
    sidebar_title.set_text_color(TEXT);
    sidebar_header.add(&sidebar_title);

    let sidebar_scroll = ui::ScrollView::new();
    sidebar_scroll.set_dock(ui::DOCK_FILL);
    sidebar_scroll.set_color(SIDEBAR_BG);
    sidebar.add(&sidebar_scroll);

    let sidebar_canvas = ui::Canvas::new(SIDEBAR_CONTENT_W, sidebar_content_height());
    sidebar_canvas.set_position(0, 0);
    sidebar_scroll.add(&sidebar_canvas);

    let content = ui::View::new();
    content.set_dock(ui::DOCK_FILL);
    content.set_color(APP_BG);
    win.add(&content);

    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(0, HEADER_H);
    header.set_color(APP_BG);
    content.add(&header);

    let title_label = ui::Label::new("");
    title_label.set_position(24, 12);
    title_label.set_size(520, 26);
    title_label.set_font_size(18);
    title_label.set_text_color(TEXT);
    header.add(&title_label);

    let subtitle_label = ui::Label::new("");
    subtitle_label.set_position(24, 38);
    subtitle_label.set_size(620, 20);
    subtitle_label.set_font_size(12);
    subtitle_label.set_text_color(TEXT_DIM);
    header.add(&subtitle_label);

    let footer = ui::View::new();
    footer.set_dock(ui::DOCK_BOTTOM);
    footer.set_size(0, FOOTER_H);
    footer.set_color(0xFF2B2B2B);
    content.add(&footer);

    let status_left = ui::Label::new("");
    status_left.set_position(18, 8);
    status_left.set_size(250, 18);
    status_left.set_font_size(11);
    status_left.set_text_color(TEXT_DIM);
    footer.add(&status_left);

    let status_mid = ui::Label::new("");
    status_mid.set_position(250, 8);
    status_mid.set_size(220, 18);
    status_mid.set_font_size(11);
    status_mid.set_text_color(TEXT_DIM);
    footer.add(&status_mid);

    let status_right = ui::Label::new("");
    status_right.set_position(500, 8);
    status_right.set_size(220, 18);
    status_right.set_font_size(11);
    status_right.set_text_color(TEXT_DIM);
    footer.add(&status_right);

    let detail_panel = ui::View::new();
    detail_panel.set_dock(ui::DOCK_FILL);
    detail_panel.set_color(APP_BG);
    content.add(&detail_panel);

    let detail_canvas = ui::Canvas::new(WIN_W - SIDEBAR_W, WIN_H - HEADER_H - FOOTER_H);
    detail_canvas.set_dock(ui::DOCK_FILL);
    detail_panel.add(&detail_canvas);

    let process_panel = ui::View::new();
    process_panel.set_dock(ui::DOCK_FILL);
    process_panel.set_color(APP_BG);
    process_panel.set_visible(false);
    content.add(&process_panel);

    let proc_toolbar = ui::View::new();
    proc_toolbar.set_dock(ui::DOCK_TOP);
    proc_toolbar.set_size(0, 42);
    proc_toolbar.set_color(0xFF2C2C2C);
    process_panel.add(&proc_toolbar);

    let kill_btn = ui::Button::new("End Process");
    kill_btn.set_position(16, 7);
    kill_btn.set_size(132, 28);
    kill_btn.set_enabled(false);
    proc_toolbar.add(&kill_btn);

    let focus_btn = ui::Button::new("Show Window");
    focus_btn.set_position(156, 7);
    focus_btn.set_size(116, 28);
    focus_btn.set_enabled(false);
    proc_toolbar.add(&focus_btn);

    let proc_hint = ui::Label::new("Sorted by CPU, groups show threads belonging to a process.");
    proc_hint.set_position(292, 11);
    proc_hint.set_size(470, 18);
    proc_hint.set_font_size(11);
    proc_hint.set_text_color(TEXT_DIM);
    proc_toolbar.add(&proc_hint);

    let proc_grid = ui::DataGrid::new(720, 480);
    proc_grid.set_dock(ui::DOCK_FILL);
    proc_grid.set_font_size(11);
    proc_grid.set_row_height(24);
    proc_grid.set_header_height(24);
    proc_grid.set_indent_column(0);
    proc_grid.set_columns(&[
        ColumnDef::new("Process").width(150),
        ColumnDef::new("TID").width(52).align(ALIGN_RIGHT).numeric(),
        ColumnDef::new("Arch").width(36).align(ALIGN_RIGHT),
        ColumnDef::new("User").width(60),
        ColumnDef::new("Status").width(62),
        ColumnDef::new("CPU").width(50).align(ALIGN_RIGHT).numeric(),
        ColumnDef::new("RAM").width(60).align(ALIGN_RIGHT).numeric(),
        ColumnDef::new("Read/s")
            .width(64)
            .align(ALIGN_RIGHT)
            .numeric(),
        ColumnDef::new("Write/s")
            .width(76)
            .align(ALIGN_RIGHT)
            .numeric(),
        ColumnDef::new("Net").width(60).align(ALIGN_RIGHT).numeric(),
        ColumnDef::new("Pri").width(36).align(ALIGN_RIGHT).numeric(),
    ]);
    process_panel.add(&proc_grid);

    let mut cpu = CpuState::new();
    fetch_cpu(&mut cpu);
    let mut cpu_history = CpuHistory::new();
    cpu_history.push(&cpu);

    let mem = fetch_memory().unwrap_or(MemInfo {
        total_frames: 0,
        free_frames: 0,
        heap_used: 0,
        heap_total: 0,
    });
    let hw = fetch_hwinfo();

    unsafe {
        APP = Some(AppState {
            win,
            sidebar_canvas,
            detail_panel,
            detail_canvas,
            process_panel,
            proc_grid,
            kill_btn,
            focus_btn,
            title_label,
            subtitle_label,
            status_left,
            status_mid,
            status_right,
            selected: ResourceView::Processes,
            selected_tid: INVALID_TID,
            thread_buf: [0u8; THREAD_ENTRY_SIZE * MAX_TASKS],
            prev: PrevTicks {
                entries: [(0, 0); MAX_TASKS],
                net_entries: [(0, 0, 0); MAX_TASKS],
                io_entries: [(0, 0, 0); MAX_TASKS],
                count: 0,
                prev_total: 0,
            },
            cpu,
            cpu_history,
            history: ActivityHistory::new(),
            mem,
            hw,
            net_totals: None,
            prev_net_totals: None,
            tasks: Vec::new(),
            display_rows: Vec::new(),
            expanded_leaders: Vec::new(),
            icon_cache: Vec::new(),
            grid_data: Vec::new(),
            colors: Vec::new(),
            indents: Vec::new(),
            disk_read_bps: 0,
            disk_write_bps: 0,
            net_rx_bps: 0,
            net_tx_bps: 0,
            total_io_read: 0,
            total_io_write: 0,
            running_count: 0,
            ready_count: 0,
            blocked_count: 0,
        });
    }

    wire_events();
    refresh();
    ui::set_timer(1000, refresh);

    ui::run();
}

fn wire_events() {
    app().win.on_close(|_| ui::quit());
    app().win.on_resize(|_| {
        let a = app();
        render_sidebar(a);
        if a.selected == ResourceView::Processes {
            update_process_grid(a);
        } else {
            render_detail(a);
        }
    });

    app().sidebar_canvas.on_mouse_down(|_x, y, _button| {
        if let Some(view) = sidebar_view_at(y) {
            select_resource(view);
        }
    });

    app().proc_grid.on_selection_changed(|ev| {
        let a = app();
        let row = ev.index;
        if row == u32::MAX {
            set_selected_task(a, INVALID_TID, false);
            return;
        }
        if let Some(dr) = a.display_rows.get(row as usize).copied() {
            let task_idx = dr.task_idx as usize;
            if task_idx < a.tasks.len() {
                let task = &a.tasks[task_idx];
                let tid = task.tid;
                let mut name = [0u8; 24];
                let name_len = task.name_len.min(name.len());
                name[..name_len].copy_from_slice(&task.name[..name_len]);

                if dr.kind == 1 {
                    if let Some(pos) = a.expanded_leaders.iter().position(|&t| t == tid) {
                        a.expanded_leaders.remove(pos);
                    } else {
                        a.expanded_leaders.push(tid);
                    }
                    update_process_grid(a);
                }
                let name = core::str::from_utf8(&name[..name_len]).unwrap_or("");
                let killable = tid > 3 && !name.starts_with("idle/") && dr.kind != 1;
                set_selected_task(a, tid, killable);
            }
        }
    });

    app().kill_btn.on_click(|_| {
        let a = app();
        if a.selected_tid != INVALID_TID {
            let tid = a.selected_tid;
            if process::kill(tid) == 0 {
                ui::show_notification("Activity Monitor", "Process ended", None, 2500);
            }
            set_selected_task(a, INVALID_TID, false);
            refresh();
        }
    });

    app().focus_btn.on_click(|_| {
        let tid = app().selected_tid;
        if tid != INVALID_TID {
            ui::focus_by_tid(tid);
        }
    });

    let mut mb = ui::MenuBarBuilder::new()
        .menu("File")
        .item(1, "Quit", 0)
        .end_menu()
        .menu("View")
        .item(10, "Overview", 0)
        .item(11, "Processes", 0)
        .item(12, "Processor", 0)
        .item(13, "Memory", 0)
        .item(14, "Storage", 0)
        .item(15, "Network", 0)
        .item(16, "System", 0)
        .end_menu();
    let menu_data = mb.build();
    let menu = ui::MenuBar::set(app().win.id(), menu_data);
    menu.on_item(|e| match e.item_id {
        1 => ui::quit(),
        10 => select_resource(ResourceView::Overview),
        11 => select_resource(ResourceView::Processes),
        12 => select_resource(ResourceView::Cpu),
        13 => select_resource(ResourceView::Memory),
        14 => select_resource(ResourceView::Disk),
        15 => select_resource(ResourceView::Network),
        16 => select_resource(ResourceView::System),
        _ => {}
    });
}

fn refresh() {
    let a = app();

    fetch_cpu(&mut a.cpu);
    a.cpu_history.push(&a.cpu);
    fetch_tasks(
        &mut a.thread_buf,
        &mut a.prev,
        a.cpu.total_sched_ticks,
        &mut a.tasks,
    );
    sort_tasks_by_activity(&mut a.tasks);
    a.hw = fetch_hwinfo();
    if let Some(mem) = fetch_memory() {
        a.mem = mem;
    }

    update_rates_and_counts(a);
    push_histories(a);
    validate_selected_task(a);
    update_header(a);
    update_status(a);
    render_sidebar(a);
    if a.selected == ResourceView::Processes {
        update_process_grid(a);
    } else {
        render_detail(a);
    }
}

fn update_rates_and_counts(a: &mut AppState) {
    a.running_count = 0;
    a.ready_count = 0;
    a.blocked_count = 0;
    a.disk_read_bps = 0;
    a.disk_write_bps = 0;
    a.total_io_read = 0;
    a.total_io_write = 0;

    let mut task_net_rx = 0u32;
    let mut task_net_tx = 0u32;

    for task in a.tasks.iter() {
        match task.state {
            0 => a.ready_count += 1,
            1 => a.running_count += 1,
            2 => a.blocked_count += 1,
            _ => {}
        }
        a.disk_read_bps = a.disk_read_bps.saturating_add(task.io_read_bps);
        a.disk_write_bps = a.disk_write_bps.saturating_add(task.io_write_bps);
        a.total_io_read = a.total_io_read.saturating_add(task.io_read_bytes);
        a.total_io_write = a.total_io_write.saturating_add(task.io_write_bytes);
        task_net_rx = task_net_rx.saturating_add(task.net_rx_bps);
        task_net_tx = task_net_tx.saturating_add(task.net_tx_bps);
    }

    if let Some(net) = fetch_net_totals() {
        if let Some(prev) = a.prev_net_totals {
            let global_rx = net
                .rx_bytes
                .wrapping_sub(prev.rx_bytes)
                .min(u32::MAX as u64) as u32;
            let global_tx = net
                .tx_bytes
                .wrapping_sub(prev.tx_bytes)
                .min(u32::MAX as u64) as u32;
            if global_rx == 0 && global_tx == 0 && (task_net_rx > 0 || task_net_tx > 0) {
                a.net_rx_bps = task_net_rx;
                a.net_tx_bps = task_net_tx;
            } else {
                a.net_rx_bps = global_rx;
                a.net_tx_bps = global_tx;
            }
        } else {
            a.net_rx_bps = task_net_rx;
            a.net_tx_bps = task_net_tx;
        }
        a.prev_net_totals = Some(net);
        a.net_totals = Some(net);
    } else {
        a.net_rx_bps = task_net_rx;
        a.net_tx_bps = task_net_tx;
        a.net_totals = None;
    }
}

fn push_histories(a: &mut AppState) {
    let total_kb = a.mem.total_frames.saturating_mul(4);
    let free_kb = a.mem.free_frames.saturating_mul(4);
    let used_kb = total_kb.saturating_sub(free_kb);
    let mem_pct = percent_u64(used_kb as u64, total_kb as u64);
    let heap_pct = percent_u64(a.mem.heap_used as u64, a.mem.heap_total as u64);

    a.history.cpu.push(a.cpu.overall_pct.min(100));
    a.history.cpu_freq.push(a.cpu.avg_freq_mhz);
    a.history.memory.push(mem_pct);
    a.history.heap.push(heap_pct);
    a.history.disk_read.push_burst_smoothed(a.disk_read_bps);
    a.history.disk_write.push_burst_smoothed(a.disk_write_bps);
    a.history.net_rx.push_burst_smoothed(a.net_rx_bps);
    a.history.net_tx.push_burst_smoothed(a.net_tx_bps);
    a.history.process_count.push(a.tasks.len() as u32);
}

fn update_header(a: &mut AppState) {
    a.title_label.set_text(a.selected.title());
    a.subtitle_label.set_text(a.selected.subtitle());
    a.title_label.set_text_color(a.selected.accent());
    a.detail_panel
        .set_visible(a.selected != ResourceView::Processes);
    a.process_panel
        .set_visible(a.selected == ResourceView::Processes);
}

fn update_status(a: &mut AppState) {
    let mut left = [0u8; 80];
    let mut p = 0usize;
    let mut n = [0u8; 12];
    let s = fmt_u32(&mut n, a.tasks.len() as u32);
    left[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    let processes = b" Processes ";
    left[p..p + processes.len()].copy_from_slice(processes);
    p += processes.len();
    let s = fmt_u32(&mut n, a.running_count);
    left[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    let active = b" active";
    left[p..p + active.len()].copy_from_slice(active);
    p += active.len();
    if let Ok(text) = core::str::from_utf8(&left[..p]) {
        a.status_left.set_text(text);
    }

    let mut mid = [0u8; 80];
    let mut p = 0usize;
    mid[p..p + 5].copy_from_slice(b"CPU: ");
    p += 5;
    let s = fmt_u32(&mut n, a.cpu.overall_pct);
    mid[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    mid[p] = b'%';
    p += 1;
    if a.cpu.avg_freq_mhz > 0 {
        mid[p..p + 2].copy_from_slice(b"  ");
        p += 2;
        let s = fmt_u32(&mut n, a.cpu.avg_freq_mhz);
        mid[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        mid[p..p + 4].copy_from_slice(b" MHz");
        p += 4;
    }
    if let Ok(text) = core::str::from_utf8(&mid[..p]) {
        a.status_mid.set_text(text);
    }

    let total_mb = a.mem.total_frames.saturating_mul(4) / 1024;
    let used_mb = a
        .mem
        .total_frames
        .saturating_sub(a.mem.free_frames)
        .saturating_mul(4)
        / 1024;
    let mut right = [0u8; 80];
    let mut p = 0usize;
    right[p..p + 5].copy_from_slice(b"RAM: ");
    p += 5;
    let s = fmt_u32(&mut n, used_mb);
    right[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    right[p] = b'/';
    p += 1;
    let s = fmt_u32(&mut n, total_mb);
    right[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    right[p..p + 4].copy_from_slice(b" MiB");
    p += 4;
    if let Ok(text) = core::str::from_utf8(&right[..p]) {
        a.status_right.set_text(text);
    }
}

fn render_sidebar(a: &mut AppState) {
    let w = a.sidebar_canvas.get_stride();
    let h = a.sidebar_canvas.get_height();
    a.sidebar_canvas.clear(SIDEBAR_BG);
    if w == 0 || h == 0 {
        return;
    }

    for idx in 0..ResourceView::COUNT {
        let view = ResourceView::from_index(idx);
        let y = 8 + idx as i32 * (SIDEBAR_ITEM_H + SIDEBAR_GAP);
        let mut value_buf = [0u8; 48];
        let (subtitle, value, hist) = sidebar_data(a, view, &mut value_buf);
        draw_sidebar_item(
            &a.sidebar_canvas,
            y,
            w,
            view,
            a.selected == view,
            subtitle,
            value,
            hist,
        );
    }
}

fn sidebar_content_height() -> u32 {
    (ResourceView::COUNT as i32 * (SIDEBAR_ITEM_H + SIDEBAR_GAP) + 8) as u32
}

fn sidebar_data<'a>(
    a: &'a AppState,
    view: ResourceView,
    buf: &'a mut [u8],
) -> (&'static str, &'a str, &'a MetricHistory) {
    match view {
        ResourceView::Overview => ("At a glance", "Live", &a.history.cpu),
        ResourceView::Processes => {
            let s = fmt_count_label(buf, a.tasks.len() as u32, "processes");
            ("Applications and threads", s, &a.history.process_count)
        }
        ResourceView::Cpu => {
            let s = fmt_percent_label(buf, a.cpu.overall_pct);
            ("Total utilization", s, &a.history.cpu)
        }
        ResourceView::Memory => {
            let total_mb = a.mem.total_frames.saturating_mul(4) / 1024;
            let used_mb = a
                .mem
                .total_frames
                .saturating_sub(a.mem.free_frames)
                .saturating_mul(4)
                / 1024;
            let s = fmt_used_total(buf, used_mb, total_mb, "MiB");
            ("RAM used", s, &a.history.memory)
        }
        ResourceView::Disk => {
            let s = fmt_two_rates(buf, "R", a.disk_read_bps, "W", a.disk_write_bps);
            ("File I/O", s, &a.history.disk_read)
        }
        ResourceView::Network => {
            let s = fmt_two_rates(buf, "In", a.net_rx_bps, "Out", a.net_tx_bps);
            ("Throughput", s, &a.history.net_rx)
        }
        ResourceView::System => {
            let s = fmt_count_label(buf, a.hw.cpu_count, "cores");
            ("Hardware", s, &a.history.cpu_freq)
        }
    }
}

fn render_detail(a: &mut AppState) {
    let w = a.detail_canvas.get_stride();
    let h = a.detail_canvas.get_height();
    a.detail_canvas.clear(APP_BG);
    if w < 240 || h < 160 {
        return;
    }

    match a.selected {
        ResourceView::Overview => render_overview(a, w, h),
        ResourceView::Cpu => render_cpu(a, w, h),
        ResourceView::Memory => render_memory(a, w, h),
        ResourceView::Disk => render_disk(a, w, h),
        ResourceView::Network => render_network(a, w, h),
        ResourceView::System => render_system(a, w, h),
        ResourceView::Processes => {}
    }
}

fn render_overview(a: &mut AppState, w: u32, h: u32) {
    let pad = 22i32;
    let gap = 16i32;
    let col_w = ((w as i32 - pad * 2 - gap) / 2).max(220) as u32;
    let row_h = ((h as i32 - pad * 2 - gap) / 2).max(160) as u32;
    let x2 = pad + col_w as i32 + gap;
    let y2 = pad + row_h as i32 + gap;

    let mut v1 = [0u8; 32];
    draw_history_chart(
        &a.detail_canvas,
        pad,
        pad,
        col_w,
        row_h,
        "CPU Usage",
        fmt_percent_label(&mut v1, a.cpu.overall_pct),
        &a.history.cpu,
        100,
        CPU_COLOR,
        0x332657A8,
    );

    let total_mb = a.mem.total_frames.saturating_mul(4) / 1024;
    let used_mb = a
        .mem
        .total_frames
        .saturating_sub(a.mem.free_frames)
        .saturating_mul(4)
        / 1024;
    let mut v2 = [0u8; 40];
    draw_history_chart(
        &a.detail_canvas,
        x2,
        pad,
        col_w,
        row_h,
        "Memory",
        fmt_used_total(&mut v2, used_mb, total_mb, "MiB"),
        &a.history.memory,
        100,
        MEM_COLOR,
        0x332D0F24,
    );

    let mut r = [0u8; 32];
    let mut wr = [0u8; 32];
    draw_dual_history_chart(
        &a.detail_canvas,
        pad,
        y2,
        col_w,
        row_h,
        "Storage",
        fmt_rate_prefixed(&mut r, "Read", a.disk_read_bps),
        fmt_rate_prefixed(&mut wr, "Write", a.disk_write_bps),
        &a.history.disk_read,
        &a.history.disk_write,
        DISK_COLOR,
        0xFFFFB74D,
    );

    let mut rx = [0u8; 32];
    let mut tx = [0u8; 32];
    draw_dual_history_chart(
        &a.detail_canvas,
        x2,
        y2,
        col_w,
        row_h,
        "Network",
        fmt_rate_prefixed(&mut rx, "Receive", a.net_rx_bps),
        fmt_rate_prefixed(&mut tx, "Send", a.net_tx_bps),
        &a.history.net_rx,
        &a.history.net_tx,
        NET_COLOR,
        0xFF77DDE7,
    );
}

fn render_cpu(a: &mut AppState, w: u32, h: u32) {
    let pad = 22i32;
    let mut val = [0u8; 48];
    draw_history_chart(
        &a.detail_canvas,
        pad,
        pad,
        w.saturating_sub((pad * 2) as u32),
        230,
        "Usage",
        fmt_cpu_value(&mut val, a.cpu.overall_pct, a.cpu.avg_freq_mhz),
        &a.history.cpu,
        100,
        CPU_COLOR,
        0x332657A8,
    );

    let grid_y = pad + 246;
    let grid_h = h.saturating_sub(grid_y as u32 + 22).max(180);
    draw_cpu_core_grid(
        &a.detail_canvas,
        pad,
        grid_y,
        w.saturating_sub((pad * 2) as u32),
        grid_h,
        &a.cpu,
        &a.cpu_history,
    );
}

fn render_memory(a: &mut AppState, w: u32, _h: u32) {
    let pad = 22i32;
    let total_mb = a.mem.total_frames.saturating_mul(4) / 1024;
    let free_mb = a.mem.free_frames.saturating_mul(4) / 1024;
    let used_mb = total_mb.saturating_sub(free_mb);
    let mem_pct = percent_u64(used_mb as u64, total_mb as u64);
    let heap_pct = percent_u64(a.mem.heap_used as u64, a.mem.heap_total as u64);

    let mut val = [0u8; 48];
    draw_history_chart(
        &a.detail_canvas,
        pad,
        pad,
        w.saturating_sub((pad * 2) as u32),
        240,
        "RAM Usage",
        fmt_used_total_pct(&mut val, used_mb, total_mb, "MiB", mem_pct),
        &a.history.memory,
        100,
        MEM_COLOR,
        0x332D0F24,
    );

    let chart_y = pad + 256;
    let mut heap_val = [0u8; 48];
    draw_history_chart(
        &a.detail_canvas,
        pad,
        chart_y,
        w.saturating_sub((pad * 2) as u32),
        170,
        "Kernel Heap",
        fmt_used_total_pct(
            &mut heap_val,
            a.mem.heap_used / 1024,
            a.mem.heap_total / 1024,
            "KiB",
            heap_pct,
        ),
        &a.history.heap,
        100,
        0xFFAB47BC,
        0x332D0F24,
    );

    let card_y = chart_y + 186;
    let card_w = (w.saturating_sub((pad * 2) as u32 + 24)) / 3;
    draw_metric_card(
        &a.detail_canvas,
        pad,
        card_y,
        card_w,
        "Used",
        used_mb,
        "MiB",
    );
    draw_metric_card(
        &a.detail_canvas,
        pad + card_w as i32 + 12,
        card_y,
        card_w,
        "Free",
        free_mb,
        "MiB",
    );
    draw_metric_card(
        &a.detail_canvas,
        pad + (card_w as i32 + 12) * 2,
        card_y,
        card_w,
        "Total",
        total_mb,
        "MiB",
    );
}

fn render_disk(a: &mut AppState, w: u32, h: u32) {
    let pad = 22i32;
    let mut read = [0u8; 32];
    let mut write = [0u8; 32];
    draw_dual_history_chart(
        &a.detail_canvas,
        pad,
        pad,
        w.saturating_sub((pad * 2) as u32),
        260,
        "I/O Activity",
        fmt_rate_prefixed(&mut read, "Read", a.disk_read_bps),
        fmt_rate_prefixed(&mut write, "Write", a.disk_write_bps),
        &a.history.disk_read,
        &a.history.disk_write,
        DISK_COLOR,
        0xFFFFB74D,
    );

    let y = pad + 276;
    fill_card(
        &a.detail_canvas,
        pad,
        y,
        w.saturating_sub((pad * 2) as u32),
        h.saturating_sub(y as u32 + 22),
    );
    draw_text(
        &a.detail_canvas,
        pad + 14,
        y + 12,
        TEXT,
        14,
        "Top Processes",
    );
    draw_top_io_processes(a, pad + 14, y + 42, w.saturating_sub((pad * 2 + 28) as u32));
}

fn render_network(a: &mut AppState, w: u32, h: u32) {
    let pad = 22i32;
    let mut rx = [0u8; 32];
    let mut tx = [0u8; 32];
    draw_dual_history_chart(
        &a.detail_canvas,
        pad,
        pad,
        w.saturating_sub((pad * 2) as u32),
        260,
        "Throughput",
        fmt_rate_prefixed(&mut rx, "Receive", a.net_rx_bps),
        fmt_rate_prefixed(&mut tx, "Send", a.net_tx_bps),
        &a.history.net_rx,
        &a.history.net_tx,
        NET_COLOR,
        0xFF77DDE7,
    );

    let card_y = pad + 276;
    let card_w = (w.saturating_sub((pad * 2) as u32 + 36)) / 4;
    if let Some(net) = a.net_totals {
        draw_metric_card_u64(
            &a.detail_canvas,
            pad,
            card_y,
            card_w,
            "RX Packets",
            net.rx_packets,
        );
        draw_metric_card_u64(
            &a.detail_canvas,
            pad + card_w as i32 + 12,
            card_y,
            card_w,
            "TX Packets",
            net.tx_packets,
        );
        draw_metric_card_u64(
            &a.detail_canvas,
            pad + (card_w as i32 + 12) * 2,
            card_y,
            card_w,
            "Errors",
            net.rx_errors + net.tx_errors + net.tcp_conn_errors as u64,
        );
        draw_metric_card(
            &a.detail_canvas,
            pad + (card_w as i32 + 12) * 3,
            card_y,
            card_w,
            "TCP Open",
            net.tcp_curr_established,
            "",
        );
    } else {
        draw_text(
            &a.detail_canvas,
            pad,
            card_y,
            TEXT_DIM,
            13,
            "No global network statistics available.",
        );
    }

    let y = card_y + 98;
    fill_card(
        &a.detail_canvas,
        pad,
        y,
        w.saturating_sub((pad * 2) as u32),
        h.saturating_sub(y as u32 + 22),
    );
    draw_text(
        &a.detail_canvas,
        pad + 14,
        y + 12,
        TEXT,
        14,
        "Network by Process",
    );
    draw_top_net_processes(a, pad + 14, y + 42, w.saturating_sub((pad * 2 + 28) as u32));
}

fn render_system(a: &mut AppState, w: u32, h: u32) {
    let pad = 22i32;
    let mut speed = [0u8; 48];
    draw_history_chart(
        &a.detail_canvas,
        pad,
        pad,
        w.saturating_sub((pad * 2) as u32),
        210,
        "CPU Clock",
        fmt_mhz_value(&mut speed, a.cpu.avg_freq_mhz, a.cpu.max_freq_mhz),
        &a.history.cpu_freq,
        a.cpu.max_freq_mhz.max(a.history.cpu_freq.max()).max(1),
        SYS_COLOR,
        0x333B5A22,
    );

    let y = pad + 226;
    fill_card(
        &a.detail_canvas,
        pad,
        y,
        w.saturating_sub((pad * 2) as u32),
        h.saturating_sub(y as u32 + 22),
    );
    draw_text(&a.detail_canvas, pad + 14, y + 12, TEXT, 14, "Properties");

    let brand_len = a.hw.brand.iter().position(|&b| b == 0).unwrap_or(48);
    let brand = trim_leading_spaces(&a.hw.brand[..brand_len]);
    let brand_text = core::str::from_utf8(brand).unwrap_or("Unknown processor");
    draw_text(&a.detail_canvas, pad + 14, y + 44, TEXT, 13, brand_text);

    let vendor_len = a.hw.vendor.iter().position(|&b| b == 0).unwrap_or(16);
    let vendor = trim_leading_spaces(&a.hw.vendor[..vendor_len]);
    let vendor_text = core::str::from_utf8(vendor).unwrap_or("Unknown vendor");
    draw_system_line(&a.detail_canvas, pad + 14, y + 72, "Vendor", vendor_text);

    let left_x = pad + 14;
    let right_x = pad + 360;
    let mut line_y = y + 106;
    let boot = if a.hw.boot_mode == 1 { "UEFI" } else { "BIOS" };
    draw_system_line(&a.detail_canvas, left_x, line_y, "Boot Mode", boot);

    let mut value = [0u8; 48];
    draw_system_line(
        &a.detail_canvas,
        right_x,
        line_y,
        "Display",
        fmt_resolution(&mut value, a.hw.fb_width, a.hw.fb_height, a.hw.fb_bpp),
    );

    line_y += 28;
    let mut cpu_count = [0u8; 24];
    draw_system_line(
        &a.detail_canvas,
        left_x,
        line_y,
        "Cores",
        fmt_value_unit(&mut cpu_count, a.hw.cpu_count, ""),
    );
    let mut tsc = [0u8; 24];
    draw_system_line(
        &a.detail_canvas,
        right_x,
        line_y,
        "TSC",
        fmt_value_unit(&mut tsc, a.hw.tsc_mhz, "MHz"),
    );

    line_y += 28;
    let mut avg = [0u8; 24];
    draw_system_line(
        &a.detail_canvas,
        left_x,
        line_y,
        "CPU Clock",
        fmt_value_unit(&mut avg, a.hw.cpu_freq_mhz, "MHz"),
    );
    let mut max = [0u8; 24];
    draw_system_line(
        &a.detail_canvas,
        right_x,
        line_y,
        "Max Clock",
        fmt_value_unit(&mut max, a.hw.max_freq_mhz, "MHz"),
    );

    line_y += 28;
    let mut total_freq = [0u8; 24];
    draw_system_line(
        &a.detail_canvas,
        left_x,
        line_y,
        "Total Clock",
        fmt_value_unit(&mut total_freq, a.hw.total_cpu_freq_mhz, "MHz"),
    );
    let mut fastest_core = 0u32;
    for freq in a.hw.core_freq_mhz.iter() {
        fastest_core = fastest_core.max(*freq);
    }
    let mut fastest = [0u8; 24];
    draw_system_line(
        &a.detail_canvas,
        right_x,
        line_y,
        "Fastest Core",
        fmt_value_unit(&mut fastest, fastest_core, "MHz"),
    );

    line_y += 28;
    let mut mem_total = [0u8; 24];
    draw_system_line(
        &a.detail_canvas,
        left_x,
        line_y,
        "RAM Total",
        fmt_value_unit(&mut mem_total, a.hw.total_mem_mib, "MiB"),
    );
    let mut mem_free = [0u8; 24];
    draw_system_line(
        &a.detail_canvas,
        right_x,
        line_y,
        "RAM Free",
        fmt_value_unit(&mut mem_free, a.hw.free_mem_mib, "MiB"),
    );

    line_y += 28;
    draw_system_line(
        &a.detail_canvas,
        left_x,
        line_y,
        "Power Profile",
        power_profile_name(a.hw.power_profile),
    );
    draw_system_line(
        &a.detail_canvas,
        right_x,
        line_y,
        "Power Driver",
        power_driver_name(a.hw.power_driver),
    );

    line_y += 28;
    let mut features = [0u8; 64];
    draw_system_line(
        &a.detail_canvas,
        left_x,
        line_y,
        "Features",
        fmt_power_features(&mut features, a.hw.power_features),
    );
}

fn update_process_grid(a: &mut AppState) {
    build_display_list(&a.tasks, &a.expanded_leaders, &mut a.display_rows);
    let row_count = a.display_rows.len();
    a.proc_grid.set_row_count(row_count as u32);

    a.grid_data.clear();
    a.colors.clear();
    a.indents.clear();
    a.colors.resize(row_count * PROCESS_COLS, 0);

    for (ri, dr) in a.display_rows.iter().enumerate() {
        if ri > 0 {
            a.grid_data.push(0x1E);
        }
        let task = &a.tasks[dr.task_idx as usize];
        push_process_name(&mut a.grid_data, task, *dr, &a.expanded_leaders);
        push_sep(&mut a.grid_data);
        push_u32_cell(&mut a.grid_data, task.tid);
        push_sep(&mut a.grid_data);
        push_str(&mut a.grid_data, arch_name(task.arch));
        push_sep(&mut a.grid_data);
        push_user_cell(&mut a.grid_data, task.uid);
        push_sep(&mut a.grid_data);
        push_str(
            &mut a.grid_data,
            state_name(if dr.kind == 1 {
                dr.agg_state
            } else {
                task.state
            }),
        );
        push_sep(&mut a.grid_data);
        let mut pct = [0u8; 12];
        let cpu = if dr.kind == 1 {
            dr.agg_cpu
        } else {
            task.cpu_pct_x10
        };
        push_str(&mut a.grid_data, fmt_pct(&mut pct, cpu));
        push_sep(&mut a.grid_data);
        let mut mem = [0u8; 16];
        let mem_pages = if dr.kind == 1 {
            dr.agg_pages
        } else {
            task.user_pages
        };
        push_str(&mut a.grid_data, fmt_mem_pages(&mut mem, mem_pages));
        push_sep(&mut a.grid_data);
        let read_bps = if dr.kind == 1 {
            dr.agg_read_bps
        } else {
            task.io_read_bps
        };
        let mut read = [0u8; 32];
        push_str(&mut a.grid_data, fmt_rate(&mut read, read_bps));
        push_sep(&mut a.grid_data);
        let write_bps = if dr.kind == 1 {
            dr.agg_write_bps
        } else {
            task.io_write_bps
        };
        let mut write = [0u8; 32];
        push_str(&mut a.grid_data, fmt_rate(&mut write, write_bps));
        push_sep(&mut a.grid_data);
        let mut net = [0u8; 32];
        let net_bps = (if dr.kind == 1 {
            dr.agg_net
        } else {
            task.net_kbit
        })
        .saturating_mul(125);
        push_str(&mut a.grid_data, fmt_rate(&mut net, net_bps));
        push_sep(&mut a.grid_data);
        push_u32_cell(&mut a.grid_data, task.priority as u32);

        let state_color = match if dr.kind == 1 {
            dr.agg_state
        } else {
            task.state
        } {
            0 => 0xFFFFD166,
            1 => 0xFF7BD88F,
            2 => 0xFFFF6B6B,
            3 => TEXT_MUTED,
            _ => TEXT_MUTED,
        };
        a.colors[ri * PROCESS_COLS + 4] = state_color;
        a.colors[ri * PROCESS_COLS + 5] = if cpu > 500 { 0xFFFFD166 } else { CPU_COLOR };
        a.colors[ri * PROCESS_COLS + 7] = DISK_COLOR;
        a.colors[ri * PROCESS_COLS + 8] = DISK_COLOR;
        a.colors[ri * PROCESS_COLS + 9] = NET_COLOR;
        if dr.kind == 2 {
            a.colors[ri * PROCESS_COLS] = TEXT_DIM;
        }
        a.indents.push(if dr.kind == 2 { 24 } else { 0 });
    }

    a.proc_grid.set_data_raw(&a.grid_data);
    a.proc_grid.set_cell_colors(&a.colors);
    a.proc_grid.set_row_indents(&a.indents);

    for (ri, dr) in a.display_rows.iter().enumerate() {
        let task = &a.tasks[dr.task_idx as usize];
        if let Ok(name) = core::str::from_utf8(&task.name[..task.name_len]) {
            ensure_icon_cached(&mut a.icon_cache, name);
            if let Some(pixels) = find_icon(&a.icon_cache, name) {
                a.proc_grid
                    .set_cell_icon(ri as u32, 0, pixels, ICON_SIZE, ICON_SIZE);
            }
        }
    }
}

fn draw_top_io_processes(a: &AppState, x: i32, y: i32, _w: u32) {
    let mut shown = 0;
    for task in a.tasks.iter() {
        let total = task.io_read_bps.saturating_add(task.io_write_bps);
        if total == 0 {
            continue;
        }
        draw_process_rate_line(
            &a.detail_canvas,
            x,
            y + shown * 24,
            task,
            task.io_read_bps,
            task.io_write_bps,
            "R",
            "W",
        );
        shown += 1;
        if shown >= 7 {
            break;
        }
    }
    if shown == 0 {
        draw_text(
            &a.detail_canvas,
            x,
            y,
            TEXT_DIM,
            12,
            "No measurable I/O activity right now.",
        );
    }
}

fn draw_top_net_processes(a: &AppState, x: i32, y: i32, _w: u32) {
    let mut shown = 0;
    for task in a.tasks.iter() {
        let total = task.net_rx_bps.saturating_add(task.net_tx_bps);
        if total == 0 {
            continue;
        }
        draw_process_rate_line(
            &a.detail_canvas,
            x,
            y + shown * 24,
            task,
            task.net_rx_bps,
            task.net_tx_bps,
            "In",
            "Out",
        );
        shown += 1;
        if shown >= 7 {
            break;
        }
    }
    if shown == 0 {
        draw_text(
            &a.detail_canvas,
            x,
            y,
            TEXT_DIM,
            12,
            "No measurable network activity right now.",
        );
    }
}

fn draw_process_rate_line(
    cv: &ui::Canvas,
    x: i32,
    y: i32,
    task: &TaskEntry,
    a_rate: u32,
    b_rate: u32,
    a_label: &str,
    b_label: &str,
) {
    let name = core::str::from_utf8(&task.name[..task.name_len]).unwrap_or("?");
    draw_text(cv, x, y, TEXT, 12, name);
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    draw_text(
        cv,
        x + 220,
        y,
        TEXT_DIM,
        12,
        fmt_rate_prefixed(&mut a, a_label, a_rate),
    );
    draw_text(
        cv,
        x + 390,
        y,
        TEXT_DIM,
        12,
        fmt_rate_prefixed(&mut b, b_label, b_rate),
    );
}

fn draw_metric_card(cv: &ui::Canvas, x: i32, y: i32, w: u32, label: &str, value: u32, unit: &str) {
    fill_card(cv, x, y, w, 82);
    let mut buf = [0u8; 32];
    let text = fmt_value_unit(&mut buf, value, unit);
    draw_kv(cv, x + 14, y + 14, label, text);
}

fn draw_metric_card_u64(cv: &ui::Canvas, x: i32, y: i32, w: u32, label: &str, value: u64) {
    fill_card(cv, x, y, w, 82);
    let mut buf = [0u8; 32];
    let text = fmt_u64_short(&mut buf, value);
    draw_kv(cv, x + 14, y + 14, label, text);
}

fn draw_system_line(cv: &ui::Canvas, x: i32, y: i32, label: &str, value: &str) {
    draw_text(cv, x, y, TEXT_MUTED, 11, label);
    draw_text(cv, x + 116, y, TEXT, 12, value);
}

fn select_resource(view: ResourceView) {
    let a = app();
    a.selected = view;
    update_header(a);
    render_sidebar(a);
    if view == ResourceView::Processes {
        update_process_grid(a);
    } else {
        render_detail(a);
    }
}

fn sidebar_view_at(y: i32) -> Option<ResourceView> {
    if y < 8 {
        return None;
    }
    let rel = y - 8;
    let stride = SIDEBAR_ITEM_H + SIDEBAR_GAP;
    let idx = rel / stride;
    if idx < 0 || idx as usize >= ResourceView::COUNT {
        return None;
    }
    if rel % stride >= SIDEBAR_ITEM_H {
        return None;
    }
    Some(ResourceView::from_index(idx as usize))
}

fn set_selected_task(a: &mut AppState, tid: u32, killable: bool) {
    a.selected_tid = tid;
    a.kill_btn.set_enabled(killable);
    a.focus_btn.set_enabled(tid != INVALID_TID);
}

fn validate_selected_task(a: &mut AppState) {
    if a.selected_tid == INVALID_TID {
        return;
    }
    if !a.tasks.iter().any(|t| t.tid == a.selected_tid) {
        set_selected_task(a, INVALID_TID, false);
    }
}

fn percent_u64(used: u64, total: u64) -> u32 {
    if total == 0 {
        0
    } else {
        ((used * 100) / total).min(100) as u32
    }
}

fn state_name(state: u8) -> &'static str {
    match state {
        0 => "Ready",
        1 => "Active",
        2 => "Blocked",
        3 => "Exited",
        4 => "Stopped",
        _ => "Unknown",
    }
}

fn arch_name(arch: u8) -> &'static str {
    match arch {
        1 => "32",
        2 => "ARM",
        _ => "64",
    }
}

fn power_profile_name(profile: u32) -> &'static str {
    match profile {
        0 => "Power Saver",
        2 => "Performance",
        _ => "Balanced",
    }
}

fn power_driver_name(driver: u32) -> &'static str {
    match driver {
        1 => "Intel HWP",
        2 => "Intel Legacy",
        3 => "AMD P-State",
        4 => "KVM Host",
        _ => "None",
    }
}

fn fmt_power_features<'a>(buf: &'a mut [u8], bits: u32) -> &'a str {
    let mut p = 0usize;
    p = push_feature(buf, p, bits & 1 != 0, "HWP");
    p = push_feature(buf, p, bits & 2 != 0, "Turbo");
    p = push_feature(buf, p, bits & 4 != 0, "APERF");
    p = push_feature(buf, p, bits & 8 != 0, "Hypervisor");
    p = push_feature(buf, p, bits & 16 != 0, "Control");
    if p == 0 {
        buf[..4].copy_from_slice(b"None");
        p = 4;
    }
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn push_feature(buf: &mut [u8], mut p: usize, enabled: bool, label: &str) -> usize {
    if !enabled {
        return p;
    }
    if p > 0 {
        buf[p..p + 2].copy_from_slice(b", ");
        p += 2;
    }
    let b = label.as_bytes();
    buf[p..p + b.len()].copy_from_slice(b);
    p + b.len()
}

fn push_sep(buf: &mut Vec<u8>) {
    buf.push(0x1F);
}

fn push_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
}

fn push_u32_cell(buf: &mut Vec<u8>, value: u32) {
    let mut tmp = [0u8; 12];
    let s = fmt_u32(&mut tmp, value);
    push_str(buf, s);
}

fn push_user_cell(buf: &mut Vec<u8>, uid: u16) {
    let mut ubuf = [0u8; 16];
    let n = process::getusername(uid, &mut ubuf);
    if n != u32::MAX && n > 0 {
        buf.extend_from_slice(&ubuf[..n as usize]);
    } else {
        push_str(buf, "?");
    }
}

fn push_process_name(buf: &mut Vec<u8>, task: &TaskEntry, dr: DisplayRow, expanded: &[u32]) {
    match dr.kind {
        1 => {
            if expanded.iter().any(|&tid| tid == task.tid) {
                push_str(buf, "- ");
            } else {
                push_str(buf, "+ ");
            }
            buf.extend_from_slice(&task.name[..task.name_len]);
            push_str(buf, " (");
            push_u32_cell(buf, dr.thread_count as u32);
            push_str(buf, ")");
        }
        _ => buf.extend_from_slice(&task.name[..task.name_len]),
    }
}

fn fmt_percent_label<'a>(buf: &'a mut [u8], pct: u32) -> &'a str {
    let mut num = [0u8; 12];
    let s = fmt_u32(&mut num, pct.min(100));
    let mut p = 0usize;
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p] = b'%';
    p += 1;
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_count_label<'a>(buf: &'a mut [u8], count: u32, label: &str) -> &'a str {
    let mut num = [0u8; 12];
    let s = fmt_u32(&mut num, count);
    let mut p = 0usize;
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p] = b' ';
    p += 1;
    let l = label.as_bytes();
    buf[p..p + l.len()].copy_from_slice(l);
    p += l.len();
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_used_total<'a>(buf: &'a mut [u8], used: u32, total: u32, unit: &str) -> &'a str {
    let mut num = [0u8; 12];
    let mut p = 0usize;
    let s = fmt_u32(&mut num, used);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p] = b'/';
    p += 1;
    let s = fmt_u32(&mut num, total);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p] = b' ';
    p += 1;
    let u = unit.as_bytes();
    buf[p..p + u.len()].copy_from_slice(u);
    p += u.len();
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_used_total_pct<'a>(
    buf: &'a mut [u8],
    used: u32,
    total: u32,
    unit: &str,
    pct: u32,
) -> &'a str {
    let mut p = 0usize;
    let mut tmp = [0u8; 32];
    let s = fmt_used_total(&mut tmp, used, total, unit);
    let b = s.as_bytes();
    buf[p..p + b.len()].copy_from_slice(b);
    p += b.len();
    buf[p..p + 3].copy_from_slice(b" - ");
    p += 3;
    let mut num = [0u8; 12];
    let ps = fmt_u32(&mut num, pct);
    buf[p..p + ps.len()].copy_from_slice(ps.as_bytes());
    p += ps.len();
    buf[p] = b'%';
    p += 1;
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_rate<'a>(buf: &'a mut [u8], bps: u32) -> &'a str {
    let mut tmp = [0u8; 20];
    let s = fmt_bytes(&mut tmp, bps as u64);
    let mut p = 0usize;
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p..p + 2].copy_from_slice(b"/s");
    p += 2;
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_rate_prefixed<'a>(buf: &'a mut [u8], prefix: &str, bps: u32) -> &'a str {
    let mut p = 0usize;
    let pre = prefix.as_bytes();
    buf[p..p + pre.len()].copy_from_slice(pre);
    p += pre.len();
    buf[p..p + 2].copy_from_slice(b": ");
    p += 2;
    let mut rb = [0u8; 24];
    let s = fmt_rate(&mut rb, bps);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_two_rates<'a>(
    buf: &'a mut [u8],
    a_label: &str,
    a_bps: u32,
    b_label: &str,
    b_bps: u32,
) -> &'a str {
    let mut p = 0usize;
    let mut tmp = [0u8; 24];
    let a = fmt_rate_prefixed(&mut tmp, a_label, a_bps);
    buf[p..p + a.len()].copy_from_slice(a.as_bytes());
    p += a.len();
    buf[p..p + 2].copy_from_slice(b"  ");
    p += 2;
    let mut tmp2 = [0u8; 24];
    let b = fmt_rate_prefixed(&mut tmp2, b_label, b_bps);
    buf[p..p + b.len()].copy_from_slice(b.as_bytes());
    p += b.len();
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_cpu_value<'a>(buf: &'a mut [u8], pct: u32, mhz: u32) -> &'a str {
    let mut p = 0usize;
    let mut num = [0u8; 12];
    let s = fmt_u32(&mut num, pct);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p..p + 2].copy_from_slice(b"% ");
    p += 2;
    if mhz > 0 {
        let s = fmt_u32(&mut num, mhz);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        buf[p..p + 4].copy_from_slice(b" MHz");
        p += 4;
    }
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_mhz_value<'a>(buf: &'a mut [u8], current: u32, max: u32) -> &'a str {
    let mut p = 0usize;
    let mut num = [0u8; 12];
    let s = fmt_u32(&mut num, current);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p..p + 4].copy_from_slice(b" MHz");
    p += 4;
    if max > 0 {
        buf[p..p + 7].copy_from_slice(b" / max ");
        p += 7;
        let s = fmt_u32(&mut num, max);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
    }
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_value_unit<'a>(buf: &'a mut [u8], value: u32, unit: &str) -> &'a str {
    let mut p = 0usize;
    let mut num = [0u8; 12];
    let s = fmt_u32(&mut num, value);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    if !unit.is_empty() {
        buf[p] = b' ';
        p += 1;
        let u = unit.as_bytes();
        buf[p..p + u.len()].copy_from_slice(u);
        p += u.len();
    }
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}

fn fmt_u64_short<'a>(buf: &'a mut [u8], value: u64) -> &'a str {
    if value > u32::MAX as u64 {
        let mib = (value / 1024 / 1024).min(u32::MAX as u64) as u32;
        return fmt_value_unit(buf, mib, "M");
    }
    let mut num = [0u8; 12];
    let s = fmt_u32(&mut num, value as u32);
    buf[..s.len()].copy_from_slice(s.as_bytes());
    core::str::from_utf8(&buf[..s.len()]).unwrap_or("")
}

fn fmt_resolution<'a>(buf: &'a mut [u8], w: u32, h: u32, bpp: u32) -> &'a str {
    let mut p = 0usize;
    let mut num = [0u8; 12];
    let s = fmt_u32(&mut num, w);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p] = b'x';
    p += 1;
    let s = fmt_u32(&mut num, h);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    if bpp > 0 {
        buf[p..p + 2].copy_from_slice(b"  ");
        p += 2;
        let s = fmt_u32(&mut num, bpp);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        buf[p..p + 4].copy_from_slice(b" bpp");
        p += 4;
    }
    core::str::from_utf8(&buf[..p]).unwrap_or("")
}
