// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Client library for libfont.so — TTF font engine.
//!
//! Provides safe Rust wrappers around libfont's exported symbols,
//! resolved at runtime via `dl_open` / `dl_sym` (ELF dynamic linking).

#![no_std]

dynlink::dll_exports! {
    lib_path: "/Libraries/libfont.so",
    lib_struct: FontLib,
    init_call: "font_init",
    symbols: {
        font_load(data: *const u8, len: u32) -> u32,
        font_unload(id: u32) -> (),
        font_measure_string(font_id: u32, size: u16, text: *const u8, len: u32, out_w: *mut u32, out_h: *mut u32) -> (),
        font_draw_string_buf(buf: *mut u32, buf_w: u32, buf_h: u32, x: i32, y: i32, color: u32, font_id: u32, size: u16, text: *const u8, len: u32) -> (),
        font_line_height(font_id: u32, size: u16) -> u32,
        font_set_subpixel(enabled: u32) -> (),
    }
}

/// Load a font from a file path.
///
/// Returns `Some(font_id)` on success, or `None` if loading failed.
pub fn load(path: &str) -> Option<u32> {
    let id = (lib().font_load)(path.as_ptr(), path.len() as u32);
    if id != u32::MAX {
        Some(id)
    } else {
        None
    }
}

/// Unload a previously loaded font.
pub fn unload(font_id: u32) {
    (lib().font_unload)(font_id);
}

/// Measure the pixel dimensions of text rendered with a given font and size.
///
/// Returns `(width, height)` in pixels.
pub fn measure(font_id: u32, size: u16, text: &str) -> (u32, u32) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    (lib().font_measure_string)(
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
    (lib().font_draw_string_buf)(
        buf, buf_w, buf_h,
        x, y, color,
        font_id, size,
        text.as_ptr(), text.len() as u32,
    );
}

/// Get line height for a font at a given size.
pub fn line_height(font_id: u32, size: u16) -> u32 {
    (lib().font_line_height)(font_id, size)
}

/// Override subpixel rendering mode.
/// Normally auto-detected on init via SYS_GPU_HAS_ACCEL.
pub fn set_subpixel(enabled: bool) {
    (lib().font_set_subpixel)(if enabled { 1 } else { 0 });
}
