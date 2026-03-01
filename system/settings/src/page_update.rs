//! Settings page: Update — placeholder for system updates.
//!
//! Displays a simple "your system is up to date" message with a
//! disabled "Check for Updates" button.

use anyos_std::i18n;
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

/// Build the Update settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::bg());

    layout::build_page_header(&panel, i18n::t("Update"), i18n::t("System updates"));

    // ── Status card ─────────────────────────────────────────────────────
    let card = layout::build_auto_card(&panel);

    // Checkmark + status
    let status_lbl = ui::Label::new(i18n::t("Your system is up to date"));
    status_lbl.set_dock(ui::DOCK_TOP);
    status_lbl.set_size(400, 24);
    status_lbl.set_font_size(16);
    status_lbl.set_text_color(ui::theme::colors().success);
    status_lbl.set_margin(24, 16, 24, 0);
    card.add(&status_lbl);

    // Version info
    let ver_text = anyos_std::format!("anyOS 1.0 — {}: {}", i18n::t("Last checked"), i18n::t("Never"));
    let ver_lbl = ui::Label::new(&ver_text);
    ver_lbl.set_dock(ui::DOCK_TOP);
    ver_lbl.set_size(400, 18);
    ver_lbl.set_font_size(12);
    ver_lbl.set_text_color(layout::text_dim());
    ver_lbl.set_margin(24, 6, 24, 0);
    card.add(&ver_lbl);

    // Check for Updates button (disabled)
    let btn = ui::Button::new(i18n::t("Check for Updates"));
    btn.set_dock(ui::DOCK_TOP);
    btn.set_size(160, 32);
    btn.set_enabled(false);
    btn.set_margin(24, 12, 24, 16);
    card.add(&btn);

    parent.add(&panel);
    panel.id()
}
