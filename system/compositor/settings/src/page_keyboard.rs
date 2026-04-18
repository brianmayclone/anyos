// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Settings page: Keyboard Shortcuts — configure global keyboard shortcuts.
//!
//! Reads/writes the compositor shortcut registry blob via confd.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::i18n;
use libconf_schema::{default_string, manifest, RegistryScope, ServiceSchema};
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

const SHORTCUTS_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] =
    &[default_string("shortcuts/mappings_blob", "")];
const SHORTCUTS_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const SHORTCUTS_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "system/compositor",
    RegistryScope::System,
    1,
    &["shortcuts"],
    SHORTCUTS_DEFAULTS,
    SHORTCUTS_MIGRATIONS,
);

fn shortcuts_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("settings", &SHORTCUTS_MANIFEST)
}

// ── Data ───────────────────────────────────────────────────────────────────

struct ShortcutEntry {
    combo: String,
    action: String,
}

fn read_shortcuts() -> Vec<ShortcutEntry> {
    let _ = shortcuts_schema().register();
    let text = shortcuts_schema()
        .read_string("shortcuts/mappings_blob")
        .unwrap_or_default();

    let mut entries = Vec::new();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some(eq) = line.find('=') {
            let combo = line[..eq].trim();
            let action = line[eq + 1..].trim();
            if !combo.is_empty() && !action.is_empty() {
                entries.push(ShortcutEntry {
                    combo: String::from(combo),
                    action: String::from(action),
                });
            }
        }
    }
    entries
}

/// Send CMD_RELOAD_SHORTCUTS to the compositor so changes take effect immediately.
fn notify_compositor_reload() {
    let chan = ui::get_compositor_channel();
    let cmd: [u32; 5] = [0x1019, 0, 0, 0, 0]; // CMD_RELOAD_SHORTCUTS
    anyos_std::ipc::evt_chan_emit(chan, &cmd);
}

fn save_shortcuts(entries: &[ShortcutEntry]) {
    let _ = shortcuts_schema().register();
    let mut result = String::new();
    for e in entries {
        result.push_str(&e.combo);
        result.push('=');
        result.push_str(&e.action);
        result.push('\n');
    }
    let _ = shortcuts_schema().write_string("shortcuts/mappings_blob", result.trim_end());
}

// ── Built-in action display names ──────────────────────────────────────────

fn action_display(action: &str) -> &str {
    match action {
        "show_desktop" => "Show Desktop",
        "tile_windows" => "Tile Windows",
        "lock_screen" => "Lock Screen",
        _ => action,
    }
}

// ── System shortcuts (hardcoded in compositor) ─────────────────────────────

const SYSTEM_SHORTCUTS: &[(&str, &str)] = &[
    ("Alt+Tab",           "Switch to next window"),
    ("Alt+Shift+Tab",     "Switch to previous window"),
    ("Alt+F4",            "Close focused window"),
    ("Alt+Enter",         "Toggle fullscreen"),
    ("Ctrl+Alt+Delete",   "Exit fullscreen / System dialog"),
    ("Escape",            "Close menu / Exit fullscreen"),
    ("F10",               "Activate menu bar"),
    ("Super (Win)",       "Open system menu"),
    ("Tab (no focus)",    "Focus dock"),
    ("Volume Up/Down",    "Adjust system volume"),
];

fn build_system_shortcuts_card(panel: &ui::View) {
    let tc = ui::theme::colors();

    // Section label
    let section = ui::Label::new(i18n::t("System Shortcuts"));
    section.set_dock(ui::DOCK_TOP);
    section.set_size(500, 24);
    section.set_font_size(14);
    section.set_text_color(layout::text());
    section.set_margin(24, 8, 24, 0);
    panel.add(&section);

    let card = layout::build_auto_card(panel);

    for (i, &(combo, desc)) in SYSTEM_SHORTCUTS.iter().enumerate() {
        let row = layout::build_setting_row(&card, combo, i == 0);
        let val = ui::Label::new(i18n::t(desc));
        val.set_position(200, 12);
        val.set_size(340, 20);
        val.set_text_color(tc.text_secondary);
        val.set_font_size(13);
        row.add(&val);

        if i + 1 < SYSTEM_SHORTCUTS.len() {
            layout::build_separator(&card);
        }
    }
}

// ── Page builder ───────────────────────────────────────────────────────────

