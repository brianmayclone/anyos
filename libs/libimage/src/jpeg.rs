// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! JPEG (JFIF) decoder — Baseline (SOF0) and Progressive (SOF2).
//!
//! Supports 8-bit precision, 1-3 components, chroma subsampling 4:4:4,
//! 4:2:2 and 4:2:0, restart intervals, and the full progressive scan model
//! (DC initial + refinement, AC initial + refinement with EOB runs).
//!
//! All working memory is caller-provided via a scratch buffer.
//! Fixed-point integer math only (no FPU).

use crate::types::*;
use crate::jpeg_tables::*;

#[cfg(feature = "host")]
macro_rules! trace { ($($arg:tt)*) => { eprintln!($($arg)*); }; }
#[cfg(not(feature = "host"))]
macro_rules! trace { ($($arg:tt)*) => {}; }

#[cfg(feature = "host")]
macro_rules! fail { ($code:expr, $($arg:tt)*) => {{ eprintln!("jpeg-fail {}:{}: {}", file!(), line!(), format!($($arg)*)); return $code; }}; }
#[cfg(not(feature = "host"))]
macro_rules! fail { ($code:expr, $($arg:tt)*) => { return $code; }; }

// ---------------------------------------------------------------------------
// JPEG markers
// ---------------------------------------------------------------------------

const M_SOI: u8 = 0xD8;
const M_EOI: u8 = 0xD9;
const M_SOF0: u8 = 0xC0; // Baseline DCT
const M_SOF2: u8 = 0xC2; // Progressive DCT
const M_DHT: u8 = 0xC4;
const M_DQT: u8 = 0xDB;
const M_DRI: u8 = 0xDD;
const M_SOS: u8 = 0xDA;

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

struct HuffTable {
    fast: [u16; 256],
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

