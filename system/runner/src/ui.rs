// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! UI construction and event handling — Spotlight-style launcher.

use libanyui_client as ui;
use ui::Widget;
use crate::{apps, render, search, state};

const BLUR_RADIUS: u32 = 12;
const TEXT_COLOR: u32 = 0xFF_F0_F0_F0;
const FONT_SIZE: u32 = 20;

/// Maximum expanded height (search bar + results).
const MAX_HEIGHT: u32 = 500;

/// Creates the Runner window and wires all events. Call `ui::run()` after.
pub fn build() {
    let apps_list = apps::scan_apps();

    // Center horizontally, place in upper quarter of screen
    let (sw, sh) = ui::screen_size();
    let x = (sw.saturating_sub(render::WIN_WIDTH) / 2) as i32;
    let y = (sh / 4).saturating_sub(render::SEARCH_HEIGHT / 2) as i32;

    // Start with just the search bar height — will resize when results appear
    let win = ui::Window::new_with_flags(
        "",
        x, y,
        render::WIN_WIDTH, render::SEARCH_HEIGHT,
        ui::WIN_FLAG_BORDERLESS
            | ui::WIN_FLAG_NOT_RESIZABLE
            | ui::WIN_FLAG_ALWAYS_ON_TOP
            | ui::WIN_FLAG_SHADOW
            | ui::WIN_FLAG_NO_MINIMIZE
            | ui::WIN_FLAG_NO_MAXIMIZE,
    );
    win.set_color(0x00_000000);
    ui::set_blur_behind(&win, BLUR_RADIUS);

    // Canvas fills entire window
    let canvas = ui::Canvas::new(render::WIN_WIDTH, render::SEARCH_HEIGHT);
    canvas.set_dock(ui::DOCK_FILL);
    win.add(&canvas);

    // Initial draw: just the search bar, no results
    render::draw(&canvas, render::WIN_WIDTH, render::SEARCH_HEIGHT, &[], &apps_list, 0);

    // Click on canvas → launch clicked result
    canvas.on_click(|_| on_canvas_click());

    // Text field overlaid on the search bar area
    let field = ui::TextField::new();
    field.set_size(render::WIN_WIDTH - 36, 36);
    field.set_position(18, 14);
    field.set_color(render::BG_COLOR);
    field.set_text_color(TEXT_COLOR);
    field.set_font_size(FONT_SIZE);
    field.set_placeholder("Type to search \u{2026}");
    win.add(&field);

    let field_id = field.id();
    state::init(apps_list, field_id, canvas, win);

    // Live filter as user types
    field.on_text_changed(|_| on_query_changed());

    // Enter launches selected result
    field.on_submit(|_| on_submit());

    // ESC + Arrow keys on the text field (window key events don't fire
    // when a child control has focus)
    field.on_key_down(|e| {
        match e.keycode {
            0x103 => ui::quit(),             // ESC
            0x106 => move_selection(-1),      // Arrow Up
            0x107 => move_selection(1),       // Arrow Down
            _ => {}
        }
    });

    win.on_close(|_| { ui::quit(); });
    field.focus();
}

fn on_query_changed() {
    let s = state::get();
    let ctrl = ui::Control::from_id(s.field_id);
    let mut buf = [0u8; 256];
    let len = ctrl.get_text(&mut buf) as usize;
    let query = core::str::from_utf8(&buf[..len.min(256)]).unwrap_or("");

    s.results = search::filter(&s.apps, query);
    s.selected = 0;

    redraw();
}

fn move_selection(delta: i32) {
    let s = state::get();
    if s.results.is_empty() {
        return;
    }
    let count = s.results.len() as i32;
    let new = (s.selected as i32 + delta).rem_euclid(count) as usize;
    s.selected = new;
    redraw();
}

fn on_submit() {
    let s = state::get();
    if let Some(result) = s.results.get(s.selected) {
        let path = s.apps[result.app_idx].path.clone();
        ui::quit();
        anyos_std::process::launch_app(&path, "");
    }
}

fn on_canvas_click() {
    let s = state::get();
    let (_mx, my, _btn) = s.canvas.get_mouse();

    if let Some(idx) = render::hit_test(&s.results, my) {
        // Select and launch
        s.selected = idx;
        if let Some(result) = s.results.get(idx) {
            let path = s.apps[result.app_idx].path.clone();
            ui::quit();
            anyos_std::process::launch_app(&path, "");
        }
    }
}

fn redraw() {
    let s = state::get();
    let new_h = render::calc_height(&s.results).min(MAX_HEIGHT);

    // Resize window if height changed
    if new_h != s.current_height {
        s.win.resize(render::WIN_WIDTH, new_h);
        s.current_height = new_h;
    }

    render::draw(&s.canvas, render::WIN_WIDTH, new_h, &s.results, &s.apps, s.selected);
}
