// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Baseline JPEG (JFIF) decoder.
//!
//! Supports SOF0 (baseline DCT), 8-bit precision, 1-3 components,
//! chroma subsampling 4:4:4, 4:2:2, and 4:2:0.
//!
//! All working memory is caller-provided via a scratch buffer.
//! Fixed-point integer math only (no FPU).

use crate::types::*;
use crate::jpeg_tables::*;

// ---------------------------------------------------------------------------
// JPEG markers
// ---------------------------------------------------------------------------

const M_SOI: u8 = 0xD8;
const M_EOI: u8 = 0xD9;
const M_SOF0: u8 = 0xC0; // Baseline DCT
const M_DHT: u8 = 0xC4;
const M_DQT: u8 = 0xDB;
const M_DRI: u8 = 0xDD;
const M_SOS: u8 = 0xDA;

// Fixed-point precision for IDCT
const IDCT_BITS: i32 = 13;
const IDCT_HALF: i32 = 1 << (IDCT_BITS - 1);

// Maximum image dimensions we support
const MAX_DIM: u32 = 16384;
// Max components (Y, Cb, Cr)
const MAX_COMP: usize = 3;
// Max quantization tables
const MAX_QTABLES: usize = 4;

// ---------------------------------------------------------------------------
// Helper: read big-endian values from byte slice
// ---------------------------------------------------------------------------

fn read_u16_be(data: &[u8], off: usize) -> u16 {
    ((data[off] as u16) << 8) | data[off + 1] as u16
}

// ---------------------------------------------------------------------------
// Huffman table (flat lookup + slow fallback)
// ---------------------------------------------------------------------------

/// A Huffman decoding table.
///
/// Uses an 8-bit lookup table for fast decode of short codes, and a linear
/// walk for codes longer than 8 bits (rare in typical JPEG files).
struct HuffTable {
    /// Fast lookup: index by next 8 bits from the stream.
    /// Value bits [15:8] = decoded symbol, bits [7:0] = code length (0 = invalid).
    fast: [u16; 256],
    /// Full code table for slow path.
    /// codes[i] = (code_value, code_length, symbol)
    codes: [(u16, u8, u8); 256],
    num_codes: usize,
}

impl HuffTable {
    const fn new() -> Self {
        HuffTable {
            fast: [0u16; 256],
            codes: [(0u16, 0u8, 0u8); 256],
            num_codes: 0,
        }
    }

    /// Build from JPEG DHT segment data (counts[16] + symbols[]).
    fn build(&mut self, counts: &[u8; 16], symbols: &[u8]) {
        self.fast = [0u16; 256];
        self.num_codes = 0;

        let mut code: u16 = 0;
        let mut sym_idx: usize = 0;

        for bits in 0..16u8 {
            let length = bits + 1; // 1..16
            let count = counts[bits as usize] as usize;
            for _ in 0..count {
                if sym_idx >= symbols.len() || self.num_codes >= 256 {
                    return;
                }
                let sym = symbols[sym_idx];
                self.codes[self.num_codes] = (code, length, sym);
                self.num_codes += 1;

                // Fill fast table for codes <= 8 bits
                if length <= 8 {
                    let pad_bits = 8 - length;
                    let base = (code as usize) << pad_bits;
                    let count_fast = 1usize << pad_bits;
                    let entry = ((sym as u16) << 8) | length as u16;
                    let mut i = 0;
                    while i < count_fast {
                        self.fast[base + i] = entry;
                        i += 1;
                    }
                }

                sym_idx += 1;
                code += 1;
            }
            code <<= 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Bit reader (MSB-first, handles JPEG byte-stuffing 0xFF00)
// ---------------------------------------------------------------------------

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bits: u32,   // bit accumulator
    count: i32,  // number of valid bits in accumulator
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], start: usize) -> Self {
        BitReader { data, pos: start, bits: 0, count: 0 }
    }

