#![no_std]
#![no_main]

use anyos_std::sys;
use anyos_std::net;
use anyos_std::process;
use anyos_std::fs;
use anyos_std::ui::window;

anyos_std::entry!(main);

use uisys_client::*;

// Layout
const SIDEBAR_W: u32 = 160;
const PAD: i32 = 16;
const ROW_H: i32 = 40;

// Pages
const PAGE_GENERAL: usize = 0;
const PAGE_DISPLAY: usize = 1;
const PAGE_WALLPAPER: usize = 2;
const PAGE_NETWORK: usize = 3;
const PAGE_ABOUT: usize = 4;
const PAGE_NAMES: [&str; 5] = ["General", "Display", "Wallpaper", "Network", "About"];

// Wallpaper thumbnail dimensions
const THUMB_W: u32 = 120;
const THUMB_H: u32 = 80;
const THUMB_PAD: i32 = 12;
const MAX_WALLPAPERS: usize = 16;

/// Compute how many thumbnail columns fit in the content area.
fn thumb_cols(win_w: u32) -> usize {
    let cw = win_w.saturating_sub(SIDEBAR_W + 40);
    let inner = cw.saturating_sub(PAD as u32 * 2);
    let col_w = THUMB_W + THUMB_PAD as u32;
    // (inner + THUMB_PAD) / col_w — the extra THUMB_PAD accounts for no trailing pad
    let cols = (inner + THUMB_PAD as u32) / col_w;
    (cols as usize).max(1)
}

struct WallpaperEntry {
    name: [u8; 56],
    name_len: usize,
    path: [u8; 128],
    path_len: usize,
    thumbnail: alloc::vec::Vec<u32>,
    loaded: bool,
}

