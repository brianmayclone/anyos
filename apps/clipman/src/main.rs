// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
#![no_std]
#![no_main]

use anyos_std::i18n;
use anyos_std::String;
use anyos_std::Vec;
use anyui::Widget;
use libanyui_client as anyui;
use libconf_schema::{default_int, manifest, RegistryScope, ServiceSchema};

anyos_std::entry!(main);

// ── Constants ────────────────────────────────────────────────────────────────

const DEFAULT_RETENTION_DAYS: u32 = 3;
const POLL_INTERVAL_MS: u32 = 500;
const MAX_PREVIEW_LEN: usize = 120;
const HISTORY_FILE: &str = ".clipboard_history.json";
const NUM_COLS: usize = 3;

// ── confd schema (per-user settings) ─────────────────────────────────────────

const CLIPMAN_DIRS: &[&str] = &["config"];
const CLIPMAN_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] =
    &[default_int("config/retention_days", DEFAULT_RETENTION_DAYS as i64)];
const CLIPMAN_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const CLIPMAN_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "apps/clipman",
    RegistryScope::User,
    1,
    CLIPMAN_DIRS,
    CLIPMAN_DEFAULTS,
    CLIPMAN_MIGRATIONS,
);
const CLIPMAN_SCHEMA: ServiceSchema<'static> =
    ServiceSchema::new("clipman", &CLIPMAN_MANIFEST);

fn load_retention_days() -> u32 {
    match CLIPMAN_SCHEMA.read_i64("config/retention_days") {
        Some(v) if v >= 0 => v as u32,
        _ => DEFAULT_RETENTION_DAYS,
    }
}

fn save_retention_days(days: u32) {
    let _ = CLIPMAN_SCHEMA.write_i64("config/retention_days", days as i64);
}

// ── Data model ───────────────────────────────────────────────────────────────

struct ClipEntry {
    text: String,
    time: String,
}

struct AppState {
    win: anyui::Window,
    grid: anyui::DataGrid,
    status_label: anyui::Label,
    retention_label: anyui::Label,

    // Settings panel
    settings_panel: anyui::View,
    retention_field: anyui::TextField,
    settings_active: bool,

    // Add-text panel (multi-line)
    add_panel: anyui::View,
    add_field: anyui::TextArea,
    add_active: bool,

    entries: Vec<ClipEntry>,
    retention_days: u32,
    last_clipboard: String,
    history_path: String,
    paused: bool,
}

anyos_std::global_app_state!(AppState);

// ── Time helpers ─────────────────────────────────────────────────────────────

fn now_string() -> String {
    let mut buf = [0u8; 8];
    anyos_std::sys::time(&mut buf);
    let year = buf[0] as u16 | ((buf[1] as u16) << 8);
    let month = buf[2];
    let day = buf[3];
    let hour = buf[4];
    let min = buf[5];
    let sec = buf[6];
    anyos_std::format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year,
        month,
        day,
        hour,
        min,
        sec
    )
}

fn now_days() -> u32 {
    let mut buf = [0u8; 8];
    anyos_std::sys::time(&mut buf);
    let year = (buf[0] as u32) | ((buf[1] as u32) << 8);
    let month = buf[2] as u32;
    let day = buf[3] as u32;
    // Approximate days since epoch (good enough for retention comparison)
    year * 365 + month * 30 + day
}

fn timestamp_to_days(ts: &str) -> u32 {
    // Parse "YYYY-MM-DD HH:MM:SS"
    let bytes = ts.as_bytes();
    if bytes.len() < 10 {
        return 0;
    }
    let year = parse_num(&bytes[0..4]);
    let month = parse_num(&bytes[5..7]);
    let day = parse_num(&bytes[8..10]);
    year * 365 + month * 30 + day
}

fn parse_num(bytes: &[u8]) -> u32 {
    let mut n: u32 = 0;
    for &b in bytes {
        if b >= b'0' && b <= b'9' {
            n = n * 10 + (b - b'0') as u32;
        }
    }
    n
}

// ── JSON persistence ─────────────────────────────────────────────────────────

fn get_history_path() -> String {
    let mut home_buf = [0u8; 256];
    let len = anyos_std::env::get("HOME", &mut home_buf);
    if len == u32::MAX || len == 0 {
        return String::from("/tmp/.clipboard_history.json");
    }
    let home = core::str::from_utf8(&home_buf[..len as usize]).unwrap_or("/tmp");
    anyos_std::format!("{}/{}", home, HISTORY_FILE)
}

