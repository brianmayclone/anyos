//! Common layout helpers for the Settings app.
//!
//! Provides reusable building blocks for constructing settings page UIs with
//! consistent styling: page headers, section cards, setting rows, info rows,
//! toggles, and separators. All helpers follow the dark-mode theme
//! (background 0xFF1E1E1E, card 0xFF2D2D30, accent 0xFF007AFF).

use libanyui_client as ui;
#[allow(unused_imports)]
use ui::Widget;

// ── Theme colour constants ─────────────────────────────────────────────────

/// Page/panel background.
pub const BG: u32 = 0xFF1E1E1E;
/// Card background.
pub const CARD_BG: u32 = 0xFF2D2D30;
/// Primary text colour.
pub const TEXT: u32 = 0xFFCCCCCC;
/// Dimmed / secondary text colour.
pub const TEXT_DIM: u32 = 0xFF969696;
/// Accent colour (links, selection highlights).
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
    title_lbl.set_text_color(0xFFFFFFFF);
    title_lbl.set_margin(24, 16, 24, 0);
    panel.add(&title_lbl);

    if !subtitle.is_empty() {
        let sub = ui::Label::new(subtitle);
        sub.set_dock(ui::DOCK_TOP);
        sub.set_size(600, 22);
        sub.set_font_size(12);
        sub.set_text_color(0xFF808080);
        sub.set_margin(24, 2, 24, 8);
        panel.add(&sub);
    }
}

// ── Section cards ───────────────────────────────────────────────────────────

/// Build a fixed-height section card inside `parent`.
///
/// The card is docked to the top with standard margins and the dark card
/// background colour (0xFF2D2D30).
pub fn build_section_card(parent: &ui::View, height: u32) -> ui::Card {
    let card = ui::Card::new();
    card.set_dock(ui::DOCK_TOP);
    card.set_size(600, height);
    card.set_margin(24, 8, 24, 8);
    card.set_color(0xFF2D2D30);
    parent.add(&card);
    card
}

/// Build a section card that auto-sizes based on its content.
///
/// Same styling as [`build_section_card`] but with height 0 and auto-size
/// enabled so the card grows to fit its children.
pub fn build_auto_card(parent: &ui::View) -> ui::Card {
    let card = ui::Card::new();
    card.set_dock(ui::DOCK_TOP);
    card.set_size(600, 0);
    card.set_margin(24, 8, 24, 8);
    card.set_color(0xFF2D2D30);
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
    lbl.set_text_color(0xFFCCCCCC);
    lbl.set_font_size(13);
    row.add(&lbl);

    card.add(&row);
    row
}

// ── Info rows ───────────────────────────────────────────────────────────────

/// Build a read-only information row (label on the left, value on the right).
///
/// The value is displayed in a muted colour (0xFF969696).
pub fn build_info_row(card: &ui::Card, label_text: &str, value_text: &str, first: bool) {
    let row = build_setting_row(card, label_text, first);
    let val = ui::Label::new(value_text);
    val.set_position(200, 12);
    val.set_size(340, 20);
    val.set_text_color(0xFF969696);
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
///
/// Returns the [`Toggle`](ui::Toggle) so the caller can attach event handlers.
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
