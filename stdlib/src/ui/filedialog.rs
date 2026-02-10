//! File dialogs — Open File, Open Folder, Save File, Create Folder.
//!
//! Each dialog creates a real compositor window, runs its own event loop, and
//! returns the result. The caller's event loop is paused while the dialog is
//! open, giving natural "modal" behavior.
//!
//! # Usage
//! ```rust,ignore
//! use anyos_std::ui::filedialog;
//!
//! match filedialog::open_file("/") {
//!     filedialog::FileDialogResult::Selected(path) => { /* use path */ }
//!     filedialog::FileDialogResult::Cancelled => { /* user cancelled */ }
//! }
//! ```

use alloc::string::String;
use alloc::vec::Vec;
use crate::ui::window;
use crate::{fs, process};

// ── Public API ──────────────────────────────────────────────────────────────

/// Result of a file dialog interaction.
pub enum FileDialogResult {
    Selected(String),
    Cancelled,
}

/// Open a file browser to select a file.
pub fn open_file(starting_path: &str) -> FileDialogResult {
    run_dialog(Mode::OpenFile, starting_path, "")
}

/// Open a file browser to select a folder.
pub fn open_folder(starting_path: &str) -> FileDialogResult {
    run_dialog(Mode::OpenFolder, starting_path, "")
}

/// Open a save dialog. `default_name` pre-fills the filename field.
pub fn save_file(starting_path: &str, default_name: &str) -> FileDialogResult {
    run_dialog(Mode::SaveFile, starting_path, default_name)
}

/// Open a dialog to create a new folder.
pub fn create_folder(parent_path: &str) -> FileDialogResult {
    run_create_folder_dialog(parent_path)
}

// ── Internal types ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    OpenFile,
    OpenFolder,
    SaveFile,
}

struct DirEntry {
    name: [u8; 56],
    name_len: u8,
    entry_type: u8,    // 1 = file, 2 = dir
    size: u32,
}

struct DialogState {
    mode: Mode,
    current_path: [u8; 257],
    path_len: usize,
    entries: Vec<DirEntry>,
    selected: Option<usize>,
    scroll_offset: u32,
    filename_buf: [u8; 128],
    filename_len: usize,
    filename_focused: bool,
}

// ── Colors ──────────────────────────────────────────────────────────────────

const COLOR_BG: u32 = 0xFF1E1E1E;
const COLOR_PATH_BG: u32 = 0xFF2A2A2A;
const COLOR_TEXT: u32 = 0xFFE6E6E6;
const COLOR_TEXT_SEC: u32 = 0xFF969696;
const COLOR_TEXT_DIM: u32 = 0xFF5A5A5A;
const COLOR_SEL: u32 = 0xFF0058D0;
const COLOR_ROW_ALT: u32 = 0xFF232323;
const COLOR_BORDER: u32 = 0xFF3D3D3D;
const COLOR_BTN_BG: u32 = 0xFF3C3C3C;
const COLOR_BTN_PRIMARY: u32 = 0xFF007AFF;
const COLOR_BTN_TEXT: u32 = 0xFFFFFFFF;
const COLOR_INPUT_BG: u32 = 0xFF2C2C2E;
const COLOR_INPUT_BORDER: u32 = 0xFF48484A;
const COLOR_DIR_ICON: u32 = 0xFF007AFF;

// ── Layout ──────────────────────────────────────────────────────────────────

const WIN_W: u16 = 420;
const WIN_H: u16 = 340;
const PATH_H: i16 = 28;
const ROW_H: i16 = 24;
const BTN_ROW_H: i16 = 40;
const NAME_ROW_H: i16 = 32;
const BTN_H: u16 = 28;
const BTN_MIN_W: u16 = 72;
const PAD: i16 = 8;

// ── Main file browser dialog ────────────────────────────────────────────────

