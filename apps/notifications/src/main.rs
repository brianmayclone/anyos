#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::i18n;
use libanyui_client as anyui;
use anyui::Widget;

anyos_std::entry!(main);

// ── Constants ────────────────────────────────────────────────────────────────

const HISTORY_FILE: &str = "/tmp/.notification_history.json";
const POLL_INTERVAL_MS: u32 = 2000;
const NUM_COLS: usize = 3;
const MAX_PREVIEW_LEN: usize = 120;

// ── Data model ───────────────────────────────────────────────────────────────

struct NotifEntry {
    title: String,
    message: String,
    time: String,
}

struct AppState {
    win: anyui::Window,
    grid: anyui::DataGrid,
    status_label: anyui::Label,
    detail_label: anyui::Label,
    detail_panel: anyui::View,
    detail_visible: bool,
    entries: Vec<NotifEntry>,
    last_count: usize,
}

anyos_std::global_app_state!(AppState);

// ── File reading ────────────────────────────────────────────────────────────

fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX { return None; }
    let mut content = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        content.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);
    Some(content)
}

// ── Load notification history ───────────────────────────────────────────────

fn load_notifications() -> Vec<NotifEntry> {
    let data = match read_file(HISTORY_FILE) {
        Some(d) => d,
        None => return Vec::new(),
    };
    let text = match core::str::from_utf8(&data) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let json = match anyos_std::json::Value::parse(text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    if let Some(arr) = json["notifications"].as_array() {
        for item in arr {
            let title = item["title"].as_str().unwrap_or("");
            let message = item["message"].as_str().unwrap_or("");
            let time = item["time"].as_str().unwrap_or("");
            if !title.is_empty() {
                entries.push(NotifEntry {
                    title: String::from(title),
                    message: String::from(message),
                    time: String::from(time),
                });
            }
        }
    }
    entries
}

// ── Grid population ─────────────────────────────────────────────────────────

fn populate_grid(s: &AppState) {
    let n = s.entries.len();
    if n == 0 {
        s.grid.set_row_count(0);
        s.grid.set_cell_colors(&[]);
        s.status_label.set_text(i18n::t("No notifications"));
        return;
    }

    s.grid.set_row_count(n as u32);

    let mut rows: Vec<Vec<&str>> = Vec::new();
    let mut preview_strs: Vec<String> = Vec::new();

    for entry in &s.entries {
        let preview = make_preview(&entry.message);
        preview_strs.push(preview);
    }

    for i in 0..n {
        let mut row = Vec::new();
        row.push(s.entries[i].title.as_str());
        row.push(preview_strs[i].as_str());
        row.push(s.entries[i].time.as_str());
        rows.push(row);
    }
    s.grid.set_data(&rows);

    // Color the timestamps with dim theme colors
    let tc = anyui::theme::colors();
    let total = n * NUM_COLS;
    let mut colors: Vec<u32> = Vec::new();
    colors.resize(total, 0);
    for i in 0..n {
        colors[i * NUM_COLS + 2] = tc.text_secondary; // timestamp dim
    }
    s.grid.set_cell_colors(&colors);

    update_status(s);
}

fn make_preview(text: &str) -> String {
    let first_line = text.split('\n').next().unwrap_or(text);
    let trimmed = first_line.trim();
    if trimmed.len() > MAX_PREVIEW_LEN {
        let mut s = String::from(&trimmed[..MAX_PREVIEW_LEN]);
        s.push_str("...");
        s
    } else {
        String::from(trimmed)
    }
}

fn update_status(s: &AppState) {
    let count = s.entries.len();
    let status = if count == 1 {
        anyos_std::format!("1 {}", i18n::t("notification"))
    } else {
        anyos_std::format!("{} {}", count, i18n::t("notifications"))
    };
    s.status_label.set_text(&status);
}

// ── Actions ──────────────────────────────────────────────────────────────────

fn show_detail() {
    let s = app();
    let row = s.grid.selected_row();
    if row == u32::MAX { return; }
    let row = row as usize;
    if row >= s.entries.len() { return; }

    let entry = &s.entries[row];
    let detail = anyos_std::format!("{}\n\n{}\n\n{}", entry.title, entry.message, entry.time);
    s.detail_label.set_text(&detail);
    s.detail_panel.set_visible(true);
    s.detail_visible = true;
}

fn hide_detail() {
    let s = app();
    s.detail_panel.set_visible(false);
    s.detail_visible = false;
}

fn clear_all() {
    let s = app();
    if s.entries.is_empty() { return; }
    s.entries.clear();
    // Write empty history
    let mut root = anyos_std::json::Value::new_object();
    root.set("notifications", anyos_std::json::Value::new_array());
    let json = root.to_json_string_pretty();
    let _ = anyos_std::fs::write_bytes(HISTORY_FILE, json.as_bytes());
    populate_grid(s);
    hide_detail();
    s.status_label.set_text(i18n::t("All notifications cleared"));
}

fn delete_selected() {
    let s = app();
    let row = s.grid.selected_row();
    if row == u32::MAX { return; }
    let row = row as usize;
    if row >= s.entries.len() { return; }

    s.entries.remove(row);
    save_history(s);
    populate_grid(s);
    hide_detail();
}

fn save_history(s: &AppState) {
    let mut root = anyos_std::json::Value::new_object();
    let mut arr = anyos_std::json::Value::new_array();
    for entry in &s.entries {
        let mut obj = anyos_std::json::Value::new_object();
        obj.set("title", entry.title.as_str().into());
        obj.set("message", entry.message.as_str().into());
        obj.set("time", entry.time.as_str().into());
        obj.set("sender", (0i64).into());
        arr.push(obj);
    }
    root.set("notifications", arr);
    let json = root.to_json_string_pretty();
    let _ = anyos_std::fs::write_bytes(HISTORY_FILE, json.as_bytes());
}

fn refresh() {
    let s = app();
    s.entries = load_notifications();
    populate_grid(s);
    hide_detail();
}

fn poll_for_new() {
    let s = app();
    let new_entries = load_notifications();
    if new_entries.len() != s.last_count {
        s.last_count = new_entries.len();
        s.entries = new_entries;
        populate_grid(s);
    }
}

// ── Keyboard handler ────────────────────────────────────────────────────────

fn handle_key(ke: &anyui::KeyEvent) {
    if ke.keycode == anyui::KEY_ENTER {
        show_detail();
        return;
    }
    if ke.keycode == anyui::KEY_DELETE {
        delete_selected();
        return;
    }
    if ke.keycode == anyui::KEY_ESCAPE {
        let s = app();
        if s.detail_visible {
            hide_detail();
        } else {
            anyui::quit();
        }
        return;
    }
    if ke.keycode == anyui::KEY_F5 {
        refresh();
        return;
    }
}

// ── Main entry ───────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() {
        anyos_std::println!("notifications: failed to load libanyui.so");
        return;
    }
    i18n::init();

    // Load history
    let entries = load_notifications();
    let last_count = entries.len();

    // Create window
    let tc = anyui::theme::colors();
    let win = anyui::Window::new(i18n::t("Notifications"), -1, -1, 700, 450);

    // ── Menu bar ──
    let mut mb = anyui::MenuBarBuilder::new()
        .menu(i18n::t("File"))
            .item(1, i18n::t("Quit"), 0)
        .end_menu()
        .menu(i18n::t("Edit"))
            .item(10, i18n::t("Delete Selected"), 0)
            .separator()
            .item(11, i18n::t("Clear All"), 0)
        .end_menu()
        .menu(i18n::t("View"))
            .item(20, i18n::t("Refresh"), 0)
        .end_menu();
    let menu_data = mb.build();
    let menu = anyui::MenuBar::set(win.id(), menu_data);

    // ── Toolbar ──
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(700, 46);
    toolbar.set_color(tc.toolbar_bg);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_refresh = toolbar.add_icon_button("");
    btn_refresh.set_size(34, 34);
    btn_refresh.set_system_icon("refresh", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_refresh.set_tooltip(i18n::t("Refresh"));

    let btn_delete = toolbar.add_icon_button("");
    btn_delete.set_size(34, 34);
    btn_delete.set_system_icon("trash", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_delete.set_tooltip(i18n::t("Delete Selected"));

    toolbar.add_separator();

    let btn_clear = toolbar.add_icon_button("");
    btn_clear.set_size(34, 34);
    btn_clear.set_system_icon("trash-x", anyui::IconType::Outline, 0xFFCCCCCC, 24);
    btn_clear.set_tooltip(i18n::t("Clear All"));

    win.add(&toolbar);

    // ── Status bar ──
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(700, 24);
    status_bar.set_color(tc.sidebar_bg);

    let status_label = anyui::Label::new("");
    status_label.set_position(8, 4);
    status_label.set_text_color(tc.text_secondary);
    status_label.set_font_size(12);
    status_bar.add(&status_label);

    win.add(&status_bar);

    // ── Detail panel (DOCK_BOTTOM, above status bar) ──
    let detail_panel = anyui::View::new();
    detail_panel.set_dock(anyui::DOCK_BOTTOM);
    detail_panel.set_size(700, 100);
    detail_panel.set_color(tc.card_bg);
    detail_panel.set_visible(false);

    let detail_label = anyui::Label::new("");
    detail_label.set_position(12, 8);
    detail_label.set_text_color(tc.text);
    detail_label.set_font_size(13);
    detail_label.set_size(670, 80);
    detail_panel.add(&detail_label);

    let btn_close_detail = anyui::Button::new("X");
    btn_close_detail.set_position(668, 4);
    btn_close_detail.set_size(24, 24);
    detail_panel.add(&btn_close_detail);

    win.add(&detail_panel);

    // ── DataGrid ──
    let grid = anyui::DataGrid::new(680, 380);
    grid.set_dock(anyui::DOCK_FILL);
    grid.set_row_height(22);
    grid.set_header_height(24);
    grid.set_columns(&[
        anyui::ColumnDef::new(i18n::t("Title")).width(180),
        anyui::ColumnDef::new(i18n::t("Message")).width(360),
        anyui::ColumnDef::new(i18n::t("Time")).width(140),
    ]);

    // Context menu
    let ctx_items = anyos_std::format!("{}|-|{}|{}",
        i18n::t("Show Details"),
        i18n::t("Delete"), i18n::t("Clear All"));
    let ctx_menu = anyui::ContextMenu::new(&ctx_items);
    grid.set_context_menu(&ctx_menu);

    win.add(&grid);

    // ── Initialize global state ──
    unsafe {
        APP = Some(AppState {
            win,
            grid,
            status_label,
            detail_label,
            detail_panel,
            detail_visible: false,
            entries,
            last_count,
        });
    }

    // Populate grid
    let s = app();
    populate_grid(s);

    // ── Register callbacks ──
    btn_refresh.on_click(|_| { refresh(); });
    btn_delete.on_click(|_| { delete_selected(); });
    btn_clear.on_click(|_| { clear_all(); });
    btn_close_detail.on_click(|_| { hide_detail(); });

    // Context menu
    ctx_menu.on_item_click(|e| {
        match e.index {
            0 => show_detail(),
            2 => delete_selected(),
            3 => clear_all(),
            _ => {}
        }
    });

    // Menu bar
    menu.on_item(move |e| {
        match e.item_id {
            1 => anyui::quit(),
            10 => delete_selected(),
            11 => clear_all(),
            20 => refresh(),
            _ => {}
        }
    });

    // Double-click to show detail
    app().grid.on_submit(|_| {
        show_detail();
    });

    // Window close
    win.on_close(|_| {
        anyui::quit();
    });

    // Keyboard shortcuts
    win.on_key_down(|ke| { handle_key(ke); });

    // ── Start polling timer for new notifications ──
    anyui::set_timer(POLL_INTERVAL_MS, || {
        poll_for_new();
    });

    // ── Run event loop ──
    anyui::run();
}
