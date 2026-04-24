#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libami::{AmiClient, AmiItem, AmiValue};
use libanyui_client as ui;

anyos_std::entry!(main);

const WIN_W: u32 = 1040;
const WIN_H: u32 = 660;
const TOOLBAR_H: u32 = 38;
const STATUS_H: u32 = 28;
const DETAIL_H: u32 = 132;
const SUMMARY_H: u32 = 54;
const SIDEBAR_W: u32 = 220;
const REFRESH_MS: u32 = 750;
const HIGHLIGHT_MS: u32 = 850;
const GRID_COLS: usize = 5;
const MINI_ICON_SIZE: u32 = 12;
const MINI_ICON_PIXELS: usize = 144;

struct HighlightEntry {
    key: String,
    until_ms: u32,
}

struct TreeNode {
    id: u32,
    prefix: String,
}

struct App {
    client: Option<AmiClient>,
    items: Vec<AmiItem>,
    visible: Vec<usize>,
    highlights: Vec<HighlightEntry>,
    tree_nodes: Vec<TreeNode>,
    namespace_filter: String,
    search_text: String,
    selected_key: String,
    grid: ui::DataGrid,
    tree: ui::TreeView,
    detail: ui::Label,
    status: ui::Label,
    heading: ui::Label,
    connection: ui::Label,
    summary_total: ui::Label,
    summary_visible: ui::Label,
    summary_changed: ui::Label,
}

anyos_std::global_app_state!(App);