fn run_dialog(mode: Mode, starting_path: &str, default_name: &str) -> FileDialogResult {
    let (sw, sh) = window::screen_size();
    let dx = ((sw as i32 - WIN_W as i32) / 2).max(0) as u16;
    let dy = ((sh as i32 - WIN_H as i32) / 2).max(0) as u16;

    let title = match mode {
        Mode::OpenFile => "Open File",
        Mode::OpenFolder => "Open Folder",
        Mode::SaveFile => "Save File",
    };
    let win = window::create_ex(title, dx, dy, WIN_W, WIN_H,
        window::WIN_FLAG_NOT_RESIZABLE);
    if win == u32::MAX {
        return FileDialogResult::Cancelled;
    }

    let mut state = DialogState {
        mode,
        current_path: [0u8; 257],
        path_len: 0,
        entries: Vec::new(),
        selected: None,
        scroll_offset: 0,
        filename_buf: [0u8; 128],
        filename_len: 0,
        filename_focused: mode == Mode::SaveFile,
    };

    // Set initial path
    set_path(&mut state, starting_path);
    load_entries(&mut state);

    // Pre-fill filename for SaveFile
    if mode == Mode::SaveFile && !default_name.is_empty() {
        let len = default_name.len().min(127);
        state.filename_buf[..len].copy_from_slice(&default_name.as_bytes()[..len]);
        state.filename_len = len;
    }

    render_file_dialog(win, &state);
    window::present(win);

    let mut event = [0u32; 5];
    let mut last_click_tick: u32 = 0;
    let mut last_click_idx: Option<usize> = None;
    let dblclick_ticks = {
        let hz = crate::sys::tick_hz();
        if hz > 0 { hz * 400 / 1000 } else { 400 }
    };

    let result;
    loop {
        if window::get_event(win, &mut event) == 1 {
            match event[0] {
                window::EVENT_KEY_DOWN => {
                    let key = event[1];
                    let ch = event[2];
                    match key {
                        0x103 => { // Escape
                            result = FileDialogResult::Cancelled;
                            break;
                        }
                        0x100 => { // Enter
                            if state.filename_focused && state.filename_len > 0 && mode == Mode::SaveFile {
                                // Confirm save
                                let name = core::str::from_utf8(&state.filename_buf[..state.filename_len]).unwrap_or("");
                                let full = build_full_path(&state, name);
                                result = FileDialogResult::Selected(full);
                                break;
                            }
                            if let Some(idx) = state.selected {
                                if idx < state.entries.len() {
                                    if state.entries[idx].entry_type == 2 {
                                        // Navigate into dir
                                        let name = entry_name(&state.entries[idx]);
                                        let full = build_full_path(&state, &name);
                                        set_path(&mut state, &full);
                                        load_entries(&mut state);
                                        state.selected = None;
                                        state.scroll_offset = 0;
                                        render_file_dialog(win, &state);
                                        window::present(win);
                                    } else if mode != Mode::OpenFolder {
                                        // Confirm file selection
                                        let name = entry_name(&state.entries[idx]);
                                        let full = build_full_path(&state, &name);
                                        result = FileDialogResult::Selected(full);
                                        break;
                                    }
                                }
                            }
                        }
                        0x101 => { // Backspace
                            if state.filename_focused && state.filename_len > 0 {
                                state.filename_len -= 1;
                                render_file_dialog(win, &state);
                                window::present(win);
                            }
                        }
                        0x105 => { // Up
                            if !state.filename_focused {
                                match state.selected {
                                    Some(0) | None => state.selected = Some(0),
                                    Some(i) => state.selected = Some(i - 1),
                                }
                                scroll_to_selection(&mut state);
                                render_file_dialog(win, &state);
                                window::present(win);
                            }
                        }
                        0x106 => { // Down
                            if !state.filename_focused {
                                let max = state.entries.len().saturating_sub(1);
                                match state.selected {
                                    None => state.selected = Some(0),
                                    Some(i) if i < max => state.selected = Some(i + 1),
                                    _ => {}
                                }
                                scroll_to_selection(&mut state);
                                render_file_dialog(win, &state);
                                window::present(win);
                            }
                        }
                        0x102 => { // Tab — toggle filename focus
                            if mode == Mode::SaveFile {
                                state.filename_focused = !state.filename_focused;
                                render_file_dialog(win, &state);
                                window::present(win);
                            }
                        }
                        _ => {
                            // Printable char → filename input
                            if state.filename_focused && ch >= 0x20 && ch < 0x7F
                                && state.filename_len < 127
                            {
                                state.filename_buf[state.filename_len] = ch as u8;
                                state.filename_len += 1;
                                render_file_dialog(win, &state);
                                window::present(win);
                            }
                        }
                    }
                }
                window::EVENT_MOUSE_DOWN => {
                    let mx = event[1] as i16;
                    let my = event[2] as i16;
                    let now = crate::sys::uptime();

                    // Check [^] (parent) button
                    let parent_btn_x = WIN_W as i16 - 32;
                    if mx >= parent_btn_x && mx < WIN_W as i16 - 4 && my >= 2 && my < PATH_H - 2 {
                        navigate_parent(&mut state);
                        render_file_dialog(win, &state);
                        window::present(win);
                        continue;
                    }

                    // Check file list area
                    let list_y_start = PATH_H;
                    let list_h = file_list_height(mode);
                    if my >= list_y_start && my < list_y_start + list_h {
                        state.filename_focused = false;
                        let row_idx = ((my - list_y_start) / ROW_H) as usize + state.scroll_offset as usize;
                        if row_idx < state.entries.len() {
                            // Double-click detection
                            let is_dbl = last_click_idx == Some(row_idx)
                                && now.wrapping_sub(last_click_tick) < dblclick_ticks;
                            last_click_idx = Some(row_idx);
                            last_click_tick = now;

                            state.selected = Some(row_idx);

                            if is_dbl {
                                if state.entries[row_idx].entry_type == 2 {
                                    let name = entry_name(&state.entries[row_idx]);
                                    let full = build_full_path(&state, &name);
                                    set_path(&mut state, &full);
                                    load_entries(&mut state);
                                    state.selected = None;
                                    state.scroll_offset = 0;
                                } else if mode != Mode::OpenFolder {
                                    let name = entry_name(&state.entries[row_idx]);
                                    let full = build_full_path(&state, &name);
                                    result = FileDialogResult::Selected(full);
                                    break;
                                }
                            }

                            // For SaveFile: copy filename on single click
                            if mode == Mode::SaveFile && state.entries[row_idx].entry_type == 1 {
                                let n = entry_name(&state.entries[row_idx]);
                                let nb = n.as_bytes();
                                let len = nb.len().min(127);
                                state.filename_buf[..len].copy_from_slice(&nb[..len]);
                                state.filename_len = len;
                            }

                            render_file_dialog(win, &state);
                            window::present(win);
                        }
                        continue;
                    }

                    // Check filename field click (SaveFile)
                    if mode == Mode::SaveFile {
                        let name_y = PATH_H + file_list_height(mode);
                        if my >= name_y && my < name_y + NAME_ROW_H as i16 {
                            state.filename_focused = true;
                            render_file_dialog(win, &state);
                            window::present(win);
                            continue;
                        }
                    }

                    // Check buttons
                    let btn_y = WIN_H as i16 - BTN_ROW_H;
                    if my >= btn_y {
                        // Cancel button
                        let cancel_x = WIN_W as i16 - 16 - BTN_MIN_W as i16 - 8 - BTN_MIN_W as i16;
                        if mx >= cancel_x && mx < cancel_x + BTN_MIN_W as i16 {
                            result = FileDialogResult::Cancelled;
                            break;
                        }
                        // OK button
                        let ok_x = WIN_W as i16 - 16 - BTN_MIN_W as i16;
                        if mx >= ok_x && mx < ok_x + BTN_MIN_W as i16 {
                            if let Some(r) = confirm_action(&state) {
                                result = r;
                                break;
                            }
                        }
                    }
                }
                window::EVENT_MOUSE_SCROLL => {
                    let dz = event[1] as i32;
                    if dz < 0 {
                        state.scroll_offset = state.scroll_offset.saturating_sub((-dz) as u32);
                    } else {
                        let max_visible = (file_list_height(state.mode) / ROW_H) as usize;
                        let max_scroll = state.entries.len().saturating_sub(max_visible) as u32;
                        state.scroll_offset = (state.scroll_offset + dz as u32).min(max_scroll);
                    }
                    render_file_dialog(win, &state);
                    window::present(win);
                }
                window::EVENT_WINDOW_CLOSE => {
                    result = FileDialogResult::Cancelled;
                    break;
                }
                _ => {}
            }
        } else {
            process::yield_cpu();
        }
    }

    window::destroy(win);
    result
}

