#![no_std]
#![no_main]

use anyos_std::fs;
use anyos_std::{Vec, String, format};
use libanyui_client as anyui;
#[allow(unused_imports)]
use anyui::Widget;

anyos_std::entry!(main);

const WIN_W: u32 = 1100;
const WIN_H: u32 = 650;
const CHART_SIZE: u32 = 500;

// ── Trig (integer-LUT based, no libm dependency) ───────────────────────────

const TRIG_N: usize = 1024;
static mut SIN_TAB: [i32; TRIG_N] = [0i32; TRIG_N];
static mut COS_TAB: [i32; TRIG_N] = [0i32; TRIG_N];
const TRIG_SCALE: i32 = 16384; // fixed-point 14-bit

fn init_trig() {
    // Build sin/cos LUT via integer Taylor (good enough for drawing)
    // angle index 0..1024 maps to 0..2*PI
    for i in 0..TRIG_N {
        // Use Cordic-style: angle = i * 2*PI / 1024
        // sin via 5-term Taylor around 0
        let a_fixed = (i as i64) * 6283185 / (TRIG_N as i64); // angle * 1e6
        let a = a_fixed as f64 / 1_000_000.0;
        let s = taylor_sin(a);
        let c = taylor_sin(a + 1.5707963267948966); // cos = sin(a + pi/2)
        unsafe {
            SIN_TAB[i] = (s * TRIG_SCALE as f64) as i32;
            COS_TAB[i] = (c * TRIG_SCALE as f64) as i32;
        }
    }
}

fn f64_floor(x: f64) -> f64 {
    let i = x as i64;
    if (i as f64) > x { (i - 1) as f64 } else { i as f64 }
}

fn taylor_sin(mut x: f64) -> f64 {
    // Reduce to [-PI, PI]
    const TWO_PI: f64 = 6.283185307179586;
    const PI: f64 = 3.141592653589793;
    x = x - (f64_floor(x / TWO_PI) * TWO_PI);
    if x > PI { x -= TWO_PI; }
    if x < -PI { x += TWO_PI; }
    // Taylor: sin(x) ≈ x - x³/6 + x⁵/120 - x⁷/5040 + x⁹/362880
    let x2 = x * x;
    let x3 = x2 * x;
    let x5 = x3 * x2;
    let x7 = x5 * x2;
    let x9 = x7 * x2;
    x - x3 / 6.0 + x5 / 120.0 - x7 / 5040.0 + x9 / 362880.0
}

/// Return (sin, cos) * TRIG_SCALE for angle in 0..65536 (= 0..2*PI)
fn sincos(angle_65k: u32) -> (i32, i32) {
    let idx = ((angle_65k as usize) * TRIG_N / 65536) % TRIG_N;
    unsafe { (SIN_TAB[idx], COS_TAB[idx]) }
}

// ── Color palette for chart segments ────────────────────────────────────────

const PALETTE: [u32; 16] = [
    0xFFE53935, // red
    0xFF8E24AA, // purple
    0xFF1E88E5, // blue
    0xFF00ACC1, // cyan
    0xFF43A047, // green
    0xFFFFB300, // amber
    0xFFFF7043, // deep orange
    0xFF5E35B1, // deep purple
    0xFF039BE5, // light blue
    0xFF00897B, // teal
    0xFF7CB342, // light green
    0xFFFDD835, // yellow
    0xFFF4511E, // orange
    0xFF3949AB, // indigo
    0xFF00BFA5, // teal accent
    0xFFD81B60, // pink
];

fn darken(color: u32, factor: u32) -> u32 {
    let a = color & 0xFF000000;
    let r = ((color >> 16) & 0xFF) * factor / 100;
    let g = ((color >> 8) & 0xFF) * factor / 100;
    let b = (color & 0xFF) * factor / 100;
    a | (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
}

// ── Size formatting ─────────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        let whole = bytes / GB;
        let frac = (bytes % GB) * 10 / GB;
        format!("{}.{} GB", whole, frac)
    } else if bytes >= MB {
        let whole = bytes / MB;
        let frac = (bytes % MB) * 10 / MB;
        format!("{}.{} MB", whole, frac)
    } else if bytes >= KB {
        let whole = bytes / KB;
        let frac = (bytes % KB) * 10 / KB;
        format!("{}.{} KB", whole, frac)
    } else {
        format!("{} B", bytes)
    }
}

