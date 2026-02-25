//! Built-in: Network settings (IP, Subnet, Gateway, DNS, MAC, Status).

use anyos_std::net;
use libanyui_client as ui;
use ui::Widget;

/// Build the Network settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_FILL);
    panel.set_color(0xFF1E1E1E);

    // Title
    let title = ui::Label::new("Network");
    title.set_dock(ui::DOCK_TOP);
    title.set_size(560, 36);
    title.set_font_size(18);
    title.set_text_color(0xFFFFFFFF);
    title.set_margin(16, 12, 16, 4);
    panel.add(&title);

    // Read network config
    let mut net_buf = [0u8; 24];
    net::get_config(&mut net_buf);

    let ip = [net_buf[0], net_buf[1], net_buf[2], net_buf[3]];
    let mask = [net_buf[4], net_buf[5], net_buf[6], net_buf[7]];
    let gw = [net_buf[8], net_buf[9], net_buf[10], net_buf[11]];
    let dns_ip = [net_buf[12], net_buf[13], net_buf[14], net_buf[15]];
    let mac = [net_buf[16], net_buf[17], net_buf[18], net_buf[19], net_buf[20], net_buf[21]];
    let link_up = net_buf[22] != 0;

    // Card
    let card = ui::Card::new();
    card.set_dock(ui::DOCK_TOP);
    card.set_size(560, 280);
    card.set_margin(16, 8, 16, 8);
    card.set_color(0xFF2D2D30);

    // Status row
    build_row(&card, "Status", if link_up { "Connected" } else { "Disconnected" },
        if link_up { 0xFF4EC970 } else { 0xFFE06C75 }, true);

    // IP Address
    let mut b = [0u8; 20];
    build_row(&card, "IP Address", fmt_ip(&mut b, &ip), 0xFF969696, false);

    // Subnet Mask
    let mut b = [0u8; 20];
    build_row(&card, "Subnet Mask", fmt_ip(&mut b, &mask), 0xFF969696, false);

    // Gateway
    let mut b = [0u8; 20];
    build_row(&card, "Gateway", fmt_ip(&mut b, &gw), 0xFF969696, false);

    // DNS Server
    let mut b = [0u8; 20];
    build_row(&card, "DNS Server", fmt_ip(&mut b, &dns_ip), 0xFF969696, false);

    // MAC Address
    let mut b = [0u8; 20];
    build_row(&card, "MAC Address", fmt_mac(&mut b, &mac), 0xFF969696, false);

    panel.add(&card);
    parent.add(&panel);
    panel.id()
}

fn build_row(card: &ui::Card, label_text: &str, value_text: &str, value_color: u32, first: bool) {
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(528, 40);
    row.set_margin(16, if first { 8 } else { 0 }, 16, 0);

    let lbl = ui::Label::new(label_text);
    lbl.set_position(0, 10);
    lbl.set_size(140, 20);
    lbl.set_text_color(0xFFCCCCCC);
    lbl.set_font_size(13);
    row.add(&lbl);

    let val = ui::Label::new(value_text);
    val.set_position(150, 10);
    val.set_size(360, 20);
    val.set_text_color(value_color);
    val.set_font_size(13);
    row.add(&val);

    card.add(&row);
}

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
