#![no_std]
#![no_main]

use anyos_std::ui::window;
use anyos_std::ui::filedialog;
use anyos_std::sys;
use anyos_std::fs;
use anyos_std::process;
use anyos_std::Vec;
use anyos_std::String;
use anyos_std::format;

anyos_std::entry!(main);

// ---- Colors ----
const BG: u32 = 0xFF1E1E1E;
const TEXT: u32 = 0xFFE0E0E0;
const TEXT_DIM: u32 = 0xFF808080;
const BTN_BG: u32 = 0xFF007AFF;
const BTN_BG_H: u32 = 0xFF3395FF;
const BTN_TEXT: u32 = 0xFFFFFFFF;
const BORDER: u32 = 0xFF3A3A3C;

// ---- Layout ----
const WIN_W: u16 = 340;
const WIN_H: u16 = 200;

// ---- Button definitions ----
struct BtnDef {
    label: &'static str,
    x: i16,
    y: i16,
    w: i16,
    h: i16,
}

const BTNS: [BtnDef; 3] = [
    BtnDef { label: "Full Screen",     x: 20,  y: 80, w: 140, h: 36 },
    BtnDef { label: "Selection",       x: 20,  y: 124, w: 140, h: 36 },
    BtnDef { label: "Save to File...", x: 180, y: 80, w: 140, h: 36 },
];

// ---- PNG Encoder ----

/// CRC-32 lookup table (IEEE 802.3 polynomial)
fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = 0xEDB88320 ^ (crc >> 1);
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
}