fn main() {
    if !ui::init() {
        anyos_std::println!("amiconsole: failed to load libanyui.so");
        return;
    }

    let tc = ui::theme::colors();
    let win = ui::Window::new("AMI Console", -1, -1, WIN_W, WIN_H);

    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(WIN_W, TOOLBAR_H);
    toolbar.set_padding(8, 5, 8, 5);
    win.add(&toolbar);

    let btn_refresh = toolbar.add_icon_button("");
    btn_refresh.set_icon(ui::ICON_REFRESH);
    btn_refresh.set_size(32, 28);
    btn_refresh.set_tooltip("Refresh AMI snapshot");

    toolbar.add_separator();

    let title = toolbar.add_label("AMI Console");
    title.set_font_size(14);
    title.set_text_color(tc.text);
    title.set_size(136, 26);

    toolbar.add_separator();

    let search = ui::SearchField::new();
    search.set_placeholder("Search keys, values, types...");
    search.set_size(300, 28);
    toolbar.add(&search);

    let connection = toolbar.add_label("Connecting...");
    connection.set_font_size(12);
    connection.set_text_color(tc.warning);
    connection.set_size(132, 22);

    let summary = ui::View::new();
    summary.set_dock(ui::DOCK_TOP);
    summary.set_size(WIN_W, SUMMARY_H);
    summary.set_color(tc.window_bg);
    summary.set_padding(14, 8, 14, 8);
    win.add(&summary);

    let summary_total = ui::Label::new("Keys\n-");
    summary_total.set_position(14, 8);
    summary_total.set_size(132, 36);
    summary_total.set_font_size(12);
    summary_total.set_text_color(tc.text);
    summary.add(&summary_total);

    let summary_visible = ui::Label::new("Visible\n-");
    summary_visible.set_position(160, 8);
    summary_visible.set_size(132, 36);
    summary_visible.set_font_size(12);
    summary_visible.set_text_color(tc.text);
    summary.add(&summary_visible);

    let summary_changed = ui::Label::new("Changes\n-");
    summary_changed.set_position(306, 8);
    summary_changed.set_size(160, 36);
    summary_changed.set_font_size(12);
    summary_changed.set_text_color(tc.text);
    summary.add(&summary_changed);

    let summary_hint = ui::Label::new("Live runtime state from amid");
    summary_hint.set_position(500, 16);
    summary_hint.set_size(280, 20);
    summary_hint.set_font_size(12);
    summary_hint.set_text_color(tc.text_secondary);
    summary.add(&summary_hint);

    let sep_summary = ui::Divider::new();
    sep_summary.set_dock(ui::DOCK_TOP);
    sep_summary.set_size(WIN_W, 1);
    win.add(&sep_summary);

    let status_bar = ui::View::new();
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, STATUS_H);
    status_bar.set_color(tc.toolbar_bg);
    status_bar.set_padding(10, 6, 10, 6);
    win.add(&status_bar);

    let status = ui::Label::new("Waiting for amid...");
    status.set_position(10, 6);
    status.set_size(WIN_W - 20, 16);
    status.set_text_color(tc.text_secondary);
    status.set_font_size(12);
    status_bar.add(&status);

    let detail_view = ui::View::new();
    detail_view.set_dock(ui::DOCK_BOTTOM);
    detail_view.set_size(WIN_W, DETAIL_H);
    detail_view.set_color(tc.editor_bg);
    detail_view.set_padding(14, 10, 14, 10);
    win.add(&detail_view);

    let heading = ui::Label::new("All Keys");
    heading.set_position(14, 10);
    heading.set_size(320, 18);
    heading.set_text_color(tc.text_secondary);
    heading.set_font_size(11);
    detail_view.add(&heading);

    let detail = ui::Label::new("Select a key to inspect its live AMI value.");
    detail.set_position(14, 34);
    detail.set_size(WIN_W - 28, DETAIL_H - 44);
    detail.set_text_color(tc.text);
    detail.set_font_size(12);
    detail.set_font(4);
    detail_view.add(&detail);

    let sep_bottom = ui::Divider::new();
    sep_bottom.set_dock(ui::DOCK_BOTTOM);
    sep_bottom.set_size(WIN_W, 1);
    win.add(&sep_bottom);

    let sidebar = ui::View::new();
    sidebar.set_dock(ui::DOCK_LEFT);
    sidebar.set_size(
        SIDEBAR_W,
        WIN_H - TOOLBAR_H - SUMMARY_H - DETAIL_H - STATUS_H - 1,
    );
    sidebar.set_color(tc.sidebar_bg);
    sidebar.set_padding(8, 8, 8, 8);
    win.add(&sidebar);

    let sidebar_title = ui::Label::new("Namespaces");
    sidebar_title.set_dock(ui::DOCK_TOP);
    sidebar_title.set_size(SIDEBAR_W - 20, 22);
    sidebar_title.set_text_color(tc.text_secondary);
    sidebar_title.set_font_size(11);
    sidebar.add(&sidebar_title);

    let tree = ui::TreeView::new(SIDEBAR_W - 16, WIN_H);
    tree.set_dock(ui::DOCK_FILL);
    tree.set_size(
        SIDEBAR_W - 16,
        WIN_H - TOOLBAR_H - SUMMARY_H - DETAIL_H - STATUS_H - 40,
    );
    tree.set_indent_width(14);
    tree.set_row_height(22);
    sidebar.add(&tree);

    let sep_side = ui::Divider::new();
    sep_side.set_dock(ui::DOCK_LEFT);
    sep_side.set_size(1, WIN_H);
    win.add(&sep_side);

    let grid = ui::DataGrid::new(WIN_W - SIDEBAR_W, WIN_H);
    grid.set_dock(ui::DOCK_FILL);
    grid.set_row_height(22);
    grid.set_header_height(26);
    grid.set_font_size(12);
    let cols = alloc::vec![
        ui::ColumnDef::new("Key").width(318),
        ui::ColumnDef::new("Value").width(300),
        ui::ColumnDef::new("Type").width(70),
        ui::ColumnDef::new("Version")
            .width(72)
            .align(ui::ALIGN_RIGHT)
            .numeric(),
        ui::ColumnDef::new("Updated")
            .width(90)
            .align(ui::ALIGN_RIGHT)
            .numeric(),
    ];
    grid.set_columns(&cols);
    win.add(&grid);

    unsafe {
        APP = Some(App {
            client: None,
            items: Vec::new(),
            visible: Vec::new(),
            highlights: Vec::new(),
            tree_nodes: Vec::new(),
            namespace_filter: String::new(),
            search_text: String::new(),
            selected_key: String::new(),
            grid,
            tree,
            detail,
            status,
            heading,
            connection,
            summary_total,
            summary_visible,
            summary_changed,
        });
    }

    tree.on_selection_changed(|e| {
        if let Some(prefix) = tree_prefix_for_node(e.index) {
            let a = app();
            a.namespace_filter = prefix;
            a.heading.set_text(&namespace_heading(&a.namespace_filter));
            refresh_visible(a);
            restore_selection(a);
        }
    });

    app().grid.on_selection_changed(|e| {
        show_detail(e.index);
    });

    search.on_text_changed(|e| {
        let a = app();
        a.search_text = to_lower(&e.text());
        refresh_visible(a);
        restore_selection(a);
    });

    btn_refresh.on_click(|_| {
        reload_snapshot();
    });

    ui::set_timer(REFRESH_MS, || {
        reload_snapshot();
    });

    reload_snapshot();
    ui::run();
}

