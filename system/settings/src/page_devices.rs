//! Settings page: Devices — enumerate all detected hardware.
//!
//! Calls `sys::devlist()` to get all devices, groups them by type
//! (Storage, Display, Input, Network, Audio, Other), and displays
//! each group as an expandable section with device details.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::sys;
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── Device type constants (must match kernel) ───────────────────────────────

const DEV_BLOCK: u8 = 0;
const DEV_CHAR: u8 = 1;
const DEV_NETWORK: u8 = 2;
const DEV_DISPLAY: u8 = 3;
const DEV_INPUT: u8 = 4;
const DEV_AUDIO: u8 = 5;
const DEV_OUTPUT: u8 = 6;
const DEV_SENSOR: u8 = 7;
const DEV_BUS: u8 = 8;

// ── Parsed device info ──────────────────────────────────────────────────────

struct DeviceInfo {
    path: String,
    driver: String,
    dev_type: u8,
}

// ── Device type helpers ─────────────────────────────────────────────────────

fn type_name(t: u8) -> &'static str {
    match t {
        DEV_BLOCK => "Storage",
        DEV_CHAR => "Character",
        DEV_NETWORK => "Network",
        DEV_DISPLAY => "Display & GPU",
        DEV_INPUT => "Input",
        DEV_AUDIO => "Audio",
        DEV_OUTPUT => "Output",
        DEV_SENSOR => "Sensor",
        DEV_BUS => "Bus",
        _ => "Other",
    }
}

fn type_icon(t: u8) -> &'static str {
    match t {
        DEV_BLOCK => "device-desktop",
        DEV_NETWORK => "network",
        DEV_DISPLAY => "device-desktop",
        DEV_INPUT => "settings",
        DEV_AUDIO => "settings",
        _ => "settings",
    }
}

// ── Group ordering ──────────────────────────────────────────────────────────

const GROUP_ORDER: &[u8] = &[
    DEV_BLOCK, DEV_DISPLAY, DEV_INPUT, DEV_NETWORK, DEV_AUDIO,
    DEV_OUTPUT, DEV_CHAR, DEV_SENSOR, DEV_BUS, 9, // 9 = Unknown
];

// ── Build ───────────────────────────────────────────────────────────────────

/// Build the Devices settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_FILL);
    panel.set_color(layout::BG);

    layout::build_page_header(&panel, "Devices", "Connected hardware and drivers");

    let devices = enumerate_devices();

    if devices.is_empty() {
        let empty = ui::Label::new("No devices detected");
        empty.set_dock(ui::DOCK_TOP);
        empty.set_size(552, 30);
        empty.set_font_size(13);
        empty.set_text_color(layout::TEXT_DIM);
        empty.set_margin(24, 8, 24, 0);
        panel.add(&empty);
    } else {
        // Summary card
        let summary = layout::build_auto_card(&panel);
        let count_str = format!("{} devices", devices.len());
        layout::build_info_row(&summary, "Total", &count_str, true);

        // Group by type
        for &gt in GROUP_ORDER {
            let group: Vec<&DeviceInfo> = devices.iter().filter(|d| d.dev_type == gt).collect();
            if group.is_empty() {
                continue;
            }
            build_device_group(&panel, type_name(gt), &group);
        }
    }

    parent.add(&panel);
    panel.id()
}

// ── Device group card ───────────────────────────────────────────────────────

fn build_device_group(panel: &ui::View, group_name: &str, devices: &[&DeviceInfo]) {
    // Group header
    let header_text = format!("{} ({})", group_name, devices.len());
    let header = ui::Label::new(&header_text);
    header.set_dock(ui::DOCK_TOP);
    header.set_size(600, 28);
    header.set_font_size(13);
    header.set_text_color(0xFFFFFFFF);
    header.set_margin(24, 12, 24, 0);
    panel.add(&header);

    let card = layout::build_auto_card(panel);

    for (i, dev) in devices.iter().enumerate() {
        let row = ui::View::new();
        row.set_dock(ui::DOCK_TOP);
        row.set_size(552, 44);
        row.set_margin(24, if i == 0 { 8 } else { 0 }, 24, 0);

        // Device path
        let path_lbl = ui::Label::new(&dev.path);
        path_lbl.set_position(0, 4);
        path_lbl.set_size(200, 18);
        path_lbl.set_text_color(0xFFCCCCCC);
        path_lbl.set_font_size(13);
        row.add(&path_lbl);

        // Driver name
        let drv_lbl = ui::Label::new(&dev.driver);
        drv_lbl.set_position(0, 24);
        drv_lbl.set_size(200, 16);
        drv_lbl.set_text_color(0xFF808080);
        drv_lbl.set_font_size(11);
        row.add(&drv_lbl);

        // Type badge on the right
        let type_lbl = ui::Label::new(type_name(dev.dev_type));
        type_lbl.set_position(400, 12);
        type_lbl.set_size(120, 20);
        type_lbl.set_text_color(layout::TEXT_DIM);
        type_lbl.set_font_size(11);
        row.add(&type_lbl);

        card.add(&row);

        if i + 1 < devices.len() {
            layout::build_separator(&card);
        }
    }

    // Bottom padding
    let pad = ui::View::new();
    pad.set_dock(ui::DOCK_TOP);
    pad.set_size(552, 8);
    card.add(&pad);
}

// ── Device enumeration ──────────────────────────────────────────────────────

fn enumerate_devices() -> Vec<DeviceInfo> {
    let mut devices = Vec::new();

    // Each entry is 64 bytes:
    //   [0..32]  path (null-terminated)
    //   [32..56] driver name (null-terminated)
    //   [56]     device type
    //   [57..64] padding
    const MAX_DEVICES: usize = 64;
    let mut buf = [0u8; MAX_DEVICES * 64];
    let count = sys::devlist(&mut buf);
    if count == u32::MAX || count == 0 {
        return devices;
    }

    let n = (count as usize).min(MAX_DEVICES);
    for i in 0..n {
        let entry = &buf[i * 64..(i + 1) * 64];

        // Path: null-terminated string in [0..32]
        let path_end = entry[..32].iter().position(|&b| b == 0).unwrap_or(32);
        let path = match core::str::from_utf8(&entry[..path_end]) {
            Ok(s) if !s.is_empty() => String::from(s),
            _ => continue,
        };

        // Driver: null-terminated string in [32..56]
        let drv_end = entry[32..56].iter().position(|&b| b == 0).unwrap_or(24);
        let driver = match core::str::from_utf8(&entry[32..32 + drv_end]) {
            Ok(s) => String::from(s),
            Err(_) => String::from("unknown"),
        };

        let dev_type = entry[56];

        devices.push(DeviceInfo {
            path,
            driver,
            dev_type,
        });
    }

    devices
}
