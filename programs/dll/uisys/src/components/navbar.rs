//! NavigationBar — top navigation bar with title and optional back button.

use crate::draw;
use crate::theme;

/// Navigation bar height.
const NAVBAR_HEIGHT: u32 = 44;
/// Back button hit area width.
const BACK_BUTTON_W: u32 = 60;

/// Render a navigation bar.
/// `title` / `title_len`: null-terminated title string.
/// `show_back`: nonzero to display a back button on the left.
pub extern "C" fn navbar_render(
    win: u32, x: i32, y: i32, w: u32,
    title: *const u8, title_len: u32,
    show_back: u32,
) {
    let h = NAVBAR_HEIGHT;

    // Background
    draw::fill_rect(win, x, y, w, h, theme::SIDEBAR_BG);

    // Bottom separator
    draw::fill_rect(win, x, y + h as i32 - 1, w, 1, theme::SEPARATOR);

    // Back button
    if show_back != 0 {
        let back_text = b"< Back\0";
        let back_x = x + 12;
        let (_, bh) = draw::text_size(b"< Back");
        let back_y = y + (h as i32 - bh as i32) / 2;
        draw::draw_text(win, back_x, back_y, theme::ACCENT, back_text);
    }

    // Title (centered)
    if !title.is_null() && title_len > 0 {
        let title_slice = unsafe { core::slice::from_raw_parts(title, title_len as usize + 1) };
        let (tw, th) = draw::text_size(&title_slice[..title_len as usize]);
        let text_x = x + (w as i32 - tw as i32) / 2;
        let text_y = y + (h as i32 - th as i32) / 2;
        draw::draw_text(win, text_x, text_y, theme::TEXT, title_slice);
    }
}

/// Hit test for the back button region.
/// Returns 1 if the click is within the back button area.
pub extern "C" fn navbar_hit_test_back(
    x: i32, y: i32, mx: i32, my: i32,
) -> u32 {
    let h = NAVBAR_HEIGHT as i32;
    if mx >= x && mx < x + BACK_BUTTON_W as i32 && my >= y && my < y + h {
        1
    } else {
        0
    }
}
