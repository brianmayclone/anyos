// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! UI construction and event handling — Spotlight-style launcher.

use libanyui_client as ui;
use ui::Widget;
use crate::{apps, render, search, state};
use crate::search::SearchResult;

const BLUR_RADIUS: u32 = 12;
const TEXT_COLOR: u32 = 0xFF_F0_F0_F0;
const FONT_SIZE: u32 = 20;
const MAX_HEIGHT: u32 = 500;

/// Minimum interval between searchd queries (ms).
const SEARCHD_INTERVAL_MS: u32 = 300;

pub fn build() {
    let apps_list = apps::scan_apps();

    let (sw, sh) = ui::screen_size();
    let x = (sw.saturating_sub(render::WIN_WIDTH) / 2) as i32;
    let y = (sh / 4).saturating_sub(render::SEARCH_HEIGHT / 2) as i32;

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

    let canvas = ui::Canvas::new(render::WIN_WIDTH, MAX_HEIGHT);
    canvas.set_position(0, 0);
    win.add(&canvas);

    render::draw(&canvas, render::WIN_WIDTH, render::SEARCH_HEIGHT, &[], &apps_list, 0);
    canvas.on_click(|_| on_canvas_click());

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

    field.on_text_changed(|_| on_query_changed());
    field.on_submit(|_| on_submit());
    field.on_key_down(|e| {
        match e.keycode {
            0x103 => ui::quit(),
            0x106 => move_selection(-1),
            0x107 => move_selection(1),
            _ => {}
        }
    });

    // Catch-up timer for skipped searchd queries
    ui::set_timer(SEARCHD_INTERVAL_MS, || on_catchup_tick());

    win.on_close(|_| { ui::quit(); });
    field.focus();
}

fn on_query_changed() {
    let s = state::get();
    let query = read_query(s.field_id);

    let now = anyos_std::sys::uptime_ms();
    let searchd_ready = now.wrapping_sub(s.last_searchd_time) >= SEARCHD_INTERVAL_MS;

    if query.len() >= 2 && searchd_ready {
        s.results = search::filter_all(&s.apps, &query);
        s.last_searchd_time = now;
        s.pending_query = false;
    } else {
        s.results = search::filter_apps(&s.apps, &query);
        if query.len() >= 2 {
            s.pending_query = true;
        }
    }
    s.selected = 0;
    redraw();
}

fn on_catchup_tick() {
    let s = state::get();
    if !s.pending_query { return; }
    let now = anyos_std::sys::uptime_ms();
    if now.wrapping_sub(s.last_searchd_time) < SEARCHD_INTERVAL_MS { return; }
    s.pending_query = false;

    let query = read_query(s.field_id);
    if query.len() >= 2 {
        s.results = search::filter_all(&s.apps, &query);
        s.last_searchd_time = now;
        if s.selected >= s.results.len() && !s.results.is_empty() {
            s.selected = 0;
        }
        redraw();
    }
}

fn read_query(field_id: u32) -> anyos_std::String {
    let ctrl = ui::Control::from_id(field_id);
    let mut buf = [0u8; 256];
    let len = ctrl.get_text(&mut buf) as usize;
    let text = core::str::from_utf8(&buf[..len.min(256)]).unwrap_or("");
    anyos_std::String::from(text)
}

fn move_selection(delta: i32) {
    let s = state::get();
    if s.results.is_empty() { return; }
    let count = s.results.len() as i32;
    s.selected = (s.selected as i32 + delta).rem_euclid(count) as usize;
    redraw();
}

fn on_submit() {
    let s = state::get();
    if let Some(result) = s.results.get(s.selected) {
        launch_result(result, &s.apps);
    }
}

fn on_canvas_click() {
    let s = state::get();
    let (_mx, my, _btn) = s.canvas.get_mouse();
    if let Some(idx) = render::hit_test(&s.results, my) {
        s.selected = idx;
        if let Some(result) = s.results.get(idx) {
            launch_result(result, &s.apps);
        }
    }
}

fn launch_result(result: &SearchResult, apps: &[crate::apps::AppEntry]) {
    match result {
        SearchResult::App { app_idx } => {
            let path = apps[*app_idx].path.clone();
            ui::quit();
            anyos_std::process::launch_app(&path, "");
        }
        SearchResult::File { path, .. } => {
            let p = path.clone();
            ui::quit();
            anyos_std::process::spawn("/System/bin/open", &p);
        }
    }
}

fn redraw() {
    let s = state::get();
    let new_h = render::calc_height(&s.results).min(MAX_HEIGHT);
    if new_h != s.current_height {
        s.win.resize(render::WIN_WIDTH, new_h);
        s.current_height = new_h;
    }
    render::draw(&s.canvas, render::WIN_WIDTH, new_h, &s.results, &s.apps, s.selected);
}
