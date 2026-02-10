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
    let (_, th) = draw::text_size(b"Ay");
    let text_y = y + (h as i32 - th as i32) / 2;

    if text_len > 0 {
        if password {
            // Draw bullet characters
            let mut buf = [0u8; 257];
            let len = (text_len as usize).min(256);
            for i in 0..len {
                buf[i] = b'*';
            }
            draw::draw_text(win, text_x, text_y, theme::TEXT, &buf[..len]);
        } else {
            let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
            draw::draw_text(win, text_x, text_y, theme::TEXT, text_slice);
        }
    } else if !placeholder.is_null() && placeholder_len > 0 {
        let ph_slice = unsafe { core::slice::from_raw_parts(placeholder, placeholder_len as usize) };
        draw::draw_text(win, text_x, text_y, theme::TEXT_SECONDARY, ph_slice);
    }

    // Cursor
    if focused {
        let cursor_x = if text_len > 0 && cursor_pos > 0 && !text.is_null() {
            let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
            text_x + draw::text_width_n(text_slice, cursor_pos as usize) as i32
        } else {
            text_x
        };
        draw::fill_rect(win, cursor_x, text_y, 1, th, theme::TEXT);
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
    // With proportional font, approximate: use half the average char width
    // For now, estimate based on average ~7px per char at 13px
    let approx_pos = (offset / 7) as u32;
    if approx_pos > text_len { text_len } else { approx_pos }
}
