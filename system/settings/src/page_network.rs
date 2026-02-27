//! Settings page: Network — connection status and per-NIC configuration.
//!
//! Reads live network state via `net::get_config()` and interface configs via
//! `net::get_interfaces()`.  Shows only kernel-detected NICs.  The "Configure..."
//! button opens a dedicated dialog for DHCP/static selection and IP entry.

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
    panel.set_color(layout::bg());

    layout::build_page_header(&panel, "Network", "Connection status and interface configuration");

    // ── Live connection status ───────────────────────────────────
    let mut net_buf = [0u8; 24];
    net::get_config(&mut net_buf);

    let ip = [net_buf[0], net_buf[1], net_buf[2], net_buf[3]];
    let mask = [net_buf[4], net_buf[5], net_buf[6], net_buf[7]];
    let gw = [net_buf[8], net_buf[9], net_buf[10], net_buf[11]];
    let dns_addr = [net_buf[12], net_buf[13], net_buf[14], net_buf[15]];
    let mac = [net_buf[16], net_buf[17], net_buf[18], net_buf[19], net_buf[20], net_buf[21]];
    let link_up = net_buf[22] != 0;

    let status_card = layout::build_auto_card(&panel);
    let (status_text, status_color) = if link_up {
        ("Connected", ui::theme::colors().success)
    } else {
        ("Disconnected", ui::theme::colors().destructive)
    };
    layout::build_info_row_colored(&status_card, "Status", status_text, status_color, true);
    layout::build_separator(&status_card);

    let mut b = [0u8; 20];
    layout::build_info_row(&status_card, "IP Address", fmt_ip(&mut b, &ip), false);
    layout::build_separator(&status_card);

    let mut b = [0u8; 20];
    layout::build_info_row(&status_card, "Subnet Mask", fmt_ip(&mut b, &mask), false);
    layout::build_separator(&status_card);

    let mut b = [0u8; 20];
    layout::build_info_row(&status_card, "Gateway", fmt_ip(&mut b, &gw), false);
    layout::build_separator(&status_card);

    let mut b = [0u8; 20];
    layout::build_info_row(&status_card, "DNS Server", fmt_ip(&mut b, &dns_addr), false);
    layout::build_separator(&status_card);

    let mut b = [0u8; 20];
    layout::build_info_row(&status_card, "MAC Address", fmt_mac(&mut b, &mac), false);

    // ── Detected interfaces card ─────────────────────────────────
    let nic_available = net::is_nic_available();
    if nic_available {
        // Get NIC driver name from the kernel
        let mut drv_buf = [0u8; 64];
        let drv_len = net::nic_driver_name(&mut drv_buf) as usize;
        let drv_name = if drv_len > 0 {
            core::str::from_utf8(&drv_buf[..drv_len]).unwrap_or("Unknown NIC")
        } else {
            "Network Adapter"
        };

        // Read current interface config for eth0
        let mut iface_buf = [0u8; 512];
        let iface_count = net::get_interfaces(&mut iface_buf);

        // Determine current method for eth0
        let (method_str, cur_method) = if iface_count > 0 && iface_count != u32::MAX {
            let m = iface_buf[0]; // method of first interface
            if m == 1 { ("Static", 1u8) } else { ("DHCP", 0u8) }
        } else {
            ("DHCP", 0u8)
        };

        let iface_card = layout::build_auto_card(&panel);

        // Driver name row
        layout::build_info_row(&iface_card, "Adapter", drv_name, true);
        layout::build_separator(&iface_card);

        // Interface row: "eth0" label + method + "Configure..." button
        let row = layout::build_setting_row(&iface_card, "eth0", false);

        let method_lbl = ui::Label::new(method_str);
        method_lbl.set_position(200, 12);
        method_lbl.set_size(140, 20);
        method_lbl.set_text_color(layout::text_dim());
        method_lbl.set_font_size(13);
        row.add(&method_lbl);

        let method_lbl_id = method_lbl.id();
        let iface_buf_copy = iface_buf;

        let btn = ui::Button::new("Configure...");
        btn.set_position(400, 8);
        btn.set_size(110, 28);
        btn.on_click(move |_| {
            open_configure_dialog("eth0", cur_method, &iface_buf_copy, method_lbl_id);
        });
        row.add(&btn);
    } else {
        let no_nic_card = layout::build_auto_card(&panel);
        layout::build_info_row(&no_nic_card, "Interfaces", "No network adapter detected", true);
    }

    parent.add(&panel);
    panel.id()
}

