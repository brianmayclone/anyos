//! Settings page: Display — GPU info, resolution picker, and wallpaper.
//!
//! Combines display information (GPU driver, acceleration status, current
//! resolution), an interactive resolution picker (dropdown), and a wallpaper
//! browser that scans `/media/wallpapers/` and shows thumbnails.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::ipc;
use anyos_std::process;
use anyos_std::ui::window;
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── Constants ───────────────────────────────────────────────────────────────

const WALLPAPER_DIR: &str = "/media/wallpapers";
const THUMB_W: u32 = 120;
const THUMB_H: u32 = 80;
const MAX_WALLPAPERS: usize = 24;

// ── Wallpaper entry ─────────────────────────────────────────────────────────

struct WallpaperEntry {
    name: String,
    path: String,
    thumbnail: Vec<u32>,
}

// ── Build ───────────────────────────────────────────────────────────────────

/// Build the Display settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::BG);

    // ── Page header ─────────────────────────────────────────────────────
    layout::build_page_header(&panel, "Display", "Monitor, resolution and wallpaper");

    // ── Display Info card ───────────────────────────────────────────────
    let info_card = layout::build_auto_card(&panel);

    // GPU Driver
    let gpu = window::gpu_name();
    layout::build_info_row(&info_card, "GPU Driver", &gpu, true);

    layout::build_separator(&info_card);

    // Hardware acceleration
    let accel = if window::gpu_has_accel() {
        "Available"
    } else {
        "Not available"
    };
    let accel_color = if window::gpu_has_accel() {
        0xFF4EC970
    } else {
        0xFFE06C75
    };
    layout::build_info_row_colored(&info_card, "Acceleration", accel, accel_color, false);

    layout::build_separator(&info_card);

    // Current Resolution
    let (sw, sh) = window::screen_size();
    let res_str = format!("{} x {}", sw, sh);
    layout::build_info_row(&info_card, "Current Resolution", &res_str, false);

    // ── Resolution picker card ──────────────────────────────────────────
    let resolutions = window::list_resolutions();
    if !resolutions.is_empty() {
        let res_card = layout::build_auto_card(&panel);

        let row = layout::build_setting_row(&res_card, "Resolution", true);

        // Build pipe-separated items string
        let mut items = String::new();
        let mut current_idx: u32 = 0;
        for (i, &(rw, rh)) in resolutions.iter().enumerate() {
            if i > 0 {
                items.push('|');
            }
            items.push_str(&format!("{} x {}", rw, rh));
            if rw == sw && rh == sh {
                current_idx = i as u32;
            }
        }

        let dropdown = ui::DropDown::new(&items);
        dropdown.set_position(200, 8);
        dropdown.set_size(280, 28);
        dropdown.set_selected_index(current_idx);

        // On selection change: apply the resolution
        let res_copy: Vec<(u32, u32)> = resolutions.clone();
        dropdown.on_selection_changed(move |e| {
            let idx = e.index as usize;
            if idx < res_copy.len() {
                let (rw, rh) = res_copy[idx];
                window::set_resolution(rw, rh);
            }
        });
        row.add(&dropdown);
    }

    // ── Wallpaper card ──────────────────────────────────────────────────
    let wallpapers = scan_wallpapers();

    let wp_card = layout::build_auto_card(&panel);
    // Section title inside card
    let hdr_row = ui::View::new();
    hdr_row.set_dock(ui::DOCK_TOP);
    hdr_row.set_size(552, 36);
    hdr_row.set_margin(24, 8, 24, 0);
    let hdr_lbl = ui::Label::new("Wallpaper");
    hdr_lbl.set_position(0, 8);
    hdr_lbl.set_size(200, 20);
    hdr_lbl.set_text_color(0xFFFFFFFF);
    hdr_lbl.set_font_size(14);
    hdr_row.add(&hdr_lbl);
    wp_card.add(&hdr_row);

    if wallpapers.is_empty() {
        let empty = ui::Label::new("No wallpapers found in /media/wallpapers/");
        empty.set_dock(ui::DOCK_TOP);
        empty.set_size(552, 30);
        empty.set_font_size(12);
        empty.set_text_color(0xFF969696);
        empty.set_margin(24, 4, 24, 8);
        wp_card.add(&empty);
    } else {
        let flow = ui::FlowPanel::new();
        flow.set_dock(ui::DOCK_TOP);
        let cols = 4usize;
        let rows = (wallpapers.len() + cols - 1) / cols;
        let flow_h = (rows as u32) * (THUMB_H + 36) + 16;
        flow.set_size(552, flow_h);
        flow.set_margin(16, 4, 16, 8);

        for wp in &wallpapers {
            let cell = ui::View::new();
            cell.set_size(THUMB_W + 8, THUMB_H + 28);
            cell.set_margin(4, 4, 4, 4);

            let canvas = ui::Canvas::new(THUMB_W, THUMB_H);
            canvas.set_position(4, 4);
            canvas.set_size(THUMB_W, THUMB_H);

            if !wp.thumbnail.is_empty() {
                canvas.copy_pixels_from(&wp.thumbnail);
            } else {
                canvas.clear(0xFF3A3A3E);
            }

            let path = wp.path.clone();
            canvas.on_click(move |_| {
                set_wallpaper_ipc(&path);
                save_wallpaper_pref(&path);
            });
            cell.add(&canvas);

            let name_label = ui::Label::new(&wp.name);
            name_label.set_position(4, THUMB_H as i32 + 6);
            name_label.set_size(THUMB_W, 18);
            name_label.set_font_size(10);
            name_label.set_text_color(0xFF969696);
            cell.add(&name_label);

            flow.add(&cell);
        }

        wp_card.add(&flow);
    }

    parent.add(&panel);
    panel.id()
}

