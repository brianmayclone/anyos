// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Export table for libimage.dlib.

use crate::types::{ImageInfo, VideoInfo};

const NUM_EXPORTS: u32 = 11;

/// Export function table — must be first in the binary (`.exports` section).
#[repr(C)]
pub struct LibimageExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub _pad: u32,
    // Video exports
    pub video_probe: extern "C" fn(*const u8, u32, *mut VideoInfo) -> i32,
    pub video_decode_frame: extern "C" fn(*const u8, u32, u32, u32, *mut u32, u32, *mut u8, u32) -> i32,
    // Image exports
    pub image_probe: extern "C" fn(*const u8, u32, *mut ImageInfo) -> i32,
    pub image_decode: extern "C" fn(*const u8, u32, *mut u32, u32, *mut u8, u32) -> i32,
    // Scale export
    pub scale_image: extern "C" fn(*const u32, u32, u32, *mut u32, u32, u32, u32) -> i32,
    // ICO size-aware exports (appended — existing offsets unchanged)
    pub ico_probe_size: extern "C" fn(*const u8, u32, u32, *mut ImageInfo) -> i32,
    pub ico_decode_size: extern "C" fn(*const u8, u32, u32, *mut u32, u32, *mut u8, u32) -> i32,
    // BMP encoder
    pub image_encode: extern "C" fn(*const u32, u32, u32, *mut u8, u32) -> i32,
    // Iconpack SVG renderer
    pub iconpack_render: extern "C" fn(*const u8, u32, *const u8, u32, u32, u32, u32, *mut u32) -> i32,
    // Iconpack cached render (no pak data needed — uses internal cache)
    pub iconpack_render_cached: extern "C" fn(*const u8, u32, u32, u32, u32, *mut u32) -> i32,
    // Trim transparent borders and scale
    pub trim_and_scale: extern "C" fn(*const u32, u32, u32, *mut u32, u32, u32) -> i32,
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
    ico_probe_size: ico_probe_size_export,
    ico_decode_size: ico_decode_size_export,
    image_encode: image_encode_export,
    iconpack_render: iconpack_render_export,
    iconpack_render_cached: iconpack_render_cached_export,
    trim_and_scale: trim_and_scale_export,
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
    if let Some(i) = crate::ico::probe(data) {
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
    // ICO: reserved=0, type=1(ICO) or 2(CUR), count>0
    if data.len() >= 6 {
        let reserved = u16::from_le_bytes([data[0], data[1]]);
        let ico_type = u16::from_le_bytes([data[2], data[3]]);
        let count = u16::from_le_bytes([data[4], data[5]]);
        if reserved == 0 && (ico_type == 1 || ico_type == 2) && count > 0 && count < 256 {
            return crate::ico::decode(data, out, scratch);
        }
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

// ── ICO size-aware exports ───────────────────────────

/// Probe an ICO file selecting the best entry for a preferred size.
extern "C" fn ico_probe_size_export(
    data: *const u8, len: u32, preferred_size: u32, info: *mut ImageInfo,
) -> i32 {
    if data.is_null() || info.is_null() || len < 6 {
        return crate::types::ERR_INVALID_DATA;
    }
    let data = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let out = unsafe { &mut *info };

    match crate::ico::probe_for_size(data, preferred_size) {
        Some(i) => {
            *out = i;
            crate::types::ERR_OK
        }
        None => crate::types::ERR_UNSUPPORTED,
    }
}

/// Decode an ICO file selecting the best entry for a preferred size.
extern "C" fn ico_decode_size_export(
    data: *const u8, len: u32, preferred_size: u32,
    out_pixels: *mut u32, out_len: u32,
    scratch: *mut u8, scratch_len: u32,
) -> i32 {
    if data.is_null() || out_pixels.is_null() || len < 6 {
        return crate::types::ERR_INVALID_DATA;
    }
    let data = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let out = unsafe { core::slice::from_raw_parts_mut(out_pixels, out_len as usize) };
    let scratch = if scratch.is_null() || scratch_len == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(scratch, scratch_len as usize) }
    };

    crate::ico::decode_for_size(data, preferred_size, out, scratch)
}

// ── BMP encode export ───────────────────────────────

/// Encode ARGB8888 pixels into BMP format.
///
/// - `pixels`/`width`/`height`: source image (ARGB8888, width*height u32s)
/// - `out`/`out_len`: output buffer for BMP file bytes
///
/// Returns total bytes written on success, or a negative error code.
extern "C" fn image_encode_export(
    pixels: *const u32, width: u32, height: u32,
    out: *mut u8, out_len: u32,
) -> i32 {
    if pixels.is_null() || out.is_null() || width == 0 || height == 0 {
        return crate::types::ERR_INVALID_DATA;
    }
    let count = (width as usize) * (height as usize);
    let px = unsafe { core::slice::from_raw_parts(pixels, count) };
    let buf = unsafe { core::slice::from_raw_parts_mut(out, out_len as usize) };
    crate::bmp::encode(px, width, height, buf)
}

