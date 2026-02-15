// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Raw FFI bindings to libimage.dll export table.

/// Base virtual address where libimage.dll is loaded.
const LIBIMAGE_BASE: usize = 0x0410_0000;

/// Image metadata returned by `image_probe`.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub scratch_needed: u32,
}

/// Video metadata returned by `video_probe`.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub num_frames: u32,
    pub scratch_needed: u32,
}

/// Format constants.
pub const FMT_UNKNOWN: u32 = 0;
pub const FMT_BMP: u32 = 1;
pub const FMT_PNG: u32 = 2;
pub const FMT_JPEG: u32 = 3;
pub const FMT_GIF: u32 = 4;
pub const FMT_ICO: u32 = 5;
pub const FMT_MJV: u32 = 10;

/// Export function table — must match the DLL's `LibimageExports` layout exactly.
///
/// ABI layout (x86_64, `#[repr(C)]`):
///   offset  0: magic [u8; 4]
///   offset  4: version u32
///   offset  8: num_exports u32
///   offset 12: _pad u32
///   offset 16: video_probe fn ptr (8 bytes)
///   offset 24: video_decode_frame fn ptr (8 bytes)
///   offset 32: image_probe fn ptr (8 bytes)
///   offset 40: image_decode fn ptr (8 bytes)
#[repr(C)]
pub struct LibimageExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub _pad: u32,
    pub video_probe: extern "C" fn(*const u8, u32, *mut VideoInfo) -> i32,
    pub video_decode_frame: extern "C" fn(*const u8, u32, u32, u32, *mut u32, u32, *mut u8, u32) -> i32,
    pub image_probe: extern "C" fn(*const u8, u32, *mut ImageInfo) -> i32,
    pub image_decode: extern "C" fn(*const u8, u32, *mut u32, u32, *mut u8, u32) -> i32,
    pub scale_image: extern "C" fn(*const u32, u32, u32, *mut u32, u32, u32, u32) -> i32,
    pub ico_probe_size: extern "C" fn(*const u8, u32, u32, *mut ImageInfo) -> i32,
    pub ico_decode_size: extern "C" fn(*const u8, u32, u32, *mut u32, u32, *mut u8, u32) -> i32,
}

/// Get a reference to the DLL export table at the fixed load address.
pub fn exports() -> &'static LibimageExports {
    unsafe { &*(LIBIMAGE_BASE as *const LibimageExports) }
}
