//! Built-in: General settings (Dark Mode, Sound, Notifications).

use libanyui_client as ui;
use ui::Widget;

/// Build the General settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_FILL);
    panel.set_color(0xFF1E1E1E);

    // Title
    let title = ui::Label::new("General");
    title.set_dock(ui::DOCK_TOP);
    title.set_size(560, 36);
    title.set_font_size(18);
    title.set_text_color(0xFFFFFFFF);
    title.set_margin(16, 12, 16, 4);
    panel.add(&title);

    // Card container
    let card = ui::Card::new();
    card.set_dock(ui::DOCK_TOP);
    card.set_size(560, 200);
    card.set_margin(16, 8, 16, 8);
    card.set_color(0xFF2D2D30);

    // Row 1: Device Name
    let row1 = ui::View::new();
    row1.set_dock(ui::DOCK_TOP);
    row1.set_size(528, 40);
    row1.set_margin(16, 8, 16, 0);
    let lbl = ui::Label::new("Device Name");
    lbl.set_position(0, 10);
    lbl.set_size(140, 20);
    lbl.set_text_color(0xFFCCCCCC);
    lbl.set_font_size(13);
    row1.add(&lbl);
    let val = ui::Label::new("anyOS Computer");
    val.set_position(150, 10);
    val.set_size(300, 20);
    val.set_text_color(0xFF969696);
    val.set_font_size(13);
    row1.add(&val);
    card.add(&row1);

    // Row 2: Dark Mode
    let row2 = ui::View::new();
    row2.set_dock(ui::DOCK_TOP);
    row2.set_size(528, 40);
    row2.set_margin(16, 0, 16, 0);
    let lbl2 = ui::Label::new("Dark Mode");
    lbl2.set_position(0, 10);
    lbl2.set_size(140, 20);
    lbl2.set_text_color(0xFFCCCCCC);
    lbl2.set_font_size(13);
    row2.add(&lbl2);
    let dark_toggle = ui::Toggle::new(ui::get_theme() == 0);
    dark_toggle.set_position(480, 6);
    dark_toggle.on_checked_changed(|e| {
        // dark mode = theme 0, light mode = theme 1
        ui::set_theme(!e.checked);
    });
    row2.add(&dark_toggle);
    card.add(&row2);

    // Row 3: Sound
    let row3 = ui::View::new();
    row3.set_dock(ui::DOCK_TOP);
    row3.set_size(528, 40);
    row3.set_margin(16, 0, 16, 0);
    let lbl3 = ui::Label::new("Sound");
    lbl3.set_position(0, 10);
    lbl3.set_size(140, 20);
    lbl3.set_text_color(0xFFCCCCCC);
    lbl3.set_font_size(13);
    row3.add(&lbl3);
    let sound_toggle = ui::Toggle::new(true);
    sound_toggle.set_position(480, 6);
    row3.add(&sound_toggle);
    card.add(&row3);

    // Row 4: Notifications
    let row4 = ui::View::new();
    row4.set_dock(ui::DOCK_TOP);
    row4.set_size(528, 40);
    row4.set_margin(16, 0, 16, 0);
    let lbl4 = ui::Label::new("Notifications");
    lbl4.set_position(0, 10);
    lbl4.set_size(140, 20);
    lbl4.set_text_color(0xFFCCCCCC);
    lbl4.set_font_size(13);
    row4.add(&lbl4);
    let notif_toggle = ui::Toggle::new(true);
    notif_toggle.set_position(480, 6);
    row4.add(&notif_toggle);
    card.add(&row4);

    panel.add(&card);
    parent.add(&panel);
    panel.id()
}
