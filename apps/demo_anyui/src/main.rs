//! demo_anyui — Demo application using the libanyui OOP UI framework.
//!
//! Creates a window with various controls to demonstrate the library.

#![no_std]
#![no_main]

use libanyui_client as ui;

anyos_std::entry!(main);

extern "C" fn on_button_click(_id: u32, _event: u32, _userdata: u64) {
    // Button was clicked — update the label
    // (In a real app, we'd store the label control ID and update it)
}

extern "C" fn on_toggle_change(id: u32, _event: u32, _userdata: u64) {
    // Toggle state changed
    let _state = ui::Control(id).get_state();
}

fn main() {
    if !ui::init() {
        anyos_std::println!("Failed to load libanyui.so");
        return;
    }

    let win = ui::Window::new("anyui Demo", 400, 350);

    // Title label
    let title = win.add_label("anyui Demo", 20, 15);
    title.set_color(0xFF007AFF); // accent blue

    // Divider
    win.add_divider(20, 45, 360);

    // Button
    let btn = win.add_button("Click Me", 20, 60, 120, 32);
    btn.on_click(on_button_click, 0);

    // Toggle
    win.add_label("Dark Mode", 20, 110);
    let toggle = win.add_toggle(120, 108, true);
    toggle.on_change(on_toggle_change, 0);

    // Checkbox
    win.add_checkbox("Enable notifications", 20, 150);

    // Slider
    win.add_label("Volume", 20, 195);
    win.add_slider(100, 195, 200, 75);

    // Progress bar
    win.add_label("Progress", 20, 235);
    win.add_progress_bar(100, 238, 200, 65);

    // Card with nested content
    let card = win.add_card(20, 270, 360, 60);
    card.add_control(ui::KIND_LABEL, 12, 8, 0, 0, "Card Title");
    card.add_control(ui::KIND_LABEL, 12, 30, 0, 0, "Nested content inside a card.");

    // Run the event loop
    ui::run();
}
