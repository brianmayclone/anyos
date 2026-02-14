#![no_std]
#![no_main]

anyos_std::entry!(main);

use libcompositor_client::{TrayClient, WindowHandle, EVT_STATUS_ICON_CLICK,
    EVT_MOUSE_DOWN, EVT_MOUSE_UP, EVT_WINDOW_CLOSE};

use uisys_client::{self, ButtonStyle, ButtonState, FontSize, TextAlign, StatusKind, UiToggle, UiButton, UiEvent};
use uisys_client::colors;

// ── Constants ────────────────────────────────────────────────────────────────

const ICON_ID: u32 = 1;
const POPUP_W: u32 = 280;
const POPUP_H: u32 = 300;
const POPUP_H_NO_DEVICE: u32 = 80;
const BORDERLESS: u32 = 0x01;

// Layout
const PAD: i32 = 16;
const ROW_H: i32 = 28;
const LABEL_X: i32 = PAD;
const VALUE_X: i32 = 130;

// ── WinSurface for uisys DLL surface-mode rendering ──────────────────────────

/// Layout-compatible with uisys DLL's draw::WinSurface.
/// When uisys receives the pointer to this struct as `win` (>= 0x0100_0000),
/// it draws directly into the pixel buffer via librender.
#[repr(C)]
struct WinSurface {
    pixels: *mut u32,
    width: u32,
    height: u32,
}

// ── Network config struct ────────────────────────────────────────────────────

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

fn ip_str<'a>(ip: &[u8; 4], buf: &'a mut [u8; 16]) -> &'a str {
    let len = fmt_ip(ip, buf);
    core::str::from_utf8(&buf[..len]).unwrap_or("?.?.?.?")
}

fn mac_str<'a>(mac: &[u8; 6], buf: &'a mut [u8; 18]) -> &'a str {
    let len = fmt_mac(mac, buf);
    core::str::from_utf8(&buf[..len]).unwrap_or("??")
}

// ── Icon drawing (16x16 ARGB) ────────────────────────────────────────────────

const GREEN: u32 = 0xFF34C759;
const DIM: u32 = 0xFF888888;

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

// ── Popup state ──────────────────────────────────────────────────────────────

struct PopupState {
    toggle: UiToggle,
    btn_dhcp: UiButton,
}

impl PopupState {
    fn new(nic_enabled: bool) -> Self {
        PopupState {
            toggle: UiToggle::new(POPUP_W as i32 - PAD - 36, PAD + 4, nic_enabled),
            btn_dhcp: UiButton::new(PAD, 0, POPUP_W as u32 - PAD as u32 * 2, 30, ButtonStyle::Primary),
        }
    }
}

// ── Popup drawing with uisys components ──────────────────────────────────────

