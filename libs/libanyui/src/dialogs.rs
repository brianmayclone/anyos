//! macOS-style file/folder dialogs — OpenFolder, OpenFile, SaveFile, CreateFolder.
//!
//! Each dialog is a standalone compositor window that is set modal to the
//! calling window. Uses the same blocking mini event loop as MessageBox.

use crate::control::{
    Control, ControlId, ControlKind, DockStyle, EVENT_CHANGE, EVENT_CLICK, EVENT_DOUBLE_CLICK,
    EVENT_SUBMIT,
};
use crate::controls;
use crate::{event_loop, state, syscall};
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

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

#[inline(always)]
unsafe fn dialog_result_ptr() -> *const u8 {
    core::ptr::addr_of!(DIALOG_RESULT).cast::<u8>()
}

/// IDs of controls used by the dialog (so callbacks can find them).
static mut DIALOG_TREE_ID: ControlId = 0;
static mut DIALOG_CARD_ID: ControlId = 0;
static mut DIALOG_PATH_LABEL_ID: ControlId = 0;
static mut DIALOG_STATUS_LABEL_ID: ControlId = 0;
static mut DIALOG_NAME_FIELD_ID: ControlId = 0;
static mut DIALOG_CONFIRM_BTN_ID: ControlId = 0;
static mut DIALOG_SHOW_FILES: bool = true;
static mut DIALOG_MODE: u64 = 0;
static mut DIALOG_INITIAL_DIR: [u8; 257] = [0; 257];
static mut DIALOG_INITIAL_DIR_LEN: usize = 0;
static mut DIALOG_DIR_COUNT: usize = 0;
static mut DIALOG_FILE_COUNT: usize = 0;
static mut DIALOG_PLACE_BUTTON_IDS: [ControlId; 6] = [0; 6];

const PLACE_CURRENT: usize = 0;
const PLACE_LABELS: [&[u8]; 6] = [
    b"Working Dir",
    b"Root",
    b"Applications",
    b"Users",
    b"System",
    b"Libraries",
];

const PLACE_PATHS: [&[u8]; 6] = [
    b"",
    b"/",
    b"/Applications",
    b"/Users",
    b"/System",
    b"/Libraries",
];

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
    // Copy base
    let copy_len = base_len.min(255);
    out[..copy_len].copy_from_slice(&base[..copy_len]);
    let mut pos = copy_len;
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
    if pos < 257 {
        out[pos] = 0;
    }
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

fn copy_path_to_dialog(path: &[u8]) {
    let len = path.len().min(256);
    unsafe {
        DIALOG_CURRENT_DIR[..len].copy_from_slice(&path[..len]);
        DIALOG_CURRENT_DIR[len] = 0;
        DIALOG_CURRENT_DIR_LEN = len;
    }
}

fn init_dialog_dir() -> usize {
    let mut cwd_buf = [0u8; 257];
    let cwd_len = syscall::getcwd(&mut cwd_buf);
    let cwd_len = if cwd_len != u32::MAX && cwd_len > 0 {
        cwd_len as usize
    } else {
        cwd_buf[0] = b'/';
        cwd_buf[1] = 0;
        1
    };
    let cwd_len = cwd_len.min(256);
    unsafe {
        DIALOG_INITIAL_DIR[..cwd_len].copy_from_slice(&cwd_buf[..cwd_len]);
        DIALOG_INITIAL_DIR_LEN = cwd_len;
    }
    copy_path_to_dialog(&cwd_buf[..cwd_len]);
    cwd_len
}

fn set_control_text(id: ControlId, text: &[u8], text_color: Option<u32>) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.set_text(text);
        if let Some(color) = text_color {
            if let Some(tb) = ctrl.text_base_mut() {
                tb.text_style.text_color = color;
            }
        }
    }
}

fn set_control_disabled(id: ControlId, disabled: bool) {
    let st = state();
    if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == id) {
        ctrl.base_mut().disabled = disabled;
        ctrl.base_mut().mark_dirty();
    }
}

fn place_path(index: usize) -> &'static [u8] {
    if index == PLACE_CURRENT {
        unsafe { &DIALOG_INITIAL_DIR[..DIALOG_INITIAL_DIR_LEN] }
    } else {
        PLACE_PATHS[index]
    }
}

fn place_is_active(index: usize) -> bool {
    let path = unsafe { &DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN] };
    let target = place_path(index);
    if target.is_empty() {
        return false;
    }
    if target == b"/" {
        return path == b"/";
    }
    path == target || (path.starts_with(target) && path.get(target.len()) == Some(&b'/'))
}

