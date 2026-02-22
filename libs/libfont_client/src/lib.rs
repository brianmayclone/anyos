// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Client library for libfont.so — TTF font engine.
//!
//! Provides safe Rust wrappers around libfont's exported symbols,
//! resolved at runtime via `dl_open` / `dl_sym` (ELF dynamic linking).

#![no_std]

use dynlink::{DlHandle, dl_open, dl_sym};

struct FontLib {
    _handle: DlHandle,
    init_fn: extern "C" fn(),
    load_fn: extern "C" fn(*const u8, u32) -> u32,
    unload_fn: extern "C" fn(u32),
    measure_fn: extern "C" fn(u32, u16, *const u8, u32, *mut u32, *mut u32),
    draw_fn: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u16, *const u8, u32),
    line_height_fn: extern "C" fn(u32, u16) -> u32,
    set_subpixel_fn: extern "C" fn(u32),
}

static mut LIB: Option<FontLib> = None;

fn lib() -> &'static FontLib {
    unsafe { LIB.as_ref().expect("libfont not loaded") }
}

/// Resolve a function pointer from the loaded library, or panic.
unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(handle, name).expect("symbol not found in libfont.so");
    core::mem::transmute_copy::<*const (), T>(&ptr)
}

/// Load and initialize libfont.so. Call once at program start.
/// Returns true on success.
pub fn init() -> bool {
    let handle = match dl_open("/Libraries/libfont.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let lib = FontLib {
            init_fn: resolve(&handle, "font_init"),
            load_fn: resolve(&handle, "font_load"),
            unload_fn: resolve(&handle, "font_unload"),
            measure_fn: resolve(&handle, "font_measure_string"),
            draw_fn: resolve(&handle, "font_draw_string_buf"),
            line_height_fn: resolve(&handle, "font_line_height"),
            set_subpixel_fn: resolve(&handle, "font_set_subpixel"),
            _handle: handle,
        };
        (lib.init_fn)();
        LIB = Some(lib);
    }

    true
}

/// Load a font from a file path.
///
/// Returns `Some(font_id)` on success, or `None` if loading failed.
pub fn load(path: &str) -> Option<u32> {
    let id = (lib().load_fn)(path.as_ptr(), path.len() as u32);
    if id != u32::MAX {
        Some(id)
    } else {
        None
    }
}

/// Unload a previously loaded font.
pub fn unload(font_id: u32) {
    (lib().unload_fn)(font_id);
}

/// Measure the pixel dimensions of text rendered with a given font and size.
///
/// Returns `(width, height)` in pixels.
pub fn measure(font_id: u32, size: u16, text: &str) -> (u32, u32) {
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    (lib().measure_fn)(
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
    (lib().draw_fn)(
        buf, buf_w, buf_h,
        x, y, color,
        font_id, size,
        text.as_ptr(), text.len() as u32,
    );
}

/// Get line height for a font at a given size.
pub fn line_height(font_id: u32, size: u16) -> u32 {
    (lib().line_height_fn)(font_id, size)
}

/// Override subpixel rendering mode.
/// Normally auto-detected on init via SYS_GPU_HAS_ACCEL.
pub fn set_subpixel(enabled: bool) {
    (lib().set_subpixel_fn)(if enabled { 1 } else { 0 });
}
