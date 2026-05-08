//! XZ container and LZMA2 decompression.
//!
//! This intentionally implements the subset used by Debian `data.tar.xz`
//! archives: single-stream XZ with one LZMA2 filter per block. The decoder
//! supports normal LZMA2 compressed chunks and uncompressed LZMA2 chunks.

use alloc::vec;
use alloc::vec::Vec;

use crate::crc32;

const XZ_MAGIC: &[u8; 6] = b"\xFD7zXZ\x00";
const XZ_FOOTER_MAGIC: &[u8; 2] = b"YZ";

const BLOCK_FILTER_LZMA2: u64 = 0x21;

const CHECK_NONE: u8 = 0;
const CHECK_CRC32: u8 = 1;
const CHECK_CRC64: u8 = 4;
const CHECK_SHA256: u8 = 10;

const LZMA_PROB_INIT: u16 = 1024;
const LZMA_BIT_MODEL_TOTAL_BITS: u32 = 11;
const LZMA_BIT_MODEL_TOTAL: u32 = 1 << LZMA_BIT_MODEL_TOTAL_BITS;
const LZMA_MOVE_BITS: u32 = 5;
const LZMA_TOP_VALUE: u32 = 1 << 24;

const LZMA_NUM_STATES: usize = 12;
const LZMA_NUM_POS_BITS_MAX: usize = 4;
const LZMA_NUM_POS_STATES_MAX: usize = 1 << LZMA_NUM_POS_BITS_MAX;
const LZMA_NUM_LEN_TO_POS_STATES: usize = 4;
const LZMA_NUM_POS_SLOT_BITS: usize = 6;
const LZMA_NUM_ALIGN_BITS: usize = 4;
const LZMA_END_POS_MODEL_INDEX: usize = 14;
const LZMA_FULL_DISTANCES: usize = 1 << (LZMA_END_POS_MODEL_INDEX / 2);
const LZMA_MATCH_MIN_LEN: usize = 2;

pub fn is_xz(data: &[u8]) -> bool {
    data.len() >= XZ_MAGIC.len() && &data[..XZ_MAGIC.len()] == XZ_MAGIC
}

pub fn xz_decompress(data: &[u8]) -> Option<Vec<u8>> {
    let mut dec = XzDecoder::new(data)?;
    dec.decode()
}

struct XzDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    check_type: u8,
    out: Vec<u8>,
    records: Vec<IndexRecord>,
}

#[derive(Clone, Copy)]
struct IndexRecord {
    unpadded_size: u64,
    uncompressed_size: u64,
}

struct BlockInfo {
    header_size: usize,
    compressed_size: Option<u64>,
    uncompressed_size: Option<u64>,
}

impl<'a> XzDecoder<'a> {
    fn new(data: &'a [u8]) -> Option<Self> {
        if !is_xz(data) || data.len() < 12 + 12 {
            return None;
        }
        let flags = [data[6], data[7]];
        if flags[0] != 0 {
            return None;
        }
        let expected_crc = read_u32_le(data, 8)?;
        if crc32::crc32(&flags) != expected_crc {
            return None;
        }
        let check_type = flags[1] & 0x0F;
        if check_type != flags[1] {
            return None;
        }
        check_size(check_type)?;

        if &data[data.len() - 2..] != XZ_FOOTER_MAGIC {
            return None;
        }
        let footer_flags = [data[data.len() - 4], data[data.len() - 3]];
        if footer_flags != flags {
            return None;
        }
        let expected_footer_crc = read_u32_le(data, data.len() - 12)?;
        if crc32::crc32(&data[data.len() - 8..data.len() - 2]) != expected_footer_crc {
            return None;
        }

        Some(Self {
            data,
            pos: 12,
            check_type,
            out: Vec::new(),
            records: Vec::new(),
        })
    }

    fn decode(&mut self) -> Option<Vec<u8>> {
        let stream_payload_end = self.data.len().checked_sub(12)?;
        while self.pos < stream_payload_end {
            if self.data[self.pos] == 0 {
                self.decode_index(stream_payload_end)?;
                return Some(core::mem::take(&mut self.out));
            }
            self.decode_block(stream_payload_end)?;
        }
        None
    }

