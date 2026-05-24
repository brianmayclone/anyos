// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! WebP image decoder — VP8L (lossless) only.
//!
//! Supports:
//!   - VP8L bitstream (lossless WebP)
//!   - SUBTRACT_GREEN transform
//!   - PREDICTOR_TRANSFORM (all 14 predictor modes)
//!   - COLOR_INDEXING_TRANSFORM (palette images)
//!   - COLOR_TRANSFORM
//!   - LZ77 backward references
//!   - Color cache
//!   - Single and multi-group Huffman meta-images
//!
//! VP8 (lossy) and VP8X animated / ICC / EXIF extensions are detected but
//! return `ERR_UNSUPPORTED` — the probe still returns valid dimensions.
//!
//! Reference: <https://developers.google.com/speed/webp/docs/webp_lossless_bitstream_specification>

#![allow(dead_code)]

use crate::types::*;
use alloc::vec::Vec;
use alloc::vec;

// ── RIFF / WebP container ─────────────────────────────────────────────────

pub fn probe(data: &[u8]) -> Option<ImageInfo> {
    if data.len() < 12 { return None; }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" { return None; }
    let (w, h) = container_dimensions(data)?;
    Some(ImageInfo { width: w, height: h, format: FMT_WEBP, scratch_needed: 0 })
}

pub fn decode(data: &[u8], out: &mut [u32], _scratch: &mut [u8]) -> i32 {
    if data.len() < 12 { return ERR_INVALID_DATA; }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" { return ERR_INVALID_DATA; }
    let (_width, _height) = match container_dimensions(data) {
        Some(dims) => dims,
        None => return ERR_INVALID_DATA,
    };

    // Walk RIFF chunks — collect VP8/VP8L/VP8X/ALPH
    let mut pos = 12usize;
    let mut vp8_chunk: Option<&[u8]> = None;
    let mut vp8l_chunk: Option<&[u8]> = None;
    let mut _alph_chunk: Option<&[u8]> = None;
    let mut is_animated = false;

    while pos + 8 <= data.len() {
        let tag = &data[pos..pos + 4];
        let size = u32::from_le_bytes([data[pos+4], data[pos+5], data[pos+6], data[pos+7]]) as usize;
        pos += 8;
        let chunk = if pos + size <= data.len() { &data[pos..pos + size] } else { &data[pos..] };

        match tag {
            b"VP8L" => { vp8l_chunk = Some(chunk); }
            b"VP8 " => { vp8_chunk = Some(chunk); }
            b"VP8X" => {}
            b"ALPH" => { _alph_chunk = Some(chunk); }
            b"ANIM" | b"ANMF" => { is_animated = true; }
            _ => {}
        }

        pos += (size + 1) & !1; // RIFF pads chunks to even size
    }

    if is_animated {
        return ERR_UNSUPPORTED;
    }

    // VP8L (lossless) has priority — it encodes its own alpha
    if let Some(chunk) = vp8l_chunk {
        return decode_vp8l(chunk, out);
    }

    // VP8 lossy needs a fully conforming entropy/residual path. Returning
    // success with visibly corrupted pixels is worse than negotiating JPEG/PNG
    // fallbacks, so keep lossy WebP disabled until that decoder is complete.
    if vp8_chunk.is_some() {
        return ERR_UNSUPPORTED;
    }

    ERR_UNSUPPORTED
}

/// Extract dimensions from the RIFF container without full decode.
fn container_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let tag = &data[pos..pos + 4];
        let size = u32::from_le_bytes([data[pos+4], data[pos+5], data[pos+6], data[pos+7]]) as usize;
        pos += 8;

        if tag == b"VP8L" && pos < data.len() && data[pos] == 0x2F {
            if pos + 4 >= data.len() { return None; }
            let v = (data[pos+1] as u32)
                | ((data[pos+2] as u32) << 8)
                | ((data[pos+3] as u32) << 16)
                | ((data[pos+4] as u32) << 24);
            let w = (v & 0x3FFF) + 1;
            let h = ((v >> 14) & 0x3FFF) + 1;
            return Some((w, h));
        }

        if tag == b"VP8 " && size >= 10 {
            // VP8 lossy: 3 bytes frame tag, then starts at byte 3
            // Keyframe: bytes 3..5 = 0x9D012A, then 2 bytes width LE, 2 bytes height LE
            let d = &data[pos..];
            if d.len() >= 10 && d[3] == 0x9D && d[4] == 0x01 && d[5] == 0x2A {
                let w = u16::from_le_bytes([d[6], d[7]]) as u32 & 0x3FFF;
                let h = u16::from_le_bytes([d[8], d[9]]) as u32 & 0x3FFF;
                return Some((w, h));
            }
        }

        if tag == b"VP8X" && pos + 10 <= data.len() {
            let w = u32::from_le_bytes([data[pos+4], data[pos+5], data[pos+6], 0]) + 1;
            let h = u32::from_le_bytes([data[pos+7], data[pos+8], data[pos+9], 0]) + 1;
            return Some((w, h));
        }

        pos += (size + 1) & !1;
    }
    None
}

// ── Bit reader ────────────────────────────────────────────────────────────

struct BitReader<'a> {
    data: &'a [u8],
    pos:  usize,
    buf:  u64,
    avail: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut r = BitReader { data, pos: 0, buf: 0, avail: 0 };
        r.fill();
        r
    }

    #[inline]
    fn fill(&mut self) {
        while self.avail <= 56 && self.pos < self.data.len() {
            self.buf |= (self.data[self.pos] as u64) << self.avail;
            self.pos += 1;
            self.avail += 8;
        }
    }

    #[inline]
    fn peek(&self, n: u32) -> u32 {
        debug_assert!(n <= 32 && n <= self.avail + (self.data.len() - self.pos) as u32 * 8);
        (self.buf & ((1u64 << n).wrapping_sub(1))) as u32
    }

    #[inline]
    fn advance(&mut self, n: u32) {
        self.buf >>= n;
        self.avail = self.avail.saturating_sub(n);
        if self.avail < 32 { self.fill(); }
    }

    #[inline]
    fn read(&mut self, n: u32) -> u32 {
        if n == 0 { return 0; }
        let v = self.peek(n);
        self.advance(n);
        v
    }

    #[inline]
    fn read_bit(&mut self) -> bool { self.read(1) != 0 }
}

// ── Huffman tree ──────────────────────────────────────────────────────────

const FAST_BITS: u32 = 9;
const FAST_SIZE: usize = 1 << FAST_BITS;

/// Huffman tree with a 9-bit fast lookup table and a slow-path list for longer
/// codes.  VP8L uses LSB-first bit order so all canonical codes are stored in
/// reversed (LSB-first) form to allow direct table indexing.
struct HuffTree {
    /// fast[index] = (symbol, code_length). code_length==0 → unused slot.
    fast: Vec<(u16, u8)>,
    /// Slow path: (reversed_canonical_code, code_length, symbol).
    slow: Vec<(u32, u32, u16)>,
}

impl HuffTree {
    fn new() -> Self {
        HuffTree { fast: vec![(0, 0); FAST_SIZE], slow: Vec::new() }
    }

    /// Build from `lengths[symbol] = code_length` (0 = symbol absent).
    fn build(&mut self, lengths: &[u8]) {
        for e in self.fast.iter_mut() { *e = (0, 0); }
        self.slow.clear();

        let max_len = lengths.iter().copied().max().unwrap_or(0) as u32;
        if max_len == 0 { return; }

        // Count symbols per code length.
        let mut cnt = [0u32; 16];
        for &l in lengths { if l > 0 { cnt[l as usize] += 1; } }

        // Starting canonical code for each length.
        let mut next = [0u32; 16];
        let mut code = 0u32;
        for l in 1..=(max_len as usize) {
            code = (code + cnt[l - 1]) << 1;
            next[l] = code;
        }

        // Assign codes and fill lookup tables.
        for (sym, &l) in lengths.iter().enumerate() {
            if l == 0 { continue; }
            let l = l as u32;
            let c = next[l as usize];
            next[l as usize] += 1;
            // Reverse bits for LSB-first decoding.
            let rev = reverse_bits(c, l);

            if l <= FAST_BITS {
                // Fill all 9-bit patterns whose lower l bits match rev.
                let fill = 1usize << (FAST_BITS - l);
                for i in 0..fill {
                    self.fast[rev as usize | (i << l)] = (sym as u16, l as u8);
                }
            } else {
                self.slow.push((rev, l, sym as u16));
            }
        }
    }

    #[inline]
    fn decode(&self, br: &mut BitReader) -> u16 {
        let (sym, len) = self.fast[br.peek(FAST_BITS) as usize];
        if len > 0 {
            br.advance(len as u32);
            return sym;
        }
        // Slow path for codes > 9 bits.
        for &(rev, l, sym) in &self.slow {
            if br.avail >= l && br.peek(l) == rev {
                br.advance(l);
                return sym;
            }
        }
        0 // error fallback
    }
}

#[inline]
fn reverse_bits(val: u32, len: u32) -> u32 {
    let mut r = 0u32;
    let mut v = val;
    for _ in 0..len { r = (r << 1) | (v & 1); v >>= 1; }
    r
}

// ── Huffman tree reading ──────────────────────────────────────────────────

