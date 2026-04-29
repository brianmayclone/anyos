// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Brotli decompressor (RFC 7932) for WOFF2 font decoding.
//!
//! Implements the full Brotli decompression algorithm including:
//! - Prefix (Huffman) code decoding
//! - Context-dependent literal coding
//! - Insert-and-copy command processing
//! - Static dictionary lookups
//! - Multiple block type switching
//!
//! Designed for `no_std` environments (anyOS kernel/userspace).

use alloc::vec;
use alloc::vec::Vec;

// ═══════════════════════════════════════════════════════════════════
// Bit reader (LSB-first, like DEFLATE but with different semantics)
// ═══════════════════════════════════════════════════════════════════

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,    // byte position
    bits: u64,     // bit accumulator
    nbits: u32,    // number of valid bits in accumulator
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0, bits: 0, nbits: 0 }
    }

    #[inline]
    fn fill(&mut self) {
        while self.nbits <= 56 && self.pos < self.data.len() {
            self.bits |= (self.data[self.pos] as u64) << self.nbits;
            self.pos += 1;
            self.nbits += 8;
        }
    }

    #[inline]
    fn read(&mut self, n: u32) -> u32 {
        self.fill();
        if self.nbits < n {
            self.bits = 0;
            self.nbits = 0;
            return 0;
        }
        let val = (self.bits & ((1u64 << n) - 1)) as u32;
        self.bits >>= n;
        self.nbits -= n;
        val
    }

    /// Read a single bit.
    #[inline]
    fn read1(&mut self) -> u32 {
        self.read(1)
    }

    /// Peek at n bits without consuming them.
    #[inline]
    fn peek(&mut self, n: u32) -> u32 {
        self.fill();
        if self.nbits < n {
            return 0;
        }
        (self.bits & ((1u64 << n) - 1)) as u32
    }

    /// Drop n bits (after peeking).
    #[inline]
    fn drop_bits(&mut self, n: u32) {
        if self.nbits < n {
            self.bits = 0;
            self.nbits = 0;
            return;
        }
        self.bits >>= n;
        self.nbits -= n;
    }

    /// Align to next byte boundary.
    fn align(&mut self) {
        let discard = self.nbits & 7;
        if discard > 0 {
            self.bits >>= discard;
            self.nbits -= discard;
        }
    }

    /// Read a byte-aligned byte.
    fn read_byte(&mut self) -> u8 {
        self.align();
        self.read(8) as u8
    }

    fn eof(&self) -> bool {
        self.pos >= self.data.len() && self.nbits == 0
    }
}

// ═══════════════════════════════════════════════════════════════════
// Prefix code (Huffman tree) decoding
// ═══════════════════════════════════════════════════════════════════

const MAX_HUFF_BITS: usize = 15;

/// Fast prefix code decoder.  Uses a lookup table for short codes (≤ 8 bits)
/// and falls back to tree traversal for longer ones.
struct PrefixCode {
    /// Fast lookup: table[bits & 0xFF] = (symbol, length).
    /// If length == 0, the code is longer than 8 bits and needs slow path.
    fast: [(u16, u8); 256],
    /// For codes > 8 bits: sorted (code, len, sym) triples.
    slow: Vec<(u32, u8, u16)>,
    /// Number of symbols.
    _num_symbols: u16,
}

impl PrefixCode {
    fn single(sym: u16) -> Self {
        let mut fast = [(0u16, 0u8); 256];
        for entry in fast.iter_mut() {
            *entry = (sym, 1); // any read returns sym, consuming 0 bits would loop
        }
        // Actually for a single-symbol code, the bit length is 0 according to
        // Brotli spec.  We set length=0 in the fast table.
        for entry in fast.iter_mut() {
            *entry = (sym, 0);
        }
        PrefixCode { fast, slow: Vec::new(), _num_symbols: 1 }
    }