fn reload_snapshot() {
    let a = app();
    let tc = ui::theme::colors();
    let now = anyos_std::sys::uptime_ms();
    let previous_scroll = a.grid.scroll_offset();

    if a.client.is_none() {
        match AmiClient::connect("ami-console") {
            Ok(client) => {
                a.client = Some(client);
                a.connection.set_text("Live");
                a.connection.set_text_color(tc.success);
            }
            Err(_) => {
                a.connection.set_text("Disconnected");
                a.connection.set_text_color(tc.destructive);
                a.status.set_text("amid is not reachable yet.");
                a.grid.set_row_count(0);
                a.tree.clear();
                a.tree_nodes.clear();
                return;
            }
        }
    }

    let result = {
        let client = a.client.as_mut().unwrap();
        client.list("")
    };

    let new_items = match result {
        Ok(items) => items,
        Err(_) => {
            a.client = None;
            a.connection.set_text("Retrying...");
            a.connection.set_text_color(tc.warning);
            a.status
                .set_text("Lost connection to amid. Will retry automatically.");
            return;
        }
    };

    let changed_count = count_changed_items(&a.items, &new_items);
    mark_highlights(&a.items, &new_items, &mut a.highlights, now);
    a.items = new_items;
    prune_highlights(&mut a.highlights, now);
    rebuild_tree(a);
    refresh_visible(a);
    restore_selection(a);
    restore_scroll(a, previous_scroll);
    update_summary(a, changed_count);
}

fn rebuild_tree(a: &mut App) {
    let prev_prefix = a.namespace_filter.clone();
    a.tree.clear();
    a.tree_nodes.clear();

    let root = a.tree.add_root("All Keys");
    a.tree.set_node_style(root, ui::STYLE_BOLD);
    a.tree_nodes.push(TreeNode {
        id: root,
        prefix: String::new(),
    });

    let roots = collect_roots(&a.items);
    for root_name in &roots {
        if root_name == "svc" {
            let node = a.tree.add_root("Services");
            a.tree.set_node_style(node, ui::STYLE_BOLD);
            a.tree_nodes.push(TreeNode {
                id: node,
                prefix: String::from("svc."),
            });
            let services = collect_service_prefixes(&a.items);
            for service in &services {
                let prefix = format!("svc.{}.", service);
                let child = a.tree.add_child(node, service);
                a.tree_nodes.push(TreeNode { id: child, prefix });
            }
        } else {
            let prefix = format!("{}.", root_name);
            let label = friendly_root_label(root_name);
            let node = a.tree.add_root(&label);
            a.tree_nodes.push(TreeNode { id: node, prefix });
        }
    }

    let selected = find_tree_node_for_prefix(&a.tree_nodes, &prev_prefix).unwrap_or(root);
    a.tree.set_selected(selected);
}

fn refresh_visible(a: &mut App) {
    a.visible.clear();
    for (idx, item) in a.items.iter().enumerate() {
        if !a.namespace_filter.is_empty() && !item.key.starts_with(a.namespace_filter.as_str()) {
            continue;
        }
        if !matches_search(item, &a.search_text) {
            continue;
        }
        a.visible.push(idx);
    }
    populate_grid(a);
}