    fn build(&mut self, counts: &[u8; 16], symbols: &[u8]) {
        self.fast = [0u16; 256];
        self.num_codes = 0;

        let mut code: u16 = 0;
        let mut sym_idx: usize = 0;

        for bits in 0..16u8 {
            let length = bits + 1;
            let count = counts[bits as usize] as usize;
            for _ in 0..count {
                if sym_idx >= symbols.len() || self.num_codes >= 256 {
                    return;
                }
                let sym = symbols[sym_idx];
                self.codes[self.num_codes] = (code, length, sym);
                self.num_codes += 1;

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
    bits: u32,
    count: i32,
    /// Set when the reader hits a real (non-stuffed) marker. Decoders use
    /// this to detect end-of-scan instead of overreading into headers.
    marker_seen: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], start: usize) -> Self {
        BitReader { data, pos: start, bits: 0, count: 0, marker_seen: 0 }
    }

    #[inline]
    fn fill(&mut self, need: i32) {
        while self.count < need {
            if self.pos >= self.data.len() || self.marker_seen != 0 {
                self.bits <<= 8;
                self.count += 8;
                continue;
            }
            let byte = self.data[self.pos] as u32;
            self.pos += 1;

            if byte == 0xFF {
                if self.pos < self.data.len() {
                    let next = self.data[self.pos];
                    if next == 0x00 {
                        self.pos += 1;
                    } else if next != 0xFF {
                        // Real marker — back up so the outer loop can read it.
                        self.pos -= 1;
                        self.marker_seen = next;
                        self.bits <<= 8;
                        self.count += 8;
                        continue;
                    }
                    // 0xFF 0xFF: marker padding; stay on the second 0xFF and
                    // the next loop iteration will rescan.
                }
            }

            self.bits = (self.bits << 8) | byte;
            self.count += 8;
        }
    }

    #[inline]
    fn read_bits(&mut self, n: i32) -> i32 {
        if n == 0 {
            return 0;
        }
        self.fill(n);
        self.count -= n;
        ((self.bits >> self.count) & ((1u32 << n) - 1)) as i32
    }

    fn decode_huff(&mut self, ht: &HuffTable) -> i32 {
        self.fill(16);

        let peek8 = ((self.bits >> (self.count - 8)) & 0xFF) as usize;
        let entry = ht.fast[peek8];
        if entry != 0 {
            let len = (entry & 0xFF) as i32;
            let sym = (entry >> 8) as i32;
            self.count -= len;
            return sym;
        }

        let mut code: u32 = 0;
        for length in 1..=16i32 {
            code = (code << 1) | ((self.bits >> (self.count - length)) & 1);
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
        -1
    }

    fn receive_extend(&mut self, n: i32) -> i32 {
        if n == 0 {
            return 0;
        }
        let val = self.read_bits(n);
        if val < (1 << (n - 1)) {
            val + (-1 << n) + 1
        } else {
            val
        }
    }

    /// Discard bits up to the next byte boundary. Called after a restart
    /// marker so the next scan segment starts at a byte boundary.
    fn align_to_byte(&mut self) {
        let drop = self.count & 7;
        self.count -= drop;
        self.bits &= (1u32 << self.count).wrapping_sub(1);
    }
}

// ---------------------------------------------------------------------------
// Frame / Component info
// ---------------------------------------------------------------------------

#[derive(Copy, Clone)]
struct Component {
    id: u8,
    h_samples: u8,
    v_samples: u8,
    qt_id: u8,
    dc_table: u8,
    ac_table: u8,
    dc_pred: i32,
    /// For progressive: number of horizontal blocks across the whole image.
    blocks_x: u32,
    /// Number of vertical blocks across the whole image.
    blocks_y: u32,
    /// Offset (in i16 elements) into the coefficient buffer where this
    /// component's blocks start. Only used by the progressive path.
    coef_offset: u32,
}

struct FrameInfo {
    width: u16,
    height: u16,
    num_comp: u8,
    progressive: bool,
    comp: [Component; MAX_COMP],
    max_h: u8,
    max_v: u8,
}

impl FrameInfo {
    const fn new() -> Self {
        const COMP_INIT: Component = Component {
            id: 0, h_samples: 1, v_samples: 1, qt_id: 0,
            dc_table: 0, ac_table: 0, dc_pred: 0,
            blocks_x: 0, blocks_y: 0, coef_offset: 0,
        };
        FrameInfo {
            width: 0, height: 0, num_comp: 0, progressive: false,
            comp: [COMP_INIT; MAX_COMP],
            max_h: 1, max_v: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API: probe
// ---------------------------------------------------------------------------

/// Detect a JPEG file and return metadata. Recognises SOF0 (baseline) and
/// SOF2 (progressive) frames.
pub fn probe(data: &[u8]) -> Option<ImageInfo> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != M_SOI {
        return None;
    }

    let mut pos: usize = 2;
    while pos + 4 <= data.len() {
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

        if pos + 2 > data.len() {
            break;
        }
        let seg_len = read_u16_be(data, pos) as usize;
        if seg_len < 2 || pos + seg_len > data.len() {
            break;
        }

        let is_sof = marker == M_SOF0 || marker == M_SOF2;
        if is_sof {
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

            let mut max_h: u32 = 1;
            let mut max_v: u32 = 1;
            let mut hv_per_comp = [(1u32, 1u32); MAX_COMP];
            if pos + 8 + (num_comp as usize) * 3 <= pos + seg_len {
                for i in 0..num_comp as usize {
                    let off = pos + 8 + i * 3;
                    let hv = data[off + 1];
                    let h = (hv >> 4) as u32;
                    let v = (hv & 0x0F) as u32;
                    if h > max_h { max_h = h; }
                    if v > max_v { max_v = v; }
                    hv_per_comp[i] = (h, v);
                }
            }

            let mcu_w = max_h * 8;
            let mcu_h = max_v * 8;
            let mcus_x = (width + mcu_w - 1) / mcu_w;
            let mcus_y = (height + mcu_h - 1) / mcu_h;

            let mut plane_total: u32 = 0;
            let mut coef_total: u32 = 0;
            for i in 0..num_comp as usize {
                let (h, v) = hv_per_comp[i];
                let pw = mcus_x * h * 8;
                let ph = mcus_y * v * 8;
                plane_total += pw * ph;
                coef_total += mcus_x * h * mcus_y * v * 64; // i16 elements
            }

            // Progressive needs the coefficient buffer too.
            let coef_bytes = if marker == M_SOF2 { coef_total * 2 } else { 0 };

            // Generous slack for alignment + Huffman scratch.
            let scratch = plane_total + coef_bytes + 4096;

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

/// Decode a JPEG (baseline or progressive) into ARGB8888 pixels.
pub fn decode(data: &[u8], out: &mut [u32], scratch: &mut [u8]) -> i32 {
    if data.len() < 4 || data[0] != 0xFF || data[1] != M_SOI {
        fail!(ERR_INVALID_DATA, "");
    }

    let mut frame = FrameInfo::new();
    let mut quant = [[0i32; 64]; MAX_QTABLES];
    let mut huff_dc = [HuffTable::new(), HuffTable::new(), HuffTable::new(), HuffTable::new()];
    let mut huff_ac = [HuffTable::new(), HuffTable::new(), HuffTable::new(), HuffTable::new()];
    let mut restart_interval: u16 = 0;

    // ── Parse markers up to first SOS, collecting frame info + tables. ──
    let mut pos: usize = 2;
    let mut sof_seen = false;
    let mut sos_pos: usize = 0;
    // Per-scan state, populated when we hit SOS.
    let mut scan_ns: usize = 0;
    let mut scan_comp_indices: [usize; MAX_COMP] = [0; MAX_COMP];
    let mut scan_ss: u8 = 0;
    let mut scan_se: u8 = 63;
    let mut scan_ah: u8 = 0;
    let mut scan_al: u8 = 0;

    loop {
        if pos + 2 > data.len() {
            fail!(ERR_INVALID_DATA, "");
        }
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }
        while pos + 1 < data.len() && data[pos + 1] == 0xFF {
            pos += 1;
        }
        if pos + 1 >= data.len() {
            fail!(ERR_INVALID_DATA, "");
        }
        let marker = data[pos + 1];
        pos += 2;

        if marker == M_EOI || marker == 0x00 {
            fail!(ERR_INVALID_DATA, "");
        }
        if marker == M_SOI {
            continue;
        }

        if marker == M_SOS {
            if !sof_seen {
                fail!(ERR_INVALID_DATA, "");
            }
            if pos + 2 > data.len() {
                fail!(ERR_INVALID_DATA, "");
            }
            let seg_len = read_u16_be(data, pos) as usize;
            if seg_len < 6 || pos + seg_len > data.len() {
                fail!(ERR_INVALID_DATA, "");
            }
            let ns = data[pos + 2] as usize;
            if ns == 0 || ns > MAX_COMP {
                fail!(ERR_INVALID_DATA, "");
            }
            for i in 0..ns {
                let off = pos + 3 + i * 2;
                let comp_id = data[off];
                let td_ta = data[off + 1];
                let td = td_ta >> 4;
                let ta = td_ta & 0x0F;
                let mut found = false;
                for c in 0..frame.num_comp as usize {
                    if frame.comp[c].id == comp_id {
                        frame.comp[c].dc_table = td;
                        frame.comp[c].ac_table = ta;
                        scan_comp_indices[i] = c;
                        found = true;
                        break;
                    }
                }
                if !found {
                    fail!(ERR_INVALID_DATA, "");
                }
            }
            // Ss, Se, Ah, Al
            let off = pos + 3 + ns * 2;
            scan_ss = data[off];
            scan_se = data[off + 1];
            let ah_al = data[off + 2];
            scan_ah = ah_al >> 4;
            scan_al = ah_al & 0x0F;
            scan_ns = ns;
            sos_pos = pos + seg_len;
            break;
        }

        if pos + 2 > data.len() {
            fail!(ERR_INVALID_DATA, "");
        }
        let seg_len = read_u16_be(data, pos) as usize;
        if seg_len < 2 || pos + seg_len > data.len() {
            fail!(ERR_INVALID_DATA, "");
        }

        match marker {
            M_SOF0 | M_SOF2 => {
                if seg_len < 8 {
                    fail!(ERR_INVALID_DATA, "");
                }
                let precision = data[pos + 2];
                if precision != 8 {
                    fail!(ERR_UNSUPPORTED, "");
                }
                frame.height = read_u16_be(data, pos + 3);
                frame.width = read_u16_be(data, pos + 5);
                frame.num_comp = data[pos + 7];
                frame.progressive = marker == M_SOF2;

                if frame.width == 0 || frame.height == 0
                    || frame.width as u32 > MAX_DIM || frame.height as u32 > MAX_DIM
                {
                    fail!(ERR_INVALID_DATA, "");
                }
                if frame.num_comp == 0 || frame.num_comp > MAX_COMP as u8 {
                    fail!(ERR_UNSUPPORTED, "");
                }

                let mut max_h: u8 = 1;
                let mut max_v: u8 = 1;
                for i in 0..frame.num_comp as usize {
                    let off = pos + 8 + i * 3;
                    if off + 2 >= pos + seg_len {
                        fail!(ERR_INVALID_DATA, "");
                    }
                    frame.comp[i].id = data[off];
                    let hv = data[off + 1];
                    frame.comp[i].h_samples = hv >> 4;
                    frame.comp[i].v_samples = hv & 0x0F;
                    frame.comp[i].qt_id = data[off + 2];
                    if frame.comp[i].h_samples > 2 || frame.comp[i].v_samples > 2 {
                        fail!(ERR_UNSUPPORTED, "");
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
                sof_seen = true;
            }

            M_DQT => {
                let mut qpos = pos + 2;
                while qpos < pos + seg_len {
                    let pq_tq = data[qpos];
                    let precision_q = pq_tq >> 4;
                    let tq = (pq_tq & 0x0F) as usize;
                    qpos += 1;
                    if tq >= MAX_QTABLES {
                        fail!(ERR_INVALID_DATA, "");
                    }
                    for i in 0..64usize {
                        let zi = ZIGZAG[i] as usize;
                        if precision_q == 0 {
                            if qpos >= data.len() {
                                fail!(ERR_INVALID_DATA, "");
                            }
                            quant[tq][zi] = data[qpos] as i32;
                            qpos += 1;
                        } else {
                            if qpos + 1 >= data.len() {
                                fail!(ERR_INVALID_DATA, "");
                            }
                            quant[tq][zi] = read_u16_be(data, qpos) as i32;
                            qpos += 2;
                        }
                    }
                }
            }

            M_DHT => {
                let mut hpos = pos + 2;
                while hpos < pos + seg_len {
                    if hpos >= data.len() {
                        fail!(ERR_INVALID_DATA, "");
                    }
                    let tc_th = data[hpos];
                    let tc = tc_th >> 4;
                    let th = (tc_th & 0x0F) as usize;
                    hpos += 1;
                    if th >= 4 {
                        fail!(ERR_UNSUPPORTED, "");
                    }
                    if hpos + 16 > data.len() {
                        fail!(ERR_INVALID_DATA, "");
                    }
                    let mut counts = [0u8; 16];
                    let mut total_sym = 0usize;
                    for i in 0..16 {
                        counts[i] = data[hpos + i];
                        total_sym += counts[i] as usize;
                    }
                    hpos += 16;
                    if total_sym > 256 || hpos + total_sym > data.len() {
                        fail!(ERR_INVALID_DATA, "");
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
                if seg_len >= 4 {
                    restart_interval = read_u16_be(data, pos + 2);
                }
            }

            _ => {}
        }

        pos += seg_len;
    }

    if !sof_seen || sos_pos == 0 || frame.width == 0 {
        fail!(ERR_INVALID_DATA, "");
    }

    let width = frame.width as usize;
    let height = frame.height as usize;

    if out.len() < width * height {
        return ERR_BUFFER_TOO_SMALL;
    }

    // ── Scratch layout: planes (u8), then optionally coefficient buffer (i16). ──
    let mcu_w = frame.max_h as usize * 8;
    let mcu_h = frame.max_v as usize * 8;
    let mcus_x = (width + mcu_w - 1) / mcu_w;
    let mcus_y = (height + mcu_h - 1) / mcu_h;

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

        // Block grid for progressive (only used in SOF2 path).
        frame.comp[c].blocks_x = (mcus_x as u32) * frame.comp[c].h_samples as u32;
        frame.comp[c].blocks_y = (mcus_y as u32) * frame.comp[c].v_samples as u32;
    }

    if scratch_used > scratch.len() {
        return ERR_SCRATCH_TOO_SMALL;
    }

    // Zero plane area.
    for i in 0..scratch_used {
        scratch[i] = 0;
    }

    if frame.progressive {
        // Coefficient buffer comes after the planes, aligned to 2 bytes.
        let coef_bytes_off = (scratch_used + 1) & !1;
        let mut coef_off_i16: u32 = 0;
        let mut coef_total: u32 = 0;
        for c in 0..frame.num_comp as usize {
            frame.comp[c].coef_offset = coef_off_i16;
            let n = frame.comp[c].blocks_x * frame.comp[c].blocks_y * 64;
            coef_off_i16 += n;
            coef_total += n;
        }
        let coef_bytes = (coef_total as usize) * 2;
        if coef_bytes_off + coef_bytes > scratch.len() {
            return ERR_SCRATCH_TOO_SMALL;
        }
        // Zero coefficients.
        for i in 0..coef_bytes {
            scratch[coef_bytes_off + i] = 0;
        }

        // Decode all scans into the coefficient buffer.
        let rc = decode_progressive(
            data, sos_pos,
            &mut frame, &huff_dc, &huff_ac,
            restart_interval,
            scratch, coef_bytes_off,
            scan_ns, &scan_comp_indices,
            scan_ss, scan_se, scan_ah, scan_al,
        );
        if rc != ERR_OK {
            return rc;
        }

        // Dequantize + iDCT every block into its component plane.
        finalize_progressive(
            &frame, &quant,
            scratch, &plane_offset, &plane_w, &plane_h,
            coef_bytes_off,
        );
    } else {
        // Baseline: single scan, decode-and-iDCT each block immediately.
        let rc = decode_baseline_scan(
            data, sos_pos,
            &mut frame, &huff_dc, &huff_ac, &quant,
            restart_interval,
            scratch, &plane_offset, &plane_w, &plane_h,
            mcus_x, mcus_y,
        );
        if rc != ERR_OK {
            return rc;
        }
    }

    // ── Color conversion + chroma upsampling ──
    if frame.num_comp == 1 {
        for y in 0..height {
            for x in 0..width {
                let g = scratch[plane_offset[0] + y * plane_w[0] + x] as u32;
                out[y * width + x] = 0xFF000000 | (g << 16) | (g << 8) | g;
            }
        }
    } else {
        for y in 0..height {
            for x in 0..width {
                let yy = scratch[plane_offset[0] + y * plane_w[0] + x] as i32;
                let cx = x * frame.comp[1].h_samples as usize / frame.max_h as usize;
                let cy = y * frame.comp[1].v_samples as usize / frame.max_v as usize;
                let cb = scratch[plane_offset[1] + cy * plane_w[1] + cx] as i32 - 128;
                let cr = scratch[plane_offset[2] + cy * plane_w[2] + cx] as i32 - 128;

                let r = yy + ((cr * 1436) >> 10);
                let g = yy - ((cb * 352) >> 10) - ((cr * 731) >> 10);
                let b = yy + ((cb * 1815) >> 10);

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
// Baseline scan decode (decode-and-render each block in one pass).
// ---------------------------------------------------------------------------

fn decode_baseline_scan(
    data: &[u8], sos_pos: usize,
    frame: &mut FrameInfo,
    huff_dc: &[HuffTable; 4], huff_ac: &[HuffTable; 4],
    quant: &[[i32; 64]; MAX_QTABLES],
    restart_interval: u16,
    scratch: &mut [u8],
    plane_offset: &[usize; MAX_COMP],
    plane_w: &[usize; MAX_COMP],
    plane_h: &[usize; MAX_COMP],
    mcus_x: usize, mcus_y: usize,
) -> i32 {
    let mut bits = BitReader::new(data, sos_pos);
    let mut mcu_count: u32 = 0;
    let mut block = [0i32; 64];

    for mcu_y in 0..mcus_y {
        for mcu_x in 0..mcus_x {
            if restart_interval > 0 && mcu_count > 0
                && (mcu_count % restart_interval as u32) == 0
            {
                consume_restart(&mut bits);
                for c in 0..frame.num_comp as usize {
                    frame.comp[c].dc_pred = 0;
                }
            }

            for c in 0..frame.num_comp as usize {
                let h_blocks = frame.comp[c].h_samples as usize;
                let v_blocks = frame.comp[c].v_samples as usize;
                let qt_id = frame.comp[c].qt_id as usize;
                let dc_id = frame.comp[c].dc_table as usize;
                let ac_id = frame.comp[c].ac_table as usize;

                if qt_id >= MAX_QTABLES || dc_id >= 4 || ac_id >= 4 {
                    fail!(ERR_INVALID_DATA, "");
                }

                for bv in 0..v_blocks {
                    for bh in 0..h_blocks {
                        for v in block.iter_mut() { *v = 0; }

                        let dc_sym = bits.decode_huff(&huff_dc[dc_id]);
                        if dc_sym < 0 {
                            fail!(ERR_INVALID_DATA, "");
                        }
                        let dc_diff = bits.receive_extend(dc_sym);
                        frame.comp[c].dc_pred += dc_diff;
                        block[0] = frame.comp[c].dc_pred;

                        let mut k = 1;
                        while k < 64 {
                            let ac_sym = bits.decode_huff(&huff_ac[ac_id]);
                            if ac_sym < 0 {
                                fail!(ERR_INVALID_DATA, "");
                            }
                            if ac_sym == 0 {
                                break;
                            }
                            let run = (ac_sym >> 4) & 0x0F;
                            let size = ac_sym & 0x0F;
                            k += run;
                            if k >= 64 { break; }
                            if size > 0 {
                                block[ZIGZAG[k as usize] as usize] =
                                    bits.receive_extend(size);
                            }
                            k += 1;
                        }

                        let qt = &quant[qt_id];
                        for j in 0..64 {
                            block[j] *= qt[j];
                        }
                        idct(&mut block);

                        let bx = (mcu_x * h_blocks + bh) * 8;
                        let by = (mcu_y * v_blocks + bv) * 8;
                        let pw = plane_w[c];
                        let base = plane_offset[c];
                        for row in 0..8 {
                            for col in 0..8 {
                                let px = bx + col;
                                let py = by + row;
                                if px < pw && py < plane_h[c] {
                                    let val = block[row * 8 + col];
                                    scratch[base + py * pw + px] = clamp_u8(val);
                                }
                            }
                        }
                    }
                }
            }
            mcu_count += 1;
        }
    }
    ERR_OK
}

// ---------------------------------------------------------------------------
// Progressive scan loop
// ---------------------------------------------------------------------------

fn decode_progressive(
    data: &[u8], first_sos_pos: usize,
    frame: &mut FrameInfo,
    huff_dc_tables: &[HuffTable; 4],
    huff_ac_tables: &[HuffTable; 4],
    initial_restart_interval: u16,
    scratch: &mut [u8], coef_bytes_off: usize,
    initial_ns: usize, initial_comp_indices: &[usize; MAX_COMP],
    initial_ss: u8, initial_se: u8, initial_ah: u8, initial_al: u8,
) -> i32 {
    let mut huff_dc: [HuffTable; 4];
    let mut huff_ac: [HuffTable; 4];
    // Take ownership-ish via reborrow: we'll mutate via fresh tables when DHT
    // updates appear between scans. Start from the parsed-pre-SOS tables.
    huff_dc = [HuffTable::new(), HuffTable::new(), HuffTable::new(), HuffTable::new()];
    huff_ac = [HuffTable::new(), HuffTable::new(), HuffTable::new(), HuffTable::new()];
    for i in 0..4 {
        huff_dc[i].copy_from(&huff_dc_tables[i]);
        huff_ac[i].copy_from(&huff_ac_tables[i]);
    }
    let mut restart_interval = initial_restart_interval;

    // Each scan needs: ns, comp_indices, ss, se, ah, al, and a sos_pos for
    // entropy data start. After consuming the scan we resume marker parsing
    // from `bits.pos` (BitReader leaves us pointing at the next 0xFF marker
    // byte once the entropy stream ends).
    let mut ns = initial_ns;
    let mut comp_indices = *initial_comp_indices;
    let mut ss = initial_ss;
    let mut se = initial_se;
    let mut ah = initial_ah;
    let mut al = initial_al;
    let mut entropy_pos = first_sos_pos;

    loop {
        // ── Decode this scan into the coefficient buffer ──
        let rc = decode_progressive_scan(
            data, entropy_pos,
            frame, &huff_dc, &huff_ac,
            restart_interval,
            scratch, coef_bytes_off,
            ns, &comp_indices,
            ss, se, ah, al,
        );
        if rc.0 != ERR_OK {
            return rc.0;
        }
        let mut pos = rc.1;

        // ── Walk markers until we hit another SOS or EOI ──
        loop {
            if pos + 2 > data.len() {
                fail!(ERR_INVALID_DATA, "");
            }
            if data[pos] != 0xFF {
                pos += 1;
                continue;
            }
            while pos + 1 < data.len() && data[pos + 1] == 0xFF {
                pos += 1;
            }
            if pos + 1 >= data.len() {
                fail!(ERR_INVALID_DATA, "");
            }
            let marker = data[pos + 1];
            pos += 2;

            if marker == M_EOI {
                return ERR_OK;
            }
            // Restart markers between scans: just skip.
            if (0xD0..=0xD7).contains(&marker) {
                continue;
            }
            if marker == 0x00 {
                continue;
            }

            if pos + 2 > data.len() {
                fail!(ERR_INVALID_DATA, "");
            }
            let seg_len = read_u16_be(data, pos) as usize;
            if seg_len < 2 || pos + seg_len > data.len() {
                fail!(ERR_INVALID_DATA, "");
            }

            match marker {
                M_DHT => {
                    let mut hpos = pos + 2;
                    while hpos < pos + seg_len {
                        if hpos >= data.len() {
                            fail!(ERR_INVALID_DATA, "");
                        }
                        let tc_th = data[hpos];
                        let tc = tc_th >> 4;
                        let th = (tc_th & 0x0F) as usize;
                        hpos += 1;
                        if th >= 4 {
                            fail!(ERR_UNSUPPORTED, "");
                        }
                        if hpos + 16 > data.len() {
                            fail!(ERR_INVALID_DATA, "");
                        }
                        let mut counts = [0u8; 16];
                        let mut total_sym = 0usize;
                        for i in 0..16 {
                            counts[i] = data[hpos + i];
                            total_sym += counts[i] as usize;
                        }
                        hpos += 16;
                        if total_sym > 256 || hpos + total_sym > data.len() {
                            fail!(ERR_INVALID_DATA, "");
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
                    if seg_len >= 4 {
                        restart_interval = read_u16_be(data, pos + 2);
                    }
                }
                M_SOS => {
                    if seg_len < 6 {
                        fail!(ERR_INVALID_DATA, "");
                    }
                    let n = data[pos + 2] as usize;
                    if n == 0 || n > MAX_COMP {
                        fail!(ERR_INVALID_DATA, "");
                    }
                    let mut new_comp = [0usize; MAX_COMP];
                    for i in 0..n {
                        let off = pos + 3 + i * 2;
                        let comp_id = data[off];
                        let td_ta = data[off + 1];
                        let td = td_ta >> 4;
                        let ta = td_ta & 0x0F;
                        let mut found = false;
                        for c in 0..frame.num_comp as usize {
                            if frame.comp[c].id == comp_id {
                                frame.comp[c].dc_table = td;
                                frame.comp[c].ac_table = ta;
                                new_comp[i] = c;
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            fail!(ERR_INVALID_DATA, "");
                        }
                    }
                    let off = pos + 3 + n * 2;
                    ss = data[off];
                    se = data[off + 1];
                    let ah_al = data[off + 2];
                    ah = ah_al >> 4;
                    al = ah_al & 0x0F;
                    ns = n;
                    comp_indices = new_comp;
                    entropy_pos = pos + seg_len;
                    break; // restart outer loop with the new scan
                }
                _ => {
                    // Skip APP/COM/etc.
                }
            }

            pos += seg_len;
        }
    }
}

/// Returns (status, position-after-scan).
fn decode_progressive_scan(
    data: &[u8], sos_pos: usize,
    frame: &mut FrameInfo,
    huff_dc: &[HuffTable; 4], huff_ac: &[HuffTable; 4],
    restart_interval: u16,
    scratch: &mut [u8], coef_bytes_off: usize,
    ns: usize, comp_indices: &[usize; MAX_COMP],
    ss: u8, se: u8, ah: u8, al: u8,
) -> (i32, usize) {
    let mut bits = BitReader::new(data, sos_pos);
    let mut mcu_count: u32 = 0;
    let mut eobrun: u32 = 0; // for AC scans

    let interleaved = ns > 1;
    let (mcus_x, mcus_y) = if interleaved {
        // Interleaved scans use the frame's MCU grid.
        let mcu_w = frame.max_h as usize * 8;
        let mcu_h = frame.max_v as usize * 8;
        (
            (frame.width as usize + mcu_w - 1) / mcu_w,
            (frame.height as usize + mcu_h - 1) / mcu_h,
        )
    } else {
        // Non-interleaved: per JPEG A.1.5, iterate over the component's
        // pixel-aligned block grid (ceil(width * Hi / (max_h * 8))). This
        // can be SMALLER than the MCU-aligned coefficient buffer grid; the
        // padding blocks at the bottom-right of the MCU grid keep whatever
        // coefficients earlier (interleaved) scans wrote into them.
        let c = comp_indices[0];
        let h = frame.comp[c].h_samples as usize;
        let v = frame.comp[c].v_samples as usize;
        let max_h = frame.max_h as usize;
        let max_v = frame.max_v as usize;
        let bx = (frame.width as usize * h + max_h * 8 - 1) / (max_h * 8);
        let by = (frame.height as usize * v + max_v * 8 - 1) / (max_v * 8);
        (bx, by)
    };

    // Reset DC predictors at scan start (per JPEG spec).
    for c in 0..frame.num_comp as usize {
        frame.comp[c].dc_pred = 0;
    }

    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            if restart_interval > 0 && mcu_count > 0
                && (mcu_count % restart_interval as u32) == 0
            {
                consume_restart(&mut bits);
                for c in 0..frame.num_comp as usize {
                    frame.comp[c].dc_pred = 0;
                }
                eobrun = 0;
            }

            if interleaved {
                for ci in 0..ns {
                    let c = comp_indices[ci];
                    let h_blocks = frame.comp[c].h_samples as usize;
                    let v_blocks = frame.comp[c].v_samples as usize;
                    for bv in 0..v_blocks {
                        for bh in 0..h_blocks {
                            let bx = mx * h_blocks + bh;
                            let by = my * v_blocks + bv;
                            let block_off = block_offset(frame, c, bx, by);
                            let rc = decode_progressive_block(
                                &mut bits, &mut eobrun,
                                frame, c,
                                huff_dc, huff_ac,
                                scratch, coef_bytes_off, block_off,
                                ss, se, ah, al,
                            );
                            if rc != ERR_OK {
                                return (rc, bits.pos);
                            }
                        }
                    }
                }
            } else {
                let c = comp_indices[0];
                let block_off = block_offset(frame, c, mx, my);
                let rc = decode_progressive_block(
                    &mut bits, &mut eobrun,
                    frame, c,
                    huff_dc, huff_ac,
                    scratch, coef_bytes_off, block_off,
                    ss, se, ah, al,
                );
                if rc != ERR_OK {
                    return (rc, bits.pos);
                }
            }
            mcu_count += 1;
        }
    }

    // After EOB run (for AC scans), bits reader may still be inside the
    // entropy stream. Position bits.pos is now at the next marker byte (or
    // near it, given the BitReader stops fetching once it sees a real
    // marker).
    (ERR_OK, bits.pos)
}

fn block_offset(frame: &FrameInfo, c: usize, bx: usize, by: usize) -> usize {
    let comp = &frame.comp[c];
    let off_i16 = comp.coef_offset as usize
        + (by * comp.blocks_x as usize + bx) * 64;
    off_i16 * 2
}

#[inline]
fn coef_get(scratch: &[u8], coef_bytes_off: usize, block_off: usize, k: usize) -> i32 {
    let p = coef_bytes_off + block_off + k * 2;
    let lo = scratch[p] as u16;
    let hi = scratch[p + 1] as u16;
    (((hi << 8) | lo) as i16) as i32
}

#[inline]
fn coef_set(scratch: &mut [u8], coef_bytes_off: usize, block_off: usize, k: usize, v: i32) {
    let p = coef_bytes_off + block_off + k * 2;
    let v16 = v as i16 as u16;
    scratch[p] = (v16 & 0xFF) as u8;
    scratch[p + 1] = (v16 >> 8) as u8;
}

fn decode_progressive_block(
    bits: &mut BitReader,
    eobrun: &mut u32,
    frame: &mut FrameInfo,
    c: usize,
    huff_dc: &[HuffTable; 4], huff_ac: &[HuffTable; 4],
    scratch: &mut [u8], coef_bytes_off: usize, block_off: usize,
    ss: u8, se: u8, ah: u8, al: u8,
) -> i32 {
    let dc_id = frame.comp[c].dc_table as usize;
    let ac_id = frame.comp[c].ac_table as usize;
    if dc_id >= 4 || ac_id >= 4 {
        fail!(ERR_INVALID_DATA, "");
    }

    if ss == 0 {
        // DC scan
        if ah == 0 {
            // Initial DC: decode DC diff, add to predictor, store << al.
            let sym = bits.decode_huff(&huff_dc[dc_id]);
            if sym < 0 || sym > 15 {
                fail!(ERR_INVALID_DATA, "");
            }
            let diff = bits.receive_extend(sym);
            frame.comp[c].dc_pred = frame.comp[c].dc_pred.wrapping_add(diff);
            let v = frame.comp[c].dc_pred << al;
            coef_set(scratch, coef_bytes_off, block_off, 0, v);
        } else {
            // DC refinement: read 1 bit, OR (bit << al) into existing DC.
            let bit = bits.read_bits(1);
            let cur = coef_get(scratch, coef_bytes_off, block_off, 0);
            coef_set(
                scratch, coef_bytes_off, block_off, 0,
                cur | (bit << al),
            );
        }
        return ERR_OK;
    }

    // AC scan
    if ah == 0 {
        // Initial AC scan
        if *eobrun > 0 {
            *eobrun -= 1;
            return ERR_OK;
        }
        let mut k = ss as i32;
        let se_i = se as i32;
        while k <= se_i {
            let sym = bits.decode_huff(&huff_ac[ac_id]);
            if sym < 0 {
                fail!(ERR_INVALID_DATA, "huff");
            }
            let run = (sym >> 4) & 0x0F;
            let size = sym & 0x0F;
            if size == 0 {
                if run < 15 {
                    // EOBn — start an EOB run of length 2^run + extra_bits.
                    let extra = bits.read_bits(run);
                    *eobrun = (1u32 << run) + extra as u32 - 1;
                    break;
                } else {
                    // ZRL — 16 zero coefficients.
                    k += 16;
                }
            } else {
                k += run;
                if k > se_i {
                    fail!(ERR_INVALID_DATA, "");
                }
                let v = bits.receive_extend(size) << al;
                let zi = ZIGZAG[k as usize] as usize;
                coef_set(scratch, coef_bytes_off, block_off, zi, v);
                k += 1;
            }
        }
        return ERR_OK;
    }

    // AC refinement scan (Ah > 0). Algorithm follows libjpeg-turbo's
    // jdphuff.c::decode_mcu_AC_refine: for each RS pair we (a) refine any
    // already-nonzero coefficients walked over, (b) skip `run` zero-valued
    // positions, and (c) optionally place a new ±(1<<Al) coefficient at the
    // next zero. EOBn switches to a tail-refinement mode that finishes the
    // current block and applies to the next eobrun-1 blocks too.
    let p1 = 1i32 << al;
    let m1 = -1i32 << al;
    let mut k = ss as i32;
    let se_i = se as i32;

    if *eobrun == 0 {
        while k <= se_i {
            let sym = bits.decode_huff(&huff_ac[ac_id]);
            if sym < 0 {
                fail!(ERR_INVALID_DATA, "huff");
            }
            let mut r = (sym >> 4) & 0x0F;
            let s_size = sym & 0x0F;
            let mut new_val: i32 = 0;
            #[cfg(feature = "host")]
            {
                let block_idx = block_off / 128;
                if block_idx == 361 {
                    eprintln!("  block#{} k={} sym=0x{:02X} (r={}, s={}) bits.pos={} acc=0x{:08X}/{}",
                        block_idx, k, sym, r, s_size, bits.pos, bits.bits, bits.count);
                }
            }

            if s_size == 0 {
                if r != 15 {
                    let extra = bits.read_bits(r);
                    *eobrun = (1u32 << r) + extra as u32;
                    break; // tail handled below
                }
                // ZRL: r stays 15, no new coefficient (new_val=0).
            } else if s_size == 1 {
                let bit = bits.read_bits(1);
                new_val = if bit != 0 { p1 } else { m1 };
            } else {
                fail!(ERR_INVALID_DATA, "size>1 in AC refine");
            }

            // Walk forward: refine nonzeros, count down `r` zeros until we
            // reach the target zero (or end of band).
            loop {
                if k > se_i {
                    break;
                }
                let zi = ZIGZAG[k as usize] as usize;
                let cur = coef_get(scratch, coef_bytes_off, block_off, zi);
                if cur != 0 {
                    let bit = bits.read_bits(1);
                    if bit != 0 && (cur & p1) == 0 {
                        let refined = if cur > 0 { cur + p1 } else { cur - p1 };
                        coef_set(scratch, coef_bytes_off, block_off, zi, refined);
                    }
                } else {
                    if r == 0 {
                        break;
                    }
                    r -= 1;
                }
                k += 1;
            }

            // Place new coefficient at the current target zero (size=1 only).
            if new_val != 0 {
                if k > se_i {
                    fail!(ERR_INVALID_DATA, "AC refine ran past Se");
                }
                let zi = ZIGZAG[k as usize] as usize;
                coef_set(scratch, coef_bytes_off, block_off, zi, new_val);
            }
            // Always advance past the position we just handled (whether we
            // placed or not), so the next RS lookup begins at the next coef.
            k += 1;
        }
    }

    // EOB-run tail: refine remaining nonzeros in this block, then decrement
    // the EOB counter so the next call to decode_progressive_block (or the
    // tail of *this* call when EOBn started mid-block) picks up correctly.
    if *eobrun > 0 {
        while k <= se_i {
            let zi = ZIGZAG[k as usize] as usize;
            let cur = coef_get(scratch, coef_bytes_off, block_off, zi);
            if cur != 0 {
                let bit = bits.read_bits(1);
                if bit != 0 && (cur & p1) == 0 {
                    let refined = if cur > 0 { cur + p1 } else { cur - p1 };
                    coef_set(scratch, coef_bytes_off, block_off, zi, refined);
                }
            }
            k += 1;
        }
        *eobrun -= 1;
    }

    ERR_OK
}

fn finalize_progressive(
    frame: &FrameInfo,
    quant: &[[i32; 64]; MAX_QTABLES],
    scratch: &mut [u8],
    plane_offset: &[usize; MAX_COMP],
    plane_w: &[usize; MAX_COMP],
    plane_h: &[usize; MAX_COMP],
    coef_bytes_off: usize,
) {
    let mut block = [0i32; 64];
    for c in 0..frame.num_comp as usize {
        let bx_count = frame.comp[c].blocks_x as usize;
        let by_count = frame.comp[c].blocks_y as usize;
        let qt_id = frame.comp[c].qt_id as usize;
        let qt = &quant[qt_id];
        for by in 0..by_count {
            for bx in 0..bx_count {
                let block_off = block_offset(frame, c, bx, by);
                for k in 0..64 {
                    block[k] = coef_get(scratch, coef_bytes_off, block_off, k) * qt[k];
                }
                idct(&mut block);

                let px0 = bx * 8;
                let py0 = by * 8;
                let pw = plane_w[c];
                let base = plane_offset[c];
                for row in 0..8 {
                    for col in 0..8 {
                        let px = px0 + col;
                        let py = py0 + row;
                        if px < pw && py < plane_h[c] {
                            scratch[base + py * pw + px] =
                                clamp_u8(block[row * 8 + col]);
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Restart helpers
// ---------------------------------------------------------------------------

fn consume_restart(bits: &mut BitReader) {
    bits.align_to_byte();
    bits.bits = 0;
    bits.count = 0;
    if bits.marker_seen >= 0xD0 && bits.marker_seen <= 0xD7 {
        // We were stopped at the RST marker; advance past it.
        bits.pos += 2;
        bits.marker_seen = 0;
        return;
    }
    while bits.pos + 1 < bits.data.len() {
        if bits.data[bits.pos] == 0xFF
            && bits.data[bits.pos + 1] >= 0xD0
            && bits.data[bits.pos + 1] <= 0xD7
        {
            bits.pos += 2;
            break;
        }
        bits.pos += 1;
    }
    bits.marker_seen = 0;
}

// Allow HuffTable to be cloned cheaply during scan transitions.
impl HuffTable {
    fn copy_from(&mut self, other: &HuffTable) {
        self.fast = other.fast;
        self.codes = other.codes;
        self.num_codes = other.num_codes;
    }
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
// Integer IDCT based on stb_image (Loeffler-Ligtenberg-Moschytz).
// ---------------------------------------------------------------------------

pub fn idct(block: &mut [i32; 64]) {
    let mut tmp = [0i32; 64];

    for col in 0..8 {
        let d = |r: usize| block[r * 8 + col];
        if d(1)==0 && d(2)==0 && d(3)==0 && d(4)==0 && d(5)==0 && d(6)==0 && d(7)==0 {
            let dc = d(0) * 4;
            for r in 0..8 { tmp[r * 8 + col] = dc; }
        } else {
            idct_1d(d(0), d(1), d(2), d(3), d(4), d(5), d(6), d(7), 512, 10,
                    |i, v| tmp[i * 8 + col] = v);
        }
    }

    let bias = 65536 + (128 << 17);
    for row in 0..8 {
        let r = |c: usize| tmp[row * 8 + c];
        idct_1d(r(0), r(1), r(2), r(3), r(4), r(5), r(6), r(7), bias, 17,
                |i, v| block[row * 8 + i] = v);
    }
}

// ---------------------------------------------------------------------------
// Host-only tests (cargo test --features host --lib)
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "host"))]
mod tests {
    use super::*;
    use alloc::vec;

    fn load_file(path: &str) -> Vec<u8> {
        std::fs::read(path).unwrap_or_else(|e| {
            panic!("failed to read {}: {}", path, e);
        })
    }

    fn save_ppm(path: &str, pixels: &[u32], w: u32, h: u32) {
        use std::io::Write;
        let mut out = std::fs::File::create(path).expect("create ppm");
        write!(out, "P6\n{} {}\n255\n", w, h).expect("ppm header");
        for &p in pixels {
            let r = ((p >> 16) & 0xFF) as u8;
            let g = ((p >> 8) & 0xFF) as u8;
            let b = (p & 0xFF) as u8;
            out.write_all(&[r, g, b]).expect("ppm pixel");
        }
    }

    #[test]
    fn decode_p150_progressive() {
        let data = load_file("/tmp/p150-prog.jpg");
        let info = probe(&data).expect("probe");
        let mut pixels = vec![0u32; (info.width as usize) * (info.height as usize)];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        let rc = decode(&data, &mut pixels, &mut scratch);
        eprintln!("p150-prog: rc={}", rc);
        assert_eq!(rc, ERR_OK);
        save_ppm("/tmp/p150-prog-decoded.ppm", &pixels, info.width, info.height);
    }

    #[test]
    fn decode_libjpeg_progressive() {
        let data = load_file("/tmp/plasma-default.jpg");
        let info = probe(&data).expect("probe");
        let mut pixels = vec![0u32; (info.width as usize) * (info.height as usize)];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        let rc = decode(&data, &mut pixels, &mut scratch);
        eprintln!("libjpeg-prog: rc={}", rc);
        assert_eq!(rc, ERR_OK);
        save_ppm("/tmp/plasma-default-decoded.ppm", &pixels, info.width, info.height);
    }

    #[test]
    fn decode_red_progressive() {
        let data = load_file("/tmp/red-prog.jpg");
        let info = probe(&data).expect("probe");
        let mut pixels = vec![0u32; (info.width as usize) * (info.height as usize)];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        let rc = decode(&data, &mut pixels, &mut scratch);
        eprintln!("red-prog: rc={} pixel[0]={:08X} pixel[mid]={:08X}",
            rc, pixels[0], pixels[pixels.len()/2]);
        assert_eq!(rc, ERR_OK);
        save_ppm("/tmp/red-prog-decoded.ppm", &pixels, info.width, info.height);
    }

    #[test]
    fn decode_plasma_progressive() {
        let data = load_file("/tmp/plasma-prog.jpg");
        let info = probe(&data).expect("probe");
        let mut pixels = vec![0u32; (info.width as usize) * (info.height as usize)];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        let rc = decode(&data, &mut pixels, &mut scratch);
        eprintln!("plasma-prog: rc={}", rc);
        assert_eq!(rc, ERR_OK);
        save_ppm("/tmp/plasma-prog-decoded.ppm", &pixels, info.width, info.height);
    }

    #[test]
    fn decode_mike_baseline() {
        let path = "/tmp/mike-base.jpg";
        let data = load_file(path);
        let info = probe(&data).expect("baseline probe");
        let mut pixels = vec![0u32; (info.width as usize) * (info.height as usize)];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        let rc = decode(&data, &mut pixels, &mut scratch);
        eprintln!("baseline test: decode rc={}", rc);
        assert_eq!(rc, ERR_OK, "baseline decode failed");
        save_ppm("/tmp/mike-base-decoded.ppm", &pixels, info.width, info.height);
    }

    #[test]
    fn decode_mike_jpg() {
        let path = std::env::var("MIKE_JPG").unwrap_or_else(|_| {
            "../../config/home/strati/Documents/mike.jpg".into()
        });
        let data = load_file(&path);
        eprintln!("test: read {} bytes from {}", data.len(), path);

        let info = probe(&data).expect("probe failed — not recognised as JPEG");
        eprintln!(
            "test: probe ok format={} {}x{} scratch={}",
            info.format, info.width, info.height, info.scratch_needed
        );
        assert_eq!(info.format, FMT_JPEG);
        assert!(info.width > 0 && info.height > 0);

        let mut pixels = vec![0u32; (info.width as usize) * (info.height as usize)];
        let mut scratch = vec![0u8; info.scratch_needed as usize];
        let rc = decode(&data, &mut pixels, &mut scratch);
        eprintln!("test: decode rc={}", rc);
        assert_eq!(rc, ERR_OK, "decode failed with code {}", rc);

        // Sanity: not all pixels black, not all white.
        let mut min = u32::MAX;
        let mut max = 0u32;
        let mut sum_r: u64 = 0;
        let mut sum_g: u64 = 0;
        let mut sum_b: u64 = 0;
        for &p in &pixels {
            min = min.min(p);
            max = max.max(p);
            sum_r += ((p >> 16) & 0xFF) as u64;
            sum_g += ((p >> 8) & 0xFF) as u64;
            sum_b += (p & 0xFF) as u64;
        }
        let n = pixels.len() as u64;
        eprintln!(
            "test: pixel stats min={:08X} max={:08X} avg=({}, {}, {})",
            min, max, sum_r / n, sum_g / n, sum_b / n,
        );
        assert!(min != max, "decoded image is uniform (likely blank)");

        // Dump as PPM next to the source so we can inspect visually.
        let out_path = "/tmp/mike-decoded.ppm";
        save_ppm(out_path, &pixels, info.width, info.height);
        eprintln!("test: wrote {}", out_path);
    }
}

fn idct_1d<F: FnMut(usize, i32)>(
    s0: i32, s1: i32, s2: i32, s3: i32,
    s4: i32, s5: i32, s6: i32, s7: i32,
    bias: i32, shift: u32,
    mut store: F,
) {
    let p2 = s2 as i64;
    let p6 = s6 as i64;
    let p1 = (p2 + p6) * 2217;
    let t2 = p1 + p6 * -7568;
    let t3 = p1 + p2 * 3135;

    let p0 = (s0 as i64 + s4 as i64) << 12;
    let p4 = (s0 as i64 - s4 as i64) << 12;

    let x0 = p0 + t3;
    let x3 = p0 - t3;
    let x1 = p4 + t2;
    let x2 = p4 - t2;

    let mut t0 = s7 as i64;
    let mut t1 = s5 as i64;
    let mut t2o = s3 as i64;
    let mut t3o = s1 as i64;

    let p3 = t0 + t2o;
    let p4o = t1 + t3o;
    let p1o = t0 + t3o;
    let p2o = t1 + t2o;
    let p5 = (p3 + p4o) * 4816;

    t0 = t0 * 1223;
    t1 = t1 * 8410;
    t2o = t2o * 12586;
    t3o = t3o * 6149;

    let p1o = p5 + p1o * -3686;
    let p2o = p5 + p2o * -10498;
    let p3 = p3 * -8035;
    let p4o = p4o * -1598;

    t0 = t0 + p1o + p3;
    t1 = t1 + p2o + p4o;
    t2o = t2o + p2o + p3;
    t3o = t3o + p1o + p4o;

    let b = bias as i64;
    store(0, ((x0 + t3o + b) >> shift) as i32);
    store(7, ((x0 - t3o + b) >> shift) as i32);
    store(1, ((x1 + t2o + b) >> shift) as i32);
    store(6, ((x1 - t2o + b) >> shift) as i32);
    store(2, ((x2 + t1 + b) >> shift) as i32);
    store(5, ((x2 - t1 + b) >> shift) as i32);
    store(3, ((x3 + t0 + b) >> shift) as i32);
    store(4, ((x3 - t0 + b) >> shift) as i32);
}
