//! Event Viewer — Windows EventViewer-style log viewer for anyOS.
//!
//! Reads log files produced by logd (/System/logs/system.log and rotated files)
//! and presents them in a sortable, filterable DataGrid with a detail pane.

#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use libanyui_client as ui;
use ui::Widget;

anyos_std::entry!(main);

// ── Constants ────────────────────────────────────────────────────────────────

const LOG_DIR: &str = "/System/logs";
const WIN_W: u32 = 800;
const WIN_H: u32 = 520;
const TOOLBAR_H: u32 = 36;
const DETAIL_H: u32 = 100;

/// Color constants for log levels (ARGB).
const COLOR_ERROR: u32 = 0xFFFF4444;
const COLOR_WARN: u32 = 0xFFFFAA00;
const COLOR_INFO: u32 = 0xFF58D68D;
const COLOR_KERN: u32 = 0xFF5DADE2;
const COLOR_DEBUG: u32 = 0xFF969696;
const COLOR_DEFAULT: u32 = 0xFFD0D0D0;

// ── Parsed log entry ─────────────────────────────────────────────────────────

struct LogEntry {
    timestamp: String,
    level: String,
    source: String,
    message: String,
}

// ── Application state ────────────────────────────────────────────────────────

struct App {
    /// All parsed log entries (newest first).
    entries: Vec<LogEntry>,
    /// Filtered view indices into `entries`.
    filtered: Vec<usize>,
    /// Current level filter: 0=All, 1=Error, 2=Warn, 3=Info, 4=Kern, 5=Debug.
    filter_level: u32,
    /// Current search text (lowercase).
    search_text: String,
    /// DataGrid control.
    grid: ui::DataGrid,
    /// Detail label.
    detail: ui::Label,
    /// Status bar label.
    status: ui::Label,
}

static mut APP: Option<App> = None;

fn app() -> &'static mut App {
    unsafe { APP.as_mut().expect("app not initialized") }
}

// ── Log file parsing ─────────────────────────────────────────────────────────

/// Parse a single log line.
/// Format: `[HH:MM:SS.mmm] LEVEL source: message`
fn parse_log_line(line: &str) -> Option<LogEntry> {
    let line = line.trim();
    if line.len() < 16 || !line.starts_with('[') {
        return None;
    }

    // Extract timestamp [HH:MM:SS.mmm]
    let close = line.find(']')?;
    let timestamp = String::from(&line[..close + 1]);

    let rest = line[close + 1..].trim_start();
    if rest.is_empty() {
        return None;
    }

    // Extract level (first whitespace-delimited word)
    let level_end = rest.find(' ').unwrap_or(rest.len());
    let level = String::from(&rest[..level_end]);

    let rest = rest[level_end..].trim_start();

    // Extract source and message: `source: message`
    let (source, message) = if let Some(colon_pos) = rest.find(": ") {
        (
            String::from(&rest[..colon_pos]),
            String::from(&rest[colon_pos + 2..]),
        )
    } else {
        (String::from(""), String::from(rest))
    };

    Some(LogEntry {
        timestamp,
        level,
        source,
        message,
    })
}

/// Read and parse all log files (system.log, system.log.1, etc.).
/// Returns entries with newest first.
fn load_log_files() -> Vec<LogEntry> {
    let mut entries = Vec::new();

    // Read current log file first (has newest entries)
    load_single_file(&format!("{}/system.log", LOG_DIR), &mut entries);

    // Read rotated files (older entries)
    for i in 1..=8 {
        let path = format!("{}/system.log.{}", LOG_DIR, i);
        let before = entries.len();
        load_single_file(&path, &mut entries);
        if entries.len() == before {
            break; // No more rotated files
        }
    }

    // Reverse so newest entries are first
    entries.reverse();
    entries
}

/// Parse a single log file and append entries to the vector.
fn load_single_file(path: &str, entries: &mut Vec<LogEntry>) {
    if let Ok(content) = anyos_std::fs::read_to_string(path) {
        for line in content.split('\n') {
            if let Some(entry) = parse_log_line(line) {
                entries.push(entry);
            }
        }
    }
}