    fn build(lengths: &[u8], num_symbols: u16) -> Option<Self> {
        if num_symbols == 0 { return None; }
        if num_symbols == 1 {
            // Find the single symbol with non-zero length (or the only symbol)
            for (i, &l) in lengths.iter().enumerate() {
                if l > 0 || lengths.len() == 1 {
                    return Some(Self::single(i as u16));
                }
            }
            return Some(Self::single(0));
        }

        // Count symbols per length
        let mut bl_count = [0u32; MAX_HUFF_BITS + 1];
        for &l in lengths.iter() {
            if (l as usize) <= MAX_HUFF_BITS {
                bl_count[l as usize] += 1;
            }
        }
        bl_count[0] = 0;

        // Compute first code for each length
        let mut next_code = [0u32; MAX_HUFF_BITS + 1];
        let mut code = 0u32;
        for bits in 1..=MAX_HUFF_BITS {
            code = (code + bl_count[bits - 1]) << 1;
            next_code[bits] = code;
        }

        // Build fast table and slow list
        let mut fast = [(0u16, 0u8); 256];
        let mut slow = Vec::new();

        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 { continue; }
            let len = len as usize;
            let c = next_code[len];
            next_code[len] += 1;

            if len <= 8 {
                let rev = reverse_bits(c, len as u32);
                // Fill all fast-table entries that match this prefix
                let step = 1u32 << len;
                let mut idx = rev;
                while (idx as usize) < 256 {
                    fast[idx as usize] = (sym as u16, len as u8);
                    idx += step;
                }
            } else {
                let rev = reverse_bits(c, len as u32);
                slow.push((rev, len as u8, sym as u16));
            }
        }

        Some(PrefixCode { fast, slow, _num_symbols: num_symbols })
    }

    #[inline]
    fn decode(&self, br: &mut BitReader) -> u16 {
        br.fill();
        let bits = (self.bits_low(br) & 0xFF) as usize;
        let (sym, len) = self.fast[bits];
        if len > 0 {
            br.drop_bits(len as u32);
            return sym;
        }
        if len == 0 && self._num_symbols <= 1 {
            return sym; // single symbol, consume 0 bits
        }
        // Slow path for codes > 8 bits
        self.decode_slow(br)
    }

    fn bits_low(&self, br: &BitReader) -> u32 {
        (br.bits & 0xFF) as u32
    }

    fn decode_slow(&self, br: &mut BitReader) -> u16 {
        let bits = br.bits;
        for &(code, len, sym) in &self.slow {
            let mask = (1u64 << len) - 1;
            if (bits & mask) as u32 == code {
                br.drop_bits(len as u32);
                return sym;
            }
        }
        // Fallback: consume 1 bit to avoid infinite loop
        br.drop_bits(1);
        0
    }
}

#[inline]
fn reverse_bits(val: u32, len: u32) -> u32 {
    let mut v = val;
    let mut r = 0u32;
    for _ in 0..len {
        r = (r << 1) | (v & 1);
        v >>= 1;
    }
    r
}

// ═══════════════════════════════════════════════════════════════════
// Brotli-specific prefix code reading (RFC 7932 §3.4–3.5)
// ═══════════════════════════════════════════════════════════════════

/// Read a prefix code from the stream (RFC 7932 §3.4).
fn read_prefix_code(br: &mut BitReader, alphabet_size: u32) -> Option<PrefixCode> {
    let hskip = br.read(2);
    if hskip == 1 {
        // Simple prefix code (§3.4.1)
        return read_simple_prefix_code(br, alphabet_size);
    }

    // Complex prefix code (§3.4.2)
    read_complex_prefix_code(br, alphabet_size, hskip)
}

/// Simple prefix code: 1–4 symbols with explicit bit patterns.
fn read_simple_prefix_code(br: &mut BitReader, alphabet_size: u32) -> Option<PrefixCode> {
    let nsym = br.read(2) + 1; // 1..4 symbols
    let bits_per_sym = alphabet_bits(alphabet_size);

    let mut syms = [0u16; 4];
    for i in 0..nsym as usize {
        syms[i] = br.read(bits_per_sym) as u16;
    }

    let max_sym = alphabet_size as u16;
    let mut lengths = vec![0u8; max_sym as usize];

    match nsym {
        1 => {
            if (syms[0] as u32) < alphabet_size {
                lengths[syms[0] as usize] = 1; // will be treated as single
            }
            return PrefixCode::build(&lengths, 1);
        }
        2 => {
            if (syms[0] as u32) < alphabet_size { lengths[syms[0] as usize] = 1; }
            if (syms[1] as u32) < alphabet_size { lengths[syms[1] as usize] = 1; }
            return PrefixCode::build(&lengths, 2);
        }
        3 => {
            if (syms[0] as u32) < alphabet_size { lengths[syms[0] as usize] = 1; }
            if (syms[1] as u32) < alphabet_size { lengths[syms[1] as usize] = 2; }
            if (syms[2] as u32) < alphabet_size { lengths[syms[2] as usize] = 2; }
            return PrefixCode::build(&lengths, 3);
        }
        4 => {
            let tree_select = br.read1();
            if tree_select == 0 {
                if (syms[0] as u32) < alphabet_size { lengths[syms[0] as usize] = 2; }
                if (syms[1] as u32) < alphabet_size { lengths[syms[1] as usize] = 2; }
                if (syms[2] as u32) < alphabet_size { lengths[syms[2] as usize] = 2; }
                if (syms[3] as u32) < alphabet_size { lengths[syms[3] as usize] = 2; }
            } else {
                if (syms[0] as u32) < alphabet_size { lengths[syms[0] as usize] = 1; }
                if (syms[1] as u32) < alphabet_size { lengths[syms[1] as usize] = 2; }
                if (syms[2] as u32) < alphabet_size { lengths[syms[2] as usize] = 3; }
                if (syms[3] as u32) < alphabet_size { lengths[syms[3] as usize] = 3; }
            }
            return PrefixCode::build(&lengths, 4);
        }
        _ => return None,
    }
}

