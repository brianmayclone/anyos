// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Raw FFI bindings to libfont.dll export table.

/// Base virtual address where libfont.dll is loaded.
const LIBFONT_BASE: usize = 0x0420_0000;

/// Export function table — must match the DLL's `LibfontExports` layout exactly.
///
/// ABI layout (x86_64, `#[repr(C)]`):
///   offset  0: magic [u8; 4]
///   offset  4: version u32
///   offset  8: num_exports u32
///   offset 12: _pad u32
///   offset 16: font_load fn ptr (8 bytes)
///   offset 24: font_unload fn ptr (8 bytes)
///   offset 32: font_measure fn ptr (8 bytes)
///   offset 40: win_draw_text_ex fn ptr (8 bytes)
///   offset 48: win_fill_rounded_rect fn ptr (8 bytes)
///   offset 56: gpu_has_accel fn ptr (8 bytes)
#[repr(C)]
pub struct LibfontExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub _pad: u32,
    pub font_load: extern "C" fn(*const u8, u32) -> u32,
    pub font_unload: extern "C" fn(u32) -> u32,
    pub font_measure: extern "C" fn(u32, u16, *const u8, u32, *mut u32, *mut u32) -> u32,
    pub win_draw_text_ex: extern "C" fn(u32, i32, i32, u32, u32, u16, *const u8, u32) -> u32,
    pub win_fill_rounded_rect: extern "C" fn(u32, i32, i32, u32, u32, u32, u32) -> u32,
    pub gpu_has_accel: extern "C" fn() -> u32,
}

/// Get a reference to the DLL export table at the fixed load address.
pub fn exports() -> &'static LibfontExports {
    unsafe { &*(LIBFONT_BASE as *const LibfontExports) }
}