    fn decode_block(&mut self, stream_payload_end: usize) -> Option<()> {
        let header_start = self.pos;
        let header_byte = *self.data.get(self.pos)?;
        let header_size = (header_byte as usize + 1).checked_mul(4)?;
        if header_size < 8 || header_start.checked_add(header_size)? > stream_payload_end {
            return None;
        }
        let header = &self.data[header_start..header_start + header_size];
        let expected_crc = read_u32_le(header, header_size - 4)?;
        if crc32::crc32(&header[..header_size - 4]) != expected_crc {
            return None;
        }
        let block = parse_block_header(header)?;
        self.pos += block.header_size;

        let out_start = self.out.len();
        let compressed_consumed = lzma2_decode(
            &self.data[self.pos..stream_payload_end],
            block.compressed_size.map(|v| v as usize),
            block.uncompressed_size.map(|v| v as usize),
            &mut self.out,
        )
        .map(|consumed| {
            self.pos += consumed;
            consumed
        })?;

        while self.pos % 4 != 0 {
            if *self.data.get(self.pos)? != 0 {
                return None;
            }
            self.pos += 1;
        }

        let check_start = self.pos;
        let check_len = check_size(self.check_type)?;
        if check_start.checked_add(check_len)? > stream_payload_end {
            return None;
        }
        let block_out = &self.out[out_start..];
        match self.check_type {
            CHECK_NONE => {}
            CHECK_CRC32 => {
                let expected = read_u32_le(self.data, check_start)?;
                if crc32::crc32(block_out) != expected {
                    return None;
                }
            }
            CHECK_CRC64 | CHECK_SHA256 => {
                // Debian packages usually use CRC64. We skip verification for
                // now, but still consume the declared check bytes.
            }
            _ => return None,
        }
        self.pos += check_len;

        let unpadded_size = (block.header_size + compressed_consumed + check_len) as u64;
        let uncompressed_size = (self.out.len() - out_start) as u64;
        if let Some(expected) = block.uncompressed_size {
            if expected != uncompressed_size {
                return None;
            }
        }
        if let Some(expected) = block.compressed_size {
            if expected != compressed_consumed as u64 {
                return None;
            }
        }
        self.records.push(IndexRecord {
            unpadded_size,
            uncompressed_size,
        });
        Some(())
    }

    fn decode_index(&mut self, stream_payload_end: usize) -> Option<()> {
        let index_start = self.pos;
        self.pos += 1; // index indicator
        let record_count = read_vli(self.data, &mut self.pos)? as usize;
        if record_count != self.records.len() {
            return None;
        }
        for expected in &self.records {
            let unpadded_size = read_vli(self.data, &mut self.pos)?;
            let uncompressed_size = read_vli(self.data, &mut self.pos)?;
            if unpadded_size != expected.unpadded_size
                || uncompressed_size != expected.uncompressed_size
            {
                return None;
            }
        }
        while self.pos % 4 != 0 {
            if *self.data.get(self.pos)? != 0 {
                return None;
            }
            self.pos += 1;
        }
        if self.pos.checked_add(4)? > stream_payload_end {
            return None;
        }
        let expected_crc = read_u32_le(self.data, self.pos)?;
        if crc32::crc32(&self.data[index_start..self.pos]) != expected_crc {
            return None;
        }
        self.pos += 4;
        if self.pos != stream_payload_end {
            return None;
        }

        let footer_back_size = read_u32_le(self.data, stream_payload_end + 4)? as u64;
        let index_size = (self.pos - index_start) as u64;
        if footer_back_size.checked_add(1)?.checked_mul(4)? != index_size {
            return None;
        }
        Some(())
    }
}

