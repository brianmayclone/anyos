// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Minimal zlib/DEFLATE compressor for VNC Zlib encoding (type 6).
//!
//! Implements fixed-Huffman DEFLATE with LZ77 matching and a persistent
//! zlib stream (adler-32 state carried across calls, SYNC_FLUSH between
//! rectangles so the client can decompress incrementally).
//!
//! Adapted from libs/libzip/src/deflate.rs.

use anyos_std::Vec;

// ─── Constants ──────────────────────────────────────────────────────────────

const HASH_SIZE: usize = 4096;
const HASH_MASK: usize = HASH_SIZE - 1;
const MAX_MATCH: usize = 258;
const MIN_MATCH: usize = 3;
const WINDOW_SIZE: usize = 32768;
const MAX_CHAIN: usize = 24; // max hash chain depth (speed vs ratio tradeoff)

// ─── Fixed Huffman tables ───────────────────────────────────────────────────

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13,
    15, 17, 19, 23, 27, 31, 35, 43, 51, 59,
    67, 83, 99, 115, 131, 163, 195, 227, 258,
];

const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1,
    1, 1, 2, 2, 2, 2, 3, 3, 3, 3,
    4, 4, 4, 4, 5, 5, 5, 5, 0,
];

const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25,
    33, 49, 65, 97, 129, 193, 257, 385, 513, 769,
    1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3,
    4, 4, 5, 5, 6, 6, 7, 7, 8, 8,
    9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];

// ─── Bit helpers ────────────────────────────────────────────────────────────

fn reverse_bits(value: u32, bits: u8) -> u32 {
    let mut result = 0u32;
    let mut v = value;
    for _ in 0..bits {
        result = (result << 1) | (v & 1);
        v >>= 1;
    }
    result
}

fn find_length_code(length: u16) -> (u16, u8, u16) {
    for i in (0..29).rev() {
        if length >= LENGTH_BASE[i] {
            return (257 + i as u16, LENGTH_EXTRA[i], length - LENGTH_BASE[i]);
        }
    }
    (257, 0, 0)
}

fn find_distance_code(dist: u16) -> (u8, u8, u16) {
    for i in (0..30).rev() {
        if dist >= DIST_BASE[i] {
            return (i as u8, DIST_EXTRA[i], dist - DIST_BASE[i]);
        }
    }
    (0, 0, 0)
}

fn hash3(data: &[u8], pos: usize) -> usize {
    if pos + 2 >= data.len() { return 0; }
    let h = (data[pos] as usize)
        ^ ((data[pos + 1] as usize) << 4)
        ^ ((data[pos + 2] as usize) << 8);
    h & HASH_MASK
}

// ─── Persistent Zlib Encoder ────────────────────────────────────────────────

/// Persistent zlib encoder for VNC sessions.
///
/// Maintains the adler-32 checksum across calls so the zlib stream is
/// continuous.  Each `compress()` call emits a non-final DEFLATE block
/// followed by a SYNC_FLUSH marker.
pub struct ZlibEncoder {
    adler_a: u32,
    adler_b: u32,
    header_written: bool,
    // Persistent hash tables (allocated once, reset per call)
    head: Vec<u32>,
    prev: Vec<u32>,
    // Bit buffer carried across calls (flushed at SYNC_FLUSH)
    bit_buf: u32,
    bit_count: u8,
}

impl ZlibEncoder {
    pub fn new() -> Self {
        ZlibEncoder {
            adler_a: 1,
            adler_b: 0,
            header_written: false,
            head: anyos_std::vec![u32::MAX; HASH_SIZE],
            prev: anyos_std::vec![u32::MAX; WINDOW_SIZE],
            bit_buf: 0,
            bit_count: 0,
        }
    }

