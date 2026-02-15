// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! ICO (Windows Icon) decoder.
//!
//! Supports ICO files containing BMP or PNG embedded images.
//! Selects the best entry (prefers 16x16, then smallest >= 16, then first).
//! BMP-in-ICO: handles 32-bit BGRA, 24-bit BGR, 8-bit and 4-bit palette,
//! and 1-bit monochrome, with AND mask transparency.
//! PNG-in-ICO: delegates to the PNG decoder.

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

/// ICO directory entry (parsed from 16-byte record).
struct IcoEntry {
    width: u32,      // 0 means 256
    height: u32,     // 0 means 256
    data_size: u32,
    data_offset: u32,
}

/// Check if data is a valid ICO file.
fn is_ico(data: &[u8]) -> bool {
    if data.len() < 6 {
        return false;
    }
    let reserved = read_u16(data, 0);
    let ico_type = read_u16(data, 2);
    let count = read_u16(data, 4);
    reserved == 0 && (ico_type == 1 || ico_type == 2) && count > 0 && count < 256
}

/// Parse the ICO directory.
fn parse_directory(data: &[u8]) -> Option<(u16, u16)> {
    // Returns (count, ico_type)
    if !is_ico(data) {
        return None;
    }
    Some((read_u16(data, 4), read_u16(data, 2)))
}

/// Read a single directory entry.
fn read_entry(data: &[u8], index: usize) -> Option<IcoEntry> {
    let off = 6 + index * 16;
    if off + 16 > data.len() {
        return None;
    }
    let w = data[off] as u32;
    let h = data[off + 1] as u32;
    let data_size = read_u32(data, off + 8);
    let data_offset = read_u32(data, off + 12);

    Some(IcoEntry {
        width: if w == 0 { 256 } else { w },
        height: if h == 0 { 256 } else { h },
        data_size,
        data_offset,
    })
}

/// Select the best entry index: prefer 16x16, then smallest >= 16, then first.
fn best_entry_index(data: &[u8], count: u16) -> usize {
    best_entry_for_size(data, count, 16)
}

/// Select the best entry index for a given preferred size.
/// Prefers exact match, then next-larger (downscale > upscale), then closest.
fn best_entry_for_size(data: &[u8], count: u16, preferred: u32) -> usize {
    let mut best_idx = 0;
    let mut best_diff: i32 = i32::MAX;

    for i in 0..count as usize {
        if let Some(e) = read_entry(data, i) {
            if e.width == preferred && e.height == preferred {
                return i;
            }
            // Prefer entries >= preferred (downscaling is better than upscaling)
            let diff = if e.width >= preferred {
                (e.width as i32 - preferred as i32)
            } else {
                (preferred as i32 - e.width as i32) + 1000
            };
            if diff < best_diff {
                best_diff = diff;
                best_idx = i;
            }
        }
    }
    best_idx
}

