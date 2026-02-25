#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::fs;
use anyos_std::icons;
use anyos_std::process;

use libanyui_client as ui;
use ui::{Icon, Widget};

anyos_std::entry!(main);

// ============================================================================
// Constants
// ============================================================================

const TYPE_FILE: u8 = 0;
const TYPE_DIR: u8 = 1;

const ICON_SIZE_SMALL: u32 = 16;
const ICON_SIZE_LARGE: u32 = 48;
const GRID_CELL_W: u32 = 90;
const GRID_CELL_H: u32 = 80;

const MAX_LOCATIONS: usize = 16;

// Default sidebar locations (used if no finder.conf found)
const DEFAULT_LOCATIONS: [(&str, &str); 6] = [
    ("Root", "/"),
    ("Applications", "/Applications"),
    ("Programs", "/System/bin"),
    ("System", "/System"),
    ("Libraries", "/Libraries"),
    ("Icons", "/System/icons"),
];

const FINDER_CONF_PATH: &str = "/System/compositor/finder.conf";

// View modes
const VIEW_LIST: u32 = 0;
const VIEW_ICONS: u32 = 1;

// ============================================================================
// Sidebar location (loaded from config)
// ============================================================================

struct Location {
    name: String,
    path: String,
}

fn load_locations() -> Vec<Location> {
    let mut locations = Vec::new();

    let fd = fs::open(FINDER_CONF_PATH, 0);
    if fd == u32::MAX {
        // No config file, use defaults
        for &(name, path) in &DEFAULT_LOCATIONS {
            locations.push(Location {
                name: String::from(name),
                path: String::from(path),
            });
        }
        return locations;
    }

    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);

    // Parse: each line is "Name=Path" or "#comment" or empty
    let mut i = 0;
    while i < data.len() && locations.len() < MAX_LOCATIONS {
        let line_start = i;
        while i < data.len() && data[i] != b'\n' { i += 1; }
        let mut line = &data[line_start..i];
        if i < data.len() { i += 1; }

        // Trim \r
        if line.last() == Some(&b'\r') { line = &line[..line.len() - 1]; }
        if line.is_empty() || line[0] == b'#' { continue; }

        if let Some(eq_pos) = line.iter().position(|&b| b == b'=') {
            let name_bytes = &line[..eq_pos];
            let path_bytes = &line[eq_pos + 1..];
            if let (Ok(name), Ok(path)) = (
                core::str::from_utf8(name_bytes),
                core::str::from_utf8(path_bytes),
            ) {
                let name = name.trim();
                let path = path.trim();
                if !name.is_empty() && !path.is_empty() {
                    locations.push(Location {
                        name: String::from(name),
                        path: String::from(path),
                    });
                }
            }
        }
    }

    if locations.is_empty() {
        // Fallback to defaults
        for &(name, path) in &DEFAULT_LOCATIONS {
            locations.push(Location {
                name: String::from(name),
                path: String::from(path),
            });
        }
    }

    locations
}

// ============================================================================
// File entry
// ============================================================================

struct FileEntry {
    name: [u8; 56],
    name_len: usize,
    entry_type: u8,
    size: u32,
}

impl FileEntry {
    fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("???")
    }

    fn is_app_bundle(&self) -> bool {
        self.entry_type == TYPE_DIR && self.name_str().ends_with(".app")
    }
}

// ============================================================================
// Icon cache — loads, decodes, and ensures icons are exactly target_size
// ============================================================================

struct CachedIcon {
    path: String,
    pixels: Vec<u32>,
    width: u32,
    height: u32,
}

struct IconCache {
    entries: Vec<CachedIcon>,
}

impl IconCache {
    fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Load an icon, ensuring it is exactly `target_size x target_size` pixels.
    fn get_or_load(&mut self, path: &str, target_size: u32) -> Option<(&[u32], u32, u32)> {
        // Build cache key: path + size
        if let Some(idx) = self.entries.iter().position(|e| e.path == path && e.width == target_size) {
            let e = &self.entries[idx];
            return Some((&e.pixels, e.width, e.height));
        }

        // Load via Icon API (selects best ICO variant for preferred_size)
        let icon = Icon::load(path, target_size)?;

        // If the icon isn't the exact target size, scale it
        let (final_pixels, final_w, final_h) = if icon.width == target_size && icon.height == target_size {
            (icon.pixels, icon.width, icon.height)
        } else {
            let pixel_count = (target_size * target_size) as usize;
            let mut scaled = Vec::new();
            scaled.resize(pixel_count, 0u32);
            libimage_client::scale_image(
                &icon.pixels, icon.width, icon.height,
                &mut scaled, target_size, target_size,
                libimage_client::MODE_CONTAIN,
            );
            (scaled, target_size, target_size)
        };

        self.entries.push(CachedIcon {
            path: String::from(path),
            pixels: final_pixels,
            width: final_w,
            height: final_h,
        });
        let e = self.entries.last().unwrap();
        Some((&e.pixels, e.width, e.height))
    }
}

// ============================================================================
// Application state
// ============================================================================

struct AppState {
    cwd: String,
    entries: Vec<FileEntry>,
    history: Vec<String>,
    history_pos: usize,
    sidebar_sel: usize,
    view_mode: u32,
    mimetypes: icons::MimeDb,
    icon_cache: IconCache,
    locations: Vec<Location>,
    // UI controls
    grid: ui::DataGrid,
    icon_scroll: ui::ScrollView,
    icon_flow: ui::FlowPanel,
    icon_item_ids: Vec<u32>,   // track created items for cleanup
    icon_selected: Vec<bool>,  // per-item selection state in icon view
    icon_anchor: usize,        // anchor index for Shift+Click range selection
    path_field: ui::TextField,
    btn_back: ui::IconButton,
    btn_fwd: ui::IconButton,
    sidebar_item_ids: Vec<u32>,
    // Status bar
    sb_items_label: ui::Label,
    sb_sel_label: ui::Label,
    // Context menu
    ctx_menu: ui::ContextMenu,
    ctx_menu_variant: u32, // 0=app, 1=dir, 2=file — set at right-click time
    // Rename panel
    rename_panel: ui::View,
    rename_field: ui::TextField,
    rename_active: bool,
    rename_idx: usize,
    // Clipboard for file copy/paste
    clip_files: Vec<String>,
    clip_is_cut: bool,
    // Copy/move operation state
    copy_op: Option<CopyOperation>,
}

/// State for an ongoing copy/move operation (timer-driven).
struct CopyOperation {
    sources: Vec<String>,
    dest_dir: String,
    is_cut: bool,
    current_idx: usize,
    total_files: u32,
    done_files: u32,
    cancelled: bool,
    // Dialog widgets
    win_id: u32,
    file_label_id: u32,
    progress_id: u32,
    count_label_id: u32,
    timer_id: u32,
}

static mut APP: Option<AppState> = None;

fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

// ============================================================================
// Directory reading
// ============================================================================

fn read_directory(path: &str) -> Vec<FileEntry> {
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

// ============================================================================
// Path utilities
// ============================================================================

fn build_full_path(cwd: &str, name: &str) -> String {
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

fn parent_path(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/",
        Some(pos) => &path[..pos],
        None => "/",
    }
}

fn get_extension(name: &str) -> Option<&str> {
    match name.rfind('.') {
        Some(pos) if pos + 1 < name.len() => Some(&name[pos + 1..]),
        _ => None,
    }
}

fn fmt_size(size: u32) -> String {
    if size >= 1024 * 1024 {
        let mb = size / (1024 * 1024);
        let frac = ((size % (1024 * 1024)) * 10 / (1024 * 1024)).min(9);
        anyos_std::format!("{}.{} MB", mb, frac)
    } else if size >= 1024 {
        let kb = size / 1024;
        anyos_std::format!("{} KB", kb)
    } else {
        anyos_std::format!("{} B", size)
    }
}

// ============================================================================
// File name helpers
// ============================================================================

/// Split "file.txt" into ("file", ".txt"). If no extension, ext is "".
fn split_name_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(pos) if pos > 0 => (&name[..pos], &name[pos..]),
        _ => (name, ""),
    }
}

/// Generate a unique destination path. If `dir/name` exists, try "Copy of name",
/// "Copy 2 of name", etc.
fn unique_dest_path(dir: &str, name: &str) -> String {
    let dest = build_full_path(dir, name);
    let mut stat_buf = [0u32; 7];
    if fs::stat(&dest, &mut stat_buf) != 0 {
        return dest; // Doesn't exist, use as-is
    }
    let (stem, ext) = split_name_ext(name);
    for i in 1..100u32 {
        let new_name = if i == 1 {
            anyos_std::format!("Copy of {}{}", stem, ext)
        } else {
            anyos_std::format!("Copy {} of {}{}", i, stem, ext)
        };
        let p = build_full_path(dir, &new_name);
        if fs::stat(&p, &mut stat_buf) != 0 {
            return p;
        }
    }
    build_full_path(dir, &anyos_std::format!("Copy of {}", name))
}

// ============================================================================
// Trash helpers
// ============================================================================

fn get_trash_dir() -> String {
    let uid = anyos_std::process::getuid();
    let mut name_buf = [0u8; 64];
    let nlen = anyos_std::process::getusername(uid, &mut name_buf);
    let trash = if nlen != u32::MAX && nlen > 0 {
        if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
            anyos_std::format!("/Users/{}/.Trash", username)
        } else {
            String::from("/tmp/.Trash")
        }
    } else {
        let mut home_buf = [0u8; 256];
        let len = anyos_std::env::get("HOME", &mut home_buf);
        if len != u32::MAX && len > 0 {
            if let Ok(h) = core::str::from_utf8(&home_buf[..len as usize]) {
                anyos_std::format!("{}/.Trash", h)
            } else {
                String::from("/tmp/.Trash")
            }
        } else {
            String::from("/tmp/.Trash")
        }
    };
    // Ensure directory exists
    let mut stat_buf = [0u32; 7];
    if fs::stat(&trash, &mut stat_buf) != 0 {
        fs::mkdir(&trash);
    }
    trash
}

