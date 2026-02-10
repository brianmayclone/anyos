//! ContextMenu — popup menu with selectable items and separators.

use crate::draw;
use crate::theme;

/// Default context menu item height.
const ITEM_HEIGHT: u32 = 28;
/// Horizontal padding for menu items.
const ITEM_PADDING_H: i32 = 12;
/// Corner radius.
const MENU_CORNER: u32 = 6;
/// Separator height.
const SEPARATOR_HEIGHT: u32 = 9;

/// Render the context menu background and border.
pub extern "C" fn contextmenu_render_bg(
    win: u32, x: i32, y: i32, w: u32, h: u32,
) {
    // Drop shadow
    draw::fill_rounded_rect(win, x + 1, y + 1, w, h, MENU_CORNER, 0xFF0A0A0A);

    // Background
    draw::fill_rounded_rect(win, x, y, w, h, MENU_CORNER, theme::CARD_BG);

    // Border
    draw::draw_border(win, x, y, w, h, theme::CARD_BORDER);
}

/// Render a single context menu item.
/// `highlighted`: nonzero if the mouse is hovering over this item.
pub extern "C" fn contextmenu_render_item(
    win: u32, x: i32, y: i32, w: u32,
    label: *const u8, label_len: u32,
    highlighted: u32,
) {
    let h = ITEM_HEIGHT;

    // Highlight background
    if highlighted != 0 {
        draw::fill_rect(win, x + 4, y, w - 8, h, theme::ACCENT);
    }

    // Label text
    if !label.is_null() && label_len > 0 {
        let label_slice = unsafe { core::slice::from_raw_parts(label, label_len as usize) };
        let text_x = x + ITEM_PADDING_H;
        let (_, th) = draw::text_size(label_slice);
        let text_y = y + (h as i32 - th as i32) / 2;
        let fg = if highlighted != 0 { 0xFFFFFFFF } else { theme::TEXT };
        draw::draw_text(win, text_x, text_y, fg, label_slice);
    }
}

/// Render a separator line between menu items.
pub extern "C" fn contextmenu_render_separator(
    win: u32, x: i32, y: i32, w: u32,
) {
    let sep_y = y + (SEPARATOR_HEIGHT as i32) / 2;
    draw::fill_rect(win, x + 8, sep_y, w - 16, 1, theme::SEPARATOR);
}

/// Hit test for a context menu item.
/// Returns 1 if the mouse is within the item bounds.
/// `item_h`: height of the item (pass 0 for default 28px).
pub extern "C" fn contextmenu_hit_test_item(
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