/// Code-length ordering (from the VP8L spec).
const CODE_LENGTH_ORDER: [usize; 19] =
    [17, 18, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

fn read_huffman_tree(br: &mut BitReader, alphabet_size: usize) -> Option<HuffTree> {
    let mut tree = HuffTree::new();
    let is_simple = br.read_bit();

    if is_simple {
        let num_syms = br.read(1) as usize + 1; // 1 or 2
        let use_8bit = br.read_bit();
        let sym1 = if use_8bit { br.read(8) } else { br.read(1) } as usize;
        if sym1 >= alphabet_size { return None; }

        let mut lengths = vec![0u8; alphabet_size];
        if num_syms == 1 {
            // Single symbol: all bit patterns map to it.
            lengths[sym1] = 1;
            tree.build(&lengths);
            // Override table so every 9-bit pattern resolves immediately.
            for e in tree.fast.iter_mut() { *e = (sym1 as u16, 1); }
        } else {
            let sym2 = br.read(8) as usize;
            if sym2 >= alphabet_size { return None; }
            lengths[sym1] = 1;
            lengths[sym2] = 1;
            tree.build(&lengths);
        }
    } else {
        // Normal code: first decode a code-length code, then use it.
        let num_cl = 4 + br.read(4) as usize; // 4..19
        let mut cl_len = [0u8; 19];
        for i in 0..num_cl.min(19) {
            cl_len[CODE_LENGTH_ORDER[i]] = br.read(3) as u8;
        }
        let mut cl_tree = HuffTree::new();
        cl_tree.build(&cl_len);

        // Decode the actual code lengths.
        let mut lengths = vec![0u8; alphabet_size];
        let mut i = 0usize;
        while i < alphabet_size {
            let sym = cl_tree.decode(br) as usize;
            match sym {
                0..=15 => { lengths[i] = sym as u8; i += 1; }
                16 => {
                    let rep = 3 + br.read(2) as usize;
                    let prev = if i > 0 { lengths[i-1] } else { 0 };
                    for _ in 0..rep { if i < alphabet_size { lengths[i] = prev; i += 1; } }
                }
                17 => { let rep = 3 + br.read(3) as usize; i = (i + rep).min(alphabet_size); }
                18 => { let rep = 11 + br.read(7) as usize; i = (i + rep).min(alphabet_size); }
                _ => return None,
            }
        }
        tree.build(&lengths);
    }
    Some(tree)
}

// ── Huffman group (5 trees) ───────────────────────────────────────────────

struct HuffGroup {
    green: HuffTree,  // G literals + length codes + color-cache codes
    red:   HuffTree,
    blue:  HuffTree,
    alpha: HuffTree,
    dist:  HuffTree,
}

impl HuffGroup {
    fn read(br: &mut BitReader, cache_size: u32) -> Option<Self> {
        let green_size = (256 + 24 + cache_size) as usize;
        Some(HuffGroup {
            green: read_huffman_tree(br, green_size)?,
            red:   read_huffman_tree(br, 256)?,
            blue:  read_huffman_tree(br, 256)?,
            alpha: read_huffman_tree(br, 256)?,
            dist:  read_huffman_tree(br, 40)?,
        })
    }
}

// ── Length / distance decoding ────────────────────────────────────────────

#[inline]
fn decode_length(prefix: u32, br: &mut BitReader) -> u32 {
    if prefix < 4 { return prefix + 2; }
    let extra = (prefix - 2) >> 1;
    let base  = (2 + (prefix & 1)) << extra;
    base + br.read(extra) + 2
}

#[inline]
fn decode_distance(prefix: u32, br: &mut BitReader) -> u32 {
    if prefix < 4 { return prefix + 1; }
    let extra = (prefix - 2) >> 1;
    let base  = (2 + (prefix & 1)) << extra;
    base + br.read(extra) + 1
}

// ── Color cache ───────────────────────────────────────────────────────────

struct ColorCache {
    data: Vec<u32>,
    bits: u32,
}

impl ColorCache {
    fn new(bits: u32) -> Self {
        ColorCache { data: vec![0u32; 1 << bits], bits }
    }
    #[inline]
    fn insert(&mut self, argb: u32) {
        let key = (argb.wrapping_mul(0x1E35A7BD) >> (32 - self.bits)) as usize;
        self.data[key] = argb;
    }
    #[inline]
    fn lookup(&self, idx: usize) -> u32 { self.data[idx] }
}

// ── VP8L pixel decoder ────────────────────────────────────────────────────

/// Decode `w×h` pixels using the provided Huffman group(s) and write to `out`.
/// `group_map` maps pixel index → group index (empty = single group at index 0).
fn decode_pixels(
    br: &mut BitReader,
    w: usize, h: usize,
    groups: &[HuffGroup],
    group_map: &[u32],
    meta_block_bits: u32,
    cache: &mut Option<ColorCache>,
    out: &mut [u32],
) -> bool {
    let total = w * h;
    let mut i = 0usize;

    while i < total {
        // Determine which Huffman group to use.
        let grp = if group_map.is_empty() {
            &groups[0]
        } else {
            // meta-image maps 2D block position to group index.
            let x = i % w;
            let y = i / w;
            let bx = x >> meta_block_bits;
            let meta_w = (w + (1 << meta_block_bits) - 1) >> meta_block_bits;
            let gidx = group_map[y * meta_w + bx] as usize;
            &groups[gidx.min(groups.len() - 1)]
        };

        let green_sym = grp.green.decode(br) as u32;

        if green_sym < 256 {
            // Literal ARGB pixel.
            let r = grp.red.decode(br) as u32;
            let g = green_sym;
            let b = grp.blue.decode(br) as u32;
            let a = grp.alpha.decode(br) as u32;
            let argb = (a << 24) | (r << 16) | (g << 8) | b;
            out[i] = argb;
            if let Some(c) = cache { c.insert(argb); }
            i += 1;
        } else if green_sym < 256 + 24 {
            // Back reference.
            let len_prefix = green_sym - 256;
            let length = decode_length(len_prefix, br) as usize;
            let dist_prefix = grp.dist.decode(br) as u32;
            let dist = decode_distance(dist_prefix, br) as usize;

            if dist > i { return false; } // corrupt
            let src_start = i.saturating_sub(dist);
            let src_end   = src_start + length;
            if src_end > total { return false; }

            // Copy — forward copy, may overlap (intentional LZ77 behaviour).
            for k in 0..length {
                let pixel = out[src_start + k];
                out[i + k] = pixel;
                if let Some(c) = cache { c.insert(pixel); }
            }
            i += length;
        } else {
            // Color cache lookup.
            let cache_idx = (green_sym - 256 - 24) as usize;
            let argb = match cache {
                Some(c) => c.lookup(cache_idx),
                None    => return false,
            };
            out[i] = argb;
            i += 1;
        }
    }
    true
}

// ── Transform data ────────────────────────────────────────────────────────

struct Transform {
    kind: u8,
    /// For PREDICTOR and COLOR: block size is `1 << block_bits` pixels.
    block_bits: u32,
    /// Decoded transform image (w_blocks × h_blocks pixels).
    data: Vec<u32>,
    /// For COLOR_INDEXING: the palette.
    palette: Vec<u32>,
    /// For COLOR_INDEXING: bits per pixel (1, 2, 4, or 8).
    bits_per_pixel: u32,
}

const PREDICTOR:     u8 = 0;
const COLOR_XFORM:   u8 = 1;
const SUB_GREEN:     u8 = 2;
const COLOR_INDEXING: u8 = 3;

// ── VP8L top-level decode ─────────────────────────────────────────────────

fn decode_vp8l(chunk: &[u8], out: &mut [u32]) -> i32 {
    if chunk.is_empty() || chunk[0] != 0x2F { return ERR_INVALID_DATA; }
    let mut br = BitReader::new(&chunk[1..]);

    // Header: width-1 (14), height-1 (14), alpha_hint (1), version (3).
    let hdr   = br.read(28);
    let width  = ((hdr & 0x3FFF) + 1) as usize;
    let height = (((hdr >> 14) & 0x3FFF) + 1) as usize;
    let version = (hdr >> 29) & 7;
    if version != 0 { return ERR_INVALID_DATA; }
    if width * height > out.len() { return ERR_BUFFER_TOO_SMALL; }

    // Read transforms (up to 4, innermost first in the stream).
    let mut transforms: Vec<Transform> = Vec::new();
    let mut actual_w = width; // may shrink for COLOR_INDEXING

    while br.read_bit() {
        let kind = br.read(2) as u8;
        match kind {
            SUB_GREEN => {
                transforms.push(Transform {
                    kind, block_bits: 0, data: Vec::new(),
                    palette: Vec::new(), bits_per_pixel: 0,
                });
            }
            PREDICTOR | COLOR_XFORM => {
                let block_bits = br.read(3) + 2; // 2..9
                let bw = (actual_w + (1 << block_bits) - 1) >> block_bits;
                let bh = (height  + (1 << block_bits) - 1) >> block_bits;
                let mut tdata = vec![0u32; bw * bh];
                // Recursively decode the transform image.
                if !decode_vp8l_image(&mut br, bw, bh, &mut tdata) {
                    return ERR_INVALID_DATA;
                }
                transforms.push(Transform {
                    kind, block_bits, data: tdata,
                    palette: Vec::new(), bits_per_pixel: 0,
                });
            }
            COLOR_INDEXING => {
                let palette_size = br.read(8) as usize + 1; // 1..256
                let mut pal = vec![0u32; palette_size];
                // Palette is a 1×palette_size VP8L image (delta-coded in G channel).
                if !decode_vp8l_image(&mut br, palette_size, 1, &mut pal) {
                    return ERR_INVALID_DATA;
                }
                // Undo delta coding (successive XOR).
                for i in 1..palette_size {
                    pal[i] = add_argb(pal[i-1], pal[i]);
                }
                let bpp: u32 = if palette_size <= 2 { 1 }
                    else if palette_size <= 4 { 2 }
                    else if palette_size <= 16 { 4 }
                    else { 8 };
                // When bpp < 8, multiple pixels are packed into one "code pixel".
                let ppu = (8 / bpp) as usize;
                let new_w = if bpp < 8 {
                    (actual_w + ppu - 1) / ppu
                } else { actual_w };
                transforms.push(Transform {
                    kind, block_bits: 0, data: Vec::new(),
                    palette: pal, bits_per_pixel: bpp,
                });
                actual_w = new_w;
            }
            _ => return ERR_INVALID_DATA,
        }
    }

    // Decode the main image into a temporary buffer (always).
    let total_actual = actual_w * height;
    let mut tmp_buf = vec![0u32; total_actual];

    if !decode_vp8l_image(&mut br, actual_w, height, &mut tmp_buf) {
        return ERR_INVALID_DATA;
    }

    // Apply transforms in reverse order (last applied = first to undo).
    for t in transforms.iter().rev() {
        match t.kind {
            SUB_GREEN => apply_subtract_green(&mut tmp_buf),
            COLOR_XFORM => apply_color_transform(&mut tmp_buf, actual_w, height, t),
            PREDICTOR => apply_predictor(&mut tmp_buf, actual_w, height, t),
            COLOR_INDEXING => {
                // Expand packed pixels into full-width output.
                apply_color_indexing(&tmp_buf, actual_w, out, width, height, t);
                return ERR_OK; // already written to out
            }
            _ => {}
        }
    }

    // Copy decoded pixels to output.
    let copy_len = (width * height).min(tmp_buf.len());
    out[..copy_len].copy_from_slice(&tmp_buf[..copy_len]);
    ERR_OK
}

fn decode_vp8l_image_stream(data: &[u8], width: usize, height: usize, out: &mut [u32]) -> i32 {
    if width == 0 || height == 0 {
        return ERR_INVALID_DATA;
    }
    if width > 16384 || height > 16384 {
        return ERR_INVALID_DATA;
    }
    if width.saturating_mul(height) > out.len() {
        return ERR_BUFFER_TOO_SMALL;
    }

    let mut br = BitReader::new(data);

    // Read transforms (up to 4, innermost first in the stream).
    let mut transforms: Vec<Transform> = Vec::new();
    let mut actual_w = width; // may shrink for COLOR_INDEXING

    while br.read_bit() {
        let kind = br.read(2) as u8;
        match kind {
            SUB_GREEN => {
                transforms.push(Transform {
                    kind, block_bits: 0, data: Vec::new(),
                    palette: Vec::new(), bits_per_pixel: 0,
                });
            }
            PREDICTOR | COLOR_XFORM => {
                let block_bits = br.read(3) + 2; // 2..9
                let bw = (actual_w + (1 << block_bits) - 1) >> block_bits;
                let bh = (height  + (1 << block_bits) - 1) >> block_bits;
                let mut tdata = vec![0u32; bw * bh];
                if !decode_vp8l_image(&mut br, bw, bh, &mut tdata) {
                    return ERR_INVALID_DATA;
                }
                transforms.push(Transform {
                    kind, block_bits, data: tdata,
                    palette: Vec::new(), bits_per_pixel: 0,
                });
            }
            COLOR_INDEXING => {
                let palette_size = br.read(8) as usize + 1; // 1..256
                let mut pal = vec![0u32; palette_size];
                if !decode_vp8l_image(&mut br, palette_size, 1, &mut pal) {
                    return ERR_INVALID_DATA;
                }
                for i in 1..palette_size {
                    pal[i] = add_argb(pal[i - 1], pal[i]);
                }
                let bpp: u32 = if palette_size <= 2 { 1 }
                    else if palette_size <= 4 { 2 }
                    else if palette_size <= 16 { 4 }
                    else { 8 };
                let ppu = (8 / bpp) as usize;
                let new_w = if bpp < 8 {
                    (actual_w + ppu - 1) / ppu
                } else { actual_w };
                transforms.push(Transform {
                    kind, block_bits: 0, data: Vec::new(),
                    palette: pal, bits_per_pixel: bpp,
                });
                actual_w = new_w;
            }
            _ => return ERR_INVALID_DATA,
        }
    }

    let total_actual = actual_w * height;
    let mut tmp_buf = vec![0u32; total_actual];

    if !decode_vp8l_image(&mut br, actual_w, height, &mut tmp_buf) {
        return ERR_INVALID_DATA;
    }

    for t in transforms.iter().rev() {
        match t.kind {
            SUB_GREEN => apply_subtract_green(&mut tmp_buf),
            COLOR_XFORM => apply_color_transform(&mut tmp_buf, actual_w, height, t),
            PREDICTOR => apply_predictor(&mut tmp_buf, actual_w, height, t),
            COLOR_INDEXING => {
                apply_color_indexing(&tmp_buf, actual_w, out, width, height, t);
                return ERR_OK;
            }
            _ => {}
        }
    }

    let copy_len = width * height;
    out[..copy_len].copy_from_slice(&tmp_buf[..copy_len]);
    ERR_OK
}

/// Recursively decode a VP8L image (used for transform sub-images).
/// No nested transforms allowed in sub-images.
fn decode_vp8l_image(br: &mut BitReader, w: usize, h: usize, out: &mut [u32]) -> bool {
    // Color cache.
    let use_cache = br.read_bit();
    let cache_bits = if use_cache { br.read(4) } else { 0 };
    let cache_size = if use_cache { 1 << cache_bits } else { 0u32 };
    let mut cache = if use_cache { Some(ColorCache::new(cache_bits)) } else { None };

    // Meta-image (Huffman group map).
    let use_meta = br.read_bit();
    let (meta_block_bits, groups, group_map) = if use_meta {
        let mbb = br.read(3) + 2; // 2..9
        let mw = (w + (1 << mbb) - 1) >> mbb;
        let mh = (h + (1 << mbb) - 1) >> mbb;
        let mut meta_pix = vec![0u32; mw * mh];
        // Decode meta-image with NO meta (recursive call depth is bounded).
        let mut no_cache: Option<ColorCache> = None;
        let mut no_map: Vec<u32> = Vec::new();
        let meta_group = match HuffGroup::read(br, 0) { Some(g) => g, None => return false };
        let mut groups_inner = vec![meta_group];
        if !decode_pixels(br, mw, mh, &groups_inner, &no_map, 0, &mut no_cache, &mut meta_pix) {
            return false;
        }
        // The number of groups = 1 + max(G channel of meta pixels) >> 8.
        let num_groups = meta_pix.iter()
            .map(|p| ((p >> 8) & 0xFFFF) as usize + 1)
            .max().unwrap_or(1);
        // Re-read the actual groups (the meta-group count includes the map group).
        // In VP8L, the meta-image pixels encode group indices in the RG channels;
        // we already decoded the first group above for the meta-image itself.
        // Now read (num_groups) more groups for the actual image.
        // Note: the first group read was the meta-image's group (group 0 of the meta).
        // The image groups follow in the bitstream.
        groups_inner.clear();
        for _ in 0..num_groups {
            match HuffGroup::read(br, cache_size) {
                Some(g) => groups_inner.push(g),
                None => return false,
            }
        }
        // Build group map: pixel G-channel bits 8..23 give the group index.
        let map: Vec<u32> = meta_pix.iter().map(|p| (p >> 8) & 0xFFFF).collect();
        (mbb, groups_inner, map)
    } else {
        let group = match HuffGroup::read(br, cache_size) { Some(g) => g, None => return false };
        (0u32, vec![group], Vec::new())
    };

    decode_pixels(br, w, h, &groups, &group_map, meta_block_bits, &mut cache, out)
}

// ── Transform application ─────────────────────────────────────────────────

#[inline]
fn add_argb(a: u32, b: u32) -> u32 {
    // Per-channel byte addition with wrapping (no carry between channels).
    let aa = ((a >> 24) & 0xFF).wrapping_add((b >> 24) & 0xFF) & 0xFF;
    let rr = ((a >> 16) & 0xFF).wrapping_add((b >> 16) & 0xFF) & 0xFF;
    let gg = ((a >>  8) & 0xFF).wrapping_add((b >>  8) & 0xFF) & 0xFF;
    let bb = ( a        & 0xFF).wrapping_add( b        & 0xFF) & 0xFF;
    (aa << 24) | (rr << 16) | (gg << 8) | bb
}

fn apply_subtract_green(pixels: &mut [u32]) {
    for px in pixels.iter_mut() {
        let a = (*px >> 24) & 0xFF;
        let r = (*px >> 16) & 0xFF;
        let g = (*px >>  8) & 0xFF;
        let b =  *px        & 0xFF;
        *px = (a << 24) | ((r.wrapping_add(g) & 0xFF) << 16) | (g << 8) | (b.wrapping_add(g) & 0xFF);
    }
}

fn apply_color_transform(pixels: &mut [u32], w: usize, h: usize, t: &Transform) {
    let bs = 1usize << t.block_bits;
    let bw = (w + bs - 1) / bs;
    for y in 0..h {
        for x in 0..w {
            let bx = x / bs;
            let by = y / bs;
            let te = t.data[by * bw + bx];
            let green_to_red   = (te >>  8) as i8;
            let green_to_blue  = (te >> 16) as i8;
            let red_to_blue    = (te >> 24) as i8;
            let px = pixels[y * w + x];
            let a = (px >> 24) & 0xFF;
            let r = ((px >> 16) & 0xFF) as i32;
            let g = ((px >>  8) & 0xFF) as i32;
            let b = ( px        & 0xFF) as i32;
            let new_r = (r + (green_to_red as i32 * g >> 5)) & 0xFF;
            let new_b = (b + (green_to_blue as i32 * g >> 5) + (red_to_blue as i32 * r >> 5)) & 0xFF;
            pixels[y * w + x] = (a << 24) | ((new_r as u32) << 16) | ((g as u32) << 8) | (new_b as u32);
        }
    }
}

fn apply_predictor(pixels: &mut [u32], w: usize, h: usize, t: &Transform) {
    let bs = 1usize << t.block_bits;
    let bw = (w + bs - 1) / bs;

    // Top-left pixel: predictor 0 (predict black = 0xFF000000).
    // First row: predictor 1 (left neighbour).
    // First column: predictor 2 (top neighbour).

    for y in 0..h {
        for x in 0..w {
            let bx = x / bs;
            let by = y / bs;
            let mode = (t.data[by * bw + bx] >> 8) & 0xFF;

            let left  = if x > 0  { pixels[y * w + x - 1] } else { 0xFF000000 };
            let top   = if y > 0  { pixels[(y-1) * w + x] } else { 0xFF000000 };
            let tl    = if x > 0 && y > 0 { pixels[(y-1) * w + x - 1] } else { 0xFF000000 };
            let tr    = if y > 0 { if x + 1 < w { pixels[(y-1) * w + x + 1] } else { pixels[(y-1) * w + x] } } else { 0xFF000000 };

            let pred = match mode {
                0  => 0xFF000000u32,
                1  => left,
                2  => top,
                3  => tr,
                4  => tl,
                5  => average2(average2(left, tr), top),
                6  => average2(left, tl),
                7  => average2(left, top),
                8  => average2(tl, top),
                9  => average2(top, tr),
                10 => average2(average2(left, tl), average2(top, tr)),
                11 => select(left, top, tl),
                12 => clamp_add_sub_full(left, top, tl),
                13 => clamp_add_sub_half(average2(left, top), tl),
                _  => left,
            };
            pixels[y * w + x] = add_argb(pixels[y * w + x], pred);
        }
    }
}

#[inline]
fn average2(a: u32, b: u32) -> u32 {
    let avg_ch = |hi: u32, lo: u32| -> u32 {
        (((a >> hi) & 0xFF) + ((b >> hi) & 0xFF)) / 2
    };
    (avg_ch(24, 24) << 24) | (avg_ch(16, 16) << 16) | (avg_ch(8, 8) << 8) | avg_ch(0, 0)
}

#[inline]
fn select(left: u32, top: u32, tl: u32) -> u32 {
    // For each channel: if |top - tl| < |left - tl|, predict top, else left.
    let ch = |shift: u32| -> u32 {
        let l  = ((left >> shift) & 0xFF) as i32;
        let t  = ((top  >> shift) & 0xFF) as i32;
        let tl = ((tl   >> shift) & 0xFF) as i32;
        if (t - tl).abs() < (l - tl).abs() { t as u32 } else { l as u32 }
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

#[inline]
fn clamp(v: i32) -> u32 { v.clamp(0, 255) as u32 }

#[inline]
fn clamp_add_sub_full(a: u32, b: u32, c: u32) -> u32 {
    let ch = |s: u32| -> u32 {
        let av = ((a >> s) & 0xFF) as i32;
        let bv = ((b >> s) & 0xFF) as i32;
        let cv = ((c >> s) & 0xFF) as i32;
        clamp(av + bv - cv)
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

#[inline]
fn clamp_add_sub_half(a: u32, b: u32) -> u32 {
    let ch = |s: u32| -> u32 {
        let av = ((a >> s) & 0xFF) as i32;
        let bv = ((b >> s) & 0xFF) as i32;
        clamp(av + (av - bv) / 2)
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

fn apply_color_indexing(
    src: &[u32], src_w: usize,
    out: &mut [u32], dst_w: usize, h: usize,
    t: &Transform,
) {
    let bpp = t.bits_per_pixel;
    let ppu = 8 / bpp; // pixels per "code pixel"
    let mask = (1u32 << bpp) - 1;
    let pal = &t.palette;

    for y in 0..h {
        for x in 0..dst_w {
            let unit = x / ppu as usize;
            let bit_off = (x % ppu as usize) as u32 * bpp;
            let packed = if unit < src_w { src[y * src_w + unit] } else { 0 };
            let g = (packed >> 8) & 0xFF; // index stored in G channel
            let idx = ((g >> bit_off) & mask) as usize;
            let idx = idx.min(pal.len().saturating_sub(1));
            out[y * dst_w + x] = pal[idx];
        }
    }
}

// We need a `?` operator for Option inside a non-option-returning function.
// The `decode_vp8l_image` helper uses `?` via the trait impl below.
// To avoid changing the function signature we use an internal Result-based wrapper.
trait OptionExt<T> {
    fn ok_or_false(self) -> Result<T, ()>;
}
impl<T> OptionExt<T> for Option<T> {
    fn ok_or_false(self) -> Result<T, ()> { self.ok_or(()) }
}

// ══════════════════════════════════════════════════════════════════════════════
// VP8 LOSSY DECODER
// ══════════════════════════════════════════════════════════════════════════════
//
// Implements the VP8 (lossy) bitstream decoder for WebP images.
// Reference: RFC 6386 — VP8 Data Format and Decoding Guide
//
// Supports:
//   - Keyframes only (sufficient for WebP — WebP lossy is always a single keyframe)
//   - All 4 luma 16x16 intra-prediction modes (DC, V, H, TM)
//   - All 10 luma 4x4 intra-prediction modes
//   - All 4 chroma 8x8 intra-prediction modes
//   - Boolean arithmetic decoder
//   - WHT (Walsh-Hadamard Transform) for DC coefficients
//   - 4x4 DCT inverse transform with dequantization
//   - Simple loop filter
//   - Segmentation and per-segment quantization
//   - Token partition (first partition only)
//   - YUV 4:2:0 to ARGB conversion

// ── Boolean arithmetic decoder (RFC 6386 §7) ────────────────────────────────

/// VP8 boolean arithmetic decoder (RFC 6386 §7).
struct BoolDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    range: u32,
    value: u32,
    bits_left: i32,
}

impl<'a> BoolDecoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        if data.len() < 2 { return BoolDecoder { data, pos: 2, range: 255, value: 0, bits_left: 0 }; }
        let value = ((data[0] as u32) << 8) | (data[1] as u32);
        BoolDecoder { data, pos: 2, range: 255, value, bits_left: 0 }
    }

    fn read_bit(&mut self, prob: u8) -> u32 {
        let split = 1 + (((self.range - 1) * prob as u32) >> 8);
        let bigsplit = split << 8;
        let big = self.value >= bigsplit;
        if big {
            self.range -= split;
            self.value -= bigsplit;
        } else {
            self.range = split;
        }
        while self.range < 128 {
            self.value <<= 1;
            self.range <<= 1;
            self.bits_left += 1;
            if self.bits_left == 8 {
                self.bits_left = 0;
                if self.pos < self.data.len() {
                    self.value |= self.data[self.pos] as u32;
                    self.pos += 1;
                }
            }
        }
        big as u32
    }

    fn read_bool(&mut self, prob: u8) -> bool { self.read_bit(prob) != 0 }

    fn read_literal(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.read_bit(128);
        }
        v
    }

    fn read_signed(&mut self, n: u32) -> i32 {
        let v = self.read_literal(n) as i32;
        if self.read_bool(128) { -v } else { v }
    }
}

// ── VP8 constants ────────────────────────────────────────────────────────────

const MB_FEATURE_TREE_PROBS: usize = 3;

// Intra 16x16 prediction modes
const DC_PRED: u8  = 0;
const V_PRED: u8   = 1;
const H_PRED: u8   = 2;
const TM_PRED: u8  = 3;

// Intra 4x4 sub-block prediction modes
const B_DC_PRED: u8 = 0;
const B_TM_PRED: u8 = 1;
const B_VE_PRED: u8 = 2;
const B_HE_PRED: u8 = 3;
const B_LD_PRED: u8 = 4;
const B_RD_PRED: u8 = 5;
const B_VR_PRED: u8 = 6;
const B_VL_PRED: u8 = 7;
const B_HD_PRED: u8 = 8;
const B_HU_PRED: u8 = 9;

// ── Default probability tables (RFC 6386 §11.2, §13.4) ──────────────────────

/// Default probabilities for the coefficient token tree.
/// Indexed by [plane_type][coeff_band][ctx][token_node]
/// plane_type: 0=Y-AC(16x16 mode), 1=Y2, 2=UV(chroma), 3=Y-DC(B_PRED 4x4 mode)
/// Simplified: we use 4 types × 8 bands × 3 ctx × 11 probabilities
static DEFAULT_COEFF_PROBS: [[[[u8; 11]; 3]; 8]; 4] = [
    // Type 0: Y (after Y2 DC) — used for AC coefficients of luma
    [
        [[128,128,128,128,128,128,128,128,128,128,128],[128,128,128,128,128,128,128,128,128,128,128],[128,128,128,128,128,128,128,128,128,128,128]],
        [[253,136,254,255,228,219,128,128,128,128,128],[189,129,242,255,227,213,255,219,128,128,128],[106,126,227,252,214,209,255,255,128,128,128]],
        [[  1, 98,248,255,236,226,255,255,128,128,128],[181,133,238,254,221,234,255,154,128,128,128],[ 78,134,202,247,198,180,255,219,128,128,128]],
        [[  1,185,249,255,243,255,128,128,128,128,128],[184,150,247,255,236,224,128,128,128,128,128],[ 77,110,216,255,236,230,128,128,128,128,128]],
        [[  1,101,251,255,241,255,128,128,128,128,128],[170,139,241,252,236,209,255,255,128,128,128],[ 37,116,196,243,228,255,255,255,128,128,128]],
        [[  1,204,254,255,245,255,128,128,128,128,128],[207,160,250,255,238,128,128,128,128,128,128],[ 102,103,231,255,211,171,128,128,128,128,128]],
        [[  1,152,252,255,240,255,128,128,128,128,128],[177,135,243,255,234,225,128,128,128,128,128],[ 80,129,211,255,194,224,128,128,128,128,128]],
        [[  1,  1,255,128,128,128,128,128,128,128,128],[246,  1,255,128,128,128,128,128,128,128,128],[255,128,128,128,128,128,128,128,128,128,128]],
    ],
    // Type 1: Y2 (DC/AC for the whole macroblock DC block)
    [
        [[198, 35,237,223,193,187,162,160,145,155,  62],[131, 45,198,221,172,176,220,157,252,221,  1],[ 68, 47,146,208,149,167,221,162,255,223,128]],
        [[  1,149,241,255,221,224,255,255,128,128,128],[184,141,234,253,222,220,255,199,128,128,128],[ 81,99,181,242,176,190,249,202,255,255,128]],
        [[  1,129,232,253,214,197,242,196,255,255,128],[99,121,210,250,201,198,255,202,128,128,128],[ 23, 91,163,242,170,187,247,210,255,255,128]],
        [[  1,200,246,255,234,255,128,128,128,128,128],[109,178,241,255,231,245,255,255,128,128,128],[ 44,130,201,253,205,192,255,255,128,128,128]],
        [[  1,132,239,251,219,209,255,165,128,128,128],[94,136,225,251,218,190,255,255,128,128,128],[ 22, 100,174,245,186,161,255,199,128,128,128]],
        [[  1,182,249,255,232,235,128,128,128,128,128],[124,143,241,255,227,234,128,128,128,128,128],[ 35,77,181,251,193,211,255,205,128,128,128]],
        [[  1,157,247,255,236,231,255,255,128,128,128],[121,141,235,255,225,227,255,255,128,128,128],[ 45,99,188,251,195,217,255,224,128,128,128]],
        [[  1,  1,251,255,213,255,128,128,128,128,128],[203,  1,248,255,255,128,128,128,128,128,128],[137, 1,177,255,224,255,128,128,128,128,128]],
    ],
    // Type 2: Y (intra 4x4, no Y2)
    [
        [[253,  9,248,251,207,208,255,192,128,128,128],[175, 13,224,243,193,185,249,198,255,255,128],[ 73, 17,171,221,161,179,236,167,255,234,128]],
        [[  1, 95,247,253,212,183,255,255,128,128,128],[239, 90,244,250,211,209,255,255,128,128,128],[ 155, 77,195,248,188,195,255,255,128,128,128]],
        [[  1, 24,239,251,218,219,255,205,128,128,128],[201, 51,219,255,196,186,128,128,128,128,128],[ 69, 46,190,239,201,218,255,228,128,128,128]],
        [[  1,191,251,255,255,128,128,128,128,128,128],[223,165,249,255,213,255,128,128,128,128,128],[141, 124,248,255,255,128,128,128,128,128,128]],
        [[  1, 16,248,255,255,128,128,128,128,128,128],[190, 36,230,255,236,255,128,128,128,128,128],[149, 1,255,128,128,128,128,128,128,128,128]],
        [[  1,226,255,128,128,128,128,128,128,128,128],[247,192,255,128,128,128,128,128,128,128,128],[240,128,255,128,128,128,128,128,128,128,128]],
        [[  1,134,252,255,255,128,128,128,128,128,128],[213, 62,250,255,255,128,128,128,128,128,128],[55, 93,255,128,128,128,128,128,128,128,128]],
        [[128,128,128,128,128,128,128,128,128,128,128],[128,128,128,128,128,128,128,128,128,128,128],[128,128,128,128,128,128,128,128,128,128,128]],
    ],
    // Type 3: UV (chroma)
    [
        [[202, 24,213,235,186,191,220,160,240,175,255],[126, 38,182,232,169,184,228,174,255,187,128],[ 61, 46,138,219,151,178,240,170,255,216,128]],
        [[  1,112,230,250,199,191,247,159,255,255,128],[166,109,228,252,211,215,255,174,128,128,128],[ 39, 77,162,232,172,180,245,178,255,255,128]],
        [[  1, 52,220,246,198,199,249,220,255,255,128],[124, 74,191,243,183,193,250,221,255,255,128],[ 24, 71,130,219,154,170,243,182,255,255,128]],
        [[  1,182,225,249,219,240,255,224,128,128,128],[149,150,226,252,216,205,255,171,128,128,128],[ 28, 108,170,242,183,194,254,223,255,255,128]],
        [[  1, 81,230,252,204,203,255,192,128,128,128],[123, 102,209,247,188,196,255,233,128,128,128],[ 20, 95,153,243,164,173,255,203,128,128,128]],
        [[  1,222,248,255,216,213,128,128,128,128,128],[168,175,246,252,235,205,255,255,128,128,128],[ 47,116,215,255,211,212,255,255,128,128,128]],
        [[  1,121,236,253,212,214,255,255,128,128,128],[141,84,213,252,201,202,255,219,128,128,128],[ 42,80,160,240,162,185,255,205,128,128,128]],
        [[  1,  1,255,128,128,128,128,128,128,128,128],[244,  1,255,128,128,128,128,128,128,128,128],[238,  1,255,128,128,128,128,128,128,128,128]],
    ],
];

/// Keyframe default Y-mode probabilities (RFC 6386 §11.3)
static KF_Y_MODE_PROBS: [u8; 4] = [145, 156, 163, 128];
/// Keyframe default UV-mode probabilities
static KF_UV_MODE_PROBS: [u8; 3] = [142, 114, 183];

/// Keyframe sub-block mode probabilities — indexed by [above_mode][left_mode][node]
/// (RFC 6386 §12.1, values from libvpx kf_bmode_prob)
static KF_BMODE_PROBS: [[[u8; 9]; 10]; 10] = [
    [[231,120,48,89,115,113,120,152,112],[152,179,64,126,170,118,46,70,95],[175,69,143,80,85,82,72,155,103],[56,58,10,171,218,189,17,13,152],[144,71,10,38,171,213,144,34,26],[114,26,17,163,44,195,21,10,173],[121,24,80,195,26,62,44,64,85],[170,46,55,19,136,160,33,206,71],[63,20,8,114,114,208,12,9,226],[81,40,11,96,182,84,29,16,36]],
    [[134,183,89,137,98,101,106,165,148],[72,187,100,130,157,111,32,75,80],[66,102,167,99,74,62,40,234,128],[41,53,9,178,241,141,26,8,107],[104,79,12,27,217,255,87,17,7],[74,43,26,146,73,166,49,23,157],[65,38,105,160,51,52,31,115,128],[87,68,71,44,114,51,15,186,23],[47,41,14,110,182,183,21,17,194],[66,45,25,102,197,189,23,18,22]],
    [[88,88,147,150,42,46,45,196,205],[43,97,183,117,85,38,35,179,61],[39,53,200,87,26,21,43,232,171],[56,34,51,104,114,102,29,93,77],[107,54,32,26,51,1,81,43,31],[39,28,85,171,58,165,90,98,64],[34,22,116,206,23,34,43,166,73],[68,25,106,22,64,171,36,225,114],[34,19,21,102,132,188,16,76,124],[62,18,78,95,85,57,50,48,51]],
    [[193,101,35,159,215,111,89,46,111],[60,148,31,172,219,228,21,18,111],[112,113,77,85,179,255,38,120,114],[40,42,1,196,245,209,10,25,109],[100,80,8,43,154,1,51,26,71],[88,43,29,140,166,213,37,43,154],[61,63,30,155,67,45,68,1,209],[142,78,78,16,255,128,34,197,171],[41,40,5,102,211,183,4,1,221],[51,50,17,168,209,192,23,25,82]],
    [[125,98,42,88,104,85,117,175,82],[95,84,53,89,128,100,113,101,45],[75,79,123,47,51,128,81,171,1],[57,17,5,71,102,57,53,41,49],[115,21,2,10,102,255,166,23,6],[38,33,13,121,57,73,26,1,85],[41,10,67,138,77,110,90,47,114],[101,29,16,10,85,128,101,196,26],[57,18,10,102,102,213,34,20,43],[117,20,15,36,163,128,68,1,26]],
    [[138,31,36,171,27,166,38,44,229],[67,87,58,169,82,115,26,59,179],[63,59,90,180,59,166,93,73,154],[40,40,21,116,143,209,34,39,175],[57,46,22,24,128,1,54,17,37],[47,15,16,183,34,223,49,45,183],[46,17,33,183,6,98,15,32,183],[65,32,73,115,28,128,23,128,205],[40,3,9,115,51,192,18,6,223],[87,37,9,115,59,77,64,21,47]],
    [[104,55,44,218,9,54,53,130,226],[64,90,70,205,40,41,23,26,57],[54,57,112,184,5,41,38,166,213],[30,34,26,133,152,116,10,32,134],[75,32,12,51,192,255,160,43,51],[39,19,53,221,26,114,32,73,255],[31,9,65,234,2,15,1,118,73],[88,31,35,67,102,85,55,186,85],[56,21,23,111,59,205,45,37,192],[55,38,70,124,73,102,1,34,98]],
    [[102,61,71,37,34,53,31,243,192],[69,60,71,38,73,119,28,222,37],[68,45,128,34,1,47,11,245,171],[62,17,19,70,146,85,55,62,70],[75,15,9,9,64,255,184,119,16],[37,43,37,154,100,163,85,160,1],[63,9,92,136,28,64,32,201,85],[86,6,28,5,64,255,25,248,1],[56,8,17,132,137,255,55,116,128],[58,15,20,82,135,57,26,121,40]],
    [[164,50,31,137,154,133,25,35,218],[51,103,44,131,131,123,31,6,158],[86,40,64,135,148,224,45,183,128],[22,26,17,131,240,154,14,1,209],[83,12,13,54,192,255,68,47,28],[45,16,21,91,64,222,7,1,197],[56,21,39,155,60,138,23,102,213],[85,26,85,85,128,128,32,146,171],[18,11,7,63,144,171,4,4,246],[35,27,10,146,174,171,12,26,128]],
    [[190,80,35,99,180,80,126,54,45],[85,126,47,87,176,51,41,20,32],[101,75,128,139,118,146,116,128,85],[56,41,15,176,236,85,37,9,62],[146,36,19,30,171,255,97,27,20],[71,30,17,119,118,255,17,18,138],[101,38,60,138,55,70,43,26,142],[138,45,61,62,219,1,81,188,64],[32,41,20,117,151,142,20,21,163],[112,19,12,61,195,128,48,4,24]],
];

/// Coefficient band index for each of the 16 DCT coefficients (zig-zag order).
static COEFF_BANDS: [u8; 16] = [0, 1, 2, 3, 6, 4, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7];

/// DC dequantization lookup indexed by QP (0..127)
static DC_QUANT: [i16; 128] = [
      4,   5,   6,   7,   8,   9,  10,  10,  11,  12,  13,  14,  15,  16,  17,  17,
     18,  19,  20,  20,  21,  21,  22,  22,  23,  23,  24,  25,  25,  26,  27,  28,
     29,  30,  31,  32,  33,  34,  35,  36,  37,  37,  38,  39,  40,  41,  42,  43,
     44,  45,  46,  46,  47,  48,  49,  50,  51,  52,  53,  54,  55,  56,  57,  58,
     59,  60,  61,  62,  63,  64,  65,  66,  67,  68,  69,  70,  71,  72,  73,  74,
     75,  76,  76,  77,  78,  79,  80,  81,  82,  83,  84,  85,  86,  87,  88,  89,
     91,  93,  95,  96,  98, 100, 101, 102, 104, 106, 108, 110, 112, 114, 116, 118,
    122, 124, 126, 128, 130, 132, 134, 136, 138, 140, 143, 145, 148, 151, 154, 157,
];

/// AC dequantization lookup indexed by QP (0..127)
static AC_QUANT: [i16; 128] = [
      4,   5,   6,   7,   8,   9,  10,  11,  12,  13,  14,  15,  16,  17,  18,  19,
     20,  21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,  32,  33,  34,  35,
     36,  37,  38,  39,  40,  41,  42,  43,  44,  45,  46,  47,  48,  49,  50,  51,
     52,  53,  54,  55,  56,  57,  58,  60,  62,  64,  66,  68,  70,  72,  74,  76,
     78,  80,  82,  84,  86,  88,  90,  92,  94,  96,  98, 100, 102, 104, 106, 108,
    110, 112, 114, 116, 119, 122, 125, 128, 131, 134, 137, 140, 143, 146, 149, 152,
    155, 158, 161, 164, 167, 170, 173, 177, 181, 185, 189, 193, 197, 201, 205, 209,
    213, 217, 221, 225, 229, 234, 239, 245, 249, 254, 259, 264, 269, 274, 279, 284,
];

/// Zig-zag order for 4x4 block
static ZIGZAG: [usize; 16] = [0,1,4,8,5,2,3,6,9,12,13,10,7,11,14,15];

// ── Frame header parsing ────────────────────────────────────────────────────

struct Vp8FrameHeader {
    width: u32,
    height: u32,
    x_scale: u32,
    y_scale: u32,
    first_part_size: u32,
}

fn parse_vp8_frame_header(data: &[u8]) -> Option<Vp8FrameHeader> {
    if data.len() < 10 { return None; }
    let tag = (data[0] as u32) | ((data[1] as u32) << 8) | ((data[2] as u32) << 16);
    let is_keyframe = (tag & 1) == 0;
    if !is_keyframe { return None; } // WebP is always keyframe
    let _version = (tag >> 1) & 7;
    let _show_frame = (tag >> 4) & 1;
    let first_part_size = tag >> 5;

    // Start code: 0x9D 0x01 0x2A
    if data[3] != 0x9D || data[4] != 0x01 || data[5] != 0x2A { return None; }

    let w_code = u16::from_le_bytes([data[6], data[7]]);
    let h_code = u16::from_le_bytes([data[8], data[9]]);
    let width = (w_code & 0x3FFF) as u32;
    let height = (h_code & 0x3FFF) as u32;
    let x_scale = (w_code >> 14) as u32;
    let y_scale = (h_code >> 14) as u32;

    Some(Vp8FrameHeader { width, height, x_scale, y_scale, first_part_size })
}

// ── Quantization parameters ─────────────────────────────────────────────────

struct QuantParams {
    y_dc: i16,
    y_ac: i16,
    y2_dc: i16,
    y2_ac: i16,
    uv_dc: i16,
    uv_ac: i16,
}

fn clamp_qp(v: i32) -> usize { (v.max(0).min(127)) as usize }

fn build_quant(base_qp: i32, y_dc_delta: i32, y2_dc_delta: i32, y2_ac_delta: i32,
               uv_dc_delta: i32, uv_ac_delta: i32) -> QuantParams {
    QuantParams {
        y_dc:  DC_QUANT[clamp_qp(base_qp + y_dc_delta)],
        y_ac:  AC_QUANT[clamp_qp(base_qp)],
        y2_dc: DC_QUANT[clamp_qp(base_qp + y2_dc_delta)] * 2,
        y2_ac: ((AC_QUANT[clamp_qp(base_qp + y2_ac_delta)] as i32 * 155 / 100) as i16).max(8),
        uv_dc: DC_QUANT[clamp_qp(base_qp + uv_dc_delta)].min(132),
        uv_ac: AC_QUANT[clamp_qp(base_qp + uv_ac_delta)],
    }
}

// ── 4×4 Inverse DCT (RFC 6386 §14.3) ───────────────────────────────────────

fn idct4x4(input: &[i16; 16], dst: &mut [u8], stride: usize) {
    // VP8 IDCT uses simplified constants (RFC 6386 §14.4):
    //   sinpi8sqrt2       = 35468
    //   cospi8sqrt2minus1 = 20091
    // Transform:  t1 = (x * sinpi8sqrt2) >> 16
    //             t2 = x + ((x * cospi8sqrt2minus1) >> 16)
    const SINPI8: i32 = 35468;
    const COSPI8M1: i32 = 20091;

    let mut tmp = [0i32; 16];

    // Columns
    for i in 0..4 {
        let a1 = input[i] as i32 + input[8 + i] as i32;
        let b1 = input[i] as i32 - input[8 + i] as i32;
        let ip4 = input[4 + i] as i32;
        let ip12 = input[12 + i] as i32;
        let t1 = (ip4 * SINPI8 >> 16) - ip12 - (ip12 * COSPI8M1 >> 16);
        let t2 = ip4 + (ip4 * COSPI8M1 >> 16) + (ip12 * SINPI8 >> 16);
        tmp[i]      = a1 + t2;
        tmp[4 + i]  = b1 + t1;
        tmp[8 + i]  = b1 - t1;
        tmp[12 + i] = a1 - t2;
    }

    // Rows
    for i in 0..4 {
        let r = i * 4;
        let a1 = tmp[r] + tmp[r + 2];
        let b1 = tmp[r] - tmp[r + 2];
        let t1 = (tmp[r + 1] * SINPI8 >> 16) - tmp[r + 3] - (tmp[r + 3] * COSPI8M1 >> 16);
        let t2 = tmp[r + 1] + (tmp[r + 1] * COSPI8M1 >> 16) + (tmp[r + 3] * SINPI8 >> 16);
        let d0 = (a1 + t2 + 4) >> 3;
        let d1 = (b1 + t1 + 4) >> 3;
        let d2 = (b1 - t1 + 4) >> 3;
        let d3 = (a1 - t2 + 4) >> 3;
        let row = i * stride;
        dst[row]     = (dst[row] as i32 + d0).clamp(0, 255) as u8;
        dst[row + 1] = (dst[row + 1] as i32 + d1).clamp(0, 255) as u8;
        dst[row + 2] = (dst[row + 2] as i32 + d2).clamp(0, 255) as u8;
        dst[row + 3] = (dst[row + 3] as i32 + d3).clamp(0, 255) as u8;
    }
}

/// Walsh-Hadamard inverse transform for the 4x4 DC block of Y2
fn iwht4x4(input: &[i16; 16], output: &mut [i16; 16]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let a = input[i] as i32 + input[12 + i] as i32;
        let b = input[4 + i] as i32 + input[8 + i] as i32;
        let c = input[4 + i] as i32 - input[8 + i] as i32;
        let d = input[i] as i32 - input[12 + i] as i32;
        tmp[i]      = a + b;
        tmp[4 + i]  = c + d;
        tmp[8 + i]  = a - b;
        tmp[12 + i] = d - c;
    }
    for i in 0..4 {
        let r = i * 4;
        let a = tmp[r] + tmp[r + 3];
        let b = tmp[r + 1] + tmp[r + 2];
        let c = tmp[r + 1] - tmp[r + 2];
        let d = tmp[r] - tmp[r + 3];
        output[r]     = ((a + b + 3) >> 3) as i16;
        output[r + 1] = ((c + d + 3) >> 3) as i16;
        output[r + 2] = ((a - b + 3) >> 3) as i16;
        output[r + 3] = ((d - c + 3) >> 3) as i16;
    }
}

// ── Intra prediction: 16x16 modes ──────────────────────────────────────────

fn predict_16x16(mode: u8, dst: &mut [u8], stride: usize, above: &[u8; 16], left: &[u8; 16], tl: u8) {
    match mode {
        DC_PRED => {
            let sum: u32 = above.iter().map(|&x| x as u32).sum::<u32>()
                + left.iter().map(|&x| x as u32).sum::<u32>();
            let dc = ((sum + 16) >> 5) as u8;
            for y in 0..16 {
                for x in 0..16 { dst[y * stride + x] = dc; }
            }
        }
        V_PRED => {
            for y in 0..16 {
                dst[y * stride..y * stride + 16].copy_from_slice(above);
            }
        }
        H_PRED => {
            for y in 0..16 {
                for x in 0..16 { dst[y * stride + x] = left[y]; }
            }
        }
        TM_PRED => {
            for y in 0..16 {
                for x in 0..16 {
                    dst[y * stride + x] = (left[y] as i32 + above[x] as i32 - tl as i32).clamp(0, 255) as u8;
                }
            }
        }
        _ => {}
    }
}

// ── Intra prediction: 8x8 chroma modes ─────────────────────────────────────

fn predict_8x8(mode: u8, dst: &mut [u8], stride: usize, above: &[u8; 8], left: &[u8; 8], tl: u8) {
    match mode {
        DC_PRED => {
            let sum: u32 = above.iter().map(|&x| x as u32).sum::<u32>()
                + left.iter().map(|&x| x as u32).sum::<u32>();
            let dc = ((sum + 8) >> 4) as u8;
            for y in 0..8 {
                for x in 0..8 { dst[y * stride + x] = dc; }
            }
        }
        V_PRED => {
            for y in 0..8 { dst[y * stride..y * stride + 8].copy_from_slice(above); }
        }
        H_PRED => {
            for y in 0..8 {
                for x in 0..8 { dst[y * stride + x] = left[y]; }
            }
        }
        TM_PRED => {
            for y in 0..8 {
                for x in 0..8 {
                    dst[y * stride + x] = (left[y] as i32 + above[x] as i32 - tl as i32).clamp(0, 255) as u8;
                }
            }
        }
        _ => {}
    }
}

// ── Intra prediction: 4x4 sub-block modes ──────────────────────────────────

fn predict_4x4(mode: u8, dst: &mut [u8], stride: usize, above: &[u8; 8], left: &[u8; 4], tl: u8) {
    // above[0..4] = pixels above, above[4..8] = pixels above-right
    match mode {
        B_DC_PRED => {
            let sum: u32 = above[..4].iter().map(|&x| x as u32).sum::<u32>()
                + left.iter().map(|&x| x as u32).sum::<u32>();
            let dc = ((sum + 4) >> 3) as u8;
            for y in 0..4 { for x in 0..4 { dst[y*stride+x] = dc; } }
        }
        B_TM_PRED => {
            for y in 0..4 { for x in 0..4 {
                dst[y*stride+x] = (left[y] as i32 + above[x] as i32 - tl as i32).clamp(0,255) as u8;
            }}
        }
        B_VE_PRED => {
            // Smoothed vertical: avg3(above[x-1], above[x], above[x+1])
            let mut row = [0u8; 4];
            row[0] = avg3(tl, above[0], above[1]);
            row[1] = avg3(above[0], above[1], above[2]);
            row[2] = avg3(above[1], above[2], above[3]);
            row[3] = avg3(above[2], above[3], above[4]);
            for y in 0..4 { dst[y*stride..y*stride+4].copy_from_slice(&row); }
        }
        B_HE_PRED => {
            let r0 = avg3(tl, left[0], left[1]);
            let r1 = avg3(left[0], left[1], left[2]);
            let r2 = avg3(left[1], left[2], left[3]);
            let r3 = avg3(left[2], left[3], left[3]);
            for x in 0..4 { dst[x] = r0; }
            for x in 0..4 { dst[stride+x] = r1; }
            for x in 0..4 { dst[2*stride+x] = r2; }
            for x in 0..4 { dst[3*stride+x] = r3; }
        }
        B_LD_PRED => {
            let a = above;
            dst[0]           = avg3(a[0],a[1],a[2]);
            dst[1]           = avg3(a[1],a[2],a[3]);
            dst[stride]      = dst[1];
            dst[2]           = avg3(a[2],a[3],a[4]);
            dst[stride+1]    = dst[2];
            dst[2*stride]    = dst[2];
            dst[3]           = avg3(a[3],a[4],a[5]);
            dst[stride+2]    = dst[3];
            dst[2*stride+1]  = dst[3];
            dst[3*stride]    = dst[3];
            let d4 = avg3(a[4],a[5],a[6]);
            dst[stride+3]    = d4;
            dst[2*stride+2]  = d4;
            dst[3*stride+1]  = d4;
            let d5 = avg3(a[5],a[6],a[7]);
            dst[2*stride+3]  = d5;
            dst[3*stride+2]  = d5;
            dst[3*stride+3]  = avg3(a[6],a[7],a[7]);
        }
        B_RD_PRED => {
            let l = left;
            let a = above;
            let p = tl;
            dst[3*stride]    = avg3(l[3],l[2],l[1]);
            dst[3*stride+1]  = avg3(l[2],l[1],l[0]);
            dst[2*stride]    = dst[3*stride+1];
            dst[3*stride+2]  = avg3(l[1],l[0],p);
            dst[2*stride+1]  = dst[3*stride+2];
            dst[stride]      = dst[3*stride+2];
            dst[3*stride+3]  = avg3(l[0],p,a[0]);
            dst[2*stride+2]  = dst[3*stride+3];
            dst[stride+1]    = dst[3*stride+3];
            dst[0]           = dst[3*stride+3];
            dst[stride+2]    = avg3(p,a[0],a[1]);
            dst[2*stride+3]  = dst[stride+2];
            dst[1]           = dst[stride+2];
            dst[stride+3]    = avg3(a[0],a[1],a[2]);
            dst[2]           = dst[stride+3];
            dst[3]           = avg3(a[1],a[2],a[3]);
        }
        B_VR_PRED => {
            let l = left;
            let a = above;
            let p = tl;
            dst[3*stride]    = avg3(l[2],l[1],l[0]);
            dst[2*stride]    = avg3(l[1],l[0],p);
            dst[3*stride+1]  = dst[2*stride];
            dst[stride]      = avg3(l[0],p,a[0]);
            dst[2*stride+1]  = dst[stride];
            dst[0]           = avg2(p,a[0]);
            dst[stride+1]    = avg3(p,a[0],a[1]);
            dst[2*stride+2]  = dst[stride+1];
            dst[1]           = avg2(a[0],a[1]);
            dst[stride+2]    = avg3(a[0],a[1],a[2]);
            dst[2*stride+3]  = dst[stride+2];
            dst[3*stride+2]  = dst[stride+2];
            dst[2]           = avg2(a[1],a[2]);
            dst[stride+3]    = avg3(a[1],a[2],a[3]);
            dst[3*stride+3]  = dst[stride+3];
            dst[3]           = avg2(a[2],a[3]);
        }
        B_VL_PRED => {
            let a = above;
            dst[0]           = avg2(a[0],a[1]);
            dst[stride]      = avg3(a[0],a[1],a[2]);
            dst[2*stride]    = avg2(a[1],a[2]);
            dst[1]           = dst[2*stride];
            dst[stride+1]    = avg3(a[1],a[2],a[3]);
            dst[3*stride]    = dst[stride+1];
            dst[2*stride+1]  = avg2(a[2],a[3]);
            dst[2]           = dst[2*stride+1];
            dst[stride+2]    = avg3(a[2],a[3],a[4]);
            dst[3*stride+1]  = dst[stride+2];
            dst[2*stride+2]  = avg2(a[3],a[4]);
            dst[3]           = dst[2*stride+2];
            dst[stride+3]    = avg3(a[3],a[4],a[5]);
            dst[3*stride+2]  = dst[stride+3];
            dst[2*stride+3]  = avg3(a[4],a[5],a[6]);
            dst[3*stride+3]  = avg3(a[5],a[6],a[7]);
        }
        B_HD_PRED => {
            let l = left;
            let a = above;
            let p = tl;
            dst[3*stride]    = avg2(l[3],l[2]);
            dst[3*stride+1]  = avg3(l[3],l[2],l[1]);
            dst[2*stride]    = avg2(l[2],l[1]);
            dst[3*stride+2]  = dst[2*stride];
            dst[2*stride+1]  = avg3(l[2],l[1],l[0]);
            dst[3*stride+3]  = dst[2*stride+1];
            dst[stride]      = avg2(l[1],l[0]);
            dst[2*stride+2]  = dst[stride];
            dst[stride+1]    = avg3(l[1],l[0],p);
            dst[2*stride+3]  = dst[stride+1];
            dst[0]           = avg2(l[0],p);
            dst[stride+2]    = dst[0];
            dst[1]           = avg3(l[0],p,a[0]);
            dst[stride+3]    = dst[1];
            dst[2]           = avg3(p,a[0],a[1]);
            dst[3]           = avg3(a[0],a[1],a[2]);
        }
        B_HU_PRED => {
            let l = left;
            dst[0]           = avg2(l[0],l[1]);
            dst[1]           = avg3(l[0],l[1],l[2]);
            dst[stride]      = avg2(l[1],l[2]);
            dst[2]           = dst[stride];
            dst[stride+1]    = avg3(l[1],l[2],l[3]);
            dst[3]           = dst[stride+1];
            dst[2*stride]    = avg2(l[2],l[3]);
            dst[stride+2]    = dst[2*stride];
            dst[2*stride+1]  = avg3(l[2],l[3],l[3]);
            dst[stride+3]    = dst[2*stride+1];
            dst[2*stride+2]  = l[3];
            dst[2*stride+3]  = l[3];
            dst[3*stride]    = l[3];
            dst[3*stride+1]  = l[3];
            dst[3*stride+2]  = l[3];
            dst[3*stride+3]  = l[3];
        }
        _ => {
            // Fallback: DC with 128
            for y in 0..4 { for x in 0..4 { dst[y*stride+x] = 128; } }
        }
    }
}

#[inline] fn avg2(a: u8, b: u8) -> u8 { ((a as u16 + b as u16 + 1) >> 1) as u8 }
#[inline] fn avg3(a: u8, b: u8, c: u8) -> u8 { ((a as u16 + 2 * b as u16 + c as u16 + 2) >> 2) as u8 }

// ── Token/coefficient decoding ──────────────────────────────────────────────

/// Read a single DCT coefficient using the token tree.
/// Returns (value, is_eob)
/// Category extra-bit probabilities (RFC 6386 §13.3).
/// Each category defines fixed probabilities for its extra bits.
static CAT1_PROB: [u8; 1] = [159];
static CAT2_PROB: [u8; 2] = [165, 145];
static CAT3_PROB: [u8; 3] = [173, 148, 140];
static CAT4_PROB: [u8; 4] = [176, 155, 140, 135];
static CAT5_PROB: [u8; 5] = [180, 157, 141, 134, 130];
static CAT6_PROB: [u8; 11] = [254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129];

/// Read extra bits for a DCT coefficient category using fixed probabilities.
fn read_cat_extra(bd: &mut BoolDecoder, probs: &[u8]) -> i16 {
    let mut v = 0i16;
    for &p in probs {
        v = (v << 1) | bd.read_bit(p) as i16;
    }
    v
}

fn read_coeff(bd: &mut BoolDecoder, probs: &[u8; 11], skip_eob: bool) -> (i16, bool) {
    // Token tree (RFC 6386 §13.2)
    // After a DCT_0 (zero) token, the EOB check is skipped because the
    // encoder guarantees that EOB never follows zero — it would have
    // emitted EOB instead of zero if no non-zero coefficients remained.

    // Check EOB (skipped when previous token was zero)
    if !skip_eob {
        if !bd.read_bool(probs[0]) {
            return (0, true);
        }
    }
    // Check zero
    if !bd.read_bool(probs[1]) {
        return (0, false);
    }
    // Check literal 1
    if !bd.read_bool(probs[2]) {
        let sign = if bd.read_bool(128) { -1i16 } else { 1i16 };
        return (sign, false);
    }
    // Category split
    let val: i16;
    if !bd.read_bool(probs[3]) {
        // 2, 3, or 4
        if !bd.read_bool(probs[4]) {
            val = 2;
        } else if !bd.read_bool(probs[5]) {
            val = 3;
        } else {
            val = 4;
        }
    } else if !bd.read_bool(probs[6]) {
        // Category 1 (5..6) or Category 2 (7..10)
        if !bd.read_bool(probs[7]) {
            // Cat 1: base=5, 1 extra bit
            val = 5 + read_cat_extra(bd, &CAT1_PROB);
        } else {
            // Cat 2: base=7, 2 extra bits
            val = 7 + read_cat_extra(bd, &CAT2_PROB);
        }
    } else {
        // Category 3..6: binary tree, NOT a linear chain!
        // Node 8 (probs[8]): left→Node9 (Cat3/Cat4), right→Node10 (Cat5/Cat6)
        // Node 9 (probs[9]): left→Cat3, right→Cat4
        // Node 10 (probs[10]): left→Cat5, right→Cat6
        if !bd.read_bool(probs[8]) {
            // Left → Node 9 (Cat3 or Cat4)
            if !bd.read_bool(probs[9]) {
                val = 11 + read_cat_extra(bd, &CAT3_PROB);
            } else {
                val = 19 + read_cat_extra(bd, &CAT4_PROB);
            }
        } else {
            // Right → Node 10 (Cat5 or Cat6)
            if !bd.read_bool(probs[10]) {
                val = 35 + read_cat_extra(bd, &CAT5_PROB);
            } else {
                val = 67 + read_cat_extra(bd, &CAT6_PROB);
            }
        }
    }
    let sign = if bd.read_bool(128) { -val } else { val };
    (sign, false)
}

/// Decode one 4×4 block of DCT coefficients.
/// Returns `true` if any non-EOB token was decoded (including zero coefficients).
/// This matches the `has_coefficients` semantics of the VP8 reference decoder,
/// which is needed for correct above/left non-zero context propagation.
fn decode_block(bd: &mut BoolDecoder, coeffs: &mut [i16; 16],
                plane_type: usize, first_coeff: usize,
                coeff_probs: &[[[[u8; 11]; 3]; 8]; 4],
                above_nz: bool, left_nz: bool) -> bool {
    let ctx = above_nz as usize + left_nz as usize; // 0, 1, or 2
    let mut has_coefficients = false;
    let mut prev_token_ctx: usize = 0; // 0=zero/none, 1=one, 2=more
    let mut skip_eob = false; // Skip EOB check after a zero token (RFC 6386 §13.2)

    for i in first_coeff..16 {
        let band = COEFF_BANDS[i] as usize;
        let c = if i == first_coeff { ctx } else { prev_token_ctx };
        let c = c.min(2);
        let probs = &coeff_probs[plane_type][band][c];

        let (val, eob) = read_coeff(bd, probs, skip_eob);
        if eob { break; }
        has_coefficients = true;
        coeffs[ZIGZAG[i]] = val;
        let abs_val = val.unsigned_abs() as usize;
        if abs_val == 0 {
            prev_token_ctx = 0;
            skip_eob = true;
        } else if abs_val == 1 {
            prev_token_ctx = 1;
            skip_eob = false;
        } else {
            prev_token_ctx = 2;
            skip_eob = false;
        }
    }
    has_coefficients
}

// ── VP8 loop filter helpers (RFC 6386 §15) ──────────────────────────────────

#[inline] fn lf_c(v: i32) -> i32 { v.clamp(-128, 127) }
#[inline] fn lf_u2s(v: u8) -> i32 { v as i32 - 128 }
#[inline] fn lf_s2u(v: i32) -> u8 { (lf_c(v) + 128) as u8 }

/// Calculate filter parameters for a macroblock.
fn calc_filter_params(
    base_fl: u8, sharpness: u8,
    seg_enabled: bool, seg_abs_delta: bool, seg_filter: &[i32; 4], seg_id: usize,
    lf_adj: bool, ref_delta: &[i32; 4], mode_delta: &[i32; 4],
    is_4x4: bool,
) -> (u8, u8, u8) {
    let mut fl = base_fl as i32;
    if fl == 0 { return (0, 0, 0); }
    if seg_enabled {
        if seg_abs_delta { fl = seg_filter[seg_id]; }
        else { fl += seg_filter[seg_id]; }
    }
    fl = fl.clamp(0, 63);
    if lf_adj {
        fl += ref_delta[0];
        if is_4x4 { fl += mode_delta[0]; }
    }
    let fl = fl.clamp(0, 63) as u8;
    let mut il = fl;
    if sharpness > 0 {
        il >>= if sharpness > 4 { 2 } else { 1 };
        if il > 9 - sharpness { il = 9 - sharpness; }
    }
    if il == 0 { il = 1; }
    let hev = if fl >= 40 { 2 } else if fl >= 15 { 1 } else { 0 };
    (fl, il, hev)
}

// ── Simple filter (affects 2 pixels per edge) ──

fn lf_simple_threshold_h(limit: i32, p: &[u8]) -> bool {
    (p[3] as i32 - p[4] as i32).abs() * 2 + (p[2] as i32 - p[5] as i32).abs() / 2 <= limit
}
fn lf_simple_threshold_v(limit: i32, p: &[u8], pt: usize, s: usize) -> bool {
    (p[pt - s] as i32 - p[pt] as i32).abs() * 2 + (p[pt - 2*s] as i32 - p[pt + s] as i32).abs() / 2 <= limit
}

fn lf_common_adjust_h(outer: bool, p: &mut [u8]) -> i32 {
    let (p1, p0, q0, q1) = (lf_u2s(p[2]), lf_u2s(p[3]), lf_u2s(p[4]), lf_u2s(p[5]));
    let o = if outer { lf_c(p1 - q1) } else { 0 };
    let a = lf_c(o + 3 * (q0 - p0));
    let b = lf_c(a + 3) >> 3;
    let a2 = lf_c(a + 4) >> 3;
    p[4] = lf_s2u(q0 - a2);
    p[3] = lf_s2u(p0 + b);
    a2
}
fn lf_common_adjust_v(outer: bool, p: &mut [u8], pt: usize, s: usize) -> i32 {
    let (p1, p0, q0, q1) = (lf_u2s(p[pt-2*s]), lf_u2s(p[pt-s]), lf_u2s(p[pt]), lf_u2s(p[pt+s]));
    let o = if outer { lf_c(p1 - q1) } else { 0 };
    let a = lf_c(o + 3 * (q0 - p0));
    let b = lf_c(a + 3) >> 3;
    let a2 = lf_c(a + 4) >> 3;
    p[pt] = lf_s2u(q0 - a2);
    p[pt - s] = lf_s2u(p0 + b);
    a2
}

fn lf_simple_h(limit: u8, p: &mut [u8]) {
    if lf_simple_threshold_h(limit as i32, p) { lf_common_adjust_h(true, p); }
}
fn lf_simple_v(limit: u8, p: &mut [u8], pt: usize, s: usize) {
    if lf_simple_threshold_v(limit as i32, p, pt, s) { lf_common_adjust_v(true, p, pt, s); }
}

// ── Normal filter threshold checks ──

fn lf_should_filter_h(il: u8, el: u8, p: &[u8]) -> bool {
    lf_simple_threshold_h(el as i32, p)
        && p[0].abs_diff(p[1]) <= il && p[1].abs_diff(p[2]) <= il && p[2].abs_diff(p[3]) <= il
        && p[7].abs_diff(p[6]) <= il && p[6].abs_diff(p[5]) <= il && p[5].abs_diff(p[4]) <= il
}
fn lf_should_filter_v(il: u8, el: u8, p: &[u8], pt: usize, s: usize) -> bool {
    lf_simple_threshold_v(el as i32, p, pt, s)
        && p[pt-4*s].abs_diff(p[pt-3*s]) <= il && p[pt-3*s].abs_diff(p[pt-2*s]) <= il
        && p[pt-2*s].abs_diff(p[pt-s]) <= il
        && p[pt+3*s].abs_diff(p[pt+2*s]) <= il && p[pt+2*s].abs_diff(p[pt+s]) <= il
        && p[pt+s].abs_diff(p[pt]) <= il
}
fn lf_hev_h(thr: u8, p: &[u8]) -> bool { p[2].abs_diff(p[3]) > thr || p[5].abs_diff(p[4]) > thr }
fn lf_hev_v(thr: u8, p: &[u8], pt: usize, s: usize) -> bool {
    p[pt-2*s].abs_diff(p[pt-s]) > thr || p[pt+s].abs_diff(p[pt]) > thr
}

// ── Macroblock edge filter (normal, up to 6 pixels) ──

fn lf_mb_h(hev_thr: u8, il: u8, el: u8, p: &mut [u8]) {
    if !lf_should_filter_h(il, el, p) { return; }
    if !lf_hev_h(hev_thr, p) {
        let (p2,p1,p0,q0,q1,q2) = (lf_u2s(p[1]),lf_u2s(p[2]),lf_u2s(p[3]),lf_u2s(p[4]),lf_u2s(p[5]),lf_u2s(p[6]));
        let w = lf_c(lf_c(p1-q1) + 3*(q0-p0));
        let a = lf_c((27*w+63)>>7); p[4]=lf_s2u(q0-a); p[3]=lf_s2u(p0+a);
        let a = lf_c((18*w+63)>>7); p[5]=lf_s2u(q1-a); p[2]=lf_s2u(p1+a);
        let a = lf_c((9*w+63)>>7);  p[6]=lf_s2u(q2-a); p[1]=lf_s2u(p2+a);
    } else {
        lf_common_adjust_h(true, p);
    }
}
fn lf_mb_v(hev_thr: u8, il: u8, el: u8, p: &mut [u8], pt: usize, s: usize) {
    if !lf_should_filter_v(il, el, p, pt, s) { return; }
    if !lf_hev_v(hev_thr, p, pt, s) {
        let (p2,p1,p0,q0,q1,q2) = (lf_u2s(p[pt-3*s]),lf_u2s(p[pt-2*s]),lf_u2s(p[pt-s]),lf_u2s(p[pt]),lf_u2s(p[pt+s]),lf_u2s(p[pt+2*s]));
        let w = lf_c(lf_c(p1-q1) + 3*(q0-p0));
        let a = lf_c((27*w+63)>>7); p[pt]=lf_s2u(q0-a); p[pt-s]=lf_s2u(p0+a);
        let a = lf_c((18*w+63)>>7); p[pt+s]=lf_s2u(q1-a); p[pt-2*s]=lf_s2u(p1+a);
        let a = lf_c((9*w+63)>>7);  p[pt+2*s]=lf_s2u(q2-a); p[pt-3*s]=lf_s2u(p2+a);
    } else {
        lf_common_adjust_v(true, p, pt, s);
    }
}

// ── Sub-block edge filter (normal, up to 4 pixels) ──

fn lf_sub_h(hev_thr: u8, il: u8, el: u8, p: &mut [u8]) {
    if !lf_should_filter_h(il, el, p) { return; }
    let hv = lf_hev_h(hev_thr, p);
    let a = (lf_common_adjust_h(hv, p) + 1) >> 1;
    if !hv { p[5] = lf_s2u(lf_u2s(p[5]) - a); p[2] = lf_s2u(lf_u2s(p[2]) + a); }
}
fn lf_sub_v(hev_thr: u8, il: u8, el: u8, p: &mut [u8], pt: usize, s: usize) {
    if !lf_should_filter_v(il, el, p, pt, s) { return; }
    let hv = lf_hev_v(hev_thr, p, pt, s);
    let a = (lf_common_adjust_v(hv, p, pt, s) + 1) >> 1;
    if !hv { p[pt+s] = lf_s2u(lf_u2s(p[pt+s]) - a); p[pt-2*s] = lf_s2u(lf_u2s(p[pt-2*s]) + a); }
}

// ── Main VP8 lossy decoder ──────────────────────────────────────────────────

#[allow(unused_variables)]
fn decode_vp8_lossy(data: &[u8], out: &mut [u32]) -> i32 {
    let hdr = match parse_vp8_frame_header(data) {
        Some(h) => h,
        None => return ERR_INVALID_DATA,
    };

    let w = hdr.width as usize;
    let h = hdr.height as usize;
    if w == 0 || h == 0 || w > 16384 || h > 16384 { return ERR_INVALID_DATA; }
    if w * h > out.len() { return ERR_BUFFER_TOO_SMALL; }

    let mb_w = (w + 15) / 16;
    let mb_h = (h + 15) / 16;
    let padded_w = mb_w * 16;
    let padded_h = mb_h * 16;

    // First partition starts after the 10-byte header
    let part1_start = 10;
    let part1_end = part1_start + hdr.first_part_size as usize;
    if part1_end > data.len() { return ERR_INVALID_DATA; }

    let mut bd = BoolDecoder::new(&data[part1_start..part1_end]);

    // ── Keyframe header (RFC 6386 §9.2-9.11) ────────────────────────────
    let color_space = bd.read_literal(1); // 0=YCbCr
    let clamping = bd.read_literal(1);

    // Segmentation
    let segmentation_enabled = bd.read_bool(128);
    let mut seg_quants = [0i32; 4];
    let mut seg_filter = [0i32; 4];
    let mut seg_probs = [255u8; MB_FEATURE_TREE_PROBS];
    let mut seg_update_map = false;
    let mut seg_abs_delta = false;
    if segmentation_enabled {
        seg_update_map = bd.read_bool(128);
        let update_data = bd.read_bool(128);
        if update_data {
            seg_abs_delta = bd.read_bool(128);
            for i in 0..4 {
                if bd.read_bool(128) {
                    seg_quants[i] = bd.read_signed(7);
                }
            }
            for i in 0..4 {
                if bd.read_bool(128) {
                    seg_filter[i] = bd.read_signed(6);
                }
            }
        }
        if seg_update_map {
            for i in 0..MB_FEATURE_TREE_PROBS {
                if bd.read_bool(128) {
                    seg_probs[i] = bd.read_literal(8) as u8;
                }
            }
        }
    }

    // Filter parameters
    let filter_type = bd.read_literal(1); // 0=normal, 1=simple
    let filter_level = bd.read_literal(6) as i32;
    let sharpness = bd.read_literal(3);

    // Loop filter adjustments
    let lf_adj_enable = bd.read_bool(128);
    let mut lf_ref_delta = [0i32; 4];
    let mut lf_mode_delta = [0i32; 4];
    if lf_adj_enable {
        let lf_adj_update = bd.read_bool(128);
        if lf_adj_update {
            for i in 0..4 {
                if bd.read_bool(128) { lf_ref_delta[i] = bd.read_signed(6); }
            }
            for i in 0..4 {
                if bd.read_bool(128) { lf_mode_delta[i] = bd.read_signed(6); }
            }
        }
    }

    // Token partitions
    let log2_nbr_of_partitions = bd.read_literal(2) as usize;
    let _nbr_partitions = 1usize << log2_nbr_of_partitions;

    // Quantization parameters
    let base_qp = bd.read_literal(7) as i32;
    let y_dc_delta  = if bd.read_bool(128) { bd.read_signed(4) } else { 0 };
    let y2_dc_delta = if bd.read_bool(128) { bd.read_signed(4) } else { 0 };
    let y2_ac_delta = if bd.read_bool(128) { bd.read_signed(4) } else { 0 };
    let uv_dc_delta = if bd.read_bool(128) { bd.read_signed(4) } else { 0 };
    let uv_ac_delta = if bd.read_bool(128) { bd.read_signed(4) } else { 0 };

    // Build per-segment quantization parameters
    let mut seg_quant_params = [
        build_quant(base_qp, y_dc_delta, y2_dc_delta, y2_ac_delta, uv_dc_delta, uv_ac_delta),
        build_quant(base_qp, y_dc_delta, y2_dc_delta, y2_ac_delta, uv_dc_delta, uv_ac_delta),
        build_quant(base_qp, y_dc_delta, y2_dc_delta, y2_ac_delta, uv_dc_delta, uv_ac_delta),
        build_quant(base_qp, y_dc_delta, y2_dc_delta, y2_ac_delta, uv_dc_delta, uv_ac_delta),
    ];
    if segmentation_enabled {
        for seg in 0..4 {
            let seg_qp = if seg_abs_delta {
                seg_quants[seg] // Absolute QP
            } else {
                base_qp + seg_quants[seg] // Delta from base
            };
            seg_quant_params[seg] = build_quant(
                seg_qp, y_dc_delta, y2_dc_delta, y2_ac_delta, uv_dc_delta, uv_ac_delta,
            );
        }
    }

    // Refresh entropy probs (not used for single-frame WebP)
    let _refresh_probs = bd.read_bool(128);

    // Coefficient probability updates
    let mut coeff_probs = DEFAULT_COEFF_PROBS;
    for i in 0..4 {
        for j in 0..8 {
            for k in 0..3 {
                for l in 0..11 {
                    if bd.read_bool(VP8_COEFF_UPDATE_PROBS[i][j][k][l]) {
                        coeff_probs[i][j][k][l] = bd.read_literal(8) as u8;
                    }
                }
            }
        }
    }

    // Skip-macroblock flag
    let mb_no_skip_coeff = bd.read_bool(128);
    let skip_prob = if mb_no_skip_coeff { bd.read_literal(8) as u8 } else { 0 };

    // ── Token partitions (RFC 6386 §9.5) ──────────────────────────────
    let nbr_partitions = _nbr_partitions;
    let token_start = part1_end;
    // Parse partition size table (3 bytes LE per partition, except last)
    let part_sizes_len = if log2_nbr_of_partitions > 0 {
        3 * (nbr_partitions - 1)
    } else { 0 };
    let sizes_start = token_start;
    if sizes_start + part_sizes_len > data.len() { return ERR_INVALID_DATA; }

    // Compute start offset of each token partition
    let mut part_offsets = Vec::with_capacity(nbr_partitions);
    let mut part_sizes_vec = Vec::with_capacity(nbr_partitions);
    let mut off = token_start + part_sizes_len;
    for i in 0..nbr_partitions {
        part_offsets.push(off);
        if i < nbr_partitions - 1 {
            let si = sizes_start + i * 3;
            let sz = data[si] as usize
                | ((data[si + 1] as usize) << 8)
                | ((data[si + 2] as usize) << 16);
            part_sizes_vec.push(sz);
            off += sz;
        } else {
            // Last partition extends to end of data
            part_sizes_vec.push(data.len().saturating_sub(off));
        }
    }

    // Create a BoolDecoder for each token partition
    let mut token_decoders: Vec<BoolDecoder> = Vec::with_capacity(nbr_partitions);
    for i in 0..nbr_partitions {
        let start = part_offsets[i];
        let end = (start + part_sizes_vec[i]).min(data.len());
        if start >= data.len() {
            token_decoders.push(BoolDecoder::new(&[]));
        } else {
            token_decoders.push(BoolDecoder::new(&data[start..end]));
        }
    }

    // ── Allocate plane buffers (YUV 4:2:0) ──────────────────────────────
    let y_stride = padded_w;
    let uv_stride = padded_w / 2;
    let mut y_plane = vec![128u8; padded_w * padded_h];
    let mut u_plane = vec![128u8; (padded_w / 2) * (padded_h / 2)];
    let mut v_plane = vec![128u8; (padded_w / 2) * (padded_h / 2)];

    // Per-MB info for loop filter (filled during decode, used in post-processing)
    struct MbInfo { seg_id: usize, is_4x4: bool, is_skip: bool, has_nonzero: bool }
    let mut mb_info = Vec::with_capacity(mb_w * mb_h);
    for _ in 0..mb_w * mb_h {
        mb_info.push(MbInfo { seg_id: 0, is_4x4: false, is_skip: false, has_nonzero: false });
    }

    // Above non-zero flags for coefficient context
    let mut above_nz_y = vec![false; mb_w * 4];
    let mut above_nz_u = vec![false; mb_w * 2];
    let mut above_nz_v = vec![false; mb_w * 2];
    let mut above_nz_dc = vec![false; mb_w];

    // Above and left sub-block modes (for 4x4 intra prediction context)
    let mut above_modes = vec![B_DC_PRED; mb_w * 4];

    // ── Decode macroblocks ──────────────────────────────────────────────
    for mb_y in 0..mb_h {
        // Select the token partition for this macroblock row (round-robin)
        let tp = mb_y % nbr_partitions;

        let mut left_nz_y = [false; 4];
        let mut left_nz_u = [false; 2];
        let mut left_nz_v = [false; 2];
        let mut left_nz_dc = false;
        let mut left_modes = [B_DC_PRED; 4];

        for mb_x in 0..mb_w {
            // Read segment ID (if segmentation)
            let seg_id = if segmentation_enabled && seg_update_map {
                if !bd.read_bool(seg_probs[0]) {
                    if !bd.read_bool(seg_probs[1]) { 0 } else { 1 }
                } else {
                    if !bd.read_bool(seg_probs[2]) { 2 } else { 3 }
                }
            } else { 0usize };

            // Skip flag
            let is_skip = if mb_no_skip_coeff { bd.read_bool(skip_prob) } else { false };

            // Read Y prediction mode (keyframe)
            let y_mode = read_kf_y_mode(&mut bd);
            let is_4x4 = y_mode == 4; // B_PRED mode

            // Store per-MB info for loop filter
            mb_info[mb_y * mb_w + mb_x] = MbInfo { seg_id, is_4x4, is_skip, has_nonzero: false };

            // Sub-block modes for intra 4x4
            let mut sub_modes = [[B_DC_PRED; 4]; 4]; // [row][col]
            if is_4x4 {
                for sy in 0..4 {
                    for sx in 0..4 {
                        let above_mode = if sy == 0 { above_modes[mb_x * 4 + sx] } else { sub_modes[sy - 1][sx] };
                        let left_mode = if sx == 0 { left_modes[sy] } else { sub_modes[sy][sx - 1] };
                        sub_modes[sy][sx] = read_kf_bmode(&mut bd, above_mode, left_mode);
                    }
                }
                // Update context
                for sx in 0..4 { above_modes[mb_x * 4 + sx] = sub_modes[3][sx]; }
                for sy in 0..4 { left_modes[sy] = sub_modes[sy][3]; }
            } else {
                let fill = match y_mode {
                    0 => B_DC_PRED,
                    1 => B_VE_PRED,
                    2 => B_HE_PRED,
                    3 => B_TM_PRED,
                    _ => B_DC_PRED,
                };
                for sx in 0..4 { above_modes[mb_x * 4 + sx] = fill; }
                for sy in 0..4 { left_modes[sy] = fill; }
            }

            // UV mode
            let uv_mode = read_kf_uv_mode(&mut bd);

            // ── Predict Y plane ─────────────────────────────────────────
            let y_off = mb_y * 16 * y_stride + mb_x * 16;
            let uv_off = mb_y * 8 * uv_stride + mb_x * 8;

            if is_4x4 {
                // 4x4 intra prediction per sub-block
                for sy in 0..4 {
                    for sx in 0..4 {
                        let bx = mb_x * 16 + sx * 4;
                        let by = mb_y * 16 + sy * 4;
                        let off = by * y_stride + bx;

                        // Gather above (8 pixels: 4 above + 4 above-right)
                        let mut above_pixels = [128u8; 8];
                        if by > 0 {
                            for i in 0..4 {
                                above_pixels[i] = y_plane[(by - 1) * y_stride + bx + i];
                            }
                            // Above-right
                            if bx + 4 < padded_w {
                                for i in 0..4 {
                                    above_pixels[4 + i] = y_plane[(by - 1) * y_stride + bx + 4 + i];
                                }
                            } else {
                                let fill = above_pixels[3];
                                for i in 0..4 { above_pixels[4 + i] = fill; }
                            }
                        }
                        let mut left_pixels = [128u8; 4];
                        if bx > 0 {
                            for i in 0..4 {
                                left_pixels[i] = y_plane[(by + i) * y_stride + bx - 1];
                            }
                        }
                        let tl_pixel = if bx > 0 && by > 0 {
                            y_plane[(by - 1) * y_stride + bx - 1]
                        } else { 128 };

                        predict_4x4(sub_modes[sy][sx], &mut y_plane[off..], y_stride,
                                    &above_pixels, &left_pixels, tl_pixel);
                    }
                }
            } else {
                // 16x16 intra prediction
                let mut above16 = [128u8; 16];
                if mb_y > 0 {
                    for i in 0..16 {
                        above16[i] = y_plane[(mb_y * 16 - 1) * y_stride + mb_x * 16 + i];
                    }
                }
                let mut left16 = [128u8; 16];
                if mb_x > 0 {
                    for i in 0..16 {
                        left16[i] = y_plane[(mb_y * 16 + i) * y_stride + mb_x * 16 - 1];
                    }
                }
                let tl16 = if mb_x > 0 && mb_y > 0 {
                    y_plane[(mb_y * 16 - 1) * y_stride + mb_x * 16 - 1]
                } else { 128 };

                predict_16x16(y_mode as u8, &mut y_plane[y_off..], y_stride,
                             &above16, &left16, tl16);
            }

            // ── Predict UV planes ───────────────────────────────────────
            {
                let mut above_u = [128u8; 8];
                let mut above_v = [128u8; 8];
                let mut left_u = [128u8; 8];
                let mut left_v = [128u8; 8];
                let mut tl_u = 128u8;
                let mut tl_v = 128u8;
                if mb_y > 0 {
                    for i in 0..8 {
                        above_u[i] = u_plane[(mb_y * 8 - 1) * uv_stride + mb_x * 8 + i];
                        above_v[i] = v_plane[(mb_y * 8 - 1) * uv_stride + mb_x * 8 + i];
                    }
                }
                if mb_x > 0 {
                    for i in 0..8 {
                        left_u[i] = u_plane[(mb_y * 8 + i) * uv_stride + mb_x * 8 - 1];
                        left_v[i] = v_plane[(mb_y * 8 + i) * uv_stride + mb_x * 8 - 1];
                    }
                }
                if mb_x > 0 && mb_y > 0 {
                    tl_u = u_plane[(mb_y * 8 - 1) * uv_stride + mb_x * 8 - 1];
                    tl_v = v_plane[(mb_y * 8 - 1) * uv_stride + mb_x * 8 - 1];
                }
                predict_8x8(uv_mode as u8, &mut u_plane[uv_off..], uv_stride,
                           &above_u, &left_u, tl_u);
                predict_8x8(uv_mode as u8, &mut v_plane[uv_off..], uv_stride,
                           &above_v, &left_v, tl_v);
            }

            // ── Decode residuals ────────────────────────────────────────
            if !is_skip {
                if !is_4x4 {
                    // Y2 block (16x16 mode): decode DC coefficients via WHT
                    let mut y2_coeffs = [0i16; 16];
                    let nz = decode_block(&mut token_decoders[tp], &mut y2_coeffs, 1, 0,
                                         &coeff_probs, above_nz_dc[mb_x], left_nz_dc);
                    above_nz_dc[mb_x] = nz;
                    left_nz_dc = nz;

                    // Dequantize Y2
                    y2_coeffs[0] = y2_coeffs[0].wrapping_mul(seg_quant_params[seg_id].y2_dc);
                    for i in 1..16 { y2_coeffs[i] = y2_coeffs[i].wrapping_mul(seg_quant_params[seg_id].y2_ac); }

                    // Inverse WHT → 16 DC values
                    let mut dc16 = [0i16; 16];
                    iwht4x4(&y2_coeffs, &mut dc16);

                    // Decode 16 Y sub-blocks (AC only, DC from WHT)
                    for sb in 0..16 {
                        let sy = sb / 4;
                        let sx = sb % 4;
                        let mut coeffs = [0i16; 16];
                        coeffs[0] = dc16[sb]; // DC from WHT
                        let nz = decode_block(&mut token_decoders[tp], &mut coeffs, 0, 1,
                                             &coeff_probs,
                                             above_nz_y[mb_x * 4 + sx],
                                             left_nz_y[sy]);
                        // Context propagation uses only AC decode result (not WHT DC).
                        // RFC 6386: the non-zero context for the next sub-block is based
                        // on whether THIS sub-block's AC coefficients were non-zero.
                        above_nz_y[mb_x * 4 + sx] = nz;
                        left_nz_y[sy] = nz;

                        // Dequantize
                        // DC already dequantized via WHT path
                        for i in 1..16 { coeffs[i] = coeffs[i].wrapping_mul(seg_quant_params[seg_id].y_ac); }

                        // Inverse DCT
                        let bx = mb_x * 16 + sx * 4;
                        let by = mb_y * 16 + sy * 4;
                        idct4x4(&coeffs, &mut y_plane[by * y_stride + bx..], y_stride);

                    }
                } else {
                    // 4x4 mode: no Y2 block, decode each sub-block with DC
                    above_nz_dc[mb_x] = false;
                    left_nz_dc = false;

                    for sb in 0..16 {
                        let sy = sb / 4;
                        let sx = sb % 4;
                        let mut coeffs = [0i16; 16];
                        let nz = decode_block(&mut token_decoders[tp], &mut coeffs, 3, 0,
                                             &coeff_probs,
                                             above_nz_y[mb_x * 4 + sx],
                                             left_nz_y[sy]);
                        above_nz_y[mb_x * 4 + sx] = nz;
                        left_nz_y[sy] = nz;

                        // Dequantize
                        coeffs[0] = coeffs[0].wrapping_mul(seg_quant_params[seg_id].y_dc);
                        for i in 1..16 { coeffs[i] = coeffs[i].wrapping_mul(seg_quant_params[seg_id].y_ac); }

                        let bx = mb_x * 16 + sx * 4;
                        let by = mb_y * 16 + sy * 4;
                        idct4x4(&coeffs, &mut y_plane[by * y_stride + bx..], y_stride);
                    }
                }

                // U sub-blocks (4 blocks, 4x4 each)
                for sb in 0..4 {
                    let sy = sb / 2;
                    let sx = sb % 2;
                    let mut coeffs = [0i16; 16];
                    let nz = decode_block(&mut token_decoders[tp], &mut coeffs, 2, 0,
                                         &coeff_probs,
                                         above_nz_u[mb_x * 2 + sx],
                                         left_nz_u[sy]);
                    above_nz_u[mb_x * 2 + sx] = nz;
                    left_nz_u[sy] = nz;

                    coeffs[0] = coeffs[0].wrapping_mul(seg_quant_params[seg_id].uv_dc);
                    for i in 1..16 { coeffs[i] = coeffs[i].wrapping_mul(seg_quant_params[seg_id].uv_ac); }

                    let bx = mb_x * 8 + sx * 4;
                    let by = mb_y * 8 + sy * 4;
                    idct4x4(&coeffs, &mut u_plane[by * uv_stride + bx..], uv_stride);
                }

                // V sub-blocks
                for sb in 0..4 {
                    let sy = sb / 2;
                    let sx = sb % 2;
                    let mut coeffs = [0i16; 16];
                    let nz = decode_block(&mut token_decoders[tp], &mut coeffs, 2, 0,
                                         &coeff_probs,
                                         above_nz_v[mb_x * 2 + sx],
                                         left_nz_v[sy]);
                    above_nz_v[mb_x * 2 + sx] = nz;
                    left_nz_v[sy] = nz;

                    coeffs[0] = coeffs[0].wrapping_mul(seg_quant_params[seg_id].uv_dc);
                    for i in 1..16 { coeffs[i] = coeffs[i].wrapping_mul(seg_quant_params[seg_id].uv_ac); }

                    let bx = mb_x * 8 + sx * 4;
                    let by = mb_y * 8 + sy * 4;
                    idct4x4(&coeffs, &mut v_plane[by * uv_stride + bx..], uv_stride);
                }
            } else {
                // Skip: clear non-zero flags
                for sx in 0..4 { above_nz_y[mb_x * 4 + sx] = false; }
                for sy in 0..4 { left_nz_y[sy] = false; }
                for sx in 0..2 { above_nz_u[mb_x * 2 + sx] = false; }
                for sy in 0..2 { left_nz_u[sy] = false; }
                for sx in 0..2 { above_nz_v[mb_x * 2 + sx] = false; }
                for sy in 0..2 { left_nz_v[sy] = false; }
                above_nz_dc[mb_x] = false;
                left_nz_dc = false;
            }
            // Update loop filter info: if not skipped, mark has_nonzero
            mb_info[mb_y * mb_w + mb_x].has_nonzero = !is_skip;
        }
    }

    // ── VP8 loop filter (deblocking, RFC 6386 §15) ────────────────────
    if filter_level > 0 {
        // Calculate filter parameters per MB and apply.
        // We stored per-MB info during decode: mb_info[mb_y * mb_w + mb_x].
        for mby in 0..mb_h {
            for mbx in 0..mb_w {
                let mi = &mb_info[mby * mb_w + mbx];
                let (fl, il, hev_thr) = calc_filter_params(
                    filter_level as u8, sharpness as u8,
                    segmentation_enabled, seg_abs_delta,
                    &seg_filter, mi.seg_id,
                    lf_adj_enable, &lf_ref_delta, &lf_mode_delta,
                    mi.is_4x4,
                );
                if fl == 0 { continue; }
                let fl = fl as u8;
                let mbedge_limit = (fl as u16 + 2) * 2 + il as u16;
                let sub_bedge_limit = fl as u16 * 2 + il as u16;
                let mbedge_limit = mbedge_limit.min(255) as u8;
                let sub_bedge_limit = sub_bedge_limit.min(255) as u8;
                let do_sub = mi.is_4x4 || (!mi.is_skip && mi.has_nonzero);

                // ── Left MB edge (horizontal filter on vertical edge)
                if mbx > 0 {
                    if filter_type == 1 {
                        for y in 0..16 {
                            let off = (mby * 16 + y) * y_stride + mbx * 16;
                            lf_simple_h(mbedge_limit, &mut y_plane[off - 4..off + 4]);
                        }
                    } else {
                        for y in 0..16 {
                            let off = (mby * 16 + y) * y_stride + mbx * 16;
                            lf_mb_h(hev_thr, il, mbedge_limit, &mut y_plane[off - 4..off + 4]);
                        }
                        for y in 0..8 {
                            let off = (mby * 8 + y) * uv_stride + mbx * 8;
                            lf_mb_h(hev_thr, il, mbedge_limit, &mut u_plane[off - 4..off + 4]);
                            lf_mb_h(hev_thr, il, mbedge_limit, &mut v_plane[off - 4..off + 4]);
                        }
                    }
                }

                // ── Internal vertical sub-block edges
                if do_sub {
                    if filter_type == 1 {
                        for x in (4..13).step_by(4) {
                            for y in 0..16 {
                                let off = (mby * 16 + y) * y_stride + mbx * 16 + x;
                                lf_simple_h(sub_bedge_limit, &mut y_plane[off - 4..off + 4]);
                            }
                        }
                    } else {
                        for x in (4..13).step_by(4) {
                            for y in 0..16 {
                                let off = (mby * 16 + y) * y_stride + mbx * 16 + x;
                                lf_sub_h(hev_thr, il, sub_bedge_limit, &mut y_plane[off - 4..off + 4]);
                            }
                        }
                        for y in 0..8 {
                            let off = (mby * 8 + y) * uv_stride + mbx * 8 + 4;
                            lf_sub_h(hev_thr, il, sub_bedge_limit, &mut u_plane[off - 4..off + 4]);
                            lf_sub_h(hev_thr, il, sub_bedge_limit, &mut v_plane[off - 4..off + 4]);
                        }
                    }
                }

                // ── Top MB edge (vertical filter on horizontal edge)
                if mby > 0 {
                    if filter_type == 1 {
                        for x in 0..16 {
                            let pt = mby * 16 * y_stride + mbx * 16 + x;
                            lf_simple_v(mbedge_limit, &mut y_plane, pt, y_stride);
                        }
                    } else {
                        for x in 0..16 {
                            let pt = mby * 16 * y_stride + mbx * 16 + x;
                            lf_mb_v(hev_thr, il, mbedge_limit, &mut y_plane, pt, y_stride);
                        }
                        for x in 0..8 {
                            let pt = mby * 8 * uv_stride + mbx * 8 + x;
                            lf_mb_v(hev_thr, il, mbedge_limit, &mut u_plane, pt, uv_stride);
                            lf_mb_v(hev_thr, il, mbedge_limit, &mut v_plane, pt, uv_stride);
                        }
                    }
                }

                // ── Internal horizontal sub-block edges
                if do_sub {
                    if filter_type == 1 {
                        for y in (4..13).step_by(4) {
                            for x in 0..16 {
                                let pt = (mby * 16 + y) * y_stride + mbx * 16 + x;
                                lf_simple_v(sub_bedge_limit, &mut y_plane, pt, y_stride);
                            }
                        }
                    } else {
                        for y in (4..13).step_by(4) {
                            for x in 0..16 {
                                let pt = (mby * 16 + y) * y_stride + mbx * 16 + x;
                                lf_sub_v(hev_thr, il, sub_bedge_limit, &mut y_plane, pt, y_stride);
                            }
                        }
                        for x in 0..8 {
                            let pt = (mby * 8 + 4) * uv_stride + mbx * 8 + x;
                            lf_sub_v(hev_thr, il, sub_bedge_limit, &mut u_plane, pt, uv_stride);
                            lf_sub_v(hev_thr, il, sub_bedge_limit, &mut v_plane, pt, uv_stride);
                        }
                    }
                }
            }
        }
    }

    // ── YUV 4:2:0 → ARGB conversion (libwebp formula) ─────────────────
    for py in 0..h {
        for px in 0..w {
            let y = y_plane[py * y_stride + px] as u32;
            let u = u_plane[(py / 2) * uv_stride + (px / 2)] as u32;
            let v = v_plane[(py / 2) * uv_stride + (px / 2)] as u32;

            // mulhi(a,b) = (a * b) >> 8, clip(x) = (x >> 6).clamp(0,255)
            let yc = (y * 19077) >> 8;
            let r = ((yc + ((v * 26149) >> 8)).wrapping_sub(14234) as i32 >> 6).clamp(0, 255) as u32;
            let g = ((yc.wrapping_sub((u * 6419) >> 8).wrapping_sub((v * 13320) >> 8)).wrapping_add(8708) as i32 >> 6).clamp(0, 255) as u32;
            let b = ((yc + ((u * 33050) >> 8)).wrapping_sub(17685) as i32 >> 6).clamp(0, 255) as u32;

            out[py * w + px] = 0xFF000000 | (r << 16) | (g << 8) | b;
        }
    }

    ERR_OK
}

// ── Keyframe mode reading (RFC 6386 §11.3) ──────────────────────────────────

fn read_kf_y_mode(bd: &mut BoolDecoder) -> u8 {
    // Tree from RFC 6386 §11.2:
    //   [-B_PRED, 2, 4, 6, -DC_PRED, -V_PRED, -H_PRED, -TM_PRED]
    // Node 0 (prob[0]=145): 0→B_PRED, 1→node1
    // Node 1 (prob[1]=156): 0→node2(DC/V), 1→node3(H/TM)
    // Node 2 (prob[2]=163): 0→DC, 1→V
    // Node 3 (prob[3]=128): 0→H, 1→TM
    if !bd.read_bool(KF_Y_MODE_PROBS[0]) { return 4; } // B_PRED
    if !bd.read_bool(KF_Y_MODE_PROBS[1]) {
        // Left subtree: DC or V
        if !bd.read_bool(KF_Y_MODE_PROBS[2]) { return DC_PRED; }
        return V_PRED;
    }
    // Right subtree: H or TM
    if !bd.read_bool(KF_Y_MODE_PROBS[3]) { return H_PRED; }
    TM_PRED
}

fn read_kf_uv_mode(bd: &mut BoolDecoder) -> u8 {
    if !bd.read_bool(KF_UV_MODE_PROBS[0]) { return DC_PRED; }
    if !bd.read_bool(KF_UV_MODE_PROBS[1]) { return V_PRED; }
    if !bd.read_bool(KF_UV_MODE_PROBS[2]) { return H_PRED; }
    TM_PRED
}

fn read_kf_bmode(bd: &mut BoolDecoder, above: u8, left: u8) -> u8 {
    let probs = &KF_BMODE_PROBS[above as usize % 10][left as usize % 10];
    // Sub-block mode tree
    if !bd.read_bool(probs[0]) { return B_DC_PRED; }
    if !bd.read_bool(probs[1]) { return B_TM_PRED; }
    if !bd.read_bool(probs[2]) { return B_VE_PRED; }
    if !bd.read_bool(probs[3]) {
        if !bd.read_bool(probs[4]) { return B_HE_PRED; }
        if !bd.read_bool(probs[5]) { return B_RD_PRED; }
        return B_VR_PRED;
    }
    if !bd.read_bool(probs[6]) { return B_LD_PRED; }
    if !bd.read_bool(probs[7]) { return B_VL_PRED; }
    if !bd.read_bool(probs[8]) { return B_HD_PRED; }
    B_HU_PRED
}

// ── Coefficient update probabilities (RFC 6386 §13.4) ───────────────────────

static VP8_COEFF_UPDATE_PROBS: [[[[u8; 11]; 3]; 8]; 4] = [
    [
        [[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[176,246,255,255,255,255,255,255,255,255,255],[223,241,252,255,255,255,255,255,255,255,255],[249,253,253,255,255,255,255,255,255,255,255]],
        [[255,244,252,255,255,255,255,255,255,255,255],[234,254,254,255,255,255,255,255,255,255,255],[253,255,255,255,255,255,255,255,255,255,255]],
        [[255,246,254,255,255,255,255,255,255,255,255],[239,253,254,255,255,255,255,255,255,255,255],[254,255,254,255,255,255,255,255,255,255,255]],
        [[255,248,254,255,255,255,255,255,255,255,255],[251,255,254,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,253,254,255,255,255,255,255,255,255,255],[251,254,254,255,255,255,255,255,255,255,255],[254,255,254,255,255,255,255,255,255,255,255]],
        [[255,254,253,255,254,255,255,255,255,255,255],[250,255,254,255,254,255,255,255,255,255,255],[254,255,255,255,255,255,255,255,255,255,255]],
        [[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
    ],
    [
        [[217,255,255,255,255,255,255,255,255,255,255],[225,252,241,253,255,255,254,255,255,255,255],[234,250,241,250,253,255,253,254,255,255,255]],
        [[255,254,255,255,255,255,255,255,255,255,255],[223,254,254,255,255,255,255,255,255,255,255],[238,253,254,254,255,255,255,255,255,255,255]],
        [[255,248,254,255,255,255,255,255,255,255,255],[249,254,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,253,255,255,255,255,255,255,255,255,255],[247,254,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,253,254,255,255,255,255,255,255,255,255],[252,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,254,254,255,255,255,255,255,255,255,255],[253,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,254,253,255,255,255,255,255,255,255,255],[250,255,255,255,255,255,255,255,255,255,255],[254,255,255,255,255,255,255,255,255,255,255]],
        [[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
    ],
    [
        [[186,251,250,255,255,255,255,255,255,255,255],[234,251,244,254,255,255,255,255,255,255,255],[251,251,243,253,254,255,254,255,255,255,255]],
        [[255,253,254,255,255,255,255,255,255,255,255],[236,253,254,255,255,255,255,255,255,255,255],[251,253,253,254,254,255,255,255,255,255,255]],
        [[255,254,254,255,255,255,255,255,255,255,255],[254,254,254,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,254,255,255,255,255,255,255,255,255,255],[254,254,254,255,255,255,255,255,255,255,255],[254,255,254,255,255,255,255,255,255,255,255]],
        [[255,255,255,255,255,255,255,255,255,255,255],[254,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
    ],
    [
        [[248,255,255,255,255,255,255,255,255,255,255],[250,254,252,254,255,255,255,255,255,255,255],[248,254,249,253,255,255,255,255,255,255,255]],
        [[255,253,253,255,255,255,255,255,255,255,255],[246,253,253,255,255,255,255,255,255,255,255],[252,254,251,254,254,255,255,255,255,255,255]],
        [[255,254,252,255,255,255,255,255,255,255,255],[248,254,253,255,255,255,255,255,255,255,255],[253,255,254,254,255,255,255,255,255,255,255]],
        [[255,251,254,255,255,255,255,255,255,255,255],[245,251,254,255,255,255,255,255,255,255,255],[253,253,254,255,255,255,255,255,255,255,255]],
        [[255,251,253,255,255,255,255,255,255,255,255],[252,253,254,255,255,255,255,255,255,255,255],[255,254,255,255,255,255,255,255,255,255,255]],
        [[255,252,255,255,255,255,255,255,255,255,255],[249,255,254,255,255,255,255,255,255,255,255],[255,255,254,255,255,255,255,255,255,255,255]],
        [[255,255,253,255,255,255,255,255,255,255,255],[250,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
        [[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255],[255,255,255,255,255,255,255,255,255,255,255]],
    ],
];

// ── Alpha chunk decoder (VP8X extended format) ──────────────────────────────

fn apply_alpha_chunk(data: &[u8], pixels: &mut [u32], width: usize, height: usize) -> i32 {
    if data.is_empty() { return ERR_INVALID_DATA; }
    if width == 0 || height == 0 { return ERR_INVALID_DATA; }
    let pixel_count = width.saturating_mul(height);
    if pixel_count == 0 || pixel_count > pixels.len() { return ERR_BUFFER_TOO_SMALL; }

    let _pre_processing = (data[0] >> 0) & 3;
    let filter_method   = (data[0] >> 2) & 3;
    let compression     = (data[0] >> 4) & 3;

    if filter_method != 0 {
        return ERR_UNSUPPORTED;
    }

    let alpha_data = &data[1..];

    if compression == 0 {
        // Uncompressed alpha
        if alpha_data.len() < pixel_count {
            return ERR_INVALID_DATA;
        }
        for i in 0..pixel_count {
            pixels[i] = (pixels[i] & 0x00FFFFFF) | ((alpha_data[i] as u32) << 24);
        }
        ERR_OK
    } else {
        if compression != 1 {
            return ERR_UNSUPPORTED;
        }

        // Compressed alpha uses a headerless VP8L image stream with implicit
        // dimensions. The actual alpha values are stored in the decoded green
        // channel.
        let mut alpha_pixels = vec![0u32; pixel_count];
        let rc = decode_vp8l_image_stream(alpha_data, width, height, &mut alpha_pixels);
        if rc != ERR_OK {
            return rc;
        }

        for i in 0..pixel_count {
            let alpha = (alpha_pixels[i] >> 8) & 0xFF;
            pixels[i] = (pixels[i] & 0x00FFFFFF) | (alpha << 24);
        }
        ERR_OK
    }
}
