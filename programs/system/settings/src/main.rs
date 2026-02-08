#![no_std]
#![no_main]

use anyos_std::sys;
use anyos_std::net;
use anyos_std::process;
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
const PAGE_NETWORK: usize = 2;
const PAGE_ABOUT: usize = 3;
const PAGE_NAMES: [&str; 4] = ["General", "Display", "Network", "About"];

fn main() {
    let win = window::create("Settings", 180, 100, 560, 400);
    if win == u32::MAX { return; }

    let (mut win_w, mut win_h) = window::get_size(win).unwrap_or((560, 400));

    // --- Components with built-in event handling ---
    let mut sidebar = UiSidebar::new(0, 0, SIDEBAR_W, win_h);
    let content_x = SIDEBAR_W as i32 + 20;

    // General page toggles
    let mut sound_toggle = UiToggle::new(0, 0, true);
    let mut notif_toggle = UiToggle::new(0, 0, true);

    // Display page slider
    let mut brightness = UiSlider::new(0, 0, 200, 0, 100, 80);

    let mut event = [0u32; 5];
    let mut needs_redraw = true;

    loop {
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
                    update_positions(&sidebar, &mut sound_toggle, &mut notif_toggle, &mut brightness, win_w);

                    // Let each component handle the event
                    if sidebar.handle_event(&ui_event, PAGE_NAMES.len()).is_some() {
                        needs_redraw = true;
                    }

                    if sidebar.selected == PAGE_GENERAL {
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
                    }
                }
                _ => {}
            }
        }

        if needs_redraw {
            // Update positions for rendering
            update_positions(&sidebar, &mut sound_toggle, &mut notif_toggle, &mut brightness, win_w);
            render(win, &sidebar, &sound_toggle, &notif_toggle, &brightness, win_w, win_h);
            window::present(win);
            needs_redraw = false;
        }

        process::yield_cpu();
    }
}

/// Compute component positions based on current window size.
fn update_positions(
    sidebar: &UiSidebar,
    sound: &mut UiToggle,
    notif: &mut UiToggle,
    brightness: &mut UiSlider,
    win_w: u32,
) {
    let cx = SIDEBAR_W as i32 + 20;
    let cw = win_w as i32 - SIDEBAR_W as i32 - 40;
    let card_y = 54;
    let toggle_x = cx + cw - PAD - 52;

    // Sound toggle (General page, row 1)
    sound.x = toggle_x;
    sound.y = card_y + PAD + ROW_H + 5;

    // Notifications toggle (General page, row 2)
    notif.x = toggle_x;
    notif.y = card_y + PAD + ROW_H * 2 + 5;

    // Brightness slider (Display page, row 1)
    brightness.x = cx + PAD + 100;
    brightness.y = card_y + PAD + ROW_H + 6;
    brightness.w = (cw - PAD * 2 - 100) as u32;
}

// ============================================================================
// Rendering
// ============================================================================

fn render(
    win: u32,
    sidebar_c: &UiSidebar,
    sound: &UiToggle,
    notif: &UiToggle,
    brightness: &UiSlider,
    win_w: u32, win_h: u32,
) {
    window::fill_rect(win, 0, 0, win_w as u16, win_h as u16, colors::WINDOW_BG);

    // Sidebar with built-in rendering
    sidebar_c.render(win, "SETTINGS", &PAGE_NAMES);

    // Content
    let cx = SIDEBAR_W as i32 + 20;
    let cw = (win_w - SIDEBAR_W - 40) as u32;

    match sidebar_c.selected {
        PAGE_GENERAL => render_general(win, sound, notif, cx, cw),
        PAGE_DISPLAY => render_display(win, brightness, cx, cw),
        PAGE_NETWORK => render_network(win, cx, cw),
        PAGE_ABOUT => render_about(win, cx, cw),
        _ => {}
    }
}

