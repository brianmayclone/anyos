// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! PNG decoder (non-interlaced, 8-bit RGB/RGBA/Grayscale).

use crate::types::*;
use crate::deflate;

const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn read_u32_be(data: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

/// Color types we support.
const CT_GRAY: u8 = 0;
const CT_RGB: u8 = 2;
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
    if bit_depth != 8 {
        return None; // Only 8-bit supported
    }
    let pixel_bytes = bpp(color_type);
    if pixel_bytes == 0 {
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
    let decompressed_size = (width as usize * pixel_bytes + 1) * height as usize;
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
    let color_type = data[25];
    let pixel_bytes = bpp(color_type);

    if out.len() < width * height {
        return ERR_BUFFER_TOO_SMALL;
    }

    // Collect all IDAT chunks
    let mut idat_total = 0usize;
    let mut pos = 8;

    // First pass: count IDAT data size
    while pos + 12 <= data.len() {
        let chunk_len = read_u32_be(data, pos) as usize;
        let chunk_type = &data[pos + 4..pos + 8];
        if chunk_type == b"IDAT" {
            idat_total += chunk_len;
        }
        pos += 12 + chunk_len; // length + type + data + crc
    }

    if idat_total == 0 {
        return ERR_INVALID_DATA;
    }

    // We need scratch for: deflate window (32K) + decompressed data + row buffer
    let scanline_size = width * pixel_bytes + 1; // +1 for filter byte
    let decompressed_size = scanline_size * height;
    let needed = 32768 + decompressed_size + width * pixel_bytes;
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

    // Reconstruct filtered scanlines
    let decomp = unsafe {
        core::slice::from_raw_parts(scratch_ptr.add(decomp_area_start) as *const u8, decompressed_len)
    };
    reconstruct_and_convert(decomp, width, height, pixel_bytes, color_type, out)
}

/// Apply PNG filter reconstruction and convert to ARGB8888.
fn reconstruct_and_convert(
    scanlines: &[u8],
    width: usize,
    height: usize,
    pixel_bytes: usize,
    color_type: u8,
    out: &mut [u32],
) -> i32 {
    let row_bytes = width * pixel_bytes;
    let scanline_size = row_bytes + 1; // filter byte + pixel data

    // We process row-by-row, keeping previous row in a buffer on the stack.
    // For max width 16384 * 4 bpp = 64K, which exceeds stack limits.
    // Instead we do two passes: first reconstruct in-place, then convert.
    // But scanlines is immutable... We'll work directly with output.

    // Since we can't modify scanlines and have no heap, do filtering + conversion
    // in one pass. Keep prev_row as the first few bytes of the output (reinterpreted).
    // Actually, we'll just process carefully using the output buffer as working storage.

    // Use out[] as temp storage: each row's filtered bytes go into out[y*width..] reinterpreted.
    // This works because out has width*height u32s = 4*width*height bytes, and we need
    // pixel_bytes*width*height bytes for reconstruction.

    // Process each row, referencing previous row's output pixels.
    for y in 0..height {
        let src_off = y * scanline_size;
        let filter = scanlines[src_off];
        let row_data = &scanlines[src_off + 1..src_off + 1 + row_bytes];

        for x in 0..width {
            let px_off = x * pixel_bytes;

            // Get raw bytes for this pixel
            let mut channels = [0u8; 4];
            for c in 0..pixel_bytes {
                let raw = row_data[px_off + c];

                // a = pixel to the left (same row, already reconstructed)
                let a = if x > 0 {
                    // Extract from already-written output pixel
                    get_channel_from_argb(out[y * width + x - 1], color_type, c)
                } else {
                    0
                };

                // b = pixel above (previous row)
                let b = if y > 0 {
                    get_channel_from_argb(out[(y - 1) * width + x], color_type, c)
                } else {
                    0
                };

                // c = pixel above-left
                let c_val = if x > 0 && y > 0 {
                    get_channel_from_argb(out[(y - 1) * width + x - 1], color_type, c)
                } else {
                    0
                };

                channels[c] = match filter {
                    0 => raw,                                   // None
                    1 => raw.wrapping_add(a),                   // Sub
                    2 => raw.wrapping_add(b),                   // Up
                    3 => raw.wrapping_add(((a as u16 + b as u16) / 2) as u8), // Average
                    4 => raw.wrapping_add(paeth(a, b, c_val)),  // Paeth
                    _ => raw,
                };
            }

            // Convert to ARGB8888
            out[y * width + x] = match color_type {
                CT_GRAY => {
                    let g = channels[0] as u32;
                    0xFF000000 | (g << 16) | (g << 8) | g
                }
                CT_RGB => {
                    0xFF000000 | ((channels[0] as u32) << 16) | ((channels[1] as u32) << 8) | (channels[2] as u32)
                }
                CT_RGBA => {
                    ((channels[3] as u32) << 24) | ((channels[0] as u32) << 16) | ((channels[1] as u32) << 8) | (channels[2] as u32)
                }
                _ => 0,
            };
        }
    }

    ERR_OK
}

/// Extract a raw channel value from an ARGB8888 pixel (for filter reconstruction).
fn get_channel_from_argb(argb: u32, color_type: u8, channel: usize) -> u8 {
    match color_type {
        CT_GRAY => (argb & 0xFF) as u8,
        CT_RGB => match channel {
            0 => ((argb >> 16) & 0xFF) as u8, // R
            1 => ((argb >> 8) & 0xFF) as u8,  // G
            2 => (argb & 0xFF) as u8,         // B
            _ => 0,
        },
        CT_RGBA => match channel {
            0 => ((argb >> 16) & 0xFF) as u8, // R
            1 => ((argb >> 8) & 0xFF) as u8,  // G
            2 => (argb & 0xFF) as u8,         // B
            3 => ((argb >> 24) & 0xFF) as u8, // A
            _ => 0,
        },
        _ => 0,
    }
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
