// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! libfont.dll — Userspace TTF font engine.
//!
//! Provides font loading, glyph rasterization (greyscale + subpixel LCD),
//! text measurement, and rendering into ARGB pixel buffers.
//!
//! State is stored per-process via the DLL state page at 0x0BFE_0000.
//! Heap memory is obtained via SYS_SBRK (bump allocator in heap.rs).
//!
//! Subpixel rendering is auto-detected on init by querying SYS_GPU_HAS_ACCEL.

#![no_std]
#![no_main]

extern crate alloc;

mod heap;
mod syscall;
mod ttf;
mod ttf_rasterizer;
mod font_manager;

// ── Export struct ─────────────────────────────────────────

const NUM_EXPORTS: u32 = 7;

/// Export function table — must be `#[repr(C)]` and placed in `.exports` section.
///
/// ABI layout (x86_64, `#[repr(C)]`):
///   offset  0: magic [u8; 4]
///   offset  4: version u32
///   offset  8: num_exports u32
///   offset 12: _pad u32
///   offset 16: init fn ptr (8 bytes)
///   offset 24: load_font fn ptr (8 bytes)
///   offset 32: unload_font fn ptr (8 bytes)
///   offset 40: measure_string fn ptr (8 bytes)
///   offset 48: draw_string_buf fn ptr (8 bytes)
///   offset 56: line_height fn ptr (8 bytes)
///   offset 64: set_subpixel fn ptr (8 bytes)
#[repr(C)]
pub struct LibfontExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub _pad: u32,
    pub init: extern "C" fn(),
    pub load_font: extern "C" fn(*const u8, u32) -> u32,
    pub unload_font: extern "C" fn(u32),
    pub measure_string: extern "C" fn(u32, u16, *const u8, u32, *mut u32, *mut u32),
    pub draw_string_buf: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u16, *const u8, u32),
    pub line_height: extern "C" fn(u32, u16) -> u32,
    pub set_subpixel: extern "C" fn(u32),
}

#[link_section = ".exports"]
#[used]
#[no_mangle]
pub static LIBFONT_EXPORTS: LibfontExports = LibfontExports {
    magic: *b"DLIB",
    version: 2,
    num_exports: NUM_EXPORTS,
    _pad: 0,
    init: init_export,
    load_font: load_font_export,
    unload_font: unload_font_export,
    measure_string: measure_string_export,
    draw_string_buf: draw_string_buf_export,
    line_height: line_height_export,
    set_subpixel: set_subpixel_export,
};

// ── Export implementations ───────────────────────────────

/// Initialize the font manager, load system fonts, auto-detect subpixel.
extern "C" fn init_export() {
    font_manager::init();
}

/// Load a custom TTF font from a file path. Returns font_id, or u32::MAX on failure.
extern "C" fn load_font_export(path_ptr: *const u8, path_len: u32) -> u32 {
    let path = unsafe { core::slice::from_raw_parts(path_ptr, path_len as usize) };
    font_manager::load_font(path)
}

/// Unload a previously loaded font by ID.
extern "C" fn unload_font_export(font_id: u32) {
    font_manager::unload_font(font_id as u16);
}

/// Measure text dimensions for a given font and size.
/// Writes pixel width/height into out_w/out_h.
extern "C" fn measure_string_export(
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
extern "C" fn draw_string_buf_export(
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

/// Get line height for a font at a given size.
extern "C" fn line_height_export(font_id: u32, size: u16) -> u32 {
    font_manager::line_height(font_id as u16, size)
}

/// Override subpixel rendering: 1 = enable, 0 = disable.
/// Normally auto-detected on init, but apps can override if needed.
extern "C" fn set_subpixel_export(enabled: u32) {
    font_manager::set_subpixel(enabled != 0);
}

// ── Entry / panic ────────────────────────────────────────

/// Dummy entry point (never called — DLL has no entry).
#[no_mangle]
pub extern "C" fn _dll_start() -> ! {
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
