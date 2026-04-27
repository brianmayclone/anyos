// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! PNG decoder (non-interlaced RGB/RGBA/Grayscale and indexed-color PNGs).

use crate::types::*;
use crate::deflate;

const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn read_u32_be(data: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

/// Color types we support.
const CT_GRAY: u8 = 0;
const CT_RGB: u8 = 2;
const CT_PALETTE: u8 = 3;
const CT_RGBA: u8 = 6;

/// Bytes per pixel for supported color types.
fn bpp(color_type: u8) -> usize {
    match color_type {
        CT_GRAY => 1,
        CT_RGB => 3,
        CT_RGBA => 4,
        _ => 0,
    }
}

fn raw_row_bytes(width: usize, bit_depth: u8, color_type: u8) -> Option<usize> {
    match color_type {
        CT_GRAY | CT_RGB | CT_RGBA if bit_depth == 8 => Some(width.checked_mul(bpp(color_type))?),
        CT_PALETTE if matches!(bit_depth, 1 | 2 | 4 | 8) => {
            Some((width.checked_mul(bit_depth as usize)? + 7) / 8)
        }
        _ => None,
    }
}

/// Probe a PNG file.
pub fn probe(data: &[u8]) -> Option<ImageInfo> {
    if data.len() < 33 || data[0..8] != PNG_SIG {
        return None;
    }

    // IHDR must be first chunk
    let chunk_len = read_u32_be(data, 8) as usize;
    if &data[12..16] != b"IHDR" || chunk_len != 13 {
        return None;
    }

    let width = read_u32_be(data, 16);
    let height = read_u32_be(data, 20);
    let bit_depth = data[24];
    let color_type = data[25];

    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return None;
    }
    let row_bytes = raw_row_bytes(width as usize, bit_depth, color_type)?;
    if color_type != CT_PALETTE && bpp(color_type) == 0 {
        return None;
    }

    // Count total IDAT compressed data size (decode needs this in scratch)
    let mut idat_total = 0usize;
    let mut cpos = 8usize;
    while cpos + 12 <= data.len() {
        let clen = read_u32_be(data, cpos) as usize;
        if &data[cpos + 4..cpos + 8] == b"IDAT" {
            idat_total += clen;
        }
        cpos += 12 + clen;
    }

    // Scratch: 32K deflate window + decompressed scanlines + IDAT compressed data
    let decompressed_size = (row_bytes + 1) * height as usize;
    let scratch = 32768 + decompressed_size + idat_total.max(1024);

    Some(ImageInfo {
        width,
        height,
        format: FMT_PNG,
        scratch_needed: scratch as u32,
    })
}

