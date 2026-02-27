//! Settings page: Apps — scan installed applications, show details, manage
//! permissions, and uninstall.
//!
//! Scans `/Applications/` for `.app` bundles, reads `Info.conf` metadata from
//! each, and presents an interactive list with icons, version info, a Details
//! button (opens a permission-management window), and an Uninstall button.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::permissions;
use anyos_std::process;
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── Capability bit definitions (must match kernel) ──────────────────────────

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

// ── App bundle metadata ─────────────────────────────────────────────────────

const APPS_DIR: &str = "/Applications";
const MAX_APPS: usize = 32;

struct AppInfo {
    name: String,
    id: String,
    version: String,
    category: String,
    capabilities: String,
    icon_path: String,
    bundle_path: String,
}

// ── Build ───────────────────────────────────────────────────────────────────

/// Build the Apps settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::BG);

    // ── Page header ─────────────────────────────────────────────────────
    layout::build_page_header(&panel, "Apps", "Manage installed applications and permissions");

    // ── Scan apps ───────────────────────────────────────────────────────
    let apps = scan_apps();

    if apps.is_empty() {
        let empty = ui::Label::new("No applications found in /Applications/");
        empty.set_dock(ui::DOCK_TOP);
        empty.set_size(552, 30);
        empty.set_font_size(13);
        empty.set_text_color(0xFF969696);
        empty.set_margin(24, 8, 24, 0);
        panel.add(&empty);
    } else {
        // One card per app
        for app in &apps {
            build_app_row(&panel, app);
        }
    }

    parent.add(&panel);
    panel.id()
}

// ── Single app row ──────────────────────────────────────────────────────────

fn build_app_row(panel: &ui::View, app: &AppInfo) {
    let card = layout::build_auto_card(panel);

    // Main row container
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(552, 48);
    row.set_margin(16, 8, 16, 8);

    // ── Right-aligned buttons (added first so DOCK_RIGHT reserves space) ──

    // Uninstall button (rightmost)
    let btn_uninstall = ui::IconButton::new("Uninstall");
    btn_uninstall.set_dock(ui::DOCK_RIGHT);
    btn_uninstall.set_size(90, 28);
    btn_uninstall.set_margin(4, 10, 0, 10);
    let uninstall_name = app.name.clone();
    let uninstall_bundle = app.bundle_path.clone();
    let uninstall_id = app.id.clone();
    btn_uninstall.on_click(move |_| {
        let msg = format!("Uninstall {}?", uninstall_name);
        ui::MessageBox::show(
            ui::MessageBoxType::Warning,
            &msg,
            Some("Uninstall"),
        );
        uninstall_app(&uninstall_bundle);
        permissions::perm_delete(&uninstall_id);
    });
    row.add(&btn_uninstall);

    // Details button (next from right)
    let btn_details = ui::IconButton::new("Details");
    btn_details.set_dock(ui::DOCK_RIGHT);
    btn_details.set_size(70, 28);
    btn_details.set_margin(4, 10, 4, 10);
    let detail_name = app.name.clone();
    let detail_id = app.id.clone();
    let detail_version = app.version.clone();
    let detail_category = app.category.clone();
    let detail_caps = app.capabilities.clone();
    btn_details.on_click(move |_| {
        open_details_window(
            &detail_name,
            &detail_id,
            &detail_version,
            &detail_category,
            &detail_caps,
        );
    });
    row.add(&btn_details);

    // ── Left-side content ────────────────────────────────────────────────

    // Icon (32x32 ImageView loaded from bundle)
    let icon_view = ui::ImageView::new(32, 32);
    icon_view.set_dock(ui::DOCK_LEFT);
    icon_view.set_size(32, 32);
    icon_view.set_margin(0, 8, 8, 8);
    if !app.icon_path.is_empty() {
        icon_view.load_ico(&app.icon_path, 32);
    }
    row.add(&icon_view);

    // Info area (name + version stacked vertically)
    let info = ui::View::new();
    info.set_dock(ui::DOCK_FILL);

    let name_lbl = ui::Label::new(&app.name);
    name_lbl.set_dock(ui::DOCK_TOP);
    name_lbl.set_size(200, 20);
    name_lbl.set_font_size(14);
    name_lbl.set_text_color(0xFFFFFFFF);
    name_lbl.set_margin(0, 6, 0, 0);
    info.add(&name_lbl);

    let sub_text = if app.category.is_empty() {
        format!("v{}", app.version)
    } else {
        format!("v{} — {}", app.version, app.category)
    };
    let sub_lbl = ui::Label::new(&sub_text);
    sub_lbl.set_dock(ui::DOCK_TOP);
    sub_lbl.set_size(200, 16);
    sub_lbl.set_font_size(11);
    sub_lbl.set_text_color(0xFF808080);
    sub_lbl.set_margin(0, 2, 0, 0);
    info.add(&sub_lbl);

    row.add(&info);

    card.add(&row);
}

