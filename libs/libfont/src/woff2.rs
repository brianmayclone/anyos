// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! WOFF2 → TTF converter (W3C WOFF2 spec, 2022-03-01).
//!
//! Parses a WOFF2 font file, decompresses the Brotli-compressed table data,
//! and reconstructs a valid TrueType/OpenType binary that `TtfFont::parse()`
//! can consume.

use alloc::vec;
use alloc::vec::Vec;

use crate::brotli;

// ═══════════════════════════════════════════════════════════════════
// WOFF2 known table tags (spec §5.1)
// ═══════════════════════════════════════════════════════════════════

static KNOWN_TAGS: [u32; 63] = [
    0x636D6170, // 0  cmap
    0x68656164, // 1  head
    0x68686561, // 2  hhea
    0x686D7478, // 3  hmtx
    0x6D617870, // 4  maxp
    0x6E616D65, // 5  name
    0x4F532F32, // 6  OS/2
    0x706F7374, // 7  post
    0x63767420, // 8  cvt
    0x6670676D, // 9  fpgm
    0x676C7966, // 10 glyf
    0x6C6F6361, // 11 loca
    0x70726570, // 12 prep
    0x43464620, // 13 CFF
    0x564F5247, // 14 VORG
    0x45424454, // 15 EBDT
    0x45424C43, // 16 EBLC
    0x67617370, // 17 gasp
    0x68646D78, // 18 hdmx
    0x6B65726E, // 19 kern
    0x4C545348, // 20 LTSH
    0x50434C54, // 21 PCLT
    0x56444D58, // 22 VDMX
    0x76686561, // 23 vhea
    0x766D7478, // 24 vmtx
    0x42415345, // 25 BASE
    0x47444546, // 26 GDEF
    0x47504F53, // 27 GPOS
    0x47535542, // 28 GSUB
    0x45425343, // 29 EBSC
    0x4A535446, // 30 JSTF
    0x4D415448, // 31 MATH
    0x43424454, // 32 CBDT
    0x43424C43, // 33 CBLC
    0x434F4C52, // 34 COLR
    0x4350414C, // 35 CPAL
    0x53564720, // 36 SVG
    0x73626978, // 37 sbix
    0x61636E74, // 38 acnt
    0x61766172, // 39 avar
    0x62646174, // 40 bdat
    0x626C6F63, // 41 bloc
    0x62736C6E, // 42 bsln
    0x63766172, // 43 cvar
    0x66647363, // 44 fdsc
    0x66656174, // 45 feat
    0x666D7478, // 46 fmtx
    0x66766172, // 47 fvar
    0x67766172, // 48 gvar
    0x68737479, // 49 hsty
    0x6A757374, // 50 just
    0x6C636172, // 51 lcar
    0x6D6F7274, // 52 mort
    0x6D6F7278, // 53 morx
    0x6F706264, // 54 opbd
    0x70726F70, // 55 prop
    0x7472616B, // 56 trak
    0x5A617066, // 57 Zapf
    0x53696C66, // 58 Silf
    0x476C6174, // 59 Glat
    0x476C6F63, // 60 Gloc
    0x46656174, // 61 Feat
    0x53696C6C, // 62 Sill
];

// ═══════════════════════════════════════════════════════════════════
// WOFF2 header and table directory parsing
// ═══════════════════════════════════════════════════════════════════

struct Woff2Table {
    tag: u32,
    transform_length: u32,
    transform_version: u8,
}

struct GlyfReconstruction {
    glyf: Vec<u8>,
    loca: Vec<u8>,
    x_mins: Vec<i16>,
}

/// Read a UIntBase128 variable-length integer (WOFF2 spec §2).
fn read_uint_base128(data: &[u8], pos: &mut usize) -> Option<u32> {
    let mut result = 0u32;
    for i in 0..5 {
        if *pos >= data.len() { return None; }
        let b = data[*pos];
        *pos += 1;
        // Leading zeros are invalid (except for the value 0 itself)
        if i == 0 && b == 0x80 { return None; }
        result = result.checked_mul(128)?.checked_add((b & 0x7F) as u32)?;
        if b & 0x80 == 0 { return Some(result); }
    }
    None // more than 5 bytes
}