// ── Create Folder dialog (simpler) ──────────────────────────────────────────

const CF_WIN_W: u16 = 340;
const CF_WIN_H: u16 = 150;

fn run_create_folder_dialog(parent_path: &str) -> FileDialogResult {
    let (sw, sh) = window::screen_size();
    let dx = ((sw as i32 - CF_WIN_W as i32) / 2).max(0) as u16;
    let dy = ((sh as i32 - CF_WIN_H as i32) / 2).max(0) as u16;

    let win = window::create_ex("Create Folder", dx, dy, CF_WIN_W, CF_WIN_H,
        window::WIN_FLAG_NOT_RESIZABLE);
    if win == u32::MAX {
        return FileDialogResult::Cancelled;
    }

    let mut name_buf = [0u8; 128];
    let mut name_len: usize = 0;

    render_create_folder(win, parent_path, &name_buf, name_len);
    window::present(win);

    let mut event = [0u32; 5];
    let result;
    loop {
        if window::get_event(win, &mut event) == 1 {
            match event[0] {
                window::EVENT_KEY_DOWN => {
                    let key = event[1];
                    let ch = event[2];
                    match key {
                        0x103 => {
                            result = FileDialogResult::Cancelled;
                            break;
                        }
                        0x100 => {
                            if name_len > 0 {
                                // Build full path and create directory
                                let name = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("");
                                let mut full_path = String::from(parent_path);
                                if !full_path.ends_with('/') {
                                    full_path.push('/');
                                }
                                full_path.push_str(name);

                                let rc = fs::mkdir(&full_path);
                                if rc == 0 {
                                    result = FileDialogResult::Selected(full_path);
                                } else {
                                    result = FileDialogResult::Cancelled;
                                }
                                break;
                            }
                        }
                        0x101 => {
                            if name_len > 0 {
                                name_len -= 1;
                                render_create_folder(win, parent_path, &name_buf, name_len);
                                window::present(win);
                            }
                        }
                        _ => {
                            if ch >= 0x20 && ch < 0x7F && name_len < 127 {
                                name_buf[name_len] = ch as u8;
                                name_len += 1;
                                render_create_folder(win, parent_path, &name_buf, name_len);
                                window::present(win);
                            }
                        }
                    }
                }
                window::EVENT_MOUSE_DOWN => {
                    let mx = event[1] as i16;
                    let my = event[2] as i16;
                    let btn_y = CF_WIN_H as i16 - 44;
                    if my >= btn_y && my < btn_y + BTN_H as i16 {
                        let cancel_x = CF_WIN_W as i16 - 16 - BTN_MIN_W as i16 - 8 - BTN_MIN_W as i16;
                        if mx >= cancel_x && mx < cancel_x + BTN_MIN_W as i16 {
                            result = FileDialogResult::Cancelled;
                            break;
                        }
                        let ok_x = CF_WIN_W as i16 - 16 - BTN_MIN_W as i16;
                        if mx >= ok_x && mx < ok_x + BTN_MIN_W as i16 && name_len > 0 {
                            let name = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("");
                            let mut full_path = String::from(parent_path);
                            if !full_path.ends_with('/') {
                                full_path.push('/');
                            }
                            full_path.push_str(name);
                            let rc = fs::mkdir(&full_path);
                            if rc == 0 {
                                result = FileDialogResult::Selected(full_path);
                            } else {
                                result = FileDialogResult::Cancelled;
                            }
                            break;
                        }
                    }
                }
                window::EVENT_WINDOW_CLOSE => {
                    result = FileDialogResult::Cancelled;
                    break;
                }
                _ => {}
            }
        } else {
            process::yield_cpu();
        }
    }

    window::destroy(win);
    result
}