fn format_percent(part: u64, total: u64) -> String {
    if total == 0 { return String::from("0%"); }
    let pct = part * 100 / total;
    let frac = (part * 1000 / total) % 10;
    format!("{}.{}%", pct, frac)
}

fn format_items(files: u32, dirs: u32) -> String {
    if dirs > 0 && files > 0 {
        format!("{} files, {} dirs", files, dirs)
    } else if dirs > 0 {
        format!("{} dirs", dirs)
    } else {
        format!("{} files", files)
    }
}

// ── Mount point info ────────────────────────────────────────────────────────

#[allow(dead_code)]
struct MountInfo {
    path: String,
    fstype: String,
    total_bytes: u64,
    used_bytes: u64,
    free_bytes: u64,
}

fn get_mounts() -> Vec<MountInfo> {
    let mut mounts = Vec::new();
    let mut buf = [0u8; 4096];
    let n = fs::list_mounts(&mut buf);
    if n == u32::MAX { return mounts; }

    let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
    for line in text.split('\n') {
        if line.is_empty() { continue; }
        let mut parts = line.splitn(2, '\t');
        let path = parts.next().unwrap_or("");
        let fstype = parts.next().unwrap_or("");
        if path.is_empty() { continue; }

        let (total, used, free) = if let Some(st) = fs::statfs(path) {
            (st.total_bytes, st.used_bytes, st.free_bytes)
        } else {
            (0, 0, 0)
        };

        mounts.push(MountInfo {
            path: String::from(path),
            fstype: String::from(fstype),
            total_bytes: total,
            used_bytes: used,
            free_bytes: free,
        });
    }
    mounts
}

// ── Directory scanning ──────────────────────────────────────────────────────

struct DirResult {
    name: String,
    path: String,
    size: u64,
    is_dir: bool,
    file_count: u32,
    dir_count: u32,
}

/// Scan children of a directory and return results sorted by size desc.
fn scan_dir_sizes(path: &str) -> Vec<DirResult> {
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries {
            if entry.name == "." || entry.name == ".." { continue; }
            let child_path = if path == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{}/{}", path, entry.name)
            };
            if entry.is_dir() {
                let (size, files, dirs) = calc_dir_size(&child_path, 0);
                results.push(DirResult {
                    name: entry.name,
                    path: child_path,
                    size,
                    is_dir: true,
                    file_count: files,
                    dir_count: dirs,
                });
            } else {
                results.push(DirResult {
                    name: entry.name,
                    path: child_path,
                    size: entry.size as u64,
                    is_dir: false,
                    file_count: 1,
                    dir_count: 0,
                });
            }
        }
    }
    results.sort_by(|a, b| b.size.cmp(&a.size));
    results
}

fn calc_dir_size(path: &str, depth: u32) -> (u64, u32, u32) {
    if depth > 20 { return (0, 0, 0); }
    let mut total: u64 = 0;
    let mut files: u32 = 0;
    let mut dirs: u32 = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries {
            if entry.name == "." || entry.name == ".." { continue; }
            if entry.is_dir() {
                dirs += 1;
                let child_path = format!("{}/{}", path, entry.name);
                let (sub_size, sub_files, sub_dirs) = calc_dir_size(&child_path, depth + 1);
                total += sub_size;
                files += sub_files;
                dirs += sub_dirs;
            } else {
                files += 1;
                total += entry.size as u64;
            }
        }
    }
    (total, files, dirs)
}

// ── Sunburst ring chart drawing ─────────────────────────────────────────────

/// Ring segment: from angle_start..angle_end (in 0..65536), at ring level
struct RingSegment {
    angle_start: u32,
    angle_end: u32,
    color: u32,
}

