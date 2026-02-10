#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;

use anyos_std::fs;
use anyos_std::process;
use anyos_std::ui::window;

anyos_std::entry!(main);

use uisys_client::*;

// ============================================================================
// Layout
// ============================================================================

const SIDEBAR_W: u32 = 160;
const TOOLBAR_H: u32 = 36;
const SIDEBAR_HEADER_H: u32 = 28;
const SIDEBAR_ITEM_H: u32 = 32;
const ROW_H: i32 = 28;
const ICON_SIZE: u32 = 16;
const EVENT_MOUSE_SCROLL: u32 = 7;

// Sidebar locations
const LOCATIONS: [(&str, &str); 5] = [
    ("Root", "/"),
    ("Programs", "/bin"),
    ("System", "/system"),
    ("Libraries", "/system/lib"),
    ("Icons", "/system/icons"),
];

// File entry type constants (from readdir)
const TYPE_FILE: u8 = 0;
const TYPE_DIR: u8 = 1;

// Double-click timing: 400ms in PIT ticks (computed at runtime via tick_hz())
static mut DBLCLICK_TICKS: u32 = 40;

// ============================================================================
// Data structures
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
}

struct MimeEntry {
    ext: String,
    app: String,
}

struct AppState {
    cwd: String,
    entries: Vec<FileEntry>,
    selected: Option<usize>,
    scroll_offset: u32,
    sidebar_sel: usize,
    // For double-click detection
    last_click_idx: Option<usize>,
    last_click_tick: u32,
    // Navigation history
    history: Vec<String>,
    history_pos: usize, // index into history; history[history_pos] == cwd
    // Mimetype associations
    mimetypes: Vec<MimeEntry>,
}

// ============================================================================
// Directory reading
// ============================================================================

fn read_directory(path: &str) -> Vec<FileEntry> {
    let mut buf = [0u8; 64 * 64]; // max 64 entries
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

    // Sort: directories first, then files. Both alphabetically.
    entries.sort_by(|a, b| {
        if a.entry_type != b.entry_type {
            // Directories first
            return a.entry_type.cmp(&b.entry_type).reverse();
        }
        a.name[..a.name_len].cmp(&b.name[..b.name_len])
    });

    entries
}

fn navigate(state: &mut AppState, path: &str) {
    // If navigating forward from middle of history, truncate forward history
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

    // Update sidebar selection if path matches a location
    state.sidebar_sel = LOCATIONS.len(); // none
    for (i, &(_, loc_path)) in LOCATIONS.iter().enumerate() {
        if loc_path == path {
            state.sidebar_sel = i;
            break;
        }
    }
}

fn navigate_back(state: &mut AppState) {
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

    state.sidebar_sel = LOCATIONS.len();
    for (i, &(_, loc_path)) in LOCATIONS.iter().enumerate() {
        if loc_path == path.as_str() {
            state.sidebar_sel = i;
            break;
        }
    }
}

fn navigate_forward(state: &mut AppState) {
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

    state.sidebar_sel = LOCATIONS.len();
    for (i, &(_, loc_path)) in LOCATIONS.iter().enumerate() {
        if loc_path == path.as_str() {
            state.sidebar_sel = i;
            break;
        }
    }
}

/// Update menu item enable/disable states based on current app state.
fn update_menu_states(win: u32, state: &AppState) {
    // "Open" (id=1): enabled only when something is selected
    if state.selected.is_some() {
        window::enable_menu_item(win, 1);
    } else {
        window::disable_menu_item(win, 1);
    }
    // "Back" (id=20): enabled only when history allows going back
    if state.history_pos > 0 {
        window::enable_menu_item(win, 20);
    } else {
        window::disable_menu_item(win, 20);
    }
    // "Forward" (id=21): enabled only when history allows going forward
    if state.history_pos + 1 < state.history.len() {
        window::enable_menu_item(win, 21);
    } else {
        window::disable_menu_item(win, 21);
    }
}

// ============================================================================
// Mimetype associations
// ============================================================================

fn load_mimetypes() -> Vec<MimeEntry> {
    let fd = fs::open("/system/mimetypes.conf", 0);
    if fd == u32::MAX {
        return Vec::new();
    }

    let mut data = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);

    let text = match core::str::from_utf8(&data) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(sep) = line.find('|') {
            let ext = line[..sep].trim();
            let app = line[sep + 1..].trim();
            if !ext.is_empty() && !app.is_empty() {
                entries.push(MimeEntry {
                    ext: String::from(ext),
                    app: String::from(app),
                });
            }
        }
    }
    entries
}

