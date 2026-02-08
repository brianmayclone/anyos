//! Badge — notification count circle or dot indicator.

use crate::draw;
use crate::theme;

/// Render a badge (notification count or dot).
/// count: the number to display inside the badge.
/// dot_mode: if nonzero, render as a small dot without text.
pub extern "C" fn badge_render(win: u32, x: i32, y: i32, count: u32, dot_mode: u32) {
    if dot_mode != 0 {
        // Small 10px dot
        let size = 10u32;
        draw::fill_rounded_rect(win, x, y, size, size, size / 2, theme::BADGE_RED);
        return;
    }

    // Determine text and badge width
    let mut buf = [0u8; 8];
    let text_len = format_u32(count, &mut buf);

    // Badge dimensions: at least 20px wide, grows for multi-digit numbers
    let char_w = 7i32;
    let text_w = text_len as i32 * char_w;
    let h = 20u32;
    let w = if text_w + 10 > h as i32 { (text_w + 10) as u32 } else { h };

    // Red pill background
    draw::fill_rounded_rect(win, x, y, w, h, h / 2, theme::BADGE_RED);

    // White centered text
    let text_x = x + (w as i32 - text_w) / 2;
    let text_y = y + (h as i32 - 16) / 2;
    draw::draw_text(win, text_x, text_y, theme::CHECK_MARK, &buf[..text_len + 1]);
}

/// Format a u32 into a decimal string (null-terminated).
/// Returns the number of digit characters written (not counting NUL).
fn format_u32(mut val: u32, buf: &mut [u8; 8]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        buf[1] = 0;
        return 1;
    }
    let mut tmp = [0u8; 7];
    let mut i = 0usize;
    while val > 0 && i < 7 {
        tmp[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    // Reverse into buf
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    buf[i] = 0;
    i
}