/// Check if the embedded data is PNG (starts with PNG magic).
fn is_png_data(data: &[u8]) -> bool {
    data.len() >= 8 && data[0..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
}

/// Probe an ICO file and return metadata for the best entry.
pub fn probe(data: &[u8]) -> Option<ImageInfo> {
    probe_for_size(data, 16)
}

/// Probe an ICO file, selecting the best entry for `preferred_size`.
pub fn probe_for_size(data: &[u8], preferred_size: u32) -> Option<ImageInfo> {
    let (count, _) = parse_directory(data)?;
    let idx = best_entry_for_size(data, count, preferred_size);
    let entry = read_entry(data, idx)?;

    let off = entry.data_offset as usize;
    let size = entry.data_size as usize;
    if off + size > data.len() {
        return None;
    }

    let embedded = &data[off..off + size];

    let scratch_needed = if is_png_data(embedded) {
        match crate::png::probe(embedded) {
            Some(info) => info.scratch_needed,
            None => return None,
        }
    } else {
        0
    };

    Some(ImageInfo {
        width: entry.width,
        height: entry.height,
        format: FMT_ICO,
        scratch_needed,
    })
}

/// Decode the best entry of an ICO file into ARGB8888 pixels.
pub fn decode(data: &[u8], out: &mut [u32], scratch: &mut [u8]) -> i32 {
    decode_for_size(data, 16, out, scratch)
}

/// Decode the best entry for `preferred_size` of an ICO file into ARGB8888 pixels.
pub fn decode_for_size(data: &[u8], preferred_size: u32, out: &mut [u32], scratch: &mut [u8]) -> i32 {
    let (count, _) = match parse_directory(data) {
        Some(v) => v,
        None => return ERR_INVALID_DATA,
    };

    let idx = best_entry_for_size(data, count, preferred_size);
    let entry = match read_entry(data, idx) {
        Some(e) => e,
        None => return ERR_INVALID_DATA,
    };

    let off = entry.data_offset as usize;
    let size = entry.data_size as usize;
    if off + size > data.len() {
        return ERR_INVALID_DATA;
    }

    let embedded = &data[off..off + size];
    let w = entry.width as usize;
    let h = entry.height as usize;

    if out.len() < w * h {
        return ERR_BUFFER_TOO_SMALL;
    }

    if is_png_data(embedded) {
        return crate::png::decode(embedded, out, scratch);
    }

    decode_bmp_dib(embedded, w, h, out)
}

/// Decode a BMP DIB (BITMAPINFOHEADER) embedded in an ICO entry.
fn decode_bmp_dib(dib: &[u8], expected_w: usize, expected_h: usize, out: &mut [u32]) -> i32 {
    if dib.len() < 40 {
        return ERR_INVALID_DATA;
    }

    let dib_size = read_u32(dib, 0);
    if dib_size < 40 {
        return ERR_INVALID_DATA;
    }

    let width = read_i32(dib, 4) as usize;
    // Height in ICO DIB is doubled (image + AND mask)
    let raw_height = read_i32(dib, 8);
    let height = (raw_height.unsigned_abs() as usize) / 2;
    let bpp = read_u16(dib, 14) as usize;
    let compression = read_u32(dib, 16);

    // Validate dimensions match the directory entry
    if width != expected_w || height != expected_h {
        // Some ICO files have mismatched dimensions, try to use the DIB values
        if width == 0 || height == 0 {
            return ERR_INVALID_DATA;
        }
    }

    let actual_w = width;
    let actual_h = height;

    if out.len() < actual_w * actual_h {
        return ERR_BUFFER_TOO_SMALL;
    }

    // Only uncompressed (BI_RGB = 0) or BITFIELDS (3) supported
    if compression != 0 && compression != 3 {
        return ERR_UNSUPPORTED;
    }

    // Palette offset (right after DIB header)
    let palette_off = dib_size as usize;

    match bpp {
        32 => decode_32bpp(dib, palette_off, actual_w, actual_h, out),
        24 => decode_24bpp(dib, palette_off, actual_w, actual_h, out),
        8 => decode_palette(dib, palette_off, actual_w, actual_h, 8, out),
        4 => decode_palette(dib, palette_off, actual_w, actual_h, 4, out),
        1 => decode_palette(dib, palette_off, actual_w, actual_h, 1, out),
        _ => ERR_UNSUPPORTED,
    }
}

/// Decode 32-bit BGRA data. Alpha comes from pixel data (AND mask ignored).
fn decode_32bpp(dib: &[u8], pixel_off: usize, w: usize, h: usize, out: &mut [u32]) -> i32 {
    let row_stride = w * 4;
    let needed = pixel_off + row_stride * h;
    if dib.len() < needed {
        return ERR_INVALID_DATA;
    }

    for y in 0..h {
        // Bottom-up row order
        let src_y = h - 1 - y;
        let row_start = pixel_off + src_y * row_stride;
        for x in 0..w {
            let px = row_start + x * 4;
            let b = dib[px] as u32;
            let g = dib[px + 1] as u32;
            let r = dib[px + 2] as u32;
            let a = dib[px + 3] as u32;
            out[y * w + x] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }

    ERR_OK
}

/// Decode 24-bit BGR data with AND mask for transparency.
fn decode_24bpp(dib: &[u8], pixel_off: usize, w: usize, h: usize, out: &mut [u32]) -> i32 {
    let row_stride = (w * 3 + 3) & !3; // 4-byte aligned rows
    let pixel_data_size = row_stride * h;
    let mask_row_stride = (w + 31) / 32 * 4; // 1-bit mask, 4-byte aligned
    let mask_off = pixel_off + pixel_data_size;

    if dib.len() < pixel_off + pixel_data_size {
        return ERR_INVALID_DATA;
    }
    let has_mask = dib.len() >= mask_off + mask_row_stride * h;

    for y in 0..h {
        let src_y = h - 1 - y;
        let row_start = pixel_off + src_y * row_stride;
        for x in 0..w {
            let px = row_start + x * 3;
            if px + 2 >= dib.len() {
                continue;
            }
            let b = dib[px] as u32;
            let g = dib[px + 1] as u32;
            let r = dib[px + 2] as u32;

            // Check AND mask: 1 = transparent, 0 = opaque
            let a = if has_mask {
                let mask_row = mask_off + src_y * mask_row_stride;
                let byte_idx = mask_row + x / 8;
                let bit_idx = 7 - (x % 8);
                if byte_idx < dib.len() && (dib[byte_idx] >> bit_idx) & 1 == 1 {
                    0u32 // transparent
                } else {
                    0xFFu32 // opaque
                }
            } else {
                0xFFu32
            };

            out[y * w + x] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }

    ERR_OK
}

/// Decode palette-based (1, 4, or 8 bpp) data with AND mask.
fn decode_palette(
    dib: &[u8],
    palette_off: usize,
    w: usize,
    h: usize,
    bpp: usize,
    out: &mut [u32],
) -> i32 {
    let num_colors = 1usize << bpp;
    let palette_size = num_colors * 4; // BGRA entries (4 bytes each)

    if dib.len() < palette_off + palette_size {
        return ERR_INVALID_DATA;
    }

    // Read palette (BGRX format, 4 bytes per entry)
    let mut palette = [0u32; 256];
    for i in 0..num_colors {
        let off = palette_off + i * 4;
        let b = dib[off] as u32;
        let g = dib[off + 1] as u32;
        let r = dib[off + 2] as u32;
        palette[i] = 0xFF000000 | (r << 16) | (g << 8) | b;
    }

    let pixel_off = palette_off + palette_size;

    // Row stride for pixel data (bits per row, 4-byte aligned)
    let bits_per_row = w * bpp;
    let row_stride = (bits_per_row + 31) / 32 * 4;
    let pixel_data_size = row_stride * h;

    // AND mask
    let mask_row_stride = (w + 31) / 32 * 4;
    let mask_off = pixel_off + pixel_data_size;

    if dib.len() < pixel_off + pixel_data_size {
        return ERR_INVALID_DATA;
    }
    let has_mask = dib.len() >= mask_off + mask_row_stride * h;

    for y in 0..h {
        let src_y = h - 1 - y;
        let row_start = pixel_off + src_y * row_stride;

        for x in 0..w {
            // Extract palette index based on bpp
            let idx = match bpp {
                8 => {
                    let byte_off = row_start + x;
                    if byte_off >= dib.len() { 0 } else { dib[byte_off] as usize }
                }
                4 => {
                    let byte_off = row_start + x / 2;
                    if byte_off >= dib.len() {
                        0
                    } else {
                        let byte = dib[byte_off];
                        if x % 2 == 0 {
                            (byte >> 4) as usize
                        } else {
                            (byte & 0x0F) as usize
                        }
                    }
                }
                1 => {
                    let byte_off = row_start + x / 8;
                    let bit = 7 - (x % 8);
                    if byte_off >= dib.len() {
                        0
                    } else {
                        ((dib[byte_off] >> bit) & 1) as usize
                    }
                }
                _ => 0,
            };

            let color = if idx < num_colors { palette[idx] } else { 0xFF000000 };

            // Apply AND mask
            let a = if has_mask {
                let mask_row = mask_off + src_y * mask_row_stride;
                let byte_idx = mask_row + x / 8;
                let bit_idx = 7 - (x % 8);
                if byte_idx < dib.len() && (dib[byte_idx] >> bit_idx) & 1 == 1 {
                    0u32
                } else {
                    0xFFu32
                }
            } else {
                (color >> 24) & 0xFF
            };

            out[y * w + x] = (a << 24) | (color & 0x00FFFFFF);
        }
    }

    ERR_OK
}
