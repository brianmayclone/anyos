//! Common layout helpers for the Settings app.
//!
//! Provides reusable building blocks for constructing settings page UIs with
//! consistent styling: page headers, section cards, setting rows, info rows,
//! toggles, and separators.  All colors come from the active theme palette.

use libanyui_client as ui;
#[allow(unused_imports)]
use ui::Widget;

// ── Theme colour accessors ────────────────────────────────────────────────

/// Page/panel background.
pub fn bg() -> u32 { ui::theme::colors().window_bg }
/// Card background.
pub fn card_bg() -> u32 { ui::theme::colors().card_bg }
/// Primary text colour.
pub fn text() -> u32 { ui::theme::colors().text }
/// Dimmed / secondary text colour.
pub fn text_dim() -> u32 { ui::theme::colors().text_secondary }
/// Accent colour (links, selection highlights).
pub fn accent() -> u32 { ui::theme::colors().accent }

// Keep old constants as aliases so pages that still reference them compile.
// TODO: remove once all call sites are migrated.
pub const BG: u32 = 0xFF1E1E1E;
pub const CARD_BG: u32 = 0xFF2D2D30;
pub const TEXT: u32 = 0xFFCCCCCC;
pub const TEXT_DIM: u32 = 0xFF969696;
pub const ACCENT: u32 = 0xFF007AFF;

// ── Page header ─────────────────────────────────────────────────────────────

/// Build a page header with a large title and an optional subtitle.
///
/// Both labels are docked to the top of `panel` with appropriate margins.
pub fn build_page_header(panel: &ui::View, title: &str, subtitle: &str) {
    let title_lbl = ui::Label::new(title);
    title_lbl.set_dock(ui::DOCK_TOP);
    title_lbl.set_size(600, 40);
    title_lbl.set_font_size(24);
    title_lbl.set_text_color(text());
    title_lbl.set_margin(24, 16, 24, 0);
    panel.add(&title_lbl);

    if !subtitle.is_empty() {
        let sub = ui::Label::new(subtitle);
        sub.set_dock(ui::DOCK_TOP);
        sub.set_size(600, 22);
        sub.set_font_size(12);
        sub.set_text_color(text_dim());
        sub.set_margin(24, 2, 24, 8);
        panel.add(&sub);
    }
}

// ── Section cards ───────────────────────────────────────────────────────────

/// Build a fixed-height section card inside `parent`.
pub fn build_section_card(parent: &ui::View, height: u32) -> ui::Card {
    let card = ui::Card::new();
    card.set_dock(ui::DOCK_TOP);
    card.set_size(600, height);
    card.set_margin(24, 8, 24, 8);
    card.set_color(card_bg());
    parent.add(&card);
    card
}

/// Build a section card that auto-sizes based on its content.
pub fn build_auto_card(parent: &ui::View) -> ui::Card {
    let card = ui::Card::new();
    card.set_dock(ui::DOCK_TOP);
    card.set_size(600, 0);
    card.set_margin(24, 8, 24, 8);
    card.set_color(card_bg());
    card.set_auto_size(true);
    parent.add(&card);
    card
}

// ── Setting rows ────────────────────────────────────────────────────────────

/// Build a standard setting row inside a card.
///
/// The row is 44px tall, docked to the top, and contains a left-aligned label.
/// If `first` is `true` an extra top margin is added. Returns the row View so
/// that additional controls (toggles, value labels, etc.) can be placed in it.
pub fn build_setting_row(card: &ui::Card, label_text: &str, first: bool) -> ui::View {
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(552, 44);
    row.set_margin(24, if first { 8 } else { 0 }, 24, 0);

    let lbl = ui::Label::new(label_text);
    lbl.set_position(0, 12);
    lbl.set_size(200, 20);
    lbl.set_text_color(text());
    lbl.set_font_size(13);
    row.add(&lbl);

    card.add(&row);
    row
}

// ── Info rows ───────────────────────────────────────────────────────────────

/// Build a read-only information row (label on the left, value on the right).
pub fn build_info_row(card: &ui::Card, label_text: &str, value_text: &str, first: bool) {
    let row = build_setting_row(card, label_text, first);
    let val = ui::Label::new(value_text);
    val.set_position(200, 12);
    val.set_size(340, 20);
    val.set_text_color(text_dim());
    val.set_font_size(13);
    row.add(&val);
}

/// Build a read-only information row with a custom value colour.
pub fn build_info_row_colored(
    card: &ui::Card,
    label_text: &str,
    value_text: &str,
    color: u32,
    first: bool,
) {
    let row = build_setting_row(card, label_text, first);
    let val = ui::Label::new(value_text);
    val.set_position(200, 12);
    val.set_size(340, 20);
    val.set_text_color(color);
    val.set_font_size(13);
    row.add(&val);
}

// ── Toggle control ──────────────────────────────────────────────────────────

/// Add a toggle switch to an existing setting row, positioned at the right edge.
pub fn add_toggle_to_row(row: &ui::View, initial: bool) -> ui::Toggle {
    let toggle = ui::Toggle::new(initial);
    toggle.set_position(500, 8);
    row.add(&toggle);
    toggle
}

// ── Separator ───────────────────────────────────────────────────────────────

/// Add a thin horizontal divider line inside a card.
pub fn build_separator(card: &ui::Card) {
    let div = ui::Divider::new();
    div.set_dock(ui::DOCK_TOP);
    div.set_size(552, 1);
    div.set_margin(24, 4, 24, 4);
    card.add(&div);
}
