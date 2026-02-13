#![no_std]
#![no_main]

anyos_std::entry!(main);

use libcompositor_client::{CompositorClient, WindowHandle, EVT_STATUS_ICON_CLICK,
    EVT_MOUSE_DOWN, EVT_WINDOW_CLOSE};

// ── Constants ────────────────────────────────────────────────────────────────

const ICON_ID: u32 = 1;
const POPUP_W: u32 = 260;
const POPUP_H: u32 = 240;
const BORDERLESS: u32 = 0x01;

// Colors (dark theme matching macOS style)
const BG: u32 = 0xF0282828;       // semi-transparent dark bg
const TEXT: u32 = 0xFFE0E0E0;     // primary text
const DIM: u32 = 0xFF888888;      // secondary text
const GREEN: u32 = 0xFF34C759;    // connected
const RED: u32 = 0xFFFF3B30;      // disconnected
const ACCENT: u32 = 0xFF007AFF;   // button blue
const BTN_TEXT: u32 = 0xFFFFFFFF; // button text
const SEPARATOR: u32 = 0xFF3A3A3C;

// ── Network config struct ────────────────────────────────────────────────────

struct NetInfo {
    ip: [u8; 4],
    mask: [u8; 4],
    gw: [u8; 4],
    dns: [u8; 4],
    mac: [u8; 6],
    link_up: bool,
    nic_enabled: bool,
}

fn read_net_info() -> NetInfo {
    let mut buf = [0u8; 24];
    anyos_std::net::get_config(&mut buf);
    let nic_enabled = anyos_std::net::is_nic_enabled();

    NetInfo {
        ip: [buf[0], buf[1], buf[2], buf[3]],
        mask: [buf[4], buf[5], buf[6], buf[7]],
        gw: [buf[8], buf[9], buf[10], buf[11]],
        dns: [buf[12], buf[13], buf[14], buf[15]],
        mac: [buf[16], buf[17], buf[18], buf[19], buf[20], buf[21]],
        link_up: buf[22] != 0,
        nic_enabled,
    }
}

// ── Icon drawing (16x16 ARGB) ────────────────────────────────────────────────

fn draw_ethernet_icon(pixels: &mut [u32; 256], connected: bool) {
    let color = if connected { GREEN } else { DIM };

    // Clear to transparent
    for p in pixels.iter_mut() { *p = 0; }

    // Draw a simple Ethernet plug icon:
    // Central rectangle (body)
    for y in 3..13 {
        for x in 4..12 {
            pixels[y * 16 + x] = color;
        }
    }
    // Top prongs (3 lines)
    for x in [5, 7, 9] {
        pixels[1 * 16 + x] = color;
        pixels[2 * 16 + x] = color;
    }
    // Bottom cable
    for y in 13..15 {
        pixels[y * 16 + 7] = color;
        pixels[y * 16 + 8] = color;
    }
    // Inner dark rectangle (connector hole)
    let inner = if connected { 0xFF1A1A1A } else { 0xFF444444 };
    for y in 5..10 {
        for x in 6..10 {
            pixels[y * 16 + x] = inner;
        }
    }
}

// ── Simple text rendering (6x8 bitmap font) ─────────────────────────────────

fn put_char(surface: &mut [u32], stride: u32, x: i32, y: i32, ch: u8, color: u32) {
    let glyph = MINI_FONT.get((ch as usize) * 8..(ch as usize) * 8 + 8);
    let glyph = match glyph {
        Some(g) => g,
        None => return,
    };
    for row in 0..8i32 {
        let bits = glyph[row as usize];
        for col in 0..6i32 {
            if bits & (0x80 >> col) != 0 {
                let px = x + col;
                let py = y + row;
                if px >= 0 && py >= 0 && (px as u32) < stride && (py as u32) < POPUP_H {
                    surface[(py as u32 * stride + px as u32) as usize] = color;
                }
            }
        }
    }
}

fn draw_text(surface: &mut [u32], stride: u32, x: i32, y: i32, text: &str, color: u32) {
    let mut cx = x;
    for &b in text.as_bytes() {
        if b >= 0x20 && b < 0x7F {
            put_char(surface, stride, cx, y, b, color);
            cx += 6;
        }
    }
}

fn draw_ip(surface: &mut [u32], stride: u32, x: i32, y: i32, ip: &[u8; 4], color: u32) {
    let mut buf = [0u8; 16];
    let len = fmt_ip(ip, &mut buf);
    let s = core::str::from_utf8(&buf[..len]).unwrap_or("?.?.?.?");
    draw_text(surface, stride, x, y, s, color);
}

