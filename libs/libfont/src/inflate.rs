//! Minimal DEFLATE (RFC 1951) decompressor for PNG bitmap decoding.
//!
//! Supports all three block types: uncompressed, fixed Huffman, dynamic Huffman.
//! Designed for small payloads (emoji PNGs ≤ 50 KB decompressed).

use alloc::vec;
use alloc::vec::Vec;

// ── Bit reader (LSB-first) ──────────────────────────────────────────

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bits: u32,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0, bits: 0, nbits: 0 }
    }

    #[inline]
    fn read(&mut self, n: u32) -> u32 {
        while self.nbits < n {
            if self.pos >= self.data.len() { return 0; }
            self.bits |= (self.data[self.pos] as u32) << self.nbits;
            self.pos += 1;
            self.nbits += 8;
        }
        let val = self.bits & ((1u32 << n) - 1);
        self.bits >>= n;
        self.nbits -= n;
        val
    }

    /// Align to next byte boundary (discard remaining bits in current byte).
    fn align(&mut self) {
        self.bits = 0;
        self.nbits = 0;
    }
}

// ── Huffman table (canonical) ────────────────────────────────────────

const MAX_BITS: usize = 15;

struct HuffTree {
    counts: [u16; MAX_BITS + 1],
    symbols: Vec<u16>,
}

impl HuffTree {
    fn build(lengths: &[u8]) -> Self {
        let mut counts = [0u16; MAX_BITS + 1];
        for &l in lengths {
            if l > 0 && (l as usize) <= MAX_BITS {
                counts[l as usize] += 1;
            }
        }
        let mut offsets = [0u16; MAX_BITS + 1];
        let mut total = 0u16;
        for i in 1..=MAX_BITS {
            offsets[i] = total;
            total += counts[i];
        }
        let mut symbols = vec![0u16; total as usize];
        for (sym, &l) in lengths.iter().enumerate() {
            if l > 0 && (l as usize) <= MAX_BITS {
                let idx = offsets[l as usize] as usize;
                if idx < symbols.len() {
                    symbols[idx] = sym as u16;
                }
                offsets[l as usize] += 1;
            }
        }
        HuffTree { counts, symbols }
    }

    fn decode(&self, br: &mut BitReader) -> u16 {
        let mut code = 0u32;
        let mut first = 0u32;
        let mut index = 0usize;
        for bits in 1..=MAX_BITS {
            code = (code << 1) | br.read(1);
            let count = self.counts[bits] as u32;
            if code.wrapping_sub(first) < count {
                return self.symbols[index + (code - first) as usize];
            }
            index += count as usize;
            first = (first + count) << 1;
        }
        0
    }
}

// ── Static tables ────────────────────────────────────────────────────

static LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31,
    35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
];
static LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2,
    3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
static DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193,
    257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
static DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6,
    7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];

/// Code-length code order for dynamic Huffman.
static CL_ORDER: [u8; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn fixed_lit_tree() -> HuffTree {
    let mut lengths = [0u8; 288];
    for i in 0..=143 { lengths[i] = 8; }
    for i in 144..=255 { lengths[i] = 9; }
    for i in 256..=279 { lengths[i] = 7; }
    for i in 280..=287 { lengths[i] = 8; }
    HuffTree::build(&lengths)
}

fn fixed_dist_tree() -> HuffTree {
    let lengths = [5u8; 32];
    HuffTree::build(&lengths)
}

// ── Core inflate ─────────────────────────────────────────────────────

pub fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut br = BitReader::new(data);
    let mut out = Vec::new();

    loop {
        let bfinal = br.read(1);
        let btype = br.read(2);

        match btype {
            0 => {
                // Uncompressed block
                br.align();
                if br.pos + 4 > br.data.len() { return None; }
                let len = br.data[br.pos] as u16 | ((br.data[br.pos + 1] as u16) << 8);
                br.pos += 4; // skip LEN + NLEN
                if br.pos + len as usize > br.data.len() { return None; }
                out.extend_from_slice(&br.data[br.pos..br.pos + len as usize]);
                br.pos += len as usize;
            }
            1 => {
                let lit = fixed_lit_tree();
                let dist = fixed_dist_tree();
                inflate_block(&mut br, &lit, &dist, &mut out)?;
            }
            2 => {
                let hlit = br.read(5) as usize + 257;
                let hdist = br.read(5) as usize + 1;
                let hclen = br.read(4) as usize + 4;

                let mut cl_lengths = [0u8; 19];
                for i in 0..hclen {
                    cl_lengths[CL_ORDER[i] as usize] = br.read(3) as u8;
                }
                let cl_tree = HuffTree::build(&cl_lengths);

                let total = hlit + hdist;
                let mut lengths = vec![0u8; total];
                let mut i = 0;
                while i < total {
                    let sym = cl_tree.decode(&mut br);
                    match sym {
                        0..=15 => {
                            lengths[i] = sym as u8;
                            i += 1;
                        }
                        16 => {
                            let rep = br.read(2) as usize + 3;
                            let val = if i > 0 { lengths[i - 1] } else { 0 };
                            for _ in 0..rep {
                                if i < total { lengths[i] = val; i += 1; }
                            }
                        }
                        17 => {
                            let rep = br.read(3) as usize + 3;
                            i += rep.min(total - i);
                        }
                        18 => {
                            let rep = br.read(7) as usize + 11;
                            i += rep.min(total - i);
                        }
                        _ => return None,
                    }
                }

                let lit = HuffTree::build(&lengths[..hlit]);
                let dist = HuffTree::build(&lengths[hlit..]);
                inflate_block(&mut br, &lit, &dist, &mut out)?;
            }
            _ => return None,
        }

        if bfinal != 0 { break; }
    }

    Some(out)
}

fn inflate_block(
    br: &mut BitReader, lit: &HuffTree, dist: &HuffTree, out: &mut Vec<u8>,
) -> Option<()> {
    loop {
        let sym = lit.decode(br) as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Some(());
        } else {
            let li = sym - 257;
            if li >= 29 { return None; }
            let length = LEN_BASE[li] as usize + br.read(LEN_EXTRA[li] as u32) as usize;

            let di = dist.decode(br) as usize;
            if di >= 30 { return None; }
            let distance = DIST_BASE[di] as usize + br.read(DIST_EXTRA[di] as u32) as usize;

            if distance == 0 || distance > out.len() { return None; }
            let start = out.len() - distance;
            for i in 0..length {
                let b = out[start + i % distance];
                out.push(b);
            }
        }
    }
}

// ── Zlib wrapper ─────────────────────────────────────────────────────

/// Decompress zlib-wrapped data (2-byte header + DEFLATE + Adler-32).
pub fn zlib_decompress(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 2 { return None; }
    let cmf = data[0];
    if cmf & 0x0F != 8 { return None; } // CM must be 8 (deflate)
    let flg = data[1];
    let start = if flg & 0x20 != 0 { 6 } else { 2 }; // skip FDICT if present
    if start >= data.len() { return None; }
    inflate(&data[start..])
}