fn render_general(win: u32, sound: &UiToggle, notif: &UiToggle, cx: i32, cw: u32) {
    label(win, cx, 20, "General", colors::TEXT, FontSize::Title, TextAlign::Left);

    let card_y = 54;
    card(win, cx, card_y, cw, (PAD * 2 + ROW_H * 3) as u32);

    // Row 0: Device Name
    let ry = card_y + PAD;
    label(win, cx + PAD, ry + 12, "Device Name", colors::TEXT, FontSize::Normal, TextAlign::Left);
    label(win, cx + PAD + 140, ry + 12, "anyOS Computer", colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    // Row 1: Sound
    let ry = ry + ROW_H;
    divider_h(win, cx + PAD, ry, cw - PAD as u32 * 2);
    label(win, cx + PAD, ry + 12, "Sound", colors::TEXT, FontSize::Normal, TextAlign::Left);
    sound.render(win);

    // Row 2: Notifications
    let ry = ry + ROW_H;
    divider_h(win, cx + PAD, ry, cw - PAD as u32 * 2);
    label(win, cx + PAD, ry + 12, "Notifications", colors::TEXT, FontSize::Normal, TextAlign::Left);
    notif.render(win);
}

fn render_display(win: u32, brightness: &UiSlider, cx: i32, cw: u32) {
    label(win, cx, 20, "Display", colors::TEXT, FontSize::Title, TextAlign::Left);

    let card_y = 54;
    card(win, cx, card_y, cw, (PAD * 2 + ROW_H * 2) as u32);

    // Row 0: Resolution
    let ry = card_y + PAD;
    label(win, cx + PAD, ry + 12, "Resolution", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let (sw, sh) = window::screen_size();
    let mut buf = [0u8; 32];
    let res = fmt_resolution(&mut buf, sw, sh);
    label(win, cx + PAD + 120, ry + 12, res, colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    // Row 1: Brightness
    let ry = ry + ROW_H;
    divider_h(win, cx + PAD, ry, cw - PAD as u32 * 2);
    label(win, cx + PAD, ry + 12, "Brightness", colors::TEXT, FontSize::Normal, TextAlign::Left);
    brightness.render(win);
}

fn render_network(win: u32, cx: i32, cw: u32) {
    label(win, cx, 20, "Network", colors::TEXT, FontSize::Title, TextAlign::Left);

    let mut net_buf = [0u8; 24];
    net::get_config(&mut net_buf);

    let ip = [net_buf[0], net_buf[1], net_buf[2], net_buf[3]];
    let mask = [net_buf[4], net_buf[5], net_buf[6], net_buf[7]];
    let gw = [net_buf[8], net_buf[9], net_buf[10], net_buf[11]];
    let dns_ip = [net_buf[12], net_buf[13], net_buf[14], net_buf[15]];
    let mac = [net_buf[16], net_buf[17], net_buf[18], net_buf[19], net_buf[20], net_buf[21]];
    let link_up = net_buf[22] != 0;

    let card_y = 54;
    card(win, cx, card_y, cw, (PAD * 2 + ROW_H * 6) as u32);

    let lx = cx + PAD;
    let vx = cx + PAD + 130;

    let ry = card_y + PAD;
    label(win, lx, ry + 12, "Status", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let kind = if link_up { StatusKind::Online } else { StatusKind::Offline };
    let text = if link_up { "Connected" } else { "Disconnected" };
    status_indicator(win, vx, ry + 12, kind, text);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "IP Address", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_ip(&mut b, &ip), colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Subnet Mask", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_ip(&mut b, &mask), colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Gateway", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_ip(&mut b, &gw), colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "DNS Server", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_ip(&mut b, &dns_ip), colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "MAC Address", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 20]; label(win, vx, ry + 12, fmt_mac(&mut b, &mac), colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);
}

fn render_about(win: u32, cx: i32, cw: u32) {
    label(win, cx, 20, "About", colors::TEXT, FontSize::Title, TextAlign::Left);

    let card_y = 54;
    card(win, cx, card_y, cw, (PAD * 2 + ROW_H * 5) as u32);

    let lx = cx + PAD;
    let vx = cx + PAD + 130;

    let ry = card_y + PAD;
    label(win, lx, ry + 12, "OS", colors::TEXT, FontSize::Normal, TextAlign::Left);
    label(win, vx, ry + 12, "anyOS 1.0", colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Kernel", colors::TEXT, FontSize::Normal, TextAlign::Left);
    label(win, vx, ry + 12, "i686-anyos", colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "CPUs", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let cpu_count = sys::sysinfo(2, &mut [0u8; 4]);
    let mut b = [0u8; 8];
    label(win, vx, ry + 12, fmt_u32(&mut b, cpu_count), colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Memory", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let mut mem = [0u8; 8];
    sys::sysinfo(0, &mut mem);
    let total = u32::from_le_bytes([mem[0], mem[1], mem[2], mem[3]]);
    let free = u32::from_le_bytes([mem[4], mem[5], mem[6], mem[7]]);
    let mut b = [0u8; 32];
    label(win, vx, ry + 12, fmt_mem(&mut b, (total * 4) / 1024, (free * 4) / 1024), colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);

    let ry = ry + ROW_H;
    divider_h(win, lx, ry, cw - PAD as u32 * 2);
    label(win, lx, ry + 12, "Uptime", colors::TEXT, FontSize::Normal, TextAlign::Left);
    let mut b = [0u8; 32];
    label(win, vx, ry + 12, fmt_uptime(&mut b, sys::uptime() / 100), colors::TEXT_SECONDARY, FontSize::Normal, TextAlign::Left);
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