// ── Configure dialog ────────────────────────────────────────────────────────

/// Open a dedicated configuration dialog for the given interface.
fn open_configure_dialog(
    iface_name: &str,
    current_method: u8,
    iface_buf: &[u8; 512],
    method_label_id: u32,
) {
    let title = "Configure Network Interface";
    let win = ui::Window::new(title, -1, -1, 420, 340);
    let win_id = win.id();

    let root = ui::View::new();
    root.set_dock(ui::DOCK_FILL);
    root.set_color(layout::bg());

    // ── Interface name ──────────────────────────────────────────
    let name_lbl = ui::Label::new(iface_name);
    name_lbl.set_dock(ui::DOCK_TOP);
    name_lbl.set_size(380, 28);
    name_lbl.set_font_size(16);
    name_lbl.set_text_color(layout::text());
    name_lbl.set_margin(20, 16, 20, 0);
    root.add(&name_lbl);

    // ── Method dropdown ─────────────────────────────────────────
    let method_row = ui::View::new();
    method_row.set_dock(ui::DOCK_TOP);
    method_row.set_size(380, 36);
    method_row.set_margin(20, 12, 20, 0);

    let method_lbl = ui::Label::new("Method");
    method_lbl.set_position(0, 8);
    method_lbl.set_size(100, 20);
    method_lbl.set_text_color(layout::text());
    method_lbl.set_font_size(13);
    method_row.add(&method_lbl);

    let dd = ui::DropDown::new("DHCP|Static");
    dd.set_position(110, 4);
    dd.set_size(260, 28);
    dd.set_selected_index(current_method as u32);
    method_row.add(&dd);
    root.add(&method_row);

    // ── IP fields ───────────────────────────────────────────────
    let addr_ip = &iface_buf[18..22];
    let addr_mask = &iface_buf[22..26];
    let addr_gw = &iface_buf[26..30];
    let addr_dns = &iface_buf[30..34];

    let is_static = current_method == 1;

    let tf_ip = build_dialog_ip_row(&root, "Address", addr_ip, is_static);
    let tf_mask = build_dialog_ip_row(&root, "Netmask", addr_mask, is_static);
    let tf_gw = build_dialog_ip_row(&root, "Gateway", addr_gw, is_static);
    let tf_dns = build_dialog_ip_row(&root, "DNS", addr_dns, is_static);

    // Wire method dropdown to enable/disable fields
    let ip_id = tf_ip.id();
    let mask_id = tf_mask.id();
    let gw_id = tf_gw.id();
    let dns_id = tf_dns.id();

    dd.on_selection_changed(move |e| {
        let enabled = e.index == 1; // Static
        ui::Control::from_id(ip_id).set_enabled(enabled);
        ui::Control::from_id(mask_id).set_enabled(enabled);
        ui::Control::from_id(gw_id).set_enabled(enabled);
        ui::Control::from_id(dns_id).set_enabled(enabled);
    });

    // ── Button row ──────────────────────────────────────────────
    let btn_row = ui::View::new();
    btn_row.set_dock(ui::DOCK_TOP);
    btn_row.set_size(380, 44);
    btn_row.set_margin(20, 16, 20, 16);

    let dd_id = dd.id();
    let wid = win_id;
    let mlbl_id = method_label_id;

    // Build name bytes for the syscall entry
    let name_bytes: [u8; 16] = {
        let mut nb = [0u8; 16];
        let src = iface_name.as_bytes();
        let len = src.len().min(16);
        nb[..len].copy_from_slice(&src[..len]);
        nb
    };
    let name_len = iface_name.len().min(16) as u8;

    let apply_btn = ui::Button::new("Apply");
    apply_btn.set_position(0, 4);
    apply_btn.set_size(100, 32);
    apply_btn.on_click(move |_| {
        let method_idx = ui::Control::from_id(dd_id).get_state() as u8;

        // Build entry (64 bytes)
        let mut entry = [0u8; 64];
        entry[0] = method_idx;
        entry[1] = name_len;
        entry[2..2 + name_len as usize].copy_from_slice(&name_bytes[..name_len as usize]);

        if method_idx == 1 {
            if let Some(a) = read_ip_field(ip_id) { entry[18..22].copy_from_slice(&a); }
            if let Some(a) = read_ip_field(mask_id) { entry[22..26].copy_from_slice(&a); }
            if let Some(a) = read_ip_field(gw_id) { entry[26..30].copy_from_slice(&a); }
            if let Some(a) = read_ip_field(dns_id) { entry[30..34].copy_from_slice(&a); }
        }

        // Syscall buffer: [count:u32, entries...]
        let count: u32 = 1;
        let mut syscall_buf = [0u8; 4 + 64];
        syscall_buf[0..4].copy_from_slice(&count.to_le_bytes());
        syscall_buf[4..68].copy_from_slice(&entry);
        net::set_interfaces(&syscall_buf);

        // Update the method label in the main settings page
        let new_text = if method_idx == 1 { "Static" } else { "DHCP" };
        ui::Control::from_id(mlbl_id).set_text(new_text);

        // Close the dialog
        let w: ui::Window = unsafe { core::mem::transmute(wid) };
        w.destroy();
    });
    btn_row.add(&apply_btn);

    let wid2 = win_id;
    let cancel_btn = ui::Button::new("Cancel");
    cancel_btn.set_position(110, 4);
    cancel_btn.set_size(100, 32);
    cancel_btn.on_click(move |_| {
        let w: ui::Window = unsafe { core::mem::transmute(wid2) };
        w.destroy();
    });
    btn_row.add(&cancel_btn);

    root.add(&btn_row);
    win.add(&root);

    let wid3 = win_id;
    win.on_close(move |_| {
        let w: ui::Window = unsafe { core::mem::transmute(wid3) };
        w.destroy();
    });
}

