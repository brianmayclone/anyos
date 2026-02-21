//! demo_anyui — Demo application using the libanyui OOP UI framework.
//!
//! Demonstrates the Windows Forms-style API: typed control structs,
//! Container.add() for parenting, and property setters.

#![no_std]
#![no_main]

use libanyui_client as ui;
use ui::Widget;

anyos_std::entry!(main);

fn main() {
    if !ui::init() {
        anyos_std::println!("Failed to load libanyui.so");
        return;
    }

    let win = ui::Window::new("anyui Demo", 400, 350);

    // Title label
    let title = ui::Label::new("anyui Demo");
    title.set_position(20, 15);
    title.set_color(0xFF007AFF); // accent blue
    win.add(&title);

    // Divider
    let div = ui::Divider::new();
    div.set_position(20, 45);
    div.set_size(360, 1);
    win.add(&div);

    // Button
    let btn = ui::Button::new("Click Me");
    btn.set_position(20, 60);
    btn.set_size(120, 32);
    btn.on_click(|_e| {
        // Handle button click
    });
    win.add(&btn);

    // Toggle
    let toggle_label = ui::Label::new("Dark Mode");
    toggle_label.set_position(20, 110);
    win.add(&toggle_label);

    let toggle = ui::Toggle::new(true);
    toggle.set_position(120, 108);
    toggle.on_checked_changed(|_e| {
        // Handle toggle change
    });
    win.add(&toggle);

    // Checkbox
    let checkbox = ui::Checkbox::new("Enable notifications");
    checkbox.set_position(20, 150);
    checkbox.on_checked_changed(|_e| {
        // Handle checkbox change
    });
    win.add(&checkbox);

    // Slider + Progress bar linked via typed event
    let vol_label = ui::Label::new("Volume");
    vol_label.set_position(20, 195);
    win.add(&vol_label);

    let slider = ui::Slider::new(75);
    slider.set_position(100, 195);
    slider.set_size(200, 20);
    win.add(&slider);

    let prog_label = ui::Label::new("Progress");
    prog_label.set_position(20, 235);
    win.add(&prog_label);

    let progress = ui::ProgressBar::new(75);
    progress.set_position(100, 238);
    progress.set_size(200, 8);
    win.add(&progress);

    // Link slider to progress bar with typed event
    slider.on_value_changed(move |e| {
        progress.set_state(e.value);
    });

    // Card with nested content
    let card = ui::Card::new();
    card.set_position(20, 270);
    card.set_size(360, 60);
    win.add(&card);

    let card_title = ui::Label::new("Card Title");
    card_title.set_position(12, 8);
    card.add(&card_title);

    let card_text = ui::Label::new("Nested content inside a card.");
    card_text.set_position(12, 30);
    card.add(&card_text);

    // Run the event loop
    ui::run();
}