/// Decode PNG data into ARGB8888 pixels.
pub fn decode(data: &[u8], out: &mut [u32], scratch: &mut [u8]) -> i32 {
    if data.len() < 33 || data[0..8] != PNG_SIG {
        return ERR_INVALID_DATA;
    }

    // Parse IHDR
    let width = read_u32_be(data, 16) as usize;
    let height = read_u32_be(data, 20) as usize;
    let bit_depth = data[24];
    let color_type = data[25];
    let Some(row_bytes) = raw_row_bytes(width, bit_depth, color_type) else {
        return ERR_UNSUPPORTED;
    };

    if out.len() < width * height {
        return ERR_BUFFER_TOO_SMALL;
    }

    // Collect all IDAT chunks
    let mut idat_total = 0usize;
    let mut pos = 8;
    let mut palette = [0xFF000000u32; 256];
    let mut palette_len = 0usize;

    // First pass: count IDAT data size
    while pos + 12 <= data.len() {
        let chunk_len = read_u32_be(data, pos) as usize;
        if pos + 12 + chunk_len > data.len() {
            return ERR_INVALID_DATA;
        }
        let chunk_type = &data[pos + 4..pos + 8];
        if chunk_type == b"IDAT" {
            idat_total += chunk_len;
        } else if chunk_type == b"PLTE" {
            palette_len = (chunk_len / 3).min(256);
            let chunk = &data[pos + 8..pos + 8 + chunk_len];
            for i in 0..palette_len {
                let r = chunk[i * 3] as u32;
                let g = chunk[i * 3 + 1] as u32;
                let b = chunk[i * 3 + 2] as u32;
                palette[i] = 0xFF000000 | (r << 16) | (g << 8) | b;
            }
        } else if chunk_type == b"tRNS" && color_type == CT_PALETTE {
            let chunk = &data[pos + 8..pos + 8 + chunk_len];
            let count = chunk.len().min(palette_len);
            for i in 0..count {
                palette[i] = (palette[i] & 0x00FF_FFFF) | ((chunk[i] as u32) << 24);
            }
        }
        pos += 12 + chunk_len; // length + type + data + crc
    }

    if idat_total == 0 {
        return ERR_INVALID_DATA;
    }
    if color_type == CT_PALETTE && palette_len == 0 {
        return ERR_UNSUPPORTED;
    }

    // We need scratch for: deflate window (32K) + decompressed data + row buffer
    let scanline_size = row_bytes + 1; // +1 for filter byte
    let decompressed_size = scanline_size * height;
    let needed = 32768 + decompressed_size + row_bytes;
    if scratch.len() < needed {
        return ERR_SCRATCH_TOO_SMALL;
    }

    let decomp_area_start = 32768;

    // We need a temporary area for the compressed IDAT data
    // Place it after the decompression output area
    let idat_buf_start = decomp_area_start + decompressed_size;
    if idat_buf_start + idat_total > scratch.len() {
        return ERR_SCRATCH_TOO_SMALL;
    }

    // Second pass: copy IDAT data
    pos = 8;
    let mut idat_off = 0usize;
    while pos + 12 <= data.len() {
        let chunk_len = read_u32_be(data, pos) as usize;
        if pos + 12 + chunk_len > data.len() {
            return ERR_INVALID_DATA;
        }
        let chunk_type = &data[pos + 4..pos + 8];
        if chunk_type == b"IDAT" {
            let src = &data[pos + 8..pos + 8 + chunk_len];
            let dst = &mut scratch[idat_buf_start + idat_off..idat_buf_start + idat_off + chunk_len];
            dst.copy_from_slice(src);
            idat_off += chunk_len;
        }
        pos += 12 + chunk_len;
    }

    let idat_data = &scratch[idat_buf_start..idat_buf_start + idat_total];

    // Strip zlib header (2 bytes: CMF + FLG)
    if idat_data.len() < 6 {
        return ERR_INVALID_DATA;
    }
    let cmf = idat_data[0];
    if cmf & 0x0F != 8 {
        return ERR_UNSUPPORTED; // Not deflate
    }
    // We can't use decomp_area as both input and output since input is in scratch.
    // Instead, we need to decompress directly. Since idat_data borrows scratch immutably
    // but we need mutable access, we must copy idat_data out. But we're in a DLL with no heap...
    // Solution: deflate reads from the IDAT region which is past our output region.
    // The output region is [decomp_area_start..decomp_area_start+decompressed_size]
    // The input region is [idat_buf_start..idat_buf_start+idat_total]
    // These don't overlap since idat_buf_start = decomp_area_start + decompressed_size.

    // We need to split scratch into non-overlapping parts. Use unsafe pointer arithmetic.
    let scratch_ptr = scratch.as_mut_ptr();
    let window = unsafe { core::slice::from_raw_parts_mut(scratch_ptr, 32768) };
    let decomp_out = unsafe {
        core::slice::from_raw_parts_mut(scratch_ptr.add(decomp_area_start), decompressed_size)
    };
    let deflate_input = unsafe {
        core::slice::from_raw_parts(scratch_ptr.add(idat_buf_start + 2) as *const u8,
                                    idat_total.saturating_sub(2))
    };

    let decompressed_len = deflate::decompress(deflate_input, decomp_out, window);
    if decompressed_len < 0 {
        return ERR_INVALID_DATA;
    }
    let decompressed_len = decompressed_len as usize;
    if decompressed_len < scanline_size * height {
        return ERR_INVALID_DATA;
    }

    if color_type == CT_PALETTE {
        let decomp = unsafe {
            core::slice::from_raw_parts_mut(scratch_ptr.add(decomp_area_start), decompressed_len)
        };
        return reconstruct_indexed(decomp, width, height, bit_depth, &palette, palette_len, out);
    }

    // Reconstruct filtered scanlines
    let decomp = unsafe {
        core::slice::from_raw_parts(scratch_ptr.add(decomp_area_start) as *const u8, decompressed_len)
    };
    reconstruct_and_convert(decomp, width, height, color_type, out)
}

