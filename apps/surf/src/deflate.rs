// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! DEFLATE decompressor (RFC 1951) + gzip wrapper.
//!
//! Copied from libimage's deflate.rs with an added `decompress_gzip()` entry
//! point that strips the gzip framing before calling the raw DEFLATE decoder.

use alloc::vec::Vec;

const WINDOW_SIZE: usize = 32768;
const WINDOW_MASK: usize = WINDOW_SIZE - 1;

// ── Huffman lookup table ────────────────────────────────────────────────────

const PRIMARY_BITS: u32 = 9;
const PRIMARY_SIZE: usize = 1 << PRIMARY_BITS;

type HuffEntry = u16;

fn entry_sym(e: HuffEntry) -> u16 { e & 0x1FF }
fn entry_len(e: HuffEntry) -> u32 { (e >> 9) as u32 & 0xF }
fn make_entry(sym: u16, len: u32) -> HuffEntry { (sym & 0x1FF) | ((len as u16 & 0xF) << 9) }

struct HuffTable {
    fast: [HuffEntry; PRIMARY_SIZE],
    counts: [u16; 16],
    symbols: [u16; 320],
}

impl HuffTable {
    fn build(lengths: &[u8], max_sym: usize) -> Self {
        let mut counts = [0u16; 16];
        for i in 0..max_sym {
            let l = lengths[i] as usize;
            if l > 0 && l < 16 {
                counts[l] += 1;
            }
        }

        let mut next_code = [0u32; 16];
        {
            let mut code = 0u32;
            for bits in 1..16u32 {
                code = (code + counts[bits as usize - 1] as u32) << 1;
                next_code[bits as usize] = code;
            }
        }

        let mut fast = [0u16; PRIMARY_SIZE];
        let mut offsets = [0u16; 16];
        let mut total = 0u16;
        for i in 1..16 {
            offsets[i] = total;
            total += counts[i];
        }

        let mut symbols = [0u16; 320];
        let mut sym_codes = [0u32; 320];
        for i in 0..max_sym {
            let l = lengths[i] as usize;
            if l > 0 && l < 16 {
                sym_codes[i] = next_code[l];
                next_code[l] += 1;
                symbols[offsets[l] as usize] = i as u16;
                offsets[l] += 1;
            }
        }

        for i in 0..max_sym {
            let l = lengths[i] as u32;
            if l == 0 || l > PRIMARY_BITS { continue; }
            let code = sym_codes[i];
            let reversed = bit_reverse(code, l);
            let entry = make_entry(i as u16, l);
            let suffix_count = 1u32 << (PRIMARY_BITS - l);
            for s in 0..suffix_count {
                fast[(reversed | (s << l)) as usize] = entry;
            }
        }

        HuffTable { fast, counts, symbols }
    }

    #[inline(always)]
    fn decode_fast(&self, bs: &mut BitStream) -> Option<u16> {
        bs.ensure(PRIMARY_BITS as u8);
        let peek = bs.peek(PRIMARY_BITS as u8);
        let entry = self.fast[peek as usize];
        if entry != 0 {
            let len = entry_len(entry);
            bs.consume(len as u8);
            return Some(entry_sym(entry));
        }
        self.decode_slow(bs)
    }

    fn decode_slow(&self, bs: &mut BitStream) -> Option<u16> {
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

fn bit_reverse(code: u32, n: u32) -> u32 {
    let mut result = 0u32;
    let mut c = code;
    for _ in 0..n {
        result = (result << 1) | (c & 1);
        c >>= 1;
    }
    result
}

// ── Buffered bit reader ─────────────────────────────────────────────────────

struct BitStream<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    bits_in: u8,
}

impl<'a> BitStream<'a> {
    fn new(data: &'a [u8], start: usize) -> Self {
        let mut s = Self { data, pos: start, buf: 0, bits_in: 0 };
        s.refill();
        s
    }

    #[inline(always)]
    fn refill(&mut self) {
        while self.bits_in <= 24 && self.pos < self.data.len() {
            self.buf |= (self.data[self.pos] as u32) << self.bits_in;
            self.pos += 1;
            self.bits_in += 8;
        }
    }

