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

// ── Trig helpers ────────────────────────────────────────────────────────────

fn f64_floor(x: f64) -> f64 {
    let i = x as i64;
    if (i as f64) > x { (i - 1) as f64 } else { i as f64 }
}

fn taylor_sin(mut x: f64) -> f64 {
    const TWO_PI: f64 = 6.283185307179586;
    const PI: f64 = 3.141592653589793;
    x = x - (f64_floor(x / TWO_PI) * TWO_PI);
    if x > PI { x -= TWO_PI; }
    if x < -PI { x += TWO_PI; }
    let x2 = x * x;
    let x3 = x2 * x;
    let x5 = x3 * x2;
    let x7 = x5 * x2;
    let x9 = x7 * x2;
    x - x3 / 6.0 + x5 / 120.0 - x7 / 5040.0 + x9 / 362880.0
}

const TRIG_N: usize = 1024;
static mut SIN_TAB: [i32; TRIG_N] = [0i32; TRIG_N];
static mut COS_TAB: [i32; TRIG_N] = [0i32; TRIG_N];

fn init_trig() {
    for i in 0..TRIG_N {
        let a = (i as i64) * 6283185 / (TRIG_N as i64);
        let a = a as f64 / 1_000_000.0;
        unsafe {
            SIN_TAB[i] = (taylor_sin(a) * 16384.0) as i32;
            COS_TAB[i] = (taylor_sin(a + 1.5707963267948966) * 16384.0) as i32;
        }
    }
}

// ── Color palette ───────────────────────────────────────────────────────────

const PALETTE: [u32; 16] = [
    0xFFE53935, 0xFF8E24AA, 0xFF1E88E5, 0xFF00ACC1,
    0xFF43A047, 0xFFFFB300, 0xFFFF7043, 0xFF5E35B1,
    0xFF039BE5, 0xFF00897B, 0xFF7CB342, 0xFFFDD835,
    0xFFF4511E, 0xFF3949AB, 0xFF00BFA5, 0xFFD81B60,
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
        format!("{}.{} GB", bytes / GB, (bytes % GB) * 10 / GB)
    } else if bytes >= MB {
        format!("{}.{} MB", bytes / MB, (bytes % MB) * 10 / MB)
    } else if bytes >= KB {
        format!("{}.{} KB", bytes / KB, (bytes % KB) * 10 / KB)
    } else {
        format!("{} B", bytes)
    }
}

fn format_percent(part: u64, total: u64) -> String {
    if total == 0 { return String::from("0%"); }
    format!("{}.{}%", part * 100 / total, (part * 1000 / total) % 10)
}

fn format_items(files: u32, dirs: u32) -> String {
    if dirs > 0 && files > 0 { format!("{} files, {} dirs", files, dirs) }
    else if dirs > 0 { format!("{} dirs", dirs) }
    else { format!("{} files", files) }
}

fn chart_background() -> u32 {
    anyui::theme::colors().window_bg
}

fn chart_center_background() -> u32 {
    anyui::theme::colors().card_bg
}

fn chart_separator_color() -> u32 {
    anyui::theme::colors().window_bg
}

fn chart_primary_text() -> u32 {
    anyui::theme::colors().text
}

fn chart_secondary_text() -> u32 {
    anyui::theme::colors().text_secondary
}

fn usage_heat_color(pct: u64) -> u32 {
    let tc = anyui::theme::colors();
    if pct >= 50 {
        tc.destructive
    } else if pct >= 25 {
        tc.warning
    } else if pct >= 10 {
        tc.accent
    } else {
        0
    }
}

// ── Mount info ──────────────────────────────────────────────────────────────

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
        let mut parts = line.splitn(3, '\t');
        let path = parts.next().unwrap_or("");
        let fstype = parts.next().unwrap_or("");
        if path.is_empty() { continue; }
        let (total, used, free) = if let Some(st) = fs::statfs(path) {
            (st.total_bytes, st.used_bytes, st.free_bytes)
        } else { (0, 0, 0) };
        mounts.push(MountInfo {
            path: String::from(path), fstype: String::from(fstype),
            total_bytes: total, used_bytes: used, free_bytes: free,
        });
    }
    mounts
}