fn read_u32_be(data: &[u8], off: usize) -> u32 {
    if off + 4 > data.len() { return 0; }
    ((data[off] as u32) << 24)
        | ((data[off + 1] as u32) << 16)
        | ((data[off + 2] as u32) << 8)
        | (data[off + 3] as u32)
}

fn read_u16_be(data: &[u8], off: usize) -> u16 {
    if off + 2 > data.len() { return 0; }
    ((data[off] as u16) << 8) | (data[off + 1] as u16)
}

fn read_i16_be(data: &[u8], off: usize) -> i16 {
    read_u16_be(data, off) as i16
}

fn write_u32_be(buf: &mut Vec<u8>, val: u32) {
    buf.push((val >> 24) as u8);
    buf.push((val >> 16) as u8);
    buf.push((val >> 8) as u8);
    buf.push(val as u8);
}

fn write_u16_be(buf: &mut Vec<u8>, val: u16) {
    buf.push((val >> 8) as u8);
    buf.push(val as u8);
}

fn write_i16_be(buf: &mut Vec<u8>, val: i16) {
    write_u16_be(buf, val as u16);
}

fn read_255_u16(data: &[u8], pos: &mut usize) -> Option<u16> {
    if *pos >= data.len() {
        return None;
    }
    let code = data[*pos];
    *pos += 1;
    match code {
        253 => {
            if *pos + 2 > data.len() {
                return None;
            }
            let value = read_u16_be(data, *pos);
            *pos += 2;
            Some(value)
        }
        254 => {
            if *pos >= data.len() {
                return None;
            }
            let value = data[*pos] as u16 + 506;
            *pos += 1;
            Some(value)
        }
        255 => {
            if *pos >= data.len() {
                return None;
            }
            let value = data[*pos] as u16 + 253;
            *pos += 1;
            Some(value)
        }
        value => Some(value as u16),
    }
}