fn main() {
    let win = window::create("Settings", 180, 100, 560, 400);
    if win == u32::MAX { return; }

    // Set up menu bar
    let mut mb = window::MenuBarBuilder::new()
        .menu("File")
            .item(1, "Close", 0)
        .end_menu()
        .menu("View")
            .item(10, "General", 0)
            .item(11, "Display", 0)
            .item(12, "Wallpaper", 0)
            .item(13, "Network", 0)
            .item(14, "About", 0)
        .end_menu();
    let data = mb.build();
    window::set_menu(win, data);

    let (mut win_w, mut win_h) = window::get_size(win).unwrap_or((560, 400));

    // --- Components with built-in event handling ---
    let mut sidebar = UiSidebar::new(0, 0, SIDEBAR_W, win_h);
    let content_x = SIDEBAR_W as i32 + 20;

    // General page toggles
    let mut dark_toggle = UiToggle::new(0, 0, window::get_theme() == 0);
    let mut sound_toggle = UiToggle::new(0, 0, true);
    let mut notif_toggle = UiToggle::new(0, 0, true);

    // Display page slider + resolution radio
    let mut brightness = UiSlider::new(0, 0, 200, 0, 100, 80);
    let mut res_radio = UiRadioGroup::new(0, 0, 24);

    // Fetch available resolutions
    let resolutions = window::list_resolutions();
    let (cur_w, cur_h) = window::screen_size();
    // Find current resolution index
    for (i, &(rw, rh)) in resolutions.iter().enumerate() {
        if rw == cur_w && rh == cur_h {
            res_radio.selected = i;
            break;
        }
    }

    // Wallpaper state
    let mut wallpapers: alloc::vec::Vec<WallpaperEntry> = alloc::vec::Vec::new();
    let mut wallpaper_selected: usize = 0;
    let mut wallpapers_scanned = false;

    let mut event = [0u32; 5];
    let mut needs_redraw = true;
    let mut scroll_y: u32 = 0;
    let mut prev_page = sidebar.selected;

    // Suppress unused variable warning
    let _ = content_x;

    loop {
        // Lazy-load wallpaper thumbnails when page is first shown
        if sidebar.selected == PAGE_WALLPAPER && !wallpapers_scanned {
            scan_wallpapers(&mut wallpapers, &mut wallpaper_selected);
            wallpapers_scanned = true;
            needs_redraw = true;
        }

        while window::get_event(win, &mut event) == 1 {
            let ui_event = UiEvent::from_raw(&event);

            match event[0] {
                window::EVENT_KEY_DOWN => {
                    if event[1] == 0x103 {
                        window::destroy(win);
                        return;
                    }
                }
                window::EVENT_RESIZE => {
                    win_w = event[1];
                    win_h = event[2];
                    sidebar.h = win_h;
                    needs_redraw = true;
                }
                window::EVENT_MOUSE_DOWN => {
                    // Update component positions before hit testing
                    update_positions(&sidebar, &mut dark_toggle, &mut sound_toggle, &mut notif_toggle, &mut brightness, &mut res_radio, resolutions.len(), win_w, scroll_y);

                    // Let each component handle the event
                    if sidebar.handle_event(&ui_event, PAGE_NAMES.len()).is_some() {
                        needs_redraw = true;
                    }

                    if sidebar.selected == PAGE_GENERAL {
                        if dark_toggle.handle_event(&ui_event).is_some() {
                            window::set_theme(if dark_toggle.on { 0 } else { 1 });
                            needs_redraw = true;
                        }
                        if sound_toggle.handle_event(&ui_event).is_some() {
                            needs_redraw = true;
                        }
                        if notif_toggle.handle_event(&ui_event).is_some() {
                            needs_redraw = true;
                        }
                    }

                    if sidebar.selected == PAGE_DISPLAY {
                        if brightness.handle_event(&ui_event).is_some() {
                            needs_redraw = true;
                        }
                        if let Some(idx) = res_radio.handle_event(&ui_event, resolutions.len()) {
                            if idx < resolutions.len() {
                                let (rw, rh) = resolutions[idx];
                                if window::set_resolution(rw, rh) {
                                    if let Some((nw, nh)) = window::get_size(win) {
                                        win_w = nw;
                                        win_h = nh;
                                        sidebar.h = win_h;
                                    }
                                }
                                needs_redraw = true;
                            }
                        }
                    }

                    if sidebar.selected == PAGE_WALLPAPER {
                        if let Some(idx) = hit_test_wallpaper(&wallpapers, &ui_event, SIDEBAR_W as i32 + 20, scroll_y, win_w) {
                            if idx != wallpaper_selected {
                                wallpaper_selected = idx;
                                let wp = &wallpapers[idx];
                                let path = core::str::from_utf8(&wp.path[..wp.path_len]).unwrap_or("");
                                window::set_wallpaper(path);
                                save_wallpaper_preference(path);
                                needs_redraw = true;
                            }
                        }
                    }
                }
                7 => { // EVENT_MOUSE_SCROLL
                    let dz = event[1] as i32;
                    let content_h = page_content_height(sidebar.selected, resolutions.len(), wallpapers.len(), win_w);
                    let max_scroll = content_h.saturating_sub(win_h);
                    if dz < 0 {
                        scroll_y = scroll_y.saturating_sub((-dz) as u32 * 30);
                    } else if dz > 0 {
                        scroll_y = (scroll_y + dz as u32 * 30).min(max_scroll);
                    }
                    needs_redraw = true;
                }
                window::EVENT_MENU_ITEM => {
                    let item_id = event[2];
                    match item_id {
                        1 => { window::destroy(win); return; }
                        10 => { sidebar.selected = PAGE_GENERAL; needs_redraw = true; }
                        11 => { sidebar.selected = PAGE_DISPLAY; needs_redraw = true; }
                        12 => { sidebar.selected = PAGE_WALLPAPER; needs_redraw = true; }
                        13 => { sidebar.selected = PAGE_NETWORK; needs_redraw = true; }
                        14 => { sidebar.selected = PAGE_ABOUT; needs_redraw = true; }
                        _ => {}
                    }
                }
                window::EVENT_WINDOW_CLOSE => {
                    window::destroy(win);
                    return;
                }
                0x0050 => {
                    dark_toggle.on = window::get_theme() == 0;
                    needs_redraw = true;
                }
                _ => {}
            }
        }

        if sidebar.selected != prev_page {
            scroll_y = 0;
            prev_page = sidebar.selected;
        }

        if needs_redraw {
            update_positions(&sidebar, &mut dark_toggle, &mut sound_toggle, &mut notif_toggle, &mut brightness, &mut res_radio, resolutions.len(), win_w, scroll_y);
            render(win, &sidebar, &dark_toggle, &sound_toggle, &notif_toggle, &brightness, &res_radio, &resolutions, &wallpapers, wallpaper_selected, win_w, win_h, scroll_y);
            window::present(win);
            needs_redraw = false;
        }

        process::sleep(16);
    }
}

// ============================================================================
// Wallpaper scanning & thumbnail generation
// ============================================================================

