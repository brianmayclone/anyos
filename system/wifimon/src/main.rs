#![no_std]
#![no_main]

//! WiFi tray icon — macOS-style WiFi menu in the menu bar.
//!
//! Shows a WiFi signal icon in the system tray. Clicking opens a popup with:
//! - WiFi enable/disable toggle
//! - Separator
//! - List of nearby networks (sorted by signal strength)
//!   - Connected network: blue dot + checkmark
//!   - Known networks: stored password, auto-connect
//!   - Unknown: clicking prompts for password
//! - "Network Settings..." at the bottom
//!
//! Only shows the tray icon when a WiFi adapter is detected.

anyos_std::entry!(main);

use anyos_std::{String, Vec, format};
use anyos_std::net::{self, BssInfo, WifiState, WifiSecurity};
use anyos_std::process;
use libcompositor_client::{TrayClient, WindowHandle,
    EVT_STATUS_ICON_CLICK, EVT_MOUSE_DOWN, EVT_WINDOW_CLOSE, EVT_FOCUS_LOST};
use librender_client::Surface;

// ── Constants ───────────────────────────────────────────────────────────────

const ICON_ID: u32 = 1;
const POPUP_W: u32 = 280;
const MAX_NETWORKS: usize = 12;
const ROW_H: u32 = 28;
const HEADER_H: u32 = 36;
const SEP_H: u32 = 9;
const FOOTER_H: u32 = 32;
const BORDERLESS: u32 = 0x01;
const PAD: i32 = 14;

// Theme colors
const BG: u32 = 0xFF2C2C2C;
const TEXT: u32 = 0xFFE6E6E6;
const TEXT_DIM: u32 = 0xFF999999;
const DIVIDER: u32 = 0xFF444444;
const HOVER_BG: u32 = 0xFF3A3A3C;
const BLUE_DOT: u32 = 0xFF007AFF;
const GREEN_CHECK: u32 = 0xFF34C759;
const LOCK_COLOR: u32 = 0xFFAAAAAA;

const FONT_NORMAL: u16 = 13;
const FONT_SMALL: u16 = 11;

// ── State ───────────────────────────────────────────────────────────────────

struct WifiMonState {
    enabled: bool,
    networks: Vec<BssInfo>,
    connected_ssid: Vec<u8>,
    connected: bool,
    /// Known network SSIDs with stored passwords
    known_networks: Vec<(Vec<u8>, Vec<u8>)>, // (ssid, password)
}

impl WifiMonState {
    fn new() -> Self {
        WifiMonState {
            enabled: true,
            networks: Vec::new(),
            connected_ssid: Vec::new(),
            connected: false,
            known_networks: Vec::new(),
        }
    }

    fn refresh(&mut self) {
        let status = net::wifi_status();
        self.connected = status.connected;
        if status.connected {
            self.connected_ssid = status.ssid[..status.ssid_len].to_vec();
        } else {
            self.connected_ssid.clear();
        }
        self.networks = net::wifi_scan_results(MAX_NETWORKS);
        // Sort by signal strength (strongest first)
        self.networks.sort_unstable_by(|a, b| b.rssi.cmp(&a.rssi));
    }

    fn is_known(&self, ssid: &[u8]) -> bool {
        self.known_networks.iter().any(|(s, _)| s.as_slice() == ssid)
    }

    fn get_password(&self, ssid: &[u8]) -> Option<&[u8]> {
        self.known_networks.iter()
            .find(|(s, _)| s.as_slice() == ssid)
            .map(|(_, p)| p.as_slice())
    }

    fn store_network(&mut self, ssid: &[u8], password: &[u8]) {
        // Remove old entry if exists
        self.known_networks.retain(|(s, _)| s.as_slice() != ssid);
        self.known_networks.push((ssid.to_vec(), password.to_vec()));
        // Persist to disk
        save_known_networks(&self.known_networks);
    }

    fn popup_height(&self) -> u32 {
        let net_count = self.networks.len().min(MAX_NETWORKS) as u32;
        HEADER_H + SEP_H + net_count * ROW_H + SEP_H + FOOTER_H
    }
}