fn populate_grid(a: &App) {
    let mut text_colors = alloc::vec![0u32; a.visible.len() * GRID_COLS];
    let mut bg_colors = alloc::vec![0u32; a.visible.len() * GRID_COLS];
    let mut minimap_colors = alloc::vec![0u32; a.visible.len()];
    let tc = ui::theme::colors();
    let now = anyos_std::sys::uptime_ms();
    a.grid.set_row_count(a.visible.len() as u32);

    for (row, &idx) in a.visible.iter().enumerate() {
        let item = &a.items[idx];
        let value = ami_value_to_string(&item.value);
        let ty = ami_value_type(&item.value);
        let version_text = format_u64(item.version);
        let updated_text = format!("{} ms", item.updated_at);

        a.grid.set_cell(row as u32, 0, &item.key);
        a.grid.set_cell(row as u32, 1, &value);
        a.grid.set_cell(row as u32, 2, ty);
        a.grid.set_cell(row as u32, 3, &version_text);
        a.grid.set_cell(row as u32, 4, &updated_text);
        set_value_icon(&a.grid, row as u32, &item.value, item.key.as_str(), tc);

        let base = if row % 2 == 0 {
            tc.window_bg
        } else {
            tc.alt_row_bg
        };
        let highlight = highlight_bg_for_key(&a.highlights, &item.key, now, base, tc.accent);
        if highlight != base {
            minimap_colors[row] = tc.accent;
        }
        for col in 0..GRID_COLS {
            bg_colors[row * GRID_COLS + col] = highlight;
        }

        text_colors[row * GRID_COLS + 1] = semantic_value_color(&item.value, item.key.as_str(), tc);
        text_colors[row * GRID_COLS + 2] = tc.text_secondary;
    }

    a.grid.set_cell_colors(&text_colors);
    a.grid.set_cell_bg_colors(&bg_colors);
    a.grid.set_minimap_colors(&minimap_colors);
    a.status.set_text(&format!(
        "{} keys visible, {} total, prefix '{}'",
        a.visible.len(),
        a.items.len(),
        if a.namespace_filter.is_empty() {
            "all"
        } else {
            a.namespace_filter.as_str()
        }
    ));
}

fn restore_scroll(a: &App, previous_scroll: u32) {
    if a.visible.is_empty() {
        a.grid.set_scroll_offset(0);
        return;
    }
    let max_scroll = (a.visible.len() as u32).saturating_sub(1);
    a.grid.set_scroll_offset(previous_scroll.min(max_scroll));
}

fn update_summary(a: &App, changed_count: usize) {
    a.summary_total
        .set_text(&format!("Keys\n{}", a.items.len()));
    a.summary_visible
        .set_text(&format!("Visible\n{}", a.visible.len()));
    a.summary_changed
        .set_text(&format!("Changes\n{}", changed_count));
}

fn restore_selection(a: &mut App) {
    if a.selected_key.is_empty() {
        if !a.visible.is_empty() {
            a.grid.set_selected_row(0);
            show_detail(0);
        } else {
            a.detail.set_text("No AMI keys match the current filter.");
        }
        return;
    }

    for (row, &idx) in a.visible.iter().enumerate() {
        if a.items[idx].key == a.selected_key {
            a.grid.set_selected_row(row as u32);
            show_detail(row as u32);
            return;
        }
    }

    if !a.visible.is_empty() {
        a.grid.set_selected_row(0);
        show_detail(0);
    } else {
        a.detail.set_text("No AMI keys match the current filter.");
    }
}

fn show_detail(row_index: u32) {
    let a = app();
    if (row_index as usize) >= a.visible.len() {
        a.detail.set_text("No AMI keys match the current filter.");
        return;
    }

    let idx = a.visible[row_index as usize];
    let item = &a.items[idx];
    a.selected_key = item.key.clone();
    a.detail.set_text(&format!(
        "Key:      {}\nType:     {}\nValue:    {}\nVersion:  {}\nUpdated:  {} ms\nPrefix:   {}",
        item.key,
        ami_value_type(&item.value),
        ami_value_to_string(&item.value),
        item.version,
        item.updated_at,
        namespace_prefix(&item.key)
    ));
}

