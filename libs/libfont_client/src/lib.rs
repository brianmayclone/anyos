// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Client library for libfont — TTF font engine.
//!
//! On anyOS: loads libfont.so at runtime via dl_open/dl_sym.
//! On host:  links directly against the libfont crate.

#![cfg_attr(not(feature = "host"), no_std)]

// ═══════════════════════════════════════════════════════════
// anyOS mode: load via DLL
// ═══════════════════════════════════════════════════════════

#[cfg(not(feature = "host"))]
dynlink::dll_exports! {
    lib_path: "/Libraries/libfont.so",
    lib_struct: FontLib,
    init_call: "font_init",
    symbols: {
        font_load(data: *const u8, len: u32) -> u32,
        font_load_data(data: *const u8, len: u32) -> u32,
        font_unload(id: u32) -> (),
        font_measure_string(font_id: u32, size: u16, text: *const u8, len: u32, out_w: *mut u32, out_h: *mut u32) -> (),
        font_draw_string_buf(buf: *mut u32, buf_w: u32, buf_h: u32, x: i32, y: i32, color: u32, font_id: u32, size: u16, text: *const u8, len: u32) -> (),
        font_line_height(font_id: u32, size: u16) -> u32,
        font_set_subpixel(enabled: u32) -> (),
    }
}

#[cfg(not(feature = "host"))]
/// Load a font from a file path.
pub fn load(path: &str) -> Option<u32> {
    let id = (lib().font_load)(path.as_ptr(), path.len() as u32);
    if id != u32::MAX {
        Some(id)
    } else {
        None
    }
}

#[cfg(not(feature = "host"))]
/// Load a font from raw TTF data in memory.
pub fn load_data(data: &[u8]) -> Option<u32> {
    let id = (lib().font_load_data)(data.as_ptr(), data.len() as u32);
    if id != u32::MAX {
        Some(id)
    } else {
        None
    }
}

#[cfg(not(feature = "host"))]
/// Unload a previously loaded font.
pub fn unload(font_id: u32) {
    (lib().font_unload)(font_id);
}

#[cfg(not(feature = "host"))]
/// Measure the pixel dimensions of text.
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

#[cfg(not(feature = "host"))]
/// Render text into an ARGB pixel buffer.
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
        buf,
        buf_w,
        buf_h,
        x,
        y,
        color,
        font_id,
        size,
        text.as_ptr(),
        text.len() as u32,
    );
}

#[cfg(not(feature = "host"))]
/// Get line height for a font at a given size.
pub fn line_height(font_id: u32, size: u16) -> u32 {
    (lib().font_line_height)(font_id, size)
}

#[cfg(not(feature = "host"))]
/// Override subpixel rendering mode.
pub fn set_subpixel(enabled: bool) {
    (lib().font_set_subpixel)(if enabled { 1 } else { 0 });
}

// ═══════════════════════════════════════════════════════════
// Host mode: call libfont via extern "C" (linked by the final binary)
// ═══════════════════════════════════════════════════════════

#[cfg(feature = "host")]
extern "C" {
    fn font_init();
    fn font_load(path_ptr: *const u8, path_len: u32) -> u32;
    fn font_load_data(data_ptr: *const u8, data_len: u32) -> u32;
    fn font_unload(font_id: u32);
    fn font_measure_string(
        font_id: u32,
        size: u16,
        text: *const u8,
        len: u32,
        out_w: *mut u32,
        out_h: *mut u32,
    );
    fn font_draw_string_buf(
        buf: *mut u32,
        buf_w: u32,
        buf_h: u32,
        x: i32,
        y: i32,
        color: u32,
        font_id: u32,
        size: u16,
        text: *const u8,
        len: u32,
    );
    fn font_line_height(font_id: u32, size: u16) -> u32;
    fn font_set_subpixel(enabled: u32);
}

#[cfg(feature = "host")]
pub fn init() -> bool {
    unsafe {
        font_init();
    }
    true
}

#[cfg(feature = "host")]
pub fn load(path: &str) -> Option<u32> {
    let id = unsafe { font_load(path.as_ptr(), path.len() as u32) };
    if id != u32::MAX {
        Some(id)
    } else {
        None
    }
}

#[cfg(feature = "host")]
pub fn load_data(data: &[u8]) -> Option<u32> {
    let id = unsafe { font_load_data(data.as_ptr(), data.len() as u32) };
    if id != u32::MAX {
        Some(id)
    } else {
        None
    }
}

#[cfg(feature = "host")]
pub fn unload(font_id: u32) {
    unsafe {
        font_unload(font_id);
    }
}

#[cfg(feature = "host")]
pub fn measure(font_id: u32, size: u16, text: &str) -> (u32, u32) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    unsafe {
        font_measure_string(
            font_id,
            size,
            text.as_ptr(),
            text.len() as u32,
            &mut w,
            &mut h,
        );
    }
    (w, h)
}

#[cfg(feature = "host")]
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
    unsafe {
        font_draw_string_buf(
            buf,
            buf_w,
            buf_h,
            x,
            y,
            color,
            font_id,
            size,
            text.as_ptr(),
            text.len() as u32,
        );
    }
}

#[cfg(feature = "host")]
pub fn line_height(font_id: u32, size: u16) -> u32 {
    unsafe { font_line_height(font_id, size) }
}

#[cfg(feature = "host")]
pub fn set_subpixel(enabled: bool) {
    unsafe {
        font_set_subpixel(if enabled { 1 } else { 0 });
    }
}