/// Draw a filled arc (ring segment) between inner and outer radius.
fn draw_arc(canvas: &anyui::Canvas, cx: i32, cy: i32,
            r_inner: i32, r_outer: i32,
            angle_start: u32, angle_end: u32, color: u32) {
    if angle_end <= angle_start { return; }
    // Bounding box for the arc
    let r = r_outer;
    let x_min = (cx - r).max(0);
    let y_min = (cy - r).max(0);
    let x_max = cx + r;
    let y_max = cy + r;

    let r_inner_sq = (r_inner as i64) * (r_inner as i64);
    let r_outer_sq = (r_outer as i64) * (r_outer as i64);

    for py in y_min..=y_max {
        for px in x_min..=x_max {
            let dx = (px - cx) as i64;
            let dy = (py - cy) as i64;
            let dist_sq = dx * dx + dy * dy;

            // Check radius
            if dist_sq < r_inner_sq || dist_sq > r_outer_sq { continue; }

            // Check angle (atan2 via LUT)
            let angle = pixel_angle(dx as i32, dy as i32);
            if angle_in_range(angle, angle_start, angle_end) {
                canvas.set_pixel(px, py, color);
            }
        }
    }
}

/// Get angle of pixel (dx, dy) in 0..65536 range. 0 = top (12 o'clock), CW.
fn pixel_angle(dx: i32, dy: i32) -> u32 {
    if dx == 0 && dy == 0 { return 0; }
    // atan2 approximation: use octant-based integer atan2
    // Map to 0..65536 with 0 = up, clockwise
    let angle = iatan2(dx, -dy); // -dy because screen Y is inverted
    angle as u32
}

/// Integer atan2 returning 0..65535 (full circle). 0 = positive Y axis.
fn iatan2(x: i32, y: i32) -> u32 {
    if x == 0 && y == 0 { return 0; }
    // Use CORDIC-style: determine octant, then interpolate
    let ax = if x < 0 { -x } else { x };
    let ay = if y < 0 { -y } else { y };

    // Base angle in 0..8192 (1/8 of circle = 45 degrees)
    let base = if ay == 0 {
        8192 // 45 degrees equivalent for edge case
    } else if ax <= ay {
        (ax as u64 * 8192 / ay as u64) as u32
    } else {
        8192 - (ay as u64 * 8192 / ax as u64) as u32
    };

    // Map octant
    let octant_size: u32 = 8192; // 65536 / 8
    if x >= 0 && y > 0 {
        base // Q1 top-right: 0..8192
    } else if x > 0 && y <= 0 {
        2 * octant_size - base // Q2 bottom-right
    } else if x <= 0 && y < 0 {
        2 * octant_size + base // Q3 bottom-left
    } else {
        4 * octant_size - base // Q4 top-left
    }
}

fn angle_in_range(angle: u32, start: u32, end: u32) -> bool {
    if end <= start { return false; }
    // No wrap-around needed since we don't wrap past 65536
    angle >= start && angle < end
}

fn draw_ring_chart(canvas: &anyui::Canvas, results: &[DirResult], total: u64, selected: i32) {
    let w = CHART_SIZE as i32;
    let h = CHART_SIZE as i32;
    let cx = w / 2;
    let cy = h / 2;

    canvas.clear(0xFF1E1E1E);

    if total == 0 || results.is_empty() {
        canvas.draw_text(cx - 40, cy - 8, 0xFF888888, 0, 13, "No data");
        return;
    }

    // Ring parameters
    let r_center = 50; // inner hole
    let ring_width = 40;
    let max_rings = 4; // depth levels
    let gap = 2;

    // Draw center circle with total size text
    canvas.fill_circle(cx, cy, r_center - 2, 0xFF2D2D30);
    let size_text = format_size(total);
    let text_x = cx - (size_text.len() as i32 * 4);
    canvas.draw_text(text_x, cy - 6, 0xFFCCCCCC, 1, 12, &size_text);

    // Outer ring 1: top-level entries
    let r_inner = r_center + gap;
    let r_outer = r_inner + ring_width;

    let mut angle: u32 = 0;
    let items_to_show = results.len().min(32);

    for (i, r) in results[..items_to_show].iter().enumerate() {
        if r.size == 0 { continue; }
        let span = (r.size as u64 * 65536 / total as u64) as u32;
        if span < 100 { // skip tiny slices
            angle += span;
            continue;
        }

        let base_color = PALETTE[i % PALETTE.len()];
        let color = if selected == i as i32 {
            0xFFFFFFFF // highlight selected
        } else {
            base_color
        };

        draw_arc(canvas, cx, cy, r_inner, r_outer, angle, angle + span, color);

        // Draw sub-rings for directories (deeper levels)
        if r.is_dir && span > 400 {
            draw_sub_ring(canvas, cx, cy, &r.path, r.size,
                          angle, span, base_color, r_outer + gap, ring_width,
                          2, max_rings);
        }

        angle += span;
    }

    // Draw ring separators (dark circles)
    for ring in 0..max_rings {
        let r = r_center + (ring as i32) * (ring_width + gap);
        canvas.draw_circle(cx, cy, r, 0xFF1E1E1E);
    }
}

