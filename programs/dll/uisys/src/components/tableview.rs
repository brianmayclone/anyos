//! TableView — scrollable table with selectable rows and column headers.

use crate::draw;
use crate::theme;

/// Render the table view background and frame.
/// `num_rows` is the total number of data rows.
/// `selected_row` is the currently selected row index (u32::MAX = none).
/// `scroll_offset` is the pixel offset of the content scrolled upward.
/// `row_height` is the height of each row in pixels.
pub extern "C" fn tableview_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    _num_rows: u32, _selected_row: u32, _scroll_offset: u32, _row_height: u32,
) {
    // Background
    draw::fill_rect(win, x, y, w, h, theme::WINDOW_BG);
    // Border
    draw::draw_border(win, x, y, w, h, theme::CARD_BORDER);
}

/// Render a single table row.
/// `text` / `text_len`: null-terminated row text.
/// `selected`: nonzero if this row is selected.
/// `alt`: nonzero if this is an alternating (even) row for subtle striping.
pub extern "C" fn tableview_render_row(
    win: u32, x: i32, y: i32, w: u32, row_height: u32,
    text: *const u8, text_len: u32,
    selected: u32, alt: u32,
) {
    // Row background
    let bg = if selected != 0 {
        theme::SELECTION
    } else if alt != 0 {
        theme::CARD_BG
    } else {
        theme::WINDOW_BG
    };
    draw::fill_rect(win, x, y, w, row_height, bg);

    // Bottom separator
    draw::fill_rect(win, x, y + row_height as i32 - 1, w, 1, theme::SEPARATOR);

    // Text
    if !text.is_null() && text_len > 0 {
        let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
        let text_x = x + 8;
        let (_, th) = draw::text_size(text_slice);
        let text_y = y + (row_height as i32 - th as i32) / 2;
        let fg = if selected != 0 { 0xFFFFFFFF } else { theme::TEXT };
        draw::draw_text(win, text_x, text_y, fg, text_slice);
    }
}

/// Hit test: given a mouse y coordinate, return the row index that was clicked.
/// Returns the 0-based row index accounting for scroll offset.
/// Caller must bounds-check against actual row count.
pub extern "C" fn tableview_hit_test_row(
    y: i32, row_height: u32, scroll_offset: u32, my: i32,
) -> u32 {
    let relative = my - y + scroll_offset as i32;
    if relative < 0 {
        return 0xFFFFFFFF;
    }
    (relative as u32) / row_height
}

/// Render a column header cell.
pub extern "C" fn tableview_render_header(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    text: *const u8, text_len: u32,
) {
    // Header background (slightly elevated)
    draw::fill_rect(win, x, y, w, h, theme::SIDEBAR_BG);

    // Bottom separator
    draw::fill_rect(win, x, y + h as i32 - 1, w, 1, theme::SEPARATOR);

    // Header text
    if !text.is_null() && text_len > 0 {
        let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
        let text_x = x + 8;
        let (_, th) = draw::text_size(text_slice);
        let text_y = y + (h as i32 - th as i32) / 2;
        draw::draw_text(win, text_x, text_y, theme::TEXT_SECONDARY, text_slice);
    }
}
