// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Runner — Spotlight-style quick application launcher (Alt+R).
//!
//! Scans /Applications/ for .app bundles and presents a floating,
//! semi-transparent search bar with autocomplete. Type to filter,
//! press Enter to launch, Escape to dismiss.

#![no_std]
#![no_main]

mod apps;
mod render;
mod search;
mod state;
mod ui;

use libanyui_client as anyui;

anyos_std::entry!(main);

fn main() {
    if !anyui::init() {
        return;
    }
    ui::build();
    anyui::run();
}
