//! Toggle — iOS-style on/off switch.

use crate::draw;
use crate::theme;

/// Render a toggle switch.
/// on: 0=off, nonzero=on.
pub extern "C" fn toggle_render(win: u32, x: i32, y: i32, on: u32) {
    let w = theme::TOGGLE_WIDTH;
    let h = theme::TOGGLE_HEIGHT;
    let thumb = theme::TOGGLE_THUMB_SIZE;

    // Track background
    let track_color = if on != 0 { theme::TOGGLE_ON() } else { theme::TOGGLE_OFF() };
    draw::fill_rounded_rect(win, x, y, w, h, h / 2, track_color);

    // Thumb position
    let margin = (h - thumb) / 2;
    let thumb_x = if on != 0 {
        x + (w - thumb - margin) as i32
    } else {
        x + margin as i32
    };
    let thumb_y = y + margin as i32;

    // Thumb circle (approximated as rounded rect)
    draw::fill_rounded_rect(win, thumb_x, thumb_y, thumb, thumb, thumb / 2, theme::TOGGLE_THUMB());
}

/// Hit test for toggle.
pub extern "C" fn toggle_hit_test(x: i32, y: i32, mx: i32, my: i32) -> u32 {
    let w = theme::TOGGLE_WIDTH as i32;
    let h = theme::TOGGLE_HEIGHT as i32;
    if mx >= x && mx < x + w && my >= y && my < y + h { 1 } else { 0 }
}