fn scan_wallpapers(wallpapers: &mut alloc::vec::Vec<WallpaperEntry>, selected: &mut usize) {
    let mut dir_buf = [0u8; 64 * 32];
    let count = fs::readdir("/media/wallpapers", &mut dir_buf);
    if count == u32::MAX || count == 0 { return; }

    let mut current_path = [0u8; 128];
    let mut current_path_len = 0usize;
    read_current_wallpaper_pref(&mut current_path, &mut current_path_len);

    for i in 0..count as usize {
        if wallpapers.len() >= MAX_WALLPAPERS { break; }

        let raw_entry = &dir_buf[i * 64..(i + 1) * 64];
        let entry_type = raw_entry[0];
        let name_len = raw_entry[1] as usize;
        if entry_type != 0 || name_len == 0 { continue; }

        let nlen = name_len.min(56);
        let name_bytes = &raw_entry[8..8 + nlen];

        if !is_image_file(name_bytes, nlen) { continue; }

        let prefix = b"/media/wallpapers/";
        let path_len = prefix.len() + nlen;
        if path_len > 127 { continue; }

        let mut path = [0u8; 128];
        path[..prefix.len()].copy_from_slice(prefix);
        path[prefix.len()..prefix.len() + nlen].copy_from_slice(&name_bytes[..nlen]);

        let mut entry = WallpaperEntry {
            name: [0u8; 56],
            name_len: nlen,
            path,
            path_len,
            thumbnail: alloc::vec::Vec::new(),
            loaded: false,
        };
        entry.name[..nlen].copy_from_slice(&name_bytes[..nlen]);
        wallpapers.push(entry);
    }

    wallpapers.sort_unstable_by(|a, b| {
        cmp_name_ci(&a.name[..a.name_len], &b.name[..b.name_len])
    });

    // Find selected index after sort
    if current_path_len > 0 {
        for (i, wp) in wallpapers.iter().enumerate() {
            if wp.path_len == current_path_len &&
               wp.path[..wp.path_len] == current_path[..current_path_len] {
                *selected = i;
                break;
            }
        }
    }

    // Use mmap for large temporary buffers — munmap actually frees the memory,
    // unlike the bump allocator (sbrk) where dealloc is a no-op.
    const MAX_PIX: usize = 1920 * 1200;
    const FILE_BUF_SIZE: usize = 4 * 1024 * 1024;
    const SCRATCH_SIZE: usize = 32768 + (1920 * 4 + 1) * 1200 + FILE_BUF_SIZE;
    const PIXEL_BUF_SIZE: usize = MAX_PIX * 4; // u32 pixels

    let file_ptr = process::mmap(FILE_BUF_SIZE);
    let pixel_ptr = process::mmap(PIXEL_BUF_SIZE);
    let scratch_ptr = process::mmap(SCRATCH_SIZE);

    if !file_ptr.is_null() && !pixel_ptr.is_null() && !scratch_ptr.is_null() {
        let file_buf = unsafe { core::slice::from_raw_parts_mut(file_ptr, FILE_BUF_SIZE) };
        let pixel_buf = unsafe { core::slice::from_raw_parts_mut(pixel_ptr as *mut u32, MAX_PIX) };
        let scratch_buf = unsafe { core::slice::from_raw_parts_mut(scratch_ptr, SCRATCH_SIZE) };

        for wp in wallpapers.iter_mut() {
            load_thumbnail_mmap(wp, file_buf, pixel_buf, scratch_buf);
        }
    }

    // Free all temporary buffers — physical memory is reclaimed
    if !scratch_ptr.is_null() { process::munmap(scratch_ptr, SCRATCH_SIZE); }
    if !pixel_ptr.is_null() { process::munmap(pixel_ptr, PIXEL_BUF_SIZE); }
    if !file_ptr.is_null() { process::munmap(file_ptr, FILE_BUF_SIZE); }
}

fn is_image_file(name: &[u8], len: usize) -> bool {
    if len < 4 { return false; }
    let lower = |b: u8| -> u8 { if b >= b'A' && b <= b'Z' { b + 32 } else { b } };
    if len >= 4 && lower(name[len-4]) == b'.' && lower(name[len-3]) == b'p' &&
       lower(name[len-2]) == b'n' && lower(name[len-1]) == b'g' { return true; }
    if len >= 4 && lower(name[len-4]) == b'.' && lower(name[len-3]) == b'j' &&
       lower(name[len-2]) == b'p' && lower(name[len-1]) == b'g' { return true; }
    if len >= 5 && lower(name[len-5]) == b'.' && lower(name[len-4]) == b'j' &&
       lower(name[len-3]) == b'p' && lower(name[len-2]) == b'e' && lower(name[len-1]) == b'g' { return true; }
    if len >= 4 && lower(name[len-4]) == b'.' && lower(name[len-3]) == b'b' &&
       lower(name[len-2]) == b'm' && lower(name[len-1]) == b'p' { return true; }
    if len >= 4 && lower(name[len-4]) == b'.' && lower(name[len-3]) == b'g' &&
       lower(name[len-2]) == b'i' && lower(name[len-1]) == b'f' { return true; }
    false
}