fn parse_block_header(header: &[u8]) -> Option<BlockInfo> {
    let header_size = (header[0] as usize + 1) * 4;
    let flags = header[1];
    if flags & 0x3C != 0 {
        return None;
    }
    let filter_count = (flags & 0x03) as usize + 1;
    if filter_count != 1 {
        return None;
    }
    let has_compressed_size = flags & 0x40 != 0;
    let has_uncompressed_size = flags & 0x80 != 0;

    let mut pos = 2usize;
    let compressed_size = if has_compressed_size {
        Some(read_vli(header, &mut pos)?)
    } else {
        None
    };
    let uncompressed_size = if has_uncompressed_size {
        Some(read_vli(header, &mut pos)?)
    } else {
        None
    };

    let filter_id = read_vli(header, &mut pos)?;
    if filter_id != BLOCK_FILTER_LZMA2 {
        return None;
    }
    let props_len = read_vli(header, &mut pos)? as usize;
    if props_len != 1 || pos.checked_add(props_len)? > header_size - 4 {
        return None;
    }
    let dict_prop = header[pos];
    decode_lzma2_dict_size(dict_prop)?;
    pos += props_len;

    while pos < header_size - 4 {
        if header[pos] != 0 {
            return None;
        }
        pos += 1;
    }
    Some(BlockInfo {
        header_size,
        compressed_size,
        uncompressed_size,
    })
}

fn check_size(check_type: u8) -> Option<usize> {
    match check_type {
        CHECK_NONE => Some(0),
        CHECK_CRC32 => Some(4),
        CHECK_CRC64 => Some(8),
        CHECK_SHA256 => Some(32),
        _ => None,
    }
}

fn read_vli(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut shift = 0u32;
    let mut value = 0u64;
    for i in 0..9 {
        let b = *data.get(*pos)?;
        *pos += 1;
        value |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            if i > 0 && b == 0 {
                return None;
            }
            return Some(value);
        }
        shift += 7;
    }
    None
}

fn read_u32_le(data: &[u8], off: usize) -> Option<u32> {
    if off.checked_add(4)? > data.len() {
        return None;
    }
    Some(u32::from_le_bytes([
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
    ]))
}

fn decode_lzma2_dict_size(prop: u8) -> Option<u32> {
    if prop > 40 {
        return None;
    }
    if prop == 40 {
        return Some(0xFFFF_FFFF);
    }
    let mant = 2u32 + (prop as u32 & 1);
    let exp = prop as u32 / 2 + 11;
    Some(mant << exp)
}

fn lzma2_decode(
    data: &[u8],
    compressed_limit: Option<usize>,
    uncompressed_size: Option<usize>,
    out: &mut Vec<u8>,
) -> Option<usize> {
    let mut pos = 0usize;
    let end = compressed_limit.unwrap_or(data.len());
    if end > data.len() {
        return None;
    }
    let start_out = out.len();
    let mut lzma = LzmaDecoder::new();
    let mut props_set = false;

    loop {
        if pos >= end {
            return None;
        }
        let control = data[pos];
        pos += 1;
        if control == 0 {
            break;
        }
        if control < 0x80 {
            if control > 2 || pos + 2 > end {
                return None;
            }
            if control == 1 {
                props_set = false;
                lzma.reset_dictionary(out.len());
            }
            let size = (((data[pos] as usize) << 8) | data[pos + 1] as usize) + 1;
            pos += 2;
            if pos.checked_add(size)? > end {
                return None;
            }
            out.extend_from_slice(&data[pos..pos + size]);
            pos += size;
            continue;
        }

        if pos + 4 > end {
            return None;
        }
        let unpack_size = (((control as usize & 0x1F) << 16)
            | ((data[pos] as usize) << 8)
            | data[pos + 1] as usize)
            + 1;
        pos += 2;
        let pack_size = (((data[pos] as usize) << 8) | data[pos + 1] as usize) + 1;
        pos += 2;

        if control >= 0xE0 {
            props_set = false;
            lzma.reset_dictionary(out.len());
        }
        if control >= 0xC0 {
            if pos >= end {
                return None;
            }
            lzma.set_props(data[pos])?;
            props_set = true;
            pos += 1;
        } else if !props_set {
            return None;
        }
        if control >= 0xA0 {
            lzma.reset_state();
        }

        if pos.checked_add(pack_size)? > end {
            return None;
        }
        let chunk = &data[pos..pos + pack_size];
        pos += pack_size;
        let before = out.len();
        lzma.decode_chunk(chunk, unpack_size, out)?;
        if out.len() - before != unpack_size {
            return None;
        }
    }

    if let Some(limit) = compressed_limit {
        if pos != limit {
            return None;
        }
    }
    if let Some(size) = uncompressed_size {
        if out.len() - start_out != size {
            return None;
        }
    }
    Some(pos)
}

