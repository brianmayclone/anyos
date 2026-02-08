//! SplitView — resizable split panels with a draggable divider.

use crate::draw;
use crate::theme;

/// Divider width in pixels.
const DIVIDER_WIDTH: u32 = 1;
/// Divider hit target width (wider than visual for easier grabbing).
const DIVIDER_HIT_WIDTH: u32 = 8;

/// Render the split view divider.
/// `split_x` is the x-offset of the divider relative to the split view origin.
pub extern "C" fn splitview_render(
    win: u32, x: i32, y: i32, _w: u32, h: u32,
    split_x: u32,
) {
    let divider_x = x + split_x as i32;
    draw::fill_rect(win, divider_x, y, DIVIDER_WIDTH, h, theme::SEPARATOR);
}

/// Hit test for the divider drag handle.
/// Returns 1 if the mouse position is within the divider grab zone.
pub extern "C" fn splitview_hit_test_divider(
    x: i32, _y: i32, _w: u32, h: u32,
    split_x: u32, mx: i32, my: i32,
) -> u32 {
    let divider_x = x + split_x as i32;
    let half = (DIVIDER_HIT_WIDTH / 2) as i32;
    if mx >= divider_x - half && mx <= divider_x + half
        && my >= _y && my < _y + h as i32
    {
        1
    } else {
        0
    }
}

/// Clamp the split position to respect minimum panel widths.
/// Returns the clamped split_x value.
pub extern "C" fn splitview_clamp(
    split_x: u32, min_left: u32, min_right: u32, total_w: u32,
) -> u32 {
    let max_x = if total_w > min_right { total_w - min_right } else { 0 };
    if split_x < min_left {
        min_left
    } else if split_x > max_x {
        max_x
    } else {
        split_x
    }
}
