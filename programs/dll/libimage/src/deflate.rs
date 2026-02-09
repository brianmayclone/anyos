// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! DEFLATE decompressor (RFC 1951).
//!
//! Supports fixed and dynamic Huffman blocks.
//! The scratch buffer must contain at least 32 KiB for the sliding window.

const WINDOW_SIZE: usize = 32768;
const WINDOW_MASK: usize = WINDOW_SIZE - 1;

/// Fixed Huffman code length tables (RFC 1951 section 3.2.6).
const fn fixed_lit_lengths() -> [u8; 288] {
    let mut t = [0u8; 288];
    let mut i = 0;
    while i <= 143 { t[i] = 8; i += 1; }
    while i <= 255 { t[i] = 9; i += 1; }
    while i <= 279 { t[i] = 7; i += 1; }
    while i <= 287 { t[i] = 8; i += 1; }
    t
}

const fn fixed_dist_lengths() -> [u8; 32] {
    let mut t = [0u8; 32];
    let mut i = 0;
    while i < 32 { t[i] = 5; i += 1; }
    t
}

static FIXED_LIT_LENS: [u8; 288] = fixed_lit_lengths();
static FIXED_DIST_LENS: [u8; 32] = fixed_dist_lengths();

/// Length base values (codes 257-285).
static LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13,
    15, 17, 19, 23, 27, 31, 35, 43, 51, 59,
    67, 83, 99, 115, 131, 163, 195, 227, 258,
];

/// Extra bits for length codes.
static LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1,
    1, 1, 2, 2, 2, 2, 3, 3, 3, 3,
    4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// Distance base values.
static DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25,
    33, 49, 65, 97, 129, 193, 257, 385, 513, 769,
    1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

/// Extra bits for distance codes.
static DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3,
    4, 4, 5, 5, 6, 6, 7, 7, 8, 8,
    9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];