// ── Known networks persistence ──────────────────────────────────────────────

const WIFI_CONF_PATH: &str = "/System/etc/wifi/known_networks.conf";

fn load_known_networks() -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut result = Vec::new();
    let fd = anyos_std::fs::open(WIFI_CONF_PATH, 0);
    if fd == u32::MAX { return result; }
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);

    // Format: SSID\tPASSWORD\n
    for line in data.split(|&b| b == b'\n') {
        if line.is_empty() || line[0] == b'#' { continue; }
        if let Some(tab) = line.iter().position(|&b| b == b'\t') {
            let ssid = &line[..tab];
            let pass = &line[tab + 1..];
            if !ssid.is_empty() {
                result.push((ssid.to_vec(), pass.to_vec()));
            }
        }
    }
    result
}

fn save_known_networks(networks: &[(Vec<u8>, Vec<u8>)]) {
    let mut data = Vec::new();
    data.extend_from_slice(b"# WiFi known networks\n");
    for (ssid, pass) in networks {
        data.extend_from_slice(ssid);
        data.push(b'\t');
        data.extend_from_slice(pass);
        data.push(b'\n');
    }
    let _ = anyos_std::fs::write_bytes(WIFI_CONF_PATH, &data);
}

// ── WiFi signal icon (16x16) ────────────────────────────────────────────────

fn draw_wifi_icon(pixels: &mut [u32; 256], connected: bool, signal: i8) {
    for p in pixels.iter_mut() { *p = 0; }
    let color = if connected { 0xFFE6E6E6 } else { 0xFF888888 };
    let bars = if signal > -50 { 4 } else if signal > -65 { 3 }
              else if signal > -75 { 2 } else if signal > -85 { 1 } else { 0 };

    // Draw signal arcs (simplified as horizontal bars of increasing width)
    // Bottom to top: wider = closer to center
    let bar_data: [(i32, i32, u32); 4] = [
        (7, 13, 2),  // bottom: narrow dot
        (5, 10, 6),  // 2nd arc
        (3, 7, 10),  // 3rd arc
        (1, 4, 14),  // top arc: widest
    ];

    for (i, &(x_off, y, w)) in bar_data.iter().enumerate() {
        let c = if (i as i8) < (4 - bars) as i8 { 0xFF555555 } else { color };
        for dx in 0..w {
            let px = x_off + dx as i32;
            if px >= 0 && px < 16 && y >= 0 && y < 16 {
                pixels[y as usize * 16 + px as usize] = c;
                if y + 1 < 16 {
                    pixels[(y + 1) as usize * 16 + px as usize] = c;
                }
            }
        }
    }
}

// ── Popup drawing ───────────────────────────────────────────────────────────