// ── Filtering ────────────────────────────────────────────────────────────────

/// Level name for filter index.
fn level_matches(entry_level: &str, filter: u32) -> bool {
    match filter {
        0 => true, // All
        1 => entry_level == "ERROR",
        2 => entry_level == "WARN",
        3 => entry_level == "INFO",
        4 => entry_level == "KERN",
        5 => entry_level == "DEBUG",
        _ => true,
    }
}

/// Apply current filters and rebuild the filtered index list.
fn apply_filters(a: &mut App) {
    a.filtered.clear();
    for (i, entry) in a.entries.iter().enumerate() {
        if !level_matches(&entry.level, a.filter_level) {
            continue;
        }
        if !a.search_text.is_empty() {
            // Simple case-insensitive substring search
            let haystack_src = to_lower(&entry.source);
            let haystack_msg = to_lower(&entry.message);
            let haystack_lvl = to_lower(&entry.level);
            if !haystack_src.contains(a.search_text.as_str())
                && !haystack_msg.contains(a.search_text.as_str())
                && !haystack_lvl.contains(a.search_text.as_str())
            {
                continue;
            }
        }
        a.filtered.push(i);
    }
}

/// Simple ASCII lowercase conversion.
fn to_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b >= b'A' && b <= b'Z' {
            out.push((b + 32) as char);
        } else {
            out.push(b as char);
        }
    }
    out
}

// ── DataGrid population ─────────────────────────────────────────────────────

/// Color for a log level string.
fn level_color(level: &str) -> u32 {
    match level {
        "ERROR" => COLOR_ERROR,
        "WARN" => COLOR_WARN,
        "INFO" => COLOR_INFO,
        "KERN" => COLOR_KERN,
        "DEBUG" => COLOR_DEBUG,
        _ => COLOR_DEFAULT,
    }
}

/// Populate the DataGrid from the filtered entries.
fn populate_grid(a: &App) {
    let count = a.filtered.len();

    // Build row data
    let mut rows: Vec<Vec<&str>> = Vec::with_capacity(count);
    for &idx in &a.filtered {
        let e = &a.entries[idx];
        rows.push(alloc::vec![
            e.timestamp.as_str(),
            e.level.as_str(),
            e.source.as_str(),
            e.message.as_str(),
        ]);
    }
    a.grid.set_data(&rows);

    // Set level column colors
    let col_count = 4u32;
    let mut colors = alloc::vec![0u32; count * col_count as usize];
    for (row, &idx) in a.filtered.iter().enumerate() {
        let color = level_color(&a.entries[idx].level);
        // Color the level cell (column 1)
        colors[row * col_count as usize + 1] = color;
    }
    a.grid.set_cell_colors(&colors);

    // Update status bar
    let total = a.entries.len();
    let shown = a.filtered.len();
    if a.filter_level == 0 && a.search_text.is_empty() {
        a.status.set_text(&format!("{} events", total));
    } else {
        a.status.set_text(&format!("{} of {} events", shown, total));
    }
}

/// Show detail for a selected row.
fn show_detail(a: &App, row_index: u32) {
    if (row_index as usize) >= a.filtered.len() {
        a.detail.set_text("");
        return;
    }
    let idx = a.filtered[row_index as usize];
    let e = &a.entries[idx];
    let detail = format!(
        "{} {} {}: {}",
        e.timestamp, e.level, e.source, e.message
    );
    a.detail.set_text(&detail);
}

// ── Reload ───────────────────────────────────────────────────────────────────