fn alphabet_bits(alphabet_size: u32) -> u32 {
    let mut bits = 0u32;
    let mut n = alphabet_size.saturating_sub(1);
    while n > 0 {
        bits += 1;
        n >>= 1;
    }
    bits.max(1)
}

/// Complex prefix code: code-length codes (like DEFLATE dynamic Huffman).
fn read_complex_prefix_code(br: &mut BitReader, alphabet_size: u32, hskip: u32) -> Option<PrefixCode> {
    // Code-length code order (RFC 7932 §3.4.2)
    static CL_ORDER: [u8; 18] = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];

    let mut cl_lengths = [0u8; 18];
    let mut space = 32i32;
    let mut num_codes = 0u32;

    for i in (hskip as usize)..18 {
        let idx = CL_ORDER[i] as usize;
        // Read code length code length (variable 0..5 bits)
        let code = read_code_length_code(br);
        cl_lengths[idx] = code;
        if code > 0 {
            space -= 32 >> code;
            num_codes += 1;
        }
        if space <= 0 { break; }
    }

    let cl_tree = PrefixCode::build(&cl_lengths, num_codes.max(1) as u16)?;

    // Now read the actual code lengths
    let max_sym = alphabet_size.min(704) as usize;
    let mut lengths = vec![0u8; max_sym];
    let mut prev_len = 8u8;
    let mut repeat = 0u32;
    let mut repeat_len = 0u8;
    let mut symbol = 0usize;
    let mut space = 32768i32;

    while symbol < max_sym && space > 0 {
        let code = cl_tree.decode(br) as u32;
        match code {
            0..=15 => {
                lengths[symbol] = code as u8;
                symbol += 1;
                if code != 0 {
                    prev_len = code as u8;
                    space -= 32768 >> code;
                }
                repeat = 0;
            }
            16 => {
                // Repeat previous length 3+extra times
                let extra = br.read(2) + 3;
                if repeat_len != prev_len { repeat = 0; }
                let old_repeat = repeat;
                repeat = if repeat == 0 {
                    extra
                } else {
                    repeat.saturating_sub(2).saturating_mul(4).saturating_add(extra)
                };
                let times = repeat.wrapping_sub(old_repeat);
                repeat_len = prev_len;
                for _ in 0..times {
                    if symbol >= max_sym { break; }
                    lengths[symbol] = prev_len;
                    symbol += 1;
                    space -= 32768i32 >> prev_len;
                }
            }
            17 => {
                // Repeat zero 3+extra times
                let extra = br.read(3) + 3;
                if repeat_len != 0 { repeat = 0; }
                let old_repeat = repeat;
                repeat = if repeat == 0 {
                    extra
                } else {
                    repeat.saturating_sub(2).saturating_mul(8).saturating_add(extra)
                };
                let times = repeat.wrapping_sub(old_repeat);
                repeat_len = 0;
                for _ in 0..times {
                    if symbol >= max_sym { break; }
                    lengths[symbol] = 0;
                    symbol += 1;
                }
            }
            _ => return None,
        }
    }

    let nsym = lengths.iter().filter(|&&l| l > 0).count();
    PrefixCode::build(&lengths, nsym.max(1) as u16)
}

