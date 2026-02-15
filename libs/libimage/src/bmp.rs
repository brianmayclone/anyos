// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! BMP (Windows Bitmap) decoder.
//!
//! Supports uncompressed 24-bit and 32-bit BMP files with BITMAPINFOHEADER.

use crate::types::*;

fn read_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn read_i32(data: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

/// Probe a BMP file and return image metadata.
pub fn probe(data: &[u8]) -> Option<ImageInfo> {
    if data.len() < 54 || data[0] != b'B' || data[1] != b'M' {
        return None;
    }

    let dib_size = read_u32(data, 14);
    if dib_size < 40 {
        return None; // Only BITMAPINFOHEADER (40+) supported
    }

    let width = read_i32(data, 18);
    let height = read_i32(data, 22);
    let bpp = read_u16(data, 28);
    let compression = read_u32(data, 30);

    if width <= 0 || width > 16384 {
        return None;
    }
    let abs_height = if height < 0 { -height } else { height };
    if abs_height <= 0 || abs_height > 16384 {
        return None;
    }

    // Only uncompressed (0) or BITFIELDS (3) for 32-bit
    if compression != 0 && compression != 3 {
        return None;
    }
    if bpp != 24 && bpp != 32 {
        return None;
    }

    Some(ImageInfo {
        width: width as u32,
        height: abs_height as u32,
        format: FMT_BMP,
        scratch_needed: 0,
    })
}

/// Decode BMP data into ARGB8888 pixels.
pub fn decode(data: &[u8], out: &mut [u32]) -> i32 {
    if data.len() < 54 || data[0] != b'B' || data[1] != b'M' {
        return ERR_INVALID_DATA;
    }

    let pixel_offset = read_u32(data, 10) as usize;
    let width = read_i32(data, 18) as usize;
    let height_raw = read_i32(data, 22);
    let bpp = read_u16(data, 28) as usize;

    let top_down = height_raw < 0;
    let height = if top_down { (-height_raw) as usize } else { height_raw as usize };

    if out.len() < width * height {
        return ERR_BUFFER_TOO_SMALL;
    }

    let bytes_per_pixel = bpp / 8;
    // BMP rows are padded to 4-byte boundaries
    let row_stride = (width * bytes_per_pixel + 3) & !3;

    if data.len() < pixel_offset + row_stride * height {
        return ERR_INVALID_DATA;
    }

    for y in 0..height {
        // BMP stores rows bottom-up by default
        let src_y = if top_down { y } else { height - 1 - y };
        let row_start = pixel_offset + src_y * row_stride;

        for x in 0..width {
            let px = row_start + x * bytes_per_pixel;
            let b = data[px] as u32;
            let g = data[px + 1] as u32;
            let r = data[px + 2] as u32;
            let a = if bpp == 32 { data[px + 3] as u32 } else { 0xFF };

            out[y * width + x] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }

    ERR_OK
}