    #[inline(always)]
    fn ensure(&mut self, n: u8) {
        if self.bits_in < n {
            self.refill();
        }
    }

    #[inline(always)]
    fn peek(&self, n: u8) -> u32 {
        self.buf & ((1u32 << n) - 1)
    }

    #[inline(always)]
    fn consume(&mut self, n: u8) {
        self.buf >>= n;
        self.bits_in -= n;
        self.refill();
    }

    #[inline(always)]
    fn read_bits(&mut self, n: u8) -> Option<u32> {
        if n == 0 { return Some(0); }
        self.ensure(n);
        if self.bits_in < n { return None; }
        let val = self.buf & ((1u32 << n) - 1);
        self.buf >>= n;
        self.bits_in -= n;
        Some(val)
    }

    fn align(&mut self) {
        let discard = self.bits_in % 8;
        if discard > 0 {
            self.buf >>= discard;
            self.bits_in -= discard;
        }
    }

    fn byte_pos(&self) -> usize {
        self.pos - (self.bits_in as usize / 8)
    }
}

// ── Static tables ───────────────────────────────────────────────────────────

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

static LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13,
    15, 17, 19, 23, 27, 31, 35, 43, 51, 59,
    67, 83, 99, 115, 131, 163, 195, 227, 258,
];

static LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1,
    1, 1, 2, 2, 2, 2, 3, 3, 3, 3,
    4, 4, 4, 4, 5, 5, 5, 5, 0,
];

static DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25,
    33, 49, 65, 97, 129, 193, 257, 385, 513, 769,
    1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

static DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3,
    4, 4, 5, 5, 6, 6, 7, 7, 8, 8,
    9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];

