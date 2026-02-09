#![allow(dead_code)]
//! TrueType font parser for `no_std` environments.
//!
//! Zero-copy parser that operates on `&[u8]` references into raw TTF data.
//! Supports simple and composite glyphs, cmap format 4 character mapping,
//! and horizontal metrics.

use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Big-endian reader helpers
// ---------------------------------------------------------------------------

/// Read a big-endian `u16` from `data` at `offset`.
#[inline]
fn read_u16_be(data: &[u8], offset: usize) -> u16 {
    let b = &data[offset..offset + 2];
    (b[0] as u16) << 8 | b[1] as u16
}

/// Read a big-endian `i16` from `data` at `offset`.
#[inline]
fn read_i16_be(data: &[u8], offset: usize) -> i16 {
    read_u16_be(data, offset) as i16
}

/// Read a big-endian `u32` from `data` at `offset`.
#[inline]
fn read_u32_be(data: &[u8], offset: usize) -> u32 {
    let b = &data[offset..offset + 4];
    (b[0] as u32) << 24 | (b[1] as u32) << 16 | (b[2] as u32) << 8 | b[3] as u32
}

// ---------------------------------------------------------------------------
// Glyph outline types
// ---------------------------------------------------------------------------

/// A single point in a glyph outline.
#[derive(Debug, Clone, Copy)]
pub struct GlyphPoint {
    pub x: i16,
    pub y: i16,
    pub on_curve: bool,
}

/// Parsed glyph outline data (simple or flattened composite).
#[derive(Debug, Clone)]
pub struct GlyphOutline {
    pub num_contours: i16,
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
    pub contour_ends: Vec<u16>,
    pub points: Vec<GlyphPoint>,
}

// ---------------------------------------------------------------------------
// Simple-glyph flag bits
// ---------------------------------------------------------------------------

const ON_CURVE_POINT: u8 = 0x01;
const X_SHORT_VECTOR: u8 = 0x02;
const Y_SHORT_VECTOR: u8 = 0x04;
const REPEAT_FLAG: u8 = 0x08;
const X_IS_SAME_OR_POSITIVE: u8 = 0x10;
const Y_IS_SAME_OR_POSITIVE: u8 = 0x20;

// ---------------------------------------------------------------------------
// Composite-glyph flag bits
// ---------------------------------------------------------------------------

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

// ---------------------------------------------------------------------------
// TtfFont
// ---------------------------------------------------------------------------

/// Parsed TrueType font.
pub struct TtfFont {
    /// Raw TTF file data.
    pub data: Vec<u8>,

    /// Number of glyphs in the font (from `maxp`).
    pub num_glyphs: u16,
    /// Design units per em square (from `head`).
    pub units_per_em: u16,
    /// Typographic ascent (from `hhea`).
    pub ascent: i16,
    /// Typographic descent (from `hhea`, typically negative).
    pub descent: i16,
    /// Line gap (from `hhea`).
    pub line_gap: i16,
    /// Index-to-loc format: 0 = short (u16 offsets), 1 = long (u32 offsets).
    pub loca_format: i16,

    /// Byte offset of the `cmap` table in `data`.
    pub cmap_offset: usize,
    /// Byte offset of the `glyf` table in `data`.
    pub glyf_offset: usize,
    /// Byte offset of the `loca` table in `data`.
    pub loca_offset: usize,
    /// Byte offset of the `hmtx` table in `data`.
    pub hmtx_offset: usize,

    /// Number of long horizontal metrics (from `hhea`).
    pub num_h_metrics: u16,
}

impl TtfFont {
    // ------------------------------------------------------------------
    // Parsing
    // ------------------------------------------------------------------