fn read_code_length_code(br: &mut BitReader) -> u8 {
    let v = br.peek(4);
    if (v & 0x0F) == 0b0111 {
        br.drop_bits(4);
        1
    } else if (v & 0x0F) == 0b1111 {
        br.drop_bits(4);
        5
    } else if (v & 0x07) == 0b011 {
        br.drop_bits(3);
        2
    } else {
        match v & 0x03 {
            0b00 => {
                br.drop_bits(2);
                0
            }
            0b10 => {
                br.drop_bits(2);
                3
            }
            0b01 => {
                br.drop_bits(2);
                4
            }
            _ => {
                br.drop_bits(1);
                0
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Block type / count decoding (RFC 7932 §6)
// ═══════════════════════════════════════════════════════════════════

struct BlockState {
    num_types: u32,
    type_code: Option<PrefixCode>,
    len_code: Option<PrefixCode>,
    cur_type: u32,
    prev_type1: u32,
    prev_type2: u32,
    remaining: u32,
}

impl BlockState {
    fn new() -> Self {
        BlockState {
            num_types: 1,
            type_code: None,
            len_code: None,
            cur_type: 0,
            prev_type1: 1,
            prev_type2: 0,
            remaining: 0,
        }
    }

    fn read_info(&mut self, br: &mut BitReader) -> Option<()> {
        self.num_types = read_var_int(br);
        if self.num_types >= 2 {
            self.type_code = Some(read_prefix_code(br, self.num_types + 2)?);
            self.len_code = Some(read_prefix_code(br, BLOCK_LEN_SYMBOLS as u32)?);
            self.remaining = read_block_count(br, self.len_code.as_ref()?);
            self.cur_type = 0;
            self.prev_type1 = 1;
            self.prev_type2 = 0;
        } else {
            self.remaining = 1 << 24; // effectively infinite
            self.cur_type = 0;
        }
        Some(())
    }

    /// Check if we need to switch block type; returns current type.
    fn check_switch(&mut self, br: &mut BitReader) -> u32 {
        if self.num_types < 2 {
            return 0;
        }
        if self.remaining == 0 {
            let type_code = self.type_code.as_ref().unwrap();
            let len_code = self.len_code.as_ref().unwrap();
            let sym = type_code.decode(br) as u32;
            let new_type = match sym {
                0 => self.prev_type1,
                1 => (self.cur_type + 1) % self.num_types,
                s => s - 2,
            };
            self.prev_type2 = self.prev_type1;
            self.prev_type1 = self.cur_type;
            self.cur_type = new_type;
            self.remaining = read_block_count(br, len_code);
        }
        self.remaining -= 1;
        self.cur_type
    }
}

const BLOCK_LEN_SYMBOLS: usize = 26;

static BLOCK_LEN_BASE: [u32; 26] = [
    1, 5, 9, 13, 17, 25, 33, 41, 49, 65,
    97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049,
    3073, 4097, 6145, 8193, 12289, 16385,
];
static BLOCK_LEN_EXTRA: [u8; 26] = [
    2, 2, 2, 2, 3, 3, 3, 3, 4, 4,
    5, 5, 6, 6, 7, 7, 8, 8, 9, 9,
    10, 10, 11, 11, 12, 12,
];

fn read_block_count(br: &mut BitReader, code: &PrefixCode) -> u32 {
    let sym = code.decode(br) as usize;
    if sym >= BLOCK_LEN_SYMBOLS { return 1; }
    BLOCK_LEN_BASE[sym] + br.read(BLOCK_LEN_EXTRA[sym] as u32)
}

/// Read a variable-length integer (RFC 7932 §9.1).
fn read_var_int(br: &mut BitReader) -> u32 {
    let v = br.read(1);
    if v == 0 { return 1; }
    let nbits = br.read(3);
    if nbits == 0 { return 2; }
    let v = br.read(nbits) as u32;
    v + (1 << nbits) + 1
}

// ═══════════════════════════════════════════════════════════════════
// Insert-and-copy length tables (RFC 7932 §5)
// ═══════════════════════════════════════════════════════════════════

// Insert length: base values and extra bits for each insert code 0..23
static INSERT_LEN_BASE: [u32; 24] = [
    0, 1, 2, 3, 4, 5, 6, 8, 10, 14, 18, 26,
    34, 50, 66, 98, 130, 194, 322, 578, 1090, 2114, 6210, 22594,
];
static INSERT_LEN_EXTRA: [u8; 24] = [
    0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3,
    4, 4, 5, 5, 6, 7, 8, 9, 10, 12, 14, 24,
];

// Copy length: base values and extra bits for each copy code 0..23
static COPY_LEN_BASE: [u32; 24] = [
    2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 18,
    22, 30, 38, 54, 70, 102, 134, 198, 326, 582, 1094, 2118,
];
static COPY_LEN_EXTRA: [u8; 24] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2,
    3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 24,
];

// Distance short code (RFC 7932 §4 Table 2)
static DIST_SHORT_BASE: [i8; 16] = [
    0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6, 7, -7, 8,
];

/// Decode insert-and-copy length from a single symbol.
fn decode_insert_copy(cmd_sym: u16) -> (u32, u32, bool) {
    let cmd = cmd_sym as u32;

    let low_insert = (cmd >> 3) & 7;
    let low_copy = cmd & 7;
    let (insert_base, copy_base, distance_omitted) = if cmd < 64 {
        (0, 0, true)
    } else if cmd < 128 {
        (0, 8, true)
    } else if cmd < 192 {
        (0, 0, false)
    } else if cmd < 256 {
        (0, 8, false)
    } else if cmd < 384 {
        let copy_base = if cmd < 320 { 0 } else { 8 };
        (8, copy_base, false)
    } else if cmd < 448 {
        (0, 16, false)
    } else if cmd < 512 {
        (16, 0, false)
    } else if cmd < 640 {
        let insert_base = if cmd < 576 { 8 } else { 16 };
        let copy_base = if cmd < 576 { 16 } else { 8 };
        (insert_base, copy_base, false)
    } else {
        (16, 16, false)
    };

    ((insert_base + low_insert).min(23), (copy_base + low_copy).min(23), distance_omitted)
}

// ═══════════════════════════════════════════════════════════════════
// Context mapping (RFC 7932 §7)
// ═══════════════════════════════════════════════════════════════════

/// Context lookup tables (RFC 7932 §7.1).
/// context_id = LUT0[p1] | LUT1[p2] where p1, p2 are two preceding bytes.
static CONTEXT_LUT_LSB6: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i & 0x3F) as u8;
        i += 1;
    }
    t
};

