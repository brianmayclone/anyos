//! TextField — single-line text input with cursor and placeholder.

use crate::draw;
use crate::theme;

/// Render a text field.
/// flags: bit 0 = focused, bit 1 = password mode
pub extern "C" fn textfield_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    text: *const u8, text_len: u32,
    placeholder: *const u8, placeholder_len: u32,
    cursor_pos: u32, flags: u32,
) {
    let focused = (flags & 1) != 0;
    let password = (flags & 2) != 0;

    // Background
    draw::fill_rounded_rect(win, x, y, w, h, 4, theme::INPUT_BG);

    // Border
    let border_color = if focused { theme::INPUT_FOCUS } else { theme::INPUT_BORDER };
    draw::draw_border(win, x, y, w, h, border_color);

    let text_x = x + 6;
    let text_y = y + (h as i32 - 16) / 2;

    if text_len > 0 {
        if password {
            // Draw bullet characters
            let mut buf = [0u8; 257];
            let len = (text_len as usize).min(256);
            for i in 0..len {
                buf[i] = b'*';
            }
            buf[len] = 0;
            draw::draw_text_mono(win, text_x, text_y, theme::TEXT, &buf[..len + 1]);
        } else {
            let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize + 1) };
            draw::draw_text_mono(win, text_x, text_y, theme::TEXT, text_slice);
        }
    } else if !placeholder.is_null() && placeholder_len > 0 {
        let ph_slice = unsafe { core::slice::from_raw_parts(placeholder, placeholder_len as usize + 1) };
        draw::draw_text_mono(win, text_x, text_y, theme::TEXT_SECONDARY, ph_slice);
    }

    // Cursor (blinking handled by caller via flags or timer)
    if focused {
        let cursor_x = text_x + cursor_pos as i32 * 8;
        draw::fill_rect(win, cursor_x, text_y, 1, 16, theme::TEXT);
    }
}

/// Hit test for text field.
pub extern "C" fn textfield_hit_test(
    x: i32, y: i32, w: u32, h: u32,
    mx: i32, my: i32,
) -> u32 {
    if mx >= x && mx < x + w as i32 && my >= y && my < y + h as i32 { 1 } else { 0 }
}

/// Calculate cursor position from mouse click x.
pub extern "C" fn textfield_cursor_from_click(
    x: i32, text_len: u32, mx: i32,
) -> u32 {
    let text_x = x + 6;
    let offset = mx - text_x;
    if offset < 0 { return 0; }
    let pos = (offset / 8) as u32;
    if pos > text_len { text_len } else { pos }
}