fn collect_roots(items: &[AmiItem]) -> Vec<String> {
    let mut roots = Vec::new();
    for item in items {
        let root = first_segment(&item.key);
        if !root.is_empty() {
            insert_sorted_unique(&mut roots, &root);
        }
    }
    roots
}

fn collect_service_prefixes(items: &[AmiItem]) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if !item.key.starts_with("svc.") {
            continue;
        }
        let rest = &item.key[4..];
        if let Some(pos) = rest.find('.') {
            insert_sorted_unique(&mut out, &rest[..pos]);
        }
    }
    out
}

fn tree_prefix_for_node(node_id: u32) -> Option<String> {
    let a = app();
    for node in &a.tree_nodes {
        if node.id == node_id {
            return Some(node.prefix.clone());
        }
    }
    None
}

fn find_tree_node_for_prefix(nodes: &[TreeNode], prefix: &str) -> Option<u32> {
    for node in nodes {
        if node.prefix == prefix {
            return Some(node.id);
        }
    }
    None
}

fn matches_search(item: &AmiItem, search: &str) -> bool {
    if search.is_empty() {
        return true;
    }
    let key = to_lower(&item.key);
    let value = to_lower(&ami_value_to_string(&item.value));
    let ty = to_lower(ami_value_type(&item.value));
    key.contains(search) || value.contains(search) || ty.contains(search)
}

fn ami_value_to_string(value: &AmiValue) -> String {
    match value {
        AmiValue::String(s) => String::from(s.as_str()),
        AmiValue::Int(v) => format!("{}", *v),
        AmiValue::Bool(v) => {
            if *v {
                String::from("true")
            } else {
                String::from("false")
            }
        }
    }
}

fn ami_value_type(value: &AmiValue) -> &'static str {
    match value {
        AmiValue::String(_) => "string",
        AmiValue::Int(_) => "int",
        AmiValue::Bool(_) => "bool",
    }
}

fn namespace_heading(prefix: &str) -> String {
    if prefix.is_empty() {
        String::from("All Keys")
    } else {
        format!("Namespace: {}", prefix)
    }
}

fn namespace_prefix(key: &str) -> String {
    if let Some(pos) = key.rfind('.') {
        String::from(&key[..=pos])
    } else {
        String::new()
    }
}

fn first_segment(key: &str) -> String {
    if let Some(pos) = key.find('.') {
        String::from(&key[..pos])
    } else {
        String::from(key)
    }
}

fn friendly_root_label(root: &str) -> String {
    match root {
        "dns" => String::from("DNS"),
        "svc" => String::from("Services"),
        "net" => String::from("Network"),
        "system" => String::from("System"),
        "update" => String::from("Updates"),
        _ => format!("{}.", root),
    }
}

fn semantic_value_color(value: &AmiValue, key: &str, tc: &ui::theme::ThemeColors) -> u32 {
    match value {
        AmiValue::Bool(true) => tc.success,
        AmiValue::Bool(false) => tc.text_secondary,
        AmiValue::String(s) if key.ends_with(".state") && s == "ready" => tc.success,
        AmiValue::String(s) if key.ends_with(".state") && s == "failed" => tc.destructive,
        AmiValue::String(s) if key.ends_with(".state") && s == "starting" => tc.warning,
        AmiValue::String(s) if key.ends_with(".state") && s == "stopping" => tc.text_secondary,
        _ => tc.text,
    }
}

fn mark_highlights(
    old_items: &[AmiItem],
    new_items: &[AmiItem],
    highlights: &mut Vec<HighlightEntry>,
    now: u32,
) {
    for item in new_items {
        if item_changed(old_items, item) {
            upsert_highlight(highlights, &item.key, now.wrapping_add(HIGHLIGHT_MS));
        }
    }
}

fn count_changed_items(old_items: &[AmiItem], new_items: &[AmiItem]) -> usize {
    let mut count = 0usize;
    for item in new_items {
        if item_changed(old_items, item) {
            count += 1;
        }
    }
    count
}