fn load_thumbnail_mmap(
    wp: &mut WallpaperEntry,
    file_buf: &mut [u8],
    pixel_buf: &mut [u32],
    scratch_buf: &mut [u8],
) {
    let path = match core::str::from_utf8(&wp.path[..wp.path_len]) {
        Ok(s) => s,
        Err(_) => return,
    };

    let fd = fs::open(path, 0);
    if fd == u32::MAX { return; }

    let mut stat_buf = [0u32; 3];
    if fs::fstat(fd, &mut stat_buf) == u32::MAX {
        fs::close(fd);
        return;
    }
    let file_size = stat_buf[1] as usize;
    if file_size == 0 || file_size > file_buf.len() {
        fs::close(fd);
        return;
    }

    let bytes_read = fs::read(fd, &mut file_buf[..file_size]) as usize;
    fs::close(fd);
    if bytes_read == 0 { return; }

    let info = match libimage_client::probe(&file_buf[..bytes_read]) {
        Some(i) => i,
        None => return,
    };

    let pixel_count = (info.width * info.height) as usize;
    if pixel_count > pixel_buf.len() { return; }

    let scratch_needed = info.scratch_needed as usize;
    if scratch_needed > scratch_buf.len() { return; }

    let mut j = 0;
    while j < pixel_count { pixel_buf[j] = 0; j += 1; }

    if libimage_client::decode(
        &file_buf[..bytes_read],
        &mut pixel_buf[..pixel_count],
        &mut scratch_buf[..scratch_needed],
    ).is_err() {
        return;
    }

    let thumb_size = (THUMB_W * THUMB_H) as usize;
    let mut thumb = alloc::vec![0u32; thumb_size];
    if libimage_client::scale_image(
        &pixel_buf[..pixel_count], info.width, info.height,
        &mut thumb, THUMB_W, THUMB_H,
        libimage_client::MODE_COVER,
    ) {
        wp.thumbnail = thumb;
        wp.loaded = true;
    }
}

fn read_current_wallpaper_pref(path: &mut [u8; 128], path_len: &mut usize) {
    let uid = process::getuid() as u32;
    let fd = fs::open("/System/users/wallpapers", 0);
    if fd == u32::MAX { return; }

    let mut buf = [0u8; 512];
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);

    let data = &buf[..n];
    let mut pos = 0;
    while pos < data.len() {
        let line_end = data[pos..].iter().position(|&b| b == b'\n')
            .map(|p| pos + p).unwrap_or(data.len());
        let line = &data[pos..line_end];
        pos = line_end + 1;

        if let Some(colon) = line.iter().position(|&b| b == b':') {
            let uid_str = &line[..colon];
            let mut parsed_uid: u32 = 0;
            let mut valid = !uid_str.is_empty();
            for &b in uid_str {
                if b >= b'0' && b <= b'9' {
                    parsed_uid = parsed_uid * 10 + (b - b'0') as u32;
                } else { valid = false; break; }
            }
            if valid && parsed_uid == uid {
                let p = &line[colon + 1..];
                let len = p.len().min(127);
                path[..len].copy_from_slice(&p[..len]);
                *path_len = len;
                return;
            }
        }
    }
}

fn save_wallpaper_preference(wallpaper_path: &str) {
    let uid = process::getuid() as u32;

    let mut existing = [0u8; 512];
    let mut existing_len = 0usize;
    let fd = fs::open("/System/users/wallpapers", 0);
    if fd != u32::MAX {
        existing_len = fs::read(fd, &mut existing) as usize;
        fs::close(fd);
    }

    let mut out = [0u8; 512];
    let mut op = 0usize;
    let mut found = false;

    let data = &existing[..existing_len];
    let mut pos = 0;
    while pos < data.len() {
        let line_end = data[pos..].iter().position(|&b| b == b'\n')
            .map(|p| pos + p).unwrap_or(data.len());
        let line = &data[pos..line_end];
        pos = line_end + 1;

        if line.is_empty() { continue; }

        let is_our_uid = if let Some(colon) = line.iter().position(|&b| b == b':') {
            let uid_str = &line[..colon];
            let mut parsed_uid: u32 = 0;
            let mut valid = !uid_str.is_empty();
            for &b in uid_str {
                if b >= b'0' && b <= b'9' {
                    parsed_uid = parsed_uid * 10 + (b - b'0') as u32;
                } else { valid = false; break; }
            }
            valid && parsed_uid == uid
        } else {
            false
        };

        if is_our_uid {
            let uid_bytes = fmt_uid_buf(uid);
            if op + uid_bytes.len() + 1 + wallpaper_path.len() + 1 < out.len() {
                out[op..op + uid_bytes.len()].copy_from_slice(uid_bytes);
                op += uid_bytes.len();
                out[op] = b':';
                op += 1;
                out[op..op + wallpaper_path.len()].copy_from_slice(wallpaper_path.as_bytes());
                op += wallpaper_path.len();
                out[op] = b'\n';
                op += 1;
            }
            found = true;
        } else {
            if op + line.len() + 1 < out.len() {
                out[op..op + line.len()].copy_from_slice(line);
                op += line.len();
                out[op] = b'\n';
                op += 1;
            }
        }
    }

    if !found {
        let uid_bytes = fmt_uid_buf(uid);
        if op + uid_bytes.len() + 1 + wallpaper_path.len() + 1 < out.len() {
            out[op..op + uid_bytes.len()].copy_from_slice(uid_bytes);
            op += uid_bytes.len();
            out[op] = b':';
            op += 1;
            out[op..op + wallpaper_path.len()].copy_from_slice(wallpaper_path.as_bytes());
            op += wallpaper_path.len();
            out[op] = b'\n';
            op += 1;
        }
    }

    let fd = fs::open("/System/users/wallpapers", fs::O_CREATE | fs::O_TRUNC);
    if fd != u32::MAX {
        fs::write(fd, &out[..op]);
        fs::close(fd);
    }
}

