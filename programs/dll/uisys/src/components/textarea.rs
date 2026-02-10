//! TextArea — multi-line text editor with optional line numbers and scrolling.

use crate::draw;
use crate::theme;

/// Gutter width when line numbers are shown (space for ~4 digits + padding).
const GUTTER_WIDTH: u32 = 40;
/// Horizontal text padding.
const TEXT_PAD: i32 = 6;

/// Render a multi-line text area.
/// `text` / `text_len`: the full text buffer (newlines as 0x0A).
/// `cursor_pos`: byte offset of the cursor in the text.
/// `scroll_offset`: vertical scroll in pixels.
/// `show_line_nums`: nonzero to display a line number gutter.
/// `focused`: nonzero if the text area has keyboard focus.
pub extern "C" fn textarea_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    text: *const u8, text_len: u32,
    cursor_pos: u32, scroll_offset: u32,
    show_line_nums: u32, focused: u32,
) {
    // Background
    draw::fill_rect(win, x, y, w, h, theme::INPUT_BG);

    // Border
    let border_color = if focused != 0 { theme::INPUT_FOCUS } else { theme::INPUT_BORDER };
    draw::draw_border(win, x, y, w, h, border_color);

    let gutter_w = if show_line_nums != 0 { GUTTER_WIDTH } else { 0 };

    // Gutter background
    if show_line_nums != 0 {
        draw::fill_rect(win, x + 1, y + 1, gutter_w, h - 2, theme::SIDEBAR_BG);
        // Gutter separator
        draw::fill_rect(
            win, x + gutter_w as i32, y + 1,
            1, h - 2, theme::SEPARATOR,
        );
    }

    let content_x = x + gutter_w as i32 + TEXT_PAD;
    // Use TTF font height for line spacing
    let (_, line_h) = draw::text_size(b"Ay");
    let line_h = line_h.max(1);
    let visible_lines = h / line_h;
    let first_visible_line = scroll_offset / line_h;

    if text.is_null() || text_len == 0 {
        // Empty state: show cursor if focused
        if focused != 0 {
            draw::fill_rect(
                win, content_x, y + 2,
                1, line_h, theme::TEXT,
            );
        }
        // Line number 1
        if show_line_nums != 0 {
            let num_buf = b"1\0";
            let (nw, _) = draw::text_size(b"1");
            let num_x = x + gutter_w as i32 - nw as i32 - 4;
            draw::draw_text(win, num_x, y + 2, theme::TEXT_SECONDARY, num_buf);
        }
        return;
    }

    let text_slice = unsafe { core::slice::from_raw_parts(text, text_len as usize) };

    // Determine cursor line and column
    let mut cursor_line = 0u32;
    let mut cursor_col = 0u32;
    {
        let mut ln = 0u32;
        let mut col = 0u32;
        for i in 0..text_len {
            if i == cursor_pos {
                cursor_line = ln;
                cursor_col = col;
            }
            if text_slice[i as usize] == b'\n' {
                ln += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        if cursor_pos == text_len {
            cursor_line = ln;
            cursor_col = col;
        }
    }

    // Render visible lines
    let mut line_num = 0u32;
    let mut i = 0u32;
    while i < text_len {
        // Find end of current line
        let mut line_end = i;
        while line_end < text_len && text_slice[line_end as usize] != b'\n' {
            line_end += 1;
        }

        // Check if this line is visible
        if line_num >= first_visible_line && line_num < first_visible_line + visible_lines + 1 {
            let draw_y = y + 2 + ((line_num - first_visible_line) * line_h) as i32;

            // Clip: only draw if within bounds
            if draw_y >= y && draw_y + line_h as i32 <= y + h as i32 {
                // Line number
                if show_line_nums != 0 {
                    let display_num = line_num + 1;
                    let mut num_buf = [0u8; 8];
                    let num_len = format_u32(display_num, &mut num_buf);
                    let (nw, _) = draw::text_size(&num_buf[..num_len]);
                    let num_x = x + gutter_w as i32 - nw as i32 - 4;
                    draw::draw_text(
                        win, num_x, draw_y,
                        theme::TEXT_SECONDARY, &num_buf[..num_len + 1],
                    );
                }

                // Line text
                let line_len = line_end - i;
                if line_len > 0 {
                    let line_slice = &text_slice[i as usize..line_end as usize];
                    draw::draw_text(
                        win, content_x, draw_y,
                        theme::TEXT, line_slice,
                    );
                }

                // Cursor on this line
                if focused != 0 && line_num == cursor_line {
                    let cx = if cursor_col > 0 {
                        let line_start = i as usize;
                        let cursor_byte = line_start + cursor_col as usize;
                        let before_cursor = &text_slice[line_start..cursor_byte.min(line_end as usize)];
                        content_x + draw::text_width_n(before_cursor, before_cursor.len()) as i32
                    } else {
                        content_x
                    };
                    draw::fill_rect(win, cx, draw_y, 1, line_h, theme::TEXT);
                }
            }
        }

        line_num += 1;
        i = line_end + 1; // skip past newline
        if line_end == text_len {
            break; // no trailing newline
        }
    }
}

/// Hit test for the text area.
/// Returns 1 if the mouse position is within the text area bounds.
pub extern "C" fn textarea_hit_test(
    x: i32, y: i32, w: u32, h: u32,
    mx: i32, my: i32,
) -> u32 {
    if mx >= x && mx < x + w as i32 && my >= y && my < y + h as i32 {
        1
    } else {
        0
    }
}

/// Format a u32 into a decimal string buffer. Returns the number of characters written.
/// The buffer must be at least 8 bytes. The result is null-terminated.
fn format_u32(mut val: u32, buf: &mut [u8; 8]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        buf[1] = 0;
        return 1;
    }
    let mut tmp = [0u8; 8];
    let mut len = 0usize;
    while val > 0 && len < 7 {
        tmp[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    // Reverse into buf
    for i in 0..len {
        buf[i] = tmp[len - 1 - i];
    }
    buf[len] = 0;
    len
}
