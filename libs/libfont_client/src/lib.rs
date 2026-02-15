// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Client library for libfont.dll — font loading, text measurement, and extended drawing.
//!
//! Provides safe Rust wrappers around the raw DLL export functions.
//! User programs depend on this crate to load TrueType fonts, measure text,
//! draw text with custom fonts, fill rounded rectangles, and query GPU acceleration.

#![no_std]

pub mod raw;

/// Load a font from a file path.
///
/// Returns `Some(font_id)` on success, or `None` if loading failed.
pub fn load(path: &str) -> Option<u32> {
    let id = (raw::exports().font_load)(path.as_ptr(), path.len() as u32);
    if id != 0 {
        Some(id)
    } else {
        None
    }
}

/// Unload a previously loaded font.
pub fn unload(font_id: u32) {
    (raw::exports().font_unload)(font_id);
}

/// Measure the pixel dimensions of text rendered with a given font and size.
///
/// Returns `(width, height)` in pixels.
pub fn measure(font_id: u32, size: u16, text: &str) -> (u32, u32) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    (raw::exports().font_measure)(
        font_id,
        size,
        text.as_ptr(),
        text.len() as u32,
        &mut w,
        &mut h,
    );
    (w, h)
}

/// Draw text in a window using a loaded font.
///
/// - `win`: window ID
/// - `x`, `y`: top-left position
/// - `color`: ARGB8888 color value
/// - `font_id`: font handle from `load()`
/// - `size`: font size in pixels
/// - `text`: the string to draw
pub fn draw_text(win: u32, x: i32, y: i32, color: u32, font_id: u32, size: u16, text: &str) {
    (raw::exports().win_draw_text_ex)(
        win,
        x,
        y,
        color,
        font_id,
        size,
        text.as_ptr(),
        text.len() as u32,
    );
}

/// Fill a rounded rectangle in a window.
///
/// - `win`: window ID
/// - `x`, `y`: top-left position
/// - `w`, `h`: rectangle dimensions
/// - `r`: corner radius
/// - `color`: ARGB8888 fill color
pub fn fill_rounded_rect(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    (raw::exports().win_fill_rounded_rect)(win, x, y, w, h, r, color);
}

/// Query whether GPU acceleration is available.
///
/// Returns `true` if the GPU supports hardware-accelerated operations.
pub fn gpu_has_accel() -> bool {
    (raw::exports().gpu_has_accel)() != 0
}