fn lookup_mimetype<'a>(mimes: &'a [MimeEntry], ext: &str) -> Option<&'a str> {
    for entry in mimes {
        if entry.ext == ext {
            return Some(&entry.app);
        }
    }
    None
}

// ============================================================================
// Open / launch
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

fn open_entry(state: &mut AppState, idx: usize) {
    let entry = &state.entries[idx];
    let name = entry.name_str();

    if entry.entry_type == TYPE_DIR {
        let new_path = build_full_path(&state.cwd, name);
        navigate(state, &new_path);
    } else {
        let full_path = build_full_path(&state.cwd, name);

        // Check mimetype association by file extension
        let ext = match name.rfind('.') {
            Some(pos) => &name[pos + 1..],
            None => "",
        };

        if !ext.is_empty() {
            if let Some(app) = lookup_mimetype(&state.mimetypes, ext) {
                // Args must include program name as argv[0]
                let args = anyos_std::format!("{} {}", app, full_path);
                process::spawn(app, &args);
                return;
            }
        }

        // Default: try to execute
        process::spawn(&full_path, &full_path);
    }
}

// ============================================================================
// Click handling
// ============================================================================

fn handle_click(state: &mut AppState, mx: i32, my: i32, win_w: u32, win_h: u32) -> bool {
    // Toolbar area
    if my < TOOLBAR_H as i32 {
        return handle_toolbar_click(state, mx, my);
    }

    let content_y = TOOLBAR_H as i32;

    // Sidebar
    if mx < SIDEBAR_W as i32 {
        let y0 = content_y + SIDEBAR_HEADER_H as i32;
        for i in 0..LOCATIONS.len() {
            let iy = y0 + i as i32 * SIDEBAR_ITEM_H as i32;
            if my >= iy && my < iy + SIDEBAR_ITEM_H as i32 {
                let (_, path) = LOCATIONS[i];
                navigate(state, path);
                return true;
            }
        }
        return false;
    }

    // File list area
    let list_x = SIDEBAR_W as i32;
    let list_y = content_y;
    let header_h = ROW_H; // column header row

    if my < list_y + header_h {
        return false; // clicked header
    }

    let file_area_y = list_y + header_h;
    let row_idx = ((my - file_area_y) as u32 + state.scroll_offset * ROW_H as u32) / ROW_H as u32;
    let row = row_idx as usize;

    if row < state.entries.len() {
        let now = anyos_std::sys::uptime();

        // Double-click detection
        if let Some(last_idx) = state.last_click_idx {
            if last_idx == row && now.wrapping_sub(state.last_click_tick) < unsafe { DBLCLICK_TICKS } {
                // Double-click!
                state.last_click_idx = None;
                open_entry(state, row);
                return true;
            }
        }

        state.selected = Some(row);
        state.last_click_idx = Some(row);
        state.last_click_tick = now;
        return true;
    } else {
        state.selected = None;
        state.last_click_idx = None;
        return true;
    }
}

fn handle_toolbar_click(state: &mut AppState, mx: i32, _my: i32) -> bool {
    // Back button: x=8, w=60
    if mx >= 8 && mx < 68 {
        navigate_back(state);
        return true;
    }
    // Forward button: x=72, w=60
    if mx >= 72 && mx < 132 {
        navigate_forward(state);
        return true;
    }
    false
}

// ============================================================================
// Rendering
// ============================================================================

fn render(win: u32, state: &AppState, win_w: u32, win_h: u32) {
    // Background
    window::fill_rect(win, 0, 0, win_w as u16, win_h as u16, colors::WINDOW_BG);

    // Toolbar
    render_toolbar(win, state, win_w);

    let content_y = TOOLBAR_H as i32;
    let content_h = win_h - TOOLBAR_H;

    // Sidebar
    sidebar_bg(win, 0, content_y, SIDEBAR_W, content_h);
    sidebar_header(win, 0, content_y, SIDEBAR_W, "LOCATIONS");

    let y0 = content_y + SIDEBAR_HEADER_H as i32;
    for (i, &(name, _)) in LOCATIONS.iter().enumerate() {
        let iy = y0 + i as i32 * SIDEBAR_ITEM_H as i32;
        sidebar_item(win, 0, iy, SIDEBAR_W, name, i == state.sidebar_sel);
    }

    // File list
    render_file_list(win, state, win_w, win_h);
}

