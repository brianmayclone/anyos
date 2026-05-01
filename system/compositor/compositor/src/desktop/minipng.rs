use alloc::vec;
use alloc::vec::Vec;

fn read_u32_be(data: &[u8], off: usize) -> u32 {
    (data[off] as u32) << 24
        | (data[off + 1] as u32) << 16
        | (data[off + 2] as u32) << 8
        | data[off + 3] as u32
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i32 + b as i32 - c as i32;
    let pa = (p - a as i32).abs();
    let pb = (p - b as i32).abs();
    let pc = (p - c as i32).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bits: u32,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bits: 0,
            nbits: 0,
        }
    }

    fn read(&mut self, n: u32) -> u32 {
        while self.nbits < n {
            if self.pos >= self.data.len() {
                return 0;
            }
            self.bits |= (self.data[self.pos] as u32) << self.nbits;
            self.pos += 1;
            self.nbits += 8;
        }
        let val = self.bits & ((1u32 << n) - 1);
        self.bits >>= n;
        self.nbits -= n;
        val
    }

    fn align(&mut self) {
        self.bits = 0;
        self.nbits = 0;
    }
}

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
        Self { counts, symbols }
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

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const CL_ORDER: [u8; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn fixed_lit_tree() -> HuffTree {
    let mut lengths = [0u8; 288];
    for item in lengths.iter_mut().take(144) {
        *item = 8;
    }
    for item in lengths.iter_mut().take(256).skip(144) {
        *item = 9;
    }
    for item in lengths.iter_mut().take(280).skip(256) {
        *item = 7;
    }
    for item in lengths.iter_mut().skip(280) {
        *item = 8;
    }
    HuffTree::build(&lengths)
}

fn fixed_dist_tree() -> HuffTree {
    HuffTree::build(&[5u8; 32])
}

fn inflate_block(
    br: &mut BitReader,
    lit: &HuffTree,
    dist: &HuffTree,
    out: &mut Vec<u8>,
) -> Option<()> {
    loop {
        let sym = lit.decode(br) as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Some(());
        } else {
            let li = sym - 257;
            if li >= 29 {
                return None;
            }
            let length = LEN_BASE[li] as usize + br.read(LEN_EXTRA[li] as u32) as usize;
            let di = dist.decode(br) as usize;
            if di >= 30 {
                return None;
            }
            let distance = DIST_BASE[di] as usize + br.read(DIST_EXTRA[di] as u32) as usize;
            if distance == 0 || distance > out.len() {
                return None;
            }
            let start = out.len() - distance;
            for i in 0..length {
                let b = out[start + i % distance];
                out.push(b);
            }
        }
    }
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut br = BitReader::new(data);
    let mut out = Vec::new();
    loop {
        let bfinal = br.read(1);
        let btype = br.read(2);
        match btype {
            0 => {
                br.align();
                if br.pos + 4 > br.data.len() {
                    return None;
                }
                let len = br.data[br.pos] as u16 | ((br.data[br.pos + 1] as u16) << 8);
                br.pos += 4;
                if br.pos + len as usize > br.data.len() {
                    return None;
                }
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
                let mut i = 0usize;
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
                                if i < total {
                                    lengths[i] = val;
                                    i += 1;
                                }
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
        if bfinal != 0 {
            break;
        }
    }
    Some(out)
}

fn zlib_decompress(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 2 {
        return None;
    }
    let cmf = data[0];
    if cmf & 0x0f != 8 {
        return None;
    }
    let flg = data[1];
    let start = if flg & 0x20 != 0 { 6 } else { 2 };
    if start >= data.len() {
        return None;
    }
    inflate(&data[start..])
}

pub fn decode_png_argb32(data: &[u8]) -> Option<(Vec<u32>, u32, u32)> {
    if data.len() < 8 {
        return None;
    }
    if data[0] != 0x89 || &data[1..4] != b"PNG" || &data[4..8] != &[0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }

    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color_type = 0u8;
    let mut idat_data = Vec::new();
    let mut palette = [(0u8, 0u8, 0u8); 256];
    let mut palette_len = 0usize;
    let mut trns_alpha = [255u8; 256];

    while pos + 8 <= data.len() {
        let length = read_u32_be(data, pos) as usize;
        let ctype = &data[pos + 4..pos + 8];
        pos += 8;
        if pos + length > data.len() {
            break;
        }
        let chunk = &data[pos..pos + length];
        if ctype == b"IHDR" {
            if length < 13 {
                return None;
            }
            width = read_u32_be(chunk, 0);
            height = read_u32_be(chunk, 4);
            let bit_depth = chunk[8];
            color_type = chunk[9];
            if bit_depth != 8 || (color_type != 6 && color_type != 2 && color_type != 3) {
                return None;
            }
        } else if ctype == b"PLTE" {
            let count = length / 3;
            palette_len = count.min(256);
            for i in 0..palette_len {
                palette[i] = (chunk[i * 3], chunk[i * 3 + 1], chunk[i * 3 + 2]);
            }
        } else if ctype == b"tRNS" {
            if color_type == 3 {
                let count = length.min(256);
                trns_alpha[..count].copy_from_slice(&chunk[..count]);
            }
        } else if ctype == b"IDAT" {
            idat_data.extend_from_slice(chunk);
        } else if ctype == b"IEND" {
            break;
        }
        pos += length + 4;
    }

    if width == 0 || height == 0 || idat_data.is_empty() {
        return None;
    }
    if color_type == 3 && palette_len == 0 {
        return None;
    }

    let raw = zlib_decompress(&idat_data)?;
    let bpp = match color_type {
        6 => 4usize,
        2 => 3usize,
        3 => 1usize,
        _ => return None,
    };
    let row_bytes = width as usize * bpp;
    let stride = row_bytes + 1;
    if raw.len() < height as usize * stride {
        return None;
    }

    let mut pixels = vec![0u32; width as usize * height as usize];
    let mut prev_row = vec![0u8; row_bytes];

    for y in 0..height as usize {
        let row_start = y * stride;
        let filter = raw[row_start];
        let row_data = &raw[row_start + 1..row_start + 1 + row_bytes];
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
                _ => return None,
            };
        }

        for px in 0..width as usize {
            let argb = match color_type {
                6 => {
                    let si = px * 4;
                    let r = cur_row[si] as u32;
                    let g = cur_row[si + 1] as u32;
                    let b = cur_row[si + 2] as u32;
                    let a = cur_row[si + 3] as u32;
                    (a << 24) | (r << 16) | (g << 8) | b
                }
                2 => {
                    let si = px * 3;
                    let r = cur_row[si] as u32;
                    let g = cur_row[si + 1] as u32;
                    let b = cur_row[si + 2] as u32;
                    0xff00_0000 | (r << 16) | (g << 8) | b
                }
                3 => {
                    let idx = cur_row[px] as usize;
                    if idx >= palette_len {
                        0
                    } else {
                        let (r, g, b) = palette[idx];
                        let a = trns_alpha[idx] as u32;
                        (a << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
                    }
                }
                _ => return None,
            };
            pixels[y * width as usize + px] = argb;
        }
        prev_row = cur_row;
    }

    Some((pixels, width, height))
}
