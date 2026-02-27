#![no_std]
#![no_main]

anyos_std::entry!(main);

use libcompositor_client::{TrayClient, WindowHandle, EVT_STATUS_ICON_CLICK,
    EVT_MOUSE_DOWN, EVT_WINDOW_CLOSE, EVT_FOCUS_LOST};

use librender_client::Surface;

// ── Constants ────────────────────────────────────────────────────────────────

const ICON_ID: u32 = 1;
const POPUP_W: u32 = 240;
const BORDERLESS: u32 = 0x01;

const PAD: i32 = 16;
const ROW_H: i32 = 32;

const CONF_PATH: &str = "/System/etc/inputmon.conf";

// Theme colors
const COLOR_CARD_BG: u32 = 0xFF2C2C2C;
const COLOR_TEXT: u32 = 0xFFE6E6E6;
const COLOR_TEXT_TITLE: u32 = 0xFFE6E6E6;
const COLOR_DIVIDER: u32 = 0xFF444444;
const COLOR_RADIO_ON: u32 = 0xFF007AFF;
const COLOR_RADIO_OFF: u32 = 0xFF666666;
const COLOR_RADIO_DOT: u32 = 0xFFFFFFFF;

const FONT_SIZE_LARGE: u16 = 16;
const FONT_SIZE_NORMAL: u16 = 13;

// ── Minimal 5x7 bitmap font for tray icon ────────────────────────────────────

static MINI_FONT: [[u8; 7]; 26] = [
    [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
    [0b11110, 0b10001, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110],
    [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
    [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
    [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
    [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
    [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
    [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
    [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
    [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
    [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
    [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
    [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
    [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
    [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
    [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010],
    [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
    [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
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
    for p in pixels.iter_mut() { *p = 0; }
    let label = find_label(layouts, current_id);
    if label.len() >= 1 { draw_mini_char(pixels, 2, 5, label[0], 0xFFE0E0E0); }
    if label.len() >= 2 { draw_mini_char(pixels, 9, 5, label[1], 0xFFE0E0E0); }
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

// ── Config persistence ───────────────────────────────────────────────────────

fn load_config() {
    let fd = anyos_std::fs::open(CONF_PATH, 0);
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
        if line.starts_with('[') { in_keyboard = line == "[keyboard]"; continue; }
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
    let fd = anyos_std::fs::open(CONF_PATH, 1 | 4 | 8);
    if fd == u32::MAX { return; }
    let mut buf = [0u8; 32];
    let header = b"[keyboard]\nlayout=";
    let hlen = header.len();
    buf[..hlen].copy_from_slice(header);
    let mut id_buf = [0u8; 4];
    let vlen = fmt_u32(id, &mut id_buf);
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
    if val == 0 { buf[0] = b'0'; return 1; }
    let mut tmp = [0u8; 4];
    let mut len = 0;
    while val > 0 && len < 4 { tmp[len] = b'0' + (val % 10) as u8; val /= 10; len += 1; }
    for i in 0..len { buf[i] = tmp[len - 1 - i]; }
    len
}

// ── Popup drawing ────────────────────────────────────────────────────────────

fn popup_height(layout_count: usize) -> u32 {
    (PAD + 28 + 8 + (layout_count as i32) * ROW_H + PAD) as u32
}

fn draw_popup(win: &WindowHandle, layouts: &[anyos_std::kbd::LayoutInfo], layout_count: usize, current_id: u32) {
    let h = popup_height(layout_count);
    let buf = win.surface();
    let mut surface = unsafe { Surface::from_raw(buf, POPUP_W, h) };

    // Card background
    surface.fill(0x00000000);
    surface.fill_rounded_rect_aa(0, 0, POPUP_W, h, 8, COLOR_CARD_BG);

    let mut y = PAD;

    // Title
    libfont_client::draw_string_buf(buf, POPUP_W, h, PAD, y + 4, COLOR_TEXT_TITLE, 0, FONT_SIZE_LARGE, "Keyboard");
    y += 28;

    // Divider
    surface.fill_rect(PAD, y, POPUP_W - PAD as u32 * 2, 1, COLOR_DIVIDER);
    y += 8;

    // Layout rows with radio buttons
    for i in 0..layout_count {
        let selected = layouts[i].id == current_id;

        // Radio button
        let radio_cx = PAD + 10;
        let radio_cy = y + 14;
        if selected {
            surface.fill_circle_aa(radio_cx, radio_cy, 7, COLOR_RADIO_ON);
            surface.fill_circle_aa(radio_cx, radio_cy, 3, COLOR_RADIO_DOT);
        } else {
            surface.draw_circle_aa(radio_cx, radio_cy, 7, COLOR_RADIO_OFF);
        }

        // Label text
        let code = anyos_std::kbd::code_str(&layouts[i].code);
        let label_text = anyos_std::kbd::label_str(&layouts[i].label);
        let mut text_buf = [0u8; 16];
        let tlen = fmt_layout_text(label_text, code, &mut text_buf);
        let text = core::str::from_utf8(&text_buf[..tlen]).unwrap_or("");
        libfont_client::draw_string_buf(buf, POPUP_W, h, PAD + 28, y + 8, COLOR_TEXT, 0, FONT_SIZE_NORMAL, text);

        y += ROW_H;
    }
}

fn fmt_layout_text<'a>(label: &str, code: &str, buf: &'a mut [u8; 16]) -> usize {
    let mut pos = 0;
    for &b in label.as_bytes() { if pos < 16 { buf[pos] = b; pos += 1; } }
    while pos < 4 { if pos < 16 { buf[pos] = b' '; pos += 1; } }
    if pos < 16 { buf[pos] = b' '; pos += 1; }
    if pos < 16 { buf[pos] = b' '; pos += 1; }
    for &b in code.as_bytes() { if pos < 16 { buf[pos] = b; pos += 1; } }
    pos
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    anyos_std::process::sleep(500);

    // Initialize libfont
    if !libfont_client::init() {
        anyos_std::println!("inputmon: libfont init failed");
        return;
    }

    let client = match TrayClient::init() {
        Some(c) => c,
        None => return,
    };

    load_config();

    let mut layouts = [anyos_std::kbd::LayoutInfo { id: 0, code: [0; 8], label: [0; 4] }; 8];
    let layout_count = anyos_std::kbd::list_layouts(&mut layouts) as usize;
    let mut current_id = anyos_std::kbd::get_layout();

    let mut icon_pixels = [0u32; 256];
    draw_layout_icon(&mut icon_pixels, &layouts[..layout_count], current_id);
    client.set_icon(ICON_ID, &icon_pixels);

    let mut popup: Option<WindowHandle> = None;

    loop {
        while let Some(event) = client.poll_event() {
            match event.event_type {
                EVT_STATUS_ICON_CLICK if event.arg1 == ICON_ID => {
                    if popup.is_some() {
                        if let Some(ref p) = popup { client.destroy_window(p); }
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

                                    draw_layout_icon(&mut icon_pixels, &layouts[..layout_count], new_id);
                                    client.set_icon(ICON_ID, &icon_pixels);

                                    draw_popup(win, &layouts[..layout_count], layout_count, new_id);
                                    client.present(win);
                                }
                            }
                        }
                    }
                }
                EVT_FOCUS_LOST | EVT_WINDOW_CLOSE => {
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
