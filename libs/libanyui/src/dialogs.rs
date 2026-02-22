//! macOS-style file/folder dialogs — OpenFolder, OpenFile, SaveFile, CreateFolder.
//!
//! Each dialog is a modal Card panel centered on the window (NO dark overlay).
//! Uses the same blocking mini event loop as MessageBox.

use alloc::vec;
use alloc::vec::Vec;
use crate::control::{Control, ControlId, ControlKind, DockStyle, EVENT_CLICK, EVENT_DOUBLE_CLICK};
use crate::controls;
use crate::{state, event_loop, syscall};

// ── Dialog state (module-level statics) ──────────────────────────────

static mut DIALOG_DISMISSED: bool = false;
static mut DIALOG_RESULT: [u8; 257] = [0; 257];
static mut DIALOG_RESULT_LEN: usize = 0;
static mut DIALOG_CURRENT_DIR: [u8; 257] = [0; 257];
static mut DIALOG_CURRENT_DIR_LEN: usize = 0;

/// Tracks which entries in the TreeView are directories vs files.
/// Index matches TreeView node index. true = directory, false = file.
static mut DIALOG_ENTRY_IS_DIR: [bool; 256] = [false; 256];
static mut DIALOG_ENTRY_COUNT: usize = 0;

/// IDs of controls used by the dialog (so callbacks can find them).
static mut DIALOG_TREE_ID: ControlId = 0;
static mut DIALOG_CARD_ID: ControlId = 0;
static mut DIALOG_PATH_LABEL_ID: ControlId = 0;
static mut DIALOG_NAME_FIELD_ID: ControlId = 0;
static mut DIALOG_SHOW_FILES: bool = true;

// ── Directory entry ──────────────────────────────────────────────────

struct DirEntry {
    name: [u8; 56],
    name_len: usize,
    is_dir: bool,
}

impl DirEntry {
    fn name_slice(&self) -> &[u8] {
        &self.name[..self.name_len]
    }
}

// ── Path helpers ─────────────────────────────────────────────────────

fn path_join(base: &[u8], name: &[u8], out: &mut [u8; 257]) -> usize {
    let base_len = base.len();
    let mut pos = 0;
    // Copy base
    let copy_len = base_len.min(255);
    out[..copy_len].copy_from_slice(&base[..copy_len]);
    pos = copy_len;
    // Add separator if needed
    if pos > 0 && pos < 256 && out[pos - 1] != b'/' {
        out[pos] = b'/';
        pos += 1;
    }
    // Copy name
    let name_copy = name.len().min(256 - pos);
    out[pos..pos + name_copy].copy_from_slice(&name[..name_copy]);
    pos += name_copy;
    // Null-terminate
    if pos < 257 { out[pos] = 0; }
    pos
}

fn path_parent(path: &[u8], out: &mut [u8; 257]) -> usize {
    let len = path.len();
    if len <= 1 {
        // Already at root
        out[0] = b'/';
        out[1] = 0;
        return 1;
    }
    // Find last '/' (skip trailing slash)
    let search_end = if path[len - 1] == b'/' { len - 1 } else { len };
    let mut last_slash = 0;
    for i in (0..search_end).rev() {
        if path[i] == b'/' {
            last_slash = i;
            break;
        }
    }
    if last_slash == 0 {
        out[0] = b'/';
        out[1] = 0;
        return 1;
    }
    out[..last_slash].copy_from_slice(&path[..last_slash]);
    out[last_slash] = 0;
    last_slash
}

// ── Directory listing ────────────────────────────────────────────────

fn list_directory(dir_path: &[u8]) -> Vec<DirEntry> {
    // Ensure null-terminated path
    let mut path_buf = [0u8; 257];
    let len = dir_path.len().min(256);
    path_buf[..len].copy_from_slice(&dir_path[..len]);
    path_buf[len] = 0;

    let mut raw_buf = vec![0u8; 8192]; // up to 128 entries
    let count = syscall::readdir(&path_buf, &mut raw_buf);
    if count == u32::MAX || count == 0 {
        return Vec::new();
    }

    let count = count as usize;
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let base = i * 64;
        if base + 64 > raw_buf.len() { break; }
        let entry_type = raw_buf[base];
        let name_len = (raw_buf[base + 1] as usize).min(56);
        let mut name = [0u8; 56];
        name[..name_len].copy_from_slice(&raw_buf[base + 8..base + 8 + name_len]);

        // Skip "." and ".."
        if name_len == 1 && name[0] == b'.' { continue; }
        if name_len == 2 && name[0] == b'.' && name[1] == b'.' { continue; }

        entries.push(DirEntry {
            name,
            name_len,
            is_dir: entry_type == 1, // 1 = directory in kernel FileType
        });
    }

    // Sort: directories first, then files, alphabetically within each group
    entries.sort_by(|a, b| {
        match (a.is_dir, b.is_dir) {
            (true, false) => core::cmp::Ordering::Less,
            (false, true) => core::cmp::Ordering::Greater,
            _ => a.name_slice().cmp(b.name_slice()),
        }
    });

    entries
}