fn draw_popup(win: &WindowHandle, state: &WifiMonState) {
    let h = state.popup_height();
    let buf = win.surface();
    let mut surface = unsafe { Surface::from_raw(buf, POPUP_W, h) };

    surface.fill(0x00000000);
    surface.fill_rounded_rect_aa(0, 0, POPUP_W, h, 8, BG);

    let mut y = 0i32;

    // ── Header: "Wi-Fi" + enable/disable ────────────────────────────────
    libfont_client::draw_string_buf(buf, POPUP_W, h, PAD, y + 10, TEXT, 0, FONT_NORMAL, "Wi-Fi");
    let toggle_text = if state.enabled { "On" } else { "Off" };
    let (tw, _) = libfont_client::measure(0, FONT_NORMAL, toggle_text);
    let toggle_color = if state.enabled { GREEN_CHECK } else { TEXT_DIM };
    libfont_client::draw_string_buf(buf, POPUP_W, h,
        POPUP_W as i32 - PAD - tw as i32, y + 10, toggle_color, 0, FONT_NORMAL, toggle_text);
    y += HEADER_H as i32;

    // Separator
    surface.fill_rect(PAD, y + 4, POPUP_W - PAD as u32 * 2, 1, DIVIDER);
    y += SEP_H as i32;

    // ── Network list ────────────────────────────────────────────────────
    for net_info in &state.networks {
        let ssid_bytes = &net_info.ssid[..net_info.ssid_len];
        let ssid = core::str::from_utf8(ssid_bytes).unwrap_or("?");
        let is_connected = state.connected && ssid_bytes == state.connected_ssid.as_slice();
        let is_secure = matches!(net_info.security, WifiSecurity::Wpa2Personal);

        // Connected indicator: blue dot
        if is_connected {
            surface.fill_circle_aa(PAD + 4, y + ROW_H as i32 / 2, 4, BLUE_DOT);
        }

        // SSID name
        let name_x = PAD + 16;
        let text_col = if is_connected { TEXT } else { TEXT };
        libfont_client::draw_string_buf(buf, POPUP_W, h, name_x, y + 6, text_col, 0, FONT_NORMAL, ssid);

        // Right side: lock icon (if secured) + checkmark (if connected)
        let mut right_x = POPUP_W as i32 - PAD;

        if is_connected {
            // Checkmark ✓
            right_x -= 16;
            libfont_client::draw_string_buf(buf, POPUP_W, h, right_x, y + 6, GREEN_CHECK, 0, FONT_NORMAL, "\u{2713}");
        }

        if is_secure {
            right_x -= 14;
            libfont_client::draw_string_buf(buf, POPUP_W, h, right_x, y + 7, LOCK_COLOR, 0, FONT_SMALL, "\u{1F512}");
        }

        // Signal strength bars (right of lock)
        let bars = if net_info.rssi > -50 { 3 } else if net_info.rssi > -65 { 2 }
                  else if net_info.rssi > -80 { 1 } else { 0 };
        let bar_x = POPUP_W as i32 - PAD - 40;
        for b in 0..3i32 {
            let bh = (4 + b * 3) as u32;
            let by = y + ROW_H as i32 - 6 - bh as i32;
            let bc = if b <= bars as i32 { TEXT } else { 0xFF555555 };
            surface.fill_rect(bar_x + b * 5, by, 3, bh, bc);
        }

        y += ROW_H as i32;
    }

    // Separator
    surface.fill_rect(PAD, y + 4, POPUP_W - PAD as u32 * 2, 1, DIVIDER);
    y += SEP_H as i32;

    // Footer: "Network Settings..."
    libfont_client::draw_string_buf(buf, POPUP_W, h, PAD, y + 8, TEXT_DIM, 0, FONT_NORMAL, "Network Settings...");
}

// ── Network click handler ───────────────────────────────────────────────────

fn handle_network_click(state: &mut WifiMonState, y_offset: i32) -> bool {
    let list_start = (HEADER_H + SEP_H) as i32;
    let rel_y = y_offset - list_start;
    if rel_y < 0 { return false; }

    let idx = (rel_y / ROW_H as i32) as usize;
    if idx >= state.networks.len() { return false; }

    let net_info = &state.networks[idx];
    let ssid = &net_info.ssid[..net_info.ssid_len];
    let ssid_str = core::str::from_utf8(ssid).unwrap_or("");

    // Already connected to this network?
    if state.connected && ssid == state.connected_ssid.as_slice() {
        // Disconnect
        net::wifi_disconnect();
        return true;
    }

    // Known network? Connect with stored password
    if let Some(pass) = state.get_password(ssid) {
        let pass_str = core::str::from_utf8(pass).unwrap_or("");
        net::wifi_connect(ssid_str, pass_str);
        return true;
    }

    // Unknown network: prompt for password
    // (In a real implementation, this would open a password dialog.
    //  For now, we use a simple approach: open a terminal prompt)
    if matches!(net_info.security, WifiSecurity::Wpa2Personal) {
        // TODO: Open a proper password dialog via anyui
        // For now, try connecting without password (will fail for WPA2)
        // The user can use `wifi connect SSID PASSWORD` from terminal
        anyos_std::println!("wifimon: network '{}' requires password", ssid_str);
        anyos_std::println!("  Use: wifi connect {} <password>", ssid_str);
    } else {
        // Open network, connect directly
        net::wifi_connect(ssid_str, "");
    }
    true
}