/// Convert WOFF2 data to a TTF byte vector.
///
/// Returns `None` if the data is not valid WOFF2 or decompression fails.
pub fn convert_to_ttf(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 48 { return None; }

    // Validate WOFF2 signature
    let signature = read_u32_be(data, 0);
    if signature != 0x774F4632 { return None; } // "wOF2"

    let flavor = read_u32_be(data, 4);
    let _length = read_u32_be(data, 8);
    let num_tables = read_u16_be(data, 12) as usize;
    let _reserved = read_u16_be(data, 14);
    let _total_sfnt_size = read_u32_be(data, 16);
    let total_compressed_size = read_u32_be(data, 20);
    // majorVersion(24), minorVersion(26), metaOffset(28), metaLength(32)
    // metaOrigLength(36), privOffset(40), privLength(44)

    // Parse table directory
    let mut pos = 48usize;
    let mut tables = Vec::with_capacity(num_tables);

    for _ in 0..num_tables {
        if pos >= data.len() { return None; }
        let flags = data[pos];
        pos += 1;

        let tag_idx = flags & 0x3F;
        let transform_version = (flags >> 6) & 0x03;

        let tag = if tag_idx == 0x3F {
            // Arbitrary tag
            if pos + 4 > data.len() { return None; }
            let t = read_u32_be(data, pos);
            pos += 4;
            t
        } else if (tag_idx as usize) < KNOWN_TAGS.len() {
            KNOWN_TAGS[tag_idx as usize]
        } else {
            return None;
        };

        let orig_length = read_uint_base128(data, &mut pos)?;

        // Transform length is present when:
        // - glyf or loca with transform_version == 0 (transformed)
        // - other tables with transform_version != 0
        let has_transform = if tag == 0x676C7966 || tag == 0x6C6F6361 {
            // glyf/loca: transform version 0 = transformed, 3 = no transform
            transform_version == 0
        } else {
            transform_version != 0
        };

        let transform_length = if has_transform {
            read_uint_base128(data, &mut pos)?
        } else {
            orig_length
        };

        tables.push(Woff2Table {
            tag,
            transform_length,
            transform_version,
        });
    }

    // Compressed data starts after table directory
    let compressed_start = pos;
    let compressed_end = (compressed_start + total_compressed_size as usize).min(data.len());
    let compressed_data = &data[compressed_start..compressed_end];

    // Decompress all table data with Brotli
    let decompressed = brotli::decompress(compressed_data)?;

    let expected_decompressed_len = tables.iter().try_fold(0usize, |sum, table| {
        sum.checked_add(table.transform_length as usize)
    })?;
    if decompressed.len() < expected_decompressed_len {
        return None;
    }

    // Split decompressed data into individual tables
    let mut table_data: Vec<Vec<u8>> = Vec::with_capacity(num_tables);
    let mut offset = 0usize;

    for table in &tables {
        let len = table.transform_length as usize;
        table_data.push(decompressed[offset..offset + len].to_vec());
        offset += len;
    }

    // For transformed glyf/loca tables, we need to reconstruct them.
    // Find glyf and loca indices.
    let glyf_idx = tables.iter().position(|t| t.tag == 0x676C7966); // glyf
    let loca_idx = tables.iter().position(|t| t.tag == 0x6C6F6361); // loca

    let mut glyf_reconstruction: Option<GlyfReconstruction> = None;

    // If glyf has transform_version 0, reconstruct glyf+loca from transformed format
    if let Some(gi) = glyf_idx {
        if tables[gi].transform_version == 0 {
            if let Some(li) = loca_idx {
                // Find num_glyphs from maxp table
                let maxp_idx = tables.iter().position(|t| t.tag == 0x6D617870);
                let num_glyphs = if let Some(mi) = maxp_idx {
                    if table_data[mi].len() >= 6 {
                        read_u16_be(&table_data[mi], 4) as u32
                    } else { 0 }
                } else { 0 };

                // Find loca format from head table
                let head_idx = tables.iter().position(|t| t.tag == 0x68656164);
                let loca_format = if let Some(hi) = head_idx {
                    if table_data[hi].len() >= 52 {
                        read_u16_be(&table_data[hi], 50)
                    } else { 0 }
                } else { 0 };

                let reconstruction = reconstruct_glyf_loca(
                    &table_data[gi], num_glyphs, loca_format
                )?;
                table_data[gi] = reconstruction.glyf.clone();
                table_data[li] = reconstruction.loca.clone();
                glyf_reconstruction = Some(reconstruction);
            }
        }
    }

    // For transformed hmtx (transform_version 1), reconstruct omitted bearings
    // from glyf xMin values as defined by the WOFF2 transform.
    if let Some(hi) = tables.iter().position(|t| t.tag == 0x686D7478) {
        if tables[hi].transform_version == 1 {
            let hhea_idx = tables.iter().position(|t| t.tag == 0x68686561)?;
            let maxp_idx = tables.iter().position(|t| t.tag == 0x6D617870)?;
            if table_data[hhea_idx].len() < 36 || table_data[maxp_idx].len() < 6 {
                return None;
            }
            let num_h_metrics = read_u16_be(&table_data[hhea_idx], 34) as usize;
            let num_glyphs = read_u16_be(&table_data[maxp_idx], 4) as usize;
            let x_mins = glyf_reconstruction.as_ref().map(|g| g.x_mins.as_slice()).unwrap_or(&[]);
            table_data[hi] = reconstruct_hmtx(&table_data[hi], num_glyphs, num_h_metrics, x_mins)?;
        }
    }

    // Reconstruct TTF binary
    reconstruct_ttf(flavor, &tables, &table_data)
}

// ═══════════════════════════════════════════════════════════════════
// Transformed glyf/loca reconstruction (WOFF2 spec §5.1)
// ═══════════════════════════════════════════════════════════════════