fn render_toolbar(win: u32, state: &AppState, win_w: u32) {
    toolbar(win, 0, 0, win_w, TOOLBAR_H);

    // Back button
    let back_state = if state.history_pos > 0 { ButtonState::Normal } else { ButtonState::Disabled };
    toolbar_button(win, 8, 2, 60, TOOLBAR_H - 4, "<", back_state);

    // Forward button
    let fwd_state = if state.history_pos + 1 < state.history.len() { ButtonState::Normal } else { ButtonState::Disabled };
    toolbar_button(win, 72, 2, 60, TOOLBAR_H - 4, ">", fwd_state);

    // Path display
    label(win, 140, 10, &state.cwd, colors::TEXT, FontSize::Normal, TextAlign::Left);

    // Item count on right
    let mut buf = [0u8; 16];
    let count_str = fmt_item_count(&mut buf, state.entries.len());
    let text_w = count_str.len() as i32 * 7;
    label(win, win_w as i32 - text_w - 12, 10, count_str, colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);
}

fn render_file_list(win: u32, state: &AppState, win_w: u32, win_h: u32) {
    let list_x = SIDEBAR_W as i32;
    let list_y = TOOLBAR_H as i32;
    let list_w = win_w - SIDEBAR_W;

    // Column header
    window::fill_rect(win, list_x as i16, list_y as i16, list_w as u16, ROW_H as u16, 0xFF333333);
    label(win, list_x + 12 + ICON_SIZE as i32 + 8, list_y + 6, "Name", colors::TEXT, FontSize::Normal, TextAlign::Left);

    let size_col_x = list_x + list_w as i32 - 100;
    label(win, size_col_x, list_y + 6, "Size", colors::TEXT, FontSize::Normal, TextAlign::Left);

    divider_h(win, list_x, list_y + ROW_H - 1, list_w);

    // File rows
    let file_area_y = list_y + ROW_H;
    let max_visible = ((win_h as i32 - file_area_y) / ROW_H) as usize;

    for i in 0..state.entries.len().min(max_visible) {
        let entry_idx = i + state.scroll_offset as usize;
        if entry_idx >= state.entries.len() {
            break;
        }
        let entry = &state.entries[entry_idx];
        let ry = file_area_y + i as i32 * ROW_H;

        // Selection highlight
        if state.selected == Some(entry_idx) {
            window::fill_rect(win, list_x as i16, ry as i16, list_w as u16, ROW_H as u16, colors::ACCENT);
        } else if i % 2 == 1 {
            // Alternating row background
            window::fill_rect(win, list_x as i16, ry as i16, list_w as u16, ROW_H as u16, 0xFF252525);
        }

        let text_color = if state.selected == Some(entry_idx) { 0xFFFFFFFF } else { colors::TEXT };
        let dim_color = if state.selected == Some(entry_idx) { 0xFFDDDDDD } else { colors::TEXT_SECONDARY };

        // Icon (simple colored square representing file type)
        let icon_x = list_x + 12;
        let icon_y = ry + (ROW_H - ICON_SIZE as i32) / 2;
        let icon_color = match entry.entry_type {
            TYPE_DIR => colors::ACCENT,        // Blue folder
            _ => {
                // Check extension/type
                let name = entry.name_str();
                if name.ends_with(".dll") {
                    0xFF9B59B6 // Purple for DLLs
                } else if name.ends_with(".icon") {
                    colors::WARNING // Yellow for icons
                } else if name.ends_with(".txt") || name.ends_with(".conf")
                    || name.ends_with(".log") || name.ends_with(".md")
                    || name.ends_with(".ini") {
                    colors::TEXT_SECONDARY // Gray for text/config files
                } else {
                    colors::SUCCESS // Green for executables
                }
            }
        };
        window::fill_rect(win, icon_x as i16, icon_y as i16, ICON_SIZE as u16, ICON_SIZE as u16, icon_color);

        // Folder icon: draw a tab shape on top
        if entry.entry_type == TYPE_DIR {
            window::fill_rect(win, icon_x as i16, (icon_y - 2) as i16, 8, 3, icon_color);
        }

        // Name
        let name_x = icon_x + ICON_SIZE as i32 + 8;
        label_ellipsis(win, name_x, ry + 6, entry.name_str(), text_color, FontSize::Normal,
            (size_col_x - name_x - 8) as u32);

        // Size (only for files)
        if entry.entry_type == TYPE_FILE {
            let mut buf = [0u8; 16];
            let size_str = fmt_size(&mut buf, entry.size);
            label(win, size_col_x, ry + 6, size_str, dim_color, FontSize::Normal, TextAlign::Left);
        } else {
            label(win, size_col_x, ry + 6, "--", dim_color, FontSize::Normal, TextAlign::Left);
        }
    }

    // Scrollbar
    let file_area_h = (win_h as i32 - file_area_y).max(0) as u32;
    let content_h = (state.entries.len() as u32) * (ROW_H as u32);
    let scroll_pixels = state.scroll_offset * (ROW_H as u32);
    scrollbar(win, list_x, file_area_y, list_w, file_area_h, content_h, scroll_pixels);
}

