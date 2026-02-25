//! Built-in: About page (OS, Kernel, CPUs, Memory, Uptime).

use alloc::format;
use anyos_std::sys;
use libanyui_client as ui;
use ui::Widget;

/// Build the About settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_FILL);
    panel.set_color(0xFF1E1E1E);

    // Title
    let title = ui::Label::new("About");
    title.set_dock(ui::DOCK_TOP);
    title.set_size(560, 36);
    title.set_font_size(18);
    title.set_text_color(0xFFFFFFFF);
    title.set_margin(16, 12, 16, 4);
    panel.add(&title);

    // Info card
    let card = ui::Card::new();
    card.set_dock(ui::DOCK_TOP);
    card.set_size(560, 240);
    card.set_margin(16, 8, 16, 8);
    card.set_color(0xFF2D2D30);

    // OS
    build_row(&card, "OS", "anyOS 1.0", true);

    // Kernel
    build_row(&card, "Kernel", "x86_64-anyos", false);

    // CPUs
    let cpu_count = sys::sysinfo(2, &mut [0u8; 4]);
    let cpu_str = format!("{}", cpu_count);
    build_row(&card, "CPUs", &cpu_str, false);

    // Memory
    let mut mem = [0u8; 8];
    sys::sysinfo(0, &mut mem);
    let total = u32::from_le_bytes([mem[0], mem[1], mem[2], mem[3]]);
    let free = u32::from_le_bytes([mem[4], mem[5], mem[6], mem[7]]);
    let total_mb = (total * 4) / 1024;
    let free_mb = (free * 4) / 1024;
    let mem_str = format!("{} MB ({} MB free)", total_mb, free_mb);
    build_row(&card, "Memory", &mem_str, false);

    // Uptime
    let hz = sys::tick_hz().max(1);
    let secs = sys::uptime() / hz;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    let uptime_str = format!("{}h {}m {}s", h, m, s);
    build_row(&card, "Uptime", &uptime_str, false);

    panel.add(&card);
    parent.add(&panel);
    panel.id()
}

fn build_row(card: &ui::Card, label_text: &str, value_text: &str, first: bool) {
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(528, 40);
    row.set_margin(16, if first { 8 } else { 0 }, 16, 0);

    let lbl = ui::Label::new(label_text);
    lbl.set_position(0, 10);
    lbl.set_size(120, 20);
    lbl.set_text_color(0xFFCCCCCC);
    lbl.set_font_size(13);
    row.add(&lbl);

    let val = ui::Label::new(value_text);
    val.set_position(130, 10);
    val.set_size(380, 20);
    val.set_text_color(0xFF969696);
    val.set_font_size(13);
    row.add(&val);

    card.add(&row);
}
