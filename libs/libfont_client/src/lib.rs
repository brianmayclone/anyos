// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Client library for libfont.dlib — TTF font engine.
//!
//! Provides safe Rust wrappers around the raw DLL export functions.
//! User programs depend on this crate to load TrueType fonts, measure text,
//! and render text into pixel buffers. Font rendering runs entirely in
//! userspace (no syscall overhead for text operations).

#![no_std]

pub mod raw;

/// Initialize the font manager and load system fonts.
///
/// Called automatically on first use, but can be called explicitly
/// for early initialization (e.g. compositor startup).
pub fn init() {
    (raw::exports().init)();
}

/// Load a font from a file path.
///
/// Returns `Some(font_id)` on success, or `None` if loading failed.
pub fn load(path: &str) -> Option<u32> {
    let id = (raw::exports().load_font)(path.as_ptr(), path.len() as u32);
    if id != u32::MAX {
        Some(id)
    } else {
        None
    }
}

/// Unload a previously loaded font.
pub fn unload(font_id: u32) {
    (raw::exports().unload_font)(font_id);
}

/// Measure the pixel dimensions of text rendered with a given font and size.
///
/// Returns `(width, height)` in pixels.
pub fn measure(font_id: u32, size: u16, text: &str) -> (u32, u32) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    (raw::exports().measure_string)(
        font_id,
        size,
        text.as_ptr(),
        text.len() as u32,
        &mut w,
        &mut h,
    );
    (w, h)
}

/// Render text into an ARGB pixel buffer.
///
/// - `buf`: pointer to the pixel buffer (ARGB8888)
/// - `buf_w`, `buf_h`: buffer dimensions
/// - `x`, `y`: top-left rendering position
/// - `color`: ARGB8888 color value
/// - `font_id`: font handle from `load()` (0 = system font)
/// - `size`: font size in pixels
/// - `text`: the string to render
pub fn draw_string_buf(
    buf: *mut u32,
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    color: u32,
    font_id: u32,
    size: u16,
    text: &str,
) {
    (raw::exports().draw_string_buf)(
        buf, buf_w, buf_h,
        x, y, color,
        font_id, size,
        text.as_ptr(), text.len() as u32,
    );
}

/// Get line height for a font at a given size.
pub fn line_height(font_id: u32, size: u16) -> u32 {
    (raw::exports().line_height)(font_id, size)
}

/// Override subpixel rendering mode.
/// Normally auto-detected on init via SYS_GPU_HAS_ACCEL.
pub fn set_subpixel(enabled: bool) {
    (raw::exports().set_subpixel)(if enabled { 1 } else { 0 });
}
