//! Label — static text display with alignment and sizing options.

use crate::draw;
use crate::theme;

/// Render a label.
/// align: 0=left, 1=center, 2=right
pub extern "C" fn label_render(
    win: u32, x: i32, y: i32,
    text: *const u8, text_len: u32,
    color: u32, font_size: u16, align: u8,
) {
    if text.is_null() || text_len == 0 { return; }
    let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize + 1) };

    let draw_x = match align {
        1 => {
            // Center: estimate text width (proportional ~7px per char)
            let est_w = text_len as i32 * char_width(font_size);
            x - est_w / 2
        }
        2 => {
            let est_w = text_len as i32 * char_width(font_size);
            x - est_w
        }
        _ => x,
    };

    if font_size <= 13 {
        draw::draw_text_mono(win, draw_x, y, color, text_slice);
    } else {
        draw::draw_text(win, draw_x, y, color, text_slice);
    }
}

/// Measure text dimensions.
pub extern "C" fn label_measure(
    text: *const u8, text_len: u32,
    font_size: u16, out_w: *mut u32, out_h: *mut u32,
) {
    if out_w.is_null() || out_h.is_null() { return; }
    let w = text_len * char_width(font_size) as u32;
    let h = line_height(font_size);
    unsafe {
        *out_w = w;
        *out_h = h;
    }
}

/// Render with ellipsis truncation if text exceeds max_width.
pub extern "C" fn label_render_ellipsis(
    win: u32, x: i32, y: i32,
    text: *const u8, text_len: u32,
    color: u32, font_size: u16, max_width: u32,
) {
    if text.is_null() || text_len == 0 { return; }
    let cw = char_width(font_size) as u32;
    let max_chars = if cw > 0 { max_width / cw } else { text_len };

    if text_len <= max_chars {
        label_render(win, x, y, text, text_len, color, font_size, 0);
    } else if max_chars > 3 {
        // Render truncated text + "..."
        let trunc_len = max_chars - 3;
        let slice = unsafe { core::slice::from_raw_parts(text, trunc_len as usize) };
        draw::draw_text(win, x, y, color, slice);
        // Draw "..."
        let dots_x = x + (trunc_len as i32) * char_width(font_size);
        let dots = b"...\0";
        draw::draw_text(win, dots_x, y, color, dots);
    }
}

/// Render multi-line text with word wrapping.
pub extern "C" fn label_render_multiline(
    win: u32, x: i32, y: i32,
    text: *const u8, text_len: u32,
    color: u32, font_size: u16, max_width: u32, line_spacing: u32,
) {
    if text.is_null() || text_len == 0 { return; }
    let cw = char_width(font_size) as u32;
    let lh = line_height(font_size) + line_spacing;
    let chars_per_line = if cw > 0 { (max_width / cw).max(1) } else { 80 };

    let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
    let mut offset = 0u32;
    let mut line_y = y;

    while offset < text_len {
        let remaining = text_len - offset;
        let line_len = remaining.min(chars_per_line);
        let slice = &text_slice[offset as usize..(offset + line_len) as usize];
        draw::draw_text(win, x, line_y, color, slice);
        offset += line_len;
        line_y += lh as i32;
    }
}

fn char_width(font_size: u16) -> i32 {
    if font_size <= 13 { 8 } else { 7 }
}

fn line_height(font_size: u16) -> u32 {
    if font_size <= 13 { 16 } else { font_size as u32 + 4 }
}