static CL_ORDER: [u8; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

// ── Raw DEFLATE decompressor ────────────────────────────────────────────────

fn decompress_raw(input: &[u8], output: &mut [u8], window: &mut [u8]) -> i32 {
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
                bs.align();
                let pos = bs.byte_pos();
                if pos + 4 > input.len() { return -1; }
                let len = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
                let _nlen = u16::from_le_bytes([input[pos + 2], input[pos + 3]]);
                let data_start = pos + 4;
                if data_start + len > input.len() { return -1; }
                if out_pos + len > output.len() { return -1; }
                for i in 0..len {
                    let byte = input[data_start + i];
                    output[out_pos] = byte;
                    window[win_pos & WINDOW_MASK] = byte;
                    out_pos += 1;
                    win_pos += 1;
                }
                bs = BitStream::new(input, data_start + len);
            }
            1 => {
                let lit_table = HuffTable::build(&FIXED_LIT_LENS, 288);
                let dist_table = HuffTable::build(&FIXED_DIST_LENS, 32);
                let r = decode_block(&mut bs, &lit_table, &dist_table,
                                     output, &mut out_pos, window, &mut win_pos);
                if r < 0 { return r; }
            }
            2 => {
                let hlit = bs.read_bits(5).unwrap_or(0) as usize + 257;
                let hdist = bs.read_bits(5).unwrap_or(0) as usize + 1;
                let hclen = bs.read_bits(4).unwrap_or(0) as usize + 4;

                let mut cl_lens = [0u8; 19];
                for i in 0..hclen {
                    cl_lens[CL_ORDER[i] as usize] = bs.read_bits(3).unwrap_or(0) as u8;
                }
                let cl_table = HuffTable::build(&cl_lens, 19);

                let total = hlit + hdist;
                let mut lengths = [0u8; 320];
                let mut i = 0;
                while i < total {
                    let sym = cl_table.decode_fast(&mut bs).unwrap_or(0) as usize;
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
        let sym = match lit_table.decode_fast(bs) {
            Some(s) => s as usize,
            None => return -1,
        };

        if sym == 256 {
            return 0;
        }

        if sym < 256 {
            if *out_pos >= output.len() { return -1; }
            let byte = sym as u8;
            output[*out_pos] = byte;
            window[*win_pos & WINDOW_MASK] = byte;
            *out_pos += 1;
            *win_pos += 1;
        } else {
            let len_idx = sym - 257;
            if len_idx >= 29 { return -1; }
            let extra = LEN_EXTRA[len_idx];
            let length = LEN_BASE[len_idx] as usize
                + if extra > 0 { bs.read_bits(extra).unwrap_or(0) as usize } else { 0 };

            let dist_sym = match dist_table.decode_fast(bs) {
                Some(s) => s as usize,
                None => return -1,
            };
            if dist_sym >= 30 { return -1; }
            let dextra = DIST_EXTRA[dist_sym];
            let distance = DIST_BASE[dist_sym] as usize
                + if dextra > 0 { bs.read_bits(dextra).unwrap_or(0) as usize } else { 0 };

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

// ── Public API ──────────────────────────────────────────────────────────────

/// Decompress a gzip stream (RFC 1952).
/// Returns the decompressed bytes or None on error.
pub fn decompress_gzip(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 18 { return None; }
    if data[0] != 0x1F || data[1] != 0x8B { return None; } // magic
    if data[2] != 8 { return None; } // method = deflate

    let flags = data[3];
    let mut pos = 10; // skip header fields (ID1, ID2, CM, FLG, MTIME[4], XFL, OS)

    // FEXTRA
    if flags & 0x04 != 0 {
        if pos + 2 > data.len() { return None; }
        let xlen = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2 + xlen;
    }
    // FNAME
    if flags & 0x08 != 0 {
        while pos < data.len() && data[pos] != 0 { pos += 1; }
        pos += 1; // skip null terminator
    }
    // FCOMMENT
    if flags & 0x10 != 0 {
        while pos < data.len() && data[pos] != 0 { pos += 1; }
        pos += 1;
    }
    // FHCRC
    if flags & 0x02 != 0 {
        pos += 2;
    }

    if pos >= data.len() { return None; }

    // Read ISIZE from last 4 bytes of gzip stream (original uncompressed size mod 2^32)
    let isize_pos = data.len() - 4;
    let orig_size = u32::from_le_bytes([
        data[isize_pos], data[isize_pos + 1], data[isize_pos + 2], data[isize_pos + 3],
    ]) as usize;

    // Cap at 32 MiB to prevent OOM
    let alloc_size = if orig_size > 0 && orig_size < 32 * 1024 * 1024 {
        orig_size
    } else {
        data.len() * 4 // heuristic
    };

    let deflate_data = &data[pos..data.len().saturating_sub(8)]; // strip CRC32 + ISIZE trailer
    let mut output = vec![0u8; alloc_size];
    let mut window = vec![0u8; WINDOW_SIZE];

    let n = decompress_raw(deflate_data, &mut output, &mut window);
    if n < 0 { return None; }

    output.truncate(n as usize);
    Some(output)
}

/// Decompress raw DEFLATE data (no framing).
/// Returns decompressed bytes or None on error.
pub fn decompress_deflate(data: &[u8]) -> Option<Vec<u8>> {
    let alloc_size = data.len() * 4; // heuristic
    let mut output = vec![0u8; alloc_size];
    let mut window = vec![0u8; WINDOW_SIZE];

    let n = decompress_raw(data, &mut output, &mut window);
    if n < 0 { return None; }

    output.truncate(n as usize);
    Some(output)
}

/// Decompress zlib-wrapped DEFLATE data (RFC 1950).
/// Returns decompressed bytes or None on error.
pub fn decompress_zlib(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 6 { return None; }
    // Check zlib header: CMF byte (method=8) + FLG byte
    let cmf = data[0];
    if cmf & 0x0F != 8 { return None; } // deflate method
    let flg = data[1];
    if (cmf as u16 * 256 + flg as u16) % 31 != 0 { return None; } // check bits

    // Skip 2-byte header (+ optional 4-byte FDICT if present)
    let start = if flg & 0x20 != 0 { 6 } else { 2 };
    // Strip 4-byte Adler-32 checksum at end
    let end = data.len().saturating_sub(4);
    if start >= end { return None; }

    decompress_deflate(&data[start..end])
}
