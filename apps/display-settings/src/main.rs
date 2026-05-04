//! display-settings — Multi-monitor configuration GUI for anyOS.
//!
//! Minimal first-cut: lists all advertised display outputs, shows the
//! current mode + EDID metadata for each, and exposes an "Apply default
//! layout" button that asks `displayd` to re-derive a left-to-right
//! layout from `display.conf` (or the default fallback).
//!
//! Drag-arrange and per-output mode picker are deferred to a richer
//! revision — they require a custom canvas widget and a fair bit of
//! event plumbing. The first version intentionally focuses on:
//!   * proving the displayd IPC round-trip works end-to-end
//!   * giving the user a way to refresh the layout after a hot-plug
//!   * surfacing the EDID-derived data so the user can tell which
//!     output is which physically

#![no_std]
#![no_main]

use anyos_std::{display, format, String};
use libanyui_client as ui;
use libdisplay_client as displayd;

anyos_std::entry!(main);

const WIN_W: u32 = 720;
const WIN_H: u32 = 520;

const COL_BG: u32 = 0xFF1E1E1E;
const COL_PANEL_BG: u32 = 0xFF2A2A2C;
const COL_TEXT: u32 = 0xFFE6E6E6;
const COL_TEXT_DIM: u32 = 0xFF9A9A9A;

fn fmt_pnpid(info: &display::DisplayInfo) -> String {
    let bytes = info.manufacturer.to_le_bytes();
    let mut s = String::new();
    for &b in &bytes[..3] {
        if b.is_ascii_uppercase() || b.is_ascii_alphanumeric() {
            s.push(b as char);
        }
    }
    if s.is_empty() {
        s.push_str("???");
    }
    s
}

/// Integer sqrt — Newton's method, fixed iterations. Good enough for
/// the diagonal-display estimate we only show as a tooltip-style hint.
fn isqrt_u64(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

fn fmt_physical(info: &display::DisplayInfo) -> String {
    let (w, h) = info.physical_mm_pair();
    if w == 0 && h == 0 {
        return String::from("size: unknown");
    }
    // Diagonal in inches as a quick reference (standard EDID convention).
    // Compute d_mm = sqrt(w² + h²) using integer arithmetic, then
    // convert to tenths of an inch (1 inch = 25.4 mm; *10 / 254).
    let d_mm = isqrt_u64((w as u64) * (w as u64) + (h as u64) * (h as u64));
    let d_in_x10 = (d_mm * 100) / 254;
    format!(
        "{}x{}mm (~{}.{}\")",
        w,
        h,
        d_in_x10 / 10,
        d_in_x10 % 10
    )
}

fn refresh_label(info: &display::DisplayInfo) -> String {
    if info.refresh_mhz == 0 {
        return String::from("- Hz");
    }
    let hz_x10 = (info.refresh_mhz + 50) / 100;
    format!("{}.{} Hz", hz_x10 / 10, hz_x10 % 10)
}

fn render_output_panel(parent: &ui::View, info: &display::DisplayInfo, idx: u32) {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_size(WIN_W - 40, 88);
    panel.set_color(COL_PANEL_BG);
    panel.set_padding(12, 12, 12, 12);
    parent.add(&panel);

    let title_text = format!(
        "Output {}{}: {} ({})",
        info.id,
        if info.is_primary() { " (primary)" } else { "" },
        fmt_pnpid(info),
        if info.is_connected() {
            "connected"
        } else {
            "disconnected"
        }
    );
    let title = ui::Label::new(&title_text);
    title.set_position(0, 0);
    title.set_size(WIN_W - 64, 22);
    title.set_text_color(COL_TEXT);
    panel.add(&title);

    let mode_text = format!(
        "Current: {}x{} @ {}    Preferred: {}x{}",
        info.current_w,
        info.current_h,
        refresh_label(info),
        info.preferred_w,
        info.preferred_h,
    );
    let mode = ui::Label::new(&mode_text);
    mode.set_position(0, 26);
    mode.set_size(WIN_W - 64, 20);
    mode.set_text_color(COL_TEXT_DIM);
    panel.add(&mode);

    let phys = ui::Label::new(&fmt_physical(info));
    phys.set_position(0, 48);
    phys.set_size(WIN_W - 64, 20);
    phys.set_text_color(COL_TEXT_DIM);
    panel.add(&phys);

    let edid_text = format!("EDID-hash: {:#018x}", info.edid_hash);
    let edid = ui::Label::new(&edid_text);
    edid.set_position(0, 68);
    edid.set_size(WIN_W - 64, 16);
    edid.set_text_color(COL_TEXT_DIM);
    panel.add(&edid);

    let _ = idx;
}

fn main() {
    if !ui::init() {
        return;
    }

    let win = ui::Window::new("Displays", -1, -1, WIN_W, WIN_H);

    // Toolbar with the only button: "Apply default layout".
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(WIN_W, 36);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(8, 8, 8, 8);
    let apply_btn = toolbar.add_icon_button("Apply default layout");
    apply_btn.set_size(190, 28);
    let probe_btn = toolbar.add_icon_button("Re-detect");
    probe_btn.set_size(110, 28);
    win.add(&toolbar);

    // Status bar at the bottom.
    let status_bar = ui::View::new();
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 24);
    status_bar.set_color(0xFF252526);
    let status_label = ui::Label::new("Ready.");
    status_label.set_position(8, 4);
    status_label.set_size(WIN_W - 16, 16);
    status_label.set_text_color(COL_TEXT_DIM);
    status_bar.add(&status_label);
    win.add(&status_bar);

    // Scrollable content area.
    let content = ui::View::new();
    content.set_dock(ui::DOCK_FILL);
    content.set_color(COL_BG);
    content.set_padding(20, 20, 20, 20);
    win.add(&content);

    let infos = display::list(16);
    if infos.is_empty() {
        let no_outputs = ui::Label::new(
            "No display outputs reported by the kernel. (SYS_DISPLAY_LIST returned 0.)",
        );
        no_outputs.set_position(0, 0);
        no_outputs.set_size(WIN_W - 40, 24);
        no_outputs.set_text_color(COL_TEXT_DIM);
        content.add(&no_outputs);
    } else {
        for (idx, info) in infos.iter().enumerate() {
            render_output_panel(&content, info, idx as u32);
        }
    }

    apply_btn.on_click(move |_| {
        if let Some(client) = displayd::DisplaydClient::connect() {
            let r = client.reapply_layout().unwrap_or(u32::MAX);
            client.disconnect();
            if r == 0 {
                anyos_std::println!("[display-settings] layout re-applied");
            } else {
                anyos_std::println!(
                    "[display-settings] layout apply failed (code={})",
                    r
                );
            }
        } else {
            anyos_std::println!("[display-settings] could not connect to displayd");
        }
    });

    probe_btn.on_click(move |_| {
        if let Some(client) = displayd::DisplaydClient::connect() {
            let _ = client.probe_hotplug();
            client.disconnect();
            anyos_std::println!("[display-settings] hotplug probe sent");
        }
    });

    win.on_close(|_| {
        ui::quit();
    });

    ui::run();
}