fn render_create_folder(win: u32, parent_path: &str, name_buf: &[u8], name_len: usize) {
    window::fill_rect(win, 0, 0, CF_WIN_W, CF_WIN_H, COLOR_BG);

    // Label
    window::draw_text_ex(win, 16, 16, COLOR_TEXT, 0, 13, "Create a new folder in:");
    // Path
    let display_path = if parent_path.len() > 40 {
        &parent_path[parent_path.len() - 40..]
    } else {
        parent_path
    };
    window::draw_text_ex(win, 16, 36, COLOR_TEXT_SEC, 0, 13, display_path);

    // Input field
    let field_y: i16 = 62;
    let field_w: u16 = CF_WIN_W - 32;
    window::fill_rounded_rect(win, 16, field_y, field_w, 28, 4, COLOR_INPUT_BG);
    // Border
    draw_rect_border(win, 16, field_y, field_w, 28, COLOR_INPUT_BORDER);

    if name_len > 0 {
        let text = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("");
        window::draw_text_ex(win, 22, field_y + 6, COLOR_TEXT, 0, 13, text);
        // Cursor
        let (tw, _) = window::font_measure(0, 13, text);
        window::fill_rect(win, 22 + tw as i16, field_y + 5, 1, 16, COLOR_TEXT);
    } else {
        window::draw_text_ex(win, 22, field_y + 6, COLOR_TEXT_DIM, 0, 13, "Folder name");
    }

    // Buttons
    let btn_y = CF_WIN_H as i16 - 44;
    let ok_x = CF_WIN_W as i16 - 16 - BTN_MIN_W as i16;
    let cancel_x = ok_x - 8 - BTN_MIN_W as i16;

    window::fill_rounded_rect(win, cancel_x, btn_y, BTN_MIN_W, BTN_H, 6, COLOR_BTN_BG);
    draw_centered_label(win, cancel_x, btn_y, BTN_MIN_W, BTN_H, "Cancel", COLOR_BTN_TEXT);

    window::fill_rounded_rect(win, ok_x, btn_y, BTN_MIN_W, BTN_H, 6, COLOR_BTN_PRIMARY);
    draw_centered_label(win, ok_x, btn_y, BTN_MIN_W, BTN_H, "Create", COLOR_BTN_TEXT);
}