fn load_history(path: &str) -> Vec<ClipEntry> {
    let data = match read_file(path) {
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
    if let Some(arr) = json["entries"].as_array() {
        for item in arr {
            let text = item["text"].as_str().unwrap_or("");
            let time = item["time"].as_str().unwrap_or("");
            if !text.is_empty() {
                entries.push(ClipEntry {
                    text: String::from(text),
                    time: String::from(time),
                });
            }
        }
    }

    entries
}

fn save_history(s: &AppState) {
    use anyos_std::json::Value;

    let mut root = Value::new_object();
    let mut arr = Value::new_array();
    for entry in &s.entries {
        let mut obj = Value::new_object();
        obj.set("text", entry.text.as_str().into());
        obj.set("time", entry.time.as_str().into());
        arr.push(obj);
    }
    root.set("entries", arr);

    let json_str = root.to_json_string_pretty();
    let _ = anyos_std::fs::write_bytes(&s.history_path, json_str.as_bytes());
}

fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut content = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        content.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);
    Some(content)
}

// ── Cleanup ──────────────────────────────────────────────────────────────────

fn cleanup_old_entries(s: &mut AppState) {
    if s.retention_days == 0 {
        return;
    } // 0 = never delete
    let today = now_days();
    s.entries.retain(|e| {
        let entry_days = timestamp_to_days(&e.time);
        if entry_days == 0 {
            return true;
        } // keep unparseable entries
        today.saturating_sub(entry_days) < s.retention_days
    });
}

// ── Grid population ──────────────────────────────────────────────────────────

fn populate_grid(s: &AppState) {
    let n = s.entries.len();
    if n == 0 {
        s.grid.set_row_count(0);
        s.grid.set_cell_colors(&[]);
        return;
    }

    s.grid.set_row_count(n as u32);

    let mut rows: Vec<Vec<&str>> = Vec::new();
    // We need to store the number strings and preview strings
    // Since set_data takes &[Vec<&str>], we need owned buffers
    let mut num_strs: Vec<String> = Vec::new();
    let mut preview_strs: Vec<String> = Vec::new();

    for (i, entry) in s.entries.iter().enumerate() {
        num_strs.push(anyos_std::format!("{}", i + 1));
        // Create preview: first line, truncated
        let preview = make_preview(&entry.text);
        preview_strs.push(preview);
    }

    for i in 0..n {
        let mut row = Vec::new();
        row.push(num_strs[i].as_str());
        row.push(preview_strs[i].as_str());
        row.push(s.entries[i].time.as_str());
        rows.push(row);
    }
    s.grid.set_data(&rows);

    // Color the line numbers and timestamps with dim theme colors
    let tc = anyui::theme::colors();
    let total = n * NUM_COLS;
    let mut colors: Vec<u32> = Vec::new();
    colors.resize(total, 0);
    for i in 0..n {
        colors[i * NUM_COLS] = tc.text_disabled; // line number dim
        colors[i * NUM_COLS + 2] = tc.text_secondary; // timestamp dim
    }
    s.grid.set_cell_colors(&colors);

    update_status(s);
}

fn make_preview(text: &str) -> String {
    // Skip leading empty/whitespace-only lines, then take first non-empty line
    let first_line = text
        .split('\n')
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let trimmed = first_line;
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
        anyos_std::format!("1 {}", i18n::t("entry"))
    } else {
        anyos_std::format!("{} {}", count, i18n::t("entries"))
    };
    s.status_label.set_text(&status);

    let retention = if s.retention_days == 0 {
        anyos_std::format!("{}: {}", i18n::t("Retention"), i18n::t("unlimited"))
    } else if s.retention_days == 1 {
        anyos_std::format!("{}: 1 {}", i18n::t("Retention"), i18n::t("day"))
    } else {
        anyos_std::format!(
            "{}: {} {}",
            i18n::t("Retention"),
            s.retention_days,
            i18n::t("days")
        )
    };
    s.retention_label.set_text(&retention);
}

// ── Clipboard polling ────────────────────────────────────────────────────────