// ═══════════════════════════════════════════════════════════════════════════
// Cached directory tree — built once when a volume is selected
// ═══════════════════════════════════════════════════════════════════════════

/// A node in the cached directory tree.
struct TreeNode {
    name: String,
    path: String,
    own_size: u64,       // size of files directly in this dir (or file size)
    total_size: u64,     // recursive total (including children)
    is_dir: bool,
    file_count: u32,     // recursive file count
    dir_count: u32,      // recursive dir count (not counting self)
    children: Vec<usize>, // indices into the global tree vec
}

/// The entire cached tree for a volume.
struct DirTree {
    nodes: Vec<TreeNode>,
    root: usize,
}

impl DirTree {
    fn new() -> Self {
        DirTree { nodes: Vec::new(), root: 0 }
    }

    /// Build the complete tree rooted at `path`, recursively.
    fn build(root_path: &str, status: &anyui::Label, progress: &anyui::ProgressBar) -> Self {
        let mut tree = DirTree { nodes: Vec::new(), root: 0 };
        let root_idx = tree.add_dir(root_path, root_path, 0, status, progress);
        tree.root = root_idx;
        tree
    }

    fn add_dir(&mut self, path: &str, name: &str, depth: u32,
               status: &anyui::Label, progress: &anyui::ProgressBar) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(TreeNode {
            name: String::from(name),
            path: String::from(path),
            own_size: 0, total_size: 0,
            is_dir: true, file_count: 0, dir_count: 0,
            children: Vec::new(),
        });

        // Progress indication for top-level dirs
        if depth == 1 {
            status.set_text(&format!("Scanning {}...", name));
        }

