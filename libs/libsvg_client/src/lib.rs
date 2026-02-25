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

use dynlink::{DlHandle, dl_open, dl_sym};

// ── Function pointer table ────────────────────────────────────────────

struct SvgLib {
    _handle: DlHandle,
    /// `svg_probe(data, len, out_w, out_h) -> i32`
    probe_fn:         extern "C" fn(*const u8, u32, *mut f32, *mut f32) -> i32,
    /// `svg_render(data, len, out_pixels, out_w, out_h) -> i32`
    render_fn:        extern "C" fn(*const u8, u32, *mut u32, u32, u32) -> i32,
    /// `svg_render_to_size(data, len, out_pixels, out_w, out_h, bg_color) -> i32`
    render_bg_fn:     extern "C" fn(*const u8, u32, *mut u32, u32, u32, u32) -> i32,
}

static mut LIB: Option<SvgLib> = None;

#[inline]
fn lib() -> &'static SvgLib {
    unsafe { LIB.as_ref().expect("libsvg not loaded — call libsvg_client::init() first") }
}

/// Resolve a function pointer from the loaded handle, or panic.
unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(handle, name).expect("symbol not found in libsvg.so");
    core::mem::transmute_copy::<*const (), T>(&ptr)
}

// ── Public API ────────────────────────────────────────────────────────

/// Load and initialise `libsvg.so`. Must be called once before any other
/// function in this module.
///
/// Returns `true` on success, `false` if the library could not be opened.
pub fn init() -> bool {
    let handle = match dl_open("/Libraries/libsvg.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let lib = SvgLib {
            probe_fn:     resolve(&handle, "svg_probe"),
            render_fn:    resolve(&handle, "svg_render"),
            render_bg_fn: resolve(&handle, "svg_render_to_size"),
            _handle:      handle,
        };
        LIB = Some(lib);
    }

    true
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
    let rc = (lib().probe_fn)(data.as_ptr(), data.len() as u32, &mut w, &mut h);
    if rc == 0 { Some((w, h)) } else { None }
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
    let rc = (lib().render_fn)(
        data.as_ptr(), data.len() as u32,
        pixels.as_mut_ptr(), out_w, out_h,
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
    let rc = (lib().render_bg_fn)(
        data.as_ptr(), data.len() as u32,
        pixels.as_mut_ptr(), out_w, out_h,
        bg_color,
    );
    rc == 0
}