fn poll_clipboard() {
    let s = app();
    if s.paused {
        return;
    }

    let mut buf = [0u8; 4096];
    let len = anyui::clipboard_get(&mut buf);
    if len == 0 {
        return;
    }

    let text = match core::str::from_utf8(&buf[..len as usize]) {
        Ok(s) => s,
        Err(_) => return,
    };

    // Skip if same as last known
    if text == s.last_clipboard.as_str() {
        return;
    }
    s.last_clipboard = String::from(text);

    // Skip empty
    if text.trim().is_empty() {
        return;
    }

    // Deduplicate: remove existing entry with same text
    let text_owned = String::from(text);
    s.entries.retain(|e| e.text != text_owned);

    // Insert at front (newest first)
    let preview = make_preview(text);
    s.entries.insert(
        0,
        ClipEntry {
            text: text_owned,
            time: now_string(),
        },
    );

    save_history(s);
    populate_grid(s);

    // Visual feedback: select the new row and show status
    s.grid.set_selected_row(0);
    let msg = anyos_std::format!("{}: {}", i18n::t("New"), preview);
    s.status_label.set_text(&msg);
}

// ── Actions ──────────────────────────────────────────────────────────────────

fn copy_to_clipboard() {
    let s = app();
    let row = s.grid.selected_row();
    if row == u32::MAX {
        return;
    }
    let row = row as usize;
    if row >= s.entries.len() {
        return;
    }

    let text = s.entries[row].text.clone();
    // Pause polling briefly to avoid re-adding the same entry
    s.paused = true;
    s.last_clipboard = text.clone();
    anyui::clipboard_set(&text);

    let msg = anyos_std::format!("{}: {}", i18n::t("Copied"), make_preview(&text));
    s.status_label.set_text(&msg);

    // Resume after a short delay
    anyui::set_timer(1000, || {
        app().paused = false;
    });
}

fn delete_selected() {
    let s = app();
    let row = s.grid.selected_row();
    if row == u32::MAX {
        return;
    }
    let row = row as usize;
    if row >= s.entries.len() {
        return;
    }

    s.entries.remove(row);
    save_history(s);
    populate_grid(s);
}

fn clear_all() {
    let s = app();
    if s.entries.is_empty() {
        return;
    }
    s.entries.clear();
    save_history(s);
    populate_grid(s);
    s.status_label.set_text(i18n::t("All entries cleared"));
}

fn toggle_settings() {
    let s = app();
    if s.settings_active {
        close_settings();
    } else {
        open_settings();
    }
}

fn open_settings() {
    let s = app();
    if s.add_active {
        close_add_panel();
    }
    s.settings_active = true;
    s.settings_panel.set_visible(true);
    let val = anyos_std::format!("{}", s.retention_days);
    s.retention_field.set_text(&val);
    s.retention_field.focus();
}

fn close_settings() {
    let s = app();
    s.settings_active = false;
    s.settings_panel.set_visible(false);
}

fn apply_settings() {
    let s = app();
    let mut buf = [0u8; 32];
    let len = s.retention_field.get_text(&mut buf);
    if len > 0 {
        let text = core::str::from_utf8(&buf[..len as usize]).unwrap_or("3");
        let val = parse_num(text.as_bytes());
        s.retention_days = val;
        save_retention_days(val);
        cleanup_old_entries(s);
        save_history(s);
        populate_grid(s);
    }
    close_settings();
}

fn refresh() {
    let s = app();
    s.entries = load_history(&s.history_path);
    s.retention_days = load_retention_days();
    cleanup_old_entries(s);
    populate_grid(s);
}

fn copy_file_to_clipboard() {
    let path = match anyui::FileDialog::open_file() {
        Some(p) => p,
        None => return,
    };
    let data = match read_file(&path) {
        Some(d) => d,
        None => {
            app()
                .status_label
                .set_text(i18n::t("Error: Could not read file"));
            return;
        }
    };
    let text = match core::str::from_utf8(&data) {
        Ok(s) => s,
        Err(_) => {
            app()
                .status_label
                .set_text(i18n::t("Error: File is not a text file"));
            return;
        }
    };
    if text.trim().is_empty() {
        app().status_label.set_text(i18n::t("File is empty"));
        return;
    }

    let s = app();
    s.paused = true;
    s.last_clipboard = String::from(text);
    anyui::clipboard_set(text);

    // Add to history
    let text_owned = String::from(text);
    s.entries.retain(|e| e.text != text_owned);
    s.entries.insert(
        0,
        ClipEntry {
            text: text_owned,
            time: now_string(),
        },
    );
    save_history(s);
    populate_grid(s);
    s.grid.set_selected_row(0);

    // Extract filename for status
    let filename = path.rsplit('/').next().unwrap_or(&path);
    let msg = anyos_std::format!("{}: {}", i18n::t("File copied"), filename);
    s.status_label.set_text(&msg);

    anyui::set_timer(1000, || {
        app().paused = false;
    });
}