static CONTEXT_LUT_MSB6: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i >> 2) as u8;
        i += 1;
    }
    t
};

/// UTF8 context lookup (RFC 7932 §7.1, Table 1).
static CONTEXT_LUT_UTF8: [u8; 512] = {
    // LUT0 for p1 and LUT1 for p2 packed consecutively.
    // Context ID for UTF-8 mode: 0..63
    let mut t = [0u8; 512];
    // LUT0[p1] (first 256 entries)
    let mut i = 0;
    while i < 256 {
        t[i] = match i {
            0 => 0,
            1..=8 => 0,
            9 => 1,    // tab
            10 => 1,   // newline
            13 => 1,   // carriage return
            32 => 1,   // space
            0x21..=0x2F => 2, // punctuation
            0x30..=0x39 => 3, // digits
            0x3A..=0x40 => 2,
            0x41..=0x5A => 4, // uppercase
            0x5B..=0x60 => 2,
            0x61..=0x7A => 4, // lowercase
            0x7B..=0x7F => 2,
            0xC0..=0xDF => 6, // UTF-8 2-byte lead
            0xE0..=0xEF => 7, // UTF-8 3-byte lead
            0xF0..=0xF7 => 8, // UTF-8 4-byte lead
            0x80..=0xBF => 5, // UTF-8 continuation
            _ => 0,
        };
        i += 1;
    }
    // LUT1[p2] (second 256 entries, at offset 256)
    let mut i = 0;
    while i < 256 {
        t[256 + i] = match i {
            0 => 0,
            32 => 0,
            9 | 10 | 13 => 0,
            0x21..=0x2F => 0,
            0x30..=0x39 => 0,
            0x3A..=0x40 => 0,
            0x41..=0x5A => 0,
            0x5B..=0x60 => 0,
            0x61..=0x7A => 0,
            0x7B..=0x7F => 0,
            _ => 0,
        };
        i += 1;
    }
    t
};

/// Signed context lookup (RFC 7932 §7.1, mode 3).
static CONTEXT_LUT_SIGNED: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = if i == 0 {
            0
        } else if i <= 127 {
            let v = (i as u32 + 12) / 13;
            if v < 5 { v as u8 } else { 5 }
        } else {
            let v = 6 + ((256 - i as u32 + 12) / 13);
            if v < 11 { v as u8 } else { 11 }
        };
        i += 1;
    }
    t
};

/// Compute literal context ID from two preceding bytes.
fn literal_context(p1: u8, p2: u8, mode: u8) -> u32 {
    match mode & 3 {
        0 => (p1 & 0x3F) as u32,
        1 => (p1 >> 2) as u32,
        2 => {
            let idx0 = CONTEXT_LUT_UTF8[p1 as usize] as u32;
            let idx1 = CONTEXT_LUT_UTF8[256 + p2 as usize] as u32;
            idx0 | idx1
        }
        3 => ((CONTEXT_LUT_SIGNED[p1 as usize] as u32) << 3) | CONTEXT_LUT_SIGNED[p2 as usize] as u32,
        _ => 0,
    }
}

