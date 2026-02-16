//! Bitmap font (8x16 fixed-width) for boot console and kernel-internal use.
//! TTF rendering has been moved to userspace libfont.dll.

use crate::graphics::color::Color;
use crate::graphics::surface::Surface;

/// Simple 8x16 bitmap font (built-in, no external files needed)
/// Each character is 8 pixels wide and 16 pixels tall.
pub const FONT_WIDTH: u32 = 8;
pub const FONT_HEIGHT: u32 = 16;

/// Embedded 8x16 font data for ASCII 32-126
/// Each character is 16 bytes (one byte per row, MSB=left)
static FONT_DATA: &[u8] = include_bytes!("font_8x16.bin");

/// Check if font data is available
pub fn is_available() -> bool {
    FONT_DATA.len() >= 95 * 16 // At least ASCII 32-126
}

// ─── Bitmap font (low-level, for terminal and boot console) ─────────

/// Draw a single character using the bitmap font (8x16 fixed-width)
pub fn draw_char_bitmap(surface: &mut Surface, x: i32, y: i32, ch: char, color: Color) {
    let c = ch as u32;
    if c < 32 || c > 126 {
        return;
    }

    let idx = (c - 32) as usize;
    let glyph_offset = idx * FONT_HEIGHT as usize;

    if glyph_offset + FONT_HEIGHT as usize > FONT_DATA.len() {
        return;
    }

    for row in 0..FONT_HEIGHT as i32 {
        let byte = FONT_DATA[glyph_offset + row as usize];
        for col in 0..FONT_WIDTH as i32 {
            if byte & (0x80 >> col) != 0 {
                surface.put_pixel(x + col, y + row, color);
            }
        }
    }
}

/// Draw a string using the bitmap font (8x16 fixed-width)
pub fn draw_string_bitmap(surface: &mut Surface, x: i32, y: i32, text: &str, color: Color) {
    let mut cx = x;
    let mut cy = y;

    for ch in text.chars() {
        if ch == '\n' {
            cx = x;
            cy += FONT_HEIGHT as i32;
            continue;
        }
        if ch == '\t' {
            cx += (FONT_WIDTH * 4) as i32;
            continue;
        }
        draw_char_bitmap(surface, cx, cy, ch, color);
        cx += FONT_WIDTH as i32;
    }
}

/// Measure a string using the bitmap font
pub fn measure_string_bitmap(text: &str) -> (u32, u32) {
    let mut max_width = 0u32;
    let mut current_width = 0u32;
    let mut lines = 1u32;

    for ch in text.chars() {
        if ch == '\n' {
            max_width = max_width.max(current_width);
            current_width = 0;
            lines += 1;
        } else if ch == '\t' {
            current_width += FONT_WIDTH * 4;
        } else {
            current_width += FONT_WIDTH;
        }
    }
    max_width = max_width.max(current_width);

    (max_width, lines * FONT_HEIGHT)
}