    /// Ensure at least `need` bits in the accumulator.
    #[inline]
    fn fill(&mut self, need: i32) {
        while self.count < need {
            if self.pos >= self.data.len() {
                // Pad with zeros at end of data (common for truncated streams)
                self.bits <<= 8;
                self.count += 8;
                continue;
            }
            let byte = self.data[self.pos] as u32;
            self.pos += 1;

            if byte == 0xFF {
                // Byte-stuffing: skip the 0x00 that follows 0xFF in entropy data
                if self.pos < self.data.len() && self.data[self.pos] == 0x00 {
                    self.pos += 1;
                }
                // If it's a real marker (non-zero), we've hit the end of the scan.
                // We still push 0xFF into the accumulator; the decoder will notice
                // EOB or run out of MCUs and stop naturally.
            }

            self.bits = (self.bits << 8) | byte;
            self.count += 8;
        }
    }

    /// Read `n` bits and return as unsigned value.
    #[inline]
    fn read_bits(&mut self, n: i32) -> i32 {
        if n == 0 {
            return 0;
        }
        self.fill(n);
        self.count -= n;
        ((self.bits >> self.count) & ((1 << n) - 1)) as i32
    }

    /// Decode a Huffman symbol using the given table.
    fn decode_huff(&mut self, ht: &HuffTable) -> i32 {
        self.fill(16); // ensure enough bits for any code

        // Fast path: 8-bit lookup
        let peek8 = ((self.bits >> (self.count - 8)) & 0xFF) as usize;
        let entry = ht.fast[peek8];
        if entry != 0 {
            let len = (entry & 0xFF) as i32;
            let sym = (entry >> 8) as i32;
            self.count -= len;
            return sym;
        }

        // Slow path: match code bit by bit
        let mut code: u32 = 0;
        for length in 1..=16i32 {
            code = (code << 1) | ((self.bits >> (self.count - length)) & 1);
            // Linear search (tables are small, max 256 entries)
            let mut i = 0;
            while i < ht.num_codes {
                let (c, l, s) = ht.codes[i];
                if l as i32 == length && c as u32 == code {
                    self.count -= length;
                    return s as i32;
                }
                i += 1;
            }
        }

        // Should not reach here with valid data
        -1
    }

    /// Receive and extend: read `n` extra bits and sign-extend.
    fn receive_extend(&mut self, n: i32) -> i32 {
        if n == 0 {
            return 0;
        }
        let val = self.read_bits(n);
        // If the high bit is not set, the value is negative
        if val < (1 << (n - 1)) {
            val + (-1 << n) + 1
        } else {
            val
        }
    }
}

// ---------------------------------------------------------------------------
// Frame / Component info
// ---------------------------------------------------------------------------

struct Component {
    id: u8,
    h_samples: u8, // horizontal sampling factor (1, 2)
    v_samples: u8, // vertical sampling factor (1, 2)
    qt_id: u8,     // quantization table index
    dc_table: u8,  // DC Huffman table index
    ac_table: u8,  // AC Huffman table index
    dc_pred: i32,  // previous DC coefficient for differential coding
}

struct FrameInfo {
    width: u16,
    height: u16,
    num_comp: u8,
    comp: [Component; MAX_COMP],
    max_h: u8, // maximum horizontal sampling factor
    max_v: u8, // maximum vertical sampling factor
}