// ── Wallpaper scanning ──────────────────────────────────────────────────────

fn scan_wallpapers() -> Vec<WallpaperEntry> {
    let mut entries = Vec::new();

    let mut dir_buf = [0u8; 64 * 32];
    let count = fs::readdir(WALLPAPER_DIR, &mut dir_buf);
    if count == u32::MAX || count == 0 {
        return entries;
    }

    // Collect filenames
    let mut names: Vec<String> = Vec::new();
    for i in 0..count as usize {
        if names.len() >= MAX_WALLPAPERS {
            break;
        }
        let raw = &dir_buf[i * 64..(i + 1) * 64];
        let entry_type = raw[0];
        let name_len = raw[1] as usize;
        if entry_type != 0 || name_len == 0 {
            continue;
        }
        let nlen = name_len.min(56);
        let name = match core::str::from_utf8(&raw[8..8 + nlen]) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if is_image(name) {
            names.push(String::from(name));
        }
    }

    names.sort_unstable();

    // Shared decode buffers via mmap
    const MAX_PIX: usize = 1920 * 1200;
    const FILE_BUF_SIZE: usize = 4 * 1024 * 1024;
    const SCRATCH_SIZE: usize = 32768 + (1920 * 4 + 1) * 1200 + FILE_BUF_SIZE;

    let file_ptr = process::mmap(FILE_BUF_SIZE);
    let pixel_ptr = process::mmap(MAX_PIX * 4);
    let scratch_ptr = process::mmap(SCRATCH_SIZE);

    let can_decode = !file_ptr.is_null() && !pixel_ptr.is_null() && !scratch_ptr.is_null();

    for name in &names {
        let path = format!("{}/{}", WALLPAPER_DIR, name);
        let display = name
            .rfind('.')
            .map(|i| &name[..i])
            .unwrap_or(name);

        let thumbnail = if can_decode {
            load_thumbnail(
                &path, file_ptr, pixel_ptr, scratch_ptr,
                FILE_BUF_SIZE, MAX_PIX, SCRATCH_SIZE,
            )
        } else {
            Vec::new()
        };

        entries.push(WallpaperEntry {
            name: String::from(display),
            path,
            thumbnail,
        });
    }

    if !scratch_ptr.is_null() {
        process::munmap(scratch_ptr, SCRATCH_SIZE);
    }
    if !pixel_ptr.is_null() {
        process::munmap(pixel_ptr, MAX_PIX * 4);
    }
    if !file_ptr.is_null() {
        process::munmap(file_ptr, FILE_BUF_SIZE);
    }

    entries
}

fn load_thumbnail(
    path: &str,
    file_ptr: *mut u8,
    pixel_ptr: *mut u8,
    scratch_ptr: *mut u8,
    file_buf_size: usize,
    max_pix: usize,
    scratch_size: usize,
) -> Vec<u32> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Vec::new();
    }

    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) != 0 {
        fs::close(fd);
        return Vec::new();
    }
    let file_size = stat_buf[1] as usize;
    if file_size == 0 || file_size > file_buf_size {
        fs::close(fd);
        return Vec::new();
    }

    let file_buf = unsafe { core::slice::from_raw_parts_mut(file_ptr, file_buf_size) };
    let bytes_read = fs::read(fd, &mut file_buf[..file_size]) as usize;
    fs::close(fd);
    if bytes_read == 0 {
        return Vec::new();
    }

    let info = match libimage_client::probe(&file_buf[..bytes_read]) {
        Some(i) => i,
        None => return Vec::new(),
    };

    let pixel_count = (info.width * info.height) as usize;
    if pixel_count > max_pix {
        return Vec::new();
    }

    let scratch_needed = info.scratch_needed as usize;
    if scratch_needed > scratch_size {
        return Vec::new();
    }

    let pixel_buf = unsafe { core::slice::from_raw_parts_mut(pixel_ptr as *mut u32, max_pix) };
    let scratch_buf = unsafe { core::slice::from_raw_parts_mut(scratch_ptr, scratch_size) };

    for p in pixel_buf[..pixel_count].iter_mut() {
        *p = 0;
    }

    if libimage_client::decode(
        &file_buf[..bytes_read],
        &mut pixel_buf[..pixel_count],
        &mut scratch_buf[..scratch_needed],
    )
    .is_err()
    {
        return Vec::new();
    }

    let thumb_count = (THUMB_W * THUMB_H) as usize;
    let mut thumb = vec![0u32; thumb_count];
    if libimage_client::scale_image(
        &pixel_buf[..pixel_count],
        info.width,
        info.height,
        &mut thumb,
        THUMB_W,
        THUMB_H,
        libimage_client::MODE_COVER,
    ) {
        thumb
    } else {
        Vec::new()
    }
}

