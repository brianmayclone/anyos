//! Minimal PNG decoder for CBDT bitmap font glyphs.
//!
//! Supports 8-bit RGBA (color type 6), 8-bit RGB (color type 2), and
//! 8-bit indexed/palette (color type 3) images with all five scanline
//! filter types. Output is always RGBA (4 bpp).

use alloc::vec;
use alloc::vec::Vec;
use super::inflate;

fn read_u32_be(data: &[u8], off: usize) -> u32 {
    (data[off] as u32) << 24 | (data[off + 1] as u32) << 16
        | (data[off + 2] as u32) << 8 | data[off + 3] as u32
}

pub struct PngImage {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data, row-major, 4 bytes per pixel.
    pub data: Vec<u8>,
}

/// Decode a PNG image from raw file data. Returns RGBA pixels.
pub fn decode_png(data: &[u8]) -> Option<PngImage> {
    if data.len() < 8 { return None; }
    // PNG signature
    if data[0] != 0x89 || &data[1..4] != b"PNG" || &data[4..8] != &[0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }

    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color_type = 0u8;
    let mut idat_data: Vec<u8> = Vec::new();
    // Palette (PLTE): up to 256 RGB entries
    let mut palette = [(0u8, 0u8, 0u8); 256];
    let mut palette_len = 0usize;
    // Transparency (tRNS): alpha values per palette entry
    let mut trns_alpha = [255u8; 256];

    while pos + 8 <= data.len() {
        let length = read_u32_be(data, pos) as usize;
        let ctype = &data[pos + 4..pos + 8];
        pos += 8;

        if pos + length > data.len() { break; }
        let chunk = &data[pos..pos + length];

        if ctype == b"IHDR" {
            if length < 13 { return None; }
            width = read_u32_be(chunk, 0);
            height = read_u32_be(chunk, 4);
            let bit_depth = chunk[8];
            color_type = chunk[9];
            if bit_depth != 8 || (color_type != 6 && color_type != 2 && color_type != 3) {
                return None;
            }
        } else if ctype == b"PLTE" {
            // Palette: sequence of RGB triplets
            let count = length / 3;
            palette_len = count.min(256);
            for i in 0..palette_len {
                palette[i] = (chunk[i * 3], chunk[i * 3 + 1], chunk[i * 3 + 2]);
            }
        } else if ctype == b"tRNS" {
            // Transparency for indexed color: one alpha byte per palette entry
            if color_type == 3 {
                let count = length.min(256);
                for i in 0..count {
                    trns_alpha[i] = chunk[i];
                }
            }
        } else if ctype == b"IDAT" {
            idat_data.extend_from_slice(chunk);
        } else if ctype == b"IEND" {
            break;
        }

        pos += length + 4; // skip chunk data + CRC
    }

    if width == 0 || height == 0 || idat_data.is_empty() { return None; }
    if color_type == 3 && palette_len == 0 { return None; }

    let raw = inflate::zlib_decompress(&idat_data)?;

    let bpp: usize = match color_type {
        6 => 4, // RGBA
        2 => 3, // RGB
        3 => 1, // Indexed (1 byte per pixel = palette index)
        _ => return None,
    };
    let row_bytes = width as usize * bpp;
    let stride = row_bytes + 1; // +1 for filter byte

    if raw.len() < height as usize * stride { return None; }

    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let mut prev_row = vec![0u8; row_bytes];

    for y in 0..height as usize {
        let row_start = y * stride;
        let filter = raw[row_start];
        let row_data = &raw[row_start + 1..row_start + 1 + row_bytes];

        // Unfilter in-place into cur_row
        let mut cur_row = vec![0u8; row_bytes];
        for i in 0..row_bytes {
            let x = row_data[i];
            let a = if i >= bpp { cur_row[i - bpp] } else { 0 };
            let b = prev_row[i];
            let c = if i >= bpp { prev_row[i - bpp] } else { 0 };

            cur_row[i] = match filter {
                0 => x,
                1 => x.wrapping_add(a),
                2 => x.wrapping_add(b),
                3 => x.wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => x.wrapping_add(paeth(a, b, c)),
                _ => x,
            };
        }

        // Write RGBA pixels
        let out_base = y * width as usize * 4;
        if color_type == 6 {
            pixels[out_base..out_base + row_bytes].copy_from_slice(&cur_row);
        } else if color_type == 2 {
            for px in 0..width as usize {
                let si = px * 3;
                let di = out_base + px * 4;
                pixels[di] = cur_row[si];
                pixels[di + 1] = cur_row[si + 1];
                pixels[di + 2] = cur_row[si + 2];
                pixels[di + 3] = 255;
            }
        } else {
            // color_type == 3: indexed
            for px in 0..width as usize {
                let idx = cur_row[px] as usize;
                let di = out_base + px * 4;
                if idx < palette_len {
                    let (r, g, b) = palette[idx];
                    pixels[di] = r;
                    pixels[di + 1] = g;
                    pixels[di + 2] = b;
                    pixels[di + 3] = trns_alpha[idx];
                } else {
                    pixels[di + 3] = 0; // transparent for out-of-range
                }
            }
        }

        prev_row = cur_row;
    }

    Some(PngImage { width, height, data: pixels })
}

/// Paeth predictor (PNG filter type 4).
#[inline]
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i32 + b as i32 - c as i32;
    let pa = (p - a as i32).abs();
    let pb = (p - b as i32).abs();
    let pc = (p - c as i32).abs();
    if pa <= pb && pa <= pc { a }
    else if pb <= pc { b }
    else { c }
}

/// Scale RGBA image using bilinear interpolation.
pub fn scale_rgba(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if dw == 0 || dh == 0 { return Vec::new(); }
    let mut dst = vec![0u8; (dw * dh * 4) as usize];

    for dy in 0..dh {
        for dx in 0..dw {
            // Fixed-point 16.16 source coordinates
            let sx_fp = (dx as u64 * (sw as u64) << 16) / dw as u64;
            let sy_fp = (dy as u64 * (sh as u64) << 16) / dh as u64;

            let sx0 = (sx_fp >> 16) as u32;
            let sy0 = (sy_fp >> 16) as u32;
            let sx1 = (sx0 + 1).min(sw - 1);
            let sy1 = (sy0 + 1).min(sh - 1);
            let fx = (sx_fp & 0xFFFF) as u64;
            let fy = (sy_fp & 0xFFFF) as u64;

            let di = (dy * dw + dx) as usize * 4;
            for c in 0..4usize {
                let p00 = src[(sy0 * sw + sx0) as usize * 4 + c] as u64;
                let p10 = src[(sy0 * sw + sx1) as usize * 4 + c] as u64;
                let p01 = src[(sy1 * sw + sx0) as usize * 4 + c] as u64;
                let p11 = src[(sy1 * sw + sx1) as usize * 4 + c] as u64;

                let top = p00 * (65536 - fx) + p10 * fx;
                let bot = p01 * (65536 - fx) + p11 * fx;
                let val = (top * (65536 - fy) + bot * fy) >> 32;
                dst[di + c] = val.min(255) as u8;
            }
        }
    }

    dst
}
