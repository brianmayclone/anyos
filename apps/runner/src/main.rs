// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Runner — quick application launcher (Alt+R).
//!
//! Scans /Applications/ for .app bundles and presents an autocomplete
//! search field with icons. Type to filter, press Enter to launch.

#![no_std]
#![no_main]

use anyos_std::{String, Vec};
use libanyui_client as ui;
use ui::Widget;

anyos_std::entry!(main);

struct AppEntry {
    name: String,
    path: String,
    icon_path: String,
}

fn scan_apps() -> Vec<AppEntry> {
    let mut apps = Vec::new();
    let mut buf = [0u8; 128 * 64];
    let count = anyos_std::fs::readdir("/Applications", &mut buf);
    if count == u32::MAX { return apps; }
    for i in 0..count as usize {
        let off = i * 64;
        if buf[off] != 1 { continue; }
        let name_len = (buf[off + 1] as usize).min(55);
        let name_str = match core::str::from_utf8(&buf[off + 8..off + 8 + name_len]) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !name_str.ends_with(".app") { continue; }

        let mut path = String::from("/Applications/");
        path.push_str(name_str);

        let display_name = read_app_name(&path, name_str);

        let mut icon = String::from(&path);
        icon.push_str("/Icon.ico");

        apps.push(AppEntry { name: display_name, path, icon_path: icon });
    }
    apps.sort_by(|a, b| {
        let la = a.name.as_bytes();
        let lb = b.name.as_bytes();
        for i in 0..la.len().min(lb.len()) {
            let ca = la[i].to_ascii_lowercase();
            let cb = lb[i].to_ascii_lowercase();
            if ca != cb { return ca.cmp(&cb); }
        }
        la.len().cmp(&lb.len())
    });
    apps
}

fn read_app_name(bundle_path: &str, folder_name: &str) -> String {
    let mut conf = String::from(bundle_path);
    conf.push_str("/Info.conf");
    let fd = anyos_std::fs::open(&conf, 0);
    if fd != u32::MAX {
        let mut fbuf = [0u8; 512];
        let n = anyos_std::fs::read(fd, &mut fbuf);
        anyos_std::fs::close(fd);
        if n > 0 && n != u32::MAX {
            if let Ok(text) = core::str::from_utf8(&fbuf[..n as usize]) {
                for line in text.split('\n') {
                    let line = line.trim();
                    if let Some(rest) = line.strip_prefix("name=") {
                        let rest = rest.trim();
                        if !rest.is_empty() {
                            return String::from(rest);
                        }
                    }
                }
            }
        }
    }
    let base = &folder_name[..folder_name.len().saturating_sub(4)];
    String::from(base)
}

// ── State ──────────────────────────────────────────────────────────────────

struct RunnerState {
    apps: Vec<AppEntry>,
    field_id: u32,
}

static mut STATE: Option<RunnerState> = None;
fn state() -> &'static mut RunnerState { unsafe { STATE.as_mut().unwrap() } }

fn main() {
    if !ui::init() { return; }

    let apps = scan_apps();

    // Build suggestion string: "icon_path\x1Fname|icon_path\x1Fname|..."
    let mut suggestions = String::new();
    for (i, app) in apps.iter().enumerate() {
        if i > 0 { suggestions.push('|'); }
        suggestions.push_str(&app.icon_path);
        suggestions.push('\x1F');
        suggestions.push_str(&app.name);
    }

    let win = ui::Window::new_with_flags(
        "Runner",
        -1, -1,
        420, 44,
        ui::WIN_FLAG_NOT_RESIZABLE | ui::WIN_FLAG_NO_MINIMIZE | ui::WIN_FLAG_NO_MAXIMIZE,
    );

    let field = ui::AutoCompleteTextField::new();
    field.set_dock(ui::DOCK_FILL);
    field.set_placeholder("Anwendung suchen...");
    win.add(&field);
    field.set_suggestions(&suggestions);

    let field_id = field.id();
    unsafe { STATE = Some(RunnerState { apps, field_id }); }

    field.on_submit(|_| {
        let s = state();
        let ctrl = ui::Control::from_id(s.field_id);
        let mut buf = [0u8; 256];
        let len = ctrl.get_text(&mut buf) as usize;
        let text = core::str::from_utf8(&buf[..len.min(256)]).unwrap_or("");
        // Find exact match first, then case-insensitive prefix
        let found = s.apps.iter().find(|a| a.name == text)
            .or_else(|| {
                let qb = text.as_bytes();
                s.apps.iter().find(|a| {
                    let nb = a.name.as_bytes();
                    if nb.len() < qb.len() { return false; }
                    nb[..qb.len()].iter().zip(qb.iter())
                        .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
                })
            });
        if let Some(app) = found {
            let path = app.path.clone();
            ui::quit();
            anyos_std::process::launch_app(&path, "");
        }
    });

    win.on_key_down(|e| {
        if e.keycode == 0x103 { ui::quit(); } // KEY_ESCAPE
    });

    win.on_close(|_| { ui::quit(); });
    field.focus();

    ui::run();
}
