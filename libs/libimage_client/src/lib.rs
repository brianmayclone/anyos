// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Client library for libimage.dlib — image and video decoding shared library.
//!
//! Provides safe Rust wrappers around the raw DLL export functions.
//! User programs depend on this crate to decode BMP, PNG, JPEG, GIF images
//! and MJV (Motion JPEG Video) files.

#![no_std]

pub mod raw;

pub use raw::{ImageInfo, VideoInfo, FMT_UNKNOWN, FMT_BMP, FMT_PNG, FMT_JPEG, FMT_GIF, FMT_ICO, FMT_MJV};

/// Scale mode: stretch to fill, ignoring aspect ratio.
pub const MODE_SCALE: u32 = 0;
/// Scale mode: fit within destination, maintaining aspect ratio (letterboxed).
pub const MODE_CONTAIN: u32 = 1;
/// Scale mode: fill destination, maintaining aspect ratio (cropped).
pub const MODE_COVER: u32 = 2;

/// Error type for image/video operations.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ImageError {
    InvalidData,
    Unsupported,
    BufferTooSmall,
    ScratchTooSmall,
    Unknown(i32),
}

fn err_from_code(code: i32) -> ImageError {
    match code {
        -1 => ImageError::InvalidData,
        -2 => ImageError::Unsupported,
        -3 => ImageError::BufferTooSmall,
        -4 => ImageError::ScratchTooSmall,
        other => ImageError::Unknown(other),
    }
}

// ── Image API ──────────────────────────────────────

/// Probe an image file to determine format and dimensions.
///
/// Returns `Some(ImageInfo)` on success, or `None` if the format is unrecognized.
/// The `scratch_needed` field tells you how large a scratch buffer to allocate
/// for `decode()`.
pub fn probe(data: &[u8]) -> Option<ImageInfo> {
    let mut info = ImageInfo {
        width: 0,
        height: 0,
        format: FMT_UNKNOWN,
        scratch_needed: 0,
    };
    let ret = (raw::exports().image_probe)(data.as_ptr(), data.len() as u32, &mut info);
    if ret == 0 {
        Some(info)
    } else {
        None
    }
}

/// Decode an image into ARGB8888 pixels.
///
/// - `data`: the raw image file bytes
/// - `pixels`: output buffer, must have at least `width * height` elements
/// - `scratch`: working memory buffer, must have at least `scratch_needed` bytes
///
/// Call `probe()` first to determine the required buffer sizes.
pub fn decode(data: &[u8], pixels: &mut [u32], scratch: &mut [u8]) -> Result<(), ImageError> {
    let ret = (raw::exports().image_decode)(
        data.as_ptr(),
        data.len() as u32,
        pixels.as_mut_ptr(),
        pixels.len() as u32,
        scratch.as_mut_ptr(),
        scratch.len() as u32,
    );
    if ret == 0 {
        Ok(())
    } else {
        Err(err_from_code(ret))
    }
}

/// Probe an ICO file, selecting the best entry for a preferred display size.
///
/// For example, `probe_ico_size(data, 48)` picks the closest entry to 48x48.
/// Falls back to the next-larger entry when no exact match exists.
pub fn probe_ico_size(data: &[u8], preferred_size: u32) -> Option<ImageInfo> {
    let mut info = ImageInfo {
        width: 0,
        height: 0,
        format: FMT_UNKNOWN,
        scratch_needed: 0,
    };
    let ret = (raw::exports().ico_probe_size)(
        data.as_ptr(), data.len() as u32, preferred_size, &mut info,
    );
    if ret == 0 { Some(info) } else { None }
}

/// Decode an ICO file, selecting the best entry for a preferred display size.
pub fn decode_ico_size(
    data: &[u8], preferred_size: u32, pixels: &mut [u32], scratch: &mut [u8],
) -> Result<(), ImageError> {
    let ret = (raw::exports().ico_decode_size)(
        data.as_ptr(), data.len() as u32, preferred_size,
        pixels.as_mut_ptr(), pixels.len() as u32,
        scratch.as_mut_ptr(), scratch.len() as u32,
    );
    if ret == 0 { Ok(()) } else { Err(err_from_code(ret)) }
}