// ── Populate tree with directory contents ────────────────────────────

fn populate_file_list(show_files: bool) {
    let st = state();
    let tree_id = unsafe { DIALOG_TREE_ID };

    // Clear tree
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == tree_id) {
        if let Some(tv) = as_tree_view_mut(ctrl) {
            tv.clear();
        }
    }

    let dir_path = unsafe { &DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN] };

    // Update path label
    let path_label_id = unsafe { DIALOG_PATH_LABEL_ID };
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == path_label_id) {
        ctrl.set_text(dir_path);
    }

    unsafe { DIALOG_ENTRY_COUNT = 0; }

    // Add ".." entry unless at root "/"
    if dir_path.len() > 1 || (dir_path.len() == 1 && dir_path[0] != b'/') {
        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == tree_id) {
            if let Some(tv) = as_tree_view_mut(ctrl) {
                let idx = tv.add_node(None, b"..");
                tv.set_node_style(idx, 1); // bold
                tv.set_node_text_color(idx, 0xFF888888);
            }
        }
        unsafe {
            DIALOG_ENTRY_IS_DIR[0] = true; // ".." is a directory
            DIALOG_ENTRY_COUNT = 1;
        }
    }

    let entries = list_directory(dir_path);

    for entry in &entries {
        if !entry.is_dir && !show_files {
            continue;
        }
        let idx_in_tracking = unsafe { DIALOG_ENTRY_COUNT };
        if idx_in_tracking >= 256 { break; }

        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == tree_id) {
            if let Some(tv) = as_tree_view_mut(ctrl) {
                let node_idx = tv.add_node(None, entry.name_slice());
                if entry.is_dir {
                    tv.set_node_style(node_idx, 1); // bold
                    tv.set_node_text_color(node_idx, 0xFFCCCCCC);
                } else {
                    tv.set_node_text_color(node_idx, 0xFFBBBBBB);
                }
            }
        }

        unsafe {
            DIALOG_ENTRY_IS_DIR[idx_in_tracking] = entry.is_dir;
            DIALOG_ENTRY_COUNT += 1;
        }
    }
}

fn as_tree_view_mut(ctrl: &mut alloc::boxed::Box<dyn Control>) -> Option<&mut controls::tree_view::TreeView> {
    if ctrl.kind() == ControlKind::TreeView {
        let raw: *mut dyn Control = &mut **ctrl;
        Some(unsafe { &mut *(raw as *mut controls::tree_view::TreeView) })
    } else {
        None
    }
}

fn as_tree_view_ref(ctrl: &alloc::boxed::Box<dyn Control>) -> Option<&controls::tree_view::TreeView> {
    if ctrl.kind() == ControlKind::TreeView {
        let raw: *const dyn Control = &**ctrl;
        Some(unsafe { &*(raw as *const controls::tree_view::TreeView) })
    } else {
        None
    }
}

// ── Get selected node's name from tree ───────────────────────────────

fn get_selected_node_text() -> Option<Vec<u8>> {
    let st = state();
    let tree_id = unsafe { DIALOG_TREE_ID };
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == tree_id) {
        if let Some(tv) = as_tree_view_ref(ctrl) {
            if let Some(sel) = tv.selected() {
                let text = tv.node_text(sel);
                if !text.is_empty() {
                    return Some(text.to_vec());
                }
            }
        }
    }
    None
}

fn get_selected_index() -> Option<usize> {
    let st = state();
    let tree_id = unsafe { DIALOG_TREE_ID };
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == tree_id) {
        if let Some(tv) = as_tree_view_ref(ctrl) {
            return tv.selected();
        }
    }
    None
}

// ── Navigate into a directory ────────────────────────────────────────