impl FrameInfo {
    const fn new() -> Self {
        const COMP_INIT: Component = Component {
            id: 0, h_samples: 1, v_samples: 1, qt_id: 0,
            dc_table: 0, ac_table: 0, dc_pred: 0,
        };
        FrameInfo {
            width: 0, height: 0, num_comp: 0,
            comp: [COMP_INIT; MAX_COMP],
            max_h: 1, max_v: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API: probe
// ---------------------------------------------------------------------------

/// Detect a JPEG file and return metadata.
///
/// Looks for the SOI marker (0xFFD8), then scans for SOF0 to extract
/// width, height, and component info. Computes the scratch buffer size
/// needed for decoding.
pub fn probe(data: &[u8]) -> Option<ImageInfo> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != M_SOI {
        return None;
    }

    // Walk markers to find SOF0
    let mut pos: usize = 2;
    while pos + 4 <= data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }

        // Skip padding 0xFF bytes
        while pos + 1 < data.len() && data[pos + 1] == 0xFF {
            pos += 1;
        }
        if pos + 1 >= data.len() {
            break;
        }

        let marker = data[pos + 1];
        pos += 2;

        if marker == M_EOI || marker == 0x00 {
            break;
        }
        // Markers without payload
        if marker == M_SOI {
            continue;
        }

        if pos + 2 > data.len() {
            break;
        }
        let seg_len = read_u16_be(data, pos) as usize;
        if seg_len < 2 || pos + seg_len > data.len() {
            break;
        }

        if marker == M_SOF0 {
            // Baseline DCT
            if seg_len < 8 {
                return None;
            }
            let precision = data[pos + 2];
            if precision != 8 {
                return None;
            }
            let height = read_u16_be(data, pos + 3) as u32;
            let width = read_u16_be(data, pos + 5) as u32;
            let num_comp = data[pos + 7] as u32;

            if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
                return None;
            }
            if num_comp == 0 || num_comp > 3 {
                return None;
            }

            // Parse sampling factors to compute MCU dimensions
            let mut max_h: u32 = 1;
            let mut max_v: u32 = 1;
            if pos + 8 + (num_comp as usize) * 3 <= pos + seg_len {
                for i in 0..num_comp as usize {
                    let off = pos + 8 + i * 3;
                    let hv = data[off + 1];
                    let h = (hv >> 4) as u32;
                    let v = (hv & 0x0F) as u32;
                    if h > max_h { max_h = h; }
                    if v > max_v { max_v = v; }
                }
            }

            // Scratch memory estimate:
            //  - 4 quantization tables: 4 * 64 * 4 = 1024 bytes
            //  - 4 Huffman tables: stored on stack (not in scratch)
            //  - Decoded MCU coefficient blocks: max_h*max_v per component * 64 * 4 bytes
            //    For 4:2:0 with 3 components: (4+1+1) * 256 = 1536
            //  - Row buffer for upsampling: width * 3 * 2 rows = width * 6 * max_v
            //  - Component planes: for each component, one row of MCUs tall
            //    = (max_h*8) * (max_v*8) * num_comp * 1 byte each
            //  - We keep it simple: allocate full component planes
            //    Y plane: width*height bytes, Cb plane: (w/h_ratio)*(h/v_ratio) bytes, same for Cr
            //  Total: generous upper bound
            let mcu_w = max_h * 8;
            let mcu_h = max_v * 8;
            let mcus_x = (width + mcu_w - 1) / mcu_w;
            let mcus_y = (height + mcu_h - 1) / mcu_h;
            let total_mcu_pixels = mcus_x * mcu_w * mcus_y * mcu_h;

            // Component plane sizes (in bytes)
            let mut plane_total: u32 = 0;
            if pos + 8 + (num_comp as usize) * 3 <= pos + seg_len {
                for i in 0..num_comp as usize {
                    let off = pos + 8 + i * 3;
                    let hv = data[off + 1];
                    let h = (hv >> 4) as u32;
                    let v = (hv & 0x0F) as u32;
                    let pw = mcus_x * h * 8;
                    let ph = mcus_y * v * 8;
                    plane_total += pw * ph;
                }
            } else {
                plane_total = total_mcu_pixels * num_comp;
            }

            // Quantization tables + alignment slack
            let scratch = plane_total + 4096;

            return Some(ImageInfo {
                width,
                height,
                format: FMT_JPEG,
                scratch_needed: scratch,
            });
        }

        pos += seg_len;
    }

    None
}

// ---------------------------------------------------------------------------
// Public API: decode
// ---------------------------------------------------------------------------