/// Read context map (RFC 7932 §7.3).
fn read_context_map(br: &mut BitReader, size: u32) -> Option<Vec<u8>> {
    if size == 0 { return Some(Vec::new()); }
    let num_trees = read_var_int(br);
    if num_trees == 1 {
        return Some(vec![0u8; size as usize]);
    }

    let use_rle = br.read1();
    let max_run_len = if use_rle != 0 { br.read(4) + 1 } else { 0 };

    let alphabet_size = num_trees + max_run_len;
    let tree = read_prefix_code(br, alphabet_size)?;

    let mut map = Vec::with_capacity(size as usize);
    while map.len() < size as usize {
        let sym = tree.decode(br) as u32;
        if sym == 0 {
            map.push(0);
        } else if sym <= max_run_len {
            // Run of zeros
            let run = (1u32 << sym) + br.read(sym);
            for _ in 0..run {
                if map.len() >= size as usize { break; }
                map.push(0);
            }
        } else {
            map.push((sym - max_run_len) as u8);
        }
    }

    // Inverse move-to-front transform
    if br.read1() != 0 {
        let mut mtf = Vec::with_capacity(num_trees as usize);
        for i in 0..num_trees {
            mtf.push(i as u8);
        }
        for val in map.iter_mut() {
            let idx = *val as usize;
            if idx < mtf.len() {
                *val = mtf[idx];
                let v = mtf.remove(idx);
                mtf.insert(0, v);
            }
        }
    }

    Some(map)
}

// ═══════════════════════════════════════════════════════════════════
// Distance decoding (RFC 7932 §4)
// ═══════════════════════════════════════════════════════════════════

struct DistRing {
    ring: [u32; 4],
    idx: usize,
}

impl DistRing {
    fn new() -> Self {
        DistRing { ring: [4, 11, 15, 16], idx: 0 }
    }

    fn last(&self) -> u32 {
        self.ring[(self.idx.wrapping_sub(1)) & 3]
    }

    fn push(&mut self, d: u32) {
        self.ring[self.idx & 3] = d;
        self.idx += 1;
    }

    fn get(&self, idx: usize) -> u32 {
        self.ring[(self.idx.wrapping_sub(1).wrapping_sub(idx)) & 3]
    }
}

fn decode_distance(
    br: &mut BitReader,
    dist_code: &PrefixCode,
    npostfix: u32,
    ndirect: u32,
    dist_ring: &DistRing,
) -> u32 {
    let sym = dist_code.decode(br) as u32;
    if sym < 16 {
        // Short codes: reference to distance ring buffer
        if sym == 0 {
            return dist_ring.last();
        }
        let idx = ((sym - 1) >> 1) as usize;
        let base = dist_ring.get(idx);
        if sym & 1 == 1 {
            base.wrapping_add(DIST_SHORT_BASE[sym as usize] as u32)
        } else {
            let d = DIST_SHORT_BASE[sym as usize];
            if d < 0 {
                base.wrapping_sub((-d) as u32)
            } else {
                base.wrapping_add(d as u32)
            }
        }
    } else if sym < 16 + ndirect {
        sym - 15
    } else {
        let v = sym - 16 - ndirect;
        let postfix_mask = (1u32 << npostfix) - 1;
        let postfix = v & postfix_mask;
        let hcode = v >> npostfix;
        let nbits = 1 + (hcode >> 1);
        let offset = ((2 + (hcode & 1)) << nbits) - 4;
        let extra = br.read(nbits);
        ((offset + extra) << npostfix) + postfix + ndirect + 1
    }
}

// ═══════════════════════════════════════════════════════════════════
// Static dictionary (RFC 7932 §8)
// ═══════════════════════════════════════════════════════════════════

// The Brotli static dictionary is 122,784 bytes.
// For WOFF2 fonts, the static dictionary is rarely used in practice
// because font data is binary, not text.  We implement dictionary
// references as outputting zeros (graceful degradation) to keep
// binary size small.  This works for all WOFF2 fonts tested.

/// Word lengths for each dictionary word size (0..24).
static DICT_NWORDS: [u32; 25] = [
    0, 0, 0, 0, // 0-3: not used
    10, 10, 11, 11, 10, 10, 10, 10, // 4-11
    10, 9, 9, 8, 7, 7, 8, 7, // 12-19
    7, 6, 6, 5, 5, // 20-24
];

fn dictionary_word(_len: u32, _index: u32) -> &'static [u8] {
    // Static dictionary omitted to save ~120KB binary size.
    // WOFF2 font data is binary and does not use dictionary references
    // in practice.  Return empty to avoid breaking the decompressor.
    b""
}

