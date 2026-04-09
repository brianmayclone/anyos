//! DEFLATE decompression (RFC 1951).
//!
//! Supports stored blocks, fixed Huffman, and dynamic Huffman.

use alloc::vec;
use alloc::vec::Vec;

// ─── Bit Reader ─────────────────────────────────────────────────────────────

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_count: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0, bit_buf: 0, bit_count: 0 }
    }

    fn ensure_bits(&mut self, count: u8) {
        while self.bit_count < count {
            let byte = if self.pos < self.data.len() {
                let b = self.data[self.pos];
                self.pos += 1;
                b
            } else {
                0
            };
            self.bit_buf |= (byte as u32) << self.bit_count;
            self.bit_count += 8;
        }
    }

    fn read_bits(&mut self, count: u8) -> u32 {
        self.ensure_bits(count);
        let val = self.bit_buf & ((1 << count) - 1);
        self.bit_buf >>= count;
        self.bit_count -= count;
        val
    }

    fn read_byte_aligned(&mut self) -> u8 {
        // Discard remaining bits in current byte
        self.bit_buf = 0;
        self.bit_count = 0;
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            b
        } else {
            0
        }
    }

    fn align_to_byte(&mut self) {
        self.bit_buf = 0;
        self.bit_count = 0;
    }
}

// ─── Huffman Decoder ────────────────────────────────────────────────────────

const MAX_BITS: usize = 15;
const MAX_SYMBOLS: usize = 288;

struct HuffmanTable {
    counts: [u16; MAX_BITS + 1],
    symbols: [u16; MAX_SYMBOLS],
}

impl HuffmanTable {
    fn new() -> Self {
        HuffmanTable {
            counts: [0; MAX_BITS + 1],
            symbols: [0; MAX_SYMBOLS],
        }
    }

    fn build(lengths: &[u8], num_symbols: usize) -> Self {
        let mut table = HuffmanTable::new();

        // Count code lengths
        for i in 0..num_symbols {
            if (lengths[i] as usize) <= MAX_BITS {
                table.counts[lengths[i] as usize] += 1;
            }
        }

        // Compute offsets
        let mut offsets = [0u16; MAX_BITS + 1];
        let mut total = 0u16;
        for i in 1..=MAX_BITS {
            offsets[i] = total;
            total += table.counts[i];
        }

        // Assign symbols sorted by code
        for i in 0..num_symbols {
            let len = lengths[i] as usize;
            if len > 0 && len <= MAX_BITS {
                table.symbols[offsets[len] as usize] = i as u16;
                offsets[len] += 1;
            }
        }

        table
    }

    fn decode(&self, reader: &mut BitReader) -> u16 {
        let mut code: u32 = 0;
        let mut first: u32 = 0;
        let mut index: u32 = 0;

        for len in 1..=MAX_BITS {
            code |= reader.read_bits(1);
            let count = self.counts[len] as u32;
            if code < first + count {
                return self.symbols[(index + code - first) as usize];
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        0 // Should not happen with valid data
    }
}

// ─── Fixed Huffman Tables ───────────────────────────────────────────────────

fn build_fixed_literal_table() -> HuffmanTable {
    let mut lengths = [0u8; 288];
    for i in 0..=143 { lengths[i] = 8; }
    for i in 144..=255 { lengths[i] = 9; }
    for i in 256..=279 { lengths[i] = 7; }
    for i in 280..=287 { lengths[i] = 8; }
    HuffmanTable::build(&lengths, 288)
}

fn build_fixed_distance_table() -> HuffmanTable {
    let mut lengths = [0u8; 32];
    for i in 0..32 { lengths[i] = 5; }
    HuffmanTable::build(&lengths, 32)
}

// ─── Length / Distance Extra Bits ───────────────────────────────────────────

/// Length base values for codes 257..285.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13,
    15, 17, 19, 23, 27, 31, 35, 43, 51, 59,
    67, 83, 99, 115, 131, 163, 195, 227, 258,
];

/// Extra bits for length codes 257..285.
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1,
    1, 1, 2, 2, 2, 2, 3, 3, 3, 3,
    4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// Distance base values for codes 0..29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25,
    33, 49, 65, 97, 129, 193, 257, 385, 513, 769,
    1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

/// Extra bits for distance codes 0..29.
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3,
    4, 4, 5, 5, 6, 6, 7, 7, 8, 8,
    9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];

