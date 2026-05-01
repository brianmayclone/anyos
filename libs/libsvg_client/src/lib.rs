// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Client library for libsvg.so — SVG 1.1 static rasterizer.
//!
//! Provides safe Rust wrappers around libsvg's exported symbols,
//! resolved at runtime via `dl_open` / `dl_sym` (ELF dynamic linking).
//!
//! # Usage
//!
//! ```rust,ignore
//! libsvg_client::init();  // load libsvg.so once at startup
//!
//! let svg = include_bytes!("logo.svg");
//!
//! // Query dimensions declared in the SVG document
//! if let Some((w, h)) = libsvg_client::probe(svg) {
//!     // Render to a 200x200 ARGB buffer with transparent background
//!     let mut pixels = vec![0u32; 200 * 200];
//!     libsvg_client::render_to_size(svg, &mut pixels, 200, 200, 0x00000000);
//! }
//! ```

#![no_std]

extern crate alloc;

dynlink::dll_exports! {
    lib_path: "/Libraries/libsvg.so",
    lib_struct: SvgLib,
    symbols: {
        svg_probe(data: *const u8, len: u32, out_w: *mut f32, out_h: *mut f32) -> i32,
        svg_render(data: *const u8, len: u32, out_pixels: *mut u32, out_w: u32, out_h: u32) -> i32,
        svg_render_to_size(data: *const u8, len: u32, out_pixels: *mut u32, out_w: u32, out_h: u32, bg_color: u32) -> i32,
    }
}

/// Probe an SVG document and return its declared canvas dimensions.
///
/// - `data`: raw SVG bytes (UTF-8)
///
/// Returns `Some((width, height))` in pixels, or `None` if the document
/// could not be parsed or has no usable dimensions.
pub fn probe(data: &[u8]) -> Option<(f32, f32)> {
    let mut w: f32 = 0.0;
    let mut h: f32 = 0.0;
    let rc = (lib().svg_probe)(data.as_ptr(), data.len() as u32, &mut w, &mut h);
    if rc == 0 {
        Some((w, h))
    } else {
        None
    }
}

/// Render an SVG document into an ARGB8888 pixel buffer.
///
/// The SVG is scaled uniformly to fit `(out_w, out_h)`, letterboxed with a
/// transparent background when aspect ratios differ.
///
/// - `data`: raw SVG bytes
/// - `pixels`: output buffer — must contain exactly `out_w * out_h` `u32` slots
/// - `out_w`, `out_h`: desired output dimensions (1–8192 each)
///
/// Returns `true` on success.
pub fn render(data: &[u8], pixels: &mut [u32], out_w: u32, out_h: u32) -> bool {
    if pixels.len() < (out_w as usize) * (out_h as usize) {
        return false;
    }
    let rc = (lib().svg_render)(
        data.as_ptr(),
        data.len() as u32,
        pixels.as_mut_ptr(),
        out_w,
        out_h,
    );
    rc == 0
}

/// Render an SVG document into an ARGB8888 pixel buffer with a custom
/// background colour.
///
/// - `data`: raw SVG bytes
/// - `pixels`: output buffer — must contain exactly `out_w * out_h` `u32` slots
/// - `out_w`, `out_h`: desired output dimensions (1–8192 each)
/// - `bg_color`: ARGB8888 background colour (`0x00000000` = transparent)
///
/// Returns `true` on success.
pub fn render_to_size(
    data: &[u8],
    pixels: &mut [u32],
    out_w: u32,
    out_h: u32,
    bg_color: u32,
) -> bool {
    if pixels.len() < (out_w as usize) * (out_h as usize) {
        return false;
    }
    let rc = (lib().svg_render_to_size)(
        data.as_ptr(),
        data.len() as u32,
        pixels.as_mut_ptr(),
        out_w,
        out_h,
        bg_color,
    );
    rc == 0
}