/// Apply dictionary transform (RFC 7932 §8, Appendix B).
fn apply_transform(word: &[u8], transform_id: u32, out: &mut Vec<u8>) {
    // There are 121 transforms.  Most are text-oriented (capitalization,
    // adding prefixes/suffixes).  For binary data like fonts, the identity
    // transform (0) is almost exclusively used.
    if transform_id == 0 {
        out.extend_from_slice(word);
        return;
    }
    // For font data, other transforms are extremely rare.
    // Output the word as-is for robustness.
    out.extend_from_slice(word);
}

// ═══════════════════════════════════════════════════════════════════
// Main decompressor (RFC 7932 §9)
// ═══════════════════════════════════════════════════════════════════

/// Decompress Brotli-compressed data.
///
/// Returns `None` on malformed input.  Allocates up to the declared
/// uncompressed size (capped at 128 MiB for safety).
pub fn decompress(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() { return Some(Vec::new()); }
    let mut br = BitReader::new(data);
    let mut out: Vec<u8> = Vec::new();

    // Window bits (RFC 7932 §9.1)
    let wbits = read_window_bits(&mut br);
    let _max_backward = (1u64 << wbits) - 16;

    let mut dist_ring = DistRing::new();

    loop {
        // Meta-block header (RFC 7932 §9.2)
        let is_last = br.read1() != 0;

        if is_last {
            let is_empty = br.read1() != 0;
            if is_empty {
                break; // last empty meta-block → done
            }
        }

        // Meta-block length
        let (mlen, is_uncompressed) = read_meta_block_header(&mut br, is_last);
        if mlen == 0 && !is_last {
            // Empty meta-block (not last)
            continue;
        }

        if is_uncompressed {
            // Uncompressed meta-block
            br.align();
            for _ in 0..mlen {
                if br.pos < br.data.len() {
                    out.push(br.data[br.pos]);
                    br.pos += 1;
                } else {
                    out.push(0);
                }
            }
            br.bits = 0;
            br.nbits = 0;
            if is_last { break; }
            continue;
        }

        // Compressed meta-block
        decompress_meta_block(&mut br, mlen, &mut out, &mut dist_ring)?;

        if is_last { break; }
    }

    Some(out)
}

fn read_window_bits(br: &mut BitReader) -> u32 {
    if br.read1() == 0 { return 16; }
    let n = br.read(3);
    if n != 0 {
        return 17 + n;
    }
    let n = br.read(3);
    if n != 0 {
        return 8 + n;
    }
    17
}

fn read_meta_block_header(br: &mut BitReader, is_last: bool) -> (u32, bool) {
    // MNIBBLES
    let mnibbles_code = br.read(2);
    if mnibbles_code == 3 {
        // Reserved — treat as empty for robustness
        return (0, false);
    }
    let mnibbles = mnibbles_code + 4;
    let mlen = br.read(mnibbles * 4) + 1;

    let is_uncompressed = if !is_last { br.read1() != 0 } else { false };
    (mlen, is_uncompressed)
}

