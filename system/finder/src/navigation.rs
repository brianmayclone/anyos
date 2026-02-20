//! Directory reading, navigation, and file opening.

use anyos_std::fs;
use anyos_std::icons;
use anyos_std::process;
use anyos_std::String;
use anyos_std::Vec;
use anyos_std::ui::window;

use crate::types::*;

pub fn read_directory(path: &str) -> Vec<FileEntry> {
    let mut buf = [0u8; 256 * 64];
    let count = fs::readdir(path, &mut buf);
    if count == u32::MAX {
        return Vec::new();
    }

    let mut entries = Vec::new();
    for i in 0..count as usize {
        let off = i * 64;
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        let size = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);
        let mut name = [0u8; 56];
        let copy_len = name_len.min(55);
        name[..copy_len].copy_from_slice(&buf[off + 8..off + 8 + copy_len]);
        entries.push(FileEntry { name, name_len: copy_len, entry_type, size });
    }

    entries.sort_by(|a, b| {
        if a.entry_type != b.entry_type {
            return a.entry_type.cmp(&b.entry_type).reverse();
        }
        a.name[..a.name_len].cmp(&b.name[..b.name_len])
    });

    entries
}

pub fn navigate(state: &mut AppState, path: &str) {
    if state.history_pos + 1 < state.history.len() {
        state.history.truncate(state.history_pos + 1);
    }
    state.cwd = String::from(path);
    state.entries = read_directory(path);
    state.selected = None;
    state.scroll_offset = 0;
    state.last_click_idx = None;
    state.history.push(state.cwd.clone());
    state.history_pos = state.history.len() - 1;

    state.path_field.set_text(path);
    state.path_field.focused = false;

    sync_sidebar_sel(state, path);
}

pub fn navigate_back(state: &mut AppState) {
    if state.history_pos == 0 {
        return;
    }
    state.history_pos -= 1;
    let path = state.history[state.history_pos].clone();
    state.cwd = path.clone();
    state.entries = read_directory(&path);
    state.selected = None;
    state.scroll_offset = 0;
    state.last_click_idx = None;
    state.path_field.set_text(&path);
    state.path_field.focused = false;

    sync_sidebar_sel(state, &path);
}

pub fn navigate_forward(state: &mut AppState) {
    if state.history_pos + 1 >= state.history.len() {
        return;
    }
    state.history_pos += 1;
    let path = state.history[state.history_pos].clone();
    state.cwd = path.clone();
    state.entries = read_directory(&path);
    state.selected = None;
    state.scroll_offset = 0;
    state.last_click_idx = None;
    state.path_field.set_text(&path);
    state.path_field.focused = false;

    sync_sidebar_sel(state, &path);
}

fn sync_sidebar_sel(state: &mut AppState, path: &str) {
    state.sidebar_sel = LOCATIONS.len();
    for (i, &(_, loc_path)) in LOCATIONS.iter().enumerate() {
        if loc_path == path {
            state.sidebar_sel = i;
            break;
        }
    }
}

pub fn build_full_path(cwd: &str, name: &str) -> String {
    if cwd == "/" {
        let mut s = String::from("/");
        s.push_str(name);
        s
    } else {
        let mut s = String::from(cwd);
        s.push('/');
        s.push_str(name);
        s
    }
}

pub fn open_entry(state: &mut AppState, idx: usize) {
    let entry = &state.entries[idx];
    let name = entry.name_str();

    if entry.entry_type == TYPE_DIR {
        if name.ends_with(".app") {
            let full_path = build_full_path(&state.cwd, name);
            process::spawn(&full_path, "");
            return;
        }
        let new_path = build_full_path(&state.cwd, name);
        navigate(state, &new_path);
    } else {
        let full_path = build_full_path(&state.cwd, name);

        let ext = match name.rfind('.') {
            Some(pos) => &name[pos + 1..],
            None => "",
        };

        if !ext.is_empty() {
            if let Some(app) = state.mimetypes.app_for_ext(ext) {
                let args = anyos_std::format!("{} {}", app, full_path);
                process::spawn(app, &args);
                return;
            }
        }

        process::spawn(&full_path, &full_path);
    }
}

/// Update menu item enable/disable states based on current app state.
pub fn update_menu_states(win: u32, state: &AppState) {
    if state.selected.is_some() {
        window::enable_menu_item(win, 1);
    } else {
        window::disable_menu_item(win, 1);
    }
    if state.history_pos > 0 {
        window::enable_menu_item(win, 20);
    } else {
        window::disable_menu_item(win, 20);
    }
    if state.history_pos + 1 < state.history.len() {
        window::enable_menu_item(win, 21);
    } else {
        window::disable_menu_item(win, 21);
    }
}
