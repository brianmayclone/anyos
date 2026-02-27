//! Dashboard page — Windows 11 Settings Home-style overview.
//!
//! Shows a hero card with computer identity, quick-access navigation tiles,
//! and system information cards (CPU, memory, GPU, network, uptime).

use alloc::format;
use alloc::string::String;
use anyos_std::{net, process, sys};
use anyos_std::ui::window;
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── Public entry point ──────────────────────────────────────────────────────

/// Build the Dashboard panel inside `parent`. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::BG);

    layout::build_page_header(&panel, "Dashboard", "");

    build_hero_section(&panel);
    build_quick_access(&panel);
    build_system_info(&panel);

    parent.add(&panel);
    panel.id()
}

// ── Hero section ────────────────────────────────────────────────────────────

fn build_hero_section(panel: &ui::View) {
    let card = layout::build_auto_card(panel);

    // Hostname
    let mut host_buf = [0u8; 64];
    let hlen = sys::get_hostname(&mut host_buf);
    let hostname = if hlen != u32::MAX && hlen > 0 {
        core::str::from_utf8(&host_buf[..hlen as usize]).unwrap_or("anyOS Computer")
    } else {
        "anyOS Computer"
    };

    let name_lbl = ui::Label::new(hostname);
    name_lbl.set_dock(ui::DOCK_TOP);
    name_lbl.set_size(350, 28);
    name_lbl.set_font_size(18);
    name_lbl.set_text_color(0xFFFFFFFF);
    name_lbl.set_margin(24, 16, 24, 0);
    card.add(&name_lbl);

    // OS version subtitle
    let os_lbl = ui::Label::new("anyOS 1.0");
    os_lbl.set_dock(ui::DOCK_TOP);
    os_lbl.set_size(200, 18);
    os_lbl.set_font_size(12);
    os_lbl.set_text_color(0xFF808080);
    os_lbl.set_margin(24, 2, 24, 0);
    card.add(&os_lbl);

    // Username
    let uid = process::getuid();
    let mut name_buf = [0u8; 64];
    let nlen = process::getusername(uid, &mut name_buf);
    let username = if nlen != u32::MAX && nlen > 0 {
        core::str::from_utf8(&name_buf[..nlen as usize]).unwrap_or("user")
    } else {
        "user"
    };
    let user_text = format!("Signed in as {}", username);
    let user_lbl = ui::Label::new(&user_text);
    user_lbl.set_dock(ui::DOCK_TOP);
    user_lbl.set_size(350, 18);
    user_lbl.set_font_size(12);
    user_lbl.set_text_color(layout::TEXT_DIM);
    user_lbl.set_margin(24, 4, 24, 12);
    card.add(&user_lbl);
}

// ── Quick access tiles ──────────────────────────────────────────────────────

fn build_quick_access(panel: &ui::View) {
    // Section label
    let section_lbl = ui::Label::new("Quick access");
    section_lbl.set_dock(ui::DOCK_TOP);
    section_lbl.set_size(600, 24);
    section_lbl.set_font_size(13);
    section_lbl.set_text_color(layout::TEXT);
    section_lbl.set_margin(24, 12, 24, 0);
    panel.add(&section_lbl);

    let flow = ui::FlowPanel::new();
    flow.set_dock(ui::DOCK_TOP);
    flow.set_size(600, 96);
    flow.set_margin(20, 4, 20, 4);

    static TILES: &[&str] = &["Display", "Network", "Apps", "Devices"];
    for &label in TILES {
        let tile = ui::Card::new();
        tile.set_size(130, 80);
        tile.set_margin(4, 4, 4, 4);
        tile.set_color(layout::CARD_BG);

        let lbl = ui::Label::new(label);
        lbl.set_position(12, 30);
        lbl.set_size(106, 20);
        lbl.set_font_size(13);
        lbl.set_text_color(layout::TEXT);
        tile.add(&lbl);

        flow.add(&tile);
    }

    panel.add(&flow);
}

// ── System information cards ────────────────────────────────────────────────

fn build_system_info(panel: &ui::View) {
    // Section label
    let section_lbl = ui::Label::new("System information");
    section_lbl.set_dock(ui::DOCK_TOP);
    section_lbl.set_size(600, 24);
    section_lbl.set_font_size(13);
    section_lbl.set_text_color(layout::TEXT);
    section_lbl.set_margin(24, 12, 24, 0);
    panel.add(&section_lbl);

    // Use a flow panel for info tiles
    let flow = ui::FlowPanel::new();
    flow.set_dock(ui::DOCK_TOP);
    flow.set_size(600, 210);
    flow.set_margin(20, 4, 20, 8);

    // CPU card
    let cpu_count = sys::sysinfo(2, &mut [0u8; 4]);
    let cpu_str = format!("{} cores", cpu_count);
    build_info_tile(&flow, "CPU", &cpu_str);

    // Memory card
    let mut mem = [0u8; 8];
    sys::sysinfo(0, &mut mem);
    let total = u32::from_le_bytes([mem[0], mem[1], mem[2], mem[3]]);
    let free = u32::from_le_bytes([mem[4], mem[5], mem[6], mem[7]]);
    let total_mb = (total * 4) / 1024;
    let free_mb = (free * 4) / 1024;
    let used_mb = total_mb.saturating_sub(free_mb);
    let mem_str = format!("{} / {} MB", used_mb, total_mb);
    build_info_tile(&flow, "Memory", &mem_str);

    // GPU card
    let gpu_name = window::gpu_name();
    let accel = window::gpu_has_accel();
    let gpu_str = if accel {
        format!("{} (HW)", gpu_name)
    } else {
        gpu_name
    };
    build_info_tile(&flow, "GPU", &gpu_str);

    // Network card
    let mut net_buf = [0u8; 24];
    net::get_config(&mut net_buf);
    let link_up = net_buf[22] != 0;
    let net_str = if link_up {
        let ip = [net_buf[0], net_buf[1], net_buf[2], net_buf[3]];
        format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
    } else {
        String::from("Disconnected")
    };
    build_info_tile_colored(
        &flow,
        "Network",
        &net_str,
        if link_up { 0xFF4EC970 } else { 0xFFE06C75 },
    );

    // Uptime card
    let hz = sys::tick_hz().max(1);
    let secs = sys::uptime() / hz;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    let uptime_str = format!("{}h {}m {}s", h, m, s);
    build_info_tile(&flow, "Uptime", &uptime_str);

    panel.add(&flow);
}

// ── Info tile helper ────────────────────────────────────────────────────────

fn build_info_tile(flow: &ui::FlowPanel, title: &str, value: &str) {
    build_info_tile_colored(flow, title, value, layout::TEXT_DIM);
}

fn build_info_tile_colored(flow: &ui::FlowPanel, title: &str, value: &str, color: u32) {
    let tile = ui::Card::new();
    tile.set_size(170, 88);
    tile.set_margin(4, 4, 4, 4);
    tile.set_color(layout::CARD_BG);

    let title_lbl = ui::Label::new(title);
    title_lbl.set_position(14, 12);
    title_lbl.set_size(142, 20);
    title_lbl.set_font_size(11);
    title_lbl.set_text_color(0xFF808080);
    tile.add(&title_lbl);

    let val_lbl = ui::Label::new(value);
    val_lbl.set_position(14, 36);
    val_lbl.set_size(142, 40);
    val_lbl.set_font_size(13);
    val_lbl.set_text_color(color);
    tile.add(&val_lbl);

    flow.add(&tile);
}