fn trash_info_path(trash_dir: &str) -> String {
    anyos_std::format!("{}/.trash_info.json", trash_dir)
}

fn load_trash_info(trash_dir: &str) -> anyos_std::json::Value {
    use anyos_std::json::Value;
    let path = trash_info_path(trash_dir);
    let fd = fs::open(&path, 0);
    if fd == u32::MAX {
        let mut root = Value::new_object();
        root.set("entries", Value::new_array());
        return root;
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    if let Ok(text) = core::str::from_utf8(&data) {
        if let Ok(val) = Value::parse(text) {
            return val;
        }
    }
    let mut root = Value::new_object();
    root.set("entries", Value::new_array());
    root
}

fn save_trash_info(trash_dir: &str, info: &anyos_std::json::Value) {
    let path = trash_info_path(trash_dir);
    let json_str = info.to_json_string_pretty();
    let _ = fs::write_bytes(&path, json_str.as_bytes());
}

fn now_string() -> String {
    let mut buf = [0u8; 8];
    anyos_std::sys::time(&mut buf);
    let year = buf[0] as u16 | ((buf[1] as u16) << 8);
    let month = buf[2];
    let day = buf[3];
    let hour = buf[4];
    let min = buf[5];
    let sec = buf[6];
    anyos_std::format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hour, min, sec)
}

/// Move a single file/dir to trash. Returns true on success.
fn move_to_trash_single(source_path: &str) -> bool {
    let trash_dir = get_trash_dir();
    let name = source_path.rsplit('/').next().unwrap_or("");
    if name.is_empty() { return false; }

    // Generate unique name in trash
    let trash_path = unique_dest_path(&trash_dir, name);
    let trash_name = trash_path.rsplit('/').next().unwrap_or(name);

    // Move file to trash
    if fs::rename(source_path, &trash_path) == u32::MAX {
        return false;
    }

    // Record metadata
    use anyos_std::json::Value;
    let mut info = load_trash_info(&trash_dir);
    let mut entry = Value::new_object();
    entry.set("name", Value::from(trash_name));
    entry.set("original_path", Value::from(source_path));
    entry.set("deleted_at", Value::from(now_string().as_str()));
    // Get mutable access to the "entries" array and push the new entry
    if let Some(obj) = info.as_object_mut() {
        if let Some(entries_val) = obj.get_mut("entries") {
            entries_val.push(entry);
        }
    }
    save_trash_info(&trash_dir, &info);
    true
}

// ============================================================================
// Recursive file copy
// ============================================================================

/// Copy a single file. Returns true on success.
fn copy_file(src: &str, dst: &str) -> bool {
    let fd = fs::open(src, 0);
    if fd == u32::MAX { return false; }
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    fs::write_bytes(dst, &data).is_ok()
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &str, dst: &str) -> bool {
    fs::mkdir(dst);
    let mut buf = [0u8; 256 * 64];
    let count = fs::readdir(src, &mut buf);
    if count == u32::MAX { return false; }

    for i in 0..count as usize {
        let off = i * 64;
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        let name_bytes = &buf[off + 8..off + 8 + name_len.min(55)];
        let name = match core::str::from_utf8(name_bytes) {
            Ok(n) => n,
            Err(_) => continue,
        };
        if name == "." || name == ".." { continue; }

        let child_src = build_full_path(src, name);
        let child_dst = build_full_path(dst, name);

        if entry_type == TYPE_DIR {
            if !copy_dir_recursive(&child_src, &child_dst) {
                return false;
            }
        } else {
            if !copy_file(&child_src, &child_dst) {
                return false;
            }
        }
    }
    true
}

/// Count files recursively in a path (file=1, dir=recurse).
#[allow(dead_code)]
fn count_files_recursive(path: &str) -> u32 {
    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) != 0 { return 0; }
    if stat_buf[0] != TYPE_DIR as u32 { return 1; } // single file

    let mut buf = [0u8; 256 * 64];
    let count = fs::readdir(path, &mut buf);
    if count == u32::MAX { return 0; }

    let mut total = 0u32;
    for i in 0..count as usize {
        let off = i * 64;
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        let name_bytes = &buf[off + 8..off + 8 + name_len.min(55)];
        let name = match core::str::from_utf8(name_bytes) {
            Ok(n) => n,
            Err(_) => continue,
        };
        if name == "." || name == ".." { continue; }
        if entry_type == TYPE_DIR {
            total += count_files_recursive(&build_full_path(path, name));
        } else {
            total += 1;
        }
    }
    total
}

/// Get the selected file indices (from grid multi-selection or icon view).
fn get_selected_indices() -> Vec<usize> {
    let s = app();
    let n = s.entries.len();
    let mut indices = Vec::new();

    if s.view_mode == VIEW_ICONS {
        for i in 0..s.icon_selected.len().min(n) {
            if s.icon_selected[i] {
                indices.push(i);
            }
        }
    }

    if indices.is_empty() {
        // Fallback to DataGrid selection
        for i in 0..n {
            if s.grid.is_row_selected(i as u32) {
                indices.push(i);
            }
        }
    }

    indices
}

// ============================================================================
// Icon path resolution (same logic as original — folder, .app, mimetype)
// ============================================================================

fn resolve_icon_path(s: &AppState, entry_idx: usize) -> String {
    let entry = &s.entries[entry_idx];
    let name = entry.name_str();

    if entry.entry_type == TYPE_DIR {
        if name.ends_with(".app") {
            let full = build_full_path(&s.cwd, name);
            let p = icons::app_icon_path(&full);
            return String::from(p.as_str());
        }
        return String::from(icons::FOLDER_ICON);
    }

    // File: check for app icon first, then mimetype
    let full = build_full_path(&s.cwd, name);
    let p = icons::app_icon_path(&full);
    if p.as_str() != icons::DEFAULT_APP_ICON {
        return String::from(p.as_str());
    }
    match get_extension(name) {
        Some(ext) => String::from(s.mimetypes.icon_for_ext(ext)),
        None => String::from(icons::DEFAULT_FILE_ICON),
    }
}

// ============================================================================
// Navigation
// ============================================================================

fn clear_selection() {
    let s = app();
    s.grid.set_selected_row(u32::MAX);
    s.icon_selected.clear();
    s.icon_anchor = 0;
}

fn navigate(path: &str) {
    // Validate that the path exists and is a directory.
    let mut stat_buf = [0u32; 7];
    let stat_ok = fs::stat(path, &mut stat_buf);
    if stat_ok != 0 || stat_buf[0] != TYPE_DIR as u32 {
        let msg = anyos_std::format!("Path not found:\n{}", path);
        ui::MessageBox::show(ui::MessageBoxType::Alert, &msg, None);
        let s = app();
        s.path_field.set_text(&s.cwd);
        s.path_field.select_all();
        return;
    }

    clear_selection();
    let s = app();
    if s.history_pos + 1 < s.history.len() {
        s.history.truncate(s.history_pos + 1);
    }
    s.cwd = String::from(path);
    s.entries = read_directory(path);
    s.history.push(s.cwd.clone());
    s.history_pos = s.history.len() - 1;
    sync_sidebar(path);
    refresh_ui();
}

fn navigate_back() {
    let s = app();
    if s.history_pos == 0 { return; }
    s.history_pos -= 1;
    let path = s.history[s.history_pos].clone();
    s.cwd = path.clone();
    s.entries = read_directory(&path);
    clear_selection();
    sync_sidebar(&path);
    refresh_ui();
}

fn navigate_forward() {
    let s = app();
    if s.history_pos + 1 >= s.history.len() { return; }
    s.history_pos += 1;
    let path = s.history[s.history_pos].clone();
    s.cwd = path.clone();
    s.entries = read_directory(&path);
    clear_selection();
    sync_sidebar(&path);
    refresh_ui();
}

fn navigate_up() {
    let s = app();
    if s.cwd == "/" { return; }
    let parent = String::from(parent_path(&s.cwd));
    navigate(&parent);
}

fn refresh_current() {
    let s = app();
    let path = s.cwd.clone();
    s.entries = read_directory(&path);
    refresh_ui();
}

fn sync_sidebar(path: &str) {
    let s = app();
    s.sidebar_sel = s.locations.len(); // none
    for (i, loc) in s.locations.iter().enumerate() {
        if loc.path.as_str() == path {
            s.sidebar_sel = i;
            break;
        }
    }
}

// ============================================================================
// Open / launch
// ============================================================================

fn open_entry(idx: usize) {
    let s = app();
    if idx >= s.entries.len() { return; }

    let name = String::from(s.entries[idx].name_str());
    let entry_type = s.entries[idx].entry_type;
    let is_app = s.entries[idx].is_app_bundle();

    if entry_type == TYPE_DIR {
        if is_app {
            let full_path = build_full_path(&s.cwd, &name);
            process::spawn(&full_path, "");
            return;
        }
        let new_path = build_full_path(&s.cwd, &name);
        navigate(&new_path);
    } else {
        let full_path = build_full_path(&s.cwd, &name);
        let ext = match name.rfind('.') {
            Some(pos) => &name[pos + 1..],
            None => "",
        };
        if !ext.is_empty() {
            if let Some(app_path) = s.mimetypes.app_for_ext(ext) {
                let args = anyos_std::format!("\"{}\" {}", app_path, full_path);
                process::spawn(app_path, &args);
                return;
            }
        }
        process::spawn(&full_path, &full_path);
    }
}

fn show_package_contents(idx: usize) {
    let s = app();
    if idx >= s.entries.len() { return; }
    let name = String::from(s.entries[idx].name_str());
    let new_path = build_full_path(&s.cwd, &name);
    navigate(&new_path);
}

