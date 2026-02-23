//! DEFLATE compression (RFC 1951).
//!
//! Implements stored blocks (no compression) and fixed Huffman encoding with
//! LZ77 matching for reasonable compression ratios.

use alloc::vec::Vec;

// ─── Bit Writer ─────────────────────────────────────────────────────────────

struct BitWriter {
    output: Vec<u8>,
    bit_buf: u32,
    bit_count: u8,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter { output: Vec::new(), bit_buf: 0, bit_count: 0 }
    }

    fn write_bits(&mut self, value: u32, count: u8) {
        self.bit_buf |= value << self.bit_count;
        self.bit_count += count;
        while self.bit_count >= 8 {
            self.output.push(self.bit_buf as u8);
            self.bit_buf >>= 8;
            self.bit_count -= 8;
        }
    }

    fn flush(&mut self) {
        if self.bit_count > 0 {
            self.output.push(self.bit_buf as u8);
            self.bit_buf = 0;
            self.bit_count = 0;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        self.flush();
        self.output
    }
}

// ─── Fixed Huffman Codes ────────────────────────────────────────────────────

/// Encode a literal/length symbol using fixed Huffman codes.
fn encode_fixed_literal(writer: &mut BitWriter, sym: u16) {
    // Fixed Huffman code table (reversed bit order for DEFLATE):
    // 0-143:   8-bit codes 00110000..10111111 (0x30..0xBF)
    // 144-255: 9-bit codes 110010000..111111111 (0x190..0x1FF)
    // 256-279: 7-bit codes 0000000..0010111 (0x00..0x17)
    // 280-287: 8-bit codes 11000000..11000111 (0xC0..0xC7)
    if sym <= 143 {
        let code = 0x30 + sym as u32;
        writer.write_bits(reverse_bits(code, 8), 8);
    } else if sym <= 255 {
        let code = 0x190 + (sym as u32 - 144);
        writer.write_bits(reverse_bits(code, 9), 9);
    } else if sym <= 279 {
        let code = sym as u32 - 256;
        writer.write_bits(reverse_bits(code, 7), 7);
    } else {
        let code = 0xC0 + (sym as u32 - 280);
        writer.write_bits(reverse_bits(code, 8), 8);
    }
}

/// Encode a distance symbol using fixed Huffman codes (all 5-bit).
fn encode_fixed_distance(writer: &mut BitWriter, sym: u8) {
    writer.write_bits(reverse_bits(sym as u32, 5), 5);
}

/// Reverse the lowest `bits` bits of `value`.
fn reverse_bits(value: u32, bits: u8) -> u32 {
    let mut result = 0u32;
    let mut v = value;
    for _ in 0..bits {
        result = (result << 1) | (v & 1);
        v >>= 1;
    }
    result
}

// ─── Length / Distance Encoding ─────────────────────────────────────────────

/// Length base values for codes 257..285.
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

fn find_length_code(length: u16) -> (u16, u8, u16) {
    for i in (0..29).rev() {
        if length >= LENGTH_BASE[i] {
            let extra_val = length - LENGTH_BASE[i];
            return (257 + i as u16, LENGTH_EXTRA[i], extra_val);
        }
    }
    (257, 0, 0)
}

fn find_distance_code(dist: u16) -> (u8, u8, u16) {
    for i in (0..30).rev() {
        if dist >= DIST_BASE[i] {
            let extra_val = dist - DIST_BASE[i];
            return (i as u8, DIST_EXTRA[i], extra_val);
        }
    }
    (0, 0, 0)
}

// ─── LZ77 Hash Chain ───────────────────────────────────────────────────────

const HASH_SIZE: usize = 4096;
const HASH_MASK: usize = HASH_SIZE - 1;
const MAX_MATCH: usize = 258;
const MIN_MATCH: usize = 3;
const WINDOW_SIZE: usize = 32768;

fn hash3(data: &[u8], pos: usize) -> usize {
    if pos + 2 >= data.len() {
        return 0;
    }
    let h = (data[pos] as usize) ^ ((data[pos + 1] as usize) << 4) ^ ((data[pos + 2] as usize) << 8);
    h & HASH_MASK
}