// ── Toggle WiFi ─────────────────────────────────────────────────────────────

fn handle_header_click(state: &mut WifiMonState) {
    state.enabled = !state.enabled;
    if !state.enabled {
        net::wifi_disconnect();
        state.networks.clear();
    } else {
        net::wifi_scan();
    }
}

// ── Main loop ───────────────────────────────────────────────────────────────

fn main() {
    process::sleep(800); // Wait for compositor + USB enumeration

    // Only show tray icon if WiFi hardware is present
    if !net::wifi_available() {
        // No WiFi adapter — silently exit, no tray icon
        return;
    }

    if !libfont_client::init() { return; }

    let client = match TrayClient::init() {
        Some(c) => c,
        None => return,
    };

    let mut state = WifiMonState::new();
    state.known_networks = load_known_networks();

    // Initial scan
    net::wifi_scan();
    process::sleep(2000); // Wait for scan results
    state.refresh();

    // Draw initial icon
    let rssi = if state.connected {
        state.networks.iter()
            .find(|n| n.ssid[..n.ssid_len] == *state.connected_ssid)
            .map(|n| n.rssi)
            .unwrap_or(-80)
    } else { -100 };
    let mut icon_pixels = [0u32; 256];
    draw_wifi_icon(&mut icon_pixels, state.connected, rssi);
    client.set_icon(ICON_ID, &icon_pixels);

    let mut popup: Option<WindowHandle> = None;
    let mut tick: u32 = 0;

    loop {
        while let Some(event) = client.poll_event() {
            match event.event_type {
                EVT_STATUS_ICON_CLICK if event.arg1 == ICON_ID => {
                    if popup.is_some() {
                        if let Some(ref p) = popup { client.destroy_window(p); }
                        popup = None;
                    } else {
                        // Refresh before showing
                        state.refresh();
                        let h = state.popup_height();
                        let h32 = h;
                        let win = match client.create_window(100, 24, POPUP_W, h32, BORDERLESS) {
                            Some(w) => w,
                            None => continue,
                        };
                        draw_popup(&win, &state);
                        client.present(&win);
                        popup = Some(win);
                    }
                }
                EVT_MOUSE_DOWN => {
                    if let Some(ref win) = popup {
                        if event.window_id == win.id {
                            let my = event.arg2 as i32;

                            // Header click → toggle WiFi
                            if my < HEADER_H as i32 {
                                handle_header_click(&mut state);
                                draw_popup(win, &state);
                                client.present(win);
                                draw_wifi_icon(&mut icon_pixels, state.connected, rssi);
                                client.set_icon(ICON_ID, &icon_pixels);
                            }
                            // Network list click
                            else if my < (state.popup_height() - FOOTER_H - SEP_H) as i32 {
                                if handle_network_click(&mut state, my) {
                                    // Close popup after action
                                    client.destroy_window(win);
                                    popup = None;
                                    // Refresh icon after connection attempt
                                    process::sleep(500);
                                    state.refresh();
                                    let new_rssi = if state.connected { -50 } else { -100 };
                                    draw_wifi_icon(&mut icon_pixels, state.connected, new_rssi);
                                    client.set_icon(ICON_ID, &icon_pixels);
                                }
                            }
                            // Footer click → open Network Settings
                            else if my >= (state.popup_height() - FOOTER_H) as i32 {
                                client.destroy_window(win);
                                popup = None;
                                // Open Settings app to Network page
                                let _ = process::spawn("/Applications/Settings.app", "network");
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

        // Periodic refresh (every ~10 seconds)
        tick += 1;
        if tick % 100 == 0 && state.enabled {
            net::wifi_scan();
        }
        if tick % 105 == 0 && state.enabled {
            state.refresh();
            let new_rssi = if state.connected { -50 } else { -100 };
            draw_wifi_icon(&mut icon_pixels, state.connected, new_rssi);
            client.set_icon(ICON_ID, &icon_pixels);
        }

        process::sleep(100);
    }
}
