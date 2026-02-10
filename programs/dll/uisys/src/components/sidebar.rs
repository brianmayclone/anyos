//! Sidebar — navigation panel with selectable items and section headers.

use crate::draw;
use crate::theme;

/// Default sidebar item height.
const ITEM_HEIGHT: u32 = 32;
/// Header height.
const HEADER_HEIGHT: u32 = 28;

/// Render the sidebar background.
pub extern "C" fn sidebar_render_bg(
    win: u32, x: i32, y: i32, w: u32, h: u32,
) {
    draw::fill_rect(win, x, y, w, h, theme::SIDEBAR_BG);
    // Right-edge separator
    draw::fill_rect(win, x + w as i32 - 1, y, 1, h, theme::SEPARATOR);
}

/// Render a sidebar navigation item.
/// `selected`: nonzero if this item is the active/selected item.
pub extern "C" fn sidebar_render_item(
    win: u32, x: i32, y: i32, w: u32,
    text: *const u8, text_len: u32,
    selected: u32,
) {
    let h = ITEM_HEIGHT;
    let padding = 12;

    // Selected highlight
    if selected != 0 {
        draw::fill_rounded_rect(
            win, x + 4, y + 1, w - 8, h - 2,
            4, theme::SELECTION,
        );
    }

    // Item text
    if !text.is_null() && text_len > 0 {
        let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
        let text_x = x + padding;
        let (_, th) = draw::text_size(text_slice);
        let text_y = y + (h as i32 - th as i32) / 2;
        let fg = if selected != 0 { 0xFFFFFFFF } else { theme::TEXT };
        draw::draw_text(win, text_x, text_y, fg, text_slice);
    }
}

/// Render a sidebar section header (small uppercase label).
pub extern "C" fn sidebar_render_header(
    win: u32, x: i32, y: i32, _w: u32,
    text: *const u8, text_len: u32,
) {
    if !text.is_null() && text_len > 0 {
        let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
        let text_x = x + 12;
        let (_, th) = draw::text_size(text_slice);
        let text_y = y + (HEADER_HEIGHT as i32 - th as i32) / 2;
        draw::draw_text(win, text_x, text_y, theme::TEXT_SECONDARY, text_slice);
    }
}

/// Hit test: returns 1 if the mouse position is within a sidebar item region.
/// `item_h` is the height of each item (pass 0 for default 32px).
pub extern "C" fn sidebar_hit_test_item(
    x: i32, y: i32, w: u32, item_h: u32,
    mx: i32, my: i32,
) -> u32 {
    let h = if item_h == 0 { ITEM_HEIGHT } else { item_h };
    if mx >= x && mx < x + w as i32 && my >= y && my < y + h as i32 {
        1
    } else {
        0
    }
}