fn draw_sub_ring(canvas: &anyui::Canvas, cx: i32, cy: i32,
                 path: &str, parent_size: u64,
                 parent_angle: u32, parent_span: u32,
                 _parent_color: u32, r_inner: i32, ring_width: i32,
                 depth: u32, max_depth: u32) {
    if depth > max_depth { return; }
    if parent_size == 0 || parent_span < 200 { return; }

    let r_outer = r_inner + ring_width;
    let gap = 2;

    // Scan children
    if let Ok(entries) = fs::read_dir(path) {
        let mut children: Vec<(String, u64, bool)> = Vec::new();
        for entry in entries {
            if entry.name == "." || entry.name == ".." { continue; }
            let child_path = format!("{}/{}", path, entry.name);
            let size = if entry.is_dir() {
                let (s, _, _) = calc_dir_size(&child_path, 0);
                s
            } else {
                entry.size as u64
            };
            if size > 0 {
                children.push((child_path, size, entry.is_dir()));
            }
        }
        children.sort_by(|a, b| b.1.cmp(&a.1));

        let mut sub_angle = parent_angle;
        for (i, (child_path, child_size, is_dir)) in children.iter().enumerate() {
            let span = (*child_size as u64 * parent_span as u64 / parent_size as u64) as u32;
            if span < 100 {
                sub_angle += span;
                continue;
            }

            let shade = 100 - (depth - 1) * 15; // progressively darker
            let color = darken(PALETTE[(i + depth as usize * 3) % PALETTE.len()], shade);

            draw_arc(canvas, cx, cy, r_inner, r_outer, sub_angle, sub_angle + span, color);

            // Recurse deeper
            if *is_dir && depth < max_depth && span > 400 {
                draw_sub_ring(canvas, cx, cy, child_path, *child_size,
                              sub_angle, span, color,
                              r_outer + gap, ring_width,
                              depth + 1, max_depth);
            }

            sub_angle += span;
        }
    }
}

// ── App modes ───────────────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    SelectMount,
    Scanning,
    Browsing,
}

const MAX_HISTORY: usize = 64;

// ── App state ───────────────────────────────────────────────────────────────

#[allow(dead_code)]
struct AppState {
    mode: Mode,
    mounts: Vec<MountInfo>,
    current_path: String,
    path_history: Vec<String>,
    dir_results: Vec<DirResult>,
    total_dir_size: u64,
    selected_row: i32,

    // UI widgets
    win: anyui::Window,
    toolbar: anyui::Toolbar,
    btn_back: anyui::IconButton,
    btn_up: anyui::IconButton,
    btn_refresh: anyui::IconButton,
    lbl_path: anyui::Label,
    lbl_info: anyui::Label,
    status_label: anyui::Label,
    grid: anyui::DataGrid,
    mount_grid: anyui::DataGrid,
    chart: anyui::Canvas,
    split: anyui::SplitView,
    progress: anyui::ProgressBar,
}

anyos_std::global_app_state!(AppState);

// ── Show mount selection ────────────────────────────────────────────────────