fn view_selected() {
    let s = app();
    let row = s.grid.selected_row();
    if row == u32::MAX {
        return;
    }
    let row = row as usize;
    if row >= s.entries.len() {
        return;
    }
    let text = s.entries[row].text.clone();
    show_view_dialog(&text);
}

fn show_view_dialog(text: &str) {
    let tc = anyui::theme::colors();
    const W: u32 = 640;
    const H: u32 = 420;
    let win = anyui::Window::new(i18n::t("View / Edit Entry"), -1, -1, W, H);
    let win_id = win.id();

    let footer = anyui::View::new();
    footer.set_dock(anyui::DOCK_BOTTOM);
    footer.set_size(W, 50);
    footer.set_color(tc.sidebar_bg);
    win.add(&footer);

    let btn_save = anyui::Button::new(i18n::t("Save as New Entry"));
    btn_save.set_size(160, 30);
    btn_save.set_position(12, 10);
    btn_save.set_color(tc.success);
    footer.add(&btn_save);

    let btn_copy = anyui::Button::new(i18n::t("Copy to Clipboard"));
    btn_copy.set_size(160, 30);
    btn_copy.set_position((W as i32) - 270, 10);
    btn_copy.set_color(tc.success);
    footer.add(&btn_copy);

    let btn_close = anyui::Button::new(i18n::t("Close"));
    btn_close.set_size(90, 30);
    btn_close.set_position((W as i32) - 100, 10);
    btn_close.set_color(tc.control_bg);
    footer.add(&btn_close);

    let area = anyui::TextArea::new();
    area.set_dock(anyui::DOCK_FILL);
    area.set_read_only(false);
    area.set_max_length(65536);
    area.set_text(text);
    win.add(&area);

    let area_copy = area;
    btn_copy.on_click(move |_| {
        let mut buf = anyos_std::Vec::new();
        buf.resize(65536, 0u8);
        let n = area_copy.get_text(&mut buf);
        let cur = match core::str::from_utf8(&buf[..n as usize]) {
            Ok(t) => t,
            Err(_) => return,
        };
        if cur.trim().is_empty() {
            return;
        }
        let s = app();
        s.paused = true;
        s.last_clipboard = String::from(cur);
        anyui::clipboard_set(cur);
        let msg = anyos_std::format!("{}: {}", i18n::t("Copied"), make_preview(cur));
        s.status_label.set_text(&msg);
        anyui::set_timer(1000, || {
            app().paused = false;
        });
        anyui::Window::from_id(win_id).destroy();
    });

    let area_save = area;
    btn_save.on_click(move |_| {
        let mut buf = anyos_std::Vec::new();
        buf.resize(65536, 0u8);
        let n = area_save.get_text(&mut buf);
        let cur = match core::str::from_utf8(&buf[..n as usize]) {
            Ok(t) => t,
            Err(_) => return,
        };
        if cur.trim().is_empty() {
            return;
        }
        let s = app();
        s.paused = true;
        s.last_clipboard = String::from(cur);
        anyui::clipboard_set(cur);
        let text_owned = String::from(cur);
        let preview = make_preview(cur);
        s.entries.retain(|e| e.text != text_owned);
        s.entries.insert(
            0,
            ClipEntry {
                text: text_owned,
                time: now_string(),
            },
        );
        save_history(s);
        populate_grid(s);
        s.grid.set_selected_row(0);
        let msg = anyos_std::format!("{}: {}", i18n::t("Saved"), preview);
        s.status_label.set_text(&msg);
        anyui::set_timer(1000, || {
            app().paused = false;
        });
        anyui::Window::from_id(win_id).destroy();
    });

    btn_close.on_click(move |_| {
        anyui::Window::from_id(win_id).destroy();
    });
}

