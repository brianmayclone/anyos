// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! libfont.so — Userspace TTF font engine (shared library).
//!
//! Provides font loading, glyph rasterization (greyscale + subpixel LCD),
//! text measurement, and rendering into ARGB pixel buffers.
//!
//! System fonts are embedded in .rodata via include_bytes!() — shared
//! read-only pages across all processes. No disk I/O at init.
//!
//! State is stored per-process via .bss statics.
//! Heap memory is obtained via SYS_SBRK (bump allocator in heap.rs).
//!
//! Subpixel rendering is auto-detected on init by querying SYS_GPU_HAS_ACCEL.

#![no_std]
#![no_main]

extern crate alloc;

mod heap;
pub(crate) mod syscall;
pub(crate) mod ttf;
mod ttf_rasterizer;
pub(crate) mod inflate;
pub(crate) mod png_decode;
pub(crate) mod font_manager;

// ── Embedded system fonts (.rodata — shared across all processes) ────

pub(crate) static FONT_SFPRO: &[u8] = include_bytes!("../../../sysroot/System/fonts/sfpro.ttf");
pub(crate) static FONT_SFPRO_BOLD: &[u8] = include_bytes!("../../../sysroot/System/fonts/sfpro-bold.ttf");
pub(crate) static FONT_SFPRO_THIN: &[u8] = include_bytes!("../../../sysroot/System/fonts/sfpro-thin.ttf");
pub(crate) static FONT_SFPRO_ITALIC: &[u8] = include_bytes!("../../../sysroot/System/fonts/sfpro-italic.ttf");
pub(crate) static FONT_ANDALE_MONO: &[u8] = include_bytes!("../../../sysroot/System/fonts/andale-mono.ttf");
pub(crate) static FONT_EMOJI: &[u8] = include_bytes!("../../../sysroot/System/fonts/NotoColorEmoji.ttf");

// ── Exported C API (resolved via dl_sym) ────────────────────────────

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

/// Unload a previously loaded font by ID.
#[no_mangle]
pub extern "C" fn font_unload(font_id: u32) {
    font_manager::unload_font(font_id as u16);
}

/// Measure text dimensions for a given font and size.
/// Writes pixel width/height into out_w/out_h.
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
/// Normally auto-detected on init, but apps can override if needed.
#[no_mangle]
pub extern "C" fn font_set_subpixel(enabled: u32) {
    font_manager::set_subpixel(enabled != 0);
}

// ── Panic handler ────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}