struct LzmaDecoder {
    lc: u32,
    lp: u32,
    pb: u32,
    state: u32,
    reps: [usize; 4],
    literal: Vec<u16>,
    is_match: Vec<u16>,
    is_rep: Vec<u16>,
    is_rep_g0: Vec<u16>,
    is_rep_g1: Vec<u16>,
    is_rep_g2: Vec<u16>,
    is_rep0_long: Vec<u16>,
    pos_slot: Vec<u16>,
    pos_decoders: Vec<u16>,
    pos_align: Vec<u16>,
    len_decoder: LenDecoder,
    rep_len_decoder: LenDecoder,
    dict_start: usize,
}

impl LzmaDecoder {
    fn new() -> Self {
        let mut dec = Self {
            lc: 3,
            lp: 0,
            pb: 2,
            state: 0,
            reps: [0; 4],
            literal: Vec::new(),
            is_match: Vec::new(),
            is_rep: Vec::new(),
            is_rep_g0: Vec::new(),
            is_rep_g1: Vec::new(),
            is_rep_g2: Vec::new(),
            is_rep0_long: Vec::new(),
            pos_slot: Vec::new(),
            pos_decoders: Vec::new(),
            pos_align: Vec::new(),
            len_decoder: LenDecoder::new(),
            rep_len_decoder: LenDecoder::new(),
            dict_start: 0,
        };
        dec.reset_props();
        dec.reset_state();
        dec
    }

    fn set_props(&mut self, prop: u8) -> Option<()> {
        if prop >= 9 * 5 * 5 {
            return None;
        }
        let mut v = prop as u32;
        self.lc = v % 9;
        v /= 9;
        self.lp = v % 5;
        self.pb = v / 5;
        if self.pb > LZMA_NUM_POS_BITS_MAX as u32 {
            return None;
        }
        self.reset_props();
        Some(())
    }

    fn reset_dictionary(&mut self, out_len: usize) {
        self.dict_start = out_len;
    }

    fn reset_state(&mut self) {
        self.state = 0;
        self.reps = [0; 4];
        fill_probs(&mut self.is_match);
        fill_probs(&mut self.is_rep);
        fill_probs(&mut self.is_rep_g0);
        fill_probs(&mut self.is_rep_g1);
        fill_probs(&mut self.is_rep_g2);
        fill_probs(&mut self.is_rep0_long);
        fill_probs(&mut self.pos_slot);
        fill_probs(&mut self.pos_decoders);
        fill_probs(&mut self.pos_align);
        fill_probs(&mut self.literal);
        self.len_decoder.reset();
        self.rep_len_decoder.reset();
    }

    fn reset_props(&mut self) {
        let literal_size = 0x300usize << (self.lc + self.lp);
        resize_probs(&mut self.literal, literal_size);
        resize_probs(
            &mut self.is_match,
            LZMA_NUM_STATES * LZMA_NUM_POS_STATES_MAX,
        );
        resize_probs(&mut self.is_rep, LZMA_NUM_STATES);
        resize_probs(&mut self.is_rep_g0, LZMA_NUM_STATES);
        resize_probs(&mut self.is_rep_g1, LZMA_NUM_STATES);
        resize_probs(&mut self.is_rep_g2, LZMA_NUM_STATES);
        resize_probs(
            &mut self.is_rep0_long,
            LZMA_NUM_STATES * LZMA_NUM_POS_STATES_MAX,
        );
        resize_probs(
            &mut self.pos_slot,
            LZMA_NUM_LEN_TO_POS_STATES * (1 << LZMA_NUM_POS_SLOT_BITS),
        );
        resize_probs(
            &mut self.pos_decoders,
            LZMA_FULL_DISTANCES - LZMA_END_POS_MODEL_INDEX,
        );
        resize_probs(&mut self.pos_align, 1 << LZMA_NUM_ALIGN_BITS);
        self.len_decoder.reset();
        self.rep_len_decoder.reset();
    }

