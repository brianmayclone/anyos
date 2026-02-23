#![no_std]
#![no_main]

anyos_std::entry!(main);

use libcompositor_client::{TrayClient, WindowHandle, EVT_STATUS_ICON_CLICK,
    EVT_MOUSE_DOWN, EVT_WINDOW_CLOSE};

use uisys_client::{self, FontSize, TextAlign};
use uisys_client::colors;

// ── Constants ────────────────────────────────────────────────────────────────

const ICON_ID: u32 = 1;
const POPUP_W: u32 = 240;
const BORDERLESS: u32 = 0x01;

const PAD: i32 = 16;
const ROW_H: i32 = 32;

const CONF_PATH: &str = "/System/etc/inputmon.conf";

// ── WinSurface for uisys DLL surface-mode rendering ──────────────────────────

#[repr(C)]
struct WinSurface {
    pixels: *mut u32,
    width: u32,
    height: u32,
}

// ── Minimal 5x7 bitmap font for tray icon ────────────────────────────────────

/// 5x7 glyph data for A-Z. Each glyph is 7 bytes, each byte is a 5-bit row
/// (bit 4 = leftmost pixel).
static MINI_FONT: [[u8; 7]; 26] = [
    // A
    [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
    // B
    [0b11110, 0b10001, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110],
    // C
    [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
    // D
    [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
    // E
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
    // F
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
    // G
    [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
    // H
    [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
    // I
    [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    // J
    [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
    // K
    [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
    // L
    [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
    // M
    [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
    // N
    [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
    // O
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
    // P
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
    // Q
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
    // R
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
    // S
    [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
    // T
    [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
    // U
    [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
    // V
    [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
    // W
    [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010],
    // X
    [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
    // Y
    [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
    // Z
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
];

fn draw_mini_char(pixels: &mut [u32; 256], x0: usize, y0: usize, ch: u8, color: u32) {
    let c = ch.to_ascii_uppercase();
    if c < b'A' || c > b'Z' { return; }
    let idx = (c - b'A') as usize;
    let glyph = &MINI_FONT[idx];
    for (dy, &row) in glyph.iter().enumerate() {
        for dx in 0..5usize {
            if row & (1 << (4 - dx)) != 0 {
                let px = x0 + dx;
                let py = y0 + dy;
                if px < 16 && py < 16 {
                    pixels[py * 16 + px] = color;
                }
            }
        }
    }
}

// ── Icon drawing ─────────────────────────────────────────────────────────────

fn draw_layout_icon(pixels: &mut [u32; 256], layouts: &[anyos_std::kbd::LayoutInfo], current_id: u32) {
    // Clear to semi-transparent dark background
    for p in pixels.iter_mut() { *p = 0; }

    // Find current layout label
    let label = find_label(layouts, current_id);

    // Draw 2 characters centered: char1 at (2,5), char2 at (9,5)
    if label.len() >= 1 {
        draw_mini_char(pixels, 2, 5, label[0], 0xFFE0E0E0);
    }
    if label.len() >= 2 {
        draw_mini_char(pixels, 9, 5, label[1], 0xFFE0E0E0);
    }
}

fn find_label(layouts: &[anyos_std::kbd::LayoutInfo], id: u32) -> &[u8] {
    for l in layouts {
        if l.id == id {
            let len = l.label.iter().position(|&b| b == 0).unwrap_or(4);
            return &l.label[..len];
        }
    }
    b"??"
}

// ── Config persistence (INI-style: [keyboard] section with layout=N) ────────

fn load_config() {
    let fd = anyos_std::fs::open(CONF_PATH, 0); // O_RDONLY
    if fd == u32::MAX { return; }
    let mut buf = [0u8; 256];
    let n = anyos_std::fs::read(fd, &mut buf) as usize;
    anyos_std::fs::close(fd);
    if n == 0 || n > buf.len() { return; }
    let text = match core::str::from_utf8(&buf[..n]) {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut in_keyboard = false;
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if line.starts_with('[') {
            in_keyboard = line == "[keyboard]";
            continue;
        }
        if !in_keyboard { continue; }
        if let Some(val) = line.strip_prefix("layout=") {
            if let Some(id) = parse_u32(val.trim()) {
                anyos_std::kbd::set_layout(id);
                return;
            }
        }
    }
}

fn save_config(id: u32) {
    let fd = anyos_std::fs::open(CONF_PATH, 1 | 4 | 8); // O_WRITE | O_CREATE | O_TRUNC
    if fd == u32::MAX { return; }
    let mut buf = [0u8; 32];
    let header = b"[keyboard]\nlayout=";
    let hlen = header.len();
    buf[..hlen].copy_from_slice(header);
    let vlen = fmt_u32(id, &mut [0u8; 4]);
    let mut id_buf = [0u8; 4];
    fmt_u32(id, &mut id_buf);
    buf[hlen..hlen + vlen].copy_from_slice(&id_buf[..vlen]);
    buf[hlen + vlen] = b'\n';
    anyos_std::fs::write(fd, &buf[..hlen + vlen + 1]);
    anyos_std::fs::close(fd);
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' { return None; }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

fn fmt_u32(mut val: u32, buf: &mut [u8; 4]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 4];
    let mut len = 0;
    while val > 0 && len < 4 {
        tmp[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    for i in 0..len {
        buf[i] = tmp[len - 1 - i];
    }
    len
}

// ── Popup drawing ────────────────────────────────────────────────────────────

fn popup_height(layout_count: usize) -> u32 {
    // title(28) + divider(8) + rows + bottom padding
    (PAD + 28 + 8 + (layout_count as i32) * ROW_H + PAD) as u32
}

fn draw_popup(win: &WindowHandle, layouts: &[anyos_std::kbd::LayoutInfo], layout_count: usize, current_id: u32) {
    let h = popup_height(layout_count);
    let surface = WinSurface {
        pixels: win.surface(),
        width: POPUP_W,
        height: h,
    };
    let w = &surface as *const WinSurface as u32;

    // Background card
    uisys_client::card(w, 0, 0, POPUP_W, h);

    let mut y = PAD;

    // Title
    uisys_client::label(w, PAD, y + 4, "Keyboard", colors::TEXT(), FontSize::Large, TextAlign::Left);
    y += 28;

    // Divider
    uisys_client::divider_h(w, PAD, y, POPUP_W as u32 - PAD as u32 * 2);
    y += 8;

    // Layout rows with radio buttons
    for i in 0..layout_count {
        let selected = layouts[i].id == current_id;
        let code = anyos_std::kbd::code_str(&layouts[i].code);
        let label_text = anyos_std::kbd::label_str(&layouts[i].label);

        uisys_client::radio(w, PAD, y + 6, selected, "");

        // "DE  de-DE" style: label first, then code
        let mut text_buf = [0u8; 16];
        let tlen = fmt_layout_text(label_text, code, &mut text_buf);
        let text = core::str::from_utf8(&text_buf[..tlen]).unwrap_or("");
        uisys_client::label(w, PAD + 28, y + 8, text, colors::TEXT(), FontSize::Normal, TextAlign::Left);

        y += ROW_H;
    }
}

fn fmt_layout_text<'a>(label: &str, code: &str, buf: &'a mut [u8; 16]) -> usize {
    let mut pos = 0;
    for &b in label.as_bytes() {
        if pos < 16 { buf[pos] = b; pos += 1; }
    }
    // Padding
    while pos < 4 {
        if pos < 16 { buf[pos] = b' '; pos += 1; }
    }
    if pos < 16 { buf[pos] = b' '; pos += 1; }
    if pos < 16 { buf[pos] = b' '; pos += 1; }
    for &b in code.as_bytes() {
        if pos < 16 { buf[pos] = b; pos += 1; }
    }
    pos
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    // Wait for compositor to be ready
    anyos_std::process::sleep(500);

    let client = match TrayClient::init() {
        Some(c) => c,
        None => return,
    };

    // Load saved layout from config file
    load_config();

    // Get layout info
    let mut layouts = [anyos_std::kbd::LayoutInfo {
        id: 0, code: [0; 8], label: [0; 4],
    }; 8];
    let layout_count = anyos_std::kbd::list_layouts(&mut layouts) as usize;
    let mut current_id = anyos_std::kbd::get_layout();

    // Register tray icon
    let mut icon_pixels = [0u32; 256];
    draw_layout_icon(&mut icon_pixels, &layouts[..layout_count], current_id);
    client.set_icon(ICON_ID, &icon_pixels);

    let mut popup: Option<WindowHandle> = None;

    loop {
        while let Some(event) = client.poll_event() {
            match event.event_type {
                EVT_STATUS_ICON_CLICK if event.arg1 == ICON_ID => {
                    if popup.is_some() {
                        if let Some(ref p) = popup {
                            client.destroy_window(p);
                        }
                        popup = None;
                    } else {
                        open_popup(&client, &mut popup, &layouts[..layout_count], layout_count, current_id);
                    }
                }
                EVT_MOUSE_DOWN => {
                    if let Some(ref win) = popup {
                        if event.window_id == win.id {
                            let click_y = event.arg2 as i32;
                            let first_row_y = PAD + 28 + 8;
                            if click_y >= first_row_y {
                                let row = ((click_y - first_row_y) / ROW_H) as usize;
                                if row < layout_count {
                                    let new_id = layouts[row].id;
                                    anyos_std::kbd::set_layout(new_id);
                                    current_id = new_id;
                                    save_config(new_id);

                                    // Update tray icon
                                    draw_layout_icon(&mut icon_pixels, &layouts[..layout_count], new_id);
                                    client.set_icon(ICON_ID, &icon_pixels);

                                    // Redraw popup with updated selection
                                    draw_popup(win, &layouts[..layout_count], layout_count, new_id);
                                    client.present(win);
                                }
                            }
                        }
                    }
                }
                EVT_WINDOW_CLOSE => {
                    if let Some(ref win) = popup {
                        if event.window_id == win.id {
                            client.destroy_window(win);
                            popup = None;
                        }
                    }
                }
                _ => {}
            }
        }
        anyos_std::process::sleep(100);
    }
}

fn open_popup(
    client: &TrayClient,
    popup: &mut Option<WindowHandle>,
    layouts: &[anyos_std::kbd::LayoutInfo],
    layout_count: usize,
    current_id: u32,
) {
    let (sw, _sh) = client.screen_size();
    let h = popup_height(layout_count);
    // Position near top-right (below menubar)
    let x = sw as i32 - POPUP_W as i32 - 8;
    let y = 26;
    let win = match client.create_window(x, y, POPUP_W, h, BORDERLESS) {
        Some(w) => w,
        None => return,
    };

    draw_popup(&win, layouts, layout_count, current_id);
    client.present(&win);
    *popup = Some(win);
}
