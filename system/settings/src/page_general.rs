//! General system settings page.
//!
//! Provides cards for computer info, hostname editing, user identity,
//! UI preferences (dark mode, sound, notifications), and keyboard layout.

use alloc::format;
use alloc::string::String;
use anyos_std::{env, fs, i18n, kbd, process, sys};
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── Public entry point ──────────────────────────────────────────────────────

/// Build the General settings panel inside `parent`. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::bg());

    layout::build_page_header(&panel, i18n::t("General"), i18n::t("System preferences and identity"));

    build_computer_info(&panel);
    build_hostname_card(&panel);
    build_user_card(&panel);
    build_preferences_card(&panel);
    build_keyboard_card(&panel);
    build_language_card(&panel);

    parent.add(&panel);
    panel.id()
}

// ── Computer Info card ──────────────────────────────────────────────────────

fn build_computer_info(panel: &ui::View) {
    let card = layout::build_auto_card(panel);
    layout::build_info_row(&card, i18n::t("OS"), "anyOS 1.0", true);
    layout::build_separator(&card);
    layout::build_info_row(&card, i18n::t("Kernel"), "x86_64-anyos", false);
    layout::build_separator(&card);
    layout::build_info_row(&card, i18n::t("Architecture"), "x86_64", false);
}

// ── Hostname card ───────────────────────────────────────────────────────────

fn build_hostname_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);

    // Current hostname display
    let mut host_buf = [0u8; 64];
    let hlen = sys::get_hostname(&mut host_buf);
    let hostname = if hlen != u32::MAX && hlen > 0 {
        core::str::from_utf8(&host_buf[..hlen as usize]).unwrap_or("anyOS Computer")
    } else {
        "anyOS Computer"
    };

    let row = layout::build_setting_row(&card, i18n::t("Hostname"), true);

    let val_lbl = ui::Label::new(hostname);
    val_lbl.set_position(200, 12);
    val_lbl.set_size(200, 20);
    val_lbl.set_text_color(layout::text_dim());
    val_lbl.set_font_size(13);
    row.add(&val_lbl);

    let val_id = val_lbl.id();
    let btn = ui::Button::new(i18n::t("Rename..."));
    btn.set_position(420, 8);
    btn.set_size(90, 28);
    btn.on_click(move |_| {
        open_rename_dialog(val_id);
    });
    row.add(&btn);
}

fn open_rename_dialog(hostname_label_id: u32) {
    let win = ui::Window::new(i18n::t("Rename Computer"), -1, -1, 360, 180);
    let win_id = win.id();

    let root = ui::View::new();
    root.set_dock(ui::DOCK_FILL);
    root.set_color(layout::bg());

    // Instruction
    let instr = ui::Label::new(i18n::t("Enter a new name for this computer:"));
    instr.set_dock(ui::DOCK_TOP);
    instr.set_size(320, 24);
    instr.set_font_size(13);
    instr.set_text_color(layout::text());
    instr.set_margin(20, 20, 20, 0);
    root.add(&instr);

    // Text field with current hostname pre-filled
    let tf = ui::TextField::new();
    tf.set_dock(ui::DOCK_TOP);
    tf.set_size(320, 28);
    tf.set_margin(20, 8, 20, 0);
    tf.set_placeholder(i18n::t("Hostname"));
    // Pre-fill with current hostname
    let mut cur = [0u8; 64];
    let clen = sys::get_hostname(&mut cur);
    if clen != u32::MAX && clen > 0 {
        if let Ok(text) = core::str::from_utf8(&cur[..clen as usize]) {
            tf.set_text(text);
        }
    }
    root.add(&tf);

    // Button row
    let btn_row = ui::View::new();
    btn_row.set_dock(ui::DOCK_TOP);
    btn_row.set_size(320, 36);
    btn_row.set_margin(20, 16, 20, 20);

    let tf_id = tf.id();
    let lbl_id = hostname_label_id;
    let wid = win_id;

    let ok_btn = ui::Button::new(i18n::t("Rename"));
    ok_btn.set_position(0, 0);
    ok_btn.set_size(100, 32);
    ok_btn.on_click(move |_| {
        let ctrl = ui::Control::from_id(tf_id);
        let mut buf = [0u8; 64];
        let len = ctrl.get_text(&mut buf) as usize;
        if len > 0 {
            if let Ok(text) = core::str::from_utf8(&buf[..len]) {
                sys::set_hostname(text);
                // Update the label in the main window
                let lbl = ui::Control::from_id(lbl_id);
                lbl.set_text(text);
            }
        }
        let w: ui::Window = unsafe { core::mem::transmute(wid) };
        w.destroy();
    });
    btn_row.add(&ok_btn);

    let wid2 = win_id;
    let cancel_btn = ui::Button::new(i18n::t("Cancel"));
    cancel_btn.set_position(110, 0);
    cancel_btn.set_size(100, 32);
    cancel_btn.on_click(move |_| {
        let w: ui::Window = unsafe { core::mem::transmute(wid2) };
        w.destroy();
    });
    btn_row.add(&cancel_btn);

    root.add(&btn_row);
    win.add(&root);

    let wid3 = win_id;
    win.on_close(move |_| {
        let w: ui::Window = unsafe { core::mem::transmute(wid3) };
        w.destroy();
    });
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
    layout::build_info_row(&card, i18n::t("Username"), username, true);
    layout::build_separator(&card);

    // UID
    let uid_str = format!("{}", uid);
    layout::build_info_row(&card, i18n::t("UID"), &uid_str, false);
    layout::build_separator(&card);

    // Home directory
    let mut home_buf = [0u8; 256];
    let hlen = env::get("HOME", &mut home_buf);
    let home = if hlen != u32::MAX && hlen > 0 {
        core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/tmp")
    } else {
        "/tmp"
    };
    layout::build_info_row(&card, i18n::t("Home"), home, false);
}