    /// Parse a TrueType font from raw file data.
    ///
    /// Returns `None` if the data is too short, has an invalid magic number,
    /// or is missing any required table.
    pub fn parse(data: Vec<u8>) -> Option<Self> {
        if data.len() < 12 {
            return None;
        }

        // Validate sfVersion: 0x00010000 (TrueType) or 0x74727565 ('true')
        let magic = read_u32_be(&data, 0);
        if magic != 0x00010000 && magic != 0x74727565 {
            return None;
        }

        let num_tables = read_u16_be(&data, 4) as usize;
        if data.len() < 12 + num_tables * 16 {
            return None;
        }

        // Walk the table directory and collect offsets we need.
        let mut head_off: Option<usize> = None;
        let mut maxp_off: Option<usize> = None;
        let mut cmap_off: Option<usize> = None;
        let mut hhea_off: Option<usize> = None;
        let mut hmtx_off: Option<usize> = None;
        let mut loca_off: Option<usize> = None;
        let mut glyf_off: Option<usize> = None;

        for i in 0..num_tables {
            let rec = 12 + i * 16;
            let tag = read_u32_be(&data, rec);
            let offset = read_u32_be(&data, rec + 8) as usize;

            match tag {
                0x68656164 => head_off = Some(offset), // 'head'
                0x6D617870 => maxp_off = Some(offset), // 'maxp'
                0x636D6170 => cmap_off = Some(offset), // 'cmap'
                0x68686561 => hhea_off = Some(offset), // 'hhea'
                0x686D7478 => hmtx_off = Some(offset), // 'hmtx'
                0x6C6F6361 => loca_off = Some(offset), // 'loca'
                0x676C7966 => glyf_off = Some(offset), // 'glyf'
                _ => {}
            }
        }

        let head_off = head_off?;
        let maxp_off = maxp_off?;
        let cmap_off = cmap_off?;
        let hhea_off = hhea_off?;
        let hmtx_off = hmtx_off?;
        let loca_off = loca_off?;
        let glyf_off = glyf_off?;

        // --- head table (minimum 54 bytes) ---
        if data.len() < head_off + 54 {
            return None;
        }
        let units_per_em = read_u16_be(&data, head_off + 18);
        let loca_format = read_i16_be(&data, head_off + 50);

        // --- maxp table (minimum 6 bytes) ---
        if data.len() < maxp_off + 6 {
            return None;
        }
        let num_glyphs = read_u16_be(&data, maxp_off + 4);

        // --- hhea table (minimum 36 bytes) ---
        if data.len() < hhea_off + 36 {
            return None;
        }
        let ascent = read_i16_be(&data, hhea_off + 4);
        let descent = read_i16_be(&data, hhea_off + 6);
        let line_gap = read_i16_be(&data, hhea_off + 8);
        let num_h_metrics = read_u16_be(&data, hhea_off + 34);

        Some(TtfFont {
            data,
            num_glyphs,
            units_per_em,
            ascent,
            descent,
            line_gap,
            loca_format,
            cmap_offset: cmap_off,
            glyf_offset: glyf_off,
            loca_offset: loca_off,
            hmtx_offset: hmtx_off,
            num_h_metrics,
        })
    }

    // ------------------------------------------------------------------
    // cmap: character -> glyph mapping (format 4)
    // ------------------------------------------------------------------