// ============================================================================
// Formatting
// ============================================================================

fn fmt_size<'a>(buf: &'a mut [u8; 16], size: u32) -> &'a str {
    if size >= 1024 * 1024 {
        let mb = size / (1024 * 1024);
        let frac = (size % (1024 * 1024)) / (1024 * 100); // 1 decimal
        let mut p = 0;
        p = write_u32(buf, p, mb);
        buf[p] = b'.'; p += 1;
        p = write_u32(buf, p, frac.min(9));
        buf[p..p + 3].copy_from_slice(b" MB");
        p += 3;
        unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
    } else if size >= 1024 {
        let kb = size / 1024;
        let mut p = 0;
        p = write_u32(buf, p, kb);
        buf[p..p + 3].copy_from_slice(b" KB");
        p += 3;
        unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
    } else {
        let mut p = 0;
        p = write_u32(buf, p, size);
        buf[p..p + 2].copy_from_slice(b" B");
        p += 2;
        unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
    }
}

fn fmt_item_count<'a>(buf: &'a mut [u8; 16], count: usize) -> &'a str {
    let mut p = 0;
    p = write_u32(buf, p, count as u32);
    buf[p..p + 6].copy_from_slice(b" items");
    p += 6;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn write_u32(buf: &mut [u8], pos: usize, val: u32) -> usize {
    if val == 0 {
        buf[pos] = b'0';
        return pos + 1;
    }
    let mut v = val;
    let mut tmp = [0u8; 10];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[pos + i] = tmp[n - 1 - i];
    }
    pos + n
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    // Initialize double-click timing from tick rate
    let hz = anyos_std::sys::tick_hz();
    if hz > 0 {
        unsafe { DBLCLICK_TICKS = hz * 400 / 1000; }
    }

    let win = window::create("Finder", 100, 60, 620, 440);
    if win == u32::MAX {
        return;
    }

    // Set up menu bar
    let mut mb = window::MenuBarBuilder::new()
        .menu("File")
            .item(1, "Open", 0)
            .separator()
            .item(2, "Close", 0)
        .end_menu()
        .menu("Go")
            .item(10, "Root", 0)
            .item(11, "Programs", 0)
            .item(12, "System", 0)
            .item(13, "Libraries", 0)
            .item(14, "Icons", 0)
            .separator()
            .item(20, "Back", 0)
            .item(21, "Forward", 0)
        .end_menu();
    let data = mb.build();
    window::set_menu(win, data);

    let (mut win_w, mut win_h) = window::get_size(win).unwrap_or((620, 440));

    let mimetypes = load_mimetypes();

    let mut state = AppState {
        cwd: String::from("/"),
        entries: Vec::new(),
        selected: None,
        scroll_offset: 0,
        sidebar_sel: 0,
        last_click_idx: None,
        last_click_tick: 0,
        history: Vec::new(),
        history_pos: 0,
        mimetypes,
    };

    // Initial directory load
    state.entries = read_directory("/");
    state.history.push(String::from("/"));

    // Set initial menu states (Open disabled, Back disabled, Forward disabled)
    update_menu_states(win, &state);

    let mut event = [0u32; 5];
    let mut needs_redraw = true;

    loop {
        while window::get_event(win, &mut event) == 1 {
            match event[0] {
                window::EVENT_KEY_DOWN => {
                    let key = event[1];
                    match key {
                        0x103 => { // ESC
                            window::destroy(win);
                            return;
                        }
                        0x101 => { // Backspace - go up
                            if state.cwd != "/" {
                                let trimmed = state.cwd.trim_end_matches('/');
                                let parent = match trimmed.rfind('/') {
                                    Some(0) => "/",
                                    Some(pos) => &state.cwd[..pos],
                                    None => "/",
                                };
                                let parent_str = String::from(parent);
                                navigate(&mut state, &parent_str);
                                needs_redraw = true;
                            }
                        }
                        0x100 => { // Enter - open selected
                            if let Some(idx) = state.selected {
                                if idx < state.entries.len() {
                                    open_entry(&mut state, idx);
                                    needs_redraw = true;
                                }
                            }
                        }
                        0x105 => { // Up arrow
                            match state.selected {
                                Some(0) | None => state.selected = Some(0),
                                Some(i) => state.selected = Some(i - 1),
                            }
                            // Scroll to keep selection visible
                            if let Some(sel) = state.selected {
                                if (sel as u32) < state.scroll_offset {
                                    state.scroll_offset = sel as u32;
                                }
                            }
                            needs_redraw = true;
                        }
                        0x106 => { // Down arrow
                            let max = state.entries.len().saturating_sub(1);
                            match state.selected {
                                None => state.selected = Some(0),
                                Some(i) if i < max => state.selected = Some(i + 1),
                                _ => {}
                            }
                            // Scroll to keep selection visible
                            if let Some(sel) = state.selected {
                                let file_area_h = win_h as i32 - TOOLBAR_H as i32 - ROW_H;
                                let max_visible = (file_area_h / ROW_H) as usize;
                                if sel >= state.scroll_offset as usize + max_visible {
                                    state.scroll_offset = (sel - max_visible + 1) as u32;
                                }
                            }
                            needs_redraw = true;
                        }
                        _ => {}
                    }
                }
                window::EVENT_MOUSE_DOWN => {
                    let mx = event[1] as i32;
                    let my = event[2] as i32;
                    if handle_click(&mut state, mx, my, win_w, win_h) {
                        needs_redraw = true;
                    }
                }
                EVENT_MOUSE_SCROLL => {
                    let dz = event[1] as i32;
                    let file_area_h = win_h as i32 - TOOLBAR_H as i32 - ROW_H;
                    let max_visible = (file_area_h / ROW_H) as usize;
                    let max_scroll = state.entries.len().saturating_sub(max_visible) as u32;
                    if dz < 0 {
                        // Scroll up
                        state.scroll_offset = state.scroll_offset.saturating_sub((-dz) as u32);
                    } else if dz > 0 {
                        // Scroll down
                        state.scroll_offset = (state.scroll_offset + dz as u32).min(max_scroll);
                    }
                    needs_redraw = true;
                }
                window::EVENT_RESIZE => {
                    win_w = event[1];
                    win_h = event[2];
                    needs_redraw = true;
                }
                window::EVENT_MENU_ITEM => {
                    let item_id = event[2];
                    match item_id {
                        1 => { // Open
                            if let Some(idx) = state.selected {
                                if idx < state.entries.len() {
                                    open_entry(&mut state, idx);
                                    needs_redraw = true;
                                }
                            }
                        }
                        2 => { window::destroy(win); return; } // Close
                        10 => { navigate(&mut state, "/"); needs_redraw = true; }
                        11 => { navigate(&mut state, "/bin"); needs_redraw = true; }
                        12 => { navigate(&mut state, "/system"); needs_redraw = true; }
                        13 => { navigate(&mut state, "/system/lib"); needs_redraw = true; }
                        14 => { navigate(&mut state, "/system/icons"); needs_redraw = true; }
                        20 => { navigate_back(&mut state); needs_redraw = true; }
                        21 => { navigate_forward(&mut state); needs_redraw = true; }
                        _ => {}
                    }
                }
                window::EVENT_WINDOW_CLOSE => {
                    window::destroy(win);
                    return;
                }
                _ => {}
            }
        }

        if needs_redraw {
            update_menu_states(win, &state);
            render(win, &state, win_w, win_h);
            window::present(win);
            needs_redraw = false;
        }

        process::yield_cpu();
    }
}