// ── Preferences card ────────────────────────────────────────────────────────

fn build_preferences_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);

    // Dark Mode toggle
    let dark_row = layout::build_setting_row(&card, i18n::t("Dark Mode"), true);
    let dark_on = ui::get_theme() == 0;
    let dark_toggle = layout::add_toggle_to_row(&dark_row, dark_on);
    dark_toggle.on_checked_changed(|e| {
        ui::set_theme(!e.checked);
        crate::invalidate_all_pages();
    });

    layout::build_separator(&card);

    // Sound toggle (UI only)
    let sound_row = layout::build_setting_row(&card, i18n::t("Sound"), false);
    layout::add_toggle_to_row(&sound_row, true);

    layout::build_separator(&card);

    // Notifications toggle (UI only)
    let notif_row = layout::build_setting_row(&card, i18n::t("Notifications"), false);
    layout::add_toggle_to_row(&notif_row, true);

    layout::build_separator(&card);

    // Serial Verbose toggle — controls whether userspace stdout/stderr
    // and driver messages are forwarded to the serial console.
    let serial_row = layout::build_setting_row(&card, i18n::t("Serial Verbose"), false);
    let serial_toggle = layout::add_toggle_to_row(&serial_row, false);
    serial_toggle.on_checked_changed(|e| {
        sys::set_serial_verbose(e.checked);
    });
}

// ── Keyboard layout card ────────────────────────────────────────────────────

fn build_keyboard_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);

    // Gather available layouts
    let mut layout_buf = [kbd::LayoutInfo {
        id: 0,
        code: [0; 8],
        label: [0; 4],
    }; 16];
    let count = kbd::list_layouts(&mut layout_buf) as usize;
    let current_id = kbd::get_layout();

    if count == 0 {
        layout::build_info_row(&card, i18n::t("Keyboard"), i18n::t("No layouts available"), true);
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
    layout::build_info_row(&card, i18n::t("Keyboard"), current_label, true);
    layout::build_separator(&card);

    // DropDown for layout selection
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(552, 44);
    row.set_margin(24, 0, 24, 8);

    let lbl = ui::Label::new(i18n::t("Layout"));
    lbl.set_position(0, 12);
    lbl.set_size(120, 20);
    lbl.set_text_color(layout::text());
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

// ── Language card ──────────────────────────────────────────────────────────

const LANG_CONF_PATH: &str = "/System/settings/language.conf";
const LANG_CODES: [&str; 5] = ["en", "de", "fr", "it", "gsw"];
const LANG_NAMES: &str = "English|Deutsch|Français|Italiano|Schwizerdütsch";

fn build_language_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);

    // Determine currently active language
    let current = i18n::lang();
    let mut selected_idx: u32 = 0;
    for (i, code) in LANG_CODES.iter().enumerate() {
        if *code == current {
            selected_idx = i as u32;
            break;
        }
    }

    layout::build_info_row(&card, i18n::t("Language"), LANG_CODES[selected_idx as usize], true);
    layout::build_separator(&card);

    // DropDown for language selection
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(552, 44);
    row.set_margin(24, 0, 24, 0);

    let lbl = ui::Label::new(i18n::t("Language"));
    lbl.set_position(0, 12);
    lbl.set_size(120, 20);
    lbl.set_text_color(layout::text());
    lbl.set_font_size(13);
    row.add(&lbl);

    let dd = ui::DropDown::new(LANG_NAMES);
    dd.set_position(130, 8);
    dd.set_size(240, 28);
    dd.set_selected_index(selected_idx);

    // Hint label (hidden initially, shown after language switch)
    let hint = ui::Label::new(i18n::t("Restart other apps to apply language change"));
    hint.set_dock(ui::DOCK_TOP);
    hint.set_size(552, 24);
    hint.set_margin(24, 4, 24, 8);
    hint.set_font_size(11);
    hint.set_text_color(layout::accent());
    hint.set_visible(false);

    dd.on_selection_changed(move |e| {
        let idx = e.index as usize;
        if idx < LANG_CODES.len() {
            let code = LANG_CODES[idx];
            // Save to config file
            let _ = fs::write_bytes(LANG_CONF_PATH, code.as_bytes());
            // Set environment variable for current session
            env::set("LANG", code);
            // Immediately refresh the Settings app UI with the new language
            crate::refresh_after_language_change();
        }
    });

    row.add(&dd);
    card.add(&row);
    card.add(&hint);
}