/// Build the Keyboard Shortcuts settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::bg());

    layout::build_page_header(
        &panel,
        i18n::t("Keyboard Shortcuts"),
        i18n::t("Configure global keyboard shortcuts"),
    );

    // ── System shortcuts (read-only) ─────────────────────────────────
    build_system_shortcuts_card(&panel);

    // ── Custom shortcuts ─────────────────────────────────────────────
    let custom_lbl = ui::Label::new(i18n::t("Custom Shortcuts"));
    custom_lbl.set_dock(ui::DOCK_TOP);
    custom_lbl.set_size(500, 24);
    custom_lbl.set_font_size(14);
    custom_lbl.set_text_color(layout::text());
    custom_lbl.set_margin(24, 16, 24, 0);
    panel.add(&custom_lbl);

    let entries = read_shortcuts();

    if entries.is_empty() {
        let card = layout::build_auto_card(&panel);
        let lbl = ui::Label::new(i18n::t("No keyboard shortcuts configured."));
        lbl.set_dock(ui::DOCK_TOP);
        lbl.set_size(500, 24);
        lbl.set_font_size(13);
        lbl.set_text_color(layout::text_dim());
        lbl.set_margin(24, 16, 24, 8);
        card.add(&lbl);
    } else {
        let card = layout::build_auto_card(&panel);

        for (i, entry) in entries.iter().enumerate() {
            let row = layout::build_setting_row(&card, &entry.combo, i == 0);
            let val = ui::Label::new(action_display(&entry.action));
            val.set_position(200, 12);
            val.set_size(260, 20);
            val.set_text_color(layout::text_dim());
            val.set_font_size(13);
            row.add(&val);

            // Delete button
            let del_btn = ui::Button::new("\u{00D7}"); // × character
            del_btn.set_position(480, 8);
            del_btn.set_size(28, 28);
            row.add(&del_btn);

            del_btn.on_click_raw(delete_shortcut_handler, i as u64);

            if i + 1 < entries.len() {
                layout::build_separator(&card);
            }
        }
    }

    // ── Add new shortcut ─────────────────────────────────────────────
    let add_card = layout::build_auto_card(&panel);

    let section_lbl = ui::Label::new(i18n::t("Add Shortcut"));
    section_lbl.set_dock(ui::DOCK_TOP);
    section_lbl.set_size(500, 24);
    section_lbl.set_font_size(14);
    section_lbl.set_text_color(layout::text());
    section_lbl.set_margin(24, 12, 24, 4);
    add_card.add(&section_lbl);

    // Key combination input
    let combo_row = ui::View::new();
    combo_row.set_dock(ui::DOCK_TOP);
    combo_row.set_size(552, 40);
    combo_row.set_margin(24, 4, 24, 0);

    let combo_lbl = ui::Label::new(i18n::t("Key Combination"));
    combo_lbl.set_position(0, 10);
    combo_lbl.set_size(140, 20);
    combo_lbl.set_text_color(layout::text());
    combo_lbl.set_font_size(13);
    combo_row.add(&combo_lbl);

    let combo_input = ui::TextField::new();
    combo_input.set_position(150, 4);
    combo_input.set_size(200, 28);
    combo_input.set_placeholder("Ctrl+Alt+T");
    combo_row.add(&combo_input);
    let combo_id = combo_input.id();

    add_card.add(&combo_row);

    // Action input
    let action_row = ui::View::new();
    action_row.set_dock(ui::DOCK_TOP);
    action_row.set_size(552, 40);
    action_row.set_margin(24, 4, 24, 0);

    let action_lbl = ui::Label::new(i18n::t("Action"));
    action_lbl.set_position(0, 10);
    action_lbl.set_size(140, 20);
    action_lbl.set_text_color(layout::text());
    action_lbl.set_font_size(13);
    action_row.add(&action_lbl);

    let action_input = ui::TextField::new();
    action_input.set_position(150, 4);
    action_input.set_size(200, 28);
    action_input.set_placeholder("/Applications/Terminal.app");
    action_row.add(&action_input);
    let action_id = action_input.id();

    add_card.add(&action_row);

    // Hint text
    let hint = ui::Label::new(i18n::t("Built-in: show_desktop, tile_windows, lock_screen"));
    hint.set_dock(ui::DOCK_TOP);
    hint.set_size(500, 18);
    hint.set_font_size(11);
    hint.set_text_color(layout::text_dim());
    hint.set_margin(24, 2, 24, 0);
    add_card.add(&hint);

    // Add button
    let add_btn = ui::Button::new(i18n::t("Add Shortcut"));
    add_btn.set_dock(ui::DOCK_TOP);
    add_btn.set_size(140, 32);
    add_btn.set_margin(24, 8, 24, 16);
    add_card.add(&add_btn);

    // Pack both IDs into one u64 for the callback
    let packed: u64 = (combo_id as u64) | ((action_id as u64) << 32);
    add_btn.on_click_raw(add_shortcut_handler, packed);

    // ── Hint card ────────────────────────────────────────────────────
    let hint_card = layout::build_auto_card(&panel);
    let note = ui::Label::new(i18n::t("Changes take effect immediately."));
    note.set_dock(ui::DOCK_TOP);
    note.set_size(500, 20);
    note.set_font_size(12);
    note.set_text_color(layout::text_dim());
    note.set_margin(24, 12, 24, 12);
    hint_card.add(&note);

    parent.add(&panel);
    panel.id()
}

// ── Callbacks ──────────────────────────────────────────────────────────────

extern "C" fn delete_shortcut_handler(_id: u32, _evt: u32, index: u64) {
    let idx = index as usize;
    let mut entries = read_shortcuts();
    if idx < entries.len() {
        entries.remove(idx);
        save_shortcuts(&entries);
        notify_compositor_reload();
        crate::invalidate_all_pages();
    }
}

extern "C" fn add_shortcut_handler(_id: u32, _evt: u32, packed: u64) {
    let combo_id = (packed & 0xFFFFFFFF) as u32;
    let action_id = (packed >> 32) as u32;

    let combo_ctrl = ui::Control::from_id(combo_id);
    let action_ctrl = ui::Control::from_id(action_id);

    let mut combo_buf = [0u8; 64];
    let combo_len = combo_ctrl.get_text(&mut combo_buf);
    let combo = core::str::from_utf8(&combo_buf[..combo_len as usize]).unwrap_or("").trim();

    let mut action_buf = [0u8; 256];
    let action_len = action_ctrl.get_text(&mut action_buf);
    let action = core::str::from_utf8(&action_buf[..action_len as usize]).unwrap_or("").trim();

    if combo.is_empty() || action.is_empty() {
        return;
    }

    let mut entries = read_shortcuts();
    entries.push(ShortcutEntry {
        combo: String::from(combo),
        action: String::from(action),
    });
    save_shortcuts(&entries);
    notify_compositor_reload();
    crate::invalidate_all_pages();
}