        if depth > 30 { return idx; } // safety limit

        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries {
                if entry.name == "." || entry.name == ".." { continue; }
                let child_path = if path == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", path, entry.name)
                };
                if entry.is_dir() {
                    let child_idx = self.add_dir(&child_path, &entry.name, depth + 1, status, progress);
                    // Accumulate child stats into parent
                    let child_total = self.nodes[child_idx].total_size;
                    let child_files = self.nodes[child_idx].file_count;
                    let child_dirs = self.nodes[child_idx].dir_count;
                    self.nodes[idx].total_size += child_total;
                    self.nodes[idx].file_count += child_files;
                    self.nodes[idx].dir_count += 1 + child_dirs;
                    self.nodes[idx].children.push(child_idx);
                } else {
                    let size = entry.size as u64;
                    let file_idx = self.nodes.len();
                    self.nodes.push(TreeNode {
                        name: entry.name,
                        path: child_path,
                        own_size: size, total_size: size,
                        is_dir: false, file_count: 1, dir_count: 0,
                        children: Vec::new(),
                    });
                    self.nodes[idx].own_size += size;
                    self.nodes[idx].total_size += size;
                    self.nodes[idx].file_count += 1;
                    self.nodes[idx].children.push(file_idx);
                }
            }
        }

        // Sort children by total_size descending — copy children out to avoid borrow conflict
        let mut children = core::mem::take(&mut self.nodes[idx].children);
        let len = children.len();
        for i in 0..len {
            for j in (i + 1)..len {
                if self.nodes[children[j]].total_size > self.nodes[children[i]].total_size {
                    children.swap(i, j);
                }
            }
        }
        self.nodes[idx].children = children;

        idx
    }

    /// Get children of a node as a slice of node indices.
    fn children_of(&self, node_idx: usize) -> &[usize] {
        &self.nodes[node_idx].children
    }

    fn node(&self, idx: usize) -> &TreeNode {
        &self.nodes[idx]
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Ring chart drawing + hit-test for tooltips
// ═══════════════════════════════════════════════════════════════════════════

// Chart layout constants
const R_CENTER: i32 = 50;
const RING_WIDTH: i32 = 40;
const RING_GAP: i32 = 2;
const MAX_RINGS: u32 = 4;

/// A drawn segment on the chart, for mouse hit-testing.
struct ChartSegment {
    r_inner: i32,
    r_outer: i32,
    angle_start: u32,
    angle_end: u32,
    node_idx: usize, // index into DirTree.nodes
}

/// Integer atan2 returning 0..65535 (full circle). 0 = top (12 o'clock), CW.
fn iatan2(x: i32, y: i32) -> u32 {
    if x == 0 && y == 0 { return 0; }
    let ax = if x < 0 { -x } else { x };
    let ay = if y < 0 { -y } else { y };
    let base = if ay == 0 {
        8192u32
    } else if ax <= ay {
        (ax as u64 * 8192 / ay as u64) as u32
    } else {
        8192 - (ay as u64 * 8192 / ax as u64) as u32
    };
    let o: u32 = 8192;
    if x >= 0 && y > 0 { base }
    else if x > 0 && y <= 0 { 2 * o - base }
    else if x <= 0 && y < 0 { 2 * o + base }
    else { 4 * o - base }
}

fn angle_in_range(angle: u32, start: u32, end: u32) -> bool {
    end > start && angle >= start && angle < end
}

/// Draw a filled arc between r_inner..r_outer, angle_start..angle_end.
fn draw_arc(canvas: &anyui::Canvas, cx: i32, cy: i32,
            r_inner: i32, r_outer: i32,
            angle_start: u32, angle_end: u32, color: u32) {
    if angle_end <= angle_start { return; }
    let r = r_outer;
    let r_inner_sq = (r_inner as i64) * (r_inner as i64);
    let r_outer_sq = (r_outer as i64) * (r_outer as i64);

    for py in (cy - r).max(0)..=(cy + r) {
        for px in (cx - r).max(0)..=(cx + r) {
            let dx = (px - cx) as i64;
            let dy = (py - cy) as i64;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < r_inner_sq || dist_sq > r_outer_sq { continue; }
            let angle = iatan2(dx as i32, -(dy as i32));
            if angle_in_range(angle, angle_start, angle_end) {
                canvas.set_pixel(px, py, color);
            }
        }
    }
}

/// Build segments list and draw the ring chart. Returns segments for hit-testing.
fn draw_ring_chart(canvas: &anyui::Canvas, tree: &DirTree, view_node: usize,
                   selected: i32, chart_w: u32, chart_h: u32) -> Vec<ChartSegment> {
    let cx = chart_w as i32 / 2;
    let cy = chart_h as i32 / 2;
    let mut segments = Vec::new();

    canvas.clear(chart_background());

    let node = tree.node(view_node);
    let total = node.total_size;
    let children = tree.children_of(view_node);

    if total == 0 || children.is_empty() {
        canvas.draw_text(cx - 40, cy - 8, chart_secondary_text(), 0, 13, "No data");
        return segments;
    }

    // Center circle with total size
    canvas.fill_circle(cx, cy, R_CENTER - 2, chart_center_background());
    let size_text = format_size(total);
    let text_x = cx - (size_text.len() as i32 * 4);
    canvas.draw_text(text_x, cy - 6, chart_primary_text(), 1, 12, &size_text);

    // Ring 1: direct children
    let r_inner = R_CENTER + RING_GAP;
    let r_outer = r_inner + RING_WIDTH;
    let mut angle: u32 = 0;
    let max_items = children.len().min(32);

    for (i, &child_idx) in children[..max_items].iter().enumerate() {
        let child = tree.node(child_idx);
        if child.total_size == 0 { continue; }
        let span = (child.total_size * 65536 / total) as u32;
        if span < 80 { angle += span; continue; }

        let base_color = PALETTE[i % PALETTE.len()];
        let color = if selected == i as i32 { 0xFFFFFFFF } else { base_color };

        draw_arc(canvas, cx, cy, r_inner, r_outer, angle, angle + span, color);
        segments.push(ChartSegment {
            r_inner, r_outer, angle_start: angle, angle_end: angle + span, node_idx: child_idx,
        });

        // Sub-rings for directory children
        if child.is_dir && span > 300 {
            draw_sub_rings(canvas, tree, child_idx, cx, cy,
                           angle, span, r_outer + RING_GAP, 2, &mut segments);
        }
        angle += span;
    }

    // Ring separator circles
    for ring in 0..=MAX_RINGS {
        let r = R_CENTER + (ring as i32) * (RING_WIDTH + RING_GAP);
        canvas.draw_circle(cx, cy, r, chart_separator_color());
    }

    segments
}

fn draw_sub_rings(canvas: &anyui::Canvas, tree: &DirTree, parent_idx: usize,
                  cx: i32, cy: i32, parent_angle: u32, parent_span: u32,
                  r_inner: i32, depth: u32, segments: &mut Vec<ChartSegment>) {
    if depth > MAX_RINGS { return; }
    if parent_span < 150 { return; }

    let parent = tree.node(parent_idx);
    let parent_size = parent.total_size;
    if parent_size == 0 { return; }

    let r_outer = r_inner + RING_WIDTH;
    let children = tree.children_of(parent_idx);
    let mut sub_angle = parent_angle;

    for (i, &child_idx) in children.iter().enumerate() {
        let child = tree.node(child_idx);
        if child.total_size == 0 { continue; }
        let span = (child.total_size * parent_span as u64 / parent_size) as u32;
        if span < 80 { sub_angle += span; continue; }

        let shade = 100u32.saturating_sub((depth - 1) * 15);
        let color = darken(PALETTE[(i + depth as usize * 3) % PALETTE.len()], shade);

        draw_arc(canvas, cx, cy, r_inner, r_outer, sub_angle, sub_angle + span, color);
        segments.push(ChartSegment {
            r_inner, r_outer, angle_start: sub_angle, angle_end: sub_angle + span, node_idx: child_idx,
        });

        if child.is_dir && depth < MAX_RINGS && span > 300 {
            draw_sub_rings(canvas, tree, child_idx, cx, cy,
                           sub_angle, span, r_outer + RING_GAP, depth + 1, segments);
        }
        sub_angle += span;
    }
}

/// Hit-test: given mouse (mx, my) on chart, find which segment the cursor is over.
fn hit_test_chart(segments: &[ChartSegment], mx: i32, my: i32,
                  chart_w: u32, chart_h: u32) -> Option<usize> {
    let cx = chart_w as i32 / 2;
    let cy = chart_h as i32 / 2;
    let dx = (mx - cx) as i64;
    let dy = (my - cy) as i64;
    let dist_sq = dx * dx + dy * dy;
    let angle = iatan2(dx as i32, -(dy as i32));

    // Check segments in reverse order (deepest ring first = drawn last)
    for seg in segments.iter().rev() {
        let ri_sq = (seg.r_inner as i64) * (seg.r_inner as i64);
        let ro_sq = (seg.r_outer as i64) * (seg.r_outer as i64);
        if dist_sq >= ri_sq && dist_sq <= ro_sq
            && angle_in_range(angle, seg.angle_start, seg.angle_end)
        {
            return Some(seg.node_idx);
        }
    }
    None
}

// ── App modes ───────────────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum Mode { SelectMount, Scanning, Browsing }

const MAX_HISTORY: usize = 64;

// ── App state ───────────────────────────────────────────────────────────────

#[allow(dead_code)]
struct AppState {
    mode: Mode,
    mounts: Vec<MountInfo>,
    current_node: usize,         // current view node in tree
    path_history: Vec<usize>,    // node index history
    tree: DirTree,               // cached full tree
    chart_segments: Vec<ChartSegment>, // for tooltip hit-testing
    selected_row: i32,
    last_tooltip_node: i32,      // avoid redundant tooltip updates

    // UI
    win: anyui::Window,
    toolbar: anyui::Toolbar,
    path_panel: anyui::View,
    info_panel: anyui::View,
    status_bar: anyui::View,
    btn_back: anyui::IconButton,
    btn_up: anyui::IconButton,
    btn_refresh: anyui::IconButton,
    btn_home: anyui::IconButton,
    lbl_path: anyui::Label,
    lbl_info: anyui::Label,
    status_label: anyui::Label,
    grid: anyui::DataGrid,
    mount_grid: anyui::DataGrid,
    chart: anyui::Canvas,
    split: anyui::SplitView,
    progress: anyui::ProgressBar,
    theme_timer: u32,
    last_theme_light: bool,
}

anyos_std::global_app_state!(AppState);

fn apply_theme() {
    let a = app();
    let tc = anyui::theme::colors();
    a.toolbar.set_color(tc.toolbar_bg);
    a.path_panel.set_color(tc.window_bg);
    a.lbl_path.set_text_color(tc.text);
    a.info_panel.set_color(tc.card_bg);
    a.lbl_info.set_text_color(tc.text_secondary);
    a.status_bar.set_color(tc.toolbar_bg);
    a.status_label.set_text_color(tc.text);
    a.progress.set_color(tc.accent);
    if a.mode == Mode::Browsing {
        let cw = a.chart.get_stride().max(450);
        let ch = a.chart.get_height().max(450);
        a.chart_segments = draw_ring_chart(&a.chart, &a.tree, a.current_node, a.selected_row, cw, ch);
    } else {
        a.chart.clear(chart_background());
    }
}

fn ensure_theme_timer() {
    let a = app();
    if a.theme_timer != 0 {
        return;
    }
    a.theme_timer = anyui::set_timer(500, || {
        let a = app();
        let is_light = anyui::theme::is_light();
        if is_light == a.last_theme_light {
            return;
        }
        a.last_theme_light = is_light;
        apply_theme();
    });
}

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

    let count = a.mounts.len();
    a.mount_grid.set_row_count(count as u32);
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

// ── Full volume scan (builds cached tree) ───────────────────────────────────

fn start_full_scan(mount_path: &str) {
    let a = app();
    a.mode = Mode::Scanning;
    a.mount_grid.set_visible(false);
    a.split.set_visible(false);
    a.progress.set_visible(true);
    a.progress.set_state(10);
    a.btn_back.set_enabled(false);
    a.btn_up.set_enabled(false);
    a.lbl_path.set_text(&format!("Analyzing {}...", mount_path));
    a.lbl_info.set_text("Building directory tree — this may take a moment...");

    // Build complete tree
    let tree = DirTree::build(mount_path, &a.status_label, &a.progress);
    a.progress.set_state(100);

    a.tree = tree;
    a.current_node = a.tree.root;
    a.path_history.clear();
    a.selected_row = -1;

    show_view();
}

// ── Show current view from cached tree ──────────────────────────────────────

fn show_view() {
    let a = app();
    a.mode = Mode::Browsing;
    a.mount_grid.set_visible(false);
    a.split.set_visible(true);
    a.progress.set_visible(false);

    let cur = a.current_node;
    let node = a.tree.node(cur);
    a.lbl_path.set_text(&node.path);

    a.btn_back.set_enabled(!a.path_history.is_empty());
    // "Up" if not at root
    let is_root = cur == a.tree.root;
    a.btn_up.set_enabled(!is_root);

    // Summary
    let total = node.total_size;
    a.lbl_info.set_text(&format!(
        "Total: {} — {} files, {} dirs",
        format_size(total), node.file_count, node.dir_count,
    ));

    // Fill DataGrid from children
    let children = a.tree.children_of(cur);
    let count = children.len();
    a.grid.set_row_count(count as u32);

    let mut text_colors = Vec::with_capacity(count * 5);

    for (i, &child_idx) in children.iter().enumerate() {
        let child = a.tree.node(child_idx);
        let row = i as u32;
        let pct = if total > 0 { child.total_size * 100 / total } else { 0 };

        if child.is_dir {
            a.grid.set_cell(row, 0, &format!("{}/", child.name));
        } else {
            a.grid.set_cell(row, 0, &child.name);
        }
        a.grid.set_cell(row, 1, &format_size(child.total_size));
        a.grid.set_cell(row, 2, &format_percent(child.total_size, total));
        if child.is_dir {
            a.grid.set_cell(row, 3, &format_items(child.file_count, child.dir_count));
        } else {
            a.grid.set_cell(row, 3, "1 file");
        }
        a.grid.set_cell(row, 4, "");

        let color = usage_heat_color(pct);
        for _ in 0..5 { text_colors.push(color); }
    }
    a.grid.set_cell_colors(&text_colors);

    // Draw chart
    let cw = a.chart.get_stride();
    let ch = a.chart.get_height();
    let chart_w = if cw > 0 { cw } else { 450 };
    let chart_h = if ch > 0 { ch } else { 450 };
    a.chart_segments = draw_ring_chart(&a.chart, &a.tree, cur, a.selected_row, chart_w, chart_h);
    a.last_tooltip_node = -1;

    a.status_label.set_text(&format!("{} items — {}", count, format_size(total)));
}

// ── Navigation (instant, from cache) ────────────────────────────────────────

fn navigate_into(grid_row: u32) {
    let a = app();
    let children = a.tree.children_of(a.current_node);
    let idx = grid_row as usize;
    if idx >= children.len() { return; }
    let child_idx = children[idx];
    if !a.tree.node(child_idx).is_dir { return; }

    if a.path_history.len() >= MAX_HISTORY { a.path_history.remove(0); }
    a.path_history.push(a.current_node);

    a.current_node = child_idx;
    a.selected_row = -1;
    show_view();
}

fn navigate_up() {
    let a = app();
    if a.current_node == a.tree.root { return; }

    // Find parent by searching the tree (walk from root)
    let target = a.current_node;
    if let Some(parent_idx) = find_parent(&a.tree, a.tree.root, target) {
        if a.path_history.len() >= MAX_HISTORY { a.path_history.remove(0); }
        a.path_history.push(a.current_node);
        a.current_node = parent_idx;
        a.selected_row = -1;
        show_view();
    }
}

fn find_parent(tree: &DirTree, node: usize, target: usize) -> Option<usize> {
    for &child in tree.children_of(node) {
        if child == target { return Some(node); }
        if tree.node(child).is_dir {
            if let Some(p) = find_parent(tree, child, target) {
                return Some(p);
            }
        }
    }
    None
}

fn navigate_back() {
    let a = app();
    if a.path_history.is_empty() { return; }
    let prev = a.path_history.pop().unwrap();
    a.current_node = prev;
    a.selected_row = -1;
    show_view();
}

// ── Chart tooltip helper ────────────────────────────────────────────────────

fn update_chart_tooltip(mx: i32, my: i32) {
    let a = app();
    if a.mode != Mode::Browsing { return; }
    let cw = a.chart.get_stride().max(450);
    let ch = a.chart.get_height().max(450);
    if let Some(node_idx) = hit_test_chart(&a.chart_segments, mx, my, cw, ch) {
        if a.last_tooltip_node == node_idx as i32 { return; }
        a.last_tooltip_node = node_idx as i32;
        let node = a.tree.node(node_idx);
        let parent_total = a.tree.node(a.current_node).total_size;
        let tip = format!("{} — {} ({})",
            node.path, format_size(node.total_size),
            format_percent(node.total_size, parent_total));
        a.chart.set_tooltip(&tip);
        a.status_label.set_text(&tip);
    } else {
        if a.last_tooltip_node != -1 {
            a.last_tooltip_node = -1;
            a.chart.set_tooltip("");
            let children = a.tree.children_of(a.current_node);
            let total = a.tree.node(a.current_node).total_size;
            a.status_label.set_text(&format!(
                "{} items — {}", children.len(), format_size(total)));
        }
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() { return; }
    init_trig();

    let win = anyui::Window::new("Disk Usage", -1, -1, WIN_W, WIN_H);
    let tc = anyui::theme::colors();

    // Toolbar
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(WIN_W, 36);
    toolbar.set_color(tc.toolbar_bg);
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

    // Path label
    let path_panel = anyui::View::new();
    path_panel.set_dock(anyui::DOCK_TOP);
    path_panel.set_size(WIN_W, 28);
    path_panel.set_color(tc.window_bg);
    let lbl_path = anyui::Label::new("");
    lbl_path.set_dock(anyui::DOCK_FILL);
    lbl_path.set_text_color(tc.text);
    lbl_path.set_font_size(13);
    path_panel.add(&lbl_path);
    win.add(&path_panel);

    // Info label
    let info_panel = anyui::View::new();
    info_panel.set_dock(anyui::DOCK_TOP);
    info_panel.set_size(WIN_W, 22);
    info_panel.set_color(tc.card_bg);
    let lbl_info = anyui::Label::new("");
    lbl_info.set_dock(anyui::DOCK_FILL);
    lbl_info.set_text_color(tc.text_secondary);
    lbl_info.set_font_size(11);
    info_panel.add(&lbl_info);
    win.add(&info_panel);

    // Status bar
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 24);
    status_bar.set_color(tc.toolbar_bg);
    let status_label = anyui::Label::new("Ready");
    status_label.set_dock(anyui::DOCK_FILL);
    status_label.set_text_color(tc.text);
    status_label.set_font_size(11);
    status_bar.add(&status_label);
    win.add(&status_bar);

    // Progress bar
    let progress = anyui::ProgressBar::new(0);
    progress.set_dock(anyui::DOCK_TOP);
    progress.set_size(WIN_W, 6);
    progress.set_color(tc.accent);
    progress.set_visible(false);
    win.add(&progress);

    // Mount grid
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

    // SplitView
    let split = anyui::SplitView::new();
    split.set_dock(anyui::DOCK_FILL);
    split.set_orientation(1); // 1=horizontal = left/right
    split.set_split_ratio(55);
    split.set_resizable(true);
    split.set_visible(false);

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

    let chart = anyui::Canvas::new(450, 450);
    chart.set_dock(anyui::DOCK_FILL);
    chart.set_interactive(true);
    chart.clear(chart_background());
    split.add(&chart);

    win.add(&split);

    // Init state
    unsafe {
        APP = Some(AppState {
            mode: Mode::SelectMount,
            mounts: Vec::new(),
            current_node: 0,
            path_history: Vec::new(),
            tree: DirTree::new(),
            chart_segments: Vec::new(),
            selected_row: -1,
            last_tooltip_node: -1,
            win, toolbar, path_panel, info_panel, status_bar, btn_back, btn_up, btn_refresh, btn_home,
            lbl_path, lbl_info, status_label,
            grid, mount_grid, chart, split, progress,
            theme_timer: 0,
            last_theme_light: anyui::theme::is_light(),
        });
    }

    apply_theme();
    ensure_theme_timer();
    show_mount_selection();

    // ── Callbacks ───────────────────────────────────────────────────────

    // Mount: double-click → full scan
    app().mount_grid.on_submit(|evt| {
        let idx = evt.index as usize;
        let a = app();
        if idx < a.mounts.len() {
            let path = a.mounts[idx].path.clone();
            start_full_scan(&path);
        }
    });

    // Grid: selection → highlight chart
    app().grid.on_selection_changed(|evt| {
        let a = app();
        a.selected_row = evt.index as i32;
        let cur = a.current_node;
        let cw = a.chart.get_stride();
        let ch = a.chart.get_height();
        let chart_w = if cw > 0 { cw } else { 450 };
        let chart_h = if ch > 0 { ch } else { 450 };
        a.chart_segments = draw_ring_chart(&a.chart, &a.tree, cur, a.selected_row, chart_w, chart_h);
    });

    // Grid: double-click → navigate
    app().grid.on_submit(|evt| { navigate_into(evt.index); });

    // Chart: mouse interaction → tooltip + status bar info
    app().chart.on_mouse_move(|mx, my| { update_chart_tooltip(mx, my); });
    app().chart.on_draw(|mx, my, _btn| { update_chart_tooltip(mx, my); });
    app().chart.on_mouse_down(|mx, my, _btn| { update_chart_tooltip(mx, my); });

    // Chart: click on segment → navigate into that directory
    app().chart.on_click(|_| {
        let a = app();
        let (mx, my, _btn) = a.chart.get_mouse();
        let cw = a.chart.get_stride().max(450);
        let ch = a.chart.get_height().max(450);
        if let Some(node_idx) = hit_test_chart(&a.chart_segments, mx, my, cw, ch) {
            let node = a.tree.node(node_idx);
            if node.is_dir {
                if a.path_history.len() >= MAX_HISTORY { a.path_history.remove(0); }
                a.path_history.push(a.current_node);
                a.current_node = node_idx;
                a.selected_row = -1;
                show_view();
            }
        }
    });

    // Toolbar
    app().btn_back.on_click(|_| { navigate_back(); });
    app().btn_up.on_click(|_| { navigate_up(); });

    app().btn_refresh.on_click(|_| {
        let a = app();
        if a.mode == Mode::Browsing {
            let path = a.tree.node(a.tree.root).path.clone();
            start_full_scan(&path);
        } else {
            show_mount_selection();
        }
    });

    btn_home.on_click(|_| { show_mount_selection(); });

    app().win.on_close(|_| { anyui::quit(); });

    anyui::run();
}
