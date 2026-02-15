// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! GIF decoder (GIF87a/GIF89a, first frame only).
//!
//! Parses the GIF header, global/local color tables, Graphic Control Extension
//! (for transparency), and image descriptor. Decodes palette-indexed LZW data
//! into ARGB8888 pixels. All working memory is caller-provided (no heap).

use crate::types::*;
use crate::lzw;

fn read_u16_le(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

/// GIF header + Logical Screen Descriptor = 13 bytes.
const HEADER_SIZE: usize = 13;

/// Probe a GIF file and return image metadata.
///
/// Returns `Some(ImageInfo)` if the data starts with a valid GIF87a or GIF89a
/// header, otherwise `None`.
pub fn probe(data: &[u8]) -> Option<ImageInfo> {
    if data.len() < HEADER_SIZE {
        return None;
    }

    // Check signature: "GIF87a" or "GIF89a"
    if &data[0..3] != b"GIF" {
        return None;
    }
    if &data[3..6] != b"87a" && &data[3..6] != b"89a" {
        return None;
    }

    let width = read_u16_le(data, 6) as u32;
    let height = read_u16_le(data, 8) as u32;

    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return None;
    }

    // scratch_needed: LZW table (4096*8) + index buffer (w*h) + margin (1024)
    let scratch = lzw::LZW_SCRATCH_SIZE as u32
        + width * height
        + 1024;

    Some(ImageInfo {
        width,
        height,
        format: FMT_GIF,
        scratch_needed: scratch,
    })
}

/// Decode the first frame of a GIF file into ARGB8888 pixels.
///
/// - `data`: complete GIF file bytes
/// - `out`: output buffer for ARGB8888 pixels (must hold width*height u32s)
/// - `scratch`: working memory (size from `probe().scratch_needed`)
///
/// Returns `ERR_OK` on success, or a negative error code.
pub fn decode(data: &[u8], out: &mut [u32], scratch: &mut [u8]) -> i32 {
    if data.len() < HEADER_SIZE {
        return ERR_INVALID_DATA;
    }

    // Validate signature
    if &data[0..3] != b"GIF" {
        return ERR_INVALID_DATA;
    }
    if &data[3..6] != b"87a" && &data[3..6] != b"89a" {
        return ERR_INVALID_DATA;
    }

    // Logical Screen Descriptor
    let canvas_w = read_u16_le(data, 6) as usize;
    let canvas_h = read_u16_le(data, 8) as usize;
    let packed = data[10];
    let has_gct = (packed & 0x80) != 0;
    let gct_size_field = packed & 0x07;
    let gct_entries = if has_gct { 1usize << (gct_size_field + 1) } else { 0 };
    // data[11] = background color index, data[12] = pixel aspect ratio (ignored)

    if canvas_w == 0 || canvas_h == 0 {
        return ERR_INVALID_DATA;
    }
    if out.len() < canvas_w * canvas_h {
        return ERR_BUFFER_TOO_SMALL;
    }

    let needed = lzw::LZW_SCRATCH_SIZE + canvas_w * canvas_h + 1024;
    if scratch.len() < needed {
        return ERR_SCRATCH_TOO_SMALL;
    }

    // Global Color Table (3 bytes per entry: R, G, B)
    let gct_start = HEADER_SIZE;
    let gct_byte_size = gct_entries * 3;
    if has_gct && data.len() < gct_start + gct_byte_size {
        return ERR_INVALID_DATA;
    }

    let mut pos = gct_start + gct_byte_size;

    // Graphic Control Extension state
    let mut transparent_flag = false;
    let mut transparent_index: u8 = 0;

    // Initialize canvas to fully transparent black
    for px in out[..canvas_w * canvas_h].iter_mut() {
        *px = 0x00000000;
    }

    // Parse blocks until we find and decode the first image descriptor
    loop {
        if pos >= data.len() {
            return ERR_INVALID_DATA;
        }

        match data[pos] {
            // Extension Introducer
            0x21 => {
                if pos + 1 >= data.len() {
                    return ERR_INVALID_DATA;
                }
                let label = data[pos + 1];
                pos += 2;

                if label == 0xF9 {
                    // Graphic Control Extension
                    if pos >= data.len() {
                        return ERR_INVALID_DATA;
                    }
                    let block_size = data[pos] as usize;
                    pos += 1;
                    if block_size >= 4 && pos + block_size <= data.len() {
                        let gce_packed = data[pos];
                        transparent_flag = (gce_packed & 0x01) != 0;
                        // data[pos+1..pos+2] = delay time (ignored, first frame only)
                        transparent_index = data[pos + 3];
                    }
                    pos += block_size;
                    // Skip block terminator
                    if pos < data.len() && data[pos] == 0x00 {
                        pos += 1;
                    }
                } else {
                    // Skip unknown extension sub-blocks
                    pos = skip_sub_blocks(data, pos);
                }
            }

            // Image Descriptor
            0x2C => {
                return decode_image(
                    data, &mut pos,
                    canvas_w, canvas_h,
                    has_gct, gct_start, gct_entries,
                    transparent_flag, transparent_index,
                    out, scratch,
                );
            }

            // Trailer
            0x3B => {
                // No image found before trailer
                return ERR_INVALID_DATA;
            }

            _ => {
                // Unknown block, try to skip
                pos += 1;
            }
        }
    }
}

