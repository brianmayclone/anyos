//! Settings page: Update — placeholder for system updates.
//!
//! Displays a simple "your system is up to date" message with a
//! disabled "Check for Updates" button.

use libanyui_client as ui;
use ui::Widget;

use crate::layout;

/// Build the Update settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_FILL);
    panel.set_color(layout::BG);

    layout::build_page_header(&panel, "Update", "System updates");

    // ── Status card ─────────────────────────────────────────────────────
    let card = layout::build_section_card(&panel, 140);

    // Checkmark + status
    let status_lbl = ui::Label::new("Your system is up to date");
    status_lbl.set_position(24, 20);
    status_lbl.set_size(400, 24);
    status_lbl.set_font_size(16);
    status_lbl.set_text_color(0xFF4EC970);
    card.add(&status_lbl);

    // Version info
    let ver_lbl = ui::Label::new("anyOS 1.0 — Last checked: Never");
    ver_lbl.set_position(24, 50);
    ver_lbl.set_size(400, 18);
    ver_lbl.set_font_size(12);
    ver_lbl.set_text_color(layout::TEXT_DIM);
    card.add(&ver_lbl);

    // Check for Updates button (disabled)
    let btn = ui::Button::new("Check for Updates");
    btn.set_position(24, 84);
    btn.set_size(160, 32);
    btn.set_enabled(false);
    card.add(&btn);

    parent.add(&panel);
    panel.id()
}