fn fmt_uid_buf(uid: u32) -> &'static [u8] {
    static mut BUF: [u8; 8] = [0u8; 8];
    static mut LEN: usize = 0;
    unsafe {
        if uid == 0 {
            BUF[0] = b'0';
            LEN = 1;
        } else {
            let mut v = uid;
            let mut tmp = [0u8; 8];
            let mut n = 0;
            while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
            for i in 0..n { BUF[i] = tmp[n - 1 - i]; }
            LEN = n;
        }
        &BUF[..LEN]
    }
}

fn hit_test_wallpaper(
    wallpapers: &[WallpaperEntry],
    event: &UiEvent,
    cx: i32,
    scroll_y: u32,
    win_w: u32,
) -> Option<usize> {
    if event.event_type != window::EVENT_MOUSE_DOWN { return None; }
    let (mx, my) = event.mouse_pos();

    let cols = thumb_cols(win_w);
    let sy = scroll_y as i32;
    let card_y = 54 - sy;
    let grid_y = card_y + PAD + 36;
    let grid_x = cx + PAD;

    for (i, _wp) in wallpapers.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let tx = grid_x + col as i32 * (THUMB_W as i32 + THUMB_PAD);
        let ty = grid_y + row as i32 * (THUMB_H as i32 + THUMB_PAD + 16);

        if mx >= tx && mx < tx + THUMB_W as i32 &&
           my >= ty && my < ty + THUMB_H as i32 {
            return Some(i);
        }
    }
    None
}

fn cmp_name_ci(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    let lower = |c: u8| -> u8 { if c >= b'A' && c <= b'Z' { c + 32 } else { c } };
    let len = a.len().min(b.len());
    for i in 0..len {
        let la = lower(a[i]);
        let lb = lower(b[i]);
        if la != lb { return la.cmp(&lb); }
    }
    a.len().cmp(&b.len())
}

fn update_positions(
    sidebar: &UiSidebar,
    dark: &mut UiToggle,
    sound: &mut UiToggle,
    notif: &mut UiToggle,
    brightness: &mut UiSlider,
    res_radio: &mut UiRadioGroup,
    num_resolutions: usize,
    win_w: u32,
    scroll_y: u32,
) {
    let cx = SIDEBAR_W as i32 + 20;
    let cw = win_w as i32 - SIDEBAR_W as i32 - 40;
    let sy = scroll_y as i32;
    let card_y = 54 - sy;
    let toggle_x = cx + cw - PAD - 36;

    dark.x = toggle_x;
    dark.y = card_y + PAD + ROW_H + 10;

    sound.x = toggle_x;
    sound.y = card_y + PAD + ROW_H * 2 + 10;

    notif.x = toggle_x;
    notif.y = card_y + PAD + ROW_H * 3 + 10;

    let brightness_row_y = card_y + PAD + ROW_H * 2;
    brightness.x = cx + PAD + 100;
    brightness.y = brightness_row_y + 6;
    brightness.w = (cw - PAD * 2 - 100) as u32;

    let res_card_y = card_y + PAD * 2 + ROW_H * 3 + 16;
    res_radio.x = cx + PAD;
    res_radio.y = res_card_y + PAD + ROW_H + 4;
    res_radio.spacing = 24;
}

