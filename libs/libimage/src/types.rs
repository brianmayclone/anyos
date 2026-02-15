// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Shared types for the libimage DLL.

/// Image format identifier.
pub const FMT_UNKNOWN: u32 = 0;
pub const FMT_BMP: u32 = 1;
pub const FMT_PNG: u32 = 2;
pub const FMT_JPEG: u32 = 3;
pub const FMT_GIF: u32 = 4;
pub const FMT_ICO: u32 = 5;

/// Error codes returned by image functions.
pub const ERR_OK: i32 = 0;
pub const ERR_INVALID_DATA: i32 = -1;
pub const ERR_UNSUPPORTED: i32 = -2;
pub const ERR_BUFFER_TOO_SMALL: i32 = -3;
pub const ERR_SCRATCH_TOO_SMALL: i32 = -4;

/// Image metadata returned by `image_probe`.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub scratch_needed: u32,
}

impl ImageInfo {
    pub const fn zero() -> Self {
        Self { width: 0, height: 0, format: FMT_UNKNOWN, scratch_needed: 0 }
    }
}

/// Video format: Motion JPEG Video (.mjv).
pub const FMT_MJV: u32 = 10;

/// Video metadata returned by `video_probe`.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub num_frames: u32,
    pub scratch_needed: u32,
}

impl VideoInfo {
    pub const fn zero() -> Self {
        Self { width: 0, height: 0, fps: 0, num_frames: 0, scratch_needed: 0 }
    }
}
