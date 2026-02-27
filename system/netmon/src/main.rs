#![no_std]
#![no_main]

anyos_std::entry!(main);

use libcompositor_client::{TrayClient, WindowHandle, EVT_STATUS_ICON_CLICK,
    EVT_MOUSE_DOWN, EVT_MOUSE_UP, EVT_WINDOW_CLOSE};

use librender_client::Surface;

// ── Constants ────────────────────────────────────────────────────────────────

const ICON_ID: u32 = 1;
const POPUP_W: u32 = 280;
const POPUP_H: u32 = 300;
const POPUP_H_NO_DEVICE: u32 = 80;
const BORDERLESS: u32 = 0x01;

const PAD: i32 = 16;
const ROW_H: i32 = 28;
const LABEL_X: i32 = PAD;
const VALUE_X: i32 = 130;

// Theme colors
const COLOR_CARD_BG: u32 = 0xFF2C2C2C;
const COLOR_TEXT: u32 = 0xFFE6E6E6;
const COLOR_TEXT_DIM: u32 = 0xFF999999;
const COLOR_DIVIDER: u32 = 0xFF444444;
const COLOR_GREEN: u32 = 0xFF34C759;
const COLOR_RED: u32 = 0xFFFF3B30;
const COLOR_GRAY: u32 = 0xFF888888;
const COLOR_TOGGLE_ON: u32 = 0xFF34C759;
const COLOR_TOGGLE_OFF: u32 = 0xFF555555;
const COLOR_BTN_BG: u32 = 0xFF007AFF;
const COLOR_BTN_TEXT: u32 = 0xFFFFFFFF;

const FONT_LARGE: u16 = 16;
const FONT_NORMAL: u16 = 13;

// ── Network info ─────────────────────────────────────────────────────────────

struct NetInfo {
    ip: [u8; 4],
    mask: [u8; 4],
    gw: [u8; 4],
    dns: [u8; 4],
    mac: [u8; 6],
    link_up: bool,
    nic_enabled: bool,
    nic_available: bool,
}

fn read_net_info() -> NetInfo {
    let nic_available = anyos_std::net::is_nic_available();
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
        nic_available,
    }
}

// ── Formatting helpers ───────────────────────────────────────────────────────

fn fmt_ip(ip: &[u8; 4], buf: &mut [u8; 16]) -> usize {
    let mut pos = 0;
    for (i, &octet) in ip.iter().enumerate() {
        if i > 0 { buf[pos] = b'.'; pos += 1; }
        pos += fmt_u8(octet, &mut buf[pos..]);
    }
    pos
}

fn fmt_u8(val: u8, buf: &mut [u8]) -> usize {
    if val >= 100 { buf[0] = b'0' + val / 100; buf[1] = b'0' + (val / 10) % 10; buf[2] = b'0' + val % 10; 3 }
    else if val >= 10 { buf[0] = b'0' + val / 10; buf[1] = b'0' + val % 10; 2 }
    else { buf[0] = b'0' + val; 1 }
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

fn ip_str<'a>(ip: &[u8; 4], buf: &'a mut [u8; 16]) -> &'a str {
    let len = fmt_ip(ip, buf);
    core::str::from_utf8(&buf[..len]).unwrap_or("?.?.?.?")
}

fn mac_str<'a>(mac: &[u8; 6], buf: &'a mut [u8; 18]) -> &'a str {
    let len = fmt_mac(mac, buf);
    core::str::from_utf8(&buf[..len]).unwrap_or("??")
}

// ── Icon drawing ─────────────────────────────────────────────────────────────

fn draw_ethernet_icon(pixels: &mut [u32; 256], connected: bool) {
    let color = if connected { COLOR_GREEN } else { COLOR_GRAY };
    for p in pixels.iter_mut() { *p = 0; }
    for y in 3..13 { for x in 4..12 { pixels[y * 16 + x] = color; } }
    for x in [5, 7, 9] { pixels[1 * 16 + x] = color; pixels[2 * 16 + x] = color; }
    for y in 13..15 { pixels[y * 16 + 7] = color; pixels[y * 16 + 8] = color; }
    let inner = if connected { 0xFF1A1A1A } else { 0xFF444444 };
    for y in 5..10 { for x in 6..10 { pixels[y * 16 + x] = inner; } }
}

