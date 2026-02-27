//! Settings page: Network — connection status and configuration.
//!
//! Reads network configuration via `net::get_config()` and displays
//! connection status, IP address, subnet mask, gateway, DNS, and MAC.

use anyos_std::net;
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── Build ───────────────────────────────────────────────────────────────────

/// Build the Network settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::BG);

    layout::build_page_header(&panel, "Network", "Connection status and configuration");

    // Read network config
    let mut net_buf = [0u8; 24];
    net::get_config(&mut net_buf);

    let ip = [net_buf[0], net_buf[1], net_buf[2], net_buf[3]];
    let mask = [net_buf[4], net_buf[5], net_buf[6], net_buf[7]];
    let gw = [net_buf[8], net_buf[9], net_buf[10], net_buf[11]];
    let dns = [net_buf[12], net_buf[13], net_buf[14], net_buf[15]];
    let mac = [net_buf[16], net_buf[17], net_buf[18], net_buf[19], net_buf[20], net_buf[21]];
    let link_up = net_buf[22] != 0;

    // ── Connection status card ──────────────────────────────────────────
    let status_card = layout::build_auto_card(&panel);
    let (status_text, status_color) = if link_up {
        ("Connected", 0xFF4EC970u32)
    } else {
        ("Disconnected", 0xFFE06C75u32)
    };
    layout::build_info_row_colored(&status_card, "Status", status_text, status_color, true);

    // ── Configuration card ──────────────────────────────────────────────
    let config_card = layout::build_auto_card(&panel);

    let mut b = [0u8; 20];
    layout::build_info_row(&config_card, "IP Address", fmt_ip(&mut b, &ip), true);
    layout::build_separator(&config_card);

    let mut b = [0u8; 20];
    layout::build_info_row(&config_card, "Subnet Mask", fmt_ip(&mut b, &mask), false);
    layout::build_separator(&config_card);

    let mut b = [0u8; 20];
    layout::build_info_row(&config_card, "Gateway", fmt_ip(&mut b, &gw), false);
    layout::build_separator(&config_card);

    let mut b = [0u8; 20];
    layout::build_info_row(&config_card, "DNS Server", fmt_ip(&mut b, &dns), false);
    layout::build_separator(&config_card);

    let mut b = [0u8; 20];
    layout::build_info_row(&config_card, "MAC Address", fmt_mac(&mut b, &mac), false);

    parent.add(&panel);
    panel.id()
}

// ── IP / MAC formatting ─────────────────────────────────────────────────────

fn fmt_ip<'a>(buf: &'a mut [u8; 20], ip: &[u8; 4]) -> &'a str {
    let mut p = 0;
    for i in 0..4 {
        p = write_u8_dec(buf, p, ip[i]);
        if i < 3 {
            buf[p] = b'.';
            p += 1;
        }
    }
    core::str::from_utf8(&buf[..p]).unwrap_or("?")
}

fn fmt_mac<'a>(buf: &'a mut [u8; 20], mac: &[u8; 6]) -> &'a str {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut p = 0;
    for i in 0..6 {
        buf[p] = HEX[(mac[i] >> 4) as usize];
        buf[p + 1] = HEX[(mac[i] & 0xF) as usize];
        p += 2;
        if i < 5 {
            buf[p] = b':';
            p += 1;
        }
    }
    core::str::from_utf8(&buf[..p]).unwrap_or("?")
}

fn write_u8_dec(buf: &mut [u8], pos: usize, val: u8) -> usize {
    let mut p = pos;
    if val >= 100 {
        buf[p] = b'0' + val / 100;
        p += 1;
    }
    if val >= 10 {
        buf[p] = b'0' + (val / 10) % 10;
        p += 1;
    }
    buf[p] = b'0' + val % 10;
    p + 1
}