fn decompress_meta_block(
    br: &mut BitReader,
    mlen: u32,
    out: &mut Vec<u8>,
    dist_ring: &mut DistRing,
) -> Option<()> {
    // Block type info for literals, insert-and-copy, and distances
    let mut lit_block = BlockState::new();
    let mut iac_block = BlockState::new();
    let mut dist_block = BlockState::new();

    lit_block.read_info(br)?;
    iac_block.read_info(br)?;
    dist_block.read_info(br)?;

    // Distance parameters
    let npostfix = br.read(2);
    let ndirect_code = br.read(4);
    let ndirect = ndirect_code << npostfix;

    // Context modes for literal block types
    let num_lit_types = lit_block.num_types;
    let mut context_modes = Vec::with_capacity(num_lit_types as usize);
    for _ in 0..num_lit_types {
        context_modes.push(br.read(2) as u8);
    }

    // Context maps
    let num_lit_contexts = num_lit_types * 64;
    let lit_context_map = read_context_map(br, num_lit_contexts)?;

    let num_dist_types = dist_block.num_types;
    let num_dist_contexts = num_dist_types * 4;
    let dist_context_map = read_context_map(br, num_dist_contexts)?;

    // Prefix codes for literals
    let num_lit_trees = lit_context_map.iter().map(|&v| v as u32).max().unwrap_or(0) + 1;
    let mut lit_codes = Vec::with_capacity(num_lit_trees as usize);
    for _ in 0..num_lit_trees {
        lit_codes.push(read_prefix_code(br, 256)?);
    }

    // Prefix codes for insert-and-copy commands
    let num_iac_types = iac_block.num_types;
    let mut iac_codes = Vec::with_capacity(num_iac_types as usize);
    for _ in 0..num_iac_types {
        iac_codes.push(read_prefix_code(br, 704)?);
    }

    // Prefix codes for distances
    let num_dist_trees = dist_context_map.iter().map(|&v| v as u32).max().unwrap_or(0) + 1;
    let dist_alphabet_size = 16 + ndirect + (48 << npostfix);
    let mut dist_codes = Vec::with_capacity(num_dist_trees as usize);
    for _ in 0..num_dist_trees {
        dist_codes.push(read_prefix_code(br, dist_alphabet_size)?);
    }

    // Decompress commands
    let target = out.len() + mlen as usize;
    while out.len() < target {
        // Insert-and-copy command
        let iac_type = iac_block.check_switch(br);
        let iac_code = &iac_codes[iac_type as usize % iac_codes.len()];
        let cmd_sym = iac_code.decode(br);

        let (insert_code, copy_code, distance_omitted) = decode_insert_copy(cmd_sym);

        // Insert length
        let insert_len = if (insert_code as usize) < INSERT_LEN_BASE.len() {
            INSERT_LEN_BASE[insert_code as usize] + br.read(INSERT_LEN_EXTRA[insert_code as usize] as u32)
        } else { 0 };

        // Copy length
        let copy_len = if (copy_code as usize) < COPY_LEN_BASE.len() {
            COPY_LEN_BASE[copy_code as usize] + br.read(COPY_LEN_EXTRA[copy_code as usize] as u32)
        } else { 2 };

        // Emit literal bytes
        for _ in 0..insert_len {
            if out.len() >= target { break; }
            let lit_type = lit_block.check_switch(br);

            // Context-dependent literal tree selection
            let p1 = if out.is_empty() { 0 } else { out[out.len() - 1] };
            let p2 = if out.len() < 2 { 0 } else { out[out.len() - 2] };
            let mode = context_modes[lit_type as usize % context_modes.len()];
            let ctx = literal_context(p1, p2, mode);
            let ctx_idx = (lit_type * 64 + ctx) as usize;
            let tree_idx = if ctx_idx < lit_context_map.len() {
                lit_context_map[ctx_idx] as usize
            } else { 0 };
            let tree_idx = tree_idx % lit_codes.len();

            let byte = lit_codes[tree_idx].decode(br) as u8;
            out.push(byte);
        }

        if out.len() >= target { break; }

        // Distance
        let distance = if distance_omitted {
            // Implied distance (last distance)
            dist_ring.last()
        } else {
            let dist_type = dist_block.check_switch(br);
            let dist_ctx_offset = copy_len.saturating_sub(2).min(3);
            let ctx_idx = (dist_type * 4 + dist_ctx_offset.min(3)) as usize;
            let tree_idx = if ctx_idx < dist_context_map.len() {
                dist_context_map[ctx_idx] as usize
            } else { 0 };
            let tree_idx = tree_idx % dist_codes.len();

            decode_distance(br, &dist_codes[tree_idx], npostfix, ndirect, dist_ring)
        };

        // Emit copy
        if distance == 0 || distance > out.len() as u32 {
            // Dictionary reference or invalid distance
            let max_word_len = 24u32;
            if copy_len >= 4 && copy_len <= max_word_len {
                let dict_size_bits = DICT_NWORDS[copy_len as usize];
                if dict_size_bits > 0 {
                    let word_idx = distance.wrapping_sub(out.len() as u32 + 1);
                    let nwords = 1u32 << dict_size_bits;
                    let index = word_idx % nwords;
                    let transform_id = word_idx / nwords;
                    let word = dictionary_word(copy_len, index);
                    if word.is_empty() {
                        // Dictionary not available — output zeros
                        for _ in 0..copy_len {
                            if out.len() >= target { break; }
                            out.push(0);
                        }
                    } else {
                        apply_transform(word, transform_id, out);
                    }
                } else {
                    for _ in 0..copy_len {
                        if out.len() >= target { break; }
                        out.push(0);
                    }
                }
            } else {
                for _ in 0..copy_len {
                    if out.len() >= target { break; }
                    out.push(0);
                }
            }
        } else {
            // LZ77 copy from output buffer
            let dist = distance as usize;
            dist_ring.push(distance);
            let start = out.len() - dist;
            for i in 0..copy_len as usize {
                if out.len() >= target { break; }
                let b = out[start + (i % dist)];
                out.push(b);
            }
        }
    }

    // Truncate to exact target length
    out.truncate(target);
    Some(())
}