// ── Encode API ──────────────────────────────────────

/// Encode ARGB8888 pixels into BMP format.
///
/// - `pixels`: ARGB8888 pixel data (width * height elements)
/// - `width`, `height`: image dimensions
/// - `out`: output buffer for BMP file bytes (must be at least `54 + width*height*4` bytes)
///
/// Returns the number of bytes written on success, or an error.
pub fn encode_bmp(pixels: &[u32], width: u32, height: u32, out: &mut [u8]) -> Result<usize, ImageError> {
    let ret = (raw::exports().image_encode)(
        pixels.as_ptr(),
        width,
        height,
        out.as_mut_ptr(),
        out.len() as u32,
    );
    if ret > 0 {
        Ok(ret as usize)
    } else {
        Err(err_from_code(ret))
    }
}

/// Get the format name as a string.
pub fn format_name(format: u32) -> &'static str {
    match format {
        FMT_BMP => "BMP",
        FMT_PNG => "PNG",
        FMT_JPEG => "JPEG",
        FMT_GIF => "GIF",
        FMT_MJV => "MJV",
        _ => "Unknown",
    }
}

// ── Video API ──────────────────────────────────────

/// Probe a video file to determine format and metadata.
///
/// Returns `Some(VideoInfo)` on success, or `None` if the format is unrecognized.
/// The `scratch_needed` field tells you how large a scratch buffer to allocate
/// for `video_decode_frame()`.
pub fn video_probe(data: &[u8]) -> Option<VideoInfo> {
    let mut info = VideoInfo {
        width: 0,
        height: 0,
        fps: 0,
        num_frames: 0,
        scratch_needed: 0,
    };
    let ret = (raw::exports().video_probe)(data.as_ptr(), data.len() as u32, &mut info);
    if ret == 0 {
        Some(info)
    } else {
        None
    }
}

/// Decode a single video frame into ARGB8888 pixels.
///
/// - `data`: the raw video file bytes (entire .mjv file)
/// - `num_frames`: total frame count (from `video_probe`)
/// - `frame_idx`: zero-based frame index to decode
/// - `pixels`: output buffer, must have at least `width * height` elements
/// - `scratch`: working memory buffer, must have at least `scratch_needed` bytes
pub fn video_decode_frame(
    data: &[u8],
    num_frames: u32,
    frame_idx: u32,
    pixels: &mut [u32],
    scratch: &mut [u8],
) -> Result<(), ImageError> {
    let ret = (raw::exports().video_decode_frame)(
        data.as_ptr(),
        data.len() as u32,
        num_frames,
        frame_idx,
        pixels.as_mut_ptr(),
        pixels.len() as u32,
        scratch.as_mut_ptr(),
        scratch.len() as u32,
    );
    if ret == 0 {
        Ok(())
    } else {
        Err(err_from_code(ret))
    }
}

// ── Scale API ──────────────────────────────────────

/// Scale an ARGB8888 image using bilinear interpolation.
///
/// - `src`: source pixel buffer (`src_w * src_h` elements)
/// - `dst`: destination pixel buffer (`dst_w * dst_h` elements)
/// - `mode`: one of [`MODE_SCALE`], [`MODE_CONTAIN`], or [`MODE_COVER`]
///
/// Returns `true` on success, `false` on error (e.g. wrong buffer size or
/// invalid mode).
pub fn scale_image(
    src: &[u32],
    src_w: u32,
    src_h: u32,
    dst: &mut [u32],
    dst_w: u32,
    dst_h: u32,
    mode: u32,
) -> bool {
    if (src_w as usize) * (src_h as usize) > src.len() {
        return false;
    }
    if (dst_w as usize) * (dst_h as usize) > dst.len() {
        return false;
    }
    let ret = (raw::exports().scale_image)(
        src.as_ptr(),
        src_w,
        src_h,
        dst.as_mut_ptr(),
        dst_w,
        dst_h,
        mode,
    );
    ret == 0
}