fn reconstruct_indexed(
    scanlines: &mut [u8],
    width: usize,
    height: usize,
    bit_depth: u8,
    palette: &[u32; 256],
    palette_len: usize,
    out: &mut [u32],
) -> i32 {
    let row_bytes = match raw_row_bytes(width, bit_depth, CT_PALETTE) {
        Some(v) => v,
        None => return ERR_UNSUPPORTED,
    };
    let scanline_size = row_bytes + 1;

    for y in 0..height {
        let row_start = y * scanline_size;
        if row_start + scanline_size > scanlines.len() {
            return ERR_INVALID_DATA;
        }
        let filter = scanlines[row_start];
        let prev_start = if y > 0 { Some((y - 1) * scanline_size) } else { None };

        for x in 0..row_bytes {
            let idx = row_start + 1 + x;
            let raw = scanlines[idx];
            let left = if x > 0 { scanlines[idx - 1] } else { 0 };
            let above = prev_start.map(|p| scanlines[p + 1 + x]).unwrap_or(0);
            let upper_left = if x > 0 {
                prev_start.map(|p| scanlines[p + x]).unwrap_or(0)
            } else {
                0
            };
            scanlines[idx] = match filter {
                0 => raw,
                1 => raw.wrapping_add(left),
                2 => raw.wrapping_add(above),
                3 => raw.wrapping_add(((left as u16 + above as u16) / 2) as u8),
                4 => raw.wrapping_add(paeth(left, above, upper_left)),
                _ => return ERR_INVALID_DATA,
            };
        }

        let row = &scanlines[row_start + 1..row_start + 1 + row_bytes];
        for x in 0..width {
            let palette_index = match bit_depth {
                8 => row[x] as usize,
                4 => {
                    let b = row[x / 2];
                    if x & 1 == 0 { (b >> 4) as usize } else { (b & 0x0F) as usize }
                }
                2 => {
                    let b = row[x / 4];
                    let shift = 6 - ((x & 3) * 2);
                    ((b >> shift) & 0x03) as usize
                }
                1 => {
                    let b = row[x / 8];
                    let shift = 7 - (x & 7);
                    ((b >> shift) & 0x01) as usize
                }
                _ => return ERR_UNSUPPORTED,
            };
            if palette_index >= palette_len {
                return ERR_INVALID_DATA;
            }
            out[y * width + x] = palette[palette_index];
        }
    }

    ERR_OK
}

/// Apply PNG filter reconstruction and convert to ARGB8888.
/// Dispatches to a specialized routine per color type to avoid per-pixel
/// match overhead on color_type and channel index.
fn reconstruct_and_convert(
    scanlines: &[u8],
    width: usize,
    height: usize,
    color_type: u8,
    out: &mut [u32],
) -> i32 {
    match color_type {
        CT_RGB  => reconstruct_rgb(scanlines, width, height, out),
        CT_RGBA => reconstruct_rgba(scanlines, width, height, out),
        CT_GRAY => reconstruct_gray(scanlines, width, height, out),
        _ => ERR_UNSUPPORTED,
    }
}

