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
    overread: bool,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            pos: 0,
            bit_buf: 0,
            bit_count: 0,
            overread: false,
        }
    }

    fn ensure_bits(&mut self, count: u8) {
        while self.bit_count < count {
            let byte = if self.pos < self.data.len() {
                let b = self.data[self.pos];
                self.pos += 1;
                b
            } else {
                self.overread = true;
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

    fn can_ensure_bits(&self, count: u8) -> bool {
        if self.bit_count >= count {
            return true;
        }
        let needed_bits = count - self.bit_count;
        let needed_bytes = needed_bits.div_ceil(8) as usize;
        self.data.len().saturating_sub(self.pos) >= needed_bytes
    }

    fn peek_bits(&self, count: u8) -> u32 {
        self.bit_buf & ((1 << count) - 1)
    }

    fn consume_bits(&mut self, count: u8) {
        self.bit_buf >>= count;
        self.bit_count -= count;
    }

    /// Return the number of input bytes actually consumed.
    /// Accounts for bits read-ahead that haven't been used.
    fn bytes_consumed(&self) -> usize {
        // pos is the next byte to read from input.
        // bit_count is the number of bits still buffered (read but not consumed).
        // Each buffered byte = 8 bits, so subtract the whole bytes worth of buffered bits.
        let buffered_bytes = (self.bit_count / 8) as usize;
        self.pos.saturating_sub(buffered_bytes)
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
            self.overread = true;
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
    lengths: [u8; MAX_SYMBOLS],
    codes: [u16; MAX_SYMBOLS],
    fast: [u16; 512],
    num_symbols: usize,
}

impl HuffmanTable {
    fn new() -> Self {
        HuffmanTable {
            counts: [0; MAX_BITS + 1],
            symbols: [0; MAX_SYMBOLS],
            lengths: [0; MAX_SYMBOLS],
            codes: [0; MAX_SYMBOLS],
            fast: [0; 512],
            num_symbols: 0,
        }
    }

    fn build(lengths: &[u8], num_symbols: usize) -> Self {
        let mut table = HuffmanTable::new();

        // Count code lengths
        for i in 0..num_symbols {
            let len = lengths[i] as usize;
            if len > 0 && len <= MAX_BITS {
                table.counts[len] += 1;
            }
        }

        // Compute offsets and canonical codes
        let mut offsets = [0u16; MAX_BITS + 1];
        let mut total = 0u16;
        for i in 1..=MAX_BITS {
            offsets[i] = total;
            total += table.counts[i];
        }

        let mut next_code = [0u32; MAX_BITS + 1];
        let mut code = 0u32;
        for bits in 1..=MAX_BITS {
            code = (code + table.counts[bits - 1] as u32) << 1;
            next_code[bits] = code;
        }

        // Assign symbols sorted by code
        for i in 0..num_symbols {
            let len = lengths[i] as usize;
            if len > 0 && len <= MAX_BITS {
                let canonical = next_code[len];
                next_code[len] += 1;
                let reversed = bit_reverse(canonical, len as u8) as u16;
                table.lengths[i] = len as u8;
                table.codes[i] = reversed;
                table.symbols[offsets[len] as usize] = i as u16;
                offsets[len] += 1;

                if len <= 9 {
                    let entry = ((len as u16) << 9) | (i as u16 + 1);
                    let fill = 1usize << (9 - len);
                    for suffix in 0..fill {
                        table.fast[reversed as usize | (suffix << len)] = entry;
                    }
                }
            }
        }

        table.num_symbols = num_symbols;
        table
    }

    fn decode(&self, reader: &mut BitReader) -> Option<u16> {
        if reader.can_ensure_bits(9) {
            reader.ensure_bits(9);
            let fast = self.fast[reader.peek_bits(9) as usize];
            if fast != 0 {
                let len = (fast >> 9) as u8;
                reader.consume_bits(len);
                return Some((fast & 0x01ff) - 1);
            }
        }

        let mut code: u32 = 0;
        for len in 1..=MAX_BITS {
            code |= reader.read_bits(1) << (len - 1);
            if reader.overread {
                return None;
            }
            for sym in 0..self.num_symbols {
                if self.lengths[sym] as usize == len && self.codes[sym] as u32 == code {
                    return Some(sym as u16);
                }
            }
        }
        None
    }
}

fn bit_reverse(code: u32, len: u8) -> u32 {
    let mut result = 0;
    for i in 0..len {
        result = (result << 1) | ((code >> i) & 1);
    }
    result
}

// ─── Fixed Huffman Tables ───────────────────────────────────────────────────

fn build_fixed_literal_table() -> HuffmanTable {
    let mut lengths = [0u8; 288];
    for i in 0..=143 {
        lengths[i] = 8;
    }
    for i in 144..=255 {
        lengths[i] = 9;
    }
    for i in 256..=279 {
        lengths[i] = 7;
    }
    for i in 280..=287 {
        lengths[i] = 8;
    }
    HuffmanTable::build(&lengths, 288)
}

fn build_fixed_distance_table() -> HuffmanTable {
    let mut lengths = [0u8; 32];
    for i in 0..32 {
        lengths[i] = 5;
    }
    HuffmanTable::build(&lengths, 32)
}

// ─── Length / Distance Extra Bits ───────────────────────────────────────────

/// Length base values for codes 257..285.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

/// Extra bits for length codes 257..285.
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// Distance base values for codes 0..29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

/// Extra bits for distance codes 0..29.
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

// ─── Code Length Order ──────────────────────────────────────────────────────

const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

// ─── Inflate ────────────────────────────────────────────────────────────────

/// Decompress DEFLATE data. Returns decompressed bytes or None on error.
pub fn inflate(compressed: &[u8]) -> Option<Vec<u8>> {
    inflate_counted(compressed).map(|(data, _)| data)
}

/// Decompress DEFLATE data. Returns (decompressed bytes, consumed input bytes).
pub fn inflate_counted(compressed: &[u8]) -> Option<(Vec<u8>, usize)> {
    inflate_counted_limited(compressed, usize::MAX)
}

/// Decompress DEFLATE data with a hard output limit.
pub fn inflate_counted_limited(compressed: &[u8], max_output: usize) -> Option<(Vec<u8>, usize)> {
    let mut reader = BitReader::new(compressed);
    let mut output = Vec::new();

    loop {
        let bfinal = reader.read_bits(1);
        let btype = reader.read_bits(2);
        if reader.overread {
            return None;
        }

        match btype {
            0 => {
                // Stored block
                reader.align_to_byte();
                let lo = reader.read_byte_aligned();
                let hi = reader.read_byte_aligned();
                let len = (lo as u16) | ((hi as u16) << 8);
                let _nlo = reader.read_byte_aligned();
                let _nhi = reader.read_byte_aligned();
                if reader.overread {
                    return None;
                }
                // nlen is one's complement of len — skip validation
                if output.len().checked_add(len as usize)? > max_output {
                    return None;
                }
                for _ in 0..len {
                    let byte = reader.read_byte_aligned();
                    if reader.overread {
                        return None;
                    }
                    output.push(byte);
                }
            }
            1 => {
                // Fixed Huffman
                let lit_table = build_fixed_literal_table();
                let dist_table = build_fixed_distance_table();
                decode_block(
                    &mut reader,
                    &lit_table,
                    &dist_table,
                    &mut output,
                    max_output,
                )?;
            }
            2 => {
                // Dynamic Huffman
                let hlit = reader.read_bits(5) as usize + 257;
                let hdist = reader.read_bits(5) as usize + 1;
                let hclen = reader.read_bits(4) as usize + 4;
                if reader.overread {
                    return None;
                }

                // Read code length code lengths
                let mut cl_lengths = [0u8; 19];
                for i in 0..hclen {
                    cl_lengths[CL_ORDER[i]] = reader.read_bits(3) as u8;
                    if reader.overread {
                        return None;
                    }
                }

                let cl_table = HuffmanTable::build(&cl_lengths, 19);

                // Read literal/length + distance code lengths
                let total = hlit + hdist;
                let mut lengths = vec![0u8; total];
                let mut i = 0;
                while i < total {
                    let sym = cl_table.decode(&mut reader)?;
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
                                if i < total {
                                    lengths[i] = prev;
                                    i += 1;
                                }
                            }
                        }
                        17 => {
                            // Repeat 0 for 3..10 times
                            let repeat = reader.read_bits(3) as usize + 3;
                            for _ in 0..repeat {
                                if i < total {
                                    lengths[i] = 0;
                                    i += 1;
                                }
                            }
                        }
                        18 => {
                            // Repeat 0 for 11..138 times
                            let repeat = reader.read_bits(7) as usize + 11;
                            for _ in 0..repeat {
                                if i < total {
                                    lengths[i] = 0;
                                    i += 1;
                                }
                            }
                        }
                        _ => return None,
                    }
                }

                let lit_table = HuffmanTable::build(&lengths[..hlit], hlit);
                let dist_table = HuffmanTable::build(&lengths[hlit..], hdist);
                decode_block(
                    &mut reader,
                    &lit_table,
                    &dist_table,
                    &mut output,
                    max_output,
                )?;
            }
            _ => return None, // Reserved/invalid
        }

        if bfinal != 0 {
            break;
        }
    }

    // Calculate consumed bytes: pos is how far we read, minus buffered bits
    let consumed = reader.bytes_consumed();
    if reader.overread {
        return None;
    }
    Some((output, consumed))
}