fn toggle_add_panel() {
    let s = app();
    if s.add_active {
        close_add_panel();
    } else {
        open_add_panel();
    }
}

fn open_add_panel() {
    let s = app();
    if s.settings_active {
        close_settings();
    }
    s.add_active = true;
    s.add_panel.set_visible(true);
    s.add_field.set_text("");
    s.add_field.focus();
}

fn close_add_panel() {
    let s = app();
    s.add_active = false;
    s.add_panel.set_visible(false);
}

fn submit_add_text() {
    let s = app();
    let mut buf = anyos_std::Vec::new();
    buf.resize(16384, 0u8);
    let len = s.add_field.get_text(&mut buf);
    if len == 0 {
        close_add_panel();
        return;
    }
    let text = match core::str::from_utf8(&buf[..len as usize]) {
        Ok(t) => t,
        Err(_) => {
            close_add_panel();
            return;
        }
    };
    if text.trim().is_empty() {
        close_add_panel();
        return;
    }

    s.paused = true;
    s.last_clipboard = String::from(text);
    anyui::clipboard_set(text);

    let text_owned = String::from(text);
    let preview = make_preview(text);
    s.entries.retain(|e| e.text != text_owned);
    s.entries.insert(
        0,
        ClipEntry {
            text: text_owned,
            time: now_string(),
        },
    );
    save_history(s);
    populate_grid(s);
    s.grid.set_selected_row(0);

    let msg = anyos_std::format!("{}: {}", i18n::t("Copied"), preview);
    s.status_label.set_text(&msg);

    close_add_panel();
    anyui::set_timer(1000, || {
        app().paused = false;
    });
}

// ── Keyboard handler ─────────────────────────────────────────────────────────

fn handle_key(ke: &anyui::KeyEvent) {
    // Enter: Copy selected entry to clipboard
    if ke.keycode == anyui::KEY_ENTER {
        copy_to_clipboard();
        return;
    }
    // Delete: Delete selected entry
    if ke.keycode == anyui::KEY_DELETE {
        delete_selected();
        return;
    }
    // Escape: Close panels or app
    if ke.keycode == anyui::KEY_ESCAPE {
        let s = app();
        if s.add_active {
            close_add_panel();
        } else if s.settings_active {
            close_settings();
        } else {
            anyui::quit();
        }
        return;
    }
    // Ctrl+A: Clear all
    if ke.ctrl() && (ke.char_code == b'a' as u32 || ke.char_code == b'A' as u32) {
        clear_all();
        return;
    }
    // F5: Refresh
    if ke.keycode == anyui::KEY_F5 {
        refresh();
        return;
    }
    // Ctrl+S: Open settings
    if ke.ctrl() && (ke.char_code == b's' as u32 || ke.char_code == b'S' as u32) {
        toggle_settings();
        return;
    }
    // Ctrl+N: Add text
    if ke.ctrl() && (ke.char_code == b'n' as u32 || ke.char_code == b'N' as u32) {
        toggle_add_panel();
        return;
    }
    // Ctrl+O: From file
    if ke.ctrl() && (ke.char_code == b'o' as u32 || ke.char_code == b'O' as u32) {
        copy_file_to_clipboard();
        return;
    }
}