fn draw_popup(win: &WindowHandle, info: &NetInfo, state: &mut PopupState) {
    let popup_h = if info.nic_available { POPUP_H } else { POPUP_H_NO_DEVICE };
    let surface = WinSurface {
        pixels: win.surface(),
        width: POPUP_W,
        height: popup_h,
    };
    let w = &surface as *const WinSurface as u32;

    // Background card (fills entire popup)
    uisys_client::card(w, 0, 0, POPUP_W, popup_h);

    if !info.nic_available {
        uisys_client::label(w, LABEL_X, PAD + 4, "Ethernet", colors::TEXT(), FontSize::Large, TextAlign::Left);
        uisys_client::divider_h(w, PAD, PAD + 30, POPUP_W as u32 - PAD as u32 * 2);
        uisys_client::label(
            w, LABEL_X, PAD + 38,
            "No Network Device Found",
            colors::TEXT_SECONDARY(),
            FontSize::Normal,
            TextAlign::Left,
        );
        return;
    }

    let mut y = PAD;

    // Title: "Ethernet" + toggle switch
    uisys_client::label(w, LABEL_X, y + 4, "Ethernet", colors::TEXT(), FontSize::Large, TextAlign::Left);
    state.toggle.render(w);
    y += ROW_H + 4;

    // Divider
    uisys_client::divider_h(w, PAD, y, POPUP_W as u32 - PAD as u32 * 2);
    y += 8;

    // Status indicator
    let (status_text, status_kind) = if !info.nic_enabled {
        ("Disabled", StatusKind::Offline)
    } else if info.link_up {
        ("Connected", StatusKind::Online)
    } else {
        ("Disconnected", StatusKind::Error)
    };
    uisys_client::status_indicator(w, LABEL_X, y + 4, status_kind, status_text);
    y += ROW_H;

    // Divider
    uisys_client::divider_h(w, PAD, y, POPUP_W as u32 - PAD as u32 * 2);
    y += 8;

    // Network info rows
    let mut b = [0u8; 16];
    uisys_client::label(w, LABEL_X, y + 2, "IP Address", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    uisys_client::label(w, VALUE_X, y + 2, ip_str(&info.ip, &mut b), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
    y += ROW_H;

    uisys_client::label(w, LABEL_X, y + 2, "Subnet Mask", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 16];
    uisys_client::label(w, VALUE_X, y + 2, ip_str(&info.mask, &mut b), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
    y += ROW_H;

    uisys_client::label(w, LABEL_X, y + 2, "Gateway", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 16];
    uisys_client::label(w, VALUE_X, y + 2, ip_str(&info.gw, &mut b), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
    y += ROW_H;

    uisys_client::label(w, LABEL_X, y + 2, "DNS Server", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 16];
    uisys_client::label(w, VALUE_X, y + 2, ip_str(&info.dns, &mut b), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
    y += ROW_H;

    uisys_client::label(w, LABEL_X, y + 2, "MAC Address", colors::TEXT(), FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 18];
    uisys_client::label(w, VALUE_X, y + 2, mac_str(&info.mac, &mut b), colors::TEXT_SECONDARY(), FontSize::Normal, TextAlign::Left);
    y += ROW_H;

    // Divider
    uisys_client::divider_h(w, PAD, y, POPUP_W as u32 - PAD as u32 * 2);
    y += 12;

    // Run DHCP button (update y position for hit testing)
    state.btn_dhcp.y = y;
    state.btn_dhcp.render(w, "Run DHCP");
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    // Wait for compositor to be ready
    anyos_std::process::sleep(500);

    let client = match TrayClient::init() {
        Some(c) => c,
        None => {
            anyos_std::println!("netmon: compositor not available");
            return;
        }
    };

    // Register tray icon — no window needed
    let mut info = read_net_info();
    let mut icon_pixels = [0u32; 256];
    draw_ethernet_icon(&mut icon_pixels, info.link_up && info.nic_enabled);
    client.set_icon(ICON_ID, &icon_pixels);

    let mut popup: Option<WindowHandle> = None;
    let mut popup_state: Option<PopupState> = None;
    let mut tick_count: u32 = 0;

    loop {
        // Single poll_event — handles ALL events (tray clicks + window events)
        while let Some(event) = client.poll_event() {
            match event.event_type {
                EVT_STATUS_ICON_CLICK if event.arg1 == ICON_ID => {
                    // Toggle popup
                    if popup.is_some() {
                        if let Some(ref p) = popup {
                            client.destroy_window(p);
                        }
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
                                let ui_evt = UiEvent {
                                    event_type: uisys_client::EVENT_MOUSE_DOWN,
                                    p1: event.arg1,
                                    p2: event.arg2,
                                    p3: 0,
                                    p4: 0,
                                };

                                // Handle toggle
                                if let Some(new_state) = state.toggle.handle_event(&ui_evt) {
                                    if new_state {
                                        anyos_std::net::enable_nic();
                                    } else {
                                        anyos_std::net::disable_nic();
                                    }
                                    anyos_std::process::sleep(50);
                                    info = read_net_info();
                                    draw_popup(win, &info, state);
                                    client.present(win);
                                    update_icon(&client, &info, &mut icon_pixels);
                                } else if state.btn_dhcp.handle_event(&ui_evt) {
                                    // Handle DHCP button
                                    let mut dhcp_buf = [0u8; 16];
                                    anyos_std::net::dhcp(&mut dhcp_buf);
                                    info = read_net_info();
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
                                let ui_evt = UiEvent {
                                    event_type: uisys_client::EVENT_MOUSE_UP,
                                    p1: event.arg1,
                                    p2: event.arg2,
                                    p3: 0,
                                    p4: 0,
                                };
                                state.btn_dhcp.handle_event(&ui_evt);
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
    let win = match client.create_window(POPUP_W, popup_h, BORDERLESS) {
        Some(w) => w,
        None => return,
    };

    // Position near top-right (near menubar)
    let x = sw as i32 - POPUP_W as i32 - 8;
    let y = 26; // below menubar
    client.move_window(&win, x, y);

    let mut state = PopupState::new(info.nic_enabled);
    draw_popup(&win, info, &mut state);
    client.present(&win);
    *popup = Some(win);
    *popup_state = Some(state);
}

fn update_icon(client: &TrayClient, info: &NetInfo, pixels: &mut [u32; 256]) {
    draw_ethernet_icon(pixels, info.link_up && info.nic_enabled);
    client.set_icon(ICON_ID, pixels);
}