/// Reconstruct glyf and loca tables from WOFF2 transformed format.
fn reconstruct_glyf_loca(
    transformed: &[u8],
    num_glyphs: u32,
    loca_format: u16,
) -> Option<GlyfReconstruction> {
    if transformed.len() < 36 || num_glyphs == 0 {
        let (glyf, loca) = build_empty_glyf_loca(num_glyphs, loca_format);
        return Some(GlyfReconstruction { glyf, loca, x_mins: vec![0; num_glyphs as usize] });
    }

    let version = read_u32_be(transformed, 0);
    let transformed_num_glyphs = read_u16_be(transformed, 4) as u32;
    let index_format = read_u16_be(transformed, 6);
    if version != 0 || transformed_num_glyphs != num_glyphs {
        return None;
    }

    let n_contour_size = read_u32_be(transformed, 8) as usize;
    let n_points_size = read_u32_be(transformed, 12) as usize;
    let flag_size = read_u32_be(transformed, 16) as usize;
    let glyph_size = read_u32_be(transformed, 20) as usize;
    let composite_size = read_u32_be(transformed, 24) as usize;
    let bbox_size = read_u32_be(transformed, 28) as usize;
    let instruction_size = read_u32_be(transformed, 32) as usize;
    let mut stream_pos = 36usize;
    let n_contour_stream = take_stream(transformed, &mut stream_pos, n_contour_size)?;
    let n_points_stream = take_stream(transformed, &mut stream_pos, n_points_size)?;
    let flag_stream = take_stream(transformed, &mut stream_pos, flag_size)?;
    let glyph_stream = take_stream(transformed, &mut stream_pos, glyph_size)?;
    let composite_stream = take_stream(transformed, &mut stream_pos, composite_size)?;
    let bbox_stream = take_stream(transformed, &mut stream_pos, bbox_size)?;
    let instruction_stream = take_stream(transformed, &mut stream_pos, instruction_size)?;

    let _index_format = index_format;
    let mut n_contours = Vec::with_capacity(num_glyphs as usize);
    for i in 0..num_glyphs as usize {
        if i * 2 + 2 > n_contour_stream.len() {
            return None;
        }
        n_contours.push(read_i16_be(n_contour_stream, i * 2));
    }

    let mut n_points_pos = 0usize;
    let mut flag_pos = 0usize;
    let mut glyph_pos = 0usize;
    let mut composite_pos = 0usize;
    let mut instruction_pos = 0usize;
    let mut bbox_reader = BboxReader::new(bbox_stream, num_glyphs as usize)?;
    let mut glyf = Vec::with_capacity(transformed.len());
    let mut offsets = Vec::with_capacity(num_glyphs as usize + 1);
    let mut x_mins = Vec::with_capacity(num_glyphs as usize);

    for glyph_id in 0..num_glyphs as usize {
        offsets.push(glyf.len() as u32);
        let nc = n_contours[glyph_id];
        if nc == 0 {
            x_mins.push(0);
            continue;
        }

        let explicit_bbox = bbox_reader.bbox_for(glyph_id)?;
        let start_len = glyf.len();

        if nc > 0 {
            let bbox = build_simple_glyph(
                &mut glyf,
                nc as usize,
                explicit_bbox,
                n_points_stream,
                &mut n_points_pos,
                flag_stream,
                &mut flag_pos,
                glyph_stream,
                &mut glyph_pos,
                instruction_stream,
                &mut instruction_pos,
            )?;
            x_mins.push(bbox.0);
        } else {
            let bbox = build_composite_glyph(
                &mut glyf,
                explicit_bbox.unwrap_or((0, 0, 0, 0)),
                composite_stream,
                &mut composite_pos,
                instruction_stream,
                &mut instruction_pos,
            )?;
            x_mins.push(bbox.0);
        }

        if glyf.len() > start_len {
            while glyf.len() & 3 != 0 { glyf.push(0); }
        }
    }
    offsets.push(glyf.len() as u32);

    let loca = build_loca_table(&offsets, loca_format);
    Some(GlyfReconstruction { glyf, loca, x_mins })
}

fn take_stream<'a>(data: &'a [u8], pos: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = pos.checked_add(len)?;
    if end > data.len() {
        return None;
    }
    let stream = &data[*pos..end];
    *pos = end;
    Some(stream)
}

struct BboxReader<'a> {
    bitmap: &'a [u8],
    values: &'a [u8],
    pos: usize,
}

impl<'a> BboxReader<'a> {
    fn new(data: &'a [u8], num_glyphs: usize) -> Option<Self> {
        let bitmap_len = ((num_glyphs + 31) / 32).checked_mul(4)?;
        if bitmap_len > data.len() {
            return None;
        }
        Some(Self { bitmap: &data[..bitmap_len], values: &data[bitmap_len..], pos: 0 })
    }

