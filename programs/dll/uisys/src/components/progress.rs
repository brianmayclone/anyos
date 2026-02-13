//! ProgressBar — horizontal progress indicator with determinate and indeterminate modes.

use crate::draw;
use crate::theme;

const CORNER: u32 = 3;

/// Render a progress bar.
/// percent: 0..100 fill percentage (clamped).
/// indeterminate: if nonzero, render a pulsing/bouncing indicator instead of a fill bar.
pub extern "C" fn progress_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    percent: u32, indeterminate: u32,
) {
    // Track background
    draw::fill_rounded_rect(win, x, y, w, h, CORNER, theme::CONTROL_BG());

    if indeterminate != 0 {
        // Indeterminate mode: draw a fixed-width accent block at 1/3 width, centered.
        // In a real OS this would animate; here we show a static indicator at center.
        let bar_w = w / 3;
        let bar_x = x + (w as i32 - bar_w as i32) / 2;
        draw::fill_rounded_rect(win, bar_x, y, bar_w, h, CORNER, theme::ACCENT());
    } else {
        // Determinate mode
        let clamped = if percent > 100 { 100 } else { percent };
        if clamped > 0 {
            let fill_w = (w * clamped) / 100;
            let fill_w = if fill_w < CORNER * 2 { CORNER * 2 } else { fill_w };
            draw::fill_rounded_rect(win, x, y, fill_w, h, CORNER, theme::ACCENT());
        }
    }
}