// ── Main entry ───────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() {
        anyos_std::println!("clipman: failed to load libanyui.so");
        return;
    }
    i18n::init();
    let _ = CLIPMAN_SCHEMA.register();

    // Load settings from confd, history from per-user JSON
    let history_path = get_history_path();
    let mut entries = load_history(&history_path);
    let retention_days = load_retention_days();

    // Get current clipboard content as baseline and add to history if new
    let mut clip_buf = [0u8; 4096];
    let clip_len = anyui::clipboard_get(&mut clip_buf);
    let last_clipboard = if clip_len > 0 {
        let text = String::from(core::str::from_utf8(&clip_buf[..clip_len as usize]).unwrap_or(""));
        if !text.trim().is_empty() && !entries.iter().any(|e| e.text == text) {
            entries.insert(
                0,
                ClipEntry {
                    text: text.clone(),
                    time: now_string(),
                },
            );
        }
        text
    } else {
        String::new()
    };

    // Create window
    let tc = anyui::theme::colors();
    let win = anyui::Window::new(i18n::t("Clipboard Manager"), -1, -1, 600, 400);

    // ── Menu bar ──
    let mut mb = anyui::MenuBarBuilder::new()
        .menu(i18n::t("File"))
        .item(1, i18n::t("Quit"), 0)
        .end_menu()
        .menu(i18n::t("Edit"))
        .item(10, i18n::t("Paste Selected"), 0)
        .item(13, i18n::t("View Full Text"), 0)
        .item(11, i18n::t("Delete Selected"), 0)
        .separator()
        .item(12, i18n::t("Clear History"), 0)
        .end_menu();
    let menu_data = mb.build();
    let menu = anyui::MenuBar::set(win.id(), menu_data);

    // ── Toolbar ──
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(600, 36);
    toolbar.set_color(tc.toolbar_bg);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_copy = toolbar.add_icon_button(i18n::t("Paste"));
    btn_copy.set_size(52, 28);

    let btn_view = toolbar.add_icon_button(i18n::t("View"));
    btn_view.set_size(52, 28);

    let btn_delete = toolbar.add_icon_button(i18n::t("Delete"));
    btn_delete.set_size(56, 28);

    toolbar.add_separator();

    let btn_clear = toolbar.add_icon_button(i18n::t("Clear All"));
    btn_clear.set_size(64, 28);

    toolbar.add_separator();

    let btn_add_text = toolbar.add_icon_button(i18n::t("Add Text"));
    btn_add_text.set_size(68, 28);

    let btn_from_file = toolbar.add_icon_button(i18n::t("From File"));
    btn_from_file.set_size(72, 28);

    toolbar.add_separator();

    let btn_settings = toolbar.add_icon_button(i18n::t("Settings"));
    btn_settings.set_size(68, 28);

    let btn_refresh = toolbar.add_icon_button(i18n::t("Refresh"));
    btn_refresh.set_size(60, 28);

    win.add(&toolbar);

    // ── Status bar ──
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(600, 24);
    status_bar.set_color(tc.sidebar_bg);

    let status_label = anyui::Label::new("");
    status_label.set_position(8, 4);
    status_label.set_text_color(tc.text_secondary);
    status_label.set_font_size(12);
    status_bar.add(&status_label);

    let retention_label = anyui::Label::new("");
    retention_label.set_position(350, 4);
    retention_label.set_text_color(tc.text_secondary);
    retention_label.set_font_size(12);
    status_bar.add(&retention_label);

    win.add(&status_bar);

    // ── Settings panel (DOCK_BOTTOM, above status bar) ──
    let settings_panel = anyui::View::new();
    settings_panel.set_dock(anyui::DOCK_BOTTOM);
    settings_panel.set_size(600, 36);
    settings_panel.set_color(tc.card_bg);
    settings_panel.set_visible(false);

    let settings_lbl = anyui::Label::new(i18n::t("Retention (days, 0=unlimited):"));
    settings_lbl.set_position(8, 9);
    settings_lbl.set_text_color(tc.text);
    settings_lbl.set_font_size(13);
    settings_panel.add(&settings_lbl);

    let retention_field = anyui::TextField::new();
    retention_field.set_position(280, 4);
    retention_field.set_size(60, 26);
    settings_panel.add(&retention_field);

    let btn_apply = anyui::Button::new("OK");
    btn_apply.set_position(350, 4);
    btn_apply.set_size(40, 26);
    settings_panel.add(&btn_apply);

    let btn_cancel = anyui::Button::new("X");
    btn_cancel.set_position(396, 4);
    btn_cancel.set_size(28, 26);
    settings_panel.add(&btn_cancel);

    win.add(&settings_panel);

    // ── Add-text panel (DOCK_BOTTOM, above settings) — multi-line ──
    let add_panel = anyui::View::new();
    add_panel.set_dock(anyui::DOCK_BOTTOM);
    add_panel.set_size(600, 180);
    add_panel.set_color(tc.card_bg);
    add_panel.set_visible(false);

    let add_lbl = anyui::Label::new(i18n::t("Text (multi-line):"));
    add_lbl.set_position(8, 6);
    add_lbl.set_text_color(tc.text);
    add_lbl.set_font_size(13);
    add_panel.add(&add_lbl);

    let add_field = anyui::TextArea::new();
    add_field.set_position(8, 28);
    add_field.set_size(584, 110);
    add_field.set_max_length(16384);
    add_panel.add(&add_field);

    let btn_add_ok = anyui::Button::new(i18n::t("Copy"));
    btn_add_ok.set_position(478, 146);
    btn_add_ok.set_size(60, 28);
    add_panel.add(&btn_add_ok);

    let btn_add_cancel = anyui::Button::new(i18n::t("Cancel"));
    btn_add_cancel.set_position(544, 146);
    btn_add_cancel.set_size(60, 28);
    add_panel.add(&btn_add_cancel);

    win.add(&add_panel);

    // ── DataGrid ──
    let grid = anyui::DataGrid::new(580, 340);
    grid.set_dock(anyui::DOCK_FILL);
    grid.set_row_height(22);
    grid.set_header_height(24);
    grid.set_columns(&[
        anyui::ColumnDef::new("#")
            .width(40)
            .align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new(i18n::t("Content")).width(400),
        anyui::ColumnDef::new(i18n::t("Time")).width(140),
    ]);

    // Context menu
    let ctx_items = anyos_std::format!(
        "{}|{}|-|{}|{}|-|{}|{}",
        i18n::t("Copy to Clipboard"),
        i18n::t("View Full Text"),
        i18n::t("Delete Entry"),
        i18n::t("Delete All"),
        i18n::t("Add Text"),
        i18n::t("Load from File")
    );
    let ctx_menu = anyui::ContextMenu::new(&ctx_items);
    grid.set_context_menu(&ctx_menu);

    win.add(&grid);

    // Cleanup old entries before displaying
    let today = now_days();
    if retention_days > 0 {
        entries.retain(|e| {
            let entry_days = timestamp_to_days(&e.time);
            if entry_days == 0 {
                return true;
            }
            today.saturating_sub(entry_days) < retention_days
        });
    }

    // ── Initialize global state ──
    unsafe {
        APP = Some(AppState {
            win,
            grid,
            status_label,
            retention_label,
            settings_panel,
            retention_field,
            settings_active: false,
            add_panel,
            add_field,
            add_active: false,
            entries,
            retention_days,
            last_clipboard,
            history_path,
            paused: false,
        });
    }

    // Populate grid
    let s = app();
    populate_grid(s);

    // ── Register callbacks ──
    btn_copy.on_click(|_| {
        copy_to_clipboard();
    });
    btn_view.on_click(|_| {
        view_selected();
    });
    btn_delete.on_click(|_| {
        delete_selected();
    });
    btn_clear.on_click(|_| {
        clear_all();
    });
    btn_settings.on_click(|_| {
        toggle_settings();
    });
    btn_refresh.on_click(|_| {
        refresh();
    });
    btn_add_text.on_click(|_| {
        toggle_add_panel();
    });
    btn_from_file.on_click(|_| {
        copy_file_to_clipboard();
    });
    btn_apply.on_click(|_| {
        apply_settings();
    });
    btn_cancel.on_click(|_| {
        close_settings();
    });
    retention_field.on_submit(|_| {
        apply_settings();
    });
    btn_add_ok.on_click(|_| {
        submit_add_text();
    });
    btn_add_cancel.on_click(|_| {
        close_add_panel();
    });

    // Context menu
    ctx_menu.on_item_click(|e| match e.index {
        0 => copy_to_clipboard(),
        1 => view_selected(),
        3 => delete_selected(),
        4 => clear_all(),
        6 => toggle_add_panel(),
        7 => copy_file_to_clipboard(),
        _ => {}
    });

    // Menu bar
    menu.on_item(move |e| match e.item_id {
        1 => {
            save_history(app());
            anyui::quit();
        }
        10 => copy_to_clipboard(),
        13 => view_selected(),
        11 => delete_selected(),
        12 => clear_all(),
        _ => {}
    });

    // Double-click to copy
    app().grid.on_submit(|_| {
        copy_to_clipboard();
    });

    // Window close
    win.on_close(|_| {
        save_history(app());
        anyui::quit();
    });

    // Keyboard shortcuts
    win.on_key_down(|ke| {
        handle_key(ke);
    });

    // ── Start clipboard polling timer ──
    anyui::set_timer(POLL_INTERVAL_MS, || {
        poll_clipboard();
    });

    // ── Run event loop ──
    anyui::run();
}