    fn decode_chunk(&mut self, data: &[u8], unpack_size: usize, out: &mut Vec<u8>) -> Option<()> {
        let mut rd = RangeDecoder::new(data)?;
        let target = out.len().checked_add(unpack_size)?;
        while out.len() < target {
            let pos_state = (out.len() as u32 & ((1 << self.pb) - 1)) as usize;
            let state_idx = self.state as usize;
            if rd.decode_bit(&mut self.is_match[state_idx * LZMA_NUM_POS_STATES_MAX + pos_state])?
                == 0
            {
                self.decode_literal(&mut rd, out)?;
                self.state = update_literal_state(self.state);
                continue;
            }

            let len;
            if rd.decode_bit(&mut self.is_rep[state_idx])? != 0 {
                len = self.decode_rep_match(&mut rd, pos_state)?;
            } else {
                self.reps[3] = self.reps[2];
                self.reps[2] = self.reps[1];
                self.reps[1] = self.reps[0];
                len = self.len_decoder.decode(&mut rd, pos_state)? + LZMA_MATCH_MIN_LEN;
                self.state = if self.state < 7 { 7 } else { 10 };
                self.reps[0] = self.decode_distance(&mut rd, len)?;
            }
            self.copy_match(out, len)?;
        }
        Some(())
    }

    fn decode_literal(&mut self, rd: &mut RangeDecoder, out: &mut Vec<u8>) -> Option<()> {
        let prev_byte = out.last().copied().unwrap_or(0);
        let pos = out.len().saturating_sub(self.dict_start) as u32;
        let lp_mask = (1u32 << self.lp) - 1;
        let literal_state = ((pos & lp_mask) << self.lc) + (prev_byte as u32 >> (8 - self.lc));
        let offset = (literal_state as usize) * 0x300;
        let matched_byte = if self.state >= 7 {
            Some(self.peek_distance(out, self.reps[0])?)
        } else {
            None
        };
        let probs = &mut self.literal[offset..offset + 0x300];

        let mut symbol = 1usize;
        if let Some(mut match_byte) = matched_byte {
            while symbol < 0x100 {
                let match_bit = (match_byte >> 7) as usize;
                match_byte <<= 1;
                let bit = rd.decode_bit(&mut probs[0x100 + (match_bit << 8) + symbol])? as usize;
                symbol = (symbol << 1) | bit;
                if match_bit != bit {
                    break;
                }
            }
        }
        while symbol < 0x100 {
            let bit = rd.decode_bit(&mut probs[symbol])? as usize;
            symbol = (symbol << 1) | bit;
        }
        out.push(symbol as u8);
        Some(())
    }

    fn decode_rep_match(&mut self, rd: &mut RangeDecoder, pos_state: usize) -> Option<usize> {
        let state_idx = self.state as usize;
        if rd.decode_bit(&mut self.is_rep_g0[state_idx])? == 0 {
            if rd.decode_bit(
                &mut self.is_rep0_long[state_idx * LZMA_NUM_POS_STATES_MAX + pos_state],
            )? == 0
            {
                self.state = if self.state < 7 { 9 } else { 11 };
                return Some(1);
            }
        } else {
            let dist;
            if rd.decode_bit(&mut self.is_rep_g1[state_idx])? == 0 {
                dist = self.reps[1];
            } else {
                if rd.decode_bit(&mut self.is_rep_g2[state_idx])? == 0 {
                    dist = self.reps[2];
                } else {
                    dist = self.reps[3];
                    self.reps[3] = self.reps[2];
                }
                self.reps[2] = self.reps[1];
            }
            self.reps[1] = self.reps[0];
            self.reps[0] = dist;
        }
        self.state = if self.state < 7 { 8 } else { 11 };
        Some(self.rep_len_decoder.decode(rd, pos_state)? + LZMA_MATCH_MIN_LEN)
    }