// ============================================================================
// Context menu handler
// ============================================================================

fn copy_path(idx: usize) {
    let s = app();
    if idx >= s.entries.len() { return; }
    let name = s.entries[idx].name_str();
    let fp = build_full_path(&s.cwd, name);
    ui::clipboard_set(fp.as_str());
    s.sb_sel_label.set_text("Path copied");
}

fn copy_selected() {
    let indices = get_selected_indices();
    if indices.is_empty() { return; }
    let s = app();
    s.clip_files.clear();
    for &idx in &indices {
        if idx < s.entries.len() {
            let name = s.entries[idx].name_str();
            s.clip_files.push(build_full_path(&s.cwd, name));
        }
    }
    s.clip_is_cut = false;
    let msg = if s.clip_files.len() == 1 {
        String::from("Copied")
    } else {
        anyos_std::format!("{} items copied", s.clip_files.len())
    };
    s.sb_sel_label.set_text(&msg);
}

fn cut_selected() {
    let indices = get_selected_indices();
    if indices.is_empty() { return; }
    let s = app();
    s.clip_files.clear();
    for &idx in &indices {
        if idx < s.entries.len() {
            let name = s.entries[idx].name_str();
            s.clip_files.push(build_full_path(&s.cwd, name));
        }
    }
    s.clip_is_cut = true;
    let msg = if s.clip_files.len() == 1 {
        String::from("Cut")
    } else {
        anyos_std::format!("{} items cut", s.clip_files.len())
    };
    s.sb_sel_label.set_text(&msg);
}

fn paste_entry() {
    let s = app();
    if s.clip_files.is_empty() {
        s.sb_sel_label.set_text("Nothing to paste");
        return;
    }

    let sources = s.clip_files.clone();
    let is_cut = s.clip_is_cut;
    let dest_dir = s.cwd.clone();

    if sources.len() == 1 && !is_cut {
        // Single file copy — do it immediately (fast path)
        let src = &sources[0];
        let src_name = src.rsplit('/').next().unwrap_or("");
        if src_name.is_empty() { return; }

        let dest = unique_dest_path(&dest_dir, src_name);
        let mut stat_buf = [0u32; 7];
        let is_dir = fs::stat(src, &mut stat_buf) == 0 && stat_buf[0] == TYPE_DIR as u32;

        let ok = if is_dir {
            copy_dir_recursive(src, &dest)
        } else {
            copy_file(src, &dest)
        };

        if ok {
            app().sb_sel_label.set_text("Pasted");
        } else {
            app().sb_sel_label.set_text("Paste failed");
        }
        refresh_current();
        return;
    }

    if sources.len() == 1 && is_cut {
        // Single file cut — just rename/move
        let src = &sources[0];
        let src_name = src.rsplit('/').next().unwrap_or("");
        if src_name.is_empty() { return; }
        let dest = unique_dest_path(&dest_dir, src_name);
        if fs::rename(src, &dest) == u32::MAX {
            app().sb_sel_label.set_text("Move failed");
        } else {
            let s = app();
            s.clip_files.clear();
            s.sb_sel_label.set_text("Moved");
        }
        refresh_current();
        return;
    }

    // Multiple files — show progress dialog
    start_copy_operation(sources, dest_dir, is_cut);
}

fn start_copy_operation(sources: Vec<String>, dest_dir: String, is_cut: bool) {
    let total = sources.len() as u32;
    let title = if is_cut { "Moving Files" } else { "Copying Files" };

    let win = ui::Window::new_with_flags(
        title, -1, -1, 420, 160,
        ui::WIN_FLAG_NOT_RESIZABLE | ui::WIN_FLAG_NO_MINIMIZE | ui::WIN_FLAG_NO_MAXIMIZE | ui::WIN_FLAG_ALWAYS_ON_TOP,
    );
    win.set_color(0xFF1E1E1E);

    let file_label = ui::Label::new("");
    file_label.set_position(16, 16);
    file_label.set_size(388, 18);
    file_label.set_font_size(12);
    file_label.set_text_color(0xFFCCCCCC);
    win.add(&file_label);

    let dest_label = ui::Label::new(&anyos_std::format!("To: {}", dest_dir));
    dest_label.set_position(16, 38);
    dest_label.set_size(388, 18);
    dest_label.set_font_size(11);
    dest_label.set_text_color(0xFF888888);
    win.add(&dest_label);

    let count_label = ui::Label::new(&anyos_std::format!("0 of {} files", total));
    count_label.set_position(16, 60);
    count_label.set_size(388, 18);
    count_label.set_font_size(12);
    count_label.set_text_color(0xFFCCCCCC);
    win.add(&count_label);

    let progress = ui::ProgressBar::new(0);
    progress.set_position(16, 84);
    progress.set_size(388, 18);
    win.add(&progress);

    let cancel_btn = ui::Button::new("Cancel");
    cancel_btn.set_position(330, 114);
    cancel_btn.set_size(74, 28);
    cancel_btn.on_click(|_| {
        if let Some(ref mut op) = app().copy_op {
            op.cancelled = true;
        }
    });
    win.add(&cancel_btn);

    let win_id = win.id();
    let file_label_id = file_label.id();
    let progress_id = progress.id();
    let count_label_id = count_label.id();

    let s = app();
    s.copy_op = Some(CopyOperation {
        sources,
        dest_dir,
        is_cut,
        current_idx: 0,
        total_files: total,
        done_files: 0,
        cancelled: false,
        win_id,
        file_label_id,
        progress_id,
        count_label_id,
        timer_id: 0,
    });

    let timer_id = ui::set_timer(10, || {
        copy_operation_tick();
    });
    if let Some(ref mut op) = app().copy_op {
        op.timer_id = timer_id;
    }
}

fn copy_operation_tick() {
    let s = app();
    let op = match s.copy_op.as_mut() {
        Some(o) => o,
        None => return,
    };

    if op.cancelled || op.current_idx >= op.sources.len() {
        // Done or cancelled — clean up
        let timer_id = op.timer_id;
        let win_id = op.win_id;
        let was_cut = op.is_cut;
        let cancelled = op.cancelled;
        let done = op.done_files;
        ui::kill_timer(timer_id);
        // Destroy dialog window by reconstructing Window from saved ID.
        // Window is a newtype wrapper (Container(Control(u32))), same size as u32.
        let dialog_win: ui::Window = unsafe { core::mem::transmute(win_id) };
        dialog_win.destroy();
        let s = app();
        s.copy_op = None;
        if was_cut && !cancelled {
            s.clip_files.clear();
        }
        if cancelled {
            s.sb_sel_label.set_text(&anyos_std::format!("{} files processed (cancelled)", done));
        } else {
            let verb = if was_cut { "Moved" } else { "Copied" };
            s.sb_sel_label.set_text(&anyos_std::format!("{} {} files", verb, done));
        }
        refresh_current();
        return;
    }

    // Process one file per tick
    let src = op.sources[op.current_idx].clone();
    let dest_dir = op.dest_dir.clone();
    let is_cut = op.is_cut;

    let src_name = src.rsplit('/').next().unwrap_or("");
    let verb = if is_cut { "Moving" } else { "Copying" };
    ui::Control::from_id(op.file_label_id).set_text(
        &anyos_std::format!("{}: {}", verb, src_name)
    );

    let dest = unique_dest_path(&dest_dir, src_name);
    let mut stat_buf = [0u32; 7];
    let is_dir = fs::stat(&src, &mut stat_buf) == 0 && stat_buf[0] == TYPE_DIR as u32;

    let ok = if is_cut {
        fs::rename(&src, &dest) != u32::MAX
    } else if is_dir {
        copy_dir_recursive(&src, &dest)
    } else {
        copy_file(&src, &dest)
    };

    let op = app().copy_op.as_mut().unwrap();
    op.current_idx += 1;
    if ok {
        op.done_files += 1;
    }

    let pct = if op.total_files > 0 {
        (op.done_files * 100 / op.total_files).min(100)
    } else { 0 };
    ui::Control::from_id(op.progress_id).set_state(pct);
    ui::Control::from_id(op.count_label_id).set_text(
        &anyos_std::format!("{} of {} files", op.done_files, op.total_files)
    );
}

fn confirm_delete() {
    let indices = get_selected_indices();
    if indices.is_empty() { return; }

    // Collect names for the confirmation message
    let s = app();
    let mut paths = Vec::new();
    let mut names = Vec::new();
    for &idx in &indices {
        if idx < s.entries.len() {
            let name = String::from(s.entries[idx].name_str());
            paths.push(build_full_path(&s.cwd, &name));
            names.push(name);
        }
    }
    if paths.is_empty() { return; }

    let msg = if names.len() == 1 {
        anyos_std::format!("Move \"{}\" to Trash?", names[0])
    } else {
        anyos_std::format!("Move {} items to Trash?", names.len())
    };

    let win = ui::Window::new_with_flags(
        "Delete", -1, -1, 380, 140,
        ui::WIN_FLAG_NOT_RESIZABLE | ui::WIN_FLAG_NO_MINIMIZE | ui::WIN_FLAG_NO_MAXIMIZE | ui::WIN_FLAG_ALWAYS_ON_TOP,
    );
    win.set_color(0xFF1E1E1E);

    let icon_lbl = ui::Label::new("\u{26A0}");
    icon_lbl.set_position(20, 20);
    icon_lbl.set_size(24, 24);
    icon_lbl.set_font_size(20);
    icon_lbl.set_text_color(0xFFE8A838);
    win.add(&icon_lbl);

    let msg_label = ui::Label::new(&msg);
    msg_label.set_position(52, 22);
    msg_label.set_size(310, 40);
    msg_label.set_font_size(13);
    msg_label.set_text_color(0xFFCCCCCC);
    win.add(&msg_label);

    let trash_btn = ui::Button::new("Move to Trash");
    trash_btn.set_position(140, 90);
    trash_btn.set_size(120, 30);
    trash_btn.set_color(0xFFC04040);
    trash_btn.on_click(move |_| {
        for p in &paths {
            move_to_trash_single(p);
        }
        win.destroy();
        refresh_current();
        let s = app();
        if names.len() == 1 {
            s.sb_sel_label.set_text("Moved to Trash");
        } else {
            let msg = anyos_std::format!("{} items moved to Trash", names.len());
            s.sb_sel_label.set_text(&msg);
        }
    });
    win.add(&trash_btn);

    let cancel_btn = ui::Button::new("Cancel");
    cancel_btn.set_position(270, 90);
    cancel_btn.set_size(90, 30);
    cancel_btn.on_click(move |_| {
        win.destroy();
    });
    win.add(&cancel_btn);

    win.on_key_down(move |ke| {
        if ke.keycode == ui::KEY_ESCAPE {
            win.destroy();
        }
    });
}

