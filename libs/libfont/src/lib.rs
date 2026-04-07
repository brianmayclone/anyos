// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! libfont — TTF font engine.
//!
//! Provides font loading, glyph rasterization (greyscale + subpixel LCD),
//! text measurement, and rendering into ARGB pixel buffers.
//!
//! System fonts are loaded from /System/fonts/ at runtime.

#![cfg_attr(not(feature = "host"), no_std)]
#![cfg_attr(not(feature = "host"), no_main)]

extern crate alloc;

#[cfg(not(feature = "host"))]
libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);

#[cfg(not(feature = "host"))]
pub(crate) mod syscall;
#[cfg(feature = "host")]
#[path = "host_syscall.rs"]
pub(crate) mod syscall;

pub(crate) mod ttf;
mod ttf_rasterizer;
pub(crate) mod inflate;
pub(crate) mod png_decode;
pub(crate) mod font_manager;
pub(crate) mod brotli;
pub(crate) mod woff2;
#[cfg(not(feature = "host"))]
pub(crate) mod fontd_client;

// ── System font paths (loaded at runtime via fontd or disk fallback) ────

const FONT_PATHS: [&[u8]; 6] = [
    b"/System/fonts/sfpro.ttf",
    b"/System/fonts/sfpro-bold.ttf",
    b"/System/fonts/sfpro-thin.ttf",
    b"/System/fonts/sfpro-italic.ttf",
    b"/System/fonts/andale-mono.ttf",
    b"/System/fonts/NotoColorEmoji.ttf",
];

/// Get the full disk path for a system font by ID.
pub(crate) fn font_paths_by_id(id: usize) -> &'static [u8] {
    if id < FONT_PATHS.len() { FONT_PATHS[id] } else { b"" }
}

// ── Public API (used directly on host, or via dl_sym on anyOS) ────────

/// Initialize the font manager, load system fonts, auto-detect subpixel.
#[no_mangle]
pub extern "C" fn font_init() {
    font_manager::init();
}

/// Load a custom TTF font from a file path. Returns font_id, or u32::MAX on failure.
#[no_mangle]
pub extern "C" fn font_load(path_ptr: *const u8, path_len: u32) -> u32 {
    let path = unsafe { core::slice::from_raw_parts(path_ptr, path_len as usize) };
    font_manager::load_font(path)
}

/// Load a custom TTF font from raw data in memory. Returns font_id, or u32::MAX on failure.
#[no_mangle]
pub extern "C" fn font_load_data(data_ptr: *const u8, data_len: u32) -> u32 {
    let data = unsafe { core::slice::from_raw_parts(data_ptr, data_len as usize) };
    let owned = alloc::vec::Vec::from(data);
    font_manager::load_font_data(owned)
}

/// Unload a previously loaded font by ID.
#[no_mangle]
pub extern "C" fn font_unload(font_id: u32) {
    font_manager::unload_font(font_id as u16);
}

/// Measure text dimensions for a given font and size.
#[no_mangle]
pub extern "C" fn font_measure_string(
    font_id: u32,
    size: u16,
    text_ptr: *const u8,
    text_len: u32,
    out_w: *mut u32,
    out_h: *mut u32,
) {
    let text = unsafe {
        let bytes = core::slice::from_raw_parts(text_ptr, text_len as usize);
        core::str::from_utf8_unchecked(bytes)
    };
    let (w, h) = font_manager::measure_string(text, font_id as u16, size);
    unsafe {
        *out_w = w;
        *out_h = h;
    }
}

/// Render text into a caller-provided ARGB pixel buffer.
#[no_mangle]
pub extern "C" fn font_draw_string_buf(
    buf: *mut u32,
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    color: u32,
    font_id: u32,
    size: u16,
    text_ptr: *const u8,
    text_len: u32,
) {
    let text = unsafe {
        let bytes = core::slice::from_raw_parts(text_ptr, text_len as usize);
        core::str::from_utf8_unchecked(bytes)
    };
    font_manager::draw_string_buf(buf, buf_w, buf_h, x, y, color, font_id as u16, size, text);
}

/// Render text into a caller-provided ARGB pixel buffer with clip rect.
#[no_mangle]
pub extern "C" fn font_draw_string_buf_clipped(
    buf: *mut u32,
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    color: u32,
    font_id: u32,
    size: u16,
    text_ptr: *const u8,
    text_len: u32,
    clip_x: i32,
    clip_y: i32,
    clip_r: i32,
    clip_b: i32,
) {
    let text = unsafe {
        let bytes = core::slice::from_raw_parts(text_ptr, text_len as usize);
        core::str::from_utf8_unchecked(bytes)
    };
    font_manager::draw_string_buf_clipped(buf, buf_w, buf_h, x, y, color, font_id as u16, size, text,
        clip_x, clip_y, clip_r, clip_b);
}

/// Get line height for a font at a given size.
#[no_mangle]
pub extern "C" fn font_line_height(font_id: u32, size: u16) -> u32 {
    font_manager::line_height(font_id as u16, size)
}

/// Override subpixel rendering: 1 = enable, 0 = disable.
#[no_mangle]
pub extern "C" fn font_set_subpixel(enabled: u32) {
    font_manager::set_subpixel(enabled != 0);
}

// ── Panic handler (anyOS only) ────────────────────────────────────────

#[cfg(not(feature = "host"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}