    fn bbox_for(&mut self, glyph_id: usize) -> Option<Option<(i16, i16, i16, i16)>> {
        let byte = *self.bitmap.get(glyph_id / 8).unwrap_or(&0);
        let has_bbox = (byte & (0x80 >> (glyph_id & 7))) != 0;
        if !has_bbox {
            return Some(None);
        }
        if self.pos + 8 > self.values.len() {
            return None;
        }
        let bbox = (
            read_i16_be(self.values, self.pos),
            read_i16_be(self.values, self.pos + 2),
            read_i16_be(self.values, self.pos + 4),
            read_i16_be(self.values, self.pos + 6),
        );
        self.pos += 8;
        Some(Some(bbox))
    }
}

fn build_simple_glyph(
    out: &mut Vec<u8>,
    contour_count: usize,
    explicit_bbox: Option<(i16, i16, i16, i16)>,
    n_points_stream: &[u8],
    n_points_pos: &mut usize,
    flag_stream: &[u8],
    flag_pos: &mut usize,
    glyph_stream: &[u8],
    glyph_pos: &mut usize,
    instruction_stream: &[u8],
    instruction_pos: &mut usize,
) -> Option<(i16, i16, i16, i16)> {
    let mut end_pts = Vec::with_capacity(contour_count);
    let mut point_count = 0usize;
    for _ in 0..contour_count {
        let count = read_255_u16(n_points_stream, n_points_pos)? as usize;
        if count == 0 {
            return None;
        }
        point_count = point_count.checked_add(count)?;
        if point_count > u16::MAX as usize + 1 {
            return None;
        }
        end_pts.push((point_count - 1) as u16);
    }

    let mut flags = Vec::with_capacity(point_count);
    let mut x_coords = Vec::with_capacity(point_count);
    let mut y_coords = Vec::with_capacity(point_count);
    let mut x = 0i16;
    let mut y = 0i16;
    for _ in 0..point_count {
        if *flag_pos >= flag_stream.len() {
            return None;
        }
        let transformed_flag = flag_stream[*flag_pos];
        *flag_pos += 1;
        let (dx, dy) = decode_triplet(transformed_flag & 0x7f, glyph_stream, glyph_pos)?;
        x = x.wrapping_add(dx);
        y = y.wrapping_add(dy);
        flags.push(if (transformed_flag & 0x80) != 0 { 0x01 } else { 0x00 });
        x_coords.push(x);
        y_coords.push(y);
    }

    let computed_bbox = compute_bbox(&x_coords, &y_coords);
    let bbox = explicit_bbox.unwrap_or(computed_bbox);
    write_i16_be(out, contour_count as i16);
    write_i16_be(out, bbox.0);
    write_i16_be(out, bbox.1);
    write_i16_be(out, bbox.2);
    write_i16_be(out, bbox.3);
    for end_pt in end_pts {
        write_u16_be(out, end_pt);
    }
    let instruction_len = read_255_u16(instruction_stream, instruction_pos)? as usize;
    if *instruction_pos + instruction_len > instruction_stream.len() {
        return None;
    }
    write_u16_be(out, instruction_len as u16);
    out.extend_from_slice(&instruction_stream[*instruction_pos..*instruction_pos + instruction_len]);
    *instruction_pos += instruction_len;

    let (ttf_flags, x_bytes, y_bytes) = encode_ttf_points(&flags, &x_coords, &y_coords);
    out.extend_from_slice(&ttf_flags);
    out.extend_from_slice(&x_bytes);
    out.extend_from_slice(&y_bytes);
    Some(bbox)
}