fn item_changed(old_items: &[AmiItem], new_item: &AmiItem) -> bool {
    for old in old_items {
        if old.key == new_item.key {
            return old.version != new_item.version;
        }
    }
    true
}

fn upsert_highlight(highlights: &mut Vec<HighlightEntry>, key: &str, until_ms: u32) {
    for entry in highlights.iter_mut() {
        if entry.key == key {
            entry.until_ms = until_ms;
            return;
        }
    }
    highlights.push(HighlightEntry {
        key: String::from(key),
        until_ms,
    });
}

fn prune_highlights(highlights: &mut Vec<HighlightEntry>, now: u32) {
    highlights.retain(|entry| entry.until_ms.wrapping_sub(now) < 0x8000_0000);
}

fn highlight_bg_for_key(
    highlights: &[HighlightEntry],
    key: &str,
    now: u32,
    base: u32,
    accent: u32,
) -> u32 {
    for entry in highlights {
        if entry.key == key && entry.until_ms.wrapping_sub(now) < 0x8000_0000 {
            let remain = entry.until_ms.wrapping_sub(now).min(HIGHLIGHT_MS);
            let strength = ((remain as u64) * 120 / (HIGHLIGHT_MS as u64)) as u8;
            return blend_colors(base, ui::theme::lighten(accent, 24), strength);
        }
    }
    base
}

fn set_value_icon(
    grid: &ui::DataGrid,
    row: u32,
    value: &AmiValue,
    key: &str,
    tc: &ui::theme::ThemeColors,
) {
    let color = semantic_value_color(value, key, tc);
    let pixels = mini_status_icon(color, tc.window_bg);
    grid.set_cell_icon(row, 0, &pixels, MINI_ICON_SIZE, MINI_ICON_SIZE);
}

fn mini_status_icon(color: u32, bg: u32) -> [u32; MINI_ICON_PIXELS] {
    let mut pixels = [0u32; MINI_ICON_PIXELS];
    let size = MINI_ICON_SIZE as i32;
    let center = size / 2;
    let radius = 4i32;
    let ring = ui::theme::lighten(color, 30);

    for y in 0..size {
        for x in 0..size {
            let dx = x - center;
            let dy = y - center;
            let dist = dx * dx + dy * dy;
            let idx = (y * size + x) as usize;
            pixels[idx] = if dist <= radius * radius {
                color
            } else if dist <= (radius + 1) * (radius + 1) {
                blend_colors(bg, ring, 150)
            } else {
                0x00000000
            };
        }
    }
    pixels
}

fn blend_colors(base: u32, overlay: u32, strength: u8) -> u32 {
    let a = 0xFF000000;
    let br = ((base >> 16) & 0xFF) as u32;
    let bg = ((base >> 8) & 0xFF) as u32;
    let bb = (base & 0xFF) as u32;
    let or = ((overlay >> 16) & 0xFF) as u32;
    let og = ((overlay >> 8) & 0xFF) as u32;
    let ob = (overlay & 0xFF) as u32;
    let s = strength as u32;
    let inv = 255 - s;
    let r = (br * inv + or * s) / 255;
    let g = (bg * inv + og * s) / 255;
    let b = (bb * inv + ob * s) / 255;
    a | (r << 16) | (g << 8) | b
}

fn insert_sorted_unique(list: &mut Vec<String>, value: &str) {
    for item in list.iter() {
        if item == value {
            return;
        }
    }
    let mut pos = list.len();
    for (i, item) in list.iter().enumerate() {
        if value < item.as_str() {
            pos = i;
            break;
        }
    }
    list.insert(pos, String::from(value));
}

fn to_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        out.push((if b'A' <= b && b <= b'Z' { b + 32 } else { b }) as char);
    }
    out
}

fn format_u64(mut value: u64) -> String {
    if value == 0 {
        return String::from("0");
    }
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    while value > 0 {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    let mut out = String::with_capacity(len);
    while len > 0 {
        len -= 1;
        out.push(buf[len] as char);
    }
    out
}
