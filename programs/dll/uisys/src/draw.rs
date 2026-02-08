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

/// Draw a filled rounded rectangle using pixel-center tests for correct corners.
pub fn fill_rounded_rect(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    if r == 0 || w < r * 2 || h < r * 2 {
        fill_rect(win, x, y, w, h, color);
        return;
    }
    // Main body (full width, reduced height — no overlap with strips)
    if h > r * 2 {
        fill_rect(win, x, y + r as i32, w, h - r * 2, color);
    }
    // Top strip (narrower, between corners)
    if w > r * 2 {
        fill_rect(win, x + r as i32, y, w - r * 2, r, color);
        // Bottom strip
        fill_rect(win, x + r as i32, y + (h - r) as i32, w - r * 2, r, color);
    }
    // Corner fills using pixel-center test:
    // Pixel (dx, dy) in corner [0..r) is inside if its center is within the arc.
    // Test: (2*dx + 1 - 2*r)² + (2*dy + 1 - 2*r)² ≤ (2*r)²
    let r2x4 = (2 * r as i32) * (2 * r as i32);
    for dy in 0..r {
        let cy = 2 * dy as i32 + 1 - 2 * r as i32;
        let cy2 = cy * cy;
        // Find leftmost pixel inside the arc (scan left to right)
        let mut fill_start = r;
        for dx in 0..r {
            let cx = 2 * dx as i32 + 1 - 2 * r as i32;
            if cx * cx + cy2 <= r2x4 {
                fill_start = dx;
                break;
            }
        }
        let fill_width = r - fill_start;
        if fill_width > 0 {
            let fs = fill_start as i32;
            // Top-left: arc curves from top-edge to left-edge
            fill_rect(win, x + fs, y + dy as i32, fill_width, 1, color);
            // Top-right: mirror horizontally — fill from left side of quadrant
            fill_rect(win, x + (w - r) as i32, y + dy as i32, fill_width, 1, color);
            // Bottom-left: mirror vertically — reverse row order
            fill_rect(win, x + fs, y + (h as i32 - 1 - dy as i32), fill_width, 1, color);
            // Bottom-right: mirror both axes
            fill_rect(win, x + (w - r) as i32, y + (h as i32 - 1 - dy as i32), fill_width, 1, color);
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

/// Draw a 1px rounded border following the same arc as fill_rounded_rect.
pub fn draw_rounded_border(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    if r == 0 || w < r * 2 || h < r * 2 {
        draw_border(win, x, y, w, h, color);
        return;
    }
    // Top edge (between corners)
    if w > r * 2 {
        fill_rect(win, x + r as i32, y, w - r * 2, 1, color);
        // Bottom edge
        fill_rect(win, x + r as i32, y + h as i32 - 1, w - r * 2, 1, color);
    }
    // Left edge (between corners)
    if h > r * 2 {
        fill_rect(win, x, y + r as i32, 1, h - r * 2, color);
        // Right edge
        fill_rect(win, x + w as i32 - 1, y + r as i32, 1, h - r * 2, color);
    }
    // Corner arcs — draw only the outermost pixel on each scanline
    let r2x4 = (2 * r as i32) * (2 * r as i32);
    for dy in 0..r {
        let cy = 2 * dy as i32 + 1 - 2 * r as i32;
        let cy2 = cy * cy;
        let mut fill_start = r;
        for dx in 0..r {
            let cx = 2 * dx as i32 + 1 - 2 * r as i32;
            if cx * cx + cy2 <= r2x4 {
                fill_start = dx;
                break;
            }
        }
        if fill_start < r {
            let fs = fill_start as i32;
            // Top-left arc pixel
            fill_rect(win, x + fs, y + dy as i32, 1, 1, color);
            // Top-right arc pixel
            fill_rect(win, x + (w - 1 - fill_start) as i32, y + dy as i32, 1, 1, color);
            // Bottom-left arc pixel
            fill_rect(win, x + fs, y + (h - 1 - dy) as i32, 1, 1, color);
            // Bottom-right arc pixel
            fill_rect(win, x + (w - 1 - fill_start) as i32, y + (h - 1 - dy) as i32, 1, 1, color);
        }
    }
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