fn sync_place_buttons() {
    let tc = crate::theme::colors();
    let st = state();
    let button_ids = unsafe { DIALOG_PLACE_BUTTON_IDS };
    for (index, &btn_id) in button_ids.iter().enumerate() {
        if btn_id == 0 {
            continue;
        }
        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == btn_id) {
            ctrl.set_color(if place_is_active(index) { tc.accent } else { 0 });
            if let Some(tb) = ctrl.text_base_mut() {
                tb.text_style.text_color = if place_is_active(index) {
                    0xFFFFFFFF
                } else {
                    tc.text
                };
            }
        }
    }
}

fn current_selection_name() -> Option<Vec<u8>> {
    let raw = get_selected_node_text()?;
    if raw == b".." {
        return None;
    }
    Some(strip_dir_suffix(&raw).to_vec())
}

fn sync_dialog_labels() {
    let tc = crate::theme::colors();
    let path_label_id = unsafe { DIALOG_PATH_LABEL_ID };
    let status_label_id = unsafe { DIALOG_STATUS_LABEL_ID };
    let current_dir = unsafe { &DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN] };
    set_control_text(path_label_id, current_dir, Some(tc.text_secondary));

    let mut status = if unsafe { DIALOG_MODE } == 3 {
        let location = core::str::from_utf8(current_dir).unwrap_or("/");
        format!("The new folder will be created in {}", location)
    } else {
        format!(
            "{} folders, {} files",
            unsafe { DIALOG_DIR_COUNT },
            unsafe { DIALOG_FILE_COUNT }
        )
    };
    if unsafe { DIALOG_MODE } != 3 {
        if let Some(name) = current_selection_name() {
            if let Ok(sel) = core::str::from_utf8(&name) {
                status.push_str("  •  ");
                status.push_str(sel);
            }
        }
    }
    set_control_text(status_label_id, status.as_bytes(), Some(tc.text_secondary));
    sync_place_buttons();
}

fn sync_name_field_from_selection() {
    if unsafe { DIALOG_MODE } != 2 {
        return;
    }
    let name_field_id = unsafe { DIALOG_NAME_FIELD_ID };
    if name_field_id == 0 {
        return;
    }
    if let Some(sel_idx) = get_selected_index() {
        let is_dir = unsafe { sel_idx < DIALOG_ENTRY_COUNT && DIALOG_ENTRY_IS_DIR[sel_idx] };
        if is_dir {
            return;
        }
    }
    if let Some(name) = current_selection_name() {
        let st = state();
        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == name_field_id) {
            ctrl.set_text(&name);
        }
    }
}

fn sync_confirm_button() {
    let confirm_btn_id = unsafe { DIALOG_CONFIRM_BTN_ID };
    if confirm_btn_id == 0 {
        return;
    }

    let disabled = match unsafe { DIALOG_MODE } {
        1 => get_selected_index().is_none(),
        2 | 3 => {
            let st = state();
            let name_field_id = unsafe { DIALOG_NAME_FIELD_ID };
            if let Some(ctrl) = st.controls.iter().find(|c| c.id() == name_field_id) {
                ctrl.text().is_empty()
            } else {
                true
            }
        }
        _ => false,
    };
    set_control_disabled(confirm_btn_id, disabled);
}

fn refresh_dialog_state(update_name_from_selection: bool) {
    if update_name_from_selection {
        sync_name_field_from_selection();
    }
    sync_dialog_labels();
    sync_confirm_button();
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
        if base + 64 > raw_buf.len() {
            break;
        }
        let entry_type = raw_buf[base];
        let name_len = (raw_buf[base + 1] as usize).min(56);
        let mut name = [0u8; 56];
        name[..name_len].copy_from_slice(&raw_buf[base + 8..base + 8 + name_len]);

        // Skip "." and ".."
        if name_len == 1 && name[0] == b'.' {
            continue;
        }
        if name_len == 2 && name[0] == b'.' && name[1] == b'.' {
            continue;
        }

        entries.push(DirEntry {
            name,
            name_len,
            is_dir: entry_type == 1, // 1 = directory in kernel FileType
        });
    }

    // Sort: directories first, then files, alphabetically within each group
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => core::cmp::Ordering::Less,
        (false, true) => core::cmp::Ordering::Greater,
        _ => a.name_slice().cmp(b.name_slice()),
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

    unsafe {
        DIALOG_ENTRY_COUNT = 0;
    }

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

    let mut num_dirs: usize = 0;
    let mut num_files: usize = 0;

    for entry in &entries {
        if !entry.is_dir && !show_files {
            continue;
        }
        let idx_in_tracking = unsafe { DIALOG_ENTRY_COUNT };
        if idx_in_tracking >= 256 {
            break;
        }

        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == tree_id) {
            if let Some(tv) = as_tree_view_mut(ctrl) {
                if entry.is_dir {
                    // Show directories with trailing "/" for clarity
                    let name = entry.name_slice();
                    let mut dir_label = [0u8; 58];
                    let nl = name.len().min(56);
                    dir_label[..nl].copy_from_slice(&name[..nl]);
                    dir_label[nl] = b'/';
                    let node_idx = tv.add_node(None, &dir_label[..nl + 1]);
                    tv.set_node_style(node_idx, 1); // bold
                    tv.set_node_text_color(node_idx, 0xFFE8E8E8);
                    num_dirs += 1;
                } else {
                    let node_idx = tv.add_node(None, entry.name_slice());
                    tv.set_node_text_color(node_idx, 0xFFA0A0A0);
                    num_files += 1;
                }
            }
        }

        unsafe {
            DIALOG_ENTRY_IS_DIR[idx_in_tracking] = entry.is_dir;
            DIALOG_ENTRY_COUNT += 1;
        }
    }

    unsafe {
        DIALOG_DIR_COUNT = num_dirs;
        DIALOG_FILE_COUNT = num_files;
    }
}