fn decode_block(
    reader: &mut BitReader,
    lit_table: &HuffmanTable,
    dist_table: &HuffmanTable,
    output: &mut Vec<u8>,
    max_output: usize,
) -> Option<()> {
    loop {
        let sym = lit_table.decode(reader)? as usize;

        if sym == 256 {
            // End of block
            return Some(());
        }

        if sym < 256 {
            // Literal byte
            if output.len() >= max_output {
                return None;
            }
            output.push(sym as u8);
        } else {
            // Length/distance pair
            let len_idx = sym - 257;
            if len_idx >= 29 {
                return None;
            }
            let length =
                LENGTH_BASE[len_idx] as usize + reader.read_bits(LENGTH_EXTRA[len_idx]) as usize;
            if reader.overread {
                return None;
            }

            let dist_sym = dist_table.decode(reader)? as usize;
            if dist_sym >= 30 {
                return None;
            }
            let distance =
                DIST_BASE[dist_sym] as usize + reader.read_bits(DIST_EXTRA[dist_sym]) as usize;
            if reader.overread {
                return None;
            }

            // Copy from sliding window
            if distance > output.len() {
                return None;
            }
            if output.len().checked_add(length)? > max_output {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_deflate_returns_none_quickly() {
        let compressed = crate::deflate::deflate(b"hello from a deliberately truncated stream");
        assert!(compressed.len() > 2);

        let truncated_len = compressed.len() / 2;
        assert!(
            inflate_counted_limited(&compressed[..truncated_len], 4096).is_none(),
            "truncated stream unexpectedly inflated"
        );

        let (inflated, consumed) =
            inflate_counted_limited(&compressed, 4096).expect("complete stream should inflate");
        assert_eq!(inflated, b"hello from a deliberately truncated stream");
        assert!(consumed <= compressed.len());
    }
}