/// Code length code order (for dynamic Huffman).
static CL_ORDER: [u8; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// A simple Huffman lookup table (brute-force, max 15-bit codes).
struct HuffTable {
    /// (symbol, code_length) indexed by code value (left-aligned).
    /// We use a flat lookup table up to 9 bits for fast decode,
    /// with fallback linear scan for longer codes.
    counts: [u16; 16],    // number of codes of each length
    symbols: [u16; 320],  // symbols sorted by code
}

impl HuffTable {
    fn build(lengths: &[u8], max_sym: usize) -> Self {
        let mut counts = [0u16; 16];
        let mut symbols = [0u16; 320];

        for i in 0..max_sym {
            if (lengths[i] as usize) < 16 {
                counts[lengths[i] as usize] += 1;
            }
        }

        // Compute first code for each length
        let mut offsets = [0u16; 16];
        let mut total = 0u16;
        for i in 1..16 {
            offsets[i] = total;
            total += counts[i];
        }

        // Sort symbols by code
        for i in 0..max_sym {
            let len = lengths[i] as usize;
            if len > 0 && len < 16 {
                symbols[offsets[len] as usize] = i as u16;
                offsets[len] += 1;
            }
        }

        HuffTable { counts, symbols }
    }

    fn decode(&self, bs: &mut BitStream) -> Option<u16> {
        let mut code = 0u32;
        let mut first = 0u32;
        let mut index = 0u32;

        for len in 1..16u32 {
            code |= bs.read_bits(1)? as u32;
            let count = self.counts[len as usize] as u32;
            if code < first + count {
                return Some(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

/// Bit-level reader over a byte slice.
struct BitStream<'a> {
    data: &'a [u8],
    pos: usize,   // byte position
    bit: u8,      // bit position within current byte (0-7)
}

impl<'a> BitStream<'a> {
    fn new(data: &'a [u8], start: usize) -> Self {
        Self { data, pos: start, bit: 0 }
    }

    fn read_bits(&mut self, n: u8) -> Option<u32> {
        let mut val = 0u32;
        for i in 0..n {
            if self.pos >= self.data.len() {
                return None;
            }
            let b = (self.data[self.pos] >> self.bit) & 1;
            val |= (b as u32) << i;
            self.bit += 1;
            if self.bit >= 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Some(val)
    }

    /// Align to next byte boundary.
    fn align(&mut self) {
        if self.bit > 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }

    fn byte_pos(&self) -> usize {
        self.pos
    }
}

/// Decompress DEFLATE data.
///
/// - `input`: raw DEFLATE stream (no zlib header)
/// - `output`: decompressed data buffer
/// - `window`: 32 KiB sliding window scratch
///
/// Returns number of bytes written to `output`, or negative error.
pub fn decompress(input: &[u8], output: &mut [u8], window: &mut [u8]) -> i32 {
    if window.len() < WINDOW_SIZE {
        return -1;
    }

    let mut bs = BitStream::new(input, 0);
    let mut out_pos = 0usize;
    let mut win_pos = 0usize;

    loop {
        let bfinal = bs.read_bits(1);
        let btype = bs.read_bits(2);
        if bfinal.is_none() || btype.is_none() {
            return -1;
        }
        let is_final = bfinal.unwrap() == 1;
        let block_type = btype.unwrap();

        match block_type {
            0 => {
                // Uncompressed block
                bs.align();
                let pos = bs.byte_pos();
                if pos + 4 > input.len() { return -1; }
                let len = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
                let _nlen = u16::from_le_bytes([input[pos + 2], input[pos + 3]]);
                bs.pos = pos + 4;
                if bs.pos + len > input.len() { return -1; }
                if out_pos + len > output.len() { return -1; }
                for i in 0..len {
                    let byte = input[bs.pos + i];
                    output[out_pos] = byte;
                    window[win_pos & WINDOW_MASK] = byte;
                    out_pos += 1;
                    win_pos += 1;
                }
                bs.pos += len;
            }
            1 => {
                // Fixed Huffman
                let lit_table = HuffTable::build(&FIXED_LIT_LENS, 288);
                let dist_table = HuffTable::build(&FIXED_DIST_LENS, 32);
                let r = decode_block(&mut bs, &lit_table, &dist_table,
                                     output, &mut out_pos, window, &mut win_pos);
                if r < 0 { return r; }
            }
            2 => {
                // Dynamic Huffman
                let hlit = bs.read_bits(5).unwrap_or(0) as usize + 257;
                let hdist = bs.read_bits(5).unwrap_or(0) as usize + 1;
                let hclen = bs.read_bits(4).unwrap_or(0) as usize + 4;

                // Read code length code lengths
                let mut cl_lens = [0u8; 19];
                for i in 0..hclen {
                    cl_lens[CL_ORDER[i] as usize] = bs.read_bits(3).unwrap_or(0) as u8;
                }
                let cl_table = HuffTable::build(&cl_lens, 19);

                // Decode literal/length + distance code lengths
                let total = hlit + hdist;
                let mut lengths = [0u8; 320];
                let mut i = 0;
                while i < total {
                    let sym = cl_table.decode(&mut bs).unwrap_or(0) as usize;
                    match sym {
                        0..=15 => {
                            lengths[i] = sym as u8;
                            i += 1;
                        }
                        16 => {
                            let rep = bs.read_bits(2).unwrap_or(0) as usize + 3;
                            let val = if i > 0 { lengths[i - 1] } else { 0 };
                            for _ in 0..rep {
                                if i < total { lengths[i] = val; i += 1; }
                            }
                        }
                        17 => {
                            let rep = bs.read_bits(3).unwrap_or(0) as usize + 3;
                            for _ in 0..rep {
                                if i < total { lengths[i] = 0; i += 1; }
                            }
                        }
                        18 => {
                            let rep = bs.read_bits(7).unwrap_or(0) as usize + 11;
                            for _ in 0..rep {
                                if i < total { lengths[i] = 0; i += 1; }
                            }
                        }
                        _ => return -1,
                    }
                }

                let lit_table = HuffTable::build(&lengths[..hlit], hlit);
                let dist_table = HuffTable::build(&lengths[hlit..hlit + hdist], hdist);
                let r = decode_block(&mut bs, &lit_table, &dist_table,
                                     output, &mut out_pos, window, &mut win_pos);
                if r < 0 { return r; }
            }
            _ => return -1,
        }

        if is_final {
            break;
        }
    }

    out_pos as i32
}

/// Decode a Huffman-compressed block.
fn decode_block(
    bs: &mut BitStream,
    lit_table: &HuffTable,
    dist_table: &HuffTable,
    output: &mut [u8],
    out_pos: &mut usize,
    window: &mut [u8],
    win_pos: &mut usize,
) -> i32 {
    loop {
        let sym = match lit_table.decode(bs) {
            Some(s) => s as usize,
            None => return -1,
        };

        if sym == 256 {
            return 0; // End of block
        }

        if sym < 256 {
            // Literal byte
            if *out_pos >= output.len() { return -1; }
            output[*out_pos] = sym as u8;
            window[*win_pos & WINDOW_MASK] = sym as u8;
            *out_pos += 1;
            *win_pos += 1;
        } else {
            // Length/distance pair
            let len_idx = sym - 257;
            if len_idx >= 29 { return -1; }
            let length = LEN_BASE[len_idx] as usize
                + bs.read_bits(LEN_EXTRA[len_idx]).unwrap_or(0) as usize;

            let dist_sym = match dist_table.decode(bs) {
                Some(s) => s as usize,
                None => return -1,
            };
            if dist_sym >= 30 { return -1; }
            let distance = DIST_BASE[dist_sym] as usize
                + bs.read_bits(DIST_EXTRA[dist_sym]).unwrap_or(0) as usize;

            if distance == 0 || distance > *win_pos { return -1; }
            if *out_pos + length > output.len() { return -1; }

            for _ in 0..length {
                let byte = window[(*win_pos - distance) & WINDOW_MASK];
                output[*out_pos] = byte;
                window[*win_pos & WINDOW_MASK] = byte;
                *out_pos += 1;
                *win_pos += 1;
            }
        }
    }
}