/// RGB reconstruction — inline channel extraction via shift+mask.
fn reconstruct_rgb(scanlines: &[u8], width: usize, height: usize, out: &mut [u32]) -> i32 {
    let row_bytes = width * 3;
    let scanline_size = row_bytes + 1;

    for y in 0..height {
        let src_off = y * scanline_size;
        let filter = scanlines[src_off];
        let row_data = &scanlines[src_off + 1..src_off + 1 + row_bytes];

        let mut prev_r: u8 = 0;
        let mut prev_g: u8 = 0;
        let mut prev_b: u8 = 0;

        for x in 0..width {
            let px_off = x * 3;
            let raw_r = row_data[px_off];
            let raw_g = row_data[px_off + 1];
            let raw_b = row_data[px_off + 2];

            let (above_r, above_g, above_b) = if y > 0 {
                let p = out[(y - 1) * width + x];
                (((p >> 16) & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, (p & 0xFF) as u8)
            } else {
                (0, 0, 0)
            };

            let (r, g, b) = match filter {
                0 => (raw_r, raw_g, raw_b),
                1 => (
                    raw_r.wrapping_add(prev_r),
                    raw_g.wrapping_add(prev_g),
                    raw_b.wrapping_add(prev_b),
                ),
                2 => (
                    raw_r.wrapping_add(above_r),
                    raw_g.wrapping_add(above_g),
                    raw_b.wrapping_add(above_b),
                ),
                3 => (
                    raw_r.wrapping_add(((prev_r as u16 + above_r as u16) / 2) as u8),
                    raw_g.wrapping_add(((prev_g as u16 + above_g as u16) / 2) as u8),
                    raw_b.wrapping_add(((prev_b as u16 + above_b as u16) / 2) as u8),
                ),
                4 => {
                    let (al_r, al_g, al_b) = if x > 0 && y > 0 {
                        let p = out[(y - 1) * width + x - 1];
                        (((p >> 16) & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, (p & 0xFF) as u8)
                    } else {
                        (0, 0, 0)
                    };
                    (
                        raw_r.wrapping_add(paeth(prev_r, above_r, al_r)),
                        raw_g.wrapping_add(paeth(prev_g, above_g, al_g)),
                        raw_b.wrapping_add(paeth(prev_b, above_b, al_b)),
                    )
                }
                _ => (raw_r, raw_g, raw_b),
            };

            prev_r = r;
            prev_g = g;
            prev_b = b;
            out[y * width + x] = 0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        }
    }
    ERR_OK
}

/// RGBA reconstruction.
fn reconstruct_rgba(scanlines: &[u8], width: usize, height: usize, out: &mut [u32]) -> i32 {
    let row_bytes = width * 4;
    let scanline_size = row_bytes + 1;

    for y in 0..height {
        let src_off = y * scanline_size;
        let filter = scanlines[src_off];
        let row_data = &scanlines[src_off + 1..src_off + 1 + row_bytes];

        let mut prev: [u8; 4] = [0; 4];

        for x in 0..width {
            let px_off = x * 4;
            let raw = [row_data[px_off], row_data[px_off + 1], row_data[px_off + 2], row_data[px_off + 3]];

            let above = if y > 0 {
                let p = out[(y - 1) * width + x];
                [((p >> 16) & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, (p & 0xFF) as u8, ((p >> 24) & 0xFF) as u8]
            } else {
                [0; 4]
            };

            let mut ch = [0u8; 4];
            match filter {
                0 => { ch = raw; }
                1 => { for i in 0..4 { ch[i] = raw[i].wrapping_add(prev[i]); } }
                2 => { for i in 0..4 { ch[i] = raw[i].wrapping_add(above[i]); } }
                3 => { for i in 0..4 { ch[i] = raw[i].wrapping_add(((prev[i] as u16 + above[i] as u16) / 2) as u8); } }
                4 => {
                    let al = if x > 0 && y > 0 {
                        let p = out[(y - 1) * width + x - 1];
                        [((p >> 16) & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, (p & 0xFF) as u8, ((p >> 24) & 0xFF) as u8]
                    } else {
                        [0; 4]
                    };
                    for i in 0..4 { ch[i] = raw[i].wrapping_add(paeth(prev[i], above[i], al[i])); }
                }
                _ => { ch = raw; }
            }

            prev = ch;
            out[y * width + x] = ((ch[3] as u32) << 24) | ((ch[0] as u32) << 16) | ((ch[1] as u32) << 8) | (ch[2] as u32);
        }
    }
    ERR_OK
}

/// Grayscale reconstruction.
fn reconstruct_gray(scanlines: &[u8], width: usize, height: usize, out: &mut [u32]) -> i32 {
    let scanline_size = width + 1;

    for y in 0..height {
        let src_off = y * scanline_size;
        let filter = scanlines[src_off];
        let row_data = &scanlines[src_off + 1..src_off + 1 + width];

        let mut prev_g: u8 = 0;

        for x in 0..width {
            let raw = row_data[x];
            let above = if y > 0 { (out[(y - 1) * width + x] & 0xFF) as u8 } else { 0 };

            let g = match filter {
                0 => raw,
                1 => raw.wrapping_add(prev_g),
                2 => raw.wrapping_add(above),
                3 => raw.wrapping_add(((prev_g as u16 + above as u16) / 2) as u8),
                4 => {
                    let al = if x > 0 && y > 0 { (out[(y - 1) * width + x - 1] & 0xFF) as u8 } else { 0 };
                    raw.wrapping_add(paeth(prev_g, above, al))
                }
                _ => raw,
            };

            prev_g = g;
            let g32 = g as u32;
            out[y * width + x] = 0xFF000000 | (g32 << 16) | (g32 << 8) | g32;
        }
    }
    ERR_OK
}

/// Paeth predictor.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i32 + b as i32 - c as i32;
    let pa = (p - a as i32).abs();
    let pb = (p - b as i32).abs();
    let pc = (p - c as i32).abs();
    if pa <= pb && pa <= pc { a }
    else if pb <= pc { b }
    else { c }
}
