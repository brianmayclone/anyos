//! General system settings page.
//!
//! Provides cards for computer info, hostname editing, user identity,
//! UI preferences (dark mode, sound, notifications), and keyboard layout.

use alloc::format;
use alloc::string::String;
use anyos_std::{env, kbd, process, sys};
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── Public entry point ──────────────────────────────────────────────────────

/// Build the General settings panel inside `parent`. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_FILL);
    panel.set_color(layout::BG);

    layout::build_page_header(&panel, "General", "System preferences and identity");

    build_computer_info(&panel);
    build_hostname_card(&panel);
    build_user_card(&panel);
    build_preferences_card(&panel);
    build_keyboard_card(&panel);

    parent.add(&panel);
    panel.id()
}

// ── Computer Info card ──────────────────────────────────────────────────────

fn build_computer_info(panel: &ui::View) {
    let card = layout::build_auto_card(panel);
    layout::build_info_row(&card, "OS", "anyOS 1.0", true);
    layout::build_separator(&card);
    layout::build_info_row(&card, "Kernel", "x86_64-anyos", false);
    layout::build_separator(&card);
    layout::build_info_row(&card, "Architecture", "x86_64", false);
}

// ── Hostname card ───────────────────────────────────────────────────────────

fn build_hostname_card(panel: &ui::View) {
    let card = layout::build_section_card(panel, 140);

    // Current hostname display
    let mut host_buf = [0u8; 64];
    let hlen = sys::get_hostname(&mut host_buf);
    let hostname = if hlen != u32::MAX && hlen > 0 {
        core::str::from_utf8(&host_buf[..hlen as usize]).unwrap_or("anyOS Computer")
    } else {
        "anyOS Computer"
    };

    layout::build_info_row(&card, "Hostname", hostname, true);
    layout::build_separator(&card);

    // Editable hostname row
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(552, 44);
    row.set_margin(24, 4, 24, 8);

    let lbl = ui::Label::new("New name");
    lbl.set_position(0, 12);
    lbl.set_size(120, 20);
    lbl.set_text_color(layout::TEXT);
    lbl.set_font_size(13);
    row.add(&lbl);

    let tf = ui::TextField::new();
    tf.set_position(130, 8);
    tf.set_size(280, 28);
    tf.set_placeholder("Enter hostname");
    row.add(&tf);

    let tf_id = tf.id();
    let btn = ui::Button::new("Rename");
    btn.set_position(420, 8);
    btn.set_size(80, 28);
    btn.on_click(move |_| {
        let ctrl = ui::Control::from_id(tf_id);
        let mut buf = [0u8; 64];
        let len = ctrl.get_text(&mut buf) as usize;
        if len > 0 {
            if let Ok(text) = core::str::from_utf8(&buf[..len]) {
                sys::set_hostname(text);
            }
        }
    });
    row.add(&btn);

    card.add(&row);
}

// ── User card ───────────────────────────────────────────────────────────────

fn build_user_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);

    // Username
    let uid = process::getuid();
    let mut name_buf = [0u8; 64];
    let nlen = process::getusername(uid, &mut name_buf);
    let username = if nlen != u32::MAX && nlen > 0 {
        core::str::from_utf8(&name_buf[..nlen as usize]).unwrap_or("root")
    } else {
        "root"
    };
    layout::build_info_row(&card, "Username", username, true);
    layout::build_separator(&card);

    // UID
    let uid_str = format!("{}", uid);
    layout::build_info_row(&card, "UID", &uid_str, false);
    layout::build_separator(&card);

    // Home directory
    let mut home_buf = [0u8; 256];
    let hlen = env::get("HOME", &mut home_buf);
    let home = if hlen != u32::MAX && hlen > 0 {
        core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/tmp")
    } else {
        "/tmp"
    };
    layout::build_info_row(&card, "Home", home, false);
}

// ── Preferences card ────────────────────────────────────────────────────────

fn build_preferences_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);

    // Dark Mode toggle
    let dark_row = layout::build_setting_row(&card, "Dark Mode", true);
    let dark_on = ui::get_theme() == 0;
    let dark_toggle = layout::add_toggle_to_row(&dark_row, dark_on);
    dark_toggle.on_checked_changed(|e| {
        ui::set_theme(!e.checked);
    });

    layout::build_separator(&card);

    // Sound toggle (UI only)
    let sound_row = layout::build_setting_row(&card, "Sound", false);
    layout::add_toggle_to_row(&sound_row, true);

    layout::build_separator(&card);

    // Notifications toggle (UI only)
    let notif_row = layout::build_setting_row(&card, "Notifications", false);
    layout::add_toggle_to_row(&notif_row, true);
}

// ── Keyboard layout card ────────────────────────────────────────────────────

fn build_keyboard_card(panel: &ui::View) {
    let card = layout::build_section_card(panel, 100);

    // Gather available layouts
    let mut layout_buf = [kbd::LayoutInfo {
        id: 0,
        code: [0; 8],
        label: [0; 4],
    }; 16];
    let count = kbd::list_layouts(&mut layout_buf) as usize;
    let current_id = kbd::get_layout();

    if count == 0 {
        layout::build_info_row(&card, "Keyboard", "No layouts available", true);
        return;
    }

    // Build pipe-separated label string for the DropDown and track current index
    let mut items = String::new();
    let mut selected_idx: u32 = 0;
    for i in 0..count {
        if i > 0 {
            items.push('|');
        }
        let info = &layout_buf[i];
        let display = kbd::label_str(&info.label);
        items.push_str(display);
        if info.id == current_id {
            selected_idx = i as u32;
        }
    }

    // Current layout info row
    let current_label = kbd::label_str(&layout_buf[selected_idx as usize].label);
    layout::build_info_row(&card, "Keyboard", current_label, true);
    layout::build_separator(&card);

    // DropDown for layout selection
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(552, 44);
    row.set_margin(24, 0, 24, 8);

    let lbl = ui::Label::new("Layout");
    lbl.set_position(0, 12);
    lbl.set_size(120, 20);
    lbl.set_text_color(layout::TEXT);
    lbl.set_font_size(13);
    row.add(&lbl);

    let dd = ui::DropDown::new(&items);
    dd.set_position(130, 8);
    dd.set_size(240, 28);
    dd.set_selected_index(selected_idx);

    // Copy layout IDs into a static-ish array for the closure
    let mut ids = [0u32; 16];
    for i in 0..count {
        ids[i] = layout_buf[i].id;
    }
    let n = count;
    dd.on_selection_changed(move |e| {
        let idx = e.index as usize;
        if idx < n {
            kbd::set_layout(ids[idx]);
        }
    });

    row.add(&dd);
    card.add(&row);
}