// ── Interactive widget state ─────────────────────────────────────────────────

struct PopupState {
    toggle_x: i32,
    toggle_y: i32,
    toggle_on: bool,
    btn_x: i32,
    btn_y: i32,
    btn_w: u32,
    btn_h: u32,
    btn_pressed: bool,
}

impl PopupState {
    fn new(nic_enabled: bool) -> Self {
        PopupState {
            toggle_x: POPUP_W as i32 - PAD - 36,
            toggle_y: PAD + 4,
            toggle_on: nic_enabled,
            btn_x: PAD,
            btn_y: 0, // updated during draw
            btn_w: POPUP_W - PAD as u32 * 2,
            btn_h: 30,
            btn_pressed: false,
        }
    }

    fn hit_toggle(&self, mx: i32, my: i32) -> bool {
        mx >= self.toggle_x && mx < self.toggle_x + 36
            && my >= self.toggle_y && my < self.toggle_y + 20
    }

    fn hit_button(&self, mx: i32, my: i32) -> bool {
        mx >= self.btn_x && mx < self.btn_x + self.btn_w as i32
            && my >= self.btn_y && my < self.btn_y + self.btn_h as i32
    }
}

// ── Popup drawing ────────────────────────────────────────────────────────────

fn draw_popup(win: &WindowHandle, info: &NetInfo, state: &mut PopupState) {
    let popup_h = if info.nic_available { POPUP_H } else { POPUP_H_NO_DEVICE };
    let buf = win.surface();
    let mut surface = unsafe { Surface::from_raw(buf, POPUP_W, popup_h) };

    surface.fill(0x00000000);
    surface.fill_rounded_rect_aa(0, 0, POPUP_W, popup_h, 8, COLOR_CARD_BG);

    if !info.nic_available {
        libfont_client::draw_string_buf(buf, POPUP_W, popup_h, LABEL_X, PAD + 4, COLOR_TEXT, 0, FONT_LARGE, "Ethernet");
        surface.fill_rect(PAD, PAD + 30, POPUP_W - PAD as u32 * 2, 1, COLOR_DIVIDER);
        libfont_client::draw_string_buf(buf, POPUP_W, popup_h, LABEL_X, PAD + 38, COLOR_TEXT_DIM, 0, FONT_NORMAL, "No Network Device Found");
        return;
    }

    let mut y = PAD;

    // Title + toggle
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, LABEL_X, y + 4, COLOR_TEXT, 0, FONT_LARGE, "Ethernet");

    // Toggle switch
    let tx = state.toggle_x;
    let ty = state.toggle_y;
    let track_color = if state.toggle_on { COLOR_TOGGLE_ON } else { COLOR_TOGGLE_OFF };
    surface.fill_rounded_rect_aa(tx, ty, 36, 20, 10, track_color);
    let thumb_x = if state.toggle_on { tx + 18 } else { tx + 2 };
    surface.fill_circle_aa(thumb_x + 8, ty + 10, 8, 0xFFFFFFFF);

    y += ROW_H + 4;

    // Divider
    surface.fill_rect(PAD, y, POPUP_W - PAD as u32 * 2, 1, COLOR_DIVIDER);
    y += 8;

    // Status indicator
    let (status_text, dot_color) = if !info.nic_enabled {
        ("Disabled", COLOR_GRAY)
    } else if info.link_up {
        ("Connected", COLOR_GREEN)
    } else {
        ("Disconnected", COLOR_RED)
    };
    surface.fill_circle_aa(LABEL_X + 5, y + 8, 5, dot_color);
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, LABEL_X + 16, y + 4, COLOR_TEXT, 0, FONT_NORMAL, status_text);
    y += ROW_H;

    // Divider
    surface.fill_rect(PAD, y, POPUP_W - PAD as u32 * 2, 1, COLOR_DIVIDER);
    y += 8;

    // Network info rows
    let mut b = [0u8; 16];
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, LABEL_X, y + 2, COLOR_TEXT, 0, FONT_NORMAL, "IP Address");
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, VALUE_X, y + 2, COLOR_TEXT_DIM, 0, FONT_NORMAL, ip_str(&info.ip, &mut b));
    y += ROW_H;

    let mut b = [0u8; 16];
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, LABEL_X, y + 2, COLOR_TEXT, 0, FONT_NORMAL, "Subnet Mask");
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, VALUE_X, y + 2, COLOR_TEXT_DIM, 0, FONT_NORMAL, ip_str(&info.mask, &mut b));
    y += ROW_H;

    let mut b = [0u8; 16];
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, LABEL_X, y + 2, COLOR_TEXT, 0, FONT_NORMAL, "Gateway");
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, VALUE_X, y + 2, COLOR_TEXT_DIM, 0, FONT_NORMAL, ip_str(&info.gw, &mut b));
    y += ROW_H;

    let mut b = [0u8; 16];
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, LABEL_X, y + 2, COLOR_TEXT, 0, FONT_NORMAL, "DNS Server");
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, VALUE_X, y + 2, COLOR_TEXT_DIM, 0, FONT_NORMAL, ip_str(&info.dns, &mut b));
    y += ROW_H;

    let mut b = [0u8; 18];
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, LABEL_X, y + 2, COLOR_TEXT, 0, FONT_NORMAL, "MAC Address");
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, VALUE_X, y + 2, COLOR_TEXT_DIM, 0, FONT_NORMAL, mac_str(&info.mac, &mut b));
    y += ROW_H;

    // Divider
    surface.fill_rect(PAD, y, POPUP_W - PAD as u32 * 2, 1, COLOR_DIVIDER);
    y += 12;

    // DHCP button
    state.btn_y = y;
    let btn_color = if state.btn_pressed { 0xFF005ECB } else { COLOR_BTN_BG };
    surface.fill_rounded_rect_aa(state.btn_x, y, state.btn_w, state.btn_h, 4, btn_color);
    // Center "Run DHCP" text in button
    let (tw, _) = libfont_client::measure(0, FONT_NORMAL, "Run DHCP");
    let text_x = state.btn_x + (state.btn_w as i32 - tw as i32) / 2;
    libfont_client::draw_string_buf(buf, POPUP_W, popup_h, text_x, y + 8, COLOR_BTN_TEXT, 0, FONT_NORMAL, "Run DHCP");
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    anyos_std::process::sleep(500);

    if !libfont_client::init() {
        anyos_std::println!("netmon: libfont init failed");
        return;
    }

    let client = match TrayClient::init() {
        Some(c) => c,
        None => {
            anyos_std::println!("netmon: compositor not available");
            return;
        }
    };

    let mut info = read_net_info();
    let mut icon_pixels = [0u32; 256];
    draw_ethernet_icon(&mut icon_pixels, info.link_up && info.nic_enabled);
    client.set_icon(ICON_ID, &icon_pixels);

    let mut popup: Option<WindowHandle> = None;
    let mut popup_state: Option<PopupState> = None;
    let mut tick_count: u32 = 0;
    let mut prev_link_up = info.link_up;
    let mut prev_nic_enabled = info.nic_enabled;

    loop {
        while let Some(event) = client.poll_event() {
            match event.event_type {
                EVT_STATUS_ICON_CLICK if event.arg1 == ICON_ID => {
                    if popup.is_some() {
                        if let Some(ref p) = popup { client.destroy_window(p); }
                        popup = None;
                        popup_state = None;
                    } else {
                        open_popup(&client, &mut popup, &mut popup_state, &info);
                    }
                }
                EVT_MOUSE_DOWN => {
                    if let Some(ref win) = popup {
                        if event.window_id == win.id {
                            if let Some(ref mut state) = popup_state {
                                let mx = event.arg1 as i32;
                                let my = event.arg2 as i32;

                                if state.hit_toggle(mx, my) {
                                    state.toggle_on = !state.toggle_on;
                                    if state.toggle_on {
                                        anyos_std::net::enable_nic();
                                    } else {
                                        anyos_std::net::disable_nic();
                                    }
                                    anyos_std::process::sleep(50);
                                    info = read_net_info();
                                    prev_link_up = info.link_up;
                                    prev_nic_enabled = info.nic_enabled;
                                    draw_popup(win, &info, state);
                                    client.present(win);
                                    update_icon(&client, &info, &mut icon_pixels);
                                } else if state.hit_button(mx, my) {
                                    state.btn_pressed = true;
                                    draw_popup(win, &info, state);
                                    client.present(win);

                                    let mut dhcp_buf = [0u8; 16];
                                    anyos_std::net::dhcp(&mut dhcp_buf);
                                    info = read_net_info();

                                    state.btn_pressed = false;
                                    draw_popup(win, &info, state);
                                    client.present(win);
                                    update_icon(&client, &info, &mut icon_pixels);
                                }
                            }
                        }
                    }
                }
                EVT_MOUSE_UP => {
                    if let Some(ref win) = popup {
                        if event.window_id == win.id {
                            if let Some(ref mut state) = popup_state {
                                if state.btn_pressed {
                                    state.btn_pressed = false;
                                    draw_popup(win, &info, state);
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
                            popup_state = None;
                        }
                    }
                }
                _ => {}
            }
        }

        // Periodic update
        tick_count += 1;
        if tick_count >= 20 {
            tick_count = 0;
            let new_info = read_net_info();
            let changed = new_info.link_up != info.link_up
                || new_info.nic_enabled != info.nic_enabled
                || new_info.ip != info.ip;
            info = new_info;

            if changed {
                if info.nic_enabled && info.link_up && (!prev_link_up || !prev_nic_enabled) {
                    let mut b = [0u8; 16];
                    let ip = ip_str(&info.ip, &mut b);
                    let mut msg_buf = [0u8; 32];
                    let msg_len = fmt_notif_msg(ip, &mut msg_buf);
                    client.show_notification(
                        "Network Connected",
                        core::str::from_utf8(&msg_buf[..msg_len]).unwrap_or(""),
                        Some(&icon_pixels),
                        3000,
                    );
                } else if prev_link_up && prev_nic_enabled && (!info.link_up || !info.nic_enabled) {
                    client.show_notification(
                        "Network Disconnected",
                        "Ethernet link is down",
                        Some(&icon_pixels),
                        3000,
                    );
                }
                prev_link_up = info.link_up;
                prev_nic_enabled = info.nic_enabled;

                update_icon(&client, &info, &mut icon_pixels);
                if let Some(ref win) = popup {
                    if let Some(ref mut state) = popup_state {
                        draw_popup(win, &info, state);
                        client.present(win);
                    }
                }
            }
        }

        anyos_std::process::sleep(100);
    }
}

fn open_popup(
    client: &TrayClient,
    popup: &mut Option<WindowHandle>,
    popup_state: &mut Option<PopupState>,
    info: &NetInfo,
) {
    let (sw, _sh) = client.screen_size();
    let popup_h = if info.nic_available { POPUP_H } else { POPUP_H_NO_DEVICE };
    let x = sw as i32 - POPUP_W as i32 - 8;
    let y = 26;
    let win = match client.create_window(x, y, POPUP_W, popup_h, BORDERLESS) {
        Some(w) => w,
        None => return,
    };
    let mut state = PopupState::new(info.nic_enabled);
    draw_popup(&win, info, &mut state);
    client.present(&win);
    *popup = Some(win);
    *popup_state = Some(state);
}

fn fmt_notif_msg(ip: &str, buf: &mut [u8; 32]) -> usize {
    let prefix = b"IP: ";
    let plen = prefix.len();
    buf[..plen].copy_from_slice(prefix);
    let copy_len = ip.as_bytes().len().min(buf.len() - plen);
    buf[plen..plen + copy_len].copy_from_slice(&ip.as_bytes()[..copy_len]);
    plen + copy_len
}

fn update_icon(client: &TrayClient, info: &NetInfo, pixels: &mut [u32; 256]) {
    draw_ethernet_icon(pixels, info.link_up && info.nic_enabled);
    client.set_icon(ICON_ID, pixels);
}