fn fmt_ip(ip: &[u8; 4], buf: &mut [u8; 16]) -> usize {
    let mut pos = 0;
    for (i, &octet) in ip.iter().enumerate() {
        if i > 0 { buf[pos] = b'.'; pos += 1; }
        pos += fmt_u8(octet, &mut buf[pos..]);
    }
    pos
}

fn fmt_u8(val: u8, buf: &mut [u8]) -> usize {
    if val >= 100 {
        buf[0] = b'0' + val / 100;
        buf[1] = b'0' + (val / 10) % 10;
        buf[2] = b'0' + val % 10;
        3
    } else if val >= 10 {
        buf[0] = b'0' + val / 10;
        buf[1] = b'0' + val % 10;
        2
    } else {
        buf[0] = b'0' + val;
        1
    }
}

fn fmt_mac(mac: &[u8; 6], buf: &mut [u8; 18]) -> usize {
    let hex = b"0123456789ABCDEF";
    let mut pos = 0;
    for (i, &b) in mac.iter().enumerate() {
        if i > 0 { buf[pos] = b':'; pos += 1; }
        buf[pos] = hex[(b >> 4) as usize]; pos += 1;
        buf[pos] = hex[(b & 0xF) as usize]; pos += 1;
    }
    pos
}

// ── Rectangle helpers ────────────────────────────────────────────────────────

fn fill_rect(surface: &mut [u32], stride: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    for dy in 0..h as i32 {
        for dx in 0..w as i32 {
            let px = x + dx;
            let py = y + dy;
            if px >= 0 && py >= 0 && (px as u32) < stride && (py as u32) < POPUP_H {
                surface[(py as u32 * stride + px as u32) as usize] = color;
            }
        }
    }
}

fn draw_hline(surface: &mut [u32], stride: u32, x: i32, y: i32, w: u32, color: u32) {
    fill_rect(surface, stride, x, y, w, 1, color);
}

// ── Button hit test ──────────────────────────────────────────────────────────