/// Decode a baseline JPEG to ARGB8888 pixels.
///
/// `data`: raw JPEG file bytes
/// `out`:  output buffer, must hold at least width*height u32 elements
/// `scratch`: working memory (size from `probe().scratch_needed`)
pub fn decode(data: &[u8], out: &mut [u32], scratch: &mut [u8]) -> i32 {
    if data.len() < 4 || data[0] != 0xFF || data[1] != M_SOI {
        return ERR_INVALID_DATA;
    }

    let mut frame = FrameInfo::new();
    let mut quant = [[0i32; 64]; MAX_QTABLES];
    let mut huff_dc = [HuffTable::new(), HuffTable::new()];
    let mut huff_ac = [HuffTable::new(), HuffTable::new()];
    let mut restart_interval: u16 = 0;
    let mut sos_pos: usize = 0; // position of entropy-coded data

    // ------------------------------------------------------------------
    // Parse markers
    // ------------------------------------------------------------------
    let mut pos: usize = 2;
    while pos + 2 <= data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }
        while pos + 1 < data.len() && data[pos + 1] == 0xFF {
            pos += 1;
        }
        if pos + 1 >= data.len() {
            break;
        }

        let marker = data[pos + 1];
        pos += 2;

        if marker == M_EOI || marker == 0x00 {
            break;
        }
        if marker == M_SOI {
            continue;
        }

        // SOS is special: after the header comes the entropy-coded segment
        if marker == M_SOS {
            if pos + 2 > data.len() {
                return ERR_INVALID_DATA;
            }
            let seg_len = read_u16_be(data, pos) as usize;
            if seg_len < 6 || pos + seg_len > data.len() {
                return ERR_INVALID_DATA;
            }

            let ns = data[pos + 2] as usize;
            if ns == 0 || ns > MAX_COMP || ns != frame.num_comp as usize {
                return ERR_INVALID_DATA;
            }

            for i in 0..ns {
                let off = pos + 3 + i * 2;
                let comp_id = data[off];
                let td_ta = data[off + 1];
                let td = td_ta >> 4;
                let ta = td_ta & 0x0F;
                // Match component by ID
                for c in 0..frame.num_comp as usize {
                    if frame.comp[c].id == comp_id {
                        frame.comp[c].dc_table = td;
                        frame.comp[c].ac_table = ta;
                    }
                }
            }

            sos_pos = pos + seg_len;
            break;
        }

        if pos + 2 > data.len() {
            break;
        }
        let seg_len = read_u16_be(data, pos) as usize;
        if seg_len < 2 || pos + seg_len > data.len() {
            break;
        }

        match marker {
            M_SOF0 => {
                // Baseline DCT frame header
                if seg_len < 8 {
                    return ERR_INVALID_DATA;
                }
                let precision = data[pos + 2];
                if precision != 8 {
                    return ERR_UNSUPPORTED;
                }
                frame.height = read_u16_be(data, pos + 3);
                frame.width = read_u16_be(data, pos + 5);
                frame.num_comp = data[pos + 7];

                if frame.width == 0 || frame.height == 0
                    || frame.width as u32 > MAX_DIM || frame.height as u32 > MAX_DIM
                {
                    return ERR_INVALID_DATA;
                }
                if frame.num_comp == 0 || frame.num_comp > MAX_COMP as u8 {
                    return ERR_UNSUPPORTED;
                }

                let mut max_h: u8 = 1;
                let mut max_v: u8 = 1;
                for i in 0..frame.num_comp as usize {
                    let off = pos + 8 + i * 3;
                    if off + 2 >= pos + seg_len {
                        return ERR_INVALID_DATA;
                    }
                    frame.comp[i].id = data[off];
                    let hv = data[off + 1];
                    frame.comp[i].h_samples = hv >> 4;
                    frame.comp[i].v_samples = hv & 0x0F;
                    frame.comp[i].qt_id = data[off + 2];
                    if frame.comp[i].h_samples > 2 || frame.comp[i].v_samples > 2 {
                        return ERR_UNSUPPORTED;
                    }
                    if frame.comp[i].h_samples > max_h {
                        max_h = frame.comp[i].h_samples;
                    }
                    if frame.comp[i].v_samples > max_v {
                        max_v = frame.comp[i].v_samples;
                    }
                }
                frame.max_h = max_h;
                frame.max_v = max_v;
            }

            M_DQT => {
                // Quantization table(s)
                let mut qpos = pos + 2;
                while qpos < pos + seg_len {
                    let pq_tq = data[qpos];
                    let precision_q = pq_tq >> 4;
                    let tq = (pq_tq & 0x0F) as usize;
                    qpos += 1;

                    if tq >= MAX_QTABLES {
                        return ERR_INVALID_DATA;
                    }

                    for i in 0..64usize {
                        let zi = ZIGZAG[i] as usize;
                        if precision_q == 0 {
                            // 8-bit values
                            if qpos >= data.len() {
                                return ERR_INVALID_DATA;
                            }
                            quant[tq][zi] = data[qpos] as i32;
                            qpos += 1;
                        } else {
                            // 16-bit values
                            if qpos + 1 >= data.len() {
                                return ERR_INVALID_DATA;
                            }
                            quant[tq][zi] = read_u16_be(data, qpos) as i32;
                            qpos += 2;
                        }
                    }
                }
            }

            M_DHT => {
                // Huffman table(s)
                let mut hpos = pos + 2;
                while hpos < pos + seg_len {
                    if hpos >= data.len() {
                        return ERR_INVALID_DATA;
                    }
                    let tc_th = data[hpos];
                    let tc = tc_th >> 4; // 0 = DC, 1 = AC
                    let th = (tc_th & 0x0F) as usize;
                    hpos += 1;

                    if th > 1 {
                        return ERR_UNSUPPORTED; // Only tables 0 and 1
                    }

                    if hpos + 16 > data.len() {
                        return ERR_INVALID_DATA;
                    }
                    let mut counts = [0u8; 16];
                    let mut total_sym = 0usize;
                    for i in 0..16 {
                        counts[i] = data[hpos + i];
                        total_sym += counts[i] as usize;
                    }
                    hpos += 16;

                    if total_sym > 256 || hpos + total_sym > data.len() {
                        return ERR_INVALID_DATA;
                    }

                    let symbols = &data[hpos..hpos + total_sym];
                    if tc == 0 {
                        huff_dc[th].build(&counts, symbols);
                    } else {
                        huff_ac[th].build(&counts, symbols);
                    }
                    hpos += total_sym;
                }
            }

            M_DRI => {
                // Restart interval
                if seg_len >= 4 {
                    restart_interval = read_u16_be(data, pos + 2);
                }
            }

            _ => {
                // Skip APP0, APPn, COM, etc.
            }
        }

        pos += seg_len;
    }

    if sos_pos == 0 || frame.width == 0 {
        return ERR_INVALID_DATA;
    }

    let width = frame.width as usize;
    let height = frame.height as usize;

    if out.len() < width * height {
        return ERR_BUFFER_TOO_SMALL;
    }

    // ------------------------------------------------------------------
    // Allocate component planes in scratch buffer
    // ------------------------------------------------------------------
    let mcu_w = frame.max_h as usize * 8;
    let mcu_h = frame.max_v as usize * 8;
    let mcus_x = (width + mcu_w - 1) / mcu_w;
    let mcus_y = (height + mcu_h - 1) / mcu_h;

    // Compute plane sizes and offsets within scratch
    let mut plane_offset = [0usize; MAX_COMP];
    let mut plane_w = [0usize; MAX_COMP];
    let mut plane_h = [0usize; MAX_COMP];
    let mut scratch_used = 0usize;

    for c in 0..frame.num_comp as usize {
        let pw = mcus_x * frame.comp[c].h_samples as usize * 8;
        let ph = mcus_y * frame.comp[c].v_samples as usize * 8;
        plane_w[c] = pw;
        plane_h[c] = ph;
        plane_offset[c] = scratch_used;
        scratch_used += pw * ph;
    }

    if scratch_used > scratch.len() {
        return ERR_SCRATCH_TOO_SMALL;
    }

    // Zero out scratch planes
    let mut i = 0;
    while i < scratch_used {
        scratch[i] = 0;
        i += 1;
    }

    // ------------------------------------------------------------------
    // Decode entropy-coded data (MCU by MCU)
    // ------------------------------------------------------------------
    let mut bits = BitReader::new(data, sos_pos);
    let mut restart_count: u16 = 0;
    let mut mcu_count: u32 = 0;

    // Temporary block for one 8x8 DCT block
    let mut block = [0i32; 64];

    for mcu_y in 0..mcus_y {
        for mcu_x in 0..mcus_x {
            // Handle restart markers
            if restart_interval > 0 && mcu_count > 0
                && (mcu_count % restart_interval as u32) == 0
            {
                // Align to next byte boundary
                bits.count = 0;
                bits.bits = 0;
                // Skip to next RST marker (0xFFDn)
                while bits.pos + 1 < data.len() {
                    if data[bits.pos] == 0xFF && data[bits.pos + 1] >= 0xD0
                        && data[bits.pos + 1] <= 0xD7
                    {
                        bits.pos += 2;
                        break;
                    }
                    bits.pos += 1;
                }
                // Reset DC predictors
                for c in 0..frame.num_comp as usize {
                    frame.comp[c].dc_pred = 0;
                }
                restart_count = restart_count.wrapping_add(1);
            }

            // Decode each component's blocks in this MCU
            for c in 0..frame.num_comp as usize {
                let h_blocks = frame.comp[c].h_samples as usize;
                let v_blocks = frame.comp[c].v_samples as usize;
                let qt_id = frame.comp[c].qt_id as usize;
                let dc_id = frame.comp[c].dc_table as usize;
                let ac_id = frame.comp[c].ac_table as usize;

                if qt_id >= MAX_QTABLES || dc_id > 1 || ac_id > 1 {
                    return ERR_INVALID_DATA;
                }

                for bv in 0..v_blocks {
                    for bh in 0..h_blocks {
                        // Decode one 8x8 block
                        // Clear block
                        let mut j = 0;
                        while j < 64 {
                            block[j] = 0;
                            j += 1;
                        }

                        // DC coefficient
                        let dc_sym = bits.decode_huff(&huff_dc[dc_id]);
                        if dc_sym < 0 {
                            return ERR_INVALID_DATA;
                        }
                        let dc_diff = bits.receive_extend(dc_sym);
                        frame.comp[c].dc_pred += dc_diff;
                        block[0] = frame.comp[c].dc_pred;

                        // AC coefficients
                        let mut k = 1;
                        while k < 64 {
                            let ac_sym = bits.decode_huff(&huff_ac[ac_id]);
                            if ac_sym < 0 {
                                return ERR_INVALID_DATA;
                            }
                            if ac_sym == 0 {
                                break; // EOB
                            }

                            let run = (ac_sym >> 4) & 0x0F;
                            let size = ac_sym & 0x0F;

                            k += run;
                            if k >= 64 {
                                break;
                            }

                            if size > 0 {
                                block[ZIGZAG[k as usize] as usize] =
                                    bits.receive_extend(size);
                            }
                            k += 1;
                        }

                        // Dequantize
                        let qt = &quant[qt_id];
                        let mut j = 0;
                        while j < 64 {
                            block[j] *= qt[j];
                            j += 1;
                        }

                        // Inverse DCT
                        idct(&mut block);

                        // Store decoded 8x8 block into component plane
                        let bx = (mcu_x * h_blocks + bh) * 8;
                        let by = (mcu_y * v_blocks + bv) * 8;
                        let pw = plane_w[c];
                        let base = plane_offset[c];

                        for row in 0..8 {
                            for col in 0..8 {
                                let px = bx + col;
                                let py = by + row;
                                if px < pw && py < plane_h[c] {
                                    // Clamp to 0..255
                                    let val = block[row * 8 + col] + 128;
                                    let clamped = if val < 0 {
                                        0u8
                                    } else if val > 255 {
                                        255u8
                                    } else {
                                        val as u8
                                    };
                                    scratch[base + py * pw + px] = clamped;
                                }
                            }
                        }
                    }
                }
            }

            mcu_count += 1;
        }
    }

    // ------------------------------------------------------------------
    // Color conversion + chroma upsampling
    // ------------------------------------------------------------------
    if frame.num_comp == 1 {
        // Grayscale
        for y in 0..height {
            for x in 0..width {
                let g = scratch[plane_offset[0] + y * plane_w[0] + x] as u32;
                out[y * width + x] = 0xFF000000 | (g << 16) | (g << 8) | g;
            }
        }
    } else {
        // YCbCr -> RGB
        for y in 0..height {
            for x in 0..width {
                let yy = scratch[plane_offset[0] + y * plane_w[0] + x] as i32;

                // Map pixel coordinates to chroma plane coordinates
                // using nearest-neighbor upsampling
                let cx = x * frame.comp[1].h_samples as usize / frame.max_h as usize;
                let cy = y * frame.comp[1].v_samples as usize / frame.max_v as usize;

                let cb = scratch[plane_offset[1] + cy * plane_w[1] + cx] as i32 - 128;
                let cr = scratch[plane_offset[2] + cy * plane_w[2] + cx] as i32 - 128;

                // YCbCr -> RGB using fixed-point (Q10)
                // R = Y + 1.402 * Cr
                // G = Y - 0.34414 * Cb - 0.71414 * Cr
                // B = Y + 1.772 * Cb
                let r = yy + ((cr * 1436) >> 10);               // 1.402 * 1024 ≈ 1436
                let g = yy - ((cb * 352) >> 10) - ((cr * 731) >> 10); // 0.344*1024≈352, 0.714*1024≈731
                let b = yy + ((cb * 1815) >> 10);               // 1.772 * 1024 ≈ 1815

                let r = clamp_u8(r) as u32;
                let g = clamp_u8(g) as u32;
                let b = clamp_u8(b) as u32;

                out[y * width + x] = 0xFF000000 | (r << 16) | (g << 8) | b;
            }
        }
    }

    ERR_OK
}