fn as_tree_view_mut(
    ctrl: &mut alloc::boxed::Box<dyn Control>,
) -> Option<&mut controls::tree_view::TreeView> {
    crate::control::cast_mut(ctrl, ControlKind::TreeView)
}

fn as_tree_view_ref(
    ctrl: &alloc::boxed::Box<dyn Control>,
) -> Option<&controls::tree_view::TreeView> {
    crate::control::cast_ref(ctrl, ControlKind::TreeView)
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

/// Strip trailing '/' from a directory name (as displayed in the tree).
fn strip_dir_suffix(name: &[u8]) -> &[u8] {
    if name.len() > 1 && name[name.len() - 1] == b'/' {
        &name[..name.len() - 1]
    } else {
        name
    }
}

// ── Navigate into a directory ────────────────────────────────────────

fn navigate_to(name: &[u8]) {
    if name == b".." {
        // Go to parent
        let mut parent = [0u8; 257];
        let parent_len =
            unsafe { path_parent(&DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN], &mut parent) };
        unsafe {
            DIALOG_CURRENT_DIR[..parent_len].copy_from_slice(&parent[..parent_len]);
            DIALOG_CURRENT_DIR_LEN = parent_len;
        }
    } else {
        // Go into subdirectory
        let mut joined = [0u8; 257];
        let joined_len = unsafe {
            path_join(
                &DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN],
                name,
                &mut joined,
            )
        };
        unsafe {
            DIALOG_CURRENT_DIR[..joined_len].copy_from_slice(&joined[..joined_len]);
            DIALOG_CURRENT_DIR_LEN = joined_len;
        }
    }
    let show_files = unsafe { DIALOG_SHOW_FILES };
    populate_file_list(show_files);
    refresh_dialog_state(false);
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
            if let Some(raw_name) = get_selected_node_text() {
                if raw_name == b".." {
                    navigate_to(b"..");
                    return;
                }
                let name = strip_dir_suffix(&raw_name);
                // Build full path of selected dir
                let mut full = [0u8; 257];
                let full_len = unsafe {
                    path_join(
                        &DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN],
                        name,
                        &mut full,
                    )
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
        if let Some(raw_name) = get_selected_node_text() {
            if raw_name == b".." {
                navigate_to(b"..");
                return;
            }
            let name = strip_dir_suffix(&raw_name);
            if is_dir {
                navigate_to(name);
                return;
            }
            // It's a file — select it
            let mut full = [0u8; 257];
            let full_len = unsafe {
                path_join(
                    &DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN],
                    name,
                    &mut full,
                )
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
        if text.is_empty() {
            return;
        }
        let mut full = [0u8; 257];
        let full_len = unsafe {
            path_join(
                &DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN],
                text,
                &mut full,
            )
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
        if text.is_empty() {
            return;
        }
        let mut full = [0u8; 257];
        let full_len = unsafe {
            path_join(
                &DIALOG_CURRENT_DIR[..DIALOG_CURRENT_DIR_LEN],
                text,
                &mut full,
            )
        };
        // Null-terminate for mkdir syscall
        if full_len < 257 {
            full[full_len] = 0;
        }
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

extern "C" fn dialog_up_clicked(_id: u32, _event_type: u32, _userdata: u64) {
    navigate_to(b"..");
}

extern "C" fn dialog_place_clicked(_id: u32, _event_type: u32, userdata: u64) {
    let index = userdata as usize;
    if index >= PLACE_PATHS.len() {
        return;
    }
    copy_path_to_dialog(place_path(index));
    populate_file_list(unsafe { DIALOG_SHOW_FILES });
    refresh_dialog_state(false);
}

extern "C" fn dialog_tree_double_click(_id: u32, _event_type: u32, _userdata: u64) {
    if let Some(sel_idx) = get_selected_index() {
        let is_dir = unsafe { sel_idx < DIALOG_ENTRY_COUNT && DIALOG_ENTRY_IS_DIR[sel_idx] };
        if let Some(raw_name) = get_selected_node_text() {
            if raw_name == b".." {
                navigate_to(b"..");
                return;
            }
            let name = strip_dir_suffix(&raw_name);
            if is_dir {
                navigate_to(name);
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

extern "C" fn dialog_tree_changed(_id: u32, _event_type: u32, _userdata: u64) {
    refresh_dialog_state(true);
}

extern "C" fn dialog_name_changed(_id: u32, _event_type: u32, _userdata: u64) {
    refresh_dialog_state(false);
}

extern "C" fn dialog_name_submit(_id: u32, _event_type: u32, _userdata: u64) {
    dialog_confirm_clicked(_id, _event_type, _userdata);
}

// ── Helper: add child to parent ──────────────────────────────────────

fn add_child_to_parent(parent_id: ControlId, child_id: ControlId) {
    let st = state();
    if let Some(p) = st.controls.iter_mut().find(|c| c.id() == parent_id) {
        p.add_child(child_id);
    }
    crate::mark_needs_layout();
}

// ── Common dialog creation ───────────────────────────────────────────

enum DialogType {
    OpenFolder,
    OpenFile,
    SaveFile,
    CreateFolder,
}

fn run_file_dialog(dialog_type: DialogType, default_name: &[u8]) -> usize {
    let st = state();
    let owner_win_id = st.windows.last().copied();

    let is_create_folder = matches!(dialog_type, DialogType::CreateFolder);
    let tc = crate::theme::colors();

    // Dialog dimensions and metadata
    let (dlg_w, dlg_h, title, confirm_label, show_files, has_name_field, confirm_userdata): (
        u32,
        u32,
        &[u8],
        &[u8],
        bool,
        bool,
        u64,
    ) = match dialog_type {
        DialogType::OpenFolder => (
            900,
            640,
            b"Open Folder" as &[u8],
            b"Open" as &[u8],
            false,
            false,
            0,
        ),
        DialogType::OpenFile => (
            900,
            640,
            b"Open File" as &[u8],
            b"Open" as &[u8],
            true,
            false,
            1,
        ),
        DialogType::SaveFile => (
            900,
            680,
            b"Save File" as &[u8],
            b"Save" as &[u8],
            true,
            true,
            2,
        ),
        DialogType::CreateFolder => (
            520,
            260,
            b"New Folder" as &[u8],
            b"Create" as &[u8],
            false,
            true,
            3,
        ),
    };

    // Initialize current directory from the process cwd.
    let cwd_len = init_dialog_dir();
    unsafe {
        DIALOG_SHOW_FILES = show_files;
        DIALOG_RESULT_LEN = 0;
        DIALOG_DISMISSED = false;
        DIALOG_MODE = confirm_userdata;
        DIALOG_STATUS_LABEL_ID = 0;
        DIALOG_NAME_FIELD_ID = 0;
        DIALOG_CONFIRM_BTN_ID = 0;
        DIALOG_DIR_COUNT = 0;
        DIALOG_FILE_COUNT = 0;
        DIALOG_PLACE_BUTTON_IDS = [0; 6];
    }

    // ── Create standalone dialog window, centered on owner ──────────
    // Flags: NOT_RESIZABLE(0x02) | NO_MINIMIZE(0x10) | NO_MAXIMIZE(0x20)
    let (dlg_x, dlg_y) = match owner_win_id {
        Some(id) => crate::center_on_owner(id, dlg_w, dlg_h),
        None => (-1, -1),
    };
    let dialog_win_id = crate::anyui_create_window(
        title.as_ptr(),
        title.len() as u32,
        dlg_x,
        dlg_y,
        dlg_w,
        dlg_h,
        0x02 | 0x10 | 0x20,
    );
    if dialog_win_id == 0 {
        return 0;
    }

    // Make it modal to the owner window
    if let Some(owner_id) = owner_win_id {
        crate::anyui_set_modal(dialog_win_id, owner_id);
    }

    // ── Allocate control IDs ─────────────────────────────────────────
    let st = state();
    let header_id = st.next_id;
    st.next_id += 1;
    let title_id = st.next_id;
    st.next_id += 1;
    let path_label_id = st.next_id;
    st.next_id += 1;
    let up_btn_id = if !is_create_folder {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let body_id = st.next_id;
    st.next_id += 1;
    let footer_id = st.next_id;
    st.next_id += 1;
    let status_label_id = st.next_id;
    st.next_id += 1;
    let cancel_btn_id = st.next_id;
    st.next_id += 1;
    let confirm_btn_id = st.next_id;
    st.next_id += 1;
    let name_row_id = if has_name_field {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let name_label_id = if has_name_field {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let name_field_id = if has_name_field {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let tree_id = if !is_create_folder {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let sidebar_id = if !is_create_folder {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let sidebar_title_id = if !is_create_folder {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let main_card_id = if !is_create_folder {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let main_title_id = if !is_create_folder {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let content_card_id = if is_create_folder {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let content_title_id = if is_create_folder {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let content_subtitle_id = if is_create_folder {
        let id = st.next_id;
        st.next_id += 1;
        id
    } else {
        0
    };
    let mut place_btn_ids = [0u32; 6];
    if !is_create_folder {
        for id in &mut place_btn_ids {
            *id = st.next_id;
            st.next_id += 1;
        }
    }

    // Store IDs for callbacks
    unsafe {
        DIALOG_TREE_ID = tree_id;
        DIALOG_CARD_ID = dialog_win_id;
        DIALOG_PATH_LABEL_ID = path_label_id;
        DIALOG_STATUS_LABEL_ID = status_label_id;
        DIALOG_NAME_FIELD_ID = name_field_id;
        DIALOG_CONFIRM_BTN_ID = confirm_btn_id;
        DIALOG_PLACE_BUTTON_IDS = place_btn_ids;
    }

    // ── Header ────────────────────────────────────────────────────────
    let mut header = controls::create_control(
        ControlKind::View,
        header_id,
        dialog_win_id,
        0,
        0,
        dlg_w,
        82,
        &[],
    );
    header.base_mut().dock = DockStyle::Top;
    header.set_color(tc.toolbar_bg);
    st.controls.push(header);
    add_child_to_parent(dialog_win_id, header_id);

    if !is_create_folder {
        let mut up_btn = controls::create_control(
            ControlKind::Button,
            up_btn_id,
            header_id,
            0,
            20,
            86,
            34,
            b"Up",
        );
        up_btn.base_mut().dock = DockStyle::Right;
        up_btn.base_mut().margin.top = 22;
        up_btn.base_mut().margin.right = 18;
        st.controls.push(up_btn);
        add_child_to_parent(header_id, up_btn_id);
    }

    let mut title_label = controls::create_control(
        ControlKind::Label,
        title_id,
        header_id,
        0,
        0,
        dlg_w,
        28,
        title,
    );
    title_label.base_mut().dock = DockStyle::Top;
    title_label.base_mut().margin.left = 20;
    title_label.base_mut().margin.top = 14;
    title_label.base_mut().margin.right = if is_create_folder { 20 } else { 118 };
    if let Some(tb) = title_label.text_base_mut() {
        tb.text_style.font_id = 1;
        tb.text_style.font_size = 18;
        tb.text_style.text_color = tc.text;
    }
    st.controls.push(title_label);
    add_child_to_parent(header_id, title_id);

    let mut path_label = controls::create_control(
        ControlKind::Label,
        path_label_id,
        header_id,
        0,
        0,
        dlg_w,
        24,
        unsafe { &DIALOG_CURRENT_DIR[..cwd_len] },
    );
    path_label.base_mut().dock = DockStyle::Fill;
    path_label.base_mut().margin.left = 20;
    path_label.base_mut().margin.right = if is_create_folder { 20 } else { 118 };
    path_label.base_mut().margin.bottom = 14;
    if let Some(tb) = path_label.text_base_mut() {
        tb.text_style.font_size = 12;
        tb.text_style.text_color = tc.text_secondary;
    }
    st.controls.push(path_label);
    add_child_to_parent(header_id, path_label_id);

    // ── Footer ────────────────────────────────────────────────────────
    let footer_h = if has_name_field { 104 } else { 60 };
    let mut footer = controls::create_control(
        ControlKind::View,
        footer_id,
        dialog_win_id,
        0,
        0,
        dlg_w,
        footer_h,
        &[],
    );
    footer.base_mut().dock = DockStyle::Bottom;
    footer.set_color(tc.toolbar_bg);
    st.controls.push(footer);
    add_child_to_parent(dialog_win_id, footer_id);

    if has_name_field {
        let mut name_row = controls::create_control(
            ControlKind::View,
            name_row_id,
            footer_id,
            0,
            0,
            dlg_w,
            42,
            &[],
        );
        name_row.base_mut().dock = DockStyle::Top;
        name_row.base_mut().margin.left = 18;
        name_row.base_mut().margin.right = 18;
        name_row.base_mut().margin.top = 12;
        st.controls.push(name_row);
        add_child_to_parent(footer_id, name_row_id);

        let mut name_label = controls::create_control(
            ControlKind::Label,
            name_label_id,
            name_row_id,
            0,
            8,
            78,
            30,
            if is_create_folder {
                b"Folder"
            } else {
                b"File name"
            },
        );
        name_label.base_mut().dock = DockStyle::Left;
        if let Some(tb) = name_label.text_base_mut() {
            tb.text_style.text_color = tc.text_secondary;
        }
        st.controls.push(name_label);
        add_child_to_parent(name_row_id, name_label_id);

        let mut name_field = controls::create_control(
            ControlKind::TextField,
            name_field_id,
            name_row_id,
            0,
            8,
            dlg_w,
            30,
            default_name,
        );
        name_field.base_mut().dock = DockStyle::Fill;
        name_field.base_mut().margin.left = 10;
        if let Some(tb) = name_field.text_base_mut() {
            tb.text_style.font_size = 13;
        }
        st.controls.push(name_field);
        add_child_to_parent(name_row_id, name_field_id);
        crate::anyui_textfield_set_placeholder(
            name_field_id,
            if is_create_folder {
                b"Folder name".as_ptr()
            } else {
                b"Choose a file name".as_ptr()
            },
            if is_create_folder { 11 } else { 18 },
        );
    }

    let mut cancel_btn = controls::create_control(
        ControlKind::Button,
        cancel_btn_id,
        footer_id,
        0,
        0,
        94,
        32,
        b"Cancel",
    );
    cancel_btn.base_mut().dock = DockStyle::Right;
    cancel_btn.base_mut().margin.right = 18;
    cancel_btn.base_mut().margin.bottom = 10;
    st.controls.push(cancel_btn);
    add_child_to_parent(footer_id, cancel_btn_id);

    let mut confirm_btn = controls::create_control(
        ControlKind::Button,
        confirm_btn_id,
        footer_id,
        0,
        0,
        104,
        32,
        confirm_label,
    );
    confirm_btn.base_mut().dock = DockStyle::Right;
    confirm_btn.base_mut().margin.right = 10;
    confirm_btn.base_mut().margin.bottom = 10;
    confirm_btn.set_color(tc.accent);
    st.controls.push(confirm_btn);
    add_child_to_parent(footer_id, confirm_btn_id);

    let mut status_label = controls::create_control(
        ControlKind::Label,
        status_label_id,
        footer_id,
        0,
        0,
        dlg_w,
        42,
        &[],
    );
    status_label.base_mut().dock = DockStyle::Fill;
    status_label.base_mut().margin.left = 18;
    status_label.base_mut().margin.right = 18;
    status_label.base_mut().margin.bottom = 10;
    if let Some(tb) = status_label.text_base_mut() {
        tb.text_style.font_size = 12;
        tb.text_style.text_color = tc.text_secondary;
    }
    st.controls.push(status_label);
    add_child_to_parent(footer_id, status_label_id);

    // ── Body/content ──────────────────────────────────────────────────
    let mut body = controls::create_control(
        ControlKind::View,
        body_id,
        dialog_win_id,
        0,
        0,
        dlg_w,
        dlg_h,
        &[],
    );
    body.base_mut().dock = DockStyle::Fill;
    body.set_color(tc.window_bg);
    st.controls.push(body);
    add_child_to_parent(dialog_win_id, body_id);

    if !is_create_folder {
        let mut sidebar = controls::create_control(
            ControlKind::View,
            sidebar_id,
            body_id,
            0,
            0,
            196,
            dlg_h,
            &[],
        );
        sidebar.base_mut().dock = DockStyle::Left;
        sidebar.base_mut().margin.left = 16;
        sidebar.base_mut().margin.top = 16;
        sidebar.base_mut().margin.bottom = 16;
        sidebar.set_color(tc.sidebar_bg);
        st.controls.push(sidebar);
        add_child_to_parent(body_id, sidebar_id);

        let mut sidebar_title = controls::create_control(
            ControlKind::Label,
            sidebar_title_id,
            sidebar_id,
            0,
            0,
            160,
            24,
            b"Places",
        );
        sidebar_title.base_mut().dock = DockStyle::Top;
        sidebar_title.base_mut().margin.left = 14;
        sidebar_title.base_mut().margin.top = 14;
        sidebar_title.base_mut().margin.bottom = 8;
        if let Some(tb) = sidebar_title.text_base_mut() {
            tb.text_style.font_id = 1;
            tb.text_style.text_color = tc.text_secondary;
        }
        st.controls.push(sidebar_title);
        add_child_to_parent(sidebar_id, sidebar_title_id);

        for (index, (&btn_id, &label)) in place_btn_ids.iter().zip(PLACE_LABELS.iter()).enumerate()
        {
            let mut btn = controls::create_control(
                ControlKind::Button,
                btn_id,
                sidebar_id,
                0,
                0,
                156,
                32,
                label,
            );
            btn.base_mut().dock = DockStyle::Top;
            btn.base_mut().margin.left = 12;
            btn.base_mut().margin.right = 12;
            btn.base_mut().margin.bottom = 8;
            if index == PLACE_CURRENT {
                btn.base_mut().margin.top = 2;
            }
            st.controls.push(btn);
            add_child_to_parent(sidebar_id, btn_id);
        }

        let mut main_card = controls::create_control(
            ControlKind::View,
            main_card_id,
            body_id,
            0,
            0,
            dlg_w,
            dlg_h,
            &[],
        );
        main_card.base_mut().dock = DockStyle::Fill;
        main_card.base_mut().margin.left = 14;
        main_card.base_mut().margin.right = 16;
        main_card.base_mut().margin.top = 16;
        main_card.base_mut().margin.bottom = 16;
        main_card.set_color(tc.card_bg);
        st.controls.push(main_card);
        add_child_to_parent(body_id, main_card_id);

        let mut main_title = controls::create_control(
            ControlKind::Label,
            main_title_id,
            main_card_id,
            0,
            0,
            120,
            24,
            if show_files {
                b"Folders and files"
            } else {
                b"Folders"
            },
        );
        main_title.base_mut().dock = DockStyle::Top;
        main_title.base_mut().margin.left = 16;
        main_title.base_mut().margin.top = 16;
        main_title.base_mut().margin.bottom = 10;
        if let Some(tb) = main_title.text_base_mut() {
            tb.text_style.font_id = 1;
            tb.text_style.text_color = tc.text_secondary;
        }
        st.controls.push(main_title);
        add_child_to_parent(main_card_id, main_title_id);

        let mut tree = controls::create_control(
            ControlKind::TreeView,
            tree_id,
            main_card_id,
            0,
            0,
            dlg_w,
            400,
            &[],
        );
        tree.base_mut().dock = DockStyle::Fill;
        tree.base_mut().margin.left = 16;
        tree.base_mut().margin.right = 16;
        tree.base_mut().margin.top = 0;
        tree.base_mut().margin.bottom = 16;
        st.controls.push(tree);
        add_child_to_parent(main_card_id, tree_id);

        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == tree_id) {
            if let Some(tv) = as_tree_view_mut(ctrl) {
                tv.row_height = 30;
                tv.indent_width = 0;
            }
        }

        if let Some(ctrl) = st.controls.iter_mut().find(|c| c.id() == tree_id) {
            ctrl.set_event_callback(EVENT_DOUBLE_CLICK, dialog_tree_double_click, 0);
            ctrl.set_event_callback(EVENT_CHANGE, dialog_tree_changed, 0);
            ctrl.set_event_callback(EVENT_SUBMIT, dialog_confirm_clicked, confirm_userdata);
        }

        populate_file_list(show_files);
    } else {
        let mut content_card = controls::create_control(
            ControlKind::View,
            content_card_id,
            body_id,
            0,
            0,
            dlg_w,
            dlg_h,
            &[],
        );
        content_card.base_mut().dock = DockStyle::Fill;
        content_card.base_mut().margin.left = 18;
        content_card.base_mut().margin.right = 18;
        content_card.base_mut().margin.top = 18;
        content_card.base_mut().margin.bottom = 18;
        content_card.set_color(tc.card_bg);
        st.controls.push(content_card);
        add_child_to_parent(body_id, content_card_id);

        let mut content_title = controls::create_control(
            ControlKind::Label,
            content_title_id,
            content_card_id,
            0,
            0,
            200,
            26,
            b"Create folder in",
        );
        content_title.base_mut().dock = DockStyle::Top;
        content_title.base_mut().margin.left = 18;
        content_title.base_mut().margin.top = 18;
        if let Some(tb) = content_title.text_base_mut() {
            tb.text_style.font_id = 1;
            tb.text_style.text_color = tc.text_secondary;
        }
        st.controls.push(content_title);
        add_child_to_parent(content_card_id, content_title_id);

        let mut content_subtitle = controls::create_control(
            ControlKind::Label,
            content_subtitle_id,
            content_card_id,
            0,
            0,
            300,
            24,
            unsafe { &DIALOG_CURRENT_DIR[..cwd_len] },
        );
        content_subtitle.base_mut().dock = DockStyle::Top;
        content_subtitle.base_mut().margin.left = 18;
        content_subtitle.base_mut().margin.bottom = 16;
        if let Some(tb) = content_subtitle.text_base_mut() {
            tb.text_style.font_size = 13;
            tb.text_style.text_color = tc.text;
        }
        st.controls.push(content_subtitle);
        add_child_to_parent(content_card_id, content_subtitle_id);
    }

    // ── Register callbacks ───────────────────────────────────────────
    if let Some(b) = st.controls.iter_mut().find(|c| c.id() == cancel_btn_id) {
        b.set_event_callback(EVENT_CLICK, dialog_cancel_clicked, 0);
    }
    if let Some(b) = st.controls.iter_mut().find(|c| c.id() == confirm_btn_id) {
        b.set_event_callback(EVENT_CLICK, dialog_confirm_clicked, confirm_userdata);
    }
    if let Some(f) = st.controls.iter_mut().find(|c| c.id() == name_field_id) {
        f.set_event_callback(EVENT_CHANGE, dialog_name_changed, 0);
        f.set_event_callback(EVENT_SUBMIT, dialog_name_submit, confirm_userdata);
    }
    if up_btn_id != 0 {
        if let Some(b) = st.controls.iter_mut().find(|c| c.id() == up_btn_id) {
            b.set_event_callback(EVENT_CLICK, dialog_up_clicked, 0);
        }
    }
    for (index, &btn_id) in place_btn_ids.iter().enumerate() {
        if btn_id == 0 {
            continue;
        }
        if let Some(b) = st.controls.iter_mut().find(|c| c.id() == btn_id) {
            b.set_event_callback(EVENT_CLICK, dialog_place_clicked, index as u64);
        }
    }
    // Window close button (X) → same as Cancel
    if let Some(b) = st.controls.iter_mut().find(|c| c.id() == dialog_win_id) {
        b.set_event_callback(crate::control::EVENT_CLOSE, dialog_cancel_clicked, 0);
    }

    if name_field_id != 0 {
        if default_name.is_empty() {
            crate::anyui_set_focus(name_field_id);
        } else {
            crate::anyui_textfield_select_all(name_field_id);
            crate::anyui_set_focus(name_field_id);
        }
    } else if tree_id != 0 {
        crate::anyui_set_focus(tree_id);
    }

    refresh_dialog_state(false);

    // ── Mini event loop ──────────────────────────────────────────────
    while !unsafe { DIALOG_DISMISSED } {
        let t0 = syscall::uptime_ms();
        if event_loop::run_once() == 0 {
            break;
        }
        let elapsed = syscall::uptime_ms().wrapping_sub(t0);
        if elapsed < 16 {
            syscall::sleep(16 - elapsed);
        }
    }

    // Destroy dialog window — auto-clears modal + removes all child controls
    crate::anyui_destroy_window(dialog_win_id);

    unsafe { DIALOG_RESULT_LEN }
}

// ── Public API ───────────────────────────────────────────────────────

pub fn open_folder(result_buf: *mut u8, buf_len: u32) -> u32 {
    let len = run_file_dialog(DialogType::OpenFolder, &[]);
    if len == 0 {
        return 0;
    }
    let copy_len = len.min(buf_len as usize);
    if !result_buf.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(dialog_result_ptr(), result_buf, copy_len);
        }
    }
    copy_len as u32
}

pub fn open_file(result_buf: *mut u8, buf_len: u32) -> u32 {
    let len = run_file_dialog(DialogType::OpenFile, &[]);
    if len == 0 {
        return 0;
    }
    let copy_len = len.min(buf_len as usize);
    if !result_buf.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(dialog_result_ptr(), result_buf, copy_len);
        }
    }
    copy_len as u32
}

pub fn save_file(result_buf: *mut u8, buf_len: u32, default_name: &[u8]) -> u32 {
    let len = run_file_dialog(DialogType::SaveFile, default_name);
    if len == 0 {
        return 0;
    }
    let copy_len = len.min(buf_len as usize);
    if !result_buf.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(dialog_result_ptr(), result_buf, copy_len);
        }
    }
    copy_len as u32
}

pub fn create_folder(result_buf: *mut u8, buf_len: u32) -> u32 {
    let len = run_file_dialog(DialogType::CreateFolder, &[]);
    if len == 0 {
        return 0;
    }
    let copy_len = len.min(buf_len as usize);
    if !result_buf.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(dialog_result_ptr(), result_buf, copy_len);
        }
    }
    copy_len as u32
}
