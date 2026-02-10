//! Label — static text display with alignment and sizing options.

use crate::draw;

/// Render a label.
/// align: 0=left, 1=center, 2=right
pub extern "C" fn label_render(
    win: u32, x: i32, y: i32,
    text: *const u8, text_len: u32,
    color: u32, font_size: u16, align: u8,
) {
    if text.is_null() || text_len == 0 { return; }

    let draw_x = match align {
        1 => {
            // Center: measure with TTF
            let mut tw: u32 = 0;
            let mut th: u32 = 0;
            draw::measure_text(0, font_size, text, text_len, &mut tw, &mut th);
            x - tw as i32 / 2
        }
        2 => {
            let mut tw: u32 = 0;
            let mut th: u32 = 0;
            draw::measure_text(0, font_size, text, text_len, &mut tw, &mut th);
            x - tw as i32
        }
        _ => x,
    };

    // Always use TTF rendering with the specified font size
    draw::draw_text_ex(win, draw_x, y, color, 0, font_size, text);
}

/// Measure text dimensions.
pub extern "C" fn label_measure(
    text: *const u8, text_len: u32,
    font_size: u16, out_w: *mut u32, out_h: *mut u32,
) {
    if out_w.is_null() || out_h.is_null() { return; }
    draw::measure_text(0, font_size, text, text_len, out_w, out_h);
}

/// Render with ellipsis truncation if text exceeds max_width.
pub extern "C" fn label_render_ellipsis(
    win: u32, x: i32, y: i32,
    text: *const u8, text_len: u32,
    color: u32, font_size: u16, max_width: u32,
) {
    if text.is_null() || text_len == 0 { return; }

    // Measure full text width
    let mut full_w: u32 = 0;
    let mut _h: u32 = 0;
    draw::measure_text(0, font_size, text, text_len, &mut full_w, &mut _h);

    if full_w <= max_width {
        // Text fits — render normally
        draw::draw_text_ex(win, x, y, color, 0, font_size, text);
    } else {
        // Need truncation — find how many chars fit with "..."
        let dots = b"...\0";
        let mut dots_w: u32 = 0;
        draw::measure_text(0, font_size, dots.as_ptr(), 3, &mut dots_w, &mut _h);
        let avail = if max_width > dots_w { max_width - dots_w } else { 0 };

        // Binary-search-like: find longest prefix that fits in `avail`
        let mut trunc_len = text_len;
        while trunc_len > 0 {
            let mut tw: u32 = 0;
            draw::measure_text(0, font_size, text, trunc_len, &mut tw, &mut _h);
            if tw <= avail { break; }
            trunc_len -= 1;
        }

        if trunc_len > 0 {
            // Render truncated prefix
            let prefix = unsafe { core::slice::from_raw_parts(text, trunc_len as usize) };
            draw::draw_text_sized(win, x, y, color, prefix, font_size);

            // Measure actual width of rendered prefix for dot placement
            let mut prefix_w: u32 = 0;
            draw::measure_text(0, font_size, text, trunc_len, &mut prefix_w, &mut _h);
            let dots_slice: &[u8] = b"...";
            draw::draw_text_sized(win, x + prefix_w as i32, y, color, dots_slice, font_size);
        }
    }
}

/// Render multi-line text with word wrapping.
pub extern "C" fn label_render_multiline(
    win: u32, x: i32, y: i32,
    text: *const u8, text_len: u32,
    color: u32, font_size: u16, max_width: u32, line_spacing: u32,
) {
    if text.is_null() || text_len == 0 { return; }

    // Estimate line height from font measurement
    let sample = b"Mg\0";
    let mut _sw: u32 = 0;
    let mut line_h: u32 = 0;
    draw::measure_text(0, font_size, sample.as_ptr(), 2, &mut _sw, &mut line_h);
    if line_h == 0 { line_h = font_size as u32 + 4; }
    let lh = line_h + line_spacing;

    // Estimate average char width for line-breaking
    let cw = if font_size <= 13 { 7u32 } else if font_size <= 16 { 9 } else { 11 };
    let chars_per_line = if cw > 0 { (max_width / cw).max(1) } else { 80 };

    let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };
    let mut offset = 0u32;
    let mut line_y = y;

    while offset < text_len {
        let remaining = text_len - offset;
        let line_len = remaining.min(chars_per_line);
        let slice = &text_slice[offset as usize..(offset + line_len) as usize];
        draw::draw_text_sized(win, x, line_y, color, slice, font_size);
        offset += line_len;
        line_y += lh as i32;
    }
}
