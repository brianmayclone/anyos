//! GroupBox — labeled border container for grouping related controls.

use crate::draw;
use crate::theme;

const GROUPBOX_CORNER: u32 = 6;
const LABEL_INSET: i32 = 12;
const LABEL_PAD: u32 = 4;

/// Render a group box with a title label embedded in the top border.
pub extern "C" fn groupbox_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    title: *const u8, title_len: u32,
) {
    let has_title = !title.is_null() && title_len > 0;
    let title_w = if has_title {
        let ts = unsafe { core::slice::from_raw_parts(title, title_len as usize) };
        let (tw, _) = draw::text_size(ts);
        tw + LABEL_PAD * 2
    } else { 0 };

    // Draw the border rectangle
    draw::draw_border(win, x, y + 8, w, h - 8, theme::CARD_BORDER());

    // Draw rounded corners (fill small rects at corners for visual polish)
    draw::fill_rounded_rect(win, x, y + 8, GROUPBOX_CORNER, GROUPBOX_CORNER, GROUPBOX_CORNER / 2, theme::CARD_BORDER());

    if has_title {
        // Clear the top border where the label will sit
        let label_x = x + LABEL_INSET;
        draw::fill_rect(win, label_x, y + 8, title_w, 1, theme::WINDOW_BG());

        // Draw title text
        let title_slice = unsafe { core::slice::from_raw_parts(title, title_len as usize) };
        let text_x = label_x + LABEL_PAD as i32;
        let text_y = y;
        draw::draw_text(win, text_x, text_y, theme::TEXT_SECONDARY(), title_slice);
    }
}