    /// Compress `input` bytes, appending compressed output to `out`.
    ///
    /// Writes the 2-byte zlib header on the first call.  Each call emits
    /// a non-final DEFLATE block (fixed Huffman + LZ77) followed by a
    /// SYNC_FLUSH so the VNC client can decompress immediately.
    pub fn compress(&mut self, input: &[u8], out: &mut Vec<u8>) {
        // Zlib header (once)
        if !self.header_written {
            out.push(0x78); // CMF: deflate, window=32K
            out.push(0x01); // FLG: no dict, fastest (0x78*256+0x01 = 30721 % 31 == 0)
            self.header_written = true;
        }

        // Update adler-32
        for &b in input {
            self.adler_a = (self.adler_a + b as u32) % 65521;
            self.adler_b = (self.adler_b + self.adler_a) % 65521;
        }

        // Reset hash chains (fresh dictionary per call — simpler than
        // maintaining a sliding window across calls, still effective)
        for x in self.head.iter_mut() { *x = u32::MAX; }
        for x in self.prev.iter_mut() { *x = u32::MAX; }

        // ── DEFLATE block: BFINAL=0, BTYPE=01 (fixed Huffman) ──

        self.write_bits(out, 0, 1); // BFINAL = 0 (non-final)
        self.write_bits(out, 1, 2); // BTYPE  = 01 (fixed Huffman)

        let mut pos = 0;
        while pos < input.len() {
            let (match_len, match_dist) = self.find_match(input, pos);

            if match_len >= MIN_MATCH {
                // Emit length/distance pair
                let (len_code, len_extra_bits, len_extra_val) =
                    find_length_code(match_len as u16);
                self.emit_lit_len(out, len_code);
                if len_extra_bits > 0 {
                    self.write_bits(out, len_extra_val as u32, len_extra_bits);
                }

                let (dist_code, dist_extra_bits, dist_extra_val) =
                    find_distance_code(match_dist as u16);
                self.emit_distance(out, dist_code);
                if dist_extra_bits > 0 {
                    self.write_bits(out, dist_extra_val as u32, dist_extra_bits);
                }

                // Update hash for matched positions
                for i in 0..match_len {
                    self.update_hash(input, pos + i);
                }
                pos += match_len;
            } else {
                // Emit literal byte
                self.emit_lit_len(out, input[pos] as u16);
                self.update_hash(input, pos);
                pos += 1;
            }
        }

        // End-of-block symbol
        self.emit_lit_len(out, 256);

        // ── SYNC_FLUSH: pad to byte boundary + empty stored block ──
        // This tells the decompressor it can produce all output now.
        if self.bit_count > 0 {
            out.push(self.bit_buf as u8);
            self.bit_buf = 0;
            self.bit_count = 0;
        }
        // Empty stored block: BFINAL=0, BTYPE=00, LEN=0, NLEN=0xFFFF
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0xFF, 0xFF]);
    }

    // ── Bit writer ──────────────────────────────────────────────────────

    #[inline(always)]
    fn write_bits(&mut self, out: &mut Vec<u8>, value: u32, count: u8) {
        self.bit_buf |= value << self.bit_count;
        self.bit_count += count;
        while self.bit_count >= 8 {
            out.push(self.bit_buf as u8);
            self.bit_buf >>= 8;
            self.bit_count -= 8;
        }
    }

    // ── Fixed Huffman symbol encoders ───────────────────────────────────

    /// Encode a literal (0..255), end-of-block (256), or length code (257..285).
    #[inline(always)]
    fn emit_lit_len(&mut self, out: &mut Vec<u8>, sym: u16) {
        if sym <= 143 {
            let code = 0x30 + sym as u32;
            self.write_bits(out, reverse_bits(code, 8), 8);
        } else if sym <= 255 {
            let code = 0x190 + (sym as u32 - 144);
            self.write_bits(out, reverse_bits(code, 9), 9);
        } else if sym <= 279 {
            let code = sym as u32 - 256;
            self.write_bits(out, reverse_bits(code, 7), 7);
        } else {
            let code = 0xC0 + (sym as u32 - 280);
            self.write_bits(out, reverse_bits(code, 8), 8);
        }
    }

    /// Encode a distance code (5-bit fixed Huffman).
    #[inline(always)]
    fn emit_distance(&mut self, out: &mut Vec<u8>, sym: u8) {
        self.write_bits(out, reverse_bits(sym as u32, 5), 5);
    }

    // ── LZ77 matching ──────────────────────────────────────────────────

    fn find_match(&self, data: &[u8], pos: usize) -> (usize, usize) {
        if pos + MIN_MATCH > data.len() {
            return (0, 0);
        }

        let h = hash3(data, pos);
        let mut chain = self.head[h];
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        let mut chain_limit = MAX_CHAIN;

        while chain != u32::MAX && chain_limit > 0 {
            let candidate = chain as usize;
            if candidate >= pos || pos - candidate > WINDOW_SIZE {
                break;
            }
            let dist = pos - candidate;
            let max_len = (data.len() - pos).min(MAX_MATCH);
            let mut len = 0;
            while len < max_len && data[candidate + len] == data[pos + len] {
                len += 1;
            }
            if len >= MIN_MATCH && len > best_len {
                best_len = len;
                best_dist = dist;
                if len == MAX_MATCH { break; }
            }
            chain = self.prev[candidate % WINDOW_SIZE];
            chain_limit -= 1;
        }

        (best_len, best_dist)
    }

    #[inline(always)]
    fn update_hash(&mut self, data: &[u8], pos: usize) {
        if pos + MIN_MATCH <= data.len() {
            let h = hash3(data, pos);
            self.prev[pos % WINDOW_SIZE] = self.head[h];
            self.head[h] = pos as u32;
        }
    }
}