struct Button {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

impl Button {
    fn hit(&self, mx: i32, my: i32) -> bool {
        mx >= self.x && mx < self.x + self.w as i32
            && my >= self.y && my < self.y + self.h as i32
    }
}

// ── Popup drawing ────────────────────────────────────────────────────────────

const BTN_W: u32 = 100;
const BTN_H: u32 = 24;
const PAD: i32 = 12;
const ROW_H: i32 = 18;

fn draw_popup(surface: &mut [u32], info: &NetInfo) -> (Button, Button) {
    let s = POPUP_W;

    // Background
    fill_rect(surface, s, 0, 0, POPUP_W, POPUP_H, BG);

    // 1px border
    for x in 0..POPUP_W { surface[x as usize] = SEPARATOR; }
    for x in 0..POPUP_W { surface[((POPUP_H - 1) * s + x) as usize] = SEPARATOR; }
    for y in 0..POPUP_H { surface[(y * s) as usize] = SEPARATOR; }
    for y in 0..POPUP_H { surface[(y * s + POPUP_W - 1) as usize] = SEPARATOR; }

    let mut y = PAD;

    // Title
    draw_text(surface, s, PAD, y, "Ethernet", TEXT);
    y += ROW_H + 4;

    // Status
    let (status_text, status_color) = if !info.nic_enabled {
        ("Disabled", RED)
    } else if info.link_up {
        ("Connected", GREEN)
    } else {
        ("Disconnected", RED)
    };
    draw_text(surface, s, PAD, y, "Status:", DIM);
    draw_text(surface, s, PAD + 60, y, status_text, status_color);
    y += ROW_H;

    // Separator
    draw_hline(surface, s, PAD, y, POPUP_W as u32 - 2 * PAD as u32, SEPARATOR);
    y += 8;

    // IP
    draw_text(surface, s, PAD, y, "IP:", DIM);
    draw_ip(surface, s, PAD + 60, y, &info.ip, TEXT);
    y += ROW_H;

    // Mask
    draw_text(surface, s, PAD, y, "Mask:", DIM);
    draw_ip(surface, s, PAD + 60, y, &info.mask, TEXT);
    y += ROW_H;

    // Gateway
    draw_text(surface, s, PAD, y, "Gateway:", DIM);
    draw_ip(surface, s, PAD + 60, y, &info.gw, TEXT);
    y += ROW_H;

    // DNS
    draw_text(surface, s, PAD, y, "DNS:", DIM);
    draw_ip(surface, s, PAD + 60, y, &info.dns, TEXT);
    y += ROW_H;

    // MAC
    draw_text(surface, s, PAD, y, "MAC:", DIM);
    {
        let mut mac_buf = [0u8; 18];
        let len = fmt_mac(&info.mac, &mut mac_buf);
        let mac_str = core::str::from_utf8(&mac_buf[..len]).unwrap_or("??");
        draw_text(surface, s, PAD + 60, y, mac_str, TEXT);
    }
    y += ROW_H;

    // Separator
    draw_hline(surface, s, PAD, y, POPUP_W as u32 - 2 * PAD as u32, SEPARATOR);
    y += 10;

    // Buttons
    let btn_dhcp = Button { x: PAD, y, w: BTN_W, h: BTN_H };
    fill_rect(surface, s, btn_dhcp.x, btn_dhcp.y, BTN_W, BTN_H, ACCENT);
    draw_text(surface, s, btn_dhcp.x + 18, btn_dhcp.y + 8, "Run DHCP", BTN_TEXT);

    let toggle_label = if info.nic_enabled { "Disable" } else { "Enable" };
    let toggle_color = if info.nic_enabled { RED } else { GREEN };
    let btn_toggle = Button {
        x: PAD + BTN_W as i32 + 12,
        y,
        w: BTN_W,
        h: BTN_H,
    };
    fill_rect(surface, s, btn_toggle.x, btn_toggle.y, BTN_W, BTN_H, toggle_color);
    let label_x = btn_toggle.x + (BTN_W as i32 - toggle_label.len() as i32 * 6) / 2;
    draw_text(surface, s, label_x, btn_toggle.y + 8, toggle_label, BTN_TEXT);

    (btn_dhcp, btn_toggle)
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    // Wait for compositor to be ready
    anyos_std::process::sleep(500);

    let client = match CompositorClient::init() {
        Some(c) => c,
        None => {
            anyos_std::println!("netmon: compositor not available");
            return;
        }
    };

    // Create a small anchor window (1x1, borderless, off-screen) for event delivery
    let anchor = match client.create_window(1, 1, BORDERLESS) {
        Some(w) => w,
        None => {
            anyos_std::println!("netmon: failed to create anchor window");
            return;
        }
    };
    client.move_window(&anchor, -10, -10);

    // Read initial network info and set status icon
    let mut info = read_net_info();
    let mut icon_pixels = [0u32; 256];
    draw_ethernet_icon(&mut icon_pixels, info.link_up && info.nic_enabled);
    client.add_status_icon(ICON_ID, &icon_pixels);

    let mut popup: Option<WindowHandle> = None;
    let mut btn_dhcp = Button { x: 0, y: 0, w: 0, h: 0 };
    let mut btn_toggle = Button { x: 0, y: 0, w: 0, h: 0 };
    let mut tick_count: u32 = 0;

    loop {
        // Poll events on anchor window
        while let Some(event) = client.poll_event(&anchor) {
            if event.event_type == EVT_STATUS_ICON_CLICK && event.arg1 == ICON_ID {
                // Toggle popup
                if popup.is_some() {
                    if let Some(ref p) = popup {
                        client.destroy_window(p);
                    }
                    popup = None;
                } else {
                    open_popup(&client, &mut popup, &info, &mut btn_dhcp, &mut btn_toggle);
                }
            }
        }

        // Poll events on popup window
        if let Some(ref win) = popup {
            while let Some(event) = client.poll_event(win) {
                match event.event_type {
                    EVT_MOUSE_DOWN => {
                        let mx = event.arg1 as i32;
                        let my = event.arg2 as i32;

                        if btn_dhcp.hit(mx, my) {
                            // Run DHCP
                            let mut dhcp_buf = [0u8; 16];
                            anyos_std::net::dhcp(&mut dhcp_buf);
                            info = read_net_info();
                            redraw_popup(&client, win, &info, &mut btn_dhcp, &mut btn_toggle);
                            update_icon(&client, &info, &mut icon_pixels);
                        } else if btn_toggle.hit(mx, my) {
                            // Toggle NIC
                            if info.nic_enabled {
                                anyos_std::net::disable_nic();
                            } else {
                                anyos_std::net::enable_nic();
                            }
                            anyos_std::process::sleep(50);
                            info = read_net_info();
                            redraw_popup(&client, win, &info, &mut btn_dhcp, &mut btn_toggle);
                            update_icon(&client, &info, &mut icon_pixels);
                        }
                    }
                    EVT_WINDOW_CLOSE => {
                        client.destroy_window(win);
                        popup = None;
                        break;
                    }
                    _ => {}
                }
            }
        }

        // Periodic update (every ~2 seconds = 20 iterations of 100ms sleep)
        tick_count += 1;
        if tick_count >= 20 {
            tick_count = 0;
            let new_info = read_net_info();
            let changed = new_info.link_up != info.link_up
                || new_info.nic_enabled != info.nic_enabled
                || new_info.ip != info.ip;
            info = new_info;

            if changed {
                update_icon(&client, &info, &mut icon_pixels);
                if let Some(ref win) = popup {
                    redraw_popup(&client, win, &info, &mut btn_dhcp, &mut btn_toggle);
                }
            }
        }

        anyos_std::process::sleep(100);
    }
}

fn open_popup(
    client: &CompositorClient,
    popup: &mut Option<WindowHandle>,
    info: &NetInfo,
    btn_dhcp: &mut Button,
    btn_toggle: &mut Button,
) {
    let (sw, sh) = client.screen_size();
    let win = match client.create_window(POPUP_W, POPUP_H, BORDERLESS) {
        Some(w) => w,
        None => return,
    };

    // Position near top-right (near menubar)
    let x = sw as i32 - POPUP_W as i32 - 8;
    let y = 26; // below menubar
    client.move_window(&win, x, y);

    redraw_popup(client, &win, info, btn_dhcp, btn_toggle);
    *popup = Some(win);
}

fn redraw_popup(
    client: &CompositorClient,
    win: &WindowHandle,
    info: &NetInfo,
    btn_dhcp: &mut Button,
    btn_toggle: &mut Button,
) {
    let surface = unsafe { win.surface_slice() };
    let (d, t) = draw_popup(surface, info);
    *btn_dhcp = d;
    *btn_toggle = t;
    client.present(win);
}

fn update_icon(client: &CompositorClient, info: &NetInfo, pixels: &mut [u32; 256]) {
    draw_ethernet_icon(pixels, info.link_up && info.nic_enabled);
    client.add_status_icon(ICON_ID, pixels);
}

// ── Minimal 6x8 bitmap font (ASCII 0x20–0x7E) ──────────────────────────────
// Each character is 8 bytes (8 rows), 6 columns packed left-aligned in the byte.

#[rustfmt::skip]
static MINI_FONT: &[u8] = &[
    // 0x00-0x1F: unused (32 * 8 = 256 bytes of zeros)
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,
    // 0x20 ' '
    0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
    // 0x21 '!'
    0x20,0x20,0x20,0x20,0x20,0x00,0x20,0x00,
    // 0x22 '"'
    0x50,0x50,0x50,0x00,0x00,0x00,0x00,0x00,
    // 0x23 '#'
    0x50,0x50,0xF8,0x50,0xF8,0x50,0x50,0x00,
    // 0x24 '$'
    0x20,0x78,0xA0,0x70,0x28,0xF0,0x20,0x00,
    // 0x25 '%'
    0xC0,0xC8,0x10,0x20,0x40,0x98,0x18,0x00,
    // 0x26 '&'
    0x60,0x90,0xA0,0x40,0xA8,0x90,0x68,0x00,
    // 0x27 '''
    0x30,0x30,0x20,0x00,0x00,0x00,0x00,0x00,
    // 0x28 '('
    0x10,0x20,0x40,0x40,0x40,0x20,0x10,0x00,
    // 0x29 ')'
    0x40,0x20,0x10,0x10,0x10,0x20,0x40,0x00,
    // 0x2A '*'
    0x00,0x50,0x20,0xF8,0x20,0x50,0x00,0x00,
    // 0x2B '+'
    0x00,0x20,0x20,0xF8,0x20,0x20,0x00,0x00,
    // 0x2C ','
    0x00,0x00,0x00,0x00,0x30,0x30,0x20,0x40,
    // 0x2D '-'
    0x00,0x00,0x00,0xF8,0x00,0x00,0x00,0x00,
    // 0x2E '.'
    0x00,0x00,0x00,0x00,0x00,0x30,0x30,0x00,
    // 0x2F '/'
    0x00,0x08,0x10,0x20,0x40,0x80,0x00,0x00,
    // 0x30 '0'
    0x70,0x88,0x98,0xA8,0xC8,0x88,0x70,0x00,
    // 0x31 '1'
    0x20,0x60,0x20,0x20,0x20,0x20,0x70,0x00,
    // 0x32 '2'
    0x70,0x88,0x08,0x10,0x20,0x40,0xF8,0x00,
    // 0x33 '3'
    0xF8,0x10,0x20,0x10,0x08,0x88,0x70,0x00,
    // 0x34 '4'
    0x10,0x30,0x50,0x90,0xF8,0x10,0x10,0x00,
    // 0x35 '5'
    0xF8,0x80,0xF0,0x08,0x08,0x88,0x70,0x00,
    // 0x36 '6'
    0x30,0x40,0x80,0xF0,0x88,0x88,0x70,0x00,
    // 0x37 '7'
    0xF8,0x08,0x10,0x20,0x40,0x40,0x40,0x00,
    // 0x38 '8'
    0x70,0x88,0x88,0x70,0x88,0x88,0x70,0x00,
    // 0x39 '9'
    0x70,0x88,0x88,0x78,0x08,0x10,0x60,0x00,
    // 0x3A ':'
    0x00,0x30,0x30,0x00,0x30,0x30,0x00,0x00,
    // 0x3B ';'
    0x00,0x30,0x30,0x00,0x30,0x30,0x20,0x40,
    // 0x3C '<'
    0x08,0x10,0x20,0x40,0x20,0x10,0x08,0x00,
    // 0x3D '='
    0x00,0x00,0xF8,0x00,0xF8,0x00,0x00,0x00,
    // 0x3E '>'
    0x80,0x40,0x20,0x10,0x20,0x40,0x80,0x00,
    // 0x3F '?'
    0x70,0x88,0x08,0x10,0x20,0x00,0x20,0x00,
    // 0x40 '@'
    0x70,0x88,0xB8,0xA8,0xB8,0x80,0x70,0x00,
    // 0x41 'A'
    0x70,0x88,0x88,0xF8,0x88,0x88,0x88,0x00,
    // 0x42 'B'
    0xF0,0x88,0x88,0xF0,0x88,0x88,0xF0,0x00,
    // 0x43 'C'
    0x70,0x88,0x80,0x80,0x80,0x88,0x70,0x00,
    // 0x44 'D'
    0xE0,0x90,0x88,0x88,0x88,0x90,0xE0,0x00,
    // 0x45 'E'
    0xF8,0x80,0x80,0xF0,0x80,0x80,0xF8,0x00,
    // 0x46 'F'
    0xF8,0x80,0x80,0xF0,0x80,0x80,0x80,0x00,
    // 0x47 'G'
    0x70,0x88,0x80,0xB8,0x88,0x88,0x78,0x00,
    // 0x48 'H'
    0x88,0x88,0x88,0xF8,0x88,0x88,0x88,0x00,
    // 0x49 'I'
    0x70,0x20,0x20,0x20,0x20,0x20,0x70,0x00,
    // 0x4A 'J'
    0x38,0x10,0x10,0x10,0x10,0x90,0x60,0x00,
    // 0x4B 'K'
    0x88,0x90,0xA0,0xC0,0xA0,0x90,0x88,0x00,
    // 0x4C 'L'
    0x80,0x80,0x80,0x80,0x80,0x80,0xF8,0x00,
    // 0x4D 'M'
    0x88,0xD8,0xA8,0x88,0x88,0x88,0x88,0x00,
    // 0x4E 'N'
    0x88,0xC8,0xA8,0x98,0x88,0x88,0x88,0x00,
    // 0x4F 'O'
    0x70,0x88,0x88,0x88,0x88,0x88,0x70,0x00,
    // 0x50 'P'
    0xF0,0x88,0x88,0xF0,0x80,0x80,0x80,0x00,
    // 0x51 'Q'
    0x70,0x88,0x88,0x88,0xA8,0x90,0x68,0x00,
    // 0x52 'R'
    0xF0,0x88,0x88,0xF0,0xA0,0x90,0x88,0x00,
    // 0x53 'S'
    0x78,0x80,0x80,0x70,0x08,0x08,0xF0,0x00,
    // 0x54 'T'
    0xF8,0x20,0x20,0x20,0x20,0x20,0x20,0x00,
    // 0x55 'U'
    0x88,0x88,0x88,0x88,0x88,0x88,0x70,0x00,
    // 0x56 'V'
    0x88,0x88,0x88,0x88,0x88,0x50,0x20,0x00,
    // 0x57 'W'
    0x88,0x88,0x88,0xA8,0xA8,0xD8,0x88,0x00,
    // 0x58 'X'
    0x88,0x88,0x50,0x20,0x50,0x88,0x88,0x00,
    // 0x59 'Y'
    0x88,0x88,0x50,0x20,0x20,0x20,0x20,0x00,
    // 0x5A 'Z'
    0xF8,0x08,0x10,0x20,0x40,0x80,0xF8,0x00,
    // 0x5B '['
    0x70,0x40,0x40,0x40,0x40,0x40,0x70,0x00,
    // 0x5C '\'
    0x00,0x80,0x40,0x20,0x10,0x08,0x00,0x00,
    // 0x5D ']'
    0x70,0x10,0x10,0x10,0x10,0x10,0x70,0x00,
    // 0x5E '^'
    0x20,0x50,0x88,0x00,0x00,0x00,0x00,0x00,
    // 0x5F '_'
    0x00,0x00,0x00,0x00,0x00,0x00,0xF8,0x00,
    // 0x60 '`'
    0x40,0x20,0x10,0x00,0x00,0x00,0x00,0x00,
    // 0x61 'a'
    0x00,0x00,0x70,0x08,0x78,0x88,0x78,0x00,
    // 0x62 'b'
    0x80,0x80,0xB0,0xC8,0x88,0xC8,0xB0,0x00,
    // 0x63 'c'
    0x00,0x00,0x70,0x80,0x80,0x88,0x70,0x00,
    // 0x64 'd'
    0x08,0x08,0x68,0x98,0x88,0x98,0x68,0x00,
    // 0x65 'e'
    0x00,0x00,0x70,0x88,0xF8,0x80,0x70,0x00,
    // 0x66 'f'
    0x30,0x48,0x40,0xE0,0x40,0x40,0x40,0x00,
    // 0x67 'g'
    0x00,0x00,0x68,0x98,0x98,0x68,0x08,0x70,
    // 0x68 'h'
    0x80,0x80,0xB0,0xC8,0x88,0x88,0x88,0x00,
    // 0x69 'i'
    0x20,0x00,0x60,0x20,0x20,0x20,0x70,0x00,
    // 0x6A 'j'
    0x10,0x00,0x30,0x10,0x10,0x10,0x90,0x60,
    // 0x6B 'k'
    0x80,0x80,0x90,0xA0,0xC0,0xA0,0x90,0x00,
    // 0x6C 'l'
    0x60,0x20,0x20,0x20,0x20,0x20,0x70,0x00,
    // 0x6D 'm'
    0x00,0x00,0xD0,0xA8,0xA8,0x88,0x88,0x00,
    // 0x6E 'n'
    0x00,0x00,0xB0,0xC8,0x88,0x88,0x88,0x00,
    // 0x6F 'o'
    0x00,0x00,0x70,0x88,0x88,0x88,0x70,0x00,
    // 0x70 'p'
    0x00,0x00,0xB0,0xC8,0xC8,0xB0,0x80,0x80,
    // 0x71 'q'
    0x00,0x00,0x68,0x98,0x98,0x68,0x08,0x08,
    // 0x72 'r'
    0x00,0x00,0xB0,0xC8,0x80,0x80,0x80,0x00,
    // 0x73 's'
    0x00,0x00,0x78,0x80,0x70,0x08,0xF0,0x00,
    // 0x74 't'
    0x40,0x40,0xE0,0x40,0x40,0x48,0x30,0x00,
    // 0x75 'u'
    0x00,0x00,0x88,0x88,0x88,0x98,0x68,0x00,
    // 0x76 'v'
    0x00,0x00,0x88,0x88,0x88,0x50,0x20,0x00,
    // 0x77 'w'
    0x00,0x00,0x88,0x88,0xA8,0xA8,0x50,0x00,
    // 0x78 'x'
    0x00,0x00,0x88,0x50,0x20,0x50,0x88,0x00,
    // 0x79 'y'
    0x00,0x00,0x88,0x88,0x98,0x68,0x08,0x70,
    // 0x7A 'z'
    0x00,0x00,0xF8,0x10,0x20,0x40,0xF8,0x00,
    // 0x7B '{'
    0x18,0x20,0x20,0xC0,0x20,0x20,0x18,0x00,
    // 0x7C '|'
    0x20,0x20,0x20,0x20,0x20,0x20,0x20,0x00,
    // 0x7D '}'
    0xC0,0x20,0x20,0x18,0x20,0x20,0xC0,0x00,
    // 0x7E '~'
    0x00,0x00,0x40,0xA8,0x10,0x00,0x00,0x00,
];
