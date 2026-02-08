//! Tooltip — small hover popup displaying informational text.

use crate::draw;
use crate::theme;

/// Tooltip padding.
const TOOLTIP_PAD_H: u32 = 8;
const TOOLTIP_PAD_V: u32 = 4;
/// Corner radius.
const TOOLTIP_CORNER: u32 = 4;
/// Tooltip background color (slightly lighter than card for contrast).
const TOOLTIP_BG: u32 = 0xFF383838;
/// Tooltip border color.
const TOOLTIP_BORDER: u32 = 0xFF505050;

/// Render a tooltip at the given position.
/// The tooltip is sized automatically based on text length.
/// `x, y` is the top-left anchor point of the tooltip.
pub extern "C" fn tooltip_render(
    win: u32, x: i32, y: i32,
    text: *const u8, text_len: u32,
) {
    if text.is_null() || text_len == 0 {
        return;
    }

    let text_w = text_len * theme::CHAR_WIDTH;
    let w = text_w + TOOLTIP_PAD_H * 2;
    let h = theme::CHAR_HEIGHT + TOOLTIP_PAD_V * 2;

    // Background
    draw::fill_rounded_rect(win, x, y, w, h, TOOLTIP_CORNER, TOOLTIP_BG);

    // Border
    draw::draw_border(win, x, y, w, h, TOOLTIP_BORDER);

    // Text
    let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize + 1) };
    let text_x = x + TOOLTIP_PAD_H as i32;
    let text_y = y + TOOLTIP_PAD_V as i32;
    draw::draw_text_mono(win, text_x, text_y, theme::TEXT, text_slice);
}
