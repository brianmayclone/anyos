//! Toolbar — horizontal button bar for actions.

use crate::draw;
use crate::theme;

/// Render the toolbar background.
pub extern "C" fn toolbar_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
) {
    // Background
    draw::fill_rect(win, x, y, w, h, theme::SIDEBAR_BG());

    // Bottom separator
    draw::fill_rect(win, x, y + h as i32 - 1, w, 1, theme::SEPARATOR());
}

/// Render a toolbar button.
/// `state`: 0=Normal, 1=Hover, 2=Pressed, 3=Disabled
pub extern "C" fn toolbar_render_button(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    label: *const u8, label_len: u32,
    state: u8,
) {
    let (bg, fg) = match state {
        1 => (theme::CONTROL_HOVER(), theme::TEXT()),
        2 => (theme::CONTROL_PRESSED(), theme::TEXT()),
        3 => (0u32, theme::TEXT_DISABLED()),
        _ => (0u32, theme::TEXT()),
    };

    // Button background (only if hover/pressed)
    if bg != 0 {
        draw::fill_rounded_rect(win, x + 2, y + 2, w - 4, h - 4, 4, bg);
    }

    // Label (centered)
    if !label.is_null() && label_len > 0 {
        let label_slice = unsafe { core::slice::from_raw_parts(label, label_len as usize) };
        let (tw, th) = draw::text_size(label_slice);
        let text_x = x + (w as i32 - tw as i32) / 2;
        let text_y = y + (h as i32 - th as i32) / 2;
        draw::draw_text(win, text_x, text_y, fg, label_slice);
    }
}
