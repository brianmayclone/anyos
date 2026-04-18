//! Settings page: Update — system update management.
//!
//! Shows current OS version and a button to open the Software Update app.

use anyos_std::{i18n, process};
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

const UPDATER_PATH: &str = "/Applications/Software Update.app/Software Update";

/// Build the Update settings panel. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::bg());

    layout::build_page_header(&panel, i18n::t("Update"), i18n::t("System updates"));

    // ── Status card ─────────────────────────────────────────────────────
    let card = layout::build_auto_card(&panel);

    // Version info
    let ver_text = anyos_std::format!("anyOS {}", env!("ANYOS_VERSION"));
    let ver_lbl = ui::Label::new(&ver_text);
    ver_lbl.set_dock(ui::DOCK_TOP);
    ver_lbl.set_size(400, 24);
    ver_lbl.set_font_size(16);
    ver_lbl.set_text_color(ui::theme::colors().success);
    ver_lbl.set_margin(24, 16, 24, 0);
    card.add(&ver_lbl);

    let desc_lbl = ui::Label::new(i18n::t("Open the Software Update app to check for and install available updates."));
    desc_lbl.set_dock(ui::DOCK_TOP);
    desc_lbl.set_size(400, 18);
    desc_lbl.set_font_size(12);
    desc_lbl.set_text_color(layout::text_dim());
    desc_lbl.set_margin(24, 6, 24, 0);
    card.add(&desc_lbl);

    // Check for Updates button — opens the Software Update app
    let btn = ui::Button::new(i18n::t("Check for Updates"));
    btn.set_dock(ui::DOCK_TOP);
    btn.set_size(180, 32);
    btn.set_margin(24, 12, 24, 16);
    card.add(&btn);

    btn.on_click(|_| {
        process::spawn(UPDATER_PATH, UPDATER_PATH);
    });

    parent.add(&panel);
    panel.id()
}