// ── Rendering helpers ───────────────────────────────────────────────────────

fn file_list_height(mode: Mode) -> i16 {
    let base = WIN_H as i16 - PATH_H - BTN_ROW_H;
    if mode == Mode::SaveFile {
        base - NAME_ROW_H as i16
    } else {
        base
    }
}

fn render_file_dialog(win: u32, state: &DialogState) {
    let w = WIN_W;
    let h = WIN_H;

    // Background
    window::fill_rect(win, 0, 0, w, h, COLOR_BG);

    // Path bar
    window::fill_rect(win, 0, 0, w, PATH_H as u16, COLOR_PATH_BG);
    let path_str = core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/");
    let display_path = if path_str.len() > 45 {
        &path_str[path_str.len() - 45..]
    } else {
        path_str
    };
    window::draw_text_ex(win, PAD, 6, COLOR_TEXT, 0, 13, display_path);

    // [^] parent button
    let parent_btn_x = w as i16 - 32;
    window::fill_rounded_rect(win, parent_btn_x, 3, 26, 22, 4, COLOR_BTN_BG);
    draw_centered_label(win, parent_btn_x, 3, 26, 22, "^", COLOR_BTN_TEXT);

    // Separator
    window::fill_rect(win, 0, PATH_H as i16, w, 1, COLOR_BORDER);

    // File list
    let list_y = PATH_H + 1;
    let list_h = file_list_height(state.mode);
    let max_visible = (list_h / ROW_H) as usize;

    for vi in 0..max_visible {
        let idx = state.scroll_offset as usize + vi;
        let ry = list_y + (vi as i16) * ROW_H;

        if idx < state.entries.len() {
            let entry = &state.entries[idx];
            let is_sel = state.selected == Some(idx);

            // Row background
            let bg = if is_sel {
                COLOR_SEL
            } else if idx % 2 == 1 {
                COLOR_ROW_ALT
            } else {
                COLOR_BG
            };
            window::fill_rect(win, 0, ry, w, ROW_H as u16, bg);

            // Icon / prefix
            let name = entry_name(entry);
            if entry.entry_type == 2 {
                // Directory
                window::draw_text_ex(win, PAD, ry + 4, COLOR_DIR_ICON, 0, 13, "D");
                window::draw_text_ex(win, PAD + 14, ry + 4, COLOR_TEXT, 0, 13, &name);
                window::draw_text_ex(win,
                    PAD + 14 + {
                        let (tw, _) = window::font_measure(0, 13, &name);
                        tw as i16
                    },
                    ry + 4, COLOR_TEXT_SEC, 0, 13, "/");
            } else {
                window::draw_text_ex(win, PAD + 14, ry + 4, COLOR_TEXT, 0, 13, &name);
                // File size (right-aligned)
                let size_str = format_size(entry.size);
                let (sw, _) = window::font_measure(0, 13, &size_str);
                window::draw_text_ex(win, w as i16 - PAD - sw as i16, ry + 4, COLOR_TEXT_SEC, 0, 13, &size_str);
            }
        } else {
            // Empty row
            let bg = if idx % 2 == 1 { COLOR_ROW_ALT } else { COLOR_BG };
            window::fill_rect(win, 0, ry, w, ROW_H as u16, bg);
        }
    }

    // Separator below list
    let sep_y = list_y + list_h;
    window::fill_rect(win, 0, sep_y, w, 1, COLOR_BORDER);

    // Filename field (SaveFile only)
    if state.mode == Mode::SaveFile {
        let name_y = sep_y + 1;
        window::fill_rect(win, 0, name_y, w, NAME_ROW_H as u16, COLOR_BG);
        window::draw_text_ex(win, PAD, name_y + 8, COLOR_TEXT_SEC, 0, 13, "Name:");

        let field_x: i16 = 52;
        let field_w: u16 = (w as i16 - field_x - PAD) as u16;
        let border_color = if state.filename_focused { 0xFF007AFF } else { COLOR_INPUT_BORDER };
        window::fill_rounded_rect(win, field_x, name_y + 2, field_w, 26, 4, COLOR_INPUT_BG);
        draw_rect_border(win, field_x, name_y + 2, field_w, 26, border_color);

        if state.filename_len > 0 {
            let text = core::str::from_utf8(&state.filename_buf[..state.filename_len]).unwrap_or("");
            window::draw_text_ex(win, field_x + 6, name_y + 8, COLOR_TEXT, 0, 13, text);
            if state.filename_focused {
                let (tw, _) = window::font_measure(0, 13, text);
                window::fill_rect(win, field_x + 6 + tw as i16, name_y + 6, 1, 18, COLOR_TEXT);
            }
        } else {
            window::draw_text_ex(win, field_x + 6, name_y + 8, COLOR_TEXT_DIM, 0, 13, "filename");
            if state.filename_focused {
                window::fill_rect(win, field_x + 6, name_y + 6, 1, 18, COLOR_TEXT);
            }
        }
    }

    // Button row
    let btn_y = h as i16 - BTN_ROW_H + 6;
    let ok_label = match state.mode {
        Mode::OpenFile | Mode::OpenFolder => "Open",
        Mode::SaveFile => "Save",
    };
    let ok_x = w as i16 - 16 - BTN_MIN_W as i16;
    let cancel_x = ok_x - 8 - BTN_MIN_W as i16;

    window::fill_rounded_rect(win, cancel_x, btn_y, BTN_MIN_W, BTN_H, 6, COLOR_BTN_BG);
    draw_centered_label(win, cancel_x, btn_y, BTN_MIN_W, BTN_H, "Cancel", COLOR_BTN_TEXT);

    window::fill_rounded_rect(win, ok_x, btn_y, BTN_MIN_W, BTN_H, 6, COLOR_BTN_PRIMARY);
    draw_centered_label(win, ok_x, btn_y, BTN_MIN_W, BTN_H, ok_label, COLOR_BTN_TEXT);
}

