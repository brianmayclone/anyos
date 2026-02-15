//! Card — elevated container with rounded corners and subtle border.

use crate::draw;
use crate::theme;

const CARD_CORNER: u32 = 8;

/// Render a card container.
/// Draws a rounded rectangle with CARD_BG fill and CARD_BORDER outline.
pub extern "C" fn card_render(win: u32, x: i32, y: i32, w: u32, h: u32) {
    // Background
    draw::fill_rounded_rect(win, x, y, w, h, CARD_CORNER, theme::CARD_BG());

    // Rounded border matching the fill shape
    draw::draw_rounded_border(win, x, y, w, h, CARD_CORNER, theme::CARD_BORDER());
}
