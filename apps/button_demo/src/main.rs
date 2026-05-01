// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Button Auto-Size Demo
//!
//! Demonstrates the new auto_size() method on Button and IconButton.
//! Buttons will automatically resize to fit their text content.

#![no_std]
#![no_main]

use libanyui_client as anyui;

anyos_std::entry!(main);

fn main() {
    if !anyui::init() {
        anyos_std::println!("Failed to load libanyui.so");
        return;
    }

    let win = anyui::Window::new("Button Auto-Size Demo", 100, 100, 600, 400);

    // Content area
    let content = anyui::View::new();
    content.set_dock(anyui::DOCK_FILL);
    win.add(&content);

    // Title label
    let title = anyui::Label::new("Button Auto-Size Examples");
    title.set_position(20, 20);
    title.set_size(300, 24);
    content.add(&title);

    // Button 1: Auto-sized with short text
    let btn1 = anyui::Button::new("OK");
    btn1.set_position(20, 60);
    btn1.auto_size(); // Width adapts to text
    content.add(&btn1);

    let lbl1 = anyui::Label::new("Short text button (auto-sized)");
    lbl1.set_position(150, 60);
    lbl1.set_size(400, 20);
    content.add(&lbl1);

    // Button 2: Auto-sized with longer text
    let btn2 = anyui::Button::new("Save All Changes");
    btn2.set_position(20, 100);
    btn2.auto_size(); // Width adapts to longer text
    content.add(&btn2);

    let lbl2 = anyui::Label::new("Longer text button (auto-sized)");
    lbl2.set_position(150, 100);
    lbl2.set_size(400, 20);
    content.add(&lbl2);

    // Button 3: Fixed size for comparison
    let btn3 = anyui::Button::new("Fixed");
    btn3.set_position(20, 140);
    btn3.set_size(100, 32); // Fixed size
    content.add(&btn3);

    let lbl3 = anyui::Label::new("Fixed size button (100x32)");
    lbl3.set_position(150, 140);
    lbl3.set_size(400, 20);
    content.add(&lbl3);

    // IconButton with auto-size
    let icobutton = anyui::IconButton::new("Apply");
    icobutton.set_position(20, 180);
    icobutton.set_system_icon("check", anyui::IconType::Outline, 0xFFFFFFFF, 16);
    icobutton.auto_size();
    content.add(&icobutton);

    let lbl4 = anyui::Label::new("IconButton with text (auto-sized)");
    lbl4.set_position(150, 180);
    lbl4.set_size(400, 20);
    content.add(&lbl4);

    // Close on window close
    win.on_close(|_| {
        anyui::quit();
    });

    anyui::run();
}