fn navigate_to(name: &[u8]) {
    if name == b".." {
        // Go to parent
        let mut parent = [0u8; 257];
        let parent_len = unsafe {
            path_parent(&DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN], &mut parent)
        };
        unsafe {
            DIALOG_CURRENT_DIR[..parent_len].copy_from_slice(&parent[..parent_len]);
            DIALOG_CURRENT_DIR_LEN = parent_len;
        }
    } else {
        // Go into subdirectory
        let mut joined = [0u8; 257];
        let joined_len = unsafe {
            path_join(&DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN], name, &mut joined)
        };
        unsafe {
            DIALOG_CURRENT_DIR[..joined_len].copy_from_slice(&joined[..joined_len]);
            DIALOG_CURRENT_DIR_LEN = joined_len;
        }
    }
    let show_files = unsafe { DIALOG_SHOW_FILES };
    populate_file_list(show_files);
}

// ── Callbacks ────────────────────────────────────────────────────────

extern "C" fn dialog_cancel_clicked(_id: u32, _event_type: u32, _userdata: u64) {
    unsafe {
        DIALOG_RESULT_LEN = 0;
        DIALOG_DISMISSED = true;
    }
}

extern "C" fn dialog_confirm_clicked(_id: u32, _event_type: u32, _userdata: u64) {
    // For open_folder: confirm = select current dir
    // For open_file: confirm = select highlighted file/dir
    // For save_file: confirm = use filename field
    // For create_folder: confirm = create folder
    let userdata = _userdata;

    match userdata {
        0 => confirm_open_folder(),
        1 => confirm_open_file(),
        2 => confirm_save_file(),
        3 => confirm_create_folder(),
        _ => {}
    }
}

fn confirm_open_folder() {
    // Check if a directory is selected in the tree
    if let Some(sel_idx) = get_selected_index() {
        let is_dir = unsafe { sel_idx < DIALOG_ENTRY_COUNT && DIALOG_ENTRY_IS_DIR[sel_idx] };
        if is_dir {
            if let Some(name) = get_selected_node_text() {
                if name == b".." {
                    // Navigate to parent instead
                    navigate_to(b"..");
                    return;
                }
                // Build full path of selected dir
                let mut full = [0u8; 257];
                let full_len = unsafe {
                    path_join(&DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN], &name, &mut full)
                };
                unsafe {
                    DIALOG_RESULT[..full_len].copy_from_slice(&full[..full_len]);
                    DIALOG_RESULT_LEN = full_len;
                    DIALOG_DISMISSED = true;
                }
                return;
            }
        }
    }
    // No selection → use current directory
    unsafe {
        let len = DIALOG_CURRENT_DIR_LEN;
        DIALOG_RESULT[..len].copy_from_slice(&DIALOG_CURRENT_DIR[..len]);
        DIALOG_RESULT_LEN = len;
        DIALOG_DISMISSED = true;
    }
}

fn confirm_open_file() {
    if let Some(sel_idx) = get_selected_index() {
        let is_dir = unsafe { sel_idx < DIALOG_ENTRY_COUNT && DIALOG_ENTRY_IS_DIR[sel_idx] };
        if let Some(name) = get_selected_node_text() {
            if name == b".." {
                navigate_to(b"..");
                return;
            }
            if is_dir {
                // Navigate into directory
                navigate_to(&name);
                return;
            }
            // It's a file — select it
            let mut full = [0u8; 257];
            let full_len = unsafe {
                path_join(&DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN], &name, &mut full)
            };
            unsafe {
                DIALOG_RESULT[..full_len].copy_from_slice(&full[..full_len]);
                DIALOG_RESULT_LEN = full_len;
                DIALOG_DISMISSED = true;
            }
        }
    }
}

fn confirm_save_file() {
    // Get filename from TextField
    let st = state();
    let name_field_id = unsafe { DIALOG_NAME_FIELD_ID };
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == name_field_id) {
        let text = ctrl.text();
        if text.is_empty() { return; }
        let mut full = [0u8; 257];
        let full_len = unsafe {
            path_join(&DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN], text, &mut full)
        };
        unsafe {
            DIALOG_RESULT[..full_len].copy_from_slice(&full[..full_len]);
            DIALOG_RESULT_LEN = full_len;
            DIALOG_DISMISSED = true;
        }
    }
}