// ── State helpers ───────────────────────────────────────────────────────────

fn set_path(state: &mut DialogState, path: &str) {
    let bytes = path.as_bytes();
    let len = bytes.len().min(256);
    state.current_path[..len].copy_from_slice(&bytes[..len]);
    state.path_len = len;
}

fn load_entries(state: &mut DialogState) {
    state.entries.clear();
    let path = core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/");

    let mut buf = [0u8; 64 * 64]; // max 64 entries
    let count = fs::readdir(path, &mut buf);
    if count == u32::MAX {
        return;
    }

    for i in 0..count as usize {
        let off = i * 64;
        if off + 64 > buf.len() { break; }
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        let size = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);

        if name_len == 0 || name_len > 56 { continue; }

        // Skip "." entry
        if name_len == 1 && buf[off + 8] == b'.' { continue; }
        // Skip ".." entry
        if name_len == 2 && buf[off + 8] == b'.' && buf[off + 9] == b'.' { continue; }

        // For OpenFolder mode, only show directories
        if state.mode == Mode::OpenFolder && entry_type != 2 {
            continue;
        }

        let mut name = [0u8; 56];
        name[..name_len].copy_from_slice(&buf[off + 8..off + 8 + name_len]);

        state.entries.push(DirEntry {
            name,
            name_len: name_len as u8,
            entry_type,
            size,
        });
    }

    // Sort: directories first, then alphabetical
    state.entries.sort_by(|a, b| {
        if a.entry_type == 2 && b.entry_type != 2 {
            core::cmp::Ordering::Less
        } else if a.entry_type != 2 && b.entry_type == 2 {
            core::cmp::Ordering::Greater
        } else {
            a.name[..a.name_len as usize].cmp(&b.name[..b.name_len as usize])
        }
    });
}