/// Find best match at `pos` using hash chain. Returns (length, distance) or (0, 0).
fn find_match(data: &[u8], pos: usize, head: &[u32; HASH_SIZE], prev: &[u32]) -> (usize, usize) {
    if pos + MIN_MATCH > data.len() {
        return (0, 0);
    }

    let h = hash3(data, pos);
    let mut chain = head[h];
    let mut best_len = 0usize;
    let mut best_dist = 0usize;
    let mut chain_limit = 64; // Max chain depth to search

    while chain != u32::MAX && chain_limit > 0 {
        let candidate = chain as usize;
        if candidate >= pos || pos - candidate > WINDOW_SIZE {
            break;
        }
        let dist = pos - candidate;

        // Compare
        let max_len = (data.len() - pos).min(MAX_MATCH);
        let mut len = 0;
        while len < max_len && data[candidate + len] == data[pos + len] {
            len += 1;
        }

        if len >= MIN_MATCH && len > best_len {
            best_len = len;
            best_dist = dist;
            if len == MAX_MATCH {
                break;
            }
        }

        chain = prev[candidate % WINDOW_SIZE];
        chain_limit -= 1;
    }

    (best_len, best_dist)
}

// ─── Deflate ────────────────────────────────────────────────────────────────

/// Compress data using DEFLATE with fixed Huffman codes and LZ77.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        // Empty stored block
        let mut writer = BitWriter::new();
        writer.write_bits(1, 1); // bfinal
        writer.write_bits(1, 2); // btype = fixed
        encode_fixed_literal(&mut writer, 256); // end of block
        return writer.finish();
    }

    let mut writer = BitWriter::new();
    writer.write_bits(1, 1); // bfinal
    writer.write_bits(1, 2); // btype = fixed Huffman

    // Initialize hash chains
    let mut head = [u32::MAX; HASH_SIZE];
    let mut prev = alloc::vec![u32::MAX; WINDOW_SIZE];
    let mut pos = 0;

    while pos < data.len() {
        let (match_len, match_dist) = find_match(data, pos, &head, &prev);

        if match_len >= MIN_MATCH {
            // Emit length/distance pair
            let (len_code, len_extra_bits, len_extra_val) = find_length_code(match_len as u16);
            encode_fixed_literal(&mut writer, len_code);
            if len_extra_bits > 0 {
                writer.write_bits(len_extra_val as u32, len_extra_bits);
            }

            let (dist_code, dist_extra_bits, dist_extra_val) = find_distance_code(match_dist as u16);
            encode_fixed_distance(&mut writer, dist_code);
            if dist_extra_bits > 0 {
                writer.write_bits(dist_extra_val as u32, dist_extra_bits);
            }

            // Update hash for all matched positions
            for i in 0..match_len {
                if pos + i + MIN_MATCH <= data.len() {
                    let h = hash3(data, pos + i);
                    prev[(pos + i) % WINDOW_SIZE] = head[h];
                    head[h] = (pos + i) as u32;
                }
            }
            pos += match_len;
        } else {
            // Emit literal
            encode_fixed_literal(&mut writer, data[pos] as u16);

            // Update hash
            if pos + MIN_MATCH <= data.len() {
                let h = hash3(data, pos);
                prev[pos % WINDOW_SIZE] = head[h];
                head[h] = pos as u32;
            }
            pos += 1;
        }
    }

    encode_fixed_literal(&mut writer, 256); // End of block
    writer.finish()
}

/// Store data without compression (stored blocks).
pub fn store(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        let chunk = (data.len() - offset).min(65535);
        let is_final = offset + chunk >= data.len();

        // Block header
        output.push(if is_final { 1 } else { 0 }); // bfinal | btype=00

        let len = chunk as u16;
        let nlen = !len;
        output.push(len as u8);
        output.push((len >> 8) as u8);
        output.push(nlen as u8);
        output.push((nlen >> 8) as u8);

        output.extend_from_slice(&data[offset..offset + chunk]);
        offset += chunk;
    }

    if data.is_empty() {
        // Empty stored block
        output.push(1); // bfinal, btype=00
        output.push(0);
        output.push(0);
        output.push(0xFF);
        output.push(0xFF);
    }

    output
}