// ── Wallpaper IPC ───────────────────────────────────────────────────────────

fn set_wallpaper_ipc(path: &str) {
    let path_len = path.len() as u32;
    if path_len == 0 || path_len > 255 {
        return;
    }

    let shm_id = ipc::shm_create(path_len + 1);
    if shm_id == 0 {
        return;
    }
    let shm_addr = ipc::shm_map(shm_id);
    if shm_addr == 0 {
        ipc::shm_destroy(shm_id);
        return;
    }

    unsafe {
        let dst = shm_addr as *mut u8;
        core::ptr::copy_nonoverlapping(path.as_ptr(), dst, path_len as usize);
        *dst.add(path_len as usize) = 0;
    }

    const CMD_SET_WALLPAPER: u32 = 0x100F;
    let cmd: [u32; 5] = [CMD_SET_WALLPAPER, shm_id, 0, 0, 0];
    ipc::evt_chan_emit(ui::get_compositor_channel(), &cmd);

    process::sleep(32);
    ipc::shm_unmap(shm_id);
    ipc::shm_destroy(shm_id);
}

// ── Wallpaper preference persistence ────────────────────────────────────────

fn save_wallpaper_pref(path: &str) {
    let uid = process::getuid() as u32;
    let pref_path = "/System/users/wallpapers";

    // Read existing prefs
    let mut existing = [0u8; 512];
    let mut existing_len = 0usize;
    let fd = fs::open(pref_path, 0);
    if fd != u32::MAX {
        existing_len = fs::read(fd, &mut existing) as usize;
        fs::close(fd);
    }

    // Rebuild: replace line for our UID or append
    let mut out = [0u8; 512];
    let mut op = 0usize;
    let mut found = false;

    let data = &existing[..existing_len];
    let mut pos = 0;
    while pos < data.len() {
        let line_end = data[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| pos + p)
            .unwrap_or(data.len());
        let line = &data[pos..line_end];
        pos = line_end + 1;

        if line.is_empty() {
            continue;
        }

        let is_our = if let Some(colon) = line.iter().position(|&b| b == b':') {
            parse_uid_bytes(&line[..colon]) == Some(uid)
        } else {
            false
        };

        if is_our {
            let uid_s = fmt_u32_bytes(uid);
            if op + uid_s.len() + 1 + path.len() + 1 < out.len() {
                out[op..op + uid_s.len()].copy_from_slice(uid_s);
                op += uid_s.len();
                out[op] = b':';
                op += 1;
                out[op..op + path.len()].copy_from_slice(path.as_bytes());
                op += path.len();
                out[op] = b'\n';
                op += 1;
            }
            found = true;
        } else if op + line.len() + 1 < out.len() {
            out[op..op + line.len()].copy_from_slice(line);
            op += line.len();
            out[op] = b'\n';
            op += 1;
        }
    }

    if !found {
        let uid_s = fmt_u32_bytes(uid);
        if op + uid_s.len() + 1 + path.len() + 1 < out.len() {
            out[op..op + uid_s.len()].copy_from_slice(uid_s);
            op += uid_s.len();
            out[op] = b':';
            op += 1;
            out[op..op + path.len()].copy_from_slice(path.as_bytes());
            op += path.len();
            out[op] = b'\n';
            op += 1;
        }
    }

    let fd = fs::open(pref_path, fs::O_CREATE | fs::O_TRUNC);
    if fd != u32::MAX {
        fs::write(fd, &out[..op]);
        fs::close(fd);
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn is_image(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".bmp")
}

fn parse_uid_bytes(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut val: u32 = 0;
    for &b in bytes {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val * 10 + (b - b'0') as u32;
    }
    Some(val)
}

fn fmt_u32_bytes(val: u32) -> &'static [u8] {
    static mut BUF: [u8; 10] = [0; 10];
    static mut LEN: usize = 0;
    unsafe {
        if val == 0 {
            BUF[0] = b'0';
            LEN = 1;
        } else {
            let mut v = val;
            let mut tmp = [0u8; 10];
            let mut n = 0;
            while v > 0 {
                tmp[n] = b'0' + (v % 10) as u8;
                v /= 10;
                n += 1;
            }
            for i in 0..n {
                BUF[i] = tmp[n - 1 - i];
            }
            LEN = n;
        }
        &BUF[..LEN]
    }
}
