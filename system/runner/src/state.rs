// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Global application state singleton.

use anyos_std::Vec;
use libanyui_client as ui;
use crate::apps::AppEntry;
use crate::search::SearchResult;

pub struct RunnerState {
    pub apps: Vec<AppEntry>,
    pub results: Vec<SearchResult>,
    pub selected: usize,
    pub field_id: u32,
    pub canvas: ui::Canvas,
    pub win: ui::Window,
    pub current_height: u32,
    /// True when a searchd query is pending (debounced).
    pub pending_query: bool,
    /// Timestamp of last text change (for debounce).
    pub last_query_time: u32,
}

static mut STATE: Option<RunnerState> = None;

pub fn init(apps: Vec<AppEntry>, field_id: u32, canvas: ui::Canvas, win: ui::Window) {
    unsafe {
        STATE = Some(RunnerState {
            apps,
            results: Vec::new(),
            selected: 0,
            field_id,
            canvas,
            win,
            current_height: crate::render::SEARCH_HEIGHT,
            pending_query: false,
            last_query_time: 0,
        });
    }
}

pub fn get() -> &'static mut RunnerState {
    unsafe { STATE.as_mut().unwrap() }
}