fn show_mount_selection() {
    let a = app();
    a.mode = Mode::SelectMount;
    a.mounts = get_mounts();

    a.mount_grid.set_visible(true);
    a.split.set_visible(false);
    a.progress.set_visible(false);
    a.btn_back.set_enabled(false);
    a.btn_up.set_enabled(false);

    a.lbl_path.set_text("Select a volume to analyze:");
    a.lbl_info.set_text("");

    // Build mount table data
    let mount_count = a.mounts.len();
    a.mount_grid.set_row_count(mount_count as u32);

    for (i, m) in a.mounts.iter().enumerate() {
        let row = i as u32;
        a.mount_grid.set_cell(row, 0, &m.path);
        a.mount_grid.set_cell(row, 1, &m.fstype);
        a.mount_grid.set_cell(row, 2, &format_size(m.total_bytes));
        a.mount_grid.set_cell(row, 3, &format_size(m.used_bytes));
        a.mount_grid.set_cell(row, 4, &format_size(m.free_bytes));
        if m.total_bytes > 0 {
            a.mount_grid.set_cell(row, 5, &format_percent(m.used_bytes, m.total_bytes));
        } else {
            a.mount_grid.set_cell(row, 5, "-");
        }
    }

    a.status_label.set_text("Double-click a volume to analyze.");
}

// ── Start scan ──────────────────────────────────────────────────────────────

fn start_scan(path: &str) {
    let a = app();
    a.mode = Mode::Scanning;
    a.current_path = String::from(path);

    a.mount_grid.set_visible(false);
    a.split.set_visible(false);
    a.progress.set_visible(true);
    a.progress.set_state(50);
    a.btn_back.set_enabled(false);
    a.btn_up.set_enabled(false);

    a.lbl_path.set_text(&format!("Scanning {}...", path));
    a.lbl_info.set_text("");
    a.status_label.set_text("Scanning directories...");
}

fn do_scan() {
    let path = app().current_path.clone();
    let results = scan_dir_sizes(&path);

    let a = app();
    a.dir_results = results;
    a.selected_row = -1;

    let mut total: u64 = 0;
    for r in &a.dir_results {
        total += r.size;
    }
    a.total_dir_size = total;

    show_results();
}

// ── Show results in grid + chart ────────────────────────────────────────────

fn show_results() {
    let a = app();
    a.mode = Mode::Browsing;

    a.mount_grid.set_visible(false);
    a.split.set_visible(true);
    a.progress.set_visible(false);
    a.btn_back.set_enabled(!a.path_history.is_empty());
    a.btn_up.set_enabled(a.current_path != "/");

    a.lbl_path.set_text(&a.current_path);

    // Summary
    let mut total_files: u32 = 0;
    let mut total_dirs: u32 = 0;
    for r in &a.dir_results {
        if r.is_dir {
            total_dirs += 1 + r.dir_count;
            total_files += r.file_count;
        } else {
            total_files += 1;
        }
    }
    a.lbl_info.set_text(&format!(
        "Total: {} — {} files, {} dirs",
        format_size(a.total_dir_size), total_files, total_dirs,
    ));

    // Fill DataGrid
    let total = a.total_dir_size;
    let count = a.dir_results.len();
    a.grid.set_row_count(count as u32);

    let mut text_colors = Vec::with_capacity(count * 5);

    for (i, r) in a.dir_results.iter().enumerate() {
        let row = i as u32;
        let pct = if total > 0 { r.size * 100 / total } else { 0 };

        // Column 0: Name (with / suffix for dirs)
        if r.is_dir {
            a.grid.set_cell(row, 0, &format!("{}/", r.name));
        } else {
            a.grid.set_cell(row, 0, &r.name);
        }

        // Column 1: Size
        a.grid.set_cell(row, 1, &format_size(r.size));

        // Column 2: Percentage
        a.grid.set_cell(row, 2, &format_percent(r.size, total));

        // Column 3: Items
        if r.is_dir {
            a.grid.set_cell(row, 3, &format_items(r.file_count, r.dir_count));
        } else {
            a.grid.set_cell(row, 3, "1 file");
        }

        // Column 4: Last modified (placeholder)
        a.grid.set_cell(row, 4, "");

        // Color for each column (5 cols)
        let row_color = if pct >= 50 {
            0xFFFF4444 // red
        } else if pct >= 25 {
            0xFFFF8800 // orange
        } else if pct >= 10 {
            0xFFFFCC00 // yellow
        } else {
            0 // default
        };
        for _ in 0..5 {
            text_colors.push(row_color);
        }
    }

    a.grid.set_cell_colors(&text_colors);

    // Draw chart
    draw_ring_chart(&a.chart, &a.dir_results, total, a.selected_row);

    let item_count = a.dir_results.len();
    a.status_label.set_text(&format!(
        "{} items — {}", item_count, format_size(total),
    ));
}