/// Reload log files and refresh the view.
fn reload() {
    let a = app();
    a.entries = load_log_files();
    apply_filters(a);
    populate_grid(a);
    a.detail.set_text("");
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    if !ui::init() {
        anyos_std::println!("eventviewer: failed to load libanyui.so");
        return;
    }

    // ── Window ──
    let win = ui::Window::new("Event Viewer", -1, -1, WIN_W, WIN_H);

    // ── Toolbar ──
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(WIN_W, TOOLBAR_H);
    toolbar.set_padding(6, 4, 6, 4);
    win.add(&toolbar);

    let btn_refresh = toolbar.add_button("Refresh");
    btn_refresh.set_size(70, 28);

    toolbar.add_separator();

    let lbl_filter = toolbar.add_label("Level:");
    lbl_filter.set_size(40, 28);
    lbl_filter.set_text_color(0xFF969696);

    let seg_level = ui::SegmentedControl::new("All|Error|Warn|Info|Kern|Debug");
    seg_level.set_size(360, 28);
    toolbar.add(&seg_level);

    toolbar.add_separator();

    let search = ui::SearchField::new();
    search.set_placeholder("Search...");
    search.set_size(140, 28);
    toolbar.add(&search);

    // ── Status bar (bottom) ──
    let status_bar = ui::View::new();
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 24);
    status_bar.set_color(0xFF252526);
    status_bar.set_padding(8, 3, 8, 3);
    win.add(&status_bar);

    let status_label = ui::Label::new("Loading...");
    status_label.set_text_color(0xFF969696);
    status_label.set_font_size(12);
    status_label.set_size(400, 18);
    status_bar.add(&status_label);

    // ── Detail pane (above status bar) ──
    let detail_view = ui::View::new();
    detail_view.set_dock(ui::DOCK_BOTTOM);
    detail_view.set_size(WIN_W, DETAIL_H);
    detail_view.set_color(0xFF1E1E1E);
    detail_view.set_padding(8, 6, 8, 6);
    win.add(&detail_view);

    let detail_title = ui::Label::new("Details");
    detail_title.set_position(0, 0);
    detail_title.set_size(100, 16);
    detail_title.set_text_color(0xFF969696);
    detail_title.set_font_size(11);
    detail_view.add(&detail_title);

    let detail_label = ui::Label::new("");
    detail_label.set_position(0, 20);
    detail_label.set_size(WIN_W - 16, DETAIL_H - 28);
    detail_label.set_text_color(0xFFD0D0D0);
    detail_label.set_font_size(12);
    detail_label.set_font(4); // Monospace
    detail_view.add(&detail_label);

    // ── Separator between grid and detail ──
    let sep = ui::Divider::new();
    sep.set_dock(ui::DOCK_BOTTOM);
    sep.set_size(WIN_W, 1);
    win.add(&sep);

    // ── DataGrid (fills remaining space) ──
    let grid = ui::DataGrid::new(WIN_W, WIN_H - TOOLBAR_H - DETAIL_H - 24 - 1);
    grid.set_dock(ui::DOCK_FILL);
    grid.set_row_height(20);
    grid.set_header_height(24);
    grid.set_font_size(12);
    grid.set_font(4); // Monospace for log data

    let cols = alloc::vec![
        ui::ColumnDef::new("Time").width(120),
        ui::ColumnDef::new("Level").width(60),
        ui::ColumnDef::new("Source").width(100),
        ui::ColumnDef::new("Message").width(500),
    ];
    grid.set_columns(&cols);
    win.add(&grid);

    // ── Initialize app state ──
    let entries = load_log_files();
    let entry_count = entries.len();
    let mut filtered = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        filtered.push(i);
    }

    unsafe {
        APP = Some(App {
            entries,
            filtered,
            filter_level: 0,
            search_text: String::new(),
            grid,
            detail: detail_label,
            status: status_label,
        });
    }

    // Initial grid population
    populate_grid(app());

    // ── Event handlers ──

    // Refresh button
    btn_refresh.on_click(|_| {
        reload();
    });

    // Level filter
    seg_level.on_active_changed(|e| {
        let a = app();
        a.filter_level = e.index;
        apply_filters(a);
        populate_grid(a);
        a.detail.set_text("");
    });

    // Search field — filter on each keystroke
    search.on_text_changed(|e| {
        let text = e.text();
        let a = app();
        a.search_text = to_lower(&text);
        apply_filters(a);
        populate_grid(a);
        a.detail.set_text("");
    });

    // Grid row selection
    app().grid.on_selection_changed(|e| {
        show_detail(app(), e.index);
    });

    // Auto-refresh every 10 seconds
    ui::set_timer(10_000, || {
        reload();
    });

    // ── Run ──
    ui::run();
}
