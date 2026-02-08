//! ScrollView — generic scrollable area with scrollbar rendering.

use crate::draw;
use crate::theme;

/// Scrollbar track width in pixels.
const SCROLLBAR_WIDTH: u32 = 8;
/// Minimum scrollbar thumb height in pixels.
const THUMB_MIN_H: u32 = 20;

/// Render a vertical scrollbar for a scrollable region.
/// `x, y, w, h` define the scroll view bounds.
/// `content_h` is the total content height.
/// `scroll_offset` is the current scroll position in pixels.
pub extern "C" fn scrollview_render_scrollbar(
    win: u32, x: i32, y: i32, _w: u32, h: u32,
    content_h: u32, scroll_offset: u32,
) {
    if content_h <= h {
        return; // No scrollbar needed
    }

    let bar_x = x + _w as i32 - SCROLLBAR_WIDTH as i32;

    // Track
    draw::fill_rect(win, bar_x, y, SCROLLBAR_WIDTH, h, theme::SCROLLBAR_TRACK);

    // Thumb
    let (thumb_y, thumb_h) = thumb_pos_and_size(h, content_h, scroll_offset);
    draw::fill_rounded_rect(
        win, bar_x + 1, y + thumb_y as i32,
        SCROLLBAR_WIDTH - 2, thumb_h,
        (SCROLLBAR_WIDTH - 2) / 2, theme::SCROLLBAR,
    );
}

/// Hit test: returns 1 if the mouse position is within the scrollbar track area.
pub extern "C" fn scrollview_hit_test_scrollbar(
    x: i32, y: i32, w: u32, h: u32,
    mx: i32, my: i32,
) -> u32 {
    let bar_x = x + w as i32 - SCROLLBAR_WIDTH as i32;
    if mx >= bar_x && mx < x + w as i32 && my >= y && my < y + h as i32 {
        1
    } else {
        0
    }
}

/// Compute the scrollbar thumb position and height.
/// Returns (thumb_y_offset, thumb_height) relative to the scrollbar track top.
pub extern "C" fn scrollview_thumb_pos(
    h: u32, content_h: u32, scroll_offset: u32,
) -> u64 {
    let (ty, th) = thumb_pos_and_size(h, content_h, scroll_offset);
    // Pack two u32 values into a u64: high = thumb_y, low = thumb_h
    ((ty as u64) << 32) | (th as u64)
}

/// Internal helper: compute thumb y-offset and height.
fn thumb_pos_and_size(h: u32, content_h: u32, scroll_offset: u32) -> (u32, u32) {
    if content_h <= h {
        return (0, h);
    }
    let thumb_h = ((h as u64 * h as u64) / content_h as u64).max(THUMB_MIN_H as u64) as u32;
    let thumb_h = thumb_h.min(h);
    let max_scroll = content_h - h;
    let track_range = h - thumb_h;
    let thumb_y = if max_scroll > 0 {
        ((scroll_offset as u64 * track_range as u64) / max_scroll as u64) as u32
    } else {
        0
    };
    (thumb_y, thumb_h)
}