    fn decode_distance(&mut self, rd: &mut RangeDecoder, len: usize) -> Option<usize> {
        let len_state = if len - LZMA_MATCH_MIN_LEN < LZMA_NUM_LEN_TO_POS_STATES {
            len - LZMA_MATCH_MIN_LEN
        } else {
            LZMA_NUM_LEN_TO_POS_STATES - 1
        };
        let pos_slot = decode_bit_tree(
            rd,
            &mut self.pos_slot[len_state * (1 << LZMA_NUM_POS_SLOT_BITS)..],
            LZMA_NUM_POS_SLOT_BITS,
        )? as usize;
        if pos_slot < 4 {
            return Some(pos_slot);
        }
        let num_direct_bits = (pos_slot >> 1) - 1;
        let mut dist = (2 | (pos_slot & 1)) << num_direct_bits;
        if pos_slot < LZMA_END_POS_MODEL_INDEX {
            let base = dist - pos_slot - 1;
            dist +=
                reverse_decode_bits(rd, &mut self.pos_decoders[base..], num_direct_bits)? as usize;
        } else {
            dist += (rd.decode_direct_bits(num_direct_bits - LZMA_NUM_ALIGN_BITS)? as usize)
                << LZMA_NUM_ALIGN_BITS;
            dist += reverse_decode_bits(rd, &mut self.pos_align, LZMA_NUM_ALIGN_BITS)? as usize;
        }
        Some(dist)
    }

    fn copy_match(&self, out: &mut Vec<u8>, len: usize) -> Option<()> {
        for _ in 0..len {
            let b = self.peek_distance(out, self.reps[0])?;
            out.push(b);
        }
        Some(())
    }

    fn peek_distance(&self, out: &[u8], distance: usize) -> Option<u8> {
        let back = distance.checked_add(1)?;
        if back > out.len().saturating_sub(self.dict_start) {
            return None;
        }
        Some(out[out.len() - back])
    }
}

struct LenDecoder {
    choice: [u16; 2],
    low: Vec<u16>,
    mid: Vec<u16>,
    high: Vec<u16>,
}

impl LenDecoder {
    fn new() -> Self {
        Self {
            choice: [LZMA_PROB_INIT; 2],
            low: vec![LZMA_PROB_INIT; LZMA_NUM_POS_STATES_MAX * 8],
            mid: vec![LZMA_PROB_INIT; LZMA_NUM_POS_STATES_MAX * 8],
            high: vec![LZMA_PROB_INIT; 256],
        }
    }

    fn reset(&mut self) {
        self.choice = [LZMA_PROB_INIT; 2];
        fill_probs(&mut self.low);
        fill_probs(&mut self.mid);
        fill_probs(&mut self.high);
    }

    fn decode(&mut self, rd: &mut RangeDecoder, pos_state: usize) -> Option<usize> {
        if rd.decode_bit(&mut self.choice[0])? == 0 {
            return Some(decode_bit_tree(rd, &mut self.low[pos_state * 8..], 3)? as usize);
        }
        if rd.decode_bit(&mut self.choice[1])? == 0 {
            return Some(8 + decode_bit_tree(rd, &mut self.mid[pos_state * 8..], 3)? as usize);
        }
        Some(16 + decode_bit_tree(rd, &mut self.high, 8)? as usize)
    }
}

struct RangeDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    range: u32,
    code: u32,
}

impl<'a> RangeDecoder<'a> {
    fn new(data: &'a [u8]) -> Option<Self> {
        if data.len() < 5 {
            return None;
        }
        let mut code = 0u32;
        for b in &data[..5] {
            code = (code << 8) | *b as u32;
        }
        Some(Self {
            data,
            pos: 5,
            range: 0xFFFF_FFFF,
            code,
        })
    }

    fn decode_bit(&mut self, prob: &mut u16) -> Option<u32> {
        let p = *prob as u32;
        let bound = (self.range >> LZMA_BIT_MODEL_TOTAL_BITS) * p;
        let bit;
        if self.code < bound {
            self.range = bound;
            *prob = (p + ((LZMA_BIT_MODEL_TOTAL - p) >> LZMA_MOVE_BITS)) as u16;
            bit = 0;
        } else {
            self.range -= bound;
            self.code -= bound;
            *prob = (p - (p >> LZMA_MOVE_BITS)) as u16;
            bit = 1;
        }
        self.normalize()?;
        Some(bit)
    }

    fn decode_direct_bits(&mut self, count: usize) -> Option<u32> {
        let mut result = 0u32;
        for _ in 0..count {
            self.range >>= 1;
            result <<= 1;
            if self.code >= self.range {
                self.code -= self.range;
                result |= 1;
            }
            self.normalize()?;
        }
        Some(result)
    }

