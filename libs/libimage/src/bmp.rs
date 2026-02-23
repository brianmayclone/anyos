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

/// Encode ARGB8888 pixels into BMP format (32-bit uncompressed).
///
/// - `pixels`: ARGB8888 pixel data (width * height elements)
/// - `width`, `height`: image dimensions
/// - `out`: output buffer for BMP file data
///
/// Returns total bytes written on success, or a negative error code.
pub fn encode(pixels: &[u32], width: u32, height: u32, out: &mut [u8]) -> i32 {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || pixels.len() < w * h {
        return ERR_INVALID_DATA;
    }

    let row_size = w * 4; // 32-bit, no padding needed (already 4-byte aligned)
    let pixel_data_size = row_size * h;
    let header_size = 14 + 40; // BMP header + BITMAPINFOHEADER
    let file_size = header_size + pixel_data_size;

    if out.len() < file_size {
        return ERR_BUFFER_TOO_SMALL;
    }

    // ── BMP File Header (14 bytes) ──
    out[0] = b'B';
    out[1] = b'M';
    write_u32(out, 2, file_size as u32);
    write_u32(out, 6, 0);  // reserved
    write_u32(out, 10, header_size as u32); // pixel data offset

    // ── BITMAPINFOHEADER (40 bytes) ──
    write_u32(out, 14, 40); // header size
    write_i32(out, 18, width as i32);
    write_i32(out, 22, -(height as i32)); // negative = top-down
    write_u16(out, 26, 1);  // color planes
    write_u16(out, 28, 32); // bits per pixel
    write_u32(out, 30, 0);  // no compression
    write_u32(out, 34, pixel_data_size as u32);
    write_u32(out, 38, 2835); // ~72 DPI horizontal
    write_u32(out, 42, 2835); // ~72 DPI vertical
    write_u32(out, 46, 0);  // colors in palette
    write_u32(out, 50, 0);  // important colors

    // ── Pixel data (top-down, BGRA order) ──
    let mut off = header_size;
    for y in 0..h {
        for x in 0..w {
            let argb = pixels[y * w + x];
            let a = (argb >> 24) & 0xFF;
            let r = (argb >> 16) & 0xFF;
            let g = (argb >> 8) & 0xFF;
            let b = argb & 0xFF;
            out[off] = b as u8;
            out[off + 1] = g as u8;
            out[off + 2] = r as u8;
            out[off + 3] = a as u8;
            off += 4;
        }
    }

    file_size as i32
}

fn write_u16(buf: &mut [u8], off: usize, val: u16) {
    let bytes = val.to_le_bytes();
    buf[off] = bytes[0];
    buf[off + 1] = bytes[1];
}

fn write_u32(buf: &mut [u8], off: usize, val: u32) {
    let bytes = val.to_le_bytes();
    buf[off] = bytes[0];
    buf[off + 1] = bytes[1];
    buf[off + 2] = bytes[2];
    buf[off + 3] = bytes[3];
}

fn write_i32(buf: &mut [u8], off: usize, val: i32) {
    let bytes = val.to_le_bytes();
    buf[off] = bytes[0];
    buf[off + 1] = bytes[1];
    buf[off + 2] = bytes[2];
    buf[off + 3] = bytes[3];
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
