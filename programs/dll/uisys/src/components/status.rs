//! StatusIndicator — colored status dot with optional label.

use crate::draw;
use crate::theme;

const DOT_SIZE: u32 = 10;

/// Render a status indicator dot with optional label.
/// status: 0=green (online), 1=yellow (warning), 2=red (error), 3=gray (offline).
pub extern "C" fn status_render(
    win: u32, x: i32, y: i32, status: u8,
    label: *const u8, label_len: u32,
) {
    let color = match status {
        0 => theme::SUCCESS,
        1 => theme::WARNING,
        2 => theme::DESTRUCTIVE,
        _ => theme::TEXT_DISABLED,
    };

    // Status dot (circle)
    let dot_r = DOT_SIZE / 2;
    draw::fill_rounded_rect(win, x, y, DOT_SIZE, DOT_SIZE, dot_r, color);

    // Label text
    if !label.is_null() && label_len > 0 {
        let label_slice = unsafe { core::slice::from_raw_parts(label, label_len as usize + 1) };
        let text_x = x + DOT_SIZE as i32 + 6;
        let text_y = y + (DOT_SIZE as i32 - 16) / 2;
        draw::draw_text(win, text_x, text_y, theme::TEXT, label_slice);
    }
}
