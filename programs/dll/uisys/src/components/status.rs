//! StatusIndicator — colored status dot with optional label.

use crate::draw;
use crate::theme;

const DOT_SIZE: u32 = 10;

/// Render a status indicator dot with optional label.
/// status: 0=green (online), 1=yellow (warning), 2=red (error), 3=gray (offline).
pub extern "C" fn status_render(
    win: u32, x: i32, y: i32, status: u8,
    label: *const u8, label_len: u32, font_size: u16,
) {
    let color = match status {
        0 => theme::SUCCESS(),
        1 => theme::WARNING(),
        2 => theme::DESTRUCTIVE(),
        _ => theme::TEXT_DISABLED(),
    };

    let size = if font_size == 0 { 13u16 } else { font_size };

    // Scale dot proportionally to font size
    let dot = if size < 12 { 8u32 } else { DOT_SIZE };
    let dot_r = dot / 2;
    let dot_y = y + (size as i32 - dot as i32) / 2;
    draw::fill_rounded_rect(win, x, dot_y, dot, dot, dot_r, color);

    // Label text
    if !label.is_null() && label_len > 0 {
        let label_slice = unsafe { core::slice::from_raw_parts(label, label_len as usize) };
        let text_x = x + dot as i32 + 4;
        draw::draw_text_sized(win, text_x, y, theme::TEXT(), label_slice, size);
    }
}