fn build_composite_glyph(
    out: &mut Vec<u8>,
    bbox: (i16, i16, i16, i16),
    composite_stream: &[u8],
    composite_pos: &mut usize,
    instruction_stream: &[u8],
    instruction_pos: &mut usize,
) -> Option<(i16, i16, i16, i16)> {
    write_i16_be(out, -1);
    write_i16_be(out, bbox.0);
    write_i16_be(out, bbox.1);
    write_i16_be(out, bbox.2);
    write_i16_be(out, bbox.3);

    let mut have_instructions = false;
    loop {
        if *composite_pos + 4 > composite_stream.len() {
            return None;
        }
        let flags = read_u16_be(composite_stream, *composite_pos);
        let glyph_index = read_u16_be(composite_stream, *composite_pos + 2);
        *composite_pos += 4;
        write_u16_be(out, flags);
        write_u16_be(out, glyph_index);

        let arg_len = if (flags & 0x0001) != 0 { 4 } else { 2 };
        if *composite_pos + arg_len > composite_stream.len() {
            return None;
        }
        out.extend_from_slice(&composite_stream[*composite_pos..*composite_pos + arg_len]);
        *composite_pos += arg_len;

        let scale_len = if (flags & 0x0008) != 0 {
            2
        } else if (flags & 0x0040) != 0 {
            4
        } else if (flags & 0x0080) != 0 {
            8
        } else {
            0
        };
        if *composite_pos + scale_len > composite_stream.len() {
            return None;
        }
        out.extend_from_slice(&composite_stream[*composite_pos..*composite_pos + scale_len]);
        *composite_pos += scale_len;

        have_instructions |= (flags & 0x0100) != 0;
        if (flags & 0x0020) == 0 {
            break;
        }
    }

    if have_instructions {
        let instruction_len = read_255_u16(instruction_stream, instruction_pos)? as usize;
        if *instruction_pos + instruction_len > instruction_stream.len() {
            return None;
        }
        write_u16_be(out, instruction_len as u16);
        out.extend_from_slice(&instruction_stream[*instruction_pos..*instruction_pos + instruction_len]);
        *instruction_pos += instruction_len;
    }
    Some(bbox)
}

fn decode_triplet(flag: u8, data: &[u8], pos: &mut usize) -> Option<(i16, i16)> {
    fn signed(value: i16, positive: bool) -> i16 {
        if positive { value } else { -value }
    }
    fn signed_pair(x: i16, y: i16, code: u8) -> (i16, i16) {
        match code & 3 {
            0 => (-x, -y),
            1 => (x, -y),
            2 => (-x, y),
            _ => (x, y),
        }
    }

    match flag {
        0..=9 => {
            if *pos >= data.len() { return None; }
            let b = data[*pos] as i16;
            *pos += 1;
            Some((0, signed(((flag / 2) as i16 * 256) + b, (flag & 1) != 0)))
        }
        10..=19 => {
            if *pos >= data.len() { return None; }
            let b = data[*pos] as i16;
            *pos += 1;
            Some((signed((((flag - 10) / 2) as i16 * 256) + b, (flag & 1) != 0), 0))
        }
        20..=83 => {
            if *pos >= data.len() { return None; }
            let b = data[*pos];
            *pos += 1;
            let index = flag - 20;
            let dx = (((index >> 4) as i16) * 16) + ((b >> 4) as i16) + 1;
            let dy = ((((index >> 2) & 3) as i16) * 16) + ((b & 0x0f) as i16) + 1;
            Some(signed_pair(dx, dy, index))
        }
        84..=119 => {
            if *pos + 2 > data.len() { return None; }
            let b0 = data[*pos] as i16;
            let b1 = data[*pos + 1] as i16;
            *pos += 2;
            let index = flag - 84;
            let dx = (((index / 12) as i16) * 256) + b0 + 1;
            let dy = ((((index / 4) % 3) as i16) * 256) + b1 + 1;
            Some(signed_pair(dx, dy, index))
        }
        120..=123 => {
            if *pos + 3 > data.len() { return None; }
            let b0 = data[*pos] as i16;
            let b1 = data[*pos + 1] as i16;
            let b2 = data[*pos + 2] as i16;
            *pos += 3;
            let dx = (b0 << 4) | (b1 >> 4);
            let dy = ((b1 & 0x0f) << 8) | b2;
            Some(signed_pair(dx, dy, flag - 120))
        }
        124..=127 => {
            if *pos + 4 > data.len() { return None; }
            let dx = read_u16_be(data, *pos) as i16;
            let dy = read_u16_be(data, *pos + 2) as i16;
            *pos += 4;
            Some(signed_pair(dx, dy, flag - 124))
        }
        _ => None,
    }
}