fn start_rename(idx: usize) {
    let s = app();
    if idx >= s.entries.len() { return; }
    let name = s.entries[idx].name_str();
    s.rename_idx = idx;
    s.rename_active = true;
    s.rename_panel.set_visible(true);
    s.rename_field.set_text(name);
    s.rename_field.focus();
}

fn close_rename() {
    let s = app();
    s.rename_active = false;
    s.rename_panel.set_visible(false);
}

fn apply_rename() {
    let s = app();
    let idx = s.rename_idx;
    if idx >= s.entries.len() {
        close_rename();
        return;
    }

    let mut buf = [0u8; 256];
    let len = s.rename_field.get_text(&mut buf);
    if len == 0 {
        close_rename();
        return;
    }
    let new_name = match core::str::from_utf8(&buf[..len as usize]) {
        Ok(n) => n.trim(),
        Err(_) => { close_rename(); return; }
    };
    if new_name.is_empty() {
        close_rename();
        return;
    }

    let old_name = s.entries[idx].name_str();
    if new_name == old_name {
        close_rename();
        return;
    }

    let old_path = build_full_path(&s.cwd, old_name);
    let new_path = build_full_path(&s.cwd, new_name);

    if fs::rename(&old_path, &new_path) == u32::MAX {
        s.sb_sel_label.set_text("Rename failed");
        close_rename();
        return;
    }

    // Update clipboard paths if the renamed file was in the clipboard
    for p in s.clip_files.iter_mut() {
        if p.as_str() == old_path.as_str() {
            *p = new_path.clone();
        }
    }

    s.sb_sel_label.set_text("Renamed");
    close_rename();
    refresh_current();
}

/// Add an .app bundle to the user's dock config ($HOME/.dock.conf).
fn add_to_dock(idx: usize) {
    let s = app();
    if idx >= s.entries.len() { return; }

    let entry = &s.entries[idx];
    let name = entry.name_str();
    let full_path = build_full_path(&s.cwd, name);

    // Derive display name (strip .app suffix)
    let app_name = name.strip_suffix(".app").unwrap_or(name);

    // Get user home directory from UID
    let conf_path = {
        let uid = anyos_std::process::getuid();
        let mut name_buf = [0u8; 64];
        let nlen = anyos_std::process::getusername(uid, &mut name_buf);
        if nlen != u32::MAX && nlen > 0 {
            if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
                anyos_std::format!("/Users/{}/.dock.conf", username)
            } else {
                s.sb_sel_label.set_text("Error: invalid username");
                return;
            }
        } else {
            // Fallback to $HOME
            let mut home_buf = [0u8; 256];
            let len = anyos_std::env::get("HOME", &mut home_buf);
            if len == u32::MAX || len == 0 {
                s.sb_sel_label.set_text("Error: HOME not set");
                return;
            }
            match core::str::from_utf8(&home_buf[..len as usize]) {
                Ok(h) => anyos_std::format!("{}/.dock.conf", h),
                Err(_) => return,
            }
        }
    };

    // Read existing user config; if absent, read system config as base
    let mut existing = String::new();
    let fd = fs::open(&conf_path, 0);
    if fd != u32::MAX {
        let mut buf = [0u8; 4096];
        loop {
            let n = fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                existing.push_str(text);
            }
        }
        fs::close(fd);
    } else {
        // No user config yet — read system config as base
        let sys_fd = fs::open("/System/dock/programs.conf", 0);
        if sys_fd != u32::MAX {
            let mut buf = [0u8; 4096];
            loop {
                let n = fs::read(sys_fd, &mut buf);
                if n == 0 || n == u32::MAX { break; }
                if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                    existing.push_str(text);
                }
            }
            fs::close(sys_fd);
        }
    }

    // Check if already in dock
    for line in existing.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some(path) = line.splitn(2, '|').nth(1) {
            if path.trim() == full_path {
                s.sb_sel_label.set_text("Already in Dock");
                return;
            }
        }
    }

    // Append new entry
    if !existing.ends_with('\n') && !existing.is_empty() {
        existing.push('\n');
    }
    existing.push_str(app_name);
    existing.push('|');
    existing.push_str(&full_path);
    existing.push('\n');

    let _ = fs::write_bytes(&conf_path, existing.as_bytes());

    // Notify the dock to reload its config via IPC channel
    let dock_chan = anyos_std::ipc::evt_chan_create("dock");
    anyos_std::ipc::evt_chan_emit(dock_chan, &[1, 0, 0, 0, 0]);

    s.sb_sel_label.set_text("Added to Dock");
}

fn handle_context_action(index: u32) {
    // Use the menu variant that was stored at right-click time (by update_context_menu),
    // NOT the current entry type — the directory or selection may have changed since then.
    let variant = app().ctx_menu_variant;

    // Variant 3 = background menu (no selection), only Paste at index 0
    if variant == 3 {
        if index == 0 { paste_entry(); }
        return;
    }

    // Resolve paste index for the active variant (paste needs no selection)
    let paste_idx = match variant { 0 => 5u32, 1 => 3, _ => 4 };
    if index == paste_idx {
        paste_entry();
        return;
    }

    // All other actions require a valid selection
    let s = app();
    let sel = s.grid.selected_row();
    if sel == u32::MAX { return; }
    let idx = sel as usize;
    if idx >= s.entries.len() { return; }

    if variant == 0 {
        // Menu: Open | Add to Dock | Show Package Contents | Cut | Copy | Paste | Rename | Delete | Copy Path | Properties
        match index {
            0 => open_entry(idx),
            1 => add_to_dock(idx),
            2 => show_package_contents(idx),
            3 => cut_selected(),
            4 => copy_selected(),
            6 => start_rename(idx),
            7 => confirm_delete(),
            8 => copy_path(idx),
            9 => show_properties(idx),
            _ => {}
        }
    } else if variant == 1 {
        // Menu: Open | Cut | Copy | Paste | Rename | Delete | Copy Path | Properties
        match index {
            0 => open_entry(idx),
            1 => cut_selected(),
            2 => copy_selected(),
            4 => start_rename(idx),
            5 => confirm_delete(),
            6 => copy_path(idx),
            7 => show_properties(idx),
            _ => {}
        }
    } else {
        // Menu: Open | Open With... | Cut | Copy | Paste | Rename | Delete | Copy Path | Properties
        match index {
            0 => open_entry(idx),
            1 => show_open_with(idx),
            2 => cut_selected(),
            3 => copy_selected(),
            5 => start_rename(idx),
            6 => confirm_delete(),
            7 => copy_path(idx),
            8 => show_properties(idx),
            _ => {}
        }
    }
}

fn update_context_menu() {
    let s = app();
    let sel = s.grid.selected_row();

    if sel == u32::MAX || sel as usize >= s.entries.len() {
        // No selection — show a minimal menu with just Paste
        s.ctx_menu_variant = 3; // 3 = background (no selection)
        s.ctx_menu.set_text("Paste");
        return;
    }

    let idx = sel as usize;
    let is_app = s.entries[idx].is_app_bundle();
    let is_dir = s.entries[idx].entry_type == TYPE_DIR;

    let (menu_text, variant) = if is_app {
        ("Open|Add to Dock|Show Package Contents|Cut|Copy|Paste|Rename|Delete|Copy Path|Properties", 0u32)
    } else if is_dir {
        ("Open|Cut|Copy|Paste|Rename|Delete|Copy Path|Properties", 1u32)
    } else {
        ("Open|Open With...|Cut|Copy|Paste|Rename|Delete|Copy Path|Properties", 2u32)
    };

    s.ctx_menu_variant = variant;

    // Update existing context menu text (don't create a new one — it wouldn't have a parent)
    s.ctx_menu.set_text(menu_text);
}

// ============================================================================
// Properties dialog
// ============================================================================

/// Recursively count files and sum sizes in a directory.
/// Returns (file_count, total_bytes). Caps recursion at `max_depth` levels.
fn count_dir_contents_inner(path: &str, max_depth: u32) -> (u32, u64) {
    if max_depth == 0 { return (0, 0); }

    let mut buf = [0u8; 256 * 64];
    let count = fs::readdir(path, &mut buf);
    if count == u32::MAX { return (0, 0); }

    let mut files = 0u32;
    let mut bytes = 0u64;

    for i in 0..count as usize {
        let off = i * 64;
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        let size = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);
        let name = &buf[off + 8..off + 8 + name_len.min(55)];

        // Skip . and ..
        if name == b"." || name == b".." { continue; }

        if entry_type == TYPE_DIR {
            if let Ok(name_str) = core::str::from_utf8(name) {
                let child = build_full_path(path, name_str);
                let (cf, cb) = count_dir_contents_inner(&child, max_depth - 1);
                files += cf;
                bytes += cb;
            }
        } else {
            files += 1;
            bytes += size as u64;
        }
    }

    (files, bytes)
}

