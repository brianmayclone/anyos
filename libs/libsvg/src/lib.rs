// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! libsvg.so — SVG 1.1 static rasterizer (shared library).
//!
//! Parses SVG documents from memory buffers and renders them to ARGB8888
//! pixel buffers at arbitrary resolution.
//!
//! Supported SVG subset:
//! - Elements: svg, g, path, rect, circle, ellipse, line, polyline, polygon
//! - Path commands: M/L/H/V/C/S/Q/T/A/Z (absolute and relative)
//! - Styles: fill, stroke, stroke-width, opacity, fill-opacity,
//!   stroke-opacity, fill-rule, stroke-linecap, stroke-linejoin,
//!   display, visibility
//! - Transforms: matrix, translate, rotate, scale, skewX, skewY
//! - Gradients: linearGradient, radialGradient with multiple stops,
//!   spreadMethod (pad/reflect/repeat), gradientUnits
//! - CSS: inline style="" attributes, most CSS colour formats
//!
//! Loaded via dl_open("/Libraries/libsvg.so"), symbols resolved via dl_sym.
//! State is per-process via .bss statics; heap via SYS_SBRK.

#![no_std]
#![no_main]

extern crate alloc;

libheap::dll_allocator!(crate::syscall::sbrk);
pub(crate) mod syscall;
pub mod types;
pub mod xml;
pub mod parser;
pub mod path;
pub mod gradient;
pub mod render;

// ── Error codes ──────────────────────────────────────────────────────

const ERR_OK:           i32 = 0;
const ERR_INVALID_DATA: i32 = -1;
const ERR_UNSUPPORTED:  i32 = -2;

// ── Exported C API ───────────────────────────────────────────────────

/// Probe an SVG document and return its declared dimensions.
///
/// - `data`/`len`: raw SVG bytes (UTF-8)
/// - `out_w`, `out_h`: receive the SVG canvas width and height in pixels
///
/// Returns 0 on success, negative on error.
#[no_mangle]
pub extern "C" fn svg_probe(
    data: *const u8,
    len:  u32,
    out_w: *mut f32,
    out_h: *mut f32,
) -> i32 {
    if data.is_null() || out_w.is_null() || out_h.is_null() || len < 5 {
        return ERR_INVALID_DATA;
    }
    let bytes = unsafe { core::slice::from_raw_parts(data, len as usize) };

    match parser::parse(bytes) {
        Some(doc) => {
            unsafe { *out_w = doc.width; }
            unsafe { *out_h = doc.height; }
            ERR_OK
        }
        None => ERR_UNSUPPORTED,
    }
}

/// Render an SVG document to an ARGB8888 pixel buffer.
///
/// The SVG is scaled uniformly to fit the requested output dimensions,
/// letterboxed with a transparent background when the aspect ratio differs.
///
/// - `data`/`len`: raw SVG bytes
/// - `out_pixels`: ARGB8888 output buffer (must hold `out_w * out_h` u32s)
/// - `out_w`, `out_h`: desired output dimensions in pixels (max 8192 each)
///
/// Returns 0 on success, negative on error.
#[no_mangle]
pub extern "C" fn svg_render(
    data:       *const u8,
    len:        u32,
    out_pixels: *mut u32,
    out_w:      u32,
    out_h:      u32,
) -> i32 {
    svg_render_to_size(data, len, out_pixels, out_w, out_h, 0x00000000)
}

/// Render an SVG document to an ARGB8888 pixel buffer with a custom background.
///
/// - `data`/`len`: raw SVG bytes
/// - `out_pixels`: ARGB8888 output buffer
/// - `out_w`, `out_h`: desired output dimensions
/// - `bg_color`: ARGB8888 background colour (0x00000000 = transparent)
///
/// Returns 0 on success, negative on error.
#[no_mangle]
pub extern "C" fn svg_render_to_size(
    data:       *const u8,
    len:        u32,
    out_pixels: *mut u32,
    out_w:      u32,
    out_h:      u32,
    bg_color:   u32,
) -> i32 {
    if data.is_null() || out_pixels.is_null() || len < 5 {
        return ERR_INVALID_DATA;
    }
    if out_w == 0 || out_h == 0 || out_w > 8192 || out_h > 8192 {
        return ERR_INVALID_DATA;
    }

    let bytes = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let out_count = (out_w as usize) * (out_h as usize);
    let out = unsafe { core::slice::from_raw_parts_mut(out_pixels, out_count) };

    match parser::parse(bytes) {
        Some(doc) => {
            render::render(&doc, out, out_w, out_h, bg_color);
            ERR_OK
        }
        None => ERR_UNSUPPORTED,
    }
}

// ── Dummy entry point ────────────────────────────────────────────────

/// Dummy entry point — shared libraries have no real entry.
#[no_mangle]
pub extern "C" fn _dll_start() -> ! {
    loop {}
}

// ── Panic handler ────────────────────────────────────────────────────

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    syscall::write(2, b"PANIC [libsvg]: ");
    if let Some(loc) = info.location() {
        syscall::write(2, loc.file().as_bytes());
        syscall::write(2, b":");
        let mut buf = [0u8; 10];
        let s = fmt_u32(loc.line(), &mut buf);
        syscall::write(2, s);
    }
    syscall::write(2, b"\n");
    syscall::exit(1);
}

fn fmt_u32(mut val: u32, buf: &mut [u8; 10]) -> &[u8] {
    if val == 0 {
        buf[9] = b'0';
        return &buf[9..10];
    }
    let mut i = 10usize;
    while val > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    &buf[i..10]
}