fn confirm_create_folder() {
    // Get folder name from TextField
    let st = state();
    let name_field_id = unsafe { DIALOG_NAME_FIELD_ID };
    if let Some(ctrl) = st.controls.iter().find(|c| c.id() == name_field_id) {
        let text = ctrl.text();
        if text.is_empty() { return; }
        let mut full = [0u8; 257];
        let full_len = unsafe {
            path_join(&DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN], text, &mut full)
        };
        // Null-terminate for mkdir syscall
        if full_len < 257 { full[full_len] = 0; }
        let result = syscall::mkdir(&full[..full_len + 1]);
        if result == 0 || (result as i32) >= 0 {
            unsafe {
                DIALOG_RESULT[..full_len].copy_from_slice(&full[..full_len]);
                DIALOG_RESULT_LEN = full_len;
                DIALOG_DISMISSED = true;
            }
        }
    }
}

extern "C" fn dialog_tree_double_click(_id: u32, _event_type: u32, _userdata: u64) {
    if let Some(sel_idx) = get_selected_index() {
        let is_dir = unsafe { sel_idx < DIALOG_ENTRY_COUNT && DIALOG_ENTRY_IS_DIR[sel_idx] };
        if let Some(name) = get_selected_node_text() {
            if name == b".." {
                navigate_to(b"..");
                return;
            }
            if is_dir {
                navigate_to(&name);
            } else {
                // Double-click on file in open_file mode → select it
                let show_files = unsafe { DIALOG_SHOW_FILES };
                if show_files {
                    confirm_open_file();
                }
            }
        }
    }
}

// ── Helper: add child to parent ──────────────────────────────────────

fn add_child_to_parent(parent_id: ControlId, child_id: ControlId) {
    let st = state();
    if let Some(p) = st.controls.iter_mut().find(|c| c.id() == parent_id) {
        p.add_child(child_id);
    }
}

// ── Helper: set dock on a control ────────────────────────────────────

fn set_control_dock(id: ControlId, dock: DockStyle) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().dock = dock;
        ctrl.base_mut().dirty = true;
    }
}

fn set_control_margin(id: ControlId, left: i32, top: i32, right: i32, bottom: i32) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        let b = ctrl.base_mut();
        b.margin.left = left;
        b.margin.top = top;
        b.margin.right = right;
        b.margin.bottom = bottom;
    }
}

// ── Common dialog creation ───────────────────────────────────────────

enum DialogType {
    OpenFolder,
    OpenFile,
    SaveFile,
    CreateFolder,
}