fn page_content_height(page: usize, num_resolutions: usize, num_wallpapers: usize, win_w: u32) -> u32 {
    match page {
        PAGE_GENERAL => (54 + PAD * 2 + ROW_H * 4 + 20) as u32,
        PAGE_DISPLAY => {
            let card1_h = PAD * 2 + ROW_H * 3;
            let card2_h = PAD * 2 + ROW_H + num_resolutions as i32 * 24;
            (54 + card1_h + 32 + card2_h + 20) as u32
        }
        PAGE_WALLPAPER => {
            let cols = thumb_cols(win_w);
            let rows = (num_wallpapers.max(1) + cols - 1) / cols;
            let grid_h = rows as i32 * (THUMB_H as i32 + THUMB_PAD + 16);
            (54 + PAD * 2 + 36 + grid_h + 20) as u32
        }
        PAGE_NETWORK => (54 + PAD * 2 + ROW_H * 6 + 20) as u32,
        PAGE_ABOUT => (54 + PAD * 2 + ROW_H * 5 + 20) as u32,
        _ => 400,
    }
}

// ============================================================================
// Rendering
// ============================================================================

fn render(
    win: u32,
    sidebar_c: &UiSidebar,
    dark: &UiToggle,
    sound: &UiToggle,
    notif: &UiToggle,
    brightness: &UiSlider,
    res_radio: &UiRadioGroup,
    resolutions: &[(u32, u32)],
    wallpapers: &[WallpaperEntry],
    wallpaper_selected: usize,
    win_w: u32, win_h: u32,
    scroll_y: u32,
) {
    window::fill_rect(win, 0, 0, win_w as u16, win_h as u16, colors::WINDOW_BG());

    sidebar_c.render(win, "SETTINGS", &PAGE_NAMES);

    let cx = SIDEBAR_W as i32 + 20;
    let cw = (win_w - SIDEBAR_W - 40) as u32;
    let sy = scroll_y as i32;

    match sidebar_c.selected {
        PAGE_GENERAL => render_general(win, dark, sound, notif, cx, cw, sy),
        PAGE_DISPLAY => render_display(win, brightness, res_radio, resolutions, cx, cw, sy),
        PAGE_WALLPAPER => render_wallpaper(win, wallpapers, wallpaper_selected, cx, cw, sy, win_w),
        PAGE_NETWORK => render_network(win, cx, cw, sy),
        PAGE_ABOUT => render_about(win, cx, cw, sy),
        _ => {}
    }

    let content_h = page_content_height(sidebar_c.selected, resolutions.len(), wallpapers.len(), win_w);
    let sb_x = SIDEBAR_W as i32;
    let sb_w = win_w - SIDEBAR_W;
    scrollbar(win, sb_x, 0, sb_w, win_h, content_h, scroll_y);
}

