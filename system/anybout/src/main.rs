#![no_std]
#![no_main]

use alloc::format;
use libanyui_client as ui;

anyos_std::entry!(main);

const WIN_W: u32 = 380;
const WIN_H: u32 = 440;

/// Query CPU count via sysinfo(2).
fn cpu_count() -> u32 {
    let mut buf = [0u8; 4];
    anyos_std::sys::sysinfo(2, &mut buf);
    u32::from_le_bytes(buf)
}

/// Query total memory in bytes via sysinfo(0).
fn memory_bytes() -> u32 {
    let mut buf = [0u8; 4];
    anyos_std::sys::sysinfo(0, &mut buf);
    u32::from_le_bytes(buf)
}

/// Format bytes as a human-readable string (e.g. "128 MB").
fn format_memory(bytes: u32) -> alloc::string::String {
    let mb = bytes / (1024 * 1024);
    if mb >= 1024 {
        let gb = mb / 1024;
        let remainder = (mb % 1024) / 100;
        if remainder > 0 {
            format!("{}.{} GB", gb, remainder)
        } else {
            format!("{} GB", gb)
        }
    } else {
        format!("{} MB", mb)
    }
}

fn main() {
    if !ui::init() {
        return;
    }

    let tc = ui::theme::colors();
    let flags = ui::WIN_FLAG_NOT_RESIZABLE | ui::WIN_FLAG_NO_MINIMIZE | ui::WIN_FLAG_NO_MAXIMIZE;
    let win = ui::Window::new_with_flags("About anyOS", -1, -1, WIN_W, WIN_H, flags);

    // ── Root view ────────────────────────────────────────────────────
    let root = ui::View::new();
    root.set_dock(ui::DOCK_FILL);
    root.set_color(tc.window_bg);
    win.add(&root);

    // ── OS name (.anyOS) ─────────────────────────────────────────────
    let name_label = ui::Label::new(".anyOS");
    name_label.set_dock(ui::DOCK_TOP);
    name_label.set_size(WIN_W, 40);
    name_label.set_margin(0, 28, 0, 0);
    name_label.set_font(1); // SF Pro Bold
    name_label.set_font_size(28);
    name_label.set_text_color(tc.accent);
    name_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    root.add(&name_label);

    // ── Version ──────────────────────────────────────────────────────
    let version_str = format!("Version {}", env!("ANYOS_VERSION"));
    let version_label = ui::Label::new(&version_str);
    version_label.set_dock(ui::DOCK_TOP);
    version_label.set_size(WIN_W, 20);
    version_label.set_margin(0, 4, 0, 0);
    version_label.set_font_size(13);
    version_label.set_text_color(tc.text_secondary);
    version_label.set_text_align(ui::TEXT_ALIGN_CENTER);
    root.add(&version_label);

    // ── Divider ──────────────────────────────────────────────────────
    let divider = ui::Divider::new();
    divider.set_dock(ui::DOCK_TOP);
    divider.set_size(WIN_W, 1);
    divider.set_margin(24, 16, 24, 16);
    root.add(&divider);

    // ── System info card ─────────────────────────────────────────────
    let card = ui::Card::new();
    card.set_dock(ui::DOCK_TOP);
    card.set_size(WIN_W, 140);
    card.set_margin(24, 0, 24, 0);
    root.add(&card);

    // Info rows inside the card
    add_info_row(&card, "Kernel", "anyOS Kernel", 0, tc.text, tc.text_secondary);
    add_info_row_divider(&card, 35);
    add_info_row(&card, "Architecture", "x86_64", 36, tc.text, tc.text_secondary);
    add_info_row_divider(&card, 71);

    let cpus = format!("{}", cpu_count());
    add_info_row(&card, "CPUs", &cpus, 72, tc.text, tc.text_secondary);
    add_info_row_divider(&card, 107);

    let mem = format_memory(memory_bytes());
    add_info_row(&card, "Memory", &mem, 108, tc.text, tc.text_secondary);

    // ── Copyright ────────────────────────────────────────────────────
    let copyright1 = ui::Label::new("Copyright 2024-2026");
    copyright1.set_dock(ui::DOCK_TOP);
    copyright1.set_size(WIN_W, 16);
    copyright1.set_margin(0, 20, 0, 0);
    copyright1.set_font_size(11);
    copyright1.set_text_color(tc.text_disabled);
    copyright1.set_text_align(ui::TEXT_ALIGN_CENTER);
    root.add(&copyright1);

    let copyright2 = ui::Label::new("Christian Moeller & Maik Stratmann");
    copyright2.set_dock(ui::DOCK_TOP);
    copyright2.set_size(WIN_W, 16);
    copyright2.set_margin(0, 2, 0, 0);
    copyright2.set_font_size(11);
    copyright2.set_text_color(tc.text_disabled);
    copyright2.set_text_align(ui::TEXT_ALIGN_CENTER);
    root.add(&copyright2);

    // ── OK button ────────────────────────────────────────────────────
    let btn_row = ui::View::new();
    btn_row.set_dock(ui::DOCK_TOP);
    btn_row.set_size(WIN_W, 50);
    btn_row.set_margin(0, 10, 0, 0);

    let btn = ui::Button::new("OK");
    btn.set_position((WIN_W / 2 - 40) as i32, 10);
    btn.set_size(80, 30);
    btn.on_click(|_| {
        ui::quit();
    });
    btn_row.add(&btn);
    root.add(&btn_row);

    // ── Keyboard: ESC to close ───────────────────────────────────────
    win.on_key_down(|e| {
        if e.keycode == ui::KEY_ESCAPE {
            ui::quit();
        }
    });

    win.on_close(|_| {
        ui::quit();
    });

    ui::run();
}

/// Add a label-value row inside the card at absolute y position.
fn add_info_row(
    card: &ui::Card,
    label_text: &str,
    value_text: &str,
    y: i32,
    label_color: u32,
    value_color: u32,
) {
    let label = ui::Label::new(label_text);
    label.set_position(16, y + 8);
    label.set_size(120, 20);
    label.set_font_size(13);
    label.set_text_color(label_color);
    card.add(&label);

    let value = ui::Label::new(value_text);
    value.set_position(140, y + 8);
    value.set_size(180, 20);
    value.set_font_size(13);
    value.set_text_color(value_color);
    value.set_text_align(ui::TEXT_ALIGN_RIGHT);
    card.add(&value);
}

/// Add a thin divider inside the card at absolute y position.
fn add_info_row_divider(card: &ui::Card, y: i32) {
    let div = ui::Divider::new();
    div.set_position(12, y);
    div.set_size(308, 1);
    card.add(&div);
}