/// Skip a sequence of GIF sub-blocks (each prefixed by a length byte, terminated by 0x00).
fn skip_sub_blocks(data: &[u8], mut pos: usize) -> usize {
    loop {
        if pos >= data.len() {
            return pos;
        }
        let block_len = data[pos] as usize;
        pos += 1;
        if block_len == 0 {
            return pos;
        }
        pos += block_len;
    }
}

/// Decode one image descriptor (the first frame).
fn decode_image(
    data: &[u8],
    pos: &mut usize,
    canvas_w: usize,
    canvas_h: usize,
    has_gct: bool,
    gct_start: usize,
    gct_entries: usize,
    transparent_flag: bool,
    transparent_index: u8,
    out: &mut [u32],
    scratch: &mut [u8],
) -> i32 {
    // Image Descriptor: 0x2C already consumed by caller check, but pos points to 0x2C
    if *pos + 10 > data.len() {
        return ERR_INVALID_DATA;
    }

    // Skip the 0x2C separator
    *pos += 1;

    let img_left = read_u16_le(data, *pos) as usize;
    let img_top = read_u16_le(data, *pos + 2) as usize;
    let img_w = read_u16_le(data, *pos + 4) as usize;
    let img_h = read_u16_le(data, *pos + 6) as usize;
    let img_packed = data[*pos + 8];
    *pos += 9;

    let has_lct = (img_packed & 0x80) != 0;
    let interlaced = (img_packed & 0x40) != 0;
    let lct_size_field = img_packed & 0x07;
    let lct_entries = if has_lct { 1usize << (lct_size_field + 1) } else { 0 };

    if img_w == 0 || img_h == 0 {
        return ERR_INVALID_DATA;
    }

    // Bounds check: image sub-frame must fit within the canvas
    if img_left + img_w > canvas_w || img_top + img_h > canvas_h {
        return ERR_INVALID_DATA;
    }

    // Local Color Table
    let lct_start = *pos;
    let lct_byte_size = lct_entries * 3;
    if has_lct {
        if *pos + lct_byte_size > data.len() {
            return ERR_INVALID_DATA;
        }
        *pos += lct_byte_size;
    }

    // Determine which color table to use
    let (ct_data, ct_start, ct_entries) = if has_lct {
        (data, lct_start, lct_entries)
    } else if has_gct {
        (data, gct_start, gct_entries)
    } else {
        return ERR_INVALID_DATA; // No color table available
    };

    // LZW Minimum Code Size
    if *pos >= data.len() {
        return ERR_INVALID_DATA;
    }
    let min_code_size = data[*pos];
    *pos += 1;

    if min_code_size < 2 || min_code_size > 11 {
        return ERR_INVALID_DATA;
    }

    // Concatenate all sub-blocks into the scratch buffer (after LZW table area)
    let lzw_table_end = lzw::LZW_SCRATCH_SIZE;
    let index_buf_start = lzw_table_end;
    let index_buf_size = img_w * img_h;

    // Temporary area for concatenated sub-block data: after the index buffer
    let concat_start = index_buf_start + index_buf_size;

    // First pass: measure total sub-block data
    let mut total_sub = 0usize;
    let mut scan = *pos;
    loop {
        if scan >= data.len() {
            return ERR_INVALID_DATA;
        }
        let block_len = data[scan] as usize;
        scan += 1;
        if block_len == 0 {
            break;
        }
        if scan + block_len > data.len() {
            return ERR_INVALID_DATA;
        }
        total_sub += block_len;
        scan += block_len;
    }

    if concat_start + total_sub > scratch.len() {
        return ERR_SCRATCH_TOO_SMALL;
    }

    // Second pass: copy sub-block data
    let mut concat_off = 0usize;
    loop {
        if *pos >= data.len() {
            return ERR_INVALID_DATA;
        }
        let block_len = data[*pos] as usize;
        *pos += 1;
        if block_len == 0 {
            break;
        }
        if *pos + block_len > data.len() {
            return ERR_INVALID_DATA;
        }
        let dst = &mut scratch[concat_start + concat_off..concat_start + concat_off + block_len];
        dst.copy_from_slice(&data[*pos..*pos + block_len]);
        concat_off += block_len;
        *pos += block_len;
    }

    // Split scratch into non-overlapping regions using raw pointers
    // Region layout: [0..LZW_SCRATCH_SIZE) = LZW table
    //                [index_buf_start..index_buf_start+index_buf_size) = index output
    //                [concat_start..concat_start+total_sub) = compressed data
    let scratch_ptr = scratch.as_mut_ptr();

    let lzw_scratch = unsafe {
        core::slice::from_raw_parts_mut(scratch_ptr, lzw::LZW_SCRATCH_SIZE)
    };
    let index_buf = unsafe {
        core::slice::from_raw_parts_mut(scratch_ptr.add(index_buf_start), index_buf_size)
    };
    let compressed = unsafe {
        core::slice::from_raw_parts(scratch_ptr.add(concat_start) as *const u8, total_sub)
    };

    // Decompress LZW
    let decoded = lzw::decompress(compressed, index_buf, min_code_size, lzw_scratch);
    if decoded < 0 {
        return decoded; // Propagate error
    }
    let decoded = decoded as usize;

    // Convert palette indices to ARGB8888 pixels
    // Handle interlaced images by mapping logical row to physical row
    let interlace_passes: [(usize, usize); 4] = [
        (0, 8), // Pass 1: every 8th row, starting at 0
        (4, 8), // Pass 2: every 8th row, starting at 4
        (2, 4), // Pass 3: every 4th row, starting at 2
        (1, 2), // Pass 4: every 2nd row, starting at 1
    ];

    let mut src_idx: usize = 0;

    if interlaced {
        for &(start, step) in &interlace_passes {
            let mut y = start;
            while y < img_h {
                for x in 0..img_w {
                    if src_idx >= decoded {
                        break;
                    }
                    let color_idx = index_buf[src_idx] as usize;
                    src_idx += 1;

                    let pixel = palette_to_argb(
                        ct_data, ct_start, ct_entries,
                        color_idx, transparent_flag, transparent_index,
                    );
                    out[(img_top + y) * canvas_w + (img_left + x)] = pixel;
                }
                y += step;
            }
        }
    } else {
        for y in 0..img_h {
            for x in 0..img_w {
                if src_idx >= decoded {
                    break;
                }
                let color_idx = index_buf[src_idx] as usize;
                src_idx += 1;

                let pixel = palette_to_argb(
                    ct_data, ct_start, ct_entries,
                    color_idx, transparent_flag, transparent_index,
                );
                out[(img_top + y) * canvas_w + (img_left + x)] = pixel;
            }
        }
    }

    ERR_OK
}

/// Look up a palette index and return an ARGB8888 pixel value.
#[inline]
fn palette_to_argb(
    ct_data: &[u8],
    ct_start: usize,
    ct_entries: usize,
    index: usize,
    transparent_flag: bool,
    transparent_index: u8,
) -> u32 {
    if transparent_flag && index == transparent_index as usize {
        return 0x00000000;
    }

    if index >= ct_entries {
        return 0xFF000000; // Out-of-range index: opaque black
    }

    let off = ct_start + index * 3;
    let r = ct_data[off] as u32;
    let g = ct_data[off + 1] as u32;
    let b = ct_data[off + 2] as u32;

    0xFF000000 | (r << 16) | (g << 8) | b
}
