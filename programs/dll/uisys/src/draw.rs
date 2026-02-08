//! Drawing helper functions wrapping syscalls.

use crate::syscall;

/// Fill a rectangle.
#[inline(always)]
pub fn fill_rect(win: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    syscall::win_fill_rect(win, x, y, w, h, color);
}

/// Draw a proportional-font string (null-terminated in caller memory).
/// Caller must provide a null-terminated buffer.
#[inline(always)]
pub fn draw_text(win: u32, x: i32, y: i32, color: u32, text: &[u8]) {
    // Text must be null-terminated; use the slice pointer directly.
    // The syscall reads until NUL.
    syscall::win_draw_text(win, x, y, color, text.as_ptr());
}

/// Draw monospace text.
#[inline(always)]
pub fn draw_text_mono(win: u32, x: i32, y: i32, color: u32, text: &[u8]) {
    syscall::win_draw_text_mono(win, x, y, color, text.as_ptr());
}

/// Draw a filled rounded rectangle (approximated with corner rects removed).
pub fn fill_rounded_rect(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    if r == 0 || w < r * 2 || h < r * 2 {
        fill_rect(win, x, y, w, h, color);
        return;
    }
    // Main body (full width, reduced height)
    fill_rect(win, x, y + r as i32, w, h - r * 2, color);
    // Top strip (narrower)
    fill_rect(win, x + r as i32, y, w - r * 2, r, color);
    // Bottom strip (narrower)
    fill_rect(win, x + r as i32, y + (h - r) as i32, w - r * 2, r, color);
    // Corner fills (small squares for simple approximation)
    // Top-left
    for dy in 0..r {
        let dx = r - isqrt((2 * r * dy - dy * dy).min(r * r));
        if r > dx {
            fill_rect(win, x + dx as i32, y + dy as i32, r - dx, 1, color);
        }
    }
    // Top-right
    for dy in 0..r {
        let dx = r - isqrt((2 * r * dy - dy * dy).min(r * r));
        if r > dx {
            fill_rect(win, x + (w - r) as i32, y + dy as i32, r - dx, 1, color);
        }
    }
    // Bottom-left
    for dy in 0..r {
        let dx = r - isqrt((2 * r * dy - dy * dy).min(r * r));
        if r > dx {
            fill_rect(win, x + dx as i32, y + (h - r + dy) as i32, r - dx, 1, color);
        }
    }
    // Bottom-right
    for dy in 0..r {
        let dx = r - isqrt((2 * r * dy - dy * dy).min(r * r));
        if r > dx {
            fill_rect(win, x + (w - r) as i32, y + (h - r + dy) as i32, r - dx, 1, color);
        }
    }
}

/// Draw a 1px border rectangle.
pub fn draw_border(win: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
    fill_rect(win, x, y, w, 1, color);                     // top
    fill_rect(win, x, y + h as i32 - 1, w, 1, color);     // bottom
    fill_rect(win, x, y, 1, h, color);                     // left
    fill_rect(win, x + w as i32 - 1, y, 1, h, color);     // right
}

/// Integer square root.
fn isqrt(n: u32) -> u32 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
