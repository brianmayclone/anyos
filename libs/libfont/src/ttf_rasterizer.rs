//! TrueType glyph rasterizer — converts outline data into 8-bit coverage bitmaps.
//!
//! Uses pure integer fixed-point arithmetic (8 fractional bits) for no_std
//! compatibility.  The rasterizer walks each contour, flattens quadratic Bezier
//! curves into line segments, and then computes per-pixel signed area coverage
//! with a scanline accumulator.

use alloc::vec;
use alloc::vec::Vec;
use super::ttf::{GlyphOutline, GlyphPoint};

/// Rasterized glyph as a row-major 8-bit coverage bitmap.
pub struct GlyphBitmap {
    pub width: u32,
    pub height: u32,
    pub x_offset: i32,
    pub y_offset: i32,
    pub advance: u32,
    pub coverage: Vec<u8>,
}

const FP_SHIFT: i32 = 8;
const FP_ONE: i32 = 1 << FP_SHIFT;
const FP_HALF: i32 = FP_ONE / 2;

#[inline(always)]
fn fp_mul(a: i32, b: i32) -> i32 { ((a as i64 * b as i64) >> FP_SHIFT) as i32 }

#[inline(always)]
fn fp_floor(v: i32) -> i32 { v >> FP_SHIFT }

#[inline(always)]
fn fp_ceil(v: i32) -> i32 { (v + FP_ONE - 1) >> FP_SHIFT }

#[derive(Clone)]
struct Edge { x0: i32, y0: i32, x1: i32, y1: i32, winding: i32 }

pub fn rasterize(outline: &GlyphOutline, size_px: u32, units_per_em: u16) -> Option<GlyphBitmap> {
    rasterize_impl(outline, size_px, units_per_em, 1)
}

pub fn rasterize_subpixel(outline: &GlyphOutline, size_px: u32, units_per_em: u16) -> Option<GlyphBitmap> {
    rasterize_impl(outline, size_px, units_per_em, 3)
}

fn rasterize_impl(outline: &GlyphOutline, size_px: u32, units_per_em: u16, h_scale: u32) -> Option<GlyphBitmap> {
    if outline.points.is_empty() || outline.contour_ends.is_empty() { return None; }
    if units_per_em == 0 || size_px == 0 { return None; }
    let scale_fp = ((size_px as i64 * FP_ONE as i64 * FP_ONE as i64) / units_per_em as i64) as i32;
    if scale_fp == 0 { return None; }
    let scale_x_fp = scale_fp * h_scale as i32;
    let scale_y_fp = scale_fp;
    let bbox_x_min = fp_mul(outline.x_min as i32, scale_x_fp);
    let bbox_y_min = fp_mul(outline.y_min as i32, scale_y_fp);
    let bbox_x_max = fp_mul(outline.x_max as i32, scale_x_fp);
    let bbox_y_max = fp_mul(outline.y_max as i32, scale_y_fp);
    let pad: i32 = 1;
    let px_x_min = fp_floor(bbox_x_min) - pad;
    let px_y_min = fp_floor(bbox_y_min) - pad;
    let px_x_max = fp_ceil(bbox_x_max) + pad;
    let px_y_max = fp_ceil(bbox_y_max) + pad;
    let width = (px_x_max - px_x_min) as u32;
    let height = (px_y_max - px_y_min) as u32;
    if width == 0 || height == 0 || width > 4096 || height > 4096 { return None; }
    let mut edges: Vec<Edge> = Vec::new();
    let mut contour_start: usize = 0;
    for &end_idx in &outline.contour_ends {
        let end = end_idx as usize;
        if end >= outline.points.len() || contour_start > end { contour_start = end + 1; continue; }
        let contour = &outline.points[contour_start..=end];
        if contour.len() >= 2 { flatten_contour(contour, &mut edges, scale_x_fp, scale_y_fp); }
        contour_start = end + 1;
    }
    if edges.is_empty() { return None; }
    let y_flip = px_y_max << FP_SHIFT;
    for edge in &mut edges {
        edge.y0 = y_flip - edge.y0;
        edge.y1 = y_flip - edge.y1;
        if edge.y0 > edge.y1 {
            core::mem::swap(&mut edge.y0, &mut edge.y1);
            core::mem::swap(&mut edge.x0, &mut edge.x1);
            edge.winding = -edge.winding;
        }
    }
    let offset_x = px_x_min;
    let coverage = rasterize_edges(&edges, width, height, offset_x);
    Some(GlyphBitmap { width, height, x_offset: px_x_min, y_offset: px_y_max, advance: 0, coverage })
}

