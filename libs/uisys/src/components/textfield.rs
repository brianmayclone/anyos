//! TextField — single-line text input with cursor, selection, and placeholder.

use crate::draw;
use crate::theme;

/// Render a text field (backward-compatible, no selection).
/// flags: bit 0 = focused, bit 1 = password mode
pub extern "C" fn textfield_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    text: *const u8, text_len: u32,
    placeholder: *const u8, placeholder_len: u32,
    cursor_pos: u32, flags: u32,
) {
    // Delegate to _ex with no selection
    textfield_render_ex(
        win, x, y, w, h,
        text, text_len,
        placeholder, placeholder_len,
        cursor_pos, flags,
        cursor_pos, cursor_pos,
    );
}

/// Render a text field with selection support.
/// flags: bit 0 = focused, bit 1 = password mode
/// sel_start/sel_end: selection range (char indices, equal = no selection)
pub extern "C" fn textfield_render_ex(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    text: *const u8, text_len: u32,
    placeholder: *const u8, placeholder_len: u32,
    cursor_pos: u32, flags: u32,
    sel_start: u32, sel_end: u32,
) {
    let focused = (flags & 1) != 0;
    let password = (flags & 2) != 0;

    // Background
    draw::fill_rounded_rect(win, x, y, w, h, 4, theme::INPUT_BG());

    // Border
    let border_color = if focused { theme::INPUT_FOCUS() } else { theme::INPUT_BORDER() };
    draw::draw_border(win, x, y, w, h, border_color);

    let pad = 6i32;
    let inner_w = (w as i32 - pad * 2).max(0);
    let (_, th) = draw::text_size(b"Ay");
    let text_y = y + (h as i32 - th as i32) / 2;

    if text_len > 0 && !text.is_null() {
        let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };

        // Build display text (password mode: asterisks)
        let mut pw_buf = [0u8; 257];
        let display: &[u8] = if password {
            let len = (text_len as usize).min(256);
            for i in 0..len { pw_buf[i] = b'*'; }
            &pw_buf[..len]
        } else {
            text_slice
        };

        // Measure cursor position in pixels
        let cursor_px = draw::text_width_n(display, (cursor_pos as usize).min(display.len())) as i32;

        // Compute scroll offset to keep cursor visible
        let scroll_offset = if cursor_px > inner_w - 20 {
            let pad_right = 20i32.min(inner_w / 4);
            (cursor_px - inner_w + pad_right).max(0)
        } else {
            0
        };

        let text_x = x + pad - scroll_offset;

        // Normalize selection range
        let (s_min, s_max) = if sel_start <= sel_end {
            (sel_start as usize, sel_end as usize)
        } else {
            (sel_end as usize, sel_start as usize)
        };
        let s_min = s_min.min(display.len());
        let s_max = s_max.min(display.len());

        // Draw selection highlight
        if s_min != s_max && focused {
            let sx0 = draw::text_width_n(display, s_min) as i32;
            let sx1 = draw::text_width_n(display, s_max) as i32;
            let sel_x = (text_x + sx0).max(x + pad);
            let sel_end_x = (text_x + sx1).min(x + w as i32 - pad);
            let sel_w = (sel_end_x - sel_x).max(0);
            if sel_w > 0 {
                draw::fill_rect(win, sel_x, text_y, sel_w as u32, th, theme::SELECTION());
            }
        }

        // Draw text
        draw::draw_text(win, text_x, text_y, theme::TEXT(), display);

        // Overdraw selected text in white for contrast
        if s_min != s_max && focused && s_max > s_min {
            let sx0 = draw::text_width_n(display, s_min) as i32;
            let sel_tx = text_x + sx0;
            let sel_slice = &display[s_min..s_max];
            draw::draw_text(win, sel_tx, text_y, 0xFFFFFFFF, sel_slice);
        }

        // Draw cursor
        if focused {
            let cx = text_x + cursor_px;
            if cx >= x + pad - 1 && cx < x + w as i32 - pad + 1 {
                draw::fill_rect(win, cx, text_y, 1, th, theme::TEXT());
            }
        }
    } else if !placeholder.is_null() && placeholder_len > 0 {
        let ph_slice = unsafe { core::slice::from_raw_parts(placeholder, placeholder_len as usize) };
        draw::draw_text(win, x + pad, text_y, theme::TEXT_SECONDARY(), ph_slice);

        if focused {
            draw::fill_rect(win, x + pad, text_y, 1, th, theme::TEXT());
        }
    } else if focused {
        draw::fill_rect(win, x + pad, text_y, 1, th, theme::TEXT());
    }
}

/// Hit test for text field.
pub extern "C" fn textfield_hit_test(
    x: i32, y: i32, w: u32, h: u32,
    mx: i32, my: i32,
) -> u32 {
    if mx >= x && mx < x + w as i32 && my >= y && my < y + h as i32 { 1 } else { 0 }
}

/// Calculate cursor position from mouse click x (approximate).
pub extern "C" fn textfield_cursor_from_click(
    x: i32, text_len: u32, mx: i32,
) -> u32 {
    let text_x = x + 6;
    let offset = mx - text_x;
    if offset < 0 { return 0; }
    let approx_pos = (offset / 7) as u32;
    if approx_pos > text_len { text_len } else { approx_pos }
}

/// Calculate cursor position from mouse click x using actual font measurements.
pub extern "C" fn textfield_cursor_from_click_ex(
    x: i32, text_ptr: *const u8, text_len: u32, mx: i32, scroll_offset: i32,
) -> u32 {
    if text_ptr.is_null() || text_len == 0 { return 0; }
    let text_x = x + 6 - scroll_offset;
    let target = mx - text_x;
    if target <= 0 { return 0; }

    let text_slice = unsafe { core::slice::from_raw_parts(text_ptr, text_len as usize) };

    let mut best = 0u32;
    let mut prev_w = 0i32;
    for i in 1..=text_len {
        let w = draw::text_width_n(text_slice, i as usize) as i32;
        let mid = (prev_w + w) / 2;
        if target <= mid {
            return best;
        }
        best = i;
        prev_w = w;
    }
    text_len
}