// ── Details window (permissions) ────────────────────────────────────────────

fn open_details_window(
    name: &str,
    app_id: &str,
    version: &str,
    category: &str,
    _capabilities: &str,
) {
    let title = format!("App Details — {}", name);
    let win = ui::Window::new(&title, -1, -1, 400, 450);
    let win_id = win.id();

    let root = ui::View::new();
    root.set_dock(ui::DOCK_FILL);
    root.set_color(0xFF1E1E1E);

    // ── App info section ────────────────────────────────────────────────
    let info_card = ui::Card::new();
    info_card.set_dock(ui::DOCK_TOP);
    info_card.set_size(368, 0);
    info_card.set_margin(16, 12, 16, 8);
    info_card.set_color(0xFF2D2D30);
    info_card.set_auto_size(true);

    build_detail_row(&info_card, "Name", name, true);
    build_detail_row(&info_card, "ID", app_id, false);
    build_detail_row(&info_card, "Version", version, false);
    if !category.is_empty() {
        build_detail_row(&info_card, "Category", category, false);
    }

    root.add(&info_card);

    // ── Permissions section ─────────────────────────────────────────────
    let perm_header = ui::Label::new("Permissions");
    perm_header.set_dock(ui::DOCK_TOP);
    perm_header.set_size(368, 28);
    perm_header.set_font_size(14);
    perm_header.set_text_color(0xFFFFFFFF);
    perm_header.set_margin(16, 8, 16, 0);
    root.add(&perm_header);

    let perm_card = ui::Card::new();
    perm_card.set_dock(ui::DOCK_TOP);
    perm_card.set_size(368, 0);
    perm_card.set_margin(16, 4, 16, 8);
    perm_card.set_color(0xFF2D2D30);
    perm_card.set_auto_size(true);

    let uid = process::getuid();
    let granted = permissions::perm_check(app_id, uid);
    // u32::MAX means no permission file exists — treat as all denied
    let current_granted = if granted == u32::MAX { 0u32 } else { granted };

    for (gi, group) in PERM_GROUPS.iter().enumerate() {
        let row = ui::View::new();
        row.set_dock(ui::DOCK_TOP);
        row.set_size(336, 40);
        row.set_margin(16, if gi == 0 { 8 } else { 0 }, 16, 0);

        let lbl = ui::Label::new(group.name);
        lbl.set_position(0, 10);
        lbl.set_size(200, 20);
        lbl.set_text_color(0xFFCCCCCC);
        lbl.set_font_size(13);
        row.add(&lbl);

        let is_on = (current_granted & group.mask) != 0;
        let toggle = ui::Toggle::new(is_on);
        toggle.set_position(300, 6);

        let mask = group.mask;
        let aid = String::from(app_id);
        let old_granted = current_granted;
        toggle.on_checked_changed(move |e| {
            let new_granted = if e.checked {
                old_granted | mask
            } else {
                old_granted & !mask
            };
            let uid = process::getuid();
            permissions::perm_store(&aid, new_granted, uid);
        });

        row.add(&toggle);
        perm_card.add(&row);
    }

    // Bottom padding inside perm card
    let pad = ui::View::new();
    pad.set_dock(ui::DOCK_TOP);
    pad.set_size(336, 8);
    perm_card.add(&pad);

    root.add(&perm_card);

    // ── Reset Permissions button ────────────────────────────────────────
    let reset_btn = ui::IconButton::new("Reset Permissions");
    reset_btn.set_dock(ui::DOCK_TOP);
    reset_btn.set_size(160, 32);
    reset_btn.set_margin(16, 4, 16, 8);
    let reset_id = String::from(app_id);
    let win_to_close = win_id;
    reset_btn.on_click(move |_| {
        permissions::perm_delete(&reset_id);
        // Close and reopen would be complex; just destroy the window
        destroy_window(win_to_close);
    });
    root.add(&reset_btn);

    win.add(&root);

    win.on_close(move |_| {
        destroy_window(win_id);
    });
}

fn build_detail_row(card: &ui::Card, label_text: &str, value_text: &str, first: bool) {
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(336, 32);
    row.set_margin(16, if first { 8 } else { 0 }, 16, 0);

    let lbl = ui::Label::new(label_text);
    lbl.set_position(0, 6);
    lbl.set_size(80, 20);
    lbl.set_text_color(0xFF808080);
    lbl.set_font_size(12);
    row.add(&lbl);

    let val = ui::Label::new(value_text);
    val.set_position(90, 6);
    val.set_size(240, 20);
    val.set_text_color(0xFFCCCCCC);
    val.set_font_size(12);
    row.add(&val);

    card.add(&row);
}

// ── App scanning ────────────────────────────────────────────────────────────

