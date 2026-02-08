//! SearchField — search input field with magnifying glass icon and clear indicator.

use crate::draw;
use crate::theme;

/// Icon area width (space for the magnifying glass icon).
const ICON_WIDTH: i32 = 24;
/// Horizontal padding inside the field.
const FIELD_PAD: i32 = 6;

/// Render a search field.
/// `text` / `text_len`: current search text (null-terminated).
/// `cursor_pos`: cursor position in characters.
/// `focused`: nonzero if the field has keyboard focus.
pub extern "C" fn searchfield_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    text: *const u8, text_len: u32,
    cursor_pos: u32, focused: u32,
) {
    // Background
    draw::fill_rounded_rect(win, x, y, w, h, h / 2, theme::INPUT_BG);

    // Border
    let border_color = if focused != 0 { theme::INPUT_FOCUS } else { theme::INPUT_BORDER };
    draw::draw_border(win, x, y, w, h, border_color);

    // Magnifying glass icon (simple "o-" shape)
    let icon_x = x + FIELD_PAD + 2;
    let icon_y = y + (h as i32 - theme::CHAR_HEIGHT as i32) / 2;
    let icon = b"Q\0"; // Placeholder glyph for search icon
    draw::draw_text_mono(win, icon_x, icon_y, theme::TEXT_SECONDARY, icon);

    let text_x = x + ICON_WIDTH + FIELD_PAD;
    let text_y = y + (h as i32 - theme::CHAR_HEIGHT as i32) / 2;

    if text_len > 0 && !text.is_null() {
        let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize + 1) };
        draw::draw_text_mono(win, text_x, text_y, theme::TEXT, text_slice);
    } else {
        // Placeholder
        let ph = b"Search\0";
        draw::draw_text(win, text_x, text_y, theme::TEXT_DISABLED, ph);
    }

    // Cursor
    if focused != 0 {
        let cursor_x = text_x + cursor_pos as i32 * theme::CHAR_WIDTH as i32;
        draw::fill_rect(win, cursor_x, text_y, 1, theme::CHAR_HEIGHT, theme::TEXT);
    }
}

/// Hit test for the search field.
/// Returns 1 if the mouse position is within the field bounds.
pub extern "C" fn searchfield_hit_test(
    x: i32, y: i32, w: u32, h: u32,
    mx: i32, my: i32,
) -> u32 {
    if mx >= x && mx < x + w as i32 && my >= y && my < y + h as i32 {
        1
    } else {
        0
    }
}