// ---------------------------------------------------------------------------
// Clamp to 0..255
// ---------------------------------------------------------------------------

#[inline]
fn clamp_u8(v: i32) -> u8 {
    if v < 0 { 0 }
    else if v > 255 { 255 }
    else { v as u8 }
}

// ---------------------------------------------------------------------------
// Integer IDCT (Loeffler-Ligtenberg-Moschytz, Q13 fixed-point)
// ---------------------------------------------------------------------------
//
// This is the standard LLM algorithm used by libjpeg and many other
// implementations, adapted for fixed-point integer arithmetic.
//
// Two 1-D transforms: first on rows, then on columns.
// The input block is in normal (not zig-zag) order, already dequantized.

fn idct(block: &mut [i32; 64]) {
    // Row pass
    for row in 0..8 {
        let base = row * 8;
        idct_1d_row(block, base);
    }

    // Column pass
    for col in 0..8 {
        idct_1d_col(block, col);
    }
}

/// 1D IDCT on a row of 8 elements (in-place).
fn idct_1d_row(block: &mut [i32; 64], base: usize) {
    let s0 = block[base + 0];
    let s1 = block[base + 1];
    let s2 = block[base + 2];
    let s3 = block[base + 3];
    let s4 = block[base + 4];
    let s5 = block[base + 5];
    let s6 = block[base + 6];
    let s7 = block[base + 7];

    // Prescale: shift left to give more precision for row pass
    // (column pass will shift right for final result)
    let s0 = s0 << IDCT_BITS;
    let s4 = s4 << IDCT_BITS;

    // Check for all-zero AC (common shortcut)
    if s1 == 0 && s2 == 0 && s3 == 0 && s4 == 0
        && s5 == 0 && s6 == 0 && s7 == 0
    {
        let dc = s0 + (1 << (IDCT_BITS - 4)); // rounding for later shift
        block[base + 0] = dc;
        block[base + 1] = dc;
        block[base + 2] = dc;
        block[base + 3] = dc;
        block[base + 4] = dc;
        block[base + 5] = dc;
        block[base + 6] = dc;
        block[base + 7] = dc;
        return;
    }

    // Even part
    let p2 = s2;
    let p6 = s6;
    let t2 = (p2 * FIX_0_541 + p6 * (FIX_0_541 - FIX_1_847)) + IDCT_HALF;
    let t3 = (p2 * (FIX_0_541 + FIX_0_765) + p6 * FIX_0_541) + IDCT_HALF;

    let t0 = s0 + s4;
    let t1 = s0 - s4;

    let e0 = t0 + t3;
    let e3 = t0 - t3;
    let e1 = t1 + t2;
    let e2 = t1 - t2;

    // Odd part
    let mut t0 = s7;
    let mut t1 = s5;
    let mut t2 = s3;
    let mut t3 = s1;

    let z1 = t0 + t3;
    let z2 = t1 + t2;
    let z3 = t0 + t2;
    let z4 = t1 + t3;
    let z5 = (z3 + z4) * FIX_1_175;

    t0 = t0 * FIX_0_298;
    t1 = t1 * FIX_2_053;
    t2 = t2 * FIX_3_072;
    t3 = t3 * FIX_1_501;

    let z1 = z1 * -FIX_0_899;
    let z2 = z2 * -FIX_2_562;
    let z3 = z3 * -FIX_1_961 + z5;
    let z4 = z4 * -FIX_0_390 + z5;

    t0 = t0 + z1 + z3;
    t1 = t1 + z2 + z4;
    t2 = t2 + z2 + z3;
    t3 = t3 + z1 + z4;

    // Final butterfly and descale (row pass keeps IDCT_BITS of precision)
    block[base + 0] = (e0 + t3) >> (IDCT_BITS - 2);
    block[base + 7] = (e0 - t3) >> (IDCT_BITS - 2);
    block[base + 1] = (e1 + t2) >> (IDCT_BITS - 2);
    block[base + 6] = (e1 - t2) >> (IDCT_BITS - 2);
    block[base + 2] = (e2 + t1) >> (IDCT_BITS - 2);
    block[base + 5] = (e2 - t1) >> (IDCT_BITS - 2);
    block[base + 3] = (e3 + t0) >> (IDCT_BITS - 2);
    block[base + 4] = (e3 - t0) >> (IDCT_BITS - 2);
}