fn scan_apps() -> Vec<AppInfo> {
    let mut apps = Vec::new();

    let mut dir_buf = [0u8; 64 * 64];
    let count = fs::readdir(APPS_DIR, &mut dir_buf);
    if count == u32::MAX || count == 0 {
        return apps;
    }

    for i in 0..count as usize {
        if apps.len() >= MAX_APPS {
            break;
        }
        let raw = &dir_buf[i * 64..(i + 1) * 64];
        let entry_type = raw[0];
        let name_len = raw[1] as usize;

        // We want directories (type 1) ending in .app
        if entry_type != 1 || name_len == 0 {
            continue;
        }
        let nlen = name_len.min(56);
        let name = match core::str::from_utf8(&raw[8..8 + nlen]) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !name.ends_with(".app") {
            continue;
        }

        let bundle_path = format!("{}/{}", APPS_DIR, name);
        if let Some(info) = parse_app_bundle(&bundle_path) {
            apps.push(info);
        }
    }

    // Sort by name
    apps.sort_unstable_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
    apps
}

fn parse_app_bundle(bundle_path: &str) -> Option<AppInfo> {
    let conf_path = format!("{}/Info.conf", bundle_path);
    let icon_path = format!("{}/Icon.ico", bundle_path);

    // Read Info.conf
    let mut conf_buf = [0u8; 2048];
    let fd = fs::open(&conf_path, 0);
    if fd == u32::MAX {
        return None;
    }
    let bytes_read = fs::read(fd, &mut conf_buf) as usize;
    fs::close(fd);
    if bytes_read == 0 {
        return None;
    }

    let conf_str = core::str::from_utf8(&conf_buf[..bytes_read]).ok()?;

    let mut name = String::new();
    let mut id = String::new();
    let mut version = String::from("1.0");
    let mut category = String::new();
    let mut capabilities = String::new();
    let mut icon = String::new();

    for line in conf_str.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();
            match key {
                "name" => name = String::from(value),
                "id" => id = String::from(value),
                "version" => version = String::from(value),
                "category" => category = String::from(value),
                "capabilities" => capabilities = String::from(value),
                "icon" => icon = String::from(value),
                _ => {}
            }
        }
    }

    if name.is_empty() {
        // Fall back to bundle directory name minus .app suffix
        if let Some(slash) = bundle_path.rfind('/') {
            let dir_name = &bundle_path[slash + 1..];
            if let Some(dot) = dir_name.rfind(".app") {
                name = String::from(&dir_name[..dot]);
            }
        }
    }

    // Resolve icon path: if Info.conf specifies an icon file name, use it;
    // otherwise fall back to Icon.ico in the bundle root.
    let resolved_icon = if !icon.is_empty() {
        format!("{}/{}", bundle_path, icon)
    } else {
        icon_path
    };

    Some(AppInfo {
        name,
        id,
        version,
        category,
        capabilities,
        icon_path: resolved_icon,
        bundle_path: String::from(bundle_path),
    })
}

// ── Uninstall ───────────────────────────────────────────────────────────────

fn uninstall_app(bundle_path: &str) {
    // Simple recursive delete: list directory, unlink files, then recurse
    // into subdirectories. This OS uses a flat unlink for simple deletion.
    delete_recursive(bundle_path);
}

fn delete_recursive(path: &str) {
    let mut dir_buf = [0u8; 64 * 64];
    let count = fs::readdir(path, &mut dir_buf);
    if count == u32::MAX {
        // Not a directory or doesn't exist — try unlinking as a file
        fs::unlink(path);
        return;
    }

    for i in 0..count as usize {
        let raw = &dir_buf[i * 64..(i + 1) * 64];
        let entry_type = raw[0];
        let name_len = raw[1] as usize;
        if name_len == 0 {
            continue;
        }
        let nlen = name_len.min(56);
        let name = match core::str::from_utf8(&raw[8..8 + nlen]) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Skip . and ..
        if name == "." || name == ".." {
            continue;
        }

        let child = format!("{}/{}", path, name);
        if entry_type == 1 {
            // Directory — recurse
            delete_recursive(&child);
        } else {
            // File — unlink
            fs::unlink(&child);
        }
    }

    // Remove the now-empty directory itself
    fs::unlink(path);
}

// ── Window destroy helper ───────────────────────────────────────────────────

/// Reconstruct a `ui::Window` from its raw control ID and destroy it.
///
/// Window is `#[derive(Clone, Copy)]` and consists entirely of a single `u32`
/// field nested as `Window { Container { Control { id } } }`. The transmute is
/// therefore layout-safe and lets us call `destroy()` from event closures that
/// only captured the raw ID.
fn destroy_window(id: u32) {
    let win: ui::Window = unsafe { core::mem::transmute(id) };
    win.destroy();
}