// ── Navigation ──────────────────────────────────────────────────────────────

fn navigate_into(index: u32) {
    let idx = index as usize;
    let a = app();
    if idx >= a.dir_results.len() { return; }
    if !a.dir_results[idx].is_dir { return; }

    let new_path = a.dir_results[idx].path.clone();

    if a.path_history.len() >= MAX_HISTORY {
        a.path_history.remove(0);
    }
    a.path_history.push(a.current_path.clone());

    start_scan(&new_path);
    do_scan();
}

fn navigate_up() {
    let a = app();
    if a.current_path == "/" { return; }

    let path = a.current_path.clone();
    let parent = if let Some(pos) = path.rfind('/') {
        if pos == 0 { String::from("/") } else { String::from(&path[..pos]) }
    } else {
        String::from("/")
    };

    if a.path_history.len() >= MAX_HISTORY {
        a.path_history.remove(0);
    }
    a.path_history.push(a.current_path.clone());

    start_scan(&parent);
    do_scan();
}

fn navigate_back() {
    let a = app();
    if a.path_history.is_empty() { return; }

    let prev = a.path_history.pop().unwrap();
    start_scan(&prev);
    do_scan();
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() { return; }
    init_trig();

    let win = anyui::Window::new("Disk Usage", -1, -1, WIN_W, WIN_H);

    // ── Toolbar ─────────────────────────────────────────────────────────
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(WIN_W, 36);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_back = toolbar.add_icon_button("Back");
    btn_back.set_size(60, 28);
    btn_back.set_enabled(false);

    let btn_up = toolbar.add_icon_button("Up");
    btn_up.set_size(45, 28);
    btn_up.set_enabled(false);

    let btn_refresh = toolbar.add_icon_button("Refresh");
    btn_refresh.set_size(70, 28);

    let btn_home = toolbar.add_icon_button("Volumes");
    btn_home.set_size(75, 28);

    win.add(&toolbar);

    // ── Path label ──────────────────────────────────────────────────────
    let path_panel = anyui::View::new();
    path_panel.set_dock(anyui::DOCK_TOP);
    path_panel.set_size(WIN_W, 28);
    path_panel.set_color(0xFF1E1E1E);

    let lbl_path = anyui::Label::new("");
    lbl_path.set_dock(anyui::DOCK_FILL);
    lbl_path.set_text_color(0xFFCCCCCC);
    lbl_path.set_font_size(13);
    path_panel.add(&lbl_path);

    win.add(&path_panel);

    // ── Info label ──────────────────────────────────────────────────────
    let info_panel = anyui::View::new();
    info_panel.set_dock(anyui::DOCK_TOP);
    info_panel.set_size(WIN_W, 22);
    info_panel.set_color(0xFF2D2D30);

    let lbl_info = anyui::Label::new("");
    lbl_info.set_dock(anyui::DOCK_FILL);
    lbl_info.set_text_color(0xFF888888);
    lbl_info.set_font_size(11);
    info_panel.add(&lbl_info);

    win.add(&info_panel);

    // ── Status bar (bottom) ─────────────────────────────────────────────
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 24);
    status_bar.set_color(0xFF007ACC);

    let status_label = anyui::Label::new("Ready");
    status_label.set_dock(anyui::DOCK_FILL);
    status_label.set_text_color(0xFFFFFFFF);
    status_label.set_font_size(11);
    status_bar.add(&status_label);

    win.add(&status_bar);

    // ── Progress bar ────────────────────────────────────────────────────
    let progress = anyui::ProgressBar::new(0);
    progress.set_dock(anyui::DOCK_TOP);
    progress.set_size(WIN_W, 6);
    progress.set_visible(false);
    win.add(&progress);

    // ── Mount selection grid ────────────────────────────────────────────
    let mount_grid = anyui::DataGrid::new(WIN_W, WIN_H - 120);
    mount_grid.set_dock(anyui::DOCK_FILL);
    mount_grid.set_row_height(26);
    mount_grid.set_columns(&[
        anyui::ColumnDef::new("Mount Point").width(200),
        anyui::ColumnDef::new("Type").width(100),
        anyui::ColumnDef::new("Total").width(100).align(anyui::ALIGN_RIGHT).numeric(),
        anyui::ColumnDef::new("Used").width(100).align(anyui::ALIGN_RIGHT).numeric(),
        anyui::ColumnDef::new("Free").width(100).align(anyui::ALIGN_RIGHT).numeric(),
        anyui::ColumnDef::new("Usage").width(80).align(anyui::ALIGN_RIGHT),
    ]);
    mount_grid.set_visible(false);
    win.add(&mount_grid);

    // ── SplitView: left=grid, right=chart ───────────────────────────────
    let split = anyui::SplitView::new();
    split.set_dock(anyui::DOCK_FILL);
    split.set_orientation(0); // vertical = left/right
    split.set_split_ratio(550);
    split.set_resizable(true);
    split.set_visible(false);

    // Left: directory grid
    let grid = anyui::DataGrid::new(550, 400);
    grid.set_dock(anyui::DOCK_FILL);
    grid.set_row_height(24);
    grid.set_columns(&[
        anyui::ColumnDef::new("Name").width(220),
        anyui::ColumnDef::new("Size").width(90).align(anyui::ALIGN_RIGHT).numeric(),
        anyui::ColumnDef::new("%").width(60).align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new("Contents").width(120),
        anyui::ColumnDef::new("Modified").width(80),
    ]);
    split.add(&grid);

    // Right: ring chart
    let chart = anyui::Canvas::new(CHART_SIZE, CHART_SIZE);
    chart.set_dock(anyui::DOCK_FILL);
    chart.clear(0xFF1E1E1E);
    split.add(&chart);

    win.add(&split);

    // ── Initialize state ────────────────────────────────────────────────
    unsafe {
        APP = Some(AppState {
            mode: Mode::SelectMount,
            mounts: Vec::new(),
            current_path: String::new(),
            path_history: Vec::new(),
            dir_results: Vec::new(),
            total_dir_size: 0,
            selected_row: -1,
            win,
            toolbar,
            btn_back,
            btn_up,
            btn_refresh,
            lbl_path,
            lbl_info,
            status_label,
            grid,
            mount_grid,
            chart,
            split,
            progress,
        });
    }

    show_mount_selection();

    // ── Callbacks ───────────────────────────────────────────────────────

    // Mount grid: double-click → scan
    app().mount_grid.on_submit(|evt| {
        let idx = evt.index as usize;
        let a = app();
        if idx < a.mounts.len() {
            let path = a.mounts[idx].path.clone();
            a.path_history.clear();
            start_scan(&path);
            do_scan();
        }
    });

    // Directory grid: selection changed → update chart highlight
    app().grid.on_selection_changed(|evt| {
        let a = app();
        a.selected_row = evt.index as i32;
        let total = a.total_dir_size;
        draw_ring_chart(&a.chart, &a.dir_results, total, a.selected_row);
    });

    // Directory grid: double-click → navigate into
    app().grid.on_submit(|evt| {
        navigate_into(evt.index);
    });

    // Toolbar buttons
    app().btn_back.on_click(|_| { navigate_back(); });
    app().btn_up.on_click(|_| { navigate_up(); });

    app().btn_refresh.on_click(|_| {
        let a = app();
        if a.mode == Mode::Browsing {
            let path = a.current_path.clone();
            start_scan(&path);
            do_scan();
        } else {
            show_mount_selection();
        }
    });

    btn_home.on_click(|_| { show_mount_selection(); });

    app().win.on_close(|_| { anyui::quit(); });

    anyui::run();
}