/// Build a labeled IP text field row inside a dialog.
fn build_dialog_ip_row(parent: &ui::View, label: &str, ip_bytes: &[u8], enabled: bool) -> ui::TextField {
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(380, 36);
    row.set_margin(20, 6, 20, 0);

    let lbl = ui::Label::new(label);
    lbl.set_position(0, 8);
    lbl.set_size(100, 20);
    lbl.set_text_color(layout::text());
    lbl.set_font_size(13);
    row.add(&lbl);

    let tf = ui::TextField::new();
    tf.set_position(110, 4);
    tf.set_size(260, 28);
    tf.set_placeholder("0.0.0.0");

    if ip_bytes.len() >= 4 && ip_bytes[..4] != [0, 0, 0, 0] {
        let mut buf = [0u8; 20];
        let s = fmt_ip(&mut buf, &[ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]]);
        tf.set_text(s);
    }

    tf.set_enabled(enabled);
    row.add(&tf);
    parent.add(&row);
    tf
}

/// Read an IP address from a text field control.
fn read_ip_field(tf_id: u32) -> Option<[u8; 4]> {
    let ctrl = ui::Control::from_id(tf_id);
    let mut buf = [0u8; 20];
    let len = ctrl.get_text(&mut buf) as usize;
    if len == 0 {
        return None;
    }
    let text = core::str::from_utf8(&buf[..len]).ok()?;
    parse_ip(text)
}

/// Parse "a.b.c.d" string into `[u8; 4]`.
fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut parts = [0u8; 4];
    let mut idx = 0usize;
    let mut num: u32 = 0;
    let mut has_digit = false;

    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                num = num * 10 + (b - b'0') as u32;
                if num > 255 { return None; }
                has_digit = true;
            }
            b'.' => {
                if !has_digit || idx >= 3 { return None; }
                parts[idx] = num as u8;
                idx += 1;
                num = 0;
                has_digit = false;
            }
            _ => return None,
        }
    }

    if !has_digit || idx != 3 { return None; }
    parts[3] = num as u8;
    Some(parts)
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