    fn normalize(&mut self) -> Option<()> {
        if self.range < LZMA_TOP_VALUE {
            self.range <<= 8;
            let b = *self.data.get(self.pos)?;
            self.pos += 1;
            self.code = (self.code << 8) | b as u32;
        }
        Some(())
    }
}

fn resize_probs(v: &mut Vec<u16>, len: usize) {
    v.resize(len, LZMA_PROB_INIT);
    fill_probs(v);
}

fn fill_probs(v: &mut [u16]) {
    for p in v {
        *p = LZMA_PROB_INIT;
    }
}

fn decode_bit_tree(rd: &mut RangeDecoder, probs: &mut [u16], bits: usize) -> Option<u32> {
    let mut symbol = 1usize;
    for _ in 0..bits {
        let bit = rd.decode_bit(&mut probs[symbol])? as usize;
        symbol = (symbol << 1) | bit;
    }
    Some((symbol - (1 << bits)) as u32)
}

fn reverse_decode_bits(rd: &mut RangeDecoder, probs: &mut [u16], bits: usize) -> Option<u32> {
    let mut symbol = 1usize;
    let mut result = 0u32;
    for i in 0..bits {
        let bit = rd.decode_bit(&mut probs[symbol])? as usize;
        symbol = (symbol << 1) | bit;
        result |= (bit as u32) << i;
    }
    Some(result)
}

fn update_literal_state(state: u32) -> u32 {
    if state < 4 {
        0
    } else if state < 10 {
        state - 3
    } else {
        state - 6
    }
}

#[cfg(test)]
mod tests {
    use super::xz_decompress;
    use alloc::vec::Vec;

    #[test]
    fn decodes_uncompressed_lzma2_chunk() {
        let data = [
            0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x01, 0x69, 0x22, 0xde, 0x36, 0x02, 0x00,
            0x21, 0x01, 0x16, 0x00, 0x00, 0x00, 0x74, 0x2f, 0xe5, 0xa3, 0x01, 0x00, 0x08, 0x68,
            0x65, 0x6c, 0x6c, 0x6f, 0x20, 0x78, 0x7a, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x55, 0x7e,
            0x2e, 0x7e, 0x00, 0x01, 0x1d, 0x09, 0x93, 0x61, 0x36, 0xa6, 0x90, 0x42, 0x99, 0x0d,
            0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x59, 0x5a,
        ];
        assert_eq!(
            xz_decompress(&data).as_deref(),
            Some(b"hello xz\n".as_ref())
        );
    }

    #[test]
    fn decodes_compressed_lzma2_chunk() {
        let data = [
            0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x01, 0x69, 0x22, 0xde, 0x36, 0x02, 0x00,
            0x21, 0x01, 0x16, 0x00, 0x00, 0x00, 0x74, 0x2f, 0xe5, 0xa3, 0xe0, 0x03, 0x83, 0x00,
            0x38, 0x5d, 0x00, 0x2a, 0x1a, 0x08, 0xa2, 0x03, 0x25, 0x66, 0xf1, 0x4b, 0x78, 0xc5,
            0xa2, 0x05, 0xff, 0x2e, 0xe6, 0xd9, 0xd2, 0x20, 0x1a, 0xad, 0x34, 0xf8, 0xe2, 0x1d,
            0xe8, 0x41, 0x36, 0xfa, 0xdc, 0x06, 0x69, 0xbb, 0x3c, 0xe4, 0x10, 0x34, 0x27, 0x09,
            0xeb, 0xb3, 0x66, 0xe3, 0xed, 0x37, 0x98, 0xed, 0x92, 0xad, 0xd5, 0x27, 0x3c, 0xc8,
            0x10, 0xc0, 0x00, 0x00, 0xe6, 0x4a, 0x66, 0xb0, 0x00, 0x01, 0x50, 0x84, 0x07, 0x00,
            0x00, 0x00, 0xc1, 0xf2, 0x6a, 0x16, 0x3e, 0x30, 0x0d, 0x8b, 0x02, 0x00, 0x00, 0x00,
            0x00, 0x01, 0x59, 0x5a,
        ];
        let mut expected = Vec::new();
        for _ in 0..20 {
            expected.extend_from_slice(b"The quick brown fox jumps over the lazy dog. ");
        }
        assert_eq!(xz_decompress(&data), Some(expected));
    }
}