/// 1D IDCT on a column of 8 elements (in-place).
fn idct_1d_col(block: &mut [i32; 64], col: usize) {
    let s0 = block[0 * 8 + col];
    let s1 = block[1 * 8 + col];
    let s2 = block[2 * 8 + col];
    let s3 = block[3 * 8 + col];
    let s4 = block[4 * 8 + col];
    let s5 = block[5 * 8 + col];
    let s6 = block[6 * 8 + col];
    let s7 = block[7 * 8 + col];

    // Check for all-zero AC (common shortcut)
    if s1 == 0 && s2 == 0 && s3 == 0 && s4 == 0
        && s5 == 0 && s6 == 0 && s7 == 0
    {
        // Descale: row pass left IDCT_BITS-2 of extra precision,
        // column pass needs to remove those plus the row prescale.
        // Total shift: IDCT_BITS + IDCT_BITS - 2 + 3 = 2*IDCT_BITS + 1
        let dc = (s0 + (1 << (IDCT_BITS + 1))) >> (IDCT_BITS + 2);
        block[0 * 8 + col] = dc;
        block[1 * 8 + col] = dc;
        block[2 * 8 + col] = dc;
        block[3 * 8 + col] = dc;
        block[4 * 8 + col] = dc;
        block[5 * 8 + col] = dc;
        block[6 * 8 + col] = dc;
        block[7 * 8 + col] = dc;
        return;
    }

    // Even part
    let p2 = s2;
    let p6 = s6;
    let t2 = p2 * FIX_0_541 + p6 * (FIX_0_541 - FIX_1_847);
    let t3 = p2 * (FIX_0_541 + FIX_0_765) + p6 * FIX_0_541;

    let t0 = (s0 + s4) << IDCT_BITS;
    let t1 = (s0 - s4) << IDCT_BITS;

    let e0 = t0 + t3 + IDCT_HALF;
    let e3 = t0 - t3 + IDCT_HALF;
    let e1 = t1 + t2 + IDCT_HALF;
    let e2 = t1 - t2 + IDCT_HALF;

    // Odd part
    let mut t0 = s7;
    let mut t1 = s5;
    let mut t2 = s3;
    let mut t3 = s1;

    let z1 = t0 + t3;
    let z2 = t1 + t2;
    let z3 = t0 + t2;
    let z4 = t1 + t3;
    let z5 = (z3 + z4) * FIX_1_175;

    t0 = t0 * FIX_0_298;
    t1 = t1 * FIX_2_053;
    t2 = t2 * FIX_3_072;
    t3 = t3 * FIX_1_501;

    let z1 = z1 * -FIX_0_899;
    let z2 = z2 * -FIX_2_562;
    let z3 = z3 * -FIX_1_961 + z5;
    let z4 = z4 * -FIX_0_390 + z5;

    t0 = t0 + z1 + z3;
    t1 = t1 + z2 + z4;
    t2 = t2 + z2 + z3;
    t3 = t3 + z1 + z4;

    // Final butterfly and descale
    // The combined descale from row + column pass
    let shift = IDCT_BITS + 2;
    block[0 * 8 + col] = (e0 + t3) >> shift;
    block[7 * 8 + col] = (e0 - t3) >> shift;
    block[1 * 8 + col] = (e1 + t2) >> shift;
    block[6 * 8 + col] = (e1 - t2) >> shift;
    block[2 * 8 + col] = (e2 + t1) >> shift;
    block[5 * 8 + col] = (e2 - t1) >> shift;
    block[3 * 8 + col] = (e3 + t0) >> shift;
    block[4 * 8 + col] = (e3 - t0) >> shift;
}