fn run_file_dialog(
    dialog_type: DialogType,
    default_name: &[u8],
) -> usize {
    let st = state();
    if st.windows.is_empty() { return 0; }

    let win_id = st.windows[0];
    let (win_w, win_h) = {
        let ctrl = st.controls.iter().find(|c| c.id() == win_id);
        match ctrl {
            Some(c) => (c.base().w, c.base().h),
            None => return 0,
        }
    };

    // Dialog dimensions
    let (card_w, card_h, title, confirm_label, show_files, has_name_field, confirm_userdata) = match dialog_type {
        DialogType::OpenFolder => (500u32, 400u32, b"Open Folder" as &[u8], b"Open" as &[u8], false, false, 0u64),
        DialogType::OpenFile => (500u32, 400u32, b"Open File" as &[u8], b"Open" as &[u8], true, false, 1u64),
        DialogType::SaveFile => (500u32, 400u32, b"Save File" as &[u8], b"Save" as &[u8], true, true, 2u64),
        DialogType::CreateFolder => (350u32, 200u32, b"New Folder" as &[u8], b"Create" as &[u8], false, true, 3u64),
    };

    let card_x = ((win_w as i32) - (card_w as i32)) / 2;
    let card_y = ((win_h as i32) - (card_h as i32)) / 2;

    // Initialize current directory
    let mut cwd_buf = [0u8; 257];
    let cwd_len = syscall::getcwd(&mut cwd_buf);
    let cwd_len = if cwd_len == u32::MAX || cwd_len == 0 {
        cwd_buf[0] = b'/';
        cwd_buf[1] = 0;
        1usize
    } else {
        cwd_len as usize
    };
    unsafe {
        DIALOG_CURRENT_DIR[..cwd_len].copy_from_slice(&cwd_buf[..cwd_len]);
        DIALOG_CURRENT_DIR_LEN = cwd_len;
        DIALOG_SHOW_FILES = show_files;
        DIALOG_RESULT_LEN = 0;
        DIALOG_DISMISSED = false;
    }

    // Allocate IDs
    let card_id = st.next_id; st.next_id += 1;
    let title_id = st.next_id; st.next_id += 1;
    let path_bar_id = st.next_id; st.next_id += 1;
    let path_label_id = st.next_id; st.next_id += 1;
    let bottom_bar_id = st.next_id; st.next_id += 1;
    let cancel_btn_id = st.next_id; st.next_id += 1;
    let confirm_btn_id = st.next_id; st.next_id += 1;
    let tree_id = st.next_id; st.next_id += 1;
    let name_field_id = if has_name_field { let id = st.next_id; st.next_id += 1; id } else { 0 };

    // Store IDs for callbacks
    unsafe {
        DIALOG_TREE_ID = tree_id;
        DIALOG_CARD_ID = card_id;
        DIALOG_PATH_LABEL_ID = path_label_id;
        DIALOG_NAME_FIELD_ID = name_field_id;
    }

    // ── Create card ──────────────────────────────────────────────────
    let card = controls::create_control(
        ControlKind::Card, card_id, win_id, card_x, card_y, card_w, card_h, &[],
    );
    st.controls.push(card);
    add_child_to_parent(win_id, card_id);

    // ── Title label ──────────────────────────────────────────────────
    let mut title_ctrl = controls::create_control(
        ControlKind::Label, title_id, card_id, 0, 0, card_w, 32, title,
    );
    title_ctrl.base_mut().dock = DockStyle::Top;
    title_ctrl.base_mut().margin.left = 16;
    title_ctrl.base_mut().margin.top = 12;
    title_ctrl.base_mut().margin.bottom = 4;
    title_ctrl.set_color(0xFFE0E0E0);
    st.controls.push(title_ctrl);
    add_child_to_parent(card_id, title_id);

    // ── Path bar ─────────────────────────────────────────────────────
    let mut path_bar = controls::create_control(
        ControlKind::View, path_bar_id, card_id, 0, 0, card_w, 24, &[],
    );
    path_bar.base_mut().dock = DockStyle::Top;
    path_bar.base_mut().margin.left = 12;
    path_bar.base_mut().margin.right = 12;
    path_bar.set_color(0xFF333333);
    st.controls.push(path_bar);
    add_child_to_parent(card_id, path_bar_id);

    // Path label inside path bar
    let mut path_label = controls::create_control(
        ControlKind::Label, path_label_id, path_bar_id, 0, 0, card_w - 24, 24,
        &cwd_buf[..cwd_len],
    );
    path_label.base_mut().dock = DockStyle::Fill;
    path_label.base_mut().margin.left = 8;
    path_label.set_color(0xFF9CDCFE);
    st.controls.push(path_label);
    add_child_to_parent(path_bar_id, path_label_id);

    // ── Bottom bar ───────────────────────────────────────────────────
    let mut bottom_bar = controls::create_control(
        ControlKind::View, bottom_bar_id, card_id, 0, 0, card_w, 44, &[],
    );
    bottom_bar.base_mut().dock = DockStyle::Bottom;
    bottom_bar.base_mut().margin.left = 12;
    bottom_bar.base_mut().margin.right = 12;
    bottom_bar.base_mut().margin.bottom = 8;
    bottom_bar.set_color(0x00000000); // transparent
    st.controls.push(bottom_bar);
    add_child_to_parent(card_id, bottom_bar_id);

    // Name field (Save/CreateFolder only)
    if has_name_field {
        let text_to_set = if !default_name.is_empty() { default_name } else { &[] };
        let field_w = if matches!(dialog_type, DialogType::CreateFolder) { card_w - 44 } else { card_w - 200 };
        let mut name_field = controls::create_control(
            ControlKind::TextField, name_field_id, bottom_bar_id, 0, 6, field_w, 28, text_to_set,
        );
        name_field.base_mut().dock = DockStyle::Left;
        name_field.base_mut().margin.right = 8;
        st.controls.push(name_field);
        add_child_to_parent(bottom_bar_id, name_field_id);
    }

    // Confirm button (right side)
    let mut confirm_btn = controls::create_control(
        ControlKind::Button, confirm_btn_id, bottom_bar_id,
        0, 6, 80, 30, confirm_label,
    );
    confirm_btn.base_mut().dock = DockStyle::Right;
    confirm_btn.set_color(0xFF0E639C); // blue accent
    st.controls.push(confirm_btn);
    add_child_to_parent(bottom_bar_id, confirm_btn_id);

    // Cancel button
    let mut cancel_btn = controls::create_control(
        ControlKind::Button, cancel_btn_id, bottom_bar_id,
        0, 6, 80, 30, b"Cancel",
    );
    cancel_btn.base_mut().dock = DockStyle::Right;
    cancel_btn.base_mut().margin.right = 8;
    st.controls.push(cancel_btn);
    add_child_to_parent(bottom_bar_id, cancel_btn_id);

    // ── TreeView (file list) ─────────────────────────────────────────
    let is_create_folder = matches!(dialog_type, DialogType::CreateFolder);
    if !is_create_folder {
        let tree_h = card_h.saturating_sub(120);
        let mut tree = controls::create_control(
            ControlKind::TreeView, tree_id, card_id, 0, 0, card_w, tree_h, &[],
        );
        tree.base_mut().dock = DockStyle::Fill;
        tree.base_mut().margin.left = 12;
        tree.base_mut().margin.right = 12;
        tree.base_mut().margin.top = 4;
        tree.base_mut().margin.bottom = 4;
        st.controls.push(tree);
        add_child_to_parent(card_id, tree_id);

        // Set tree row height
        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == tree_id) {
            if let Some(tv) = as_tree_view_mut(ctrl) {
                tv.row_height = 22;
                tv.indent_width = 0; // flat list, no indentation
            }
        }

        // Register double-click on tree
        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == tree_id) {
            ctrl.set_event_callback(EVENT_DOUBLE_CLICK, dialog_tree_double_click, 0);
        }

        // Populate file list
        populate_file_list(show_files);
    }

    // Register button callbacks
    if let Some(b) = st.controls.iter_mut().find(|c| c.id() == cancel_btn_id) {
        b.set_event_callback(EVENT_CLICK, dialog_cancel_clicked, 0);
    }
    if let Some(b) = st.controls.iter_mut().find(|c| c.id() == confirm_btn_id) {
        b.set_event_callback(EVENT_CLICK, dialog_confirm_clicked, confirm_userdata);
    }

    // ── Mini event loop ──────────────────────────────────────────────
    while !unsafe { DIALOG_DISMISSED } {
        let t0 = syscall::uptime_ms();
        if event_loop::run_once() == 0 { break; }
        let elapsed = syscall::uptime_ms().wrapping_sub(t0);
        if elapsed < 16 { syscall::sleep(16 - elapsed); }
    }

    // Clean up — remove card and all descendants
    crate::anyui_remove(card_id);

    unsafe { DIALOG_RESULT_LEN }
}