fn compute_bbox(xs: &[i16], ys: &[i16]) -> (i16, i16, i16, i16) {
    if xs.is_empty() || ys.is_empty() {
        return (0, 0, 0, 0);
    }
    let mut x_min = xs[0];
    let mut y_min = ys[0];
    let mut x_max = xs[0];
    let mut y_max = ys[0];
    for (&x, &y) in xs.iter().zip(ys.iter()).skip(1) {
        if x < x_min { x_min = x; }
        if y < y_min { y_min = y; }
        if x > x_max { x_max = x; }
        if y > y_max { y_max = y; }
    }
    (x_min, y_min, x_max, y_max)
}

fn encode_ttf_points(on_curve_flags: &[u8], xs: &[i16], ys: &[i16]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut flags = Vec::with_capacity(on_curve_flags.len());
    let mut x_bytes = Vec::new();
    let mut y_bytes = Vec::new();
    let mut prev_x = 0i16;
    let mut prev_y = 0i16;

    for ((&base_flag, &x), &y) in on_curve_flags.iter().zip(xs.iter()).zip(ys.iter()) {
        let mut flag = base_flag & 0x01;
        let dx = x.wrapping_sub(prev_x);
        let dy = y.wrapping_sub(prev_y);

        if dx == 0 {
            flag |= 0x10;
        } else if (1..=255).contains(&dx) {
            flag |= 0x02 | 0x10;
            x_bytes.push(dx as u8);
        } else if (-255..=-1).contains(&dx) {
            flag |= 0x02;
            x_bytes.push((-dx) as u8);
        } else {
            write_i16_be(&mut x_bytes, dx);
        }

        if dy == 0 {
            flag |= 0x20;
        } else if (1..=255).contains(&dy) {
            flag |= 0x04 | 0x20;
            y_bytes.push(dy as u8);
        } else if (-255..=-1).contains(&dy) {
            flag |= 0x04;
            y_bytes.push((-dy) as u8);
        } else {
            write_i16_be(&mut y_bytes, dy);
        }

        flags.push(flag);
        prev_x = x;
        prev_y = y;
    }

    (flags, x_bytes, y_bytes)
}

fn reconstruct_hmtx(
    transformed: &[u8],
    num_glyphs: usize,
    num_h_metrics: usize,
    x_mins: &[i16],
) -> Option<Vec<u8>> {
    if transformed.is_empty() || num_h_metrics == 0 || num_h_metrics > num_glyphs {
        return None;
    }
    let flags = transformed[0];
    if flags == 0 || (flags & 0xfc) != 0 {
        return None;
    }
    let mut pos = 1usize;
    let mut out = Vec::with_capacity(num_h_metrics * 4 + (num_glyphs - num_h_metrics) * 2);

    let mut advances = Vec::with_capacity(num_h_metrics);
    for _ in 0..num_h_metrics {
        if pos + 2 > transformed.len() {
            return None;
        }
        advances.push(read_u16_be(transformed, pos));
        pos += 2;
    }

    let mut proportional_lsbs = Vec::with_capacity(num_h_metrics);
    if (flags & 0x01) != 0 {
        for glyph_id in 0..num_h_metrics {
            proportional_lsbs.push(*x_mins.get(glyph_id).unwrap_or(&0));
        }
    } else {
        for _ in 0..num_h_metrics {
            if pos + 2 > transformed.len() {
                return None;
            }
            proportional_lsbs.push(read_i16_be(transformed, pos));
            pos += 2;
        }
    }

    let mut mono_lsbs = Vec::with_capacity(num_glyphs - num_h_metrics);
    if (flags & 0x02) != 0 {
        for glyph_id in num_h_metrics..num_glyphs {
            mono_lsbs.push(*x_mins.get(glyph_id).unwrap_or(&0));
        }
    } else {
        for _ in num_h_metrics..num_glyphs {
            if pos + 2 > transformed.len() {
                return None;
            }
            mono_lsbs.push(read_i16_be(transformed, pos));
            pos += 2;
        }
    }

    for (&advance, &lsb) in advances.iter().zip(proportional_lsbs.iter()) {
        write_u16_be(&mut out, advance);
        write_i16_be(&mut out, lsb);
    }
    for &lsb in &mono_lsbs {
        write_i16_be(&mut out, lsb);
    }

    Some(out)
}

