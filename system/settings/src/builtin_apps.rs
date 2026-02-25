//! Built-in: Apps & Permissions management.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::permissions;
use libanyui_client as ui;
use ui::Widget;

// Capability bit definitions (must match kernel)
const CAP_FILESYSTEM: u32 = 1 << 0;
const CAP_NETWORK: u32 = 1 << 1;
const CAP_AUDIO: u32 = 1 << 2;
const CAP_DISPLAY: u32 = 1 << 3;
const CAP_DEVICE: u32 = 1 << 4;
const CAP_PROCESS: u32 = 1 << 5;
const CAP_COMPOSITOR: u32 = 1 << 9;
const CAP_SYSTEM: u32 = 1 << 10;

struct PermGroup {
    mask: u32,
    name: &'static str,
}

const PERM_GROUPS: &[PermGroup] = &[
    PermGroup { mask: CAP_FILESYSTEM, name: "Files & Storage" },
    PermGroup { mask: CAP_NETWORK, name: "Internet & Network" },
    PermGroup { mask: CAP_AUDIO, name: "Audio" },
    PermGroup { mask: CAP_DISPLAY | CAP_COMPOSITOR, name: "Display" },
    PermGroup { mask: CAP_DEVICE, name: "Devices" },
    PermGroup { mask: CAP_PROCESS | CAP_SYSTEM, name: "System" },
];

struct AppEntry {
    app_id: String,
    granted: u32,
}

/// Build the Apps settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_FILL);
    panel.set_color(0xFF1E1E1E);

    // Title
    let title = ui::Label::new("Apps");
    title.set_dock(ui::DOCK_TOP);
    title.set_size(560, 36);
    title.set_font_size(18);
    title.set_text_color(0xFFFFFFFF);
    title.set_margin(16, 12, 16, 4);
    panel.add(&title);

    let desc = ui::Label::new("Manage app permissions");
    desc.set_dock(ui::DOCK_TOP);
    desc.set_size(560, 24);
    desc.set_font_size(12);
    desc.set_text_color(0xFF808080);
    desc.set_margin(16, 0, 16, 4);
    panel.add(&desc);

    // Scan apps
    let apps = scan_apps();

    if apps.is_empty() {
        let empty = ui::Label::new("No apps with stored permissions.");
        empty.set_dock(ui::DOCK_TOP);
        empty.set_size(560, 30);
        empty.set_font_size(13);
        empty.set_text_color(0xFF969696);
        empty.set_margin(16, 8, 16, 0);
        panel.add(&empty);
    } else {
        for app in &apps {
            build_app_expander(&panel, app);
        }
    }

    parent.add(&panel);
    panel.id()
}

fn build_app_expander(panel: &ui::View, app: &AppEntry) {
    let expander = ui::Expander::new(&app.app_id);
    expander.set_dock(ui::DOCK_TOP);
    expander.set_size(560, 0);
    expander.set_margin(16, 4, 16, 4);
    expander.set_expanded(false);

    // Permission toggles
    let granted = app.granted;
    let app_id = app.app_id.clone();

    for group in PERM_GROUPS {
        let row = ui::View::new();
        row.set_dock(ui::DOCK_TOP);
        row.set_size(528, 32);
        row.set_margin(8, 0, 8, 0);

        let lbl = ui::Label::new(group.name);
        lbl.set_position(0, 6);
        lbl.set_size(200, 20);
        lbl.set_text_color(0xFFCCCCCC);
        lbl.set_font_size(12);
        row.add(&lbl);

        let is_on = (granted & group.mask) != 0;
        let toggle = ui::Toggle::new(is_on);
        toggle.set_position(460, 2);

        let mask = group.mask;
        let aid = app_id.clone();
        let old_granted = granted;
        toggle.on_checked_changed(move |e| {
            let new_granted = if e.checked {
                old_granted | mask
            } else {
                old_granted & !mask
            };
            let uid = anyos_std::process::getuid();
            permissions::perm_store(&aid, new_granted, uid);
        });

        row.add(&toggle);
        expander.add(&row);
    }

    // Delete button
    let del_row = ui::View::new();
    del_row.set_dock(ui::DOCK_TOP);
    del_row.set_size(528, 32);
    del_row.set_margin(8, 4, 8, 4);

    let del_btn = ui::IconButton::new("Reset Permissions");
    del_btn.set_position(0, 0);
    del_btn.set_size(140, 28);
    let aid2 = app.app_id.clone();
    del_btn.on_click(move |_| {
        permissions::perm_delete(&aid2);
    });
    del_row.add(&del_btn);
    expander.add(&del_row);

    panel.add(&expander);
}

fn scan_apps() -> Vec<AppEntry> {
    let mut entries = Vec::new();
    let mut buf = [0u8; 4096];
    let count = permissions::perm_list(&mut buf);
    if count == 0 || count == u32::MAX {
        return entries;
    }

    // Parse "app_id\x1Fgranted_hex\n" entries
    let mut pos = 0usize;
    let data = &buf[..];

    while pos < data.len() && entries.len() < 32 {
        let line_end = data[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| pos + p)
            .unwrap_or(data.len());
        let line = &data[pos..line_end];
        pos = line_end + 1;

        if line.is_empty() {
            continue;
        }

        if let Some(sep) = line.iter().position(|&b| b == 0x1F) {
            let id_bytes = &line[..sep];
            let hex_bytes = &line[sep + 1..];

            if !id_bytes.is_empty() && id_bytes.len() <= 64 {
                if let Ok(id_str) = core::str::from_utf8(id_bytes) {
                    entries.push(AppEntry {
                        app_id: String::from(id_str),
                        granted: parse_hex(hex_bytes),
                    });
                }
            }
        }
    }

    // Sort by app_id
    entries.sort_unstable_by(|a, b| a.app_id.as_str().cmp(b.app_id.as_str()));
    entries
}

fn parse_hex(s: &[u8]) -> u32 {
    let mut val: u32 = 0;
    for &b in s {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => continue,
        };
        val = val.wrapping_shl(4) | digit as u32;
    }
    val
}