// ── Public API ───────────────────────────────────────────────────────

pub fn open_folder(result_buf: *mut u8, buf_len: u32) -> u32 {
    let len = run_file_dialog(DialogType::OpenFolder, &[]);
    if len == 0 { return 0; }
    let copy_len = len.min(buf_len as usize);
    if !result_buf.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(DIALOG_RESULT.as_ptr(), result_buf, copy_len);
        }
    }
    copy_len as u32
}

pub fn open_file(result_buf: *mut u8, buf_len: u32) -> u32 {
    let len = run_file_dialog(DialogType::OpenFile, &[]);
    if len == 0 { return 0; }
    let copy_len = len.min(buf_len as usize);
    if !result_buf.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(DIALOG_RESULT.as_ptr(), result_buf, copy_len);
        }
    }
    copy_len as u32
}

pub fn save_file(result_buf: *mut u8, buf_len: u32, default_name: &[u8]) -> u32 {
    let len = run_file_dialog(DialogType::SaveFile, default_name);
    if len == 0 { return 0; }
    let copy_len = len.min(buf_len as usize);
    if !result_buf.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(DIALOG_RESULT.as_ptr(), result_buf, copy_len);
        }
    }
    copy_len as u32
}

pub fn create_folder(result_buf: *mut u8, buf_len: u32) -> u32 {
    let len = run_file_dialog(DialogType::CreateFolder, &[]);
    if len == 0 { return 0; }
    let copy_len = len.min(buf_len as usize);
    if !result_buf.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(DIALOG_RESULT.as_ptr(), result_buf, copy_len);
        }
    }
    copy_len as u32
}