// ── Iconpack render export ───────────────────────────

/// Render a system icon from an ico.pak file to ARGB8888 pixels.
///
/// - `pak`/`pak_len`: ico.pak file data
/// - `name`/`name_len`: icon name (UTF-8)
/// - `filled`: 1 for filled, 0 for outline
/// - `size`: target pixel size (square)
/// - `color`: ARGB8888 color
/// - `out_pixels`: output buffer (size*size u32s)
///
/// Returns 0 on success, negative on error.
///
/// Supports both IPAK v1 (SVG paths, runtime rasterized) and v2 (pre-rasterized
/// alpha maps, color applied at runtime).
extern "C" fn iconpack_render_export(
    pak: *const u8, pak_len: u32,
    name: *const u8, name_len: u32,
    filled: u32, size: u32, color: u32,
    out_pixels: *mut u32,
) -> i32 {
    if pak.is_null() || name.is_null() || out_pixels.is_null() || size == 0 || size > 512 {
        return crate::types::ERR_INVALID_DATA;
    }
    let pak_data = unsafe { core::slice::from_raw_parts(pak, pak_len as usize) };
    let name_data = unsafe { core::slice::from_raw_parts(name, name_len as usize) };
    let pixel_count = (size as usize) * (size as usize);
    let out = unsafe { core::slice::from_raw_parts_mut(out_pixels, pixel_count) };

    render_from_pak(pak_data, name_data, filled != 0, size, color, out)
}

/// Cached variant: uses ico.pak from internal cache (lazy-loaded on first call).
extern "C" fn iconpack_render_cached_export(
    name: *const u8, name_len: u32,
    filled: u32, size: u32, color: u32,
    out_pixels: *mut u32,
) -> i32 {
    if name.is_null() || out_pixels.is_null() || size == 0 || size > 512 {
        return crate::types::ERR_INVALID_DATA;
    }
    let name_data = unsafe { core::slice::from_raw_parts(name, name_len as usize) };
    let pixel_count = (size as usize) * (size as usize);
    let out = unsafe { core::slice::from_raw_parts_mut(out_pixels, pixel_count) };

    let pak_data = match crate::iconpack::cached_pak() {
        Some(d) => d,
        None => return crate::types::ERR_UNSUPPORTED,
    };

    render_from_pak(pak_data, name_data, filled != 0, size, color, out)
}

// ── Trim-and-scale export ───────────────────────────

/// Trim transparent borders from an ARGB8888 image and scale to fill destination.
extern "C" fn trim_and_scale_export(
    src: *const u32, src_w: u32, src_h: u32,
    dst: *mut u32, dst_w: u32, dst_h: u32,
) -> i32 {
    crate::scale::trim_and_scale(src, src_w, src_h, dst, dst_w, dst_h)
}

/// Shared render logic for both export variants.
fn render_from_pak(pak_data: &[u8], name_data: &[u8], filled: bool, size: u32, color: u32, out: &mut [u32]) -> i32 {
    let pixel_count = (size as usize) * (size as usize);
    let ver = crate::iconpack::version(pak_data);

    if ver == 2 {
        let entry = match crate::iconpack::lookup_v2(pak_data, name_data, filled) {
            Some(e) => e,
            None => return crate::types::ERR_UNSUPPORTED,
        };

        let isz = entry.icon_size as u32;
        let ca = (color >> 24) & 0xFF;
        let color_rgb = color & 0x00FFFFFF;

        if size == isz {
            for i in 0..pixel_count {
                let alpha = entry.alpha[i] as u32;
                if alpha == 0 {
                    out[i] = 0;
                } else {
                    let a = (alpha * ca + 127) / 255;
                    out[i] = (a << 24) | color_rgb;
                }
            }
        } else {
            let src_count = (isz * isz) as usize;
            let mut tmp = alloc::vec![0u32; src_count];
            for i in 0..src_count {
                let alpha = entry.alpha[i] as u32;
                if alpha == 0 {
                    tmp[i] = 0;
                } else {
                    let a = (alpha * ca + 127) / 255;
                    tmp[i] = (a << 24) | color_rgb;
                }
            }
            crate::scale::scale_image(
                tmp.as_ptr(), isz, isz,
                out.as_mut_ptr(), size, size,
                crate::scale::MODE_SCALE,
            );
        }

        0
    } else {
        let entry = match crate::iconpack::lookup(pak_data, name_data, filled) {
            Some(e) => e,
            None => return crate::types::ERR_UNSUPPORTED,
        };

        crate::svg_raster::render_icon(entry.data, filled, size, color, out)
    }
}
