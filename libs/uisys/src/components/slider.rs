//! Slider — horizontal value slider with track and thumb.

use crate::draw;
use crate::theme;

const TRACK_HEIGHT: u32 = 4;
const THUMB_SIZE: u32 = 18;

/// Render a horizontal slider.
/// min_val, max_val: the range.
/// value: current value (clamped to range).
/// h: total height of the slider area (for vertical centering).
pub extern "C" fn slider_render(
    win: u32, x: i32, y: i32, w: u32,
    min_val: u32, max_val: u32, value: u32, h: u32,
) {
    let range = max_val.saturating_sub(min_val);
    let clamped = value.clamp(min_val, max_val);

    let track_y = y + (h as i32 - TRACK_HEIGHT as i32) / 2;
    let usable_w = if w > THUMB_SIZE { w - THUMB_SIZE } else { 1 };

    // Thumb position (fraction along the track)
    let frac = if range > 0 {
        ((clamped - min_val) * usable_w) / range
    } else {
        0
    };
    let thumb_x = x + (THUMB_SIZE / 2) as i32 + frac as i32;

    // Track background (full length)
    draw::fill_rounded_rect(win, x, track_y, w, TRACK_HEIGHT, TRACK_HEIGHT / 2, theme::TOGGLE_OFF());

    // Filled portion (left of thumb) in accent color
    let filled_w = (thumb_x - x) as u32;
    if filled_w > 0 {
        draw::fill_rounded_rect(win, x, track_y, filled_w, TRACK_HEIGHT, TRACK_HEIGHT / 2, theme::ACCENT());
    }

    // Thumb circle
    let thumb_draw_x = thumb_x - (THUMB_SIZE / 2) as i32;
    let thumb_draw_y = y + (h as i32 - THUMB_SIZE as i32) / 2;
    draw::fill_rounded_rect(
        win,
        thumb_draw_x,
        thumb_draw_y,
        THUMB_SIZE,
        THUMB_SIZE,
        THUMB_SIZE / 2,
        theme::TOGGLE_THUMB(),
    );
}

/// Hit test: returns 1 if (mx,my) is inside the slider track area.
pub extern "C" fn slider_hit_test(
    x: i32, y: i32, w: u32, h: u32,
    mx: i32, my: i32,
) -> u32 {
    if mx >= x && mx < x + w as i32 && my >= y && my < y + h as i32 { 1 } else { 0 }
}

/// Compute the value from a mouse x position.
/// Returns the value corresponding to the mouse position within the slider track.
pub extern "C" fn slider_value_from_x(
    x: i32, w: u32,
    min_val: u32, max_val: u32,
    mx: i32,
) -> u32 {
    let usable_w = if w > THUMB_SIZE { w - THUMB_SIZE } else { 1 };
    let offset = mx - x - (THUMB_SIZE / 2) as i32;
    if offset <= 0 {
        return min_val;
    }
    if offset >= usable_w as i32 {
        return max_val;
    }
    let range = max_val.saturating_sub(min_val);
    min_val + ((offset as u32) * range) / usable_w
}