fn flatten_contour(points: &[GlyphPoint], edges: &mut Vec<Edge>, scale_x: i32, scale_y: i32) {
    let n = points.len();
    if n < 2 { return; }
    let mut scaled: Vec<(i32, i32, bool)> = Vec::with_capacity(n);
    for p in points {
        scaled.push((fp_mul(p.x as i32, scale_x), fp_mul(p.y as i32, scale_y), p.on_curve));
    }
    let mut expanded: Vec<(i32, i32, bool)> = Vec::with_capacity(scaled.len() * 2);
    for i in 0..scaled.len() {
        let cur = scaled[i];
        expanded.push(cur);
        let next = scaled[(i + 1) % scaled.len()];
        if !cur.2 && !next.2 {
            expanded.push(((cur.0 + next.0) / 2, (cur.1 + next.1) / 2, true));
        }
    }
    let m = expanded.len();
    if m < 2 { return; }
    let mut i = 0;
    while i < m {
        let (x0, y0, on0) = expanded[i];
        let (x1, y1, on1) = expanded[(i + 1) % m];
        if on0 && on1 {
            add_edge(edges, x0, y0, x1, y1);
            i += 1;
        } else if on0 && !on1 {
            let (x2, y2, _) = expanded[(i + 2) % m];
            subdivide_bezier(x0, y0, x1, y1, x2, y2, edges, 0);
            i += 2;
        } else {
            add_edge(edges, x0, y0, x1, y1);
            i += 1;
        }
    }
}
fn add_edge(edges: &mut Vec<Edge>, x0: i32, y0: i32, x1: i32, y1: i32) {
    if y0 == y1 { return; }
    let (ex0, ey0, ex1, ey1, w) = if y0 < y1 {
        (x0, y0, x1, y1, 1i32)
    } else {
        (x1, y1, x0, y0, -1i32)
    };
    edges.push(Edge { x0: ex0, y0: ey0, x1: ex1, y1: ey1, winding: w });
}
fn subdivide_bezier(
    p0x: i32, p0y: i32, p1x: i32, p1y: i32, p2x: i32, p2y: i32,
    edges: &mut Vec<Edge>, depth: u32,
) {
    let mx = (p0x + p2x) / 2;
    let my = (p0y + p2y) / 2;
    let dx = p1x - mx;
    let dy = p1y - my;
    let dist_sq = dx as i64 * dx as i64 + dy as i64 * dy as i64;
    let quarter = (FP_HALF / 2) as i64;
    let threshold = quarter * quarter;
    if dist_sq <= threshold || depth >= 8 {
        add_edge(edges, p0x, p0y, p2x, p2y);
        return;
    }
    let q0x = (p0x + p1x) / 2;
    let q0y = (p0y + p1y) / 2;
    let q1x = (p1x + p2x) / 2;
    let q1y = (p1y + p2y) / 2;
    let rx = (q0x + q1x) / 2;
    let ry = (q0y + q1y) / 2;
    subdivide_bezier(p0x, p0y, q0x, q0y, rx, ry, edges, depth + 1);
    subdivide_bezier(rx, ry, q1x, q1y, p2x, p2y, edges, depth + 1);
}
fn rasterize_edges(edges: &[Edge], width: u32, height: u32, offset_x: i32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut accum: Vec<i32> = vec![0i32; w * h];
    for edge in edges {
        let ey0 = edge.y0;
        let ey1 = edge.y1;
        if ey0 >= ey1 { continue; }
        let row_start = fp_floor(ey0).max(0) as usize;
        let row_end = fp_ceil(ey1).min(height as i32) as usize;
        if row_start >= row_end { continue; }
        let dy = ey1 - ey0;
        let dx_total = (edge.x1 - edge.x0) as i64;
        for row in row_start..row_end {
            let y_top = (row as i32 * FP_ONE).max(ey0);
            let y_bot = ((row as i32 + 1) * FP_ONE).min(ey1);
            if y_top >= y_bot { continue; }
            let row_coverage = y_bot - y_top;
            let x_at_top = edge.x0 as i64 + dx_total * (y_top - ey0) as i64 / dy as i64;
            let x_at_bot = edge.x0 as i64 + dx_total * (y_bot - ey0) as i64 / dy as i64;
            let x_avg = ((x_at_top + x_at_bot) / 2) as i32;
            let col_fp = x_avg - (offset_x << FP_SHIFT);
            let col = fp_floor(col_fp);
            let frac_x = col_fp - (col << FP_SHIFT);
            let full = edge.winding * row_coverage;
            if col >= 0 && (col as usize) < w {
                let idx = row * w + col as usize;
                let c0 = fp_mul(full, FP_ONE - frac_x);
                accum[idx] += c0;
                if col as usize + 1 < w {
                    accum[idx + 1] += full - c0;
                }
            } else if col < 0 && w > 0 {
                accum[row * w] += full;
            }
        }
    }
    let mut coverage: Vec<u8> = vec![0u8; w * h];
    for row in 0..h {
        let base = row * w;
        let mut sum: i32 = 0;
        for col in 0..w {
            sum += accum[base + col];
            let v = if sum < 0 { -sum } else { sum };
            coverage[base + col] = v.min(255) as u8;
        }
    }
    coverage
}
