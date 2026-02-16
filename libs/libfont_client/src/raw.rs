// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Raw FFI bindings to libfont.dlib export table.

/// Base virtual address where libfont.dlib is loaded.
const LIBFONT_BASE: usize = 0x0420_0000;

/// Export function table — must match the DLL's `LibfontExports` layout exactly.
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

/// Get a reference to the DLL export table at the fixed load address.
pub fn exports() -> &'static LibfontExports {
    unsafe { &*(LIBFONT_BASE as *const LibfontExports) }
}
