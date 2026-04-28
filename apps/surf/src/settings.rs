// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Settings dialog for Surf.

use libanyui_client as ui_lib;
use ui_lib::Widget;

use crate::config;
use crate::state;

pub fn open_settings() {
    let st = state();

    let win = ui_lib::Window::new("Settings", -1, -1, 460, 240);

    // ── General section ──
    let panel = ui_lib::View::new();
    panel.set_dock(ui_lib::DOCK_FILL);
    panel.set_padding(16, 16, 16, 16);
    panel.set_color(0xFF1E1E1E);
    win.add(&panel);

    let lbl_homepage = ui_lib::Label::new("Homepage:");
    lbl_homepage.set_position(0, 10);
    lbl_homepage.set_size(90, 22);
    lbl_homepage.set_text_color(0xFFCCCCCC);
    panel.add(&lbl_homepage);

    let homepage_field = ui_lib::TextField::new();
    homepage_field.set_position(95, 8);
    homepage_field.set_size(330, 24);
    homepage_field.set_placeholder("https://www.example.com");
    homepage_field.set_text(&st.config.homepage);
    panel.add(&homepage_field);

    let lbl_hint = ui_lib::Label::new("Shown as suggestion in the URL bar on startup.");
    lbl_hint.set_position(95, 38);
    lbl_hint.set_size(330, 18);
    lbl_hint.set_text_color(0xFF888888);
    lbl_hint.set_font_size(11);
    panel.add(&lbl_hint);

    // ── Inhalte section ──
    let lbl_section = ui_lib::Label::new("Inhalte");
    lbl_section.set_position(0, 72);
    lbl_section.set_size(425, 22);
    lbl_section.set_text_color(0xFFCCCCCC);
    panel.add(&lbl_section);

    let chk_js = ui_lib::Checkbox::new("JavaScript aktivieren");
    chk_js.set_position(0, 100);
    chk_js.set_size(425, 22);
    chk_js.set_state(if st.config.js_enabled { 1 } else { 0 });
    panel.add(&chk_js);

    let lbl_js_hint = ui_lib::Label::new("Wenn deaktiviert, werden Skripte nicht ausgeführt.");
    lbl_js_hint.set_position(20, 124);
    lbl_js_hint.set_size(405, 18);
    lbl_js_hint.set_text_color(0xFF888888);
    lbl_js_hint.set_font_size(11);
    panel.add(&lbl_js_hint);

    // ── Buttons ──
    let btn_bar = ui_lib::View::new();
    btn_bar.set_dock(ui_lib::DOCK_BOTTOM);
    btn_bar.set_size(0, 44);
    btn_bar.set_color(0xFF252525);
    btn_bar.set_padding(8, 8, 8, 8);
    win.add(&btn_bar);

    let btn_save = ui_lib::Button::new("Save");
    btn_save.set_position(270, 4);
    btn_save.set_size(80, 28);
    btn_bar.add(&btn_save);

    let btn_cancel = ui_lib::Button::new("Cancel");
    btn_cancel.set_position(358, 4);
    btn_cancel.set_size(80, 28);
    btn_bar.add(&btn_cancel);

    let win_id = win.id();

    btn_save.on_click(move |_| {
        let st = state();
        let new_homepage = config::get_field_text(&homepage_field);
        st.config.homepage = new_homepage.clone();
        st.config.js_enabled = chk_js.get_state() != 0;
        config::save(&st.config, &st.bookmarks);

        // Update URL bar placeholder.
        if new_homepage.is_empty() {
            st.url_field.set_placeholder("Enter URL...");
        } else {
            st.url_field.set_placeholder(&new_homepage);
        }

        ui_lib::Control::from_id(win_id).set_visible(false);
    });

    btn_cancel.on_click(move |_| {
        ui_lib::Control::from_id(win_id).set_visible(false);
    });

    win.on_close(|_| {});
}
