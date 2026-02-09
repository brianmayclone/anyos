// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Export table for libimage.dll.

use crate::types::{ImageInfo, VideoInfo};

const NUM_EXPORTS: u32 = 5;

/// Export function table — must be first in the binary (`.exports` section).
///
/// ABI layout (x86_64, `#[repr(C)]`):
///   offset  0: magic [u8; 4]
///   offset  4: version u32
///   offset  8: num_exports u32
///   offset 12: _pad u32
///   offset 16: video_probe fn ptr (8 bytes)
///   offset 24: video_decode_frame fn ptr (8 bytes)
///   offset 32: image_probe fn ptr (8 bytes)  ← same offset as before
///   offset 40: image_decode fn ptr (8 bytes) ← same offset as before
#[repr(C)]
pub struct LibimageExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub _pad: u32,
    // Video exports (new)
    pub video_probe: extern "C" fn(*const u8, u32, *mut VideoInfo) -> i32,
    pub video_decode_frame: extern "C" fn(*const u8, u32, u32, u32, *mut u32, u32, *mut u8, u32) -> i32,
    // Image exports (unchanged offsets)
    pub image_probe: extern "C" fn(*const u8, u32, *mut ImageInfo) -> i32,
    pub image_decode: extern "C" fn(*const u8, u32, *mut u32, u32, *mut u8, u32) -> i32,
    // Scale export
    pub scale_image: extern "C" fn(*const u32, u32, u32, *mut u32, u32, u32, u32) -> i32,
}

#[link_section = ".exports"]
#[used]
#[no_mangle]
pub static LIBIMAGE_EXPORTS: LibimageExports = LibimageExports {
    magic: *b"DLIB",
    version: 1,
    num_exports: NUM_EXPORTS,
    _pad: 0,
    video_probe: video_probe_export,
    video_decode_frame: video_decode_frame_export,
    image_probe: image_probe,
    image_decode: image_decode,
    scale_image: scale_image_export,
};

// ── Video exports ──────────────────────────────────────

/// Probe a video file and return metadata.
extern "C" fn video_probe_export(data: *const u8, len: u32, info: *mut VideoInfo) -> i32 {
    if data.is_null() || info.is_null() || len < 32 {
        return crate::types::ERR_INVALID_DATA;
    }
    let data = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let out = unsafe { &mut *info };

    match crate::video::probe(data) {
        Some(i) => {
            *out = i;
            crate::types::ERR_OK
        }
        None => crate::types::ERR_UNSUPPORTED,
    }
}

/// Decode a single video frame into ARGB8888 pixels.
extern "C" fn video_decode_frame_export(
    data: *const u8, len: u32,
    num_frames: u32, frame_idx: u32,
    out_pixels: *mut u32, out_len: u32,
    scratch: *mut u8, scratch_len: u32,
) -> i32 {
    if data.is_null() || out_pixels.is_null() || len < 32 {
        return crate::types::ERR_INVALID_DATA;
    }
    let data = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let out = unsafe { core::slice::from_raw_parts_mut(out_pixels, out_len as usize) };
    let scratch = if scratch.is_null() || scratch_len == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(scratch, scratch_len as usize) }
    };

    crate::video::decode_frame(data, num_frames, frame_idx, out, scratch)
}

// ── Image exports ──────────────────────────────────────

/// Probe an image buffer and return metadata.
///
/// Detects format from magic bytes, parses header for dimensions,
/// and reports how much scratch buffer the decoder needs.
extern "C" fn image_probe(data: *const u8, len: u32, info: *mut ImageInfo) -> i32 {
    if data.is_null() || info.is_null() || len < 8 {
        return crate::types::ERR_INVALID_DATA;
    }
    let data = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let out = unsafe { &mut *info };

    // Try each format by magic bytes
    if let Some(i) = crate::bmp::probe(data) {
        *out = i;
        return crate::types::ERR_OK;
    }
    if let Some(i) = crate::png::probe(data) {
        *out = i;
        return crate::types::ERR_OK;
    }
    if let Some(i) = crate::jpeg::probe(data) {
        *out = i;
        return crate::types::ERR_OK;
    }
    if let Some(i) = crate::gif::probe(data) {
        *out = i;
        return crate::types::ERR_OK;
    }

    crate::types::ERR_UNSUPPORTED
}

/// Decode an image into ARGB8888 pixels.
///
/// - `data`/`len`: input image file data
/// - `out_pixels`: output ARGB8888 buffer (must be width*height u32s)
/// - `out_len`: size of output buffer in u32 elements
/// - `scratch`/`scratch_len`: working memory for decoder (size from `image_probe`)
extern "C" fn image_decode(
    data: *const u8, len: u32,
    out_pixels: *mut u32, out_len: u32,
    scratch: *mut u8, scratch_len: u32,
) -> i32 {
    if data.is_null() || out_pixels.is_null() || len < 8 {
        return crate::types::ERR_INVALID_DATA;
    }
    let data = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let out = unsafe { core::slice::from_raw_parts_mut(out_pixels, out_len as usize) };
    let scratch = if scratch.is_null() || scratch_len == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(scratch, scratch_len as usize) }
    };

    // Detect format and dispatch to decoder
    if data.len() >= 2 && data[0] == b'B' && data[1] == b'M' {
        return crate::bmp::decode(data, out);
    }
    if data.len() >= 8 && data[0..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return crate::png::decode(data, out, scratch);
    }
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
        return crate::jpeg::decode(data, out, scratch);
    }
    if data.len() >= 6 && (data[0..6] == *b"GIF87a" || data[0..6] == *b"GIF89a") {
        return crate::gif::decode(data, out, scratch);
    }

    crate::types::ERR_UNSUPPORTED
}

// ── Scale export ──────────────────────────────────────

/// Scale an ARGB8888 image using bilinear interpolation.
extern "C" fn scale_image_export(
    src: *const u32, src_w: u32, src_h: u32,
    dst: *mut u32, dst_w: u32, dst_h: u32,
    mode: u32,
) -> i32 {
    crate::scale::scale_image(src, src_w, src_h, dst, dst_w, dst_h, mode)
}