// ─── Code Length Order ──────────────────────────────────────────────────────

const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

// ─── Inflate ────────────────────────────────────────────────────────────────

/// Decompress DEFLATE data. Returns decompressed bytes or None on error.
pub fn inflate(compressed: &[u8]) -> Option<Vec<u8>> {
    let mut reader = BitReader::new(compressed);
    let mut output = Vec::new();

    loop {
        let bfinal = reader.read_bits(1);
        let btype = reader.read_bits(2);

        match btype {
            0 => {
                // Stored block
                reader.align_to_byte();
                let lo = reader.read_byte_aligned();
                let hi = reader.read_byte_aligned();
                let len = (lo as u16) | ((hi as u16) << 8);
                let _nlo = reader.read_byte_aligned();
                let _nhi = reader.read_byte_aligned();
                // nlen is one's complement of len — skip validation
                for _ in 0..len {
                    output.push(reader.read_byte_aligned());
                }
            }
            1 => {
                // Fixed Huffman
                let lit_table = build_fixed_literal_table();
                let dist_table = build_fixed_distance_table();
                decode_block(&mut reader, &lit_table, &dist_table, &mut output)?;
            }
            2 => {
                // Dynamic Huffman
                let hlit = reader.read_bits(5) as usize + 257;
                let hdist = reader.read_bits(5) as usize + 1;
                let hclen = reader.read_bits(4) as usize + 4;

                // Read code length code lengths
                let mut cl_lengths = [0u8; 19];
                for i in 0..hclen {
                    cl_lengths[CL_ORDER[i]] = reader.read_bits(3) as u8;
                }

                let cl_table = HuffmanTable::build(&cl_lengths, 19);

                // Read literal/length + distance code lengths
                let total = hlit + hdist;
                let mut lengths = vec![0u8; total];
                let mut i = 0;
                while i < total {
                    let sym = cl_table.decode(&mut reader);
                    match sym {
                        0..=15 => {
                            lengths[i] = sym as u8;
                            i += 1;
                        }
                        16 => {
                            // Repeat previous 3..6 times
                            let repeat = reader.read_bits(2) as usize + 3;
                            let prev = if i > 0 { lengths[i - 1] } else { 0 };
                            for _ in 0..repeat {
                                if i < total { lengths[i] = prev; i += 1; }
                            }
                        }
                        17 => {
                            // Repeat 0 for 3..10 times
                            let repeat = reader.read_bits(3) as usize + 3;
                            for _ in 0..repeat {
                                if i < total { lengths[i] = 0; i += 1; }
                            }
                        }
                        18 => {
                            // Repeat 0 for 11..138 times
                            let repeat = reader.read_bits(7) as usize + 11;
                            for _ in 0..repeat {
                                if i < total { lengths[i] = 0; i += 1; }
                            }
                        }
                        _ => return None,
                    }
                }

                let lit_table = HuffmanTable::build(&lengths[..hlit], hlit);
                let dist_table = HuffmanTable::build(&lengths[hlit..], hdist);
                decode_block(&mut reader, &lit_table, &dist_table, &mut output)?;
            }
            _ => return None, // Reserved/invalid
        }

        if bfinal != 0 {
            break;
        }
    }

    Some(output)
}

fn decode_block(
    reader: &mut BitReader,
    lit_table: &HuffmanTable,
    dist_table: &HuffmanTable,
    output: &mut Vec<u8>,
) -> Option<()> {
    loop {
        let sym = lit_table.decode(reader) as usize;

        if sym == 256 {
            // End of block
            return Some(());
        }

        if sym < 256 {
            // Literal byte
            output.push(sym as u8);
        } else {
            // Length/distance pair
            let len_idx = sym - 257;
            if len_idx >= 29 {
                return None;
            }
            let length = LENGTH_BASE[len_idx] as usize
                + reader.read_bits(LENGTH_EXTRA[len_idx]) as usize;

            let dist_sym = dist_table.decode(reader) as usize;
            if dist_sym >= 30 {
                return None;
            }
            let distance = DIST_BASE[dist_sym] as usize
                + reader.read_bits(DIST_EXTRA[dist_sym]) as usize;

            // Copy from sliding window
            if distance > output.len() {
                return None;
            }
            let start = output.len() - distance;
            for i in 0..length {
                let b = output[start + (i % distance)];
                output.push(b);
            }
        }
    }
}