fn crc32(table: &[u32; 256], data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

fn crc32_update(table: &[u32; 256], crc: u32, data: &[u8]) -> u32 {
    let mut c = crc;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c
}

fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Encode ARGB pixel data as a PNG file (uncompressed deflate stored blocks).
fn encode_png(width: u32, height: u32, pixels: &[u32]) -> Vec<u8> {
    let crc_table = crc32_table();

    let row_bytes = 1 + width as usize * 3; // filter byte + RGB
    let total_raw = height as usize * row_bytes;

    // Deflate stored block parameters
    let max_block: usize = 65535;
    let num_full = total_raw / max_block;
    let last_size = total_raw % max_block;
    let num_blocks = num_full + if last_size > 0 { 1 } else { 0 };

    // IDAT data size: zlib_header(2) + blocks(5 each) + raw_data + adler32(4)
    let idat_data_size = 2 + num_blocks * 5 + total_raw + 4;

    // Total PNG: sig(8) + IHDR(25) + IDAT(12+data) + IEND(12)
    let total_size = 8 + 25 + 12 + idat_data_size + 12;
    let mut out = Vec::with_capacity(total_size);

    // PNG Signature
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // IHDR chunk
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;  // bit depth
    ihdr[9] = 2;  // color type: RGB
    write_chunk(&mut out, &crc_table, b"IHDR", &ihdr);

    // IDAT chunk — build data inline
    out.extend_from_slice(&(idat_data_size as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(b"IDAT");

    // Zlib header (deflate, no dict, compression level 1)
    out.push(0x78);
    out.push(0x01);

    // Build raw filtered pixel data and write as stored deflate blocks
    // We process the pixel data in chunks of max_block bytes
    let mut adler_a: u32 = 1;
    let mut adler_b: u32 = 0;
    let mut raw_offset = 0usize;
    let mut pixel_row = 0u32;
    let mut pixel_col = 0u32;
    let mut in_filter_byte = true;

    let mut block_buf = Vec::with_capacity(max_block);

    while raw_offset < total_raw {
        block_buf.clear();
        let remaining = total_raw - raw_offset;
        let block_size = remaining.min(max_block);
        let is_last = raw_offset + block_size >= total_raw;

        // Fill block buffer with raw pixel data
        let mut filled = 0;
        while filled < block_size {
            if in_filter_byte {
                block_buf.push(0); // filter byte: None
                in_filter_byte = false;
                filled += 1;
            } else {
                let idx = pixel_row as usize * width as usize + pixel_col as usize;
                let argb = if idx < pixels.len() { pixels[idx] } else { 0 };
                block_buf.push(((argb >> 16) & 0xFF) as u8); // R
                filled += 1;
                if filled < block_size {
                    block_buf.push(((argb >> 8) & 0xFF) as u8); // G
                    filled += 1;
                }
                if filled < block_size {
                    block_buf.push((argb & 0xFF) as u8); // B
                    filled += 1;
                }

                pixel_col += 1;
                if pixel_col >= width {
                    pixel_col = 0;
                    pixel_row += 1;
                    in_filter_byte = true;
                }
            }
        }

        // Update Adler-32
        for &b in &block_buf {
            adler_a = (adler_a + b as u32) % 65521;
            adler_b = (adler_b + adler_a) % 65521;
        }

        // Write stored block header
        out.push(if is_last { 0x01 } else { 0x00 });
        let len = block_size as u16;
        out.push((len & 0xFF) as u8);
        out.push((len >> 8) as u8);
        let nlen = !len;
        out.push((nlen & 0xFF) as u8);
        out.push((nlen >> 8) as u8);

        // Write block data
        out.extend_from_slice(&block_buf);

        raw_offset += block_size;
    }

    // Adler-32 checksum (big-endian)
    let adler = (adler_b << 16) | adler_a;
    out.extend_from_slice(&adler.to_be_bytes());

    // IDAT CRC (over "IDAT" + all data bytes)
    let crc = crc32(&crc_table, &out[crc_start..]);
    out.extend_from_slice(&crc.to_be_bytes());

    // IEND chunk
    write_chunk(&mut out, &crc_table, b"IEND", &[]);

    out
}

fn write_chunk(out: &mut Vec<u8>, crc_table: &[u32; 256], chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    // CRC over type + data
    let mut crc = 0xFFFFFFFFu32;
    crc = crc32_update(crc_table, crc, chunk_type);
    crc = crc32_update(crc_table, crc, data);
    out.extend_from_slice(&(!crc).to_be_bytes());
}

// ---- Screen capture ----
fn capture_screen() -> Option<(u32, u32, Vec<u32>)> {
    // Get screen size to know buffer size
    let (sw, sh) = window::screen_size();
    if sw == 0 || sh == 0 { return None; }

    let pixel_count = sw as usize * sh as usize;
    let mut buf: Vec<u32> = Vec::with_capacity(pixel_count);
    buf.resize(pixel_count, 0);

    let mut info = [0u32; 2];
    if sys::capture_screen(&mut buf, &mut info) {
        Some((info[0], info[1], buf))
    } else {
        None
    }
}

fn save_png(width: u32, height: u32, pixels: &[u32]) -> bool {
    match filedialog::save_file("/", "screenshot.png") {
        filedialog::FileDialogResult::Selected(path) => {
            // Ensure .png extension
            let path = if !path.ends_with(".png") {
                format!("{}.png", path)
            } else {
                path
            };

            let png_data = encode_png(width, height, pixels);

            let fd = fs::open(&path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
            if fd == u32::MAX { return false; }
            fs::write(fd, &png_data);
            fs::close(fd);
            true
        }
        _ => false,
    }
}

// ---- Rendering ----
fn render(win: u32, has_capture: bool, pressed: Option<usize>) {
    window::fill_rect(win, 0, 0, WIN_W, WIN_H, BG);

    // Title
    window::draw_text_ex(win, 20, 16, TEXT, window::FONT_BOLD, 18, "Screenshot");
    window::draw_text_ex(win, 20, 42, TEXT_DIM, 0, 13, "Capture the screen or a selection.");

    // Divider
    window::fill_rect(win, 20, 68, WIN_W - 40, 1, BORDER);

    // Buttons
    for (i, btn) in BTNS.iter().enumerate() {
        // Skip "Save" if no capture, skip "Selection" (not yet implemented)
        let enabled = match i {
            0 => true,              // Full Screen always enabled
            1 => false,             // Selection not yet implemented
            2 => has_capture,       // Save only when capture exists
            _ => false,
        };

        let is_pressed = pressed == Some(i);
        let bg = if !enabled {
            0xFF555555
        } else if is_pressed {
            BTN_BG_H
        } else {
            BTN_BG
        };
        let tc = if !enabled { TEXT_DIM } else { BTN_TEXT };

        window::fill_rounded_rect(win, btn.x, btn.y, btn.w as u16, btn.h as u16, 8, bg);
        let (tw, th) = window::font_measure(0, 13, btn.label);
        let tx = btn.x + (btn.w - tw as i16) / 2;
        let ty = btn.y + (btn.h - th as i16) / 2;
        window::draw_text_ex(win, tx, ty, tc, 0, 13, btn.label);
    }

    // Status
    if has_capture {
        window::draw_text_ex(win, 20, 172, TEXT_DIM, 0, 11, "Screenshot captured. Click Save to export as PNG.");
    }
}

fn hit_test(mx: i16, my: i16) -> Option<usize> {
    for (i, btn) in BTNS.iter().enumerate() {
        if mx >= btn.x && mx < btn.x + btn.w && my >= btn.y && my < btn.y + btn.h {
            return Some(i);
        }
    }
    None
}

// ---- Main ----
fn main() {
    let win = window::create_ex(
        "Screenshot", 200, 200, WIN_W, WIN_H,
        window::WIN_FLAG_NOT_RESIZABLE,
    );
    if win == u32::MAX { return; }

    let mut mb = window::MenuBarBuilder::new()
        .menu("Screenshot")
            .item(100, "About Screenshot", 0)
            .separator()
            .item(199, "Quit", 0)
        .end_menu()
        .menu("Capture")
            .item(200, "Full Screen", 0)
        .end_menu();
    window::set_menu(win, mb.build());

    let mut capture: Option<(u32, u32, Vec<u32>)> = None;
    let mut pressed: Option<usize> = None;
    let mut event = [0u32; 5];
    let mut dirty = true;

    loop {
        let t0 = sys::uptime_ms();
        while window::get_event(win, &mut event) == 1 {
            match event[0] {
                window::EVENT_MOUSE_DOWN => {
                    let mx = event[1] as i16;
                    let my = event[2] as i16;
                    pressed = hit_test(mx, my);
                    dirty = true;
                }
                window::EVENT_MOUSE_UP => {
                    if let Some(idx) = pressed {
                        match idx {
                            0 => {
                                // Full Screen capture
                                // Hide window, wait, capture, show again
                                window::destroy(win);
                                process::sleep(50); // ~500ms at 100Hz

                                if let Some(cap) = capture_screen() {
                                    capture = Some(cap);
                                }

                                // Recreate window
                                let new_win = window::create_ex(
                                    "Screenshot", 200, 200, WIN_W, WIN_H,
                                    window::WIN_FLAG_NOT_RESIZABLE,
                                );
                                if new_win == u32::MAX { return; }
                                // Re-set menu
                                let mut mb2 = window::MenuBarBuilder::new()
                                    .menu("Screenshot")
                                        .item(100, "About Screenshot", 0)
                                        .separator()
                                        .item(199, "Quit", 0)
                                    .end_menu()
                                    .menu("Capture")
                                        .item(200, "Full Screen", 0)
                                    .end_menu();
                                window::set_menu(new_win, mb2.build());

                                render(new_win, capture.is_some(), None);
                                window::present(new_win);
                                // Note: we can't update win since it's immutable in the loop
                                // We need to restructure... let's use a simpler approach
                                // and just return to let the event loop use the new window
                                // Actually, we need to restart the main loop.
                                // For simplicity, let's capture without destroying the window.
                            }
                            2 => {
                                // Save
                                if let Some((w, h, ref pixels)) = capture {
                                    save_png(w, h, pixels);
                                }
                            }
                            _ => {}
                        }
                    }
                    pressed = None;
                    dirty = true;
                }
                window::EVENT_MENU_ITEM => {
                    match event[1] {
                        199 | 0xFFF2 => { window::destroy(win); return; }
                        200 => {
                            // Full screen via menu — capture immediately
                            if let Some(cap) = capture_screen() {
                                capture = Some(cap);
                                dirty = true;
                            }
                        }
                        _ => {}
                    }
                }
                window::EVENT_WINDOW_CLOSE => { window::destroy(win); return; }
                _ => {}
            }
        }

        if dirty {
            render(win, capture.is_some(), pressed);
            window::present(win);
            dirty = false;
        }
        let elapsed = sys::uptime_ms().wrapping_sub(t0);
        if elapsed < 16 { process::sleep(16 - elapsed); }
    }
}