fn count_dir_contents(path: &str) -> (u32, u64) {
    count_dir_contents_inner(path, 10)
}

/// Format a u64 byte count as a human-readable size string.
fn fmt_size64(size: u64) -> String {
    if size >= 1024 * 1024 * 1024 {
        let gb = size / (1024 * 1024 * 1024);
        let frac = ((size % (1024 * 1024 * 1024)) * 10 / (1024 * 1024 * 1024)).min(9);
        anyos_std::format!("{}.{} GB", gb, frac)
    } else if size >= 1024 * 1024 {
        let mb = size / (1024 * 1024);
        let frac = ((size % (1024 * 1024)) * 10 / (1024 * 1024)).min(9);
        anyos_std::format!("{}.{} MB", mb, frac)
    } else if size >= 1024 {
        let kb = size / 1024;
        anyos_std::format!("{} KB", kb)
    } else {
        anyos_std::format!("{} B", size)
    }
}

/// Convert a Unix timestamp to "YYYY-MM-DD HH:MM" string.
fn fmt_date(ts: u32) -> String {
    if ts == 0 { return String::from("--"); }

    let secs = ts as u64;
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = (time_of_day / 3600) as u32;
    let minutes = ((time_of_day % 3600) / 60) as u32;

    // Calculate year/month/day from days since 1970-01-01
    let mut y = 1970u32;
    let mut remaining = days;
    loop {
        let days_in_year = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366u64 } else { 365u64 };
        if remaining < days_in_year { break; }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let mdays: [u32; 12] = [31, if leap {29} else {28}, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0u32;
    for i in 0..12 {
        if remaining < mdays[i] as u64 { m = i as u32 + 1; break; }
        remaining -= mdays[i] as u64;
    }
    if m == 0 { m = 12; }
    let d = remaining as u32 + 1;

    anyos_std::format!("{}-{:02}-{:02} {:02}:{:02}", y, m, d, hours, minutes)
}

/// Format Unix permission bits as "rwxrwxrwx" string.
fn fmt_mode(mode: u32, is_dir: bool) -> String {
    let mut s = String::new();
    s.push(if is_dir { 'd' } else { '-' });
    let bits = [
        (mode >> 8) & 7, // owner
        (mode >> 4) & 7, // group (adjusted for 12-bit mode)
        mode & 7,        // other
    ];
    // Actually mode is stored as 12-bit: bits 11-9 = owner, 8-6 = group, 5-3 = other
    // But let's use the simpler 9-bit convention: rwx for owner (bits 8-6), group (5-3), other (2-0)
    for shift in [6u32, 3, 0] {
        let b = (mode >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

// ============================================================================
// "Open With" dialog
// ============================================================================

const APPLICATIONS_DIR: &str = "/Applications";

#[derive(Clone)]
struct AppInfo {
    display_name: String,
    bundle_path: String,
}

fn enumerate_applications() -> Vec<AppInfo> {
    let mut apps = Vec::new();
    let mut buf = [0u8; 256 * 64];
    let count = fs::readdir(APPLICATIONS_DIR, &mut buf);
    if count == u32::MAX { return apps; }

    for i in 0..count as usize {
        let off = i * 64;
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        let name = &buf[off + 8..off + 8 + name_len.min(55)];

        if entry_type != TYPE_DIR { continue; }
        if let Ok(name_str) = core::str::from_utf8(name) {
            if !name_str.ends_with(".app") { continue; }

            let bundle_path = build_full_path(APPLICATIONS_DIR, name_str);
            let display_name = icons::app_bundle_name(&bundle_path);

            apps.push(AppInfo { display_name, bundle_path });
        }
    }

    apps.sort_by(|a, b| a.display_name.as_str().cmp(b.display_name.as_str()));
    apps
}

fn show_open_with(idx: usize) {
    let s = app();
    if idx >= s.entries.len() { return; }

    let name = String::from(s.entries[idx].name_str());
    let full_path = build_full_path(&s.cwd, &name);
    let ext = match get_extension(&name) {
        Some(e) => String::from(e),
        None => String::new(),
    };

    let apps = enumerate_applications();
    if apps.is_empty() { return; }

    // Find current default for this extension
    let current_default = s.mimetypes.app_for_ext(&ext)
        .map(String::from)
        .unwrap_or_default();

    // ── Build the "Open With" dialog ──
    let title = anyos_std::format!("Open \"{}\" With...", name);
    let win = ui::Window::new_with_flags(
        &title, -1, -1, 380, 420,
        ui::WIN_FLAG_NOT_RESIZABLE | ui::WIN_FLAG_NO_MINIMIZE | ui::WIN_FLAG_NO_MAXIMIZE,
    );
    win.set_color(0xFF1E1E1E);

    // Header label
    let header = ui::Label::new("Choose an application:");
    header.set_dock(ui::DOCK_TOP);
    header.set_size(380, 30);
    header.set_font_size(13);
    header.set_text_color(0xFFCCCCCC);
    header.set_margin(12, 8, 12, 4);
    win.add(&header);

    // ── Bottom panel (DOCK_BOTTOM before DOCK_FILL) ──
    let bottom = ui::View::new();
    bottom.set_dock(ui::DOCK_BOTTOM);
    bottom.set_size(0, 70);
    bottom.set_color(0xFF1E1E1E);

    let div = ui::Divider::new();
    div.set_dock(ui::DOCK_TOP);
    div.set_size(0, 1);
    bottom.add(&div);

    let chk_default = ui::Checkbox::new("Always use for this file type");
    chk_default.set_position(12, 8);
    chk_default.set_size(250, 20);
    bottom.add(&chk_default);

    let ok_btn = ui::Button::new("Open");
    ok_btn.set_size(80, 28);
    ok_btn.set_position(200, 34);
    ok_btn.set_color(0xFF0E639C);
    bottom.add(&ok_btn);

    let cancel_btn = ui::Button::new("Cancel");
    cancel_btn.set_size(80, 28);
    cancel_btn.set_position(288, 34);
    bottom.add(&cancel_btn);

    win.add(&bottom);

    // ── DataGrid listing apps (DOCK_FILL, added last) ──
    let grid = ui::DataGrid::new(0, 0);
    grid.set_dock(ui::DOCK_FILL);
    grid.set_columns(&[
        ui::ColumnDef::new("Application").width(340),
    ]);
    grid.set_row_height(28);
    grid.set_header_height(28);

    // Populate rows
    grid.set_row_count(apps.len() as u32);
    for (i, app_info) in apps.iter().enumerate() {
        grid.set_cell(i as u32, 0, &app_info.display_name);
    }

    // Set app icons
    for (i, app_info) in apps.iter().enumerate() {
        let icon_path = icons::app_icon_path(&app_info.bundle_path);
        let s2 = app();
        if let Some((pixels, w, h)) = s2.icon_cache.get_or_load(&icon_path, ICON_SIZE_SMALL) {
            grid.set_cell_icon(i as u32, 0, pixels, w, h);
        }
    }

    // Pre-select current default
    for (i, app_info) in apps.iter().enumerate() {
        if app_info.bundle_path == current_default {
            grid.set_selected_row(i as u32);
            break;
        }
    }

    win.add(&grid);

    // ── Event handlers ──

    // Cancel
    cancel_btn.on_click(move |_| {
        win.destroy();
    });

    // OK: open with selected app, optionally set as default
    let apps_ok = apps.clone();
    let full_path_ok = full_path.clone();
    let ext_ok = ext.clone();
    ok_btn.on_click(move |_| {
        let sel = grid.selected_row();
        if sel != u32::MAX && (sel as usize) < apps_ok.len() {
            let app_path = apps_ok[sel as usize].bundle_path.as_str();

            // Set as default if checkbox is checked
            if chk_default.get_state() != 0 && !ext_ok.is_empty() {
                app().mimetypes.set_user_default(&ext_ok, app_path);
            }

            // Launch the app
            let args = anyos_std::format!("{} {}", app_path, full_path_ok);
            process::spawn(app_path, &args);
        }
        win.destroy();
    });

    // Double-click on grid row: open immediately
    let apps_dbl = apps.clone();
    let full_path_dbl = full_path.clone();
    grid.on_submit(move |e| {
        if e.index != u32::MAX && (e.index as usize) < apps_dbl.len() {
            let app_path = apps_dbl[e.index as usize].bundle_path.as_str();
            let args = anyos_std::format!("{} {}", app_path, full_path_dbl);
            process::spawn(app_path, &args);
        }
        win.destroy();
    });

    // Escape closes
    win.on_key_down(move |ke| {
        if ke.keycode == ui::KEY_ESCAPE {
            win.destroy();
        }
    });
}

fn show_properties(idx: usize) {
    let s = app();
    if idx >= s.entries.len() { return; }

    let entry = &s.entries[idx];
    let name = String::from(entry.name_str());
    let is_dir = entry.entry_type == TYPE_DIR;
    let is_app = entry.is_app_bundle();
    let full_path = build_full_path(&s.cwd, &name);
    let location = String::from(s.cwd.as_str());

    // Get icon
    let icon_path = resolve_icon_path(s, idx);

    // Stat the file for metadata
    let mut stat_buf = [0u32; 7];
    let has_stat = fs::stat(&full_path, &mut stat_buf) == 0;
    let file_size = if has_stat { stat_buf[1] } else { entry.size };
    let mtime = if has_stat { stat_buf[6] } else { 0 };
    let mode = if has_stat { stat_buf[5] } else { 0 };

    // Type description
    let type_str = if is_app {
        String::from("Application")
    } else if is_dir {
        String::from("Folder")
    } else {
        match get_extension(&name) {
            Some(ext) => {
                let mut t = String::from(ext);
                t.push_str(" File");
                t
            }
            None => String::from("File"),
        }
    };

    // For directories, count contents
    let (dir_files, dir_bytes) = if is_dir {
        count_dir_contents(&full_path)
    } else {
        (0, 0)
    };

    // Display name (strip .app suffix for bundles)
    let display_name = if is_app {
        String::from(name.as_str().strip_suffix(".app").unwrap_or(&name))
    } else {
        name.clone()
    };

    // ── Build the properties window ──
    let win_title = anyos_std::format!("{} \u{2014} Properties", display_name);
    let win = ui::Window::new_with_flags(
        &win_title, -1, -1, 340, 360,
        ui::WIN_FLAG_NOT_RESIZABLE | ui::WIN_FLAG_NO_MINIMIZE | ui::WIN_FLAG_NO_MAXIMIZE,
    );
    win.set_color(0xFF1E1E1E);

    // ── Header: icon + name ──
    let header = ui::View::new();
    header.set_dock(ui::DOCK_TOP);
    header.set_size(0, 70);
    header.set_color(0xFF1E1E1E);

    let icon_iv = ui::ImageView::new(48, 48);
    icon_iv.set_position(16, 11);
    if let Some((pixels, w, h)) = s.icon_cache.get_or_load(&icon_path, ICON_SIZE_LARGE) {
        icon_iv.set_pixels(pixels, w, h);
    }
    header.add(&icon_iv);

    let name_lbl = ui::Label::new(&display_name);
    name_lbl.set_position(76, 14);
    name_lbl.set_size(240, 20);
    name_lbl.set_font_size(15);
    name_lbl.set_text_color(0xFFFFFFFF);
    header.add(&name_lbl);

    // Subtitle (full filename)
    if is_app || display_name.as_str() != name.as_str() {
        let sub_lbl = ui::Label::new(&name);
        sub_lbl.set_position(76, 38);
        sub_lbl.set_size(240, 16);
        sub_lbl.set_font_size(11);
        sub_lbl.set_text_color(0xFF888888);
        header.add(&sub_lbl);
    }

    win.add(&header);

    // ── Divider ──
    let div1 = ui::Divider::new();
    div1.set_dock(ui::DOCK_TOP);
    div1.set_size(0, 1);
    win.add(&div1);

    // ── Properties grid ──
    let props = ui::View::new();
    props.set_dock(ui::DOCK_FILL);
    props.set_color(0xFF1E1E1E);

    let label_x: i32 = 16;
    let value_x: i32 = 110;
    let mut y: i32 = 12;
    let row_h: i32 = 24;
    let label_color: u32 = 0xFF888888;
    let value_color: u32 = 0xFFDDDDDD;

    // Helper: add a row with label + value
    macro_rules! prop_row {
        ($label:expr, $value:expr) => {{
            let lbl = ui::Label::new($label);
            lbl.set_position(label_x, y);
            lbl.set_size(90, 18);
            lbl.set_font_size(12);
            lbl.set_text_color(label_color);
            props.add(&lbl);

            let val = ui::Label::new($value);
            val.set_position(value_x, y);
            val.set_size(210, 18);
            val.set_font_size(12);
            val.set_text_color(value_color);
            props.add(&val);

            y += row_h;
        }};
    }

    prop_row!("Type:", &type_str);
    prop_row!("Location:", &location);

    if is_dir {
        let size_str = fmt_size64(dir_bytes);
        prop_row!("Size:", &size_str);

        let contains_str = if dir_files == 1 {
            String::from("1 file")
        } else {
            anyos_std::format!("{} files", dir_files)
        };
        prop_row!("Contains:", &contains_str);
    } else {
        let size_str = if file_size >= 1024 {
            anyos_std::format!("{} ({} bytes)", fmt_size(file_size), file_size)
        } else {
            anyos_std::format!("{} bytes", file_size)
        };
        prop_row!("Size:", &size_str);
    }

    let date_str = fmt_date(mtime);
    prop_row!("Modified:", &date_str);

    if mode != 0 {
        let mode_str = fmt_mode(mode, is_dir);
        prop_row!("Mode:", &mode_str);
    }

    prop_row!("Path:", &full_path);

    // ── Bottom bar with OK button ──
    let bottom = ui::View::new();
    bottom.set_dock(ui::DOCK_BOTTOM);
    bottom.set_size(0, 50);
    bottom.set_color(0xFF1E1E1E);

    let div2 = ui::Divider::new();
    div2.set_dock(ui::DOCK_TOP);
    div2.set_size(0, 1);
    bottom.add(&div2);

    let ok_btn = ui::Button::new("OK");
    ok_btn.set_size(80, 28);
    ok_btn.set_position(130, 12);
    ok_btn.on_click(move |_| {
        win.destroy();
    });
    bottom.add(&ok_btn);

    // DOCK_BOTTOM must be added before DOCK_FILL so layout reserves bottom space first
    win.add(&bottom);
    win.add(&props);

    // Escape key destroys the dialog; close button is handled by the event loop automatically
    win.on_key_down(move |ke| {
        if ke.keycode == ui::KEY_ESCAPE {
            win.destroy();
        }
    });
}

// ============================================================================
// View mode switching
// ============================================================================

fn set_view_mode(mode: u32) {
    let s = app();
    if s.view_mode == mode { return; }
    s.view_mode = mode;

    if mode == VIEW_LIST {
        s.grid.set_visible(true);
        s.icon_scroll.set_visible(false);
    } else {
        s.grid.set_visible(false);
        s.icon_scroll.set_visible(true);
    }

    refresh_view();
}

fn refresh_view() {
    let s = app();
    if s.view_mode == VIEW_LIST {
        populate_grid();
    } else {
        populate_icon_view();
    }
}

// ============================================================================
// Refresh UI
// ============================================================================

fn refresh_ui() {
    let s = app();

    s.path_field.set_text(&s.cwd);

    let count_str = anyos_std::format!("{} items", s.entries.len());
    s.sb_items_label.set_text(&count_str);
    s.sb_sel_label.set_text("");

    s.btn_back.set_enabled(s.history_pos > 0);
    s.btn_fwd.set_enabled(s.history_pos + 1 < s.history.len());

    // Sidebar highlight
    let n_loc = s.locations.len();
    for i in 0..s.sidebar_item_ids.len() {
        let ctrl = ui::Control::from_id(s.sidebar_item_ids[i]);
        if i < n_loc && i == s.sidebar_sel {
            ctrl.set_color(0xFF0A54C4);
            ctrl.set_text_color(0xFFFFFFFF);
        } else {
            ctrl.set_color(0x00000000);
            ctrl.set_text_color(0xFFCCCCCC);
        }
    }

    refresh_view();
}

// ============================================================================
// List view (DataGrid)
// ============================================================================

fn populate_grid() {
    let s = app();
    let n = s.entries.len();

    if n == 0 {
        s.grid.set_data(&[]);
        return;
    }

    let mut display_names: Vec<String> = Vec::new();
    let mut size_strings: Vec<String> = Vec::new();

    for entry in &s.entries {
        let name = entry.name_str();
        let display = if entry.entry_type == TYPE_DIR {
            String::from(name.strip_suffix(".app").unwrap_or(name))
        } else {
            String::from(name)
        };
        display_names.push(display);

        if entry.entry_type == TYPE_FILE {
            size_strings.push(fmt_size(entry.size));
        } else {
            size_strings.push(String::from("--"));
        }
    }

    let mut rows: Vec<Vec<&str>> = Vec::new();
    for i in 0..n {
        let mut row = Vec::new();
        row.push(display_names[i].as_str());
        row.push(size_strings[i].as_str());
        rows.push(row);
    }
    s.grid.set_data(&rows);

    // Set icons (always scaled to ICON_SIZE_SMALL)
    for i in 0..n {
        let icon_path = resolve_icon_path(s, i);
        if let Some((pixels, w, h)) = s.icon_cache.get_or_load(&icon_path, ICON_SIZE_SMALL) {
            s.grid.set_cell_icon(i as u32, 0, pixels, w, h);
        }
    }

    s.grid.set_scroll_offset(0);
}

// ============================================================================
// Icon/Grid view (FlowPanel)
// ============================================================================

fn populate_icon_view() {
    let s = app();

    // Remove previous icon items
    for &id in &s.icon_item_ids {
        ui::Control::from_id(id).remove();
    }
    s.icon_item_ids.clear();
    s.icon_selected.clear();

    let n = s.entries.len();

    for i in 0..n {
        // Resolve icon path first (returns owned String)
        let icon_path = resolve_icon_path(s, i);

        let entry = &s.entries[i];
        let name = entry.name_str();
        let display_name = if entry.entry_type == TYPE_DIR {
            name.strip_suffix(".app").unwrap_or(name)
        } else {
            name
        };

        // Truncate long names
        let max_chars = 12usize;
        let label_text = if display_name.len() > max_chars {
            let mut t = String::from(&display_name[..max_chars]);
            t.push_str("..");
            t
        } else {
            String::from(display_name)
        };

        // Create a cell: View containing ImageView + Label
        let cell = ui::View::new();
        cell.set_size(GRID_CELL_W, GRID_CELL_H);
        cell.set_color(0x00000000); // transparent

        // Icon (centered at top of cell)
        let iv = ui::ImageView::new(ICON_SIZE_LARGE, ICON_SIZE_LARGE);
        iv.set_dock(ui::DOCK_TOP);
        iv.set_margin(
            ((GRID_CELL_W - ICON_SIZE_LARGE) / 2) as i32,
            4,
            ((GRID_CELL_W - ICON_SIZE_LARGE) / 2) as i32,
            0,
        );
        if let Some((pixels, w, h)) = s.icon_cache.get_or_load(&icon_path, ICON_SIZE_LARGE) {
            iv.set_pixels(pixels, w, h);
        }
        cell.add(&iv);

        // Label below icon
        let lbl = ui::Label::new(&label_text);
        lbl.set_dock(ui::DOCK_TOP);
        lbl.set_size(GRID_CELL_W, 18);
        lbl.set_font_size(11);
        lbl.set_text_color(0xFFCCCCCC);
        lbl.set_text_align(ui::TEXT_ALIGN_CENTER);
        cell.add(&lbl);

        // Click → select, double-click → open, right-click → select + context menu
        cell.on_click_raw(icon_item_click_handler, i as u64);
        cell.on_double_click_raw(icon_item_dblclick_handler, i as u64);
        cell.on_event_raw(ui::EVENT_CONTEXT_MENU, icon_item_context_handler, i as u64);
        cell.set_context_menu(&s.ctx_menu);

        s.icon_flow.add(&cell);
        s.icon_item_ids.push(cell.id());
        s.icon_selected.push(false);
    }
}

fn update_selection_status_multi() {
    let s = app();
    let sel_count = s.icon_selected.iter().filter(|&&b| b).count();
    if sel_count == 0 {
        s.sb_sel_label.set_text("");
    } else if sel_count == 1 {
        // Find the single selected entry
        if let Some(idx) = s.icon_selected.iter().position(|&b| b) {
            if idx < s.entries.len() {
                let name = s.entries[idx].name_str();
                let text = if s.entries[idx].entry_type == TYPE_DIR {
                    anyos_std::format!("\"{}\" selected", name)
                } else {
                    anyos_std::format!("\"{}\" — {}", name, fmt_size(s.entries[idx].size))
                };
                s.sb_sel_label.set_text(&text);
            }
        }
    } else {
        let text = anyos_std::format!("{} items selected", sel_count);
        s.sb_sel_label.set_text(&text);
    }
}

fn update_selection_status(idx: usize) {
    let s = app();
    if idx < s.entries.len() {
        let name = s.entries[idx].name_str();
        let text = if s.entries[idx].entry_type == TYPE_DIR {
            anyos_std::format!("\"{}\" selected", name)
        } else {
            anyos_std::format!("\"{}\" — {}", name, fmt_size(s.entries[idx].size))
        };
        s.sb_sel_label.set_text(&text);
    } else {
        s.sb_sel_label.set_text("");
    }
}

fn icon_clear_selection() {
    let s = app();
    for i in 0..s.icon_selected.len() {
        if s.icon_selected[i] {
            s.icon_selected[i] = false;
            if i < s.icon_item_ids.len() {
                ui::Control::from_id(s.icon_item_ids[i]).set_color(0x00000000);
            }
        }
    }
}

fn icon_set_selected(idx: usize, selected: bool) {
    let s = app();
    if idx >= s.icon_selected.len() { return; }
    s.icon_selected[idx] = selected;
    if idx < s.icon_item_ids.len() {
        let color = if selected { 0xFF0A54C4 } else { 0x00000000 };
        ui::Control::from_id(s.icon_item_ids[idx]).set_color(color);
    }
}

fn select_icon_item(idx: usize) {
    select_icon_item_with_mods(idx, ui::get_modifiers());
}

fn select_icon_item_with_mods(idx: usize, mods: u32) {
    let s = app();
    if idx >= s.entries.len() { return; }

    let ctrl = mods & ui::MOD_CTRL != 0;
    let shift = mods & ui::MOD_SHIFT != 0;

    if ctrl {
        // Ctrl+Click: toggle this item
        let was = s.icon_selected.get(idx).copied().unwrap_or(false);
        icon_set_selected(idx, !was);
        if !was {
            app().icon_anchor = idx;
        }
    } else if shift {
        // Shift+Click: range select from anchor
        let anchor = s.icon_anchor;
        let lo = anchor.min(idx);
        let hi = anchor.max(idx);
        icon_clear_selection();
        for r in lo..=hi {
            icon_set_selected(r, true);
        }
    } else {
        // Plain click: select only this item
        icon_clear_selection();
        icon_set_selected(idx, true);
        app().icon_anchor = idx;
    }

    // Sync DataGrid selection for context menu (use first selected)
    let s = app();
    s.grid.set_selected_row(idx as u32);
    update_selection_status_multi();
}

/// Get the currently focused icon index (anchor or first selected).
fn icon_focused_index() -> Option<usize> {
    let s = app();
    if s.icon_anchor < s.icon_selected.len() && s.icon_selected[s.icon_anchor] {
        return Some(s.icon_anchor);
    }
    s.icon_selected.iter().position(|&b| b)
}

/// Compute how many columns fit per row in the icon flow panel.
fn icon_cols_per_row() -> usize {
    let s = app();
    let (w, _) = s.icon_flow.get_size();
    if w == 0 { return 1; }
    (w as usize / GRID_CELL_W as usize).max(1)
}

extern "C" fn icon_item_click_handler(_control_id: u32, _event_type: u32, userdata: u64) {
    select_icon_item(userdata as usize);
}

extern "C" fn icon_item_dblclick_handler(_control_id: u32, _event_type: u32, userdata: u64) {
    let idx = userdata as usize;
    open_entry(idx);
}

extern "C" fn icon_item_context_handler(_control_id: u32, _event_type: u32, userdata: u64) {
    let idx = userdata as usize;
    let s = app();
    // Select this item on right-click (if not already selected, preserve multi-selection)
    if idx < s.icon_selected.len() && !s.icon_selected[idx] {
        icon_clear_selection();
        icon_set_selected(idx, true);
        app().icon_anchor = idx;
    }
    app().grid.set_selected_row(idx as u32);
    update_context_menu();
    update_selection_status_multi();
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    if !ui::init() { return; }

    // Load sidebar locations from config
    let locations = load_locations();

    // ── Window ───────────────────────────────────────────────────────────
    let win = ui::Window::new("Finder", -1, -1, 750, 480);

    // ── Toolbar ──────────────────────────────────────────────────────────
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(750, 38);
    toolbar.set_color(0xFF2C2C2E);
    toolbar.set_padding(4, 4, 4, 4);
    win.add(&toolbar);

    // Nav buttons — IconButton with custom pixel icons (server-side hover/press/disabled)
    let btn_back = ui::IconButton::new("");
    btn_back.set_size(30, 28);
    if let Some(mut icon) = ui::Icon::control("left", 18) {
        icon.recolor(0xFFCCCCCC);
        btn_back.set_pixels(&icon.pixels, icon.width, icon.height);
    }
    btn_back.set_enabled(false);
    btn_back.set_tooltip("Back (Alt+Left)");
    toolbar.add(&btn_back);

    let btn_fwd = ui::IconButton::new("");
    btn_fwd.set_size(30, 28);
    if let Some(mut icon) = ui::Icon::control("right", 18) {
        icon.recolor(0xFFCCCCCC);
        btn_fwd.set_pixels(&icon.pixels, icon.width, icon.height);
    }
    btn_fwd.set_enabled(false);
    btn_fwd.set_tooltip("Forward (Alt+Right)");
    toolbar.add(&btn_fwd);

    let btn_up = ui::IconButton::new("");
    btn_up.set_size(30, 28);
    if let Some(mut icon) = ui::Icon::control("folder", 18) {
        icon.recolor(0xFFCCCCCC);
        btn_up.set_pixels(&icon.pixels, icon.width, icon.height);
    }
    btn_up.set_tooltip("Parent folder (Backspace)");
    toolbar.add(&btn_up);

    toolbar.add_separator();

    let btn_refresh = ui::IconButton::new("");
    btn_refresh.set_size(30, 28);
    if let Some(mut icon) = ui::Icon::control("refresh", 18) {
        icon.recolor(0xFFCCCCCC);
        btn_refresh.set_pixels(&icon.pixels, icon.width, icon.height);
    }
    btn_refresh.set_tooltip("Refresh (F5)");
    toolbar.add(&btn_refresh);

    toolbar.add_separator();

    // Path field
    let path_field = ui::TextField::new();
    path_field.set_size(300, 26);
    path_field.set_placeholder("Enter path...");
    path_field.set_text("/");
    toolbar.add(&path_field);

    toolbar.add_separator();

    // View mode buttons
    let btn_list = toolbar.add_icon_button("List");
    btn_list.set_size(30, 28);
    btn_list.set_system_icon("list", ui::IconType::Outline, 0xFFCCCCCC, 16);
    btn_list.set_tooltip("List view");

    let btn_grid = toolbar.add_icon_button("Grid");
    btn_grid.set_size(30, 28);
    btn_grid.set_system_icon("grid-dots", ui::IconType::Outline, 0xFFCCCCCC, 16);
    btn_grid.set_tooltip("Icon view");

    // ── Status bar (bottom) ────────────────────────────────────────────
    let status_bar = ui::View::new();
    status_bar.set_size(0, 22);
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_color(0xFF007ACC);
    win.add(&status_bar);

    let sb_items_label = ui::Label::new("0 items");
    sb_items_label.set_position(8, 3);
    sb_items_label.set_font_size(11);
    sb_items_label.set_text_color(0xFFFFFFFF);
    status_bar.add(&sb_items_label);

    let sb_sel_label = ui::Label::new("");
    sb_sel_label.set_position(140, 3);
    sb_sel_label.set_font_size(11);
    sb_sel_label.set_text_color(0xFFFFFFFF);
    status_bar.add(&sb_sel_label);

    // ── Rename panel (DOCK_BOTTOM, hidden by default) ────────────────────
    let rename_panel = ui::View::new();
    rename_panel.set_dock(ui::DOCK_BOTTOM);
    rename_panel.set_size(800, 34);
    rename_panel.set_color(0xFF2D2D30);
    rename_panel.set_visible(false);

    let rename_lbl = ui::Label::new("Rename:");
    rename_lbl.set_position(8, 8);
    rename_lbl.set_text_color(0xFFCCCCCC);
    rename_lbl.set_font_size(13);
    rename_panel.add(&rename_lbl);

    let rename_field = ui::TextField::new();
    rename_field.set_position(70, 4);
    rename_field.set_size(500, 26);
    rename_panel.add(&rename_field);

    let btn_rename_ok = ui::Button::new("OK");
    btn_rename_ok.set_position(580, 4);
    btn_rename_ok.set_size(40, 26);
    rename_panel.add(&btn_rename_ok);

    let btn_rename_cancel = ui::Button::new("X");
    btn_rename_cancel.set_position(626, 4);
    btn_rename_cancel.set_size(28, 26);
    rename_panel.add(&btn_rename_cancel);

    win.add(&rename_panel);

    // ── Main area: SplitView ─────────────────────────────────────────────
    let split = ui::SplitView::new();
    split.set_dock(ui::DOCK_FILL);
    split.set_orientation(ui::ORIENTATION_HORIZONTAL);
    split.set_split_ratio(22);
    split.set_min_split(10);
    split.set_max_split(40);
    win.add(&split);

    // ── Left: Sidebar ────────────────────────────────────────────────────
    let sidebar_panel = ui::View::new();
    sidebar_panel.set_dock(ui::DOCK_FILL);
    sidebar_panel.set_color(0xFF1E1E1E);

    let sidebar_header = ui::Label::new("LOCATIONS");
    sidebar_header.set_dock(ui::DOCK_TOP);
    sidebar_header.set_size(160, 24);
    sidebar_header.set_font_size(11);
    sidebar_header.set_text_color(0xFF777777);
    sidebar_header.set_margin(12, 8, 0, 4);
    sidebar_panel.add(&sidebar_header);

    let mut sidebar_item_ids: Vec<u32> = Vec::new();
    for (i, loc) in locations.iter().enumerate() {
        let item = ui::Label::new(&loc.name);
        item.set_dock(ui::DOCK_TOP);
        item.set_size(160, 30);
        item.set_font_size(13);
        item.set_text_color(0xFFCCCCCC);
        item.set_padding(28, 6, 8, 6);
        item.set_margin(4, 1, 4, 1);
        item.on_click_raw(sidebar_click_handler, i as u64);
        sidebar_item_ids.push(item.id());
        sidebar_panel.add(&item);
    }

    split.add(&sidebar_panel);

    // ── Right: content area (holds both DataGrid and Canvas) ─────────────
    let content_panel = ui::View::new();
    content_panel.set_dock(ui::DOCK_FILL);
    content_panel.set_color(0xFF1E1E1E);

    // DataGrid (list view — visible by default)
    let grid = ui::DataGrid::new(0, 0);
    grid.set_dock(ui::DOCK_FILL);
    grid.set_columns(&[
        ui::ColumnDef::new("Name").width(400),
        ui::ColumnDef::new("Size").width(100).align(ui::ALIGN_RIGHT).numeric(),
    ]);
    grid.set_row_height(26);
    grid.set_header_height(28);
    grid.set_selection_mode(ui::SELECTION_MULTI);

    // Context menu — attached to grid AND added to parent panel (required!)
    let ctx_menu = ui::ContextMenu::new("Open|Copy Path");
    ctx_menu.on_item_click(|e| { handle_context_action(e.index); });
    grid.set_context_menu(&ctx_menu);

    content_panel.add(&grid);
    content_panel.add(&ctx_menu);

    // FlowPanel in ScrollView (icon view — hidden by default)
    let icon_scroll = ui::ScrollView::new();
    icon_scroll.set_dock(ui::DOCK_FILL);
    icon_scroll.set_visible(false);
    icon_scroll.set_color(0xFF1E1E1E);

    let icon_flow = ui::FlowPanel::new();
    icon_flow.set_dock(ui::DOCK_FILL);
    icon_flow.set_color(0xFF1E1E1E);
    icon_scroll.add(&icon_flow);

    content_panel.add(&icon_scroll);

    split.add(&content_panel);

    // ── Create app state ─────────────────────────────────────────────────
    let mimetypes = icons::MimeDb::load();

    unsafe {
        APP = Some(AppState {
            cwd: String::from("/"),
            entries: Vec::new(),
            history: Vec::new(),
            history_pos: 0,
            sidebar_sel: 0,
            view_mode: VIEW_LIST,
            mimetypes,
            icon_cache: IconCache::new(),
            locations,
            grid,
            icon_scroll,
            icon_flow,
            icon_item_ids: Vec::new(),
            icon_selected: Vec::new(),
            icon_anchor: 0,
            path_field,
            btn_back,
            btn_fwd,
            sidebar_item_ids,
            sb_items_label,
            sb_sel_label,
            ctx_menu,
            ctx_menu_variant: 2,
            rename_panel,
            rename_field,
            rename_active: false,
            rename_idx: 0,
            clip_files: Vec::new(),
            clip_is_cut: false,
            copy_op: None,
        });
    }

    // ── Initial load ─────────────────────────────────────────────────────
    let mut args_buf = [0u8; 256];
    let raw_args = process::args(&mut args_buf);
    let initial_path = if !raw_args.is_empty() && raw_args.starts_with('/') {
        raw_args
    } else {
        "/"
    };
    navigate(initial_path);

    // ── Event handlers ───────────────────────────────────────────────────

    btn_back.on_click(|_| { navigate_back(); });
    btn_fwd.on_click(|_| { navigate_forward(); });
    btn_up.on_click(|_| { navigate_up(); });
    btn_refresh.on_click(|_| { refresh_current(); });
    btn_list.on_click(|_| { set_view_mode(VIEW_LIST); });
    btn_grid.on_click(|_| { set_view_mode(VIEW_ICONS); });
    btn_rename_ok.on_click(|_| { apply_rename(); });
    btn_rename_cancel.on_click(|_| { close_rename(); });
    rename_field.on_submit(|_| { apply_rename(); });

    path_field.on_submit(|_| {
        let s = app();
        let mut buf = [0u8; 512];
        let len = s.path_field.get_text(&mut buf);
        if len > 0 {
            if let Ok(typed) = core::str::from_utf8(&buf[..len as usize]) {
                let p = String::from(typed.trim());
                if !p.is_empty() {
                    navigate(&p);
                }
            }
        }
    });

    // DataGrid: double-click / Enter opens entry
    grid.on_submit(|e| {
        if e.index != u32::MAX {
            open_entry(e.index as usize);
        }
    });

    // DataGrid: selection changed → update context menu + status bar
    grid.on_selection_changed(|_| {
        update_context_menu();
        // Count how many rows are selected
        let s = app();
        let n = s.entries.len();
        let mut sel_count = 0usize;
        let mut last_sel = usize::MAX;
        for i in 0..n {
            if s.grid.is_row_selected(i as u32) {
                sel_count += 1;
                last_sel = i;
            }
        }
        if sel_count == 0 {
            s.sb_sel_label.set_text("");
        } else if sel_count == 1 {
            update_selection_status(last_sel);
        } else {
            let text = anyos_std::format!("{} items selected", sel_count);
            s.sb_sel_label.set_text(&text);
        }
    });

    // Keyboard shortcuts
    win.on_key_down(|ke| {
        // Escape: close rename panel or quit
        if ke.keycode == ui::KEY_ESCAPE {
            if app().rename_active {
                close_rename();
            } else {
                ui::quit();
            }
            return;
        }

        // Ctrl+C: copy selected entries
        if ke.ctrl() && ke.char_code == 'c' as u32 {
            copy_selected();
            return;
        }

        // Ctrl+X: cut selected entries
        if ke.ctrl() && ke.char_code == 'x' as u32 {
            cut_selected();
            return;
        }

        // Ctrl+V: paste
        if ke.ctrl() && ke.char_code == 'v' as u32 {
            paste_entry();
            return;
        }

        // Delete: move selected entries to trash
        if ke.keycode == ui::KEY_DELETE {
            confirm_delete();
            return;
        }

        // F2: rename selected entry
        if ke.keycode == ui::KEY_F2 {
            let sel = app().grid.selected_row();
            if sel != u32::MAX && (sel as usize) < app().entries.len() {
                start_rename(sel as usize);
            }
            return;
        }

        if ke.keycode == ui::KEY_BACKSPACE {
            navigate_up();
            return;
        }

        if ke.alt() && ke.keycode == ui::KEY_LEFT {
            navigate_back();
            return;
        }

        if ke.alt() && ke.keycode == ui::KEY_RIGHT {
            navigate_forward();
            return;
        }

        if ke.keycode == ui::KEY_F5 {
            refresh_current();
            return;
        }

        // Arrow key navigation in icon view
        let s = app();
        if s.view_mode == VIEW_ICONS && !s.entries.is_empty() {
            let n = s.entries.len();
            let cols = icon_cols_per_row();
            let cur = icon_focused_index().unwrap_or(0);
            let new_idx = match ke.keycode {
                ui::KEY_LEFT  => if cur > 0 { Some(cur - 1) } else { None },
                ui::KEY_RIGHT => if cur + 1 < n { Some(cur + 1) } else { None },
                ui::KEY_UP    => if cur >= cols { Some(cur - cols) } else { None },
                ui::KEY_DOWN  => if cur + cols < n { Some(cur + cols) } else { None },
                ui::KEY_HOME  => Some(0),
                ui::KEY_END   => Some(n - 1),
                ui::KEY_ENTER => { open_entry(cur); None },
                _ => None,
            };
            if let Some(idx) = new_idx {
                select_icon_item_with_mods(idx, ke.modifiers);
            }
        }
    });

    win.on_close(|_| { ui::quit(); });

    // ── Run ──────────────────────────────────────────────────────────────
    ui::run();
}

// ============================================================================
// Sidebar click handler
// ============================================================================

extern "C" fn sidebar_click_handler(_control_id: u32, _event_type: u32, userdata: u64) {
    let idx = userdata as usize;
    let s = app();
    if idx < s.locations.len() {
        let path = s.locations[idx].path.clone();
        navigate(&path);
    }
}