fn build_empty_glyf_loca(num_glyphs: u32, loca_format: u16) -> (Vec<u8>, Vec<u8>) {
    let glyf = Vec::new();
    let entries = num_glyphs as usize + 1;
    let mut loca = Vec::with_capacity(entries * if loca_format == 0 { 2 } else { 4 });
    for _ in 0..entries {
        if loca_format == 0 {
            write_u16_be(&mut loca, 0);
        } else {
            write_u32_be(&mut loca, 0);
        }
    }
    (glyf, loca)
}

fn build_loca_table(offsets: &[u32], loca_format: u16) -> Vec<u8> {
    let mut loca = Vec::with_capacity(offsets.len() * if loca_format == 0 { 2 } else { 4 });
    for &off in offsets {
        if loca_format == 0 {
            write_u16_be(&mut loca, (off / 2) as u16);
        } else {
            write_u32_be(&mut loca, off);
        }
    }
    loca
}

// ═══════════════════════════════════════════════════════════════════
// TTF binary reconstruction
// ═══════════════════════════════════════════════════════════════════

fn reconstruct_ttf(
    flavor: u32,
    tables: &[Woff2Table],
    table_data: &[Vec<u8>],
) -> Option<Vec<u8>> {
    let num_tables = tables.len();
    if num_tables == 0 { return None; }

    // Calculate search range values for the offset table (integer log2)
    let entry_selector = {
        let mut es = 0u32;
        let mut v = num_tables as u32;
        while v > 1 { v >>= 1; es += 1; }
        es
    };
    let search_range = (1u32 << entry_selector) * 16;
    let range_shift = (num_tables as u32 * 16).saturating_sub(search_range);

    // Build the offset table (12 bytes)
    let mut ttf = Vec::with_capacity(16384);
    write_u32_be(&mut ttf, flavor);                     // sfVersion / flavor
    write_u16_be(&mut ttf, num_tables as u16);          // numTables
    write_u16_be(&mut ttf, search_range as u16);        // searchRange
    write_u16_be(&mut ttf, entry_selector as u16);      // entrySelector
    write_u16_be(&mut ttf, range_shift as u16);         // rangeShift

    // Reserve space for table directory (16 bytes per entry)
    let dir_start = ttf.len();
    let dir_size = num_tables * 16;
    ttf.resize(dir_start + dir_size, 0);

    // Write table data and fill directory
    for (i, (table, data)) in tables.iter().zip(table_data.iter()).enumerate() {
        let dir_entry = dir_start + i * 16;

        // Align to 4-byte boundary
        while ttf.len() & 3 != 0 { ttf.push(0); }
        let current_offset = ttf.len() as u32;

        // Write table data
        ttf.extend_from_slice(data);

        // Calculate checksum
        let checksum = table_checksum(data);

        // Fill directory entry: tag(4) checksum(4) offset(4) length(4)
        ttf[dir_entry..dir_entry + 4].copy_from_slice(&table.tag.to_be_bytes());
        ttf[dir_entry + 4..dir_entry + 8].copy_from_slice(&checksum.to_be_bytes());
        ttf[dir_entry + 8..dir_entry + 12].copy_from_slice(&current_offset.to_be_bytes());
        ttf[dir_entry + 12..dir_entry + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
    }

    // Pad to 4-byte boundary
    while ttf.len() & 3 != 0 { ttf.push(0); }

    Some(ttf)
}

fn table_checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 4 <= data.len() {
        sum = sum.wrapping_add(
            ((data[i] as u32) << 24)
                | ((data[i + 1] as u32) << 16)
                | ((data[i + 2] as u32) << 8)
                | (data[i + 3] as u32),
        );
        i += 4;
    }
    // Handle remaining bytes
    if i < data.len() {
        let mut last = 0u32;
        for j in i..data.len() {
            last |= (data[j] as u32) << (24 - (j - i) as u32 * 8);
        }
        sum = sum.wrapping_add(last);
    }
    sum
}