fn navigate_parent(state: &mut DialogState) {
    let path = core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/");
    if path == "/" { return; }

    let trimmed = path.trim_end_matches('/');
    let parent = match trimmed.rfind('/') {
        Some(0) => "/",
        Some(pos) => &trimmed[..pos],
        None => "/",
    };
    let parent_str = String::from(parent);
    set_path(state, &parent_str);
    load_entries(state);
    state.selected = None;
    state.scroll_offset = 0;
}

fn confirm_action(state: &DialogState) -> Option<FileDialogResult> {
    match state.mode {
        Mode::OpenFile => {
            if let Some(idx) = state.selected {
                if idx < state.entries.len() && state.entries[idx].entry_type == 1 {
                    let name = entry_name(&state.entries[idx]);
                    let full = build_full_path(state, &name);
                    return Some(FileDialogResult::Selected(full));
                }
            }
            None
        }
        Mode::OpenFolder => {
            // Current directory or selected directory
            if let Some(idx) = state.selected {
                if idx < state.entries.len() && state.entries[idx].entry_type == 2 {
                    let name = entry_name(&state.entries[idx]);
                    let full = build_full_path(state, &name);
                    return Some(FileDialogResult::Selected(full));
                }
            }
            // If nothing selected, select current directory
            let path = core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/");
            Some(FileDialogResult::Selected(String::from(path)))
        }
        Mode::SaveFile => {
            if state.filename_len > 0 {
                let name = core::str::from_utf8(&state.filename_buf[..state.filename_len]).unwrap_or("");
                let full = build_full_path(state, name);
                return Some(FileDialogResult::Selected(full));
            }
            None
        }
    }
}

fn entry_name(entry: &DirEntry) -> String {
    let slice = &entry.name[..entry.name_len as usize];
    String::from(core::str::from_utf8(slice).unwrap_or("?"))
}

fn build_full_path(state: &DialogState, name: &str) -> String {
    let path = core::str::from_utf8(&state.current_path[..state.path_len]).unwrap_or("/");
    let mut full = String::from(path);
    if !full.ends_with('/') {
        full.push('/');
    }
    full.push_str(name);
    full
}

fn scroll_to_selection(state: &mut DialogState) {
    if let Some(sel) = state.selected {
        let max_visible = (file_list_height(state.mode) / ROW_H) as usize;
        if sel < state.scroll_offset as usize {
            state.scroll_offset = sel as u32;
        } else if sel >= state.scroll_offset as usize + max_visible {
            state.scroll_offset = (sel - max_visible + 1) as u32;
        }
    }
}

fn format_size(size: u32) -> String {
    if size >= 1_048_576 {
        let mb = size / 1_048_576;
        let frac = (size % 1_048_576) * 10 / 1_048_576;
        let mut s = String::new();
        write_u32(&mut s, mb);
        s.push('.');
        write_u32(&mut s, frac);
        s.push_str(" MB");
        s
    } else if size >= 1024 {
        let kb = size / 1024;
        let mut s = String::new();
        write_u32(&mut s, kb);
        s.push_str(" KB");
        s
    } else {
        let mut s = String::new();
        write_u32(&mut s, size);
        s.push_str(" B");
        s
    }
}

fn write_u32(s: &mut String, mut val: u32) {
    if val == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        s.push(buf[i] as char);
    }
}

fn draw_centered_label(win: u32, x: i16, y: i16, w: u16, h: u16, text: &str, color: u32) {
    let (tw, th) = window::font_measure(0, 13, text);
    let tx = x + (w as i16 - tw as i16) / 2;
    let ty = y + (h as i16 - th as i16) / 2;
    window::draw_text_ex(win, tx, ty, color, 0, 13, text);
}

fn draw_rect_border(win: u32, x: i16, y: i16, w: u16, h: u16, color: u32) {
    window::fill_rect(win, x, y, w, 1, color);
    window::fill_rect(win, x, y + h as i16 - 1, w, 1, color);
    window::fill_rect(win, x, y + 1, 1, h - 2, color);
    window::fill_rect(win, x + w as i16 - 1, y + 1, 1, h - 2, color);
}