fn render_general(win: u32, dark: &UiToggle, sound: &UiToggle, notif: &UiToggle, cx: i32, cw: u32, sy: i32) {
    label(win, cx, 20 - sy, "General", colors::TEXT(), FontSize::Title, TextAlign::Left);

    let card_y = 54 - sy;
    card(win, cx, card_y, cw, (PAD * 2 + ROW_H * 4) as u32);

    let ry = card_y + PAD;
    label(win, cx + PAD, ry + 12, "Device Name", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    label(win, cx + PAD + 140, ry + 12, "anyOS Computer", colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, cx + PAD, ry, cw - PAD as u32 * 2);
    label(win, cx + PAD, ry + 12, "Dark Mode", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    dark.render(win);

    let ry = ry + ROW_H;
    divider_h(win, cx + PAD, ry, cw - PAD as u32 * 2);
    label(win, cx + PAD, ry + 12, "Sound", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    sound.render(win);

    let ry = ry + ROW_H;
    divider_h(win, cx + PAD, ry, cw - PAD as u32 * 2);
    label(win, cx + PAD, ry + 12, "Notifications", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    notif.render(win);
}

fn render_display(win: u32, brightness: &UiSlider, res_radio: &UiRadioGroup, resolutions: &[(u32, u32)], cx: i32, cw: u32, sy: i32) {
    label(win, cx, 20 - sy, "Display", colors::TEXT(), FontSize::Title, TextAlign::Left);

    let card_y = 54 - sy;
    card(win, cx, card_y, cw, (PAD * 2 + ROW_H * 3) as u32);

    let ry = card_y + PAD;
    label(win, cx + PAD, ry + 12, "GPU Driver", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let gpu_name = window::gpu_name();
    label(win, cx + PAD + 120, ry + 12, &gpu_name, colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, cx + PAD, ry, cw - PAD as u32 * 2);
    label(win, cx + PAD, ry + 12, "Resolution", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let (sw, sh) = window::screen_size();
    let mut buf = [0u8; 32];
    let res = fmt_resolution(&mut buf, sw, sh);
    label(win, cx + PAD + 120, ry + 12, res, colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, cx + PAD, ry, cw - PAD as u32 * 2);
    label(win, cx + PAD, ry + 12, "Brightness", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    brightness.render(win);

    let res_card_y = card_y + PAD * 2 + ROW_H * 3 + 16;
    let num_res = resolutions.len();
    let res_card_h = (PAD * 2 + ROW_H + num_res as i32 * 24) as u32;
    card(win, cx, res_card_y, cw, res_card_h);

    let ry = res_card_y + PAD;
    label(win, cx + PAD, ry + 4, "Change Resolution", colors::TEXT(), FontSize::Normal, TextAlign::Left);

    let mut label_bufs: [[u8; 32]; 16] = [[0u8; 32]; 16];
    let mut label_lens: [usize; 16] = [0; 16];
    let count = num_res.min(16);
    for i in 0..count {
        let (rw, rh) = resolutions[i];
        let s = fmt_resolution(&mut label_bufs[i], rw, rh);
        label_lens[i] = s.len();
    }
    let mut labels: [&str; 16] = [""; 16];
    for i in 0..count {
        labels[i] = unsafe { core::str::from_utf8_unchecked(&label_bufs[i][..label_lens[i]]) };
    }
    res_radio.render(win, &labels[..count]);
}

fn render_wallpaper(
    win: u32,
    wallpapers: &[WallpaperEntry],
    selected: usize,
    cx: i32,
    cw: u32,
    sy: i32,
    win_w: u32,
) {
    label(win, cx, 20 - sy, "Wallpaper", colors::TEXT(), FontSize::Title, TextAlign::Left);

    let cols = thumb_cols(win_w);
    let card_y = 54 - sy;
    let rows = (wallpapers.len().max(1) + cols - 1) / cols;
    let grid_h = rows as i32 * (THUMB_H as i32 + THUMB_PAD + 16);
    let card_h = (PAD * 2 + 36 + grid_h) as u32;
    card(win, cx, card_y, cw, card_h);

    label(win, cx + PAD, card_y + PAD + 4, "Choose a wallpaper", colors::TEXT(), FontSize::Normal, TextAlign::Left);

    let grid_y = card_y + PAD + 36;
    let grid_x = cx + PAD;
    let accent = colors::ACCENT();
    let border_color = colors::SEPARATOR();

    for (i, wp) in wallpapers.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let tx = grid_x + col as i32 * (THUMB_W as i32 + THUMB_PAD);
        let ty = grid_y + row as i32 * (THUMB_H as i32 + THUMB_PAD + 16);

        let is_selected = i == selected;

        if is_selected {
            window::fill_rect(win, (tx - 3) as i16, (ty - 3) as i16,
                (THUMB_W + 6) as u16, (THUMB_H + 6) as u16, accent);
        } else {
            window::fill_rect(win, (tx - 1) as i16, (ty - 1) as i16,
                (THUMB_W + 2) as u16, (THUMB_H + 2) as u16, border_color);
        }

        if wp.loaded && wp.thumbnail.len() == (THUMB_W * THUMB_H) as usize {
            window::blit(win, tx as i16, ty as i16, THUMB_W as u16, THUMB_H as u16, &wp.thumbnail);
        } else {
            window::fill_rect(win, tx as i16, ty as i16, THUMB_W as u16, THUMB_H as u16, 0xFF3A3A3E);
        }

        let name = display_name(&wp.name, wp.name_len);
        label(win, tx, ty + THUMB_H as i32 + 4, name,
            if is_selected { accent } else { colors::TEXT_SECONDARY() },
            FontSize::Small, TextAlign::Left);
    }
}

fn display_name<'a>(name: &'a [u8], len: usize) -> &'a str {
    let end = name[..len].iter().rposition(|&b| b == b'.').unwrap_or(len);
    core::str::from_utf8(&name[..end]).unwrap_or("?")
}

fn render_network(win: u32, cx: i32, cw: u32, sy: i32) {
    label(win, cx, 20 - sy, "Network", colors::TEXT(), FontSize::Title, TextAlign::Left);

    let mut net_buf = [0u8; 24];
    net::get_config(&mut net_buf);

    let ip = [net_buf[0], net_buf[1], net_buf[2], net_buf[3]];
    let mask = [net_buf[4], net_buf[5], net_buf[6], net_buf[7]];
    let gw = [net_buf[8], net_buf[9], net_buf[10], net_buf[11]];
    let dns_ip = [net_buf[12], net_buf[13], net_buf[14], net_buf[15]];
    let mac = [net_buf[16], net_buf[17], net_buf[18], net_buf[19], net_buf[20], net_buf[21]];
    let link_up = net_buf[22] != 0;

    let card_y = 54 - sy;
    card(win, cx, card_y, cw, (PAD * 2 + ROW_H * 6) as u32);

    let lx = cx + PAD;
    let vx = cx + PAD + 130;

    let ry = card_y + PAD;
    label(win, lx, ry + 12, "Status", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let kind = if link_up { StatusKind::Online } else { StatusKind::Offline };
    let text = if link_up { "Connected" } else { "Disconnected" };
    status_indicator(win, vx, ry + 12, kind, text);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "IP Address", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_ip(&mut b, &ip), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Subnet Mask", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_ip(&mut b, &mask), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Gateway", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_ip(&mut b, &gw), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "DNS Server", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_ip(&mut b, &dns_ip), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "MAC Address", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_mac(&mut b, &mac), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
}

fn render_about(win: u32, cx: i32, cw: u32, sy: i32) {
    label(win, cx, 20 - sy, "About", colors::TEXT(), FontSize::Title, TextAlign::Left);

    let card_y = 54 - sy;
    card(win, cx, card_y, cw, (PAD * 2 + ROW_H * 5) as u32);

    let lx = cx + PAD;
    let vx = cx + PAD + 130;

    let ry = card_y + PAD;
    label(win, lx, ry + 12, "OS", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    label(win, vx, ry + 12, "anyOS 1.0", colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Kernel", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    label(win, vx, ry + 12, "x86_64-anyos", colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "CPUs", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let cpu_count = sys::sysinfo(2, &mut [0u8; 4]);
    let mut b = [0u8; 8];
    label(win, vx, ry + 12, fmt_u32(&mut b, cpu_count), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Memory", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut mem = [0u8; 8];
    sys::sysinfo(0, &mut mem);
    let total = u32::from_le_bytes([mem[0], mem[1], mem[2], mem[3]]);
    let free = u32::from_le_bytes([mem[4], mem[5], mem[6], mem[7]]);
    let mut b = [0u8; 32];
    label(win, vx, ry + 12, fmt_mem(&mut b, (total * 4) / 1024, (free * 4) / 1024), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Uptime", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 32];
    let hz = sys::tick_hz().max(1);
    label(win, vx, ry + 12, fmt_uptime(&mut b, sys::uptime() / hz), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
}

// ============================================================================
// Formatting
// ============================================================================

fn fmt_ip<'a>(buf: &'a mut [u8; 20], ip: &[u8; 4]) -> &'a str {
    let mut p = 0;
    for i in 0..4 {
        p = write_u8_dec(buf, p, ip[i]);
        if i < 3 { buf[p] = b'.'; p += 1; }
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_mac<'a>(buf: &'a mut [u8; 20], mac: &[u8; 6]) -> &'a str {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut p = 0;
    for i in 0..6 {
        buf[p] = HEX[(mac[i] >> 4) as usize]; buf[p + 1] = HEX[(mac[i] & 0xF) as usize]; p += 2;
        if i < 5 { buf[p] = b':'; p += 1; }
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_u32<'a>(buf: &'a mut [u8; 8], val: u32) -> &'a str {
    if val == 0 { buf[0] = b'0'; return unsafe { core::str::from_utf8_unchecked(&buf[..1]) }; }
    let mut v = val; let mut tmp = [0u8; 8]; let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    for i in 0..n { buf[i] = tmp[n - 1 - i]; }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

fn fmt_resolution<'a>(buf: &'a mut [u8; 32], w: u32, h: u32) -> &'a str {
    let mut p = 0; let mut t = [0u8; 8];
    let s = fmt_u32(&mut t, w); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 3].copy_from_slice(b" x "); p += 3;
    let s = fmt_u32(&mut t, h); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_mem<'a>(buf: &'a mut [u8; 32], total: u32, free: u32) -> &'a str {
    let mut p = 0; let mut t = [0u8; 8];
    let s = fmt_u32(&mut t, total); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 5].copy_from_slice(b" MB ("); p += 5;
    let s = fmt_u32(&mut t, free); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 9].copy_from_slice(b" MB free)"); p += 9;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn fmt_uptime<'a>(buf: &'a mut [u8; 32], secs: u32) -> &'a str {
    let mut p = 0; let mut t = [0u8; 8];
    let s = fmt_u32(&mut t, secs / 3600); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 2].copy_from_slice(b"h "); p += 2;
    let s = fmt_u32(&mut t, (secs % 3600) / 60); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p..p + 2].copy_from_slice(b"m "); p += 2;
    let s = fmt_u32(&mut t, secs % 60); buf[p..p + s.len()].copy_from_slice(s.as_bytes()); p += s.len();
    buf[p] = b's'; p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

fn write_u8_dec(buf: &mut [u8], pos: usize, val: u8) -> usize {
    let mut p = pos;
    if val >= 100 { buf[p] = b'0' + val / 100; p += 1; }
    if val >= 10 { buf[p] = b'0' + (val / 10) % 10; p += 1; }
    buf[p] = b'0' + val % 10; p + 1
}