    /// Map a Unicode codepoint to a glyph index using the cmap format 4
    /// subtable.  Returns 0 (the `.notdef` glyph) if the codepoint is not
    /// found or the font lacks a suitable subtable.
    pub fn char_to_glyph(&self, codepoint: u32) -> u16 {
        // We only handle BMP codepoints with format 4.
        if codepoint > 0xFFFF {
            return 0;
        }
        let cp = codepoint as u16;

        let d = &self.data;
        let cmap = self.cmap_offset;

        if d.len() < cmap + 4 {
            return 0;
        }

        let num_subtables = read_u16_be(d, cmap + 2) as usize;
        if d.len() < cmap + 4 + num_subtables * 8 {
            return 0;
        }

        // Find a format 4 subtable (prefer platform 3 encoding 1 = Windows
        // Unicode BMP, but accept platform 0 as well).
        let mut subtable_offset: Option<usize> = None;
        for i in 0..num_subtables {
            let rec = cmap + 4 + i * 8;
            let platform = read_u16_be(d, rec);
            let encoding = read_u16_be(d, rec + 2);
            let offset = read_u32_be(d, rec + 4) as usize;
            let abs = cmap + offset;

            if d.len() < abs + 4 {
                continue;
            }
            let format = read_u16_be(d, abs);
            if format != 4 {
                continue;
            }

            // Prefer Windows BMP (3,1) but fall back to Unicode (0,*)
            if platform == 3 && encoding == 1 {
                subtable_offset = Some(abs);
                break;
            }
            if platform == 0 && subtable_offset.is_none() {
                subtable_offset = Some(abs);
            }
        }

        let sub = match subtable_offset {
            Some(o) => o,
            None => return 0,
        };

        // Parse format 4 header.
        if d.len() < sub + 14 {
            return 0;
        }
        let seg_count_x2 = read_u16_be(d, sub + 6) as usize;
        let seg_count = seg_count_x2 / 2;
        if seg_count == 0 {
            return 0;
        }

        // Array bases (all relative to subtable start).
        let end_code_base = sub + 14;
        // +2 for reservedPad after endCode array
        let start_code_base = end_code_base + seg_count_x2 + 2;
        let id_delta_base = start_code_base + seg_count_x2;
        let id_range_offset_base = id_delta_base + seg_count_x2;

        let table_end = id_range_offset_base + seg_count_x2;
        if d.len() < table_end {
            return 0;
        }

        // Binary search on end_code to find the first segment whose
        // end_code >= cp.
        let mut lo: usize = 0;
        let mut hi: usize = seg_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let end_code = read_u16_be(d, end_code_base + mid * 2);
            if end_code < cp {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo >= seg_count {
            return 0;
        }

        let seg = lo;
        let end_code = read_u16_be(d, end_code_base + seg * 2);
        let start_code = read_u16_be(d, start_code_base + seg * 2);

        if cp < start_code || cp > end_code {
            return 0;
        }

        let id_delta = read_i16_be(d, id_delta_base + seg * 2);
        let id_range_offset = read_u16_be(d, id_range_offset_base + seg * 2);

        if id_range_offset == 0 {
            // Glyph index = (codepoint + idDelta) mod 65536
            return (cp as i32 + id_delta as i32) as u16;
        }

        // id_range_offset is a byte offset from its own position into the
        // glyphIdArray.
        let glyph_id_addr = id_range_offset_base
            + seg * 2
            + id_range_offset as usize
            + (cp as usize - start_code as usize) * 2;

        if glyph_id_addr + 2 > d.len() {
            return 0;
        }

        let glyph_id = read_u16_be(d, glyph_id_addr);
        if glyph_id == 0 {
            return 0;
        }

        (glyph_id as i32 + id_delta as i32) as u16
    }

    // ------------------------------------------------------------------
    // loca: glyph -> byte offset inside `glyf`
    // ------------------------------------------------------------------

    /// Return the byte offset (relative to the start of the `glyf` table) for
    /// `glyph_id`.  Returns `None` if the glyph_id is out of range or the
    /// glyph is empty (start == end in loca).
    pub fn glyph_offset(&self, glyph_id: u16) -> Option<usize> {
        if glyph_id >= self.num_glyphs {
            return None;
        }
        let d = &self.data;
        let loca = self.loca_offset;
        let id = glyph_id as usize;

        let (off, next_off) = if self.loca_format == 0 {
            // Short format: offsets stored as u16, actual offset = value * 2.
            let base = loca + id * 2;
            if d.len() < base + 4 {
                return None;
            }
            let o = read_u16_be(d, base) as usize * 2;
            let n = read_u16_be(d, base + 2) as usize * 2;
            (o, n)
        } else {
            // Long format: offsets stored as u32.
            let base = loca + id * 4;
            if d.len() < base + 8 {
                return None;
            }
            let o = read_u32_be(d, base) as usize;
            let n = read_u32_be(d, base + 4) as usize;
            (o, n)
        };

        if off == next_off {
            // Empty glyph (e.g. space).
            return None;
        }

        Some(off)
    }

    // ------------------------------------------------------------------
    // hmtx: horizontal metrics
    // ------------------------------------------------------------------

    /// Return the advance width for `glyph_id`.
    ///
    /// Glyphs beyond `num_h_metrics` share the advance width of the last
    /// entry in the `longHorMetric` array.
    pub fn advance_width(&self, glyph_id: u16) -> u16 {
        let d = &self.data;
        let hmtx = self.hmtx_offset;

        let idx = if glyph_id < self.num_h_metrics {
            glyph_id as usize
        } else {
            (self.num_h_metrics as usize).saturating_sub(1)
        };

        let off = hmtx + idx * 4;
        if d.len() < off + 2 {
            return 0;
        }
        read_u16_be(d, off)
    }

    /// Return the left side bearing for `glyph_id`.
    ///
    /// For glyphs within the `longHorMetric` array the LSB is at offset +2 in
    /// each 4-byte record.  For glyphs beyond `num_h_metrics` the LSB is in
    /// the trailing `leftSideBearing` array.
    pub fn lsb(&self, glyph_id: u16) -> i16 {
        let d = &self.data;
        let hmtx = self.hmtx_offset;
        let nhm = self.num_h_metrics as usize;

        if (glyph_id as usize) < nhm {
            let off = hmtx + glyph_id as usize * 4 + 2;
            if d.len() < off + 2 {
                return 0;
            }
            read_i16_be(d, off)
        } else {
            // Trailing leftSideBearing array starts right after the
            // longHorMetric records.
            let lsb_base = hmtx + nhm * 4;
            let idx = glyph_id as usize - nhm;
            let off = lsb_base + idx * 2;
            if d.len() < off + 2 {
                return 0;
            }
            read_i16_be(d, off)
        }
    }

    // ------------------------------------------------------------------
    // glyf: glyph outlines
    // ------------------------------------------------------------------

    /// Parse the outline for `glyph_id`.
    ///
    /// Returns `None` for empty glyphs (e.g. space) or if the glyph_id is out
    /// of range.  Handles both simple glyphs and composite glyphs
    /// (recursively resolving components).
    pub fn glyph_outline(&self, glyph_id: u16) -> Option<GlyphOutline> {
        self.glyph_outline_depth(glyph_id, 0)
    }

    /// Internal recursive parser with depth limit to prevent infinite loops
    /// on malformed fonts.
    fn glyph_outline_depth(&self, glyph_id: u16, depth: u32) -> Option<GlyphOutline> {
        if depth > 32 {
            return None; // prevent runaway recursion
        }

        let rel_off = self.glyph_offset(glyph_id)?;
        let abs = self.glyf_offset + rel_off;
        let d = &self.data;

        if d.len() < abs + 10 {
            return None;
        }

        let num_contours = read_i16_be(d, abs);
        let x_min = read_i16_be(d, abs + 2);
        let y_min = read_i16_be(d, abs + 4);
        let x_max = read_i16_be(d, abs + 6);
        let y_max = read_i16_be(d, abs + 8);

        if num_contours >= 0 {
            self.parse_simple_glyph(abs, num_contours, x_min, y_min, x_max, y_max)
        } else {
            self.parse_composite_glyph(abs, x_min, y_min, x_max, y_max, depth)
        }
    }

    /// Parse a simple glyph at absolute offset `abs`.
    fn parse_simple_glyph(
        &self,
        abs: usize,
        num_contours: i16,
        x_min: i16,
        y_min: i16,
        x_max: i16,
        y_max: i16,
    ) -> Option<GlyphOutline> {
        let d = &self.data;
        let nc = num_contours as usize;

        // Read contour end-point indices.
        let mut off = abs + 10;
        if d.len() < off + nc * 2 {
            return None;
        }

        let mut contour_ends = Vec::with_capacity(nc);
        for i in 0..nc {
            contour_ends.push(read_u16_be(d, off + i * 2));
        }
        off += nc * 2;

        let num_points = match contour_ends.last() {
            Some(&last) => last as usize + 1,
            None => {
                return Some(GlyphOutline {
                    num_contours,
                    x_min,
                    y_min,
                    x_max,
                    y_max,
                    contour_ends,
                    points: Vec::new(),
                });
            }
        };

        // Skip instructions.
        if d.len() < off + 2 {
            return None;
        }
        let instruction_len = read_u16_be(d, off) as usize;
        off += 2 + instruction_len;

        if off > d.len() {
            return None;
        }

        // --- Parse flags ---
        let mut flags: Vec<u8> = Vec::with_capacity(num_points);
        while flags.len() < num_points {
            if off >= d.len() {
                return None;
            }
            let flag = d[off];
            off += 1;
            flags.push(flag);

            if flag & REPEAT_FLAG != 0 {
                if off >= d.len() {
                    return None;
                }
                let repeat = d[off] as usize;
                off += 1;
                for _ in 0..repeat {
                    if flags.len() >= num_points {
                        break;
                    }
                    flags.push(flag);
                }
            }
        }

        // --- Parse X coordinates ---
        let mut x_coords: Vec<i16> = Vec::with_capacity(num_points);
        let mut x: i16 = 0;
        for i in 0..num_points {
            let f = flags[i];
            if f & X_SHORT_VECTOR != 0 {
                // 1-byte delta
                if off >= d.len() {
                    return None;
                }
                let dx = d[off] as i16;
                off += 1;
                if f & X_IS_SAME_OR_POSITIVE != 0 {
                    x += dx; // positive
                } else {
                    x -= dx; // negative
                }
            } else if f & X_IS_SAME_OR_POSITIVE != 0 {
                // x is same as previous (delta = 0)
            } else {
                // 2-byte signed delta
                if off + 2 > d.len() {
                    return None;
                }
                x += read_i16_be(d, off);
                off += 2;
            }
            x_coords.push(x);
        }

        // --- Parse Y coordinates ---
        let mut y_coords: Vec<i16> = Vec::with_capacity(num_points);
        let mut y: i16 = 0;
        for i in 0..num_points {
            let f = flags[i];
            if f & Y_SHORT_VECTOR != 0 {
                if off >= d.len() {
                    return None;
                }
                let dy = d[off] as i16;
                off += 1;
                if f & Y_IS_SAME_OR_POSITIVE != 0 {
                    y += dy;
                } else {
                    y -= dy;
                }
            } else if f & Y_IS_SAME_OR_POSITIVE != 0 {
                // y is same as previous
            } else {
                if off + 2 > d.len() {
                    return None;
                }
                y += read_i16_be(d, off);
                off += 2;
            }
            y_coords.push(y);
        }

        // --- Build points ---
        let mut points = Vec::with_capacity(num_points);
        for i in 0..num_points {
            points.push(GlyphPoint {
                x: x_coords[i],
                y: y_coords[i],
                on_curve: flags[i] & ON_CURVE_POINT != 0,
            });
        }

        Some(GlyphOutline {
            num_contours,
            x_min,
            y_min,
            x_max,
            y_max,
            contour_ends,
            points,
        })
    }

    /// Parse a composite glyph at absolute offset `abs`.
    fn parse_composite_glyph(
        &self,
        abs: usize,
        x_min: i16,
        y_min: i16,
        x_max: i16,
        y_max: i16,
        depth: u32,
    ) -> Option<GlyphOutline> {
        let d = &self.data;
        let mut off = abs + 10; // skip header

        let mut all_points: Vec<GlyphPoint> = Vec::new();
        let mut all_contour_ends: Vec<u16> = Vec::new();

        loop {
            if off + 4 > d.len() {
                return None;
            }

            let flags = read_u16_be(d, off);
            let component_glyph = read_u16_be(d, off + 2);
            off += 4;

            // Read translation arguments.
            let (arg1, arg2): (i32, i32);
            if flags & ARG_1_AND_2_ARE_WORDS != 0 {
                if off + 4 > d.len() {
                    return None;
                }
                if flags & ARGS_ARE_XY_VALUES != 0 {
                    arg1 = read_i16_be(d, off) as i32;
                    arg2 = read_i16_be(d, off + 2) as i32;
                } else {
                    arg1 = read_u16_be(d, off) as i32;
                    arg2 = read_u16_be(d, off + 2) as i32;
                }
                off += 4;
            } else {
                if off + 2 > d.len() {
                    return None;
                }
                if flags & ARGS_ARE_XY_VALUES != 0 {
                    arg1 = d[off] as i8 as i32;
                    arg2 = d[off + 1] as i8 as i32;
                } else {
                    arg1 = d[off] as i32;
                    arg2 = d[off + 1] as i32;
                }
                off += 2;
            }

            // Read optional scale / transform.
            // We store a 2x2 matrix as (a, b, c, d) in fixed-point F2Dot14.
            // Default is identity: a=1, b=0, c=0, d=1.
            let mut a: i32 = 0x4000; // 1.0 in F2Dot14
            let mut b: i32 = 0;
            let mut c: i32 = 0;
            let mut dd: i32 = 0x4000;

            if flags & WE_HAVE_A_SCALE != 0 {
                if off + 2 > d.len() {
                    return None;
                }
                a = read_i16_be(d, off) as i32;
                dd = a;
                off += 2;
            } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                if off + 4 > d.len() {
                    return None;
                }
                a = read_i16_be(d, off) as i32;
                dd = read_i16_be(d, off + 2) as i32;
                off += 4;
            } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
                if off + 8 > d.len() {
                    return None;
                }
                a = read_i16_be(d, off) as i32;
                b = read_i16_be(d, off + 2) as i32;
                c = read_i16_be(d, off + 4) as i32;
                dd = read_i16_be(d, off + 6) as i32;
                off += 8;
            }

            // Recursively resolve the component glyph.
            if let Some(component) = self.glyph_outline_depth(component_glyph, depth + 1) {
                // Apply transform + translation to each point.
                // F2Dot14 fixed-point: multiply then shift right by 14.
                let point_base = all_points.len() as u16;

                for pt in &component.points {
                    let px = pt.x as i32;
                    let py = pt.y as i32;

                    let tx = if flags & ARGS_ARE_XY_VALUES != 0 {
                        arg1
                    } else {
                        0
                    };
                    let ty = if flags & ARGS_ARE_XY_VALUES != 0 {
                        arg2
                    } else {
                        0
                    };

                    let new_x = ((a * px + b * py) >> 14) + tx;
                    let new_y = ((c * px + dd * py) >> 14) + ty;

                    all_points.push(GlyphPoint {
                        x: new_x as i16,
                        y: new_y as i16,
                        on_curve: pt.on_curve,
                    });
                }

                // Shift contour end indices.
                for &end in &component.contour_ends {
                    all_contour_ends.push(end + point_base);
                }
            }

            if flags & MORE_COMPONENTS == 0 {
                break;
            }
        }

        Some(GlyphOutline {
            num_contours: all_contour_ends.len() as i16,
            x_min,
            y_min,
            x_max,
            y_max,
            contour_ends: all_contour_ends,
            points: all_points,
        })
    }
}
