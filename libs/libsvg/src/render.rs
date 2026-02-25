// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! SVG-to-pixel renderer.
//!
//! Walks the parsed [`SvgDoc`] element tree, converts each shape to a
//! polyline (via [`path`] flattening), then rasterizes fills and strokes
//! using a non-zero / even-odd scanline fill algorithm.

use alloc::vec::Vec;
use alloc::vec;
use alloc::collections::BTreeMap;
use alloc::string::String;

use crate::types::{self, *};
use crate::path;
use crate::gradient::{self, Bounds};

// ── Public API ───────────────────────────────────────────────────────

/// Render a parsed [`SvgDoc`] into an ARGB8888 pixel buffer.
///
/// The SVG is scaled uniformly to fit `(out_w, out_h)` and centred
/// (letterboxed) when the aspect ratio differs.
pub fn render(
    doc: &SvgDoc,
    out: &mut [u32],
    out_w: u32,
    out_h: u32,
    bg_color: u32,
) {
    if out_w == 0 || out_h == 0 || out.len() < (out_w as usize) * (out_h as usize) {
        return;
    }

    // Fill background
    for px in out.iter_mut() { *px = bg_color; }

    let (vb_x, vb_y, vb_w, vb_h) = if let Some(vb) = doc.view_box {
        (vb[0], vb[1], vb[2], vb[3])
    } else {
        (0.0, 0.0, doc.width.max(1.0), doc.height.max(1.0))
    };

    if vb_w < 1.0 || vb_h < 1.0 { return; }

    // Compute uniform scale and offset (preserve aspect ratio)
    let scale_x = out_w as f32 / vb_w;
    let scale_y = out_h as f32 / vb_h;
    let scale = scale_x.min(scale_y);
    let off_x = (out_w as f32 - vb_w * scale) * 0.5;
    let off_y = (out_h as f32 - vb_h * scale) * 0.5;

    // Compose viewBox transform: translate by -vb_origin, scale, then offset for centering
    let vb_xform = Transform([
        scale, 0.0,
        0.0,   scale,
        -vb_x * scale + off_x,
        -vb_y * scale + off_y,
    ]);

    let mut ctx = RenderCtx {
        pixels: out,
        width: out_w,
        height: out_h,
        defs: &doc.defs,
        scale,
    };

    for el in &doc.elements {
        ctx.draw_element(el, &vb_xform, 1.0);
    }
}

// ── Render context ───────────────────────────────────────────────────

struct RenderCtx<'a> {
    pixels: &'a mut [u32],
    width:  u32,
    height: u32,
    defs:   &'a BTreeMap<String, Def>,
    /// Current effective SVG→pixel scale (used for stroke width and arc detail).
    scale:  f32,
}

impl<'a> RenderCtx<'a> {
    // ── Element dispatch ─────────────────────────────────────────────

    fn draw_element(&mut self, el: &Element, parent_xform: &Transform, parent_opacity: f32) {
        match el {
            Element::Group { elements, transform, opacity } => {
                let xform = transform.concat(parent_xform);
                let op = opacity * parent_opacity;
                for child in elements {
                    self.draw_element(child, &xform, op);
                }
            }

            Element::Path { cmds, style, transform } => {
                if !style.display { return; }
                let xform = transform.concat(parent_xform);
                let effective_op = style.opacity * parent_opacity;
                let polys = path::flatten(cmds, &xform, self.scale);
                let bounds = poly_bounds(&polys);
                self.fill_polys(&polys, style, bounds, effective_op);
                self.stroke_polys(&polys, style, bounds, effective_op);
            }

            Element::Rect { x, y, w, h, rx, ry, style, transform } => {
                if !style.display { return; }
                let xform = transform.concat(parent_xform);
                let effective_op = style.opacity * parent_opacity;
                let pts = path::rect_to_poly(*x, *y, *w, *h, *rx, *ry);
                let polys: Vec<Vec<(f32,f32)>> = pts.iter().map(|(px,py)| {
                    let (tx,ty) = xform.apply(*px, *py);
                    vec![(tx * self.scale, ty * self.scale)]
                }).collect::<Vec<_>>();
                // Build as a single polygon
                let scaled: Vec<(f32,f32)> = pts.iter().map(|(px,py)| {
                    let (tx,ty) = xform.apply(*px, *py);
                    (tx * self.scale, ty * self.scale)
                }).collect();
                let polys = vec![scaled];
                let bounds = poly_bounds(&polys);
                self.fill_polys(&polys, style, bounds, effective_op);
                self.stroke_polys(&polys, style, bounds, effective_op);
            }

            Element::Circle { cx, cy, r, style, transform } => {
                if !style.display { return; }
                let xform = transform.concat(parent_xform);
                let effective_op = style.opacity * parent_opacity;
                let pts = path::circle_to_poly(*cx, *cy, *r, &xform, self.scale);
                let polys = vec![pts];
                let bounds = poly_bounds(&polys);
                self.fill_polys(&polys, style, bounds, effective_op);
                self.stroke_polys(&polys, style, bounds, effective_op);
            }

            Element::Ellipse { cx, cy, rx, ry, style, transform } => {
                if !style.display { return; }
                let xform = transform.concat(parent_xform);
                let effective_op = style.opacity * parent_opacity;
                let pts = path::ellipse_to_poly(*cx, *cy, *rx, *ry, &xform, self.scale);
                let polys = vec![pts];
                let bounds = poly_bounds(&polys);
                self.fill_polys(&polys, style, bounds, effective_op);
                self.stroke_polys(&polys, style, bounds, effective_op);
            }

            Element::Line { x1, y1, x2, y2, style, transform } => {
                if !style.display { return; }
                let xform = transform.concat(parent_xform);
                let effective_op = style.opacity * parent_opacity;
                let (ax, ay) = { let p = xform.apply(*x1, *y1); (p.0 * self.scale, p.1 * self.scale) };
                let (bx, by) = { let p = xform.apply(*x2, *y2); (p.0 * self.scale, p.1 * self.scale) };
                let polys = vec![vec![(ax, ay), (bx, by)]];
                let bounds = poly_bounds(&polys);
                self.stroke_polys(&polys, style, bounds, effective_op);
            }

            Element::Polyline { pts, style, transform } => {
                if !style.display { return; }
                let xform = transform.concat(parent_xform);
                let effective_op = style.opacity * parent_opacity;
                let scaled: Vec<(f32,f32)> = pts.iter().map(|(px,py)| {
                    let (tx,ty) = xform.apply(*px, *py);
                    (tx * self.scale, ty * self.scale)
                }).collect();
                let polys = vec![scaled];
                let bounds = poly_bounds(&polys);
                self.stroke_polys(&polys, style, bounds, effective_op);
            }

            Element::Polygon { pts, style, transform } => {
                if !style.display { return; }
                let xform = transform.concat(parent_xform);
                let effective_op = style.opacity * parent_opacity;
                let mut scaled: Vec<(f32,f32)> = pts.iter().map(|(px,py)| {
                    let (tx,ty) = xform.apply(*px, *py);
                    (tx * self.scale, ty * self.scale)
                }).collect();
                // Close polygon
                if let (Some(first), Some(last)) = (scaled.first().copied(), scaled.last().copied()) {
                    if (first.0 - last.0).abs() > 0.5 || (first.1 - last.1).abs() > 0.5 {
                        scaled.push(first);
                    }
                }
                let polys = vec![scaled];
                let bounds = poly_bounds(&polys);
                self.fill_polys(&polys, style, bounds, effective_op);
                self.stroke_polys(&polys, style, bounds, effective_op);
            }
        }
    }

    // ── Fill rasterization ───────────────────────────────────────────

    /// Scanline-fill the given polylines using the style's fill paint.
    fn fill_polys(
        &mut self,
        polys: &[Vec<(f32, f32)>],
        style: &Style,
        bounds: Bounds,
        opacity: f32,
    ) {
        let total_alpha = clamp01(style.fill_opacity * opacity);
        if total_alpha < 1.0 / 255.0 { return; }
        if matches!(style.fill, Paint::None) { return; }

        let w = self.width as i32;
        let h = self.height as i32;

        // Determine scanline y range
        let y_min = (types::libm_floor(bounds.y) as i32).max(0);
        let y_max = types::libm_ceil(bounds.y + bounds.h) as i32;
        let y_max = y_max.min(h - 1);

        let fill_rule = style.fill_rule;

        for y in y_min..=y_max {
            let mut crossings: Vec<f32> = Vec::new();
            let yf = y as f32 + 0.5; // sample at pixel centre

            for poly in polys {
                if poly.len() < 2 { continue; }
                for i in 0..poly.len() - 1 {
                    let (x0, y0) = poly[i];
                    let (x1, y1) = poly[i + 1];
                    if (y0 <= yf && y1 > yf) || (y1 <= yf && y0 > yf) {
                        let x = x0 + (yf - y0) * (x1 - x0) / (y1 - y0);
                        crossings.push(x);
                    }
                }
            }

            if crossings.is_empty() { continue; }

            // Sort crossings
            crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));

            // Fill spans
            match fill_rule {
                FillRule::NonZero => {
                    // Treat odd number of crossings as fill spans
                    let mut i = 0;
                    while i + 1 < crossings.len() {
                        let x0 = (types::libm_ceil(crossings[i]) as i32).max(0);
                        let x1 = (types::libm_floor(crossings[i + 1]) as i32).min(w - 1);
                        if x0 <= x1 {
                            self.fill_span(y as u32, x0 as u32, x1 as u32,
                                           style, bounds, total_alpha);
                        }
                        i += 2;
                    }
                }
                FillRule::EvenOdd => {
                    let mut i = 0;
                    while i + 1 < crossings.len() {
                        let x0 = types::libm_ceil(crossings[i]) as i32;
                        let x1 = types::libm_floor(crossings[i + 1]) as i32;
                        let x0 = x0.max(0);
                        let x1 = x1.min(w - 1);
                        if x0 <= x1 {
                            self.fill_span(y as u32, x0 as u32, x1 as u32,
                                           style, bounds, total_alpha);
                        }
                        i += 2;
                    }
                }
            }
        }
    }

    /// Fill a horizontal pixel span with the style's fill paint.
    fn fill_span(
        &mut self,
        y: u32, x_start: u32, x_end: u32,
        style: &Style, bounds: Bounds, alpha: f32,
    ) {
        let row_off = (y * self.width) as usize;
        for x in x_start..=x_end {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let color = self.resolve_paint(&style.fill, px, py, bounds);
            let color = apply_alpha(color, alpha);
            let idx = row_off + x as usize;
            if idx < self.pixels.len() {
                self.pixels[idx] = alpha_blend(self.pixels[idx], color);
            }
        }
    }

    // ── Stroke rasterization ─────────────────────────────────────────

    /// Rasterize the stroke of each polyline by generating a thick polygon.
    fn stroke_polys(
        &mut self,
        polys: &[Vec<(f32, f32)>],
        style: &Style,
        bounds: Bounds,
        opacity: f32,
    ) {
        let sw = style.stroke_width * self.scale;
        if sw < 0.5 { return; }
        let total_alpha = clamp01(style.stroke_opacity * opacity);
        if total_alpha < 1.0 / 255.0 { return; }
        if matches!(style.stroke, Paint::None) { return; }

        for poly in polys {
            if poly.len() < 2 { continue; }
            // Expand stroke into a thick polygon via offset
            let thick = stroke_expand(poly, sw * 0.5, style.stroke_linecap);
            if thick.len() < 3 { continue; }

            let thick_polys = vec![thick];
            let stroke_bounds = poly_bounds(&thick_polys);

            // Fill the stroke polygon
            let stroke_style = Style {
                fill: style.stroke.clone(),
                fill_opacity: total_alpha,
                fill_rule: FillRule::NonZero,
                stroke: Paint::None,
                stroke_width: 0.0,
                stroke_opacity: 1.0,
                stroke_linecap: LineCap::Butt,
                stroke_linejoin: LineJoin::Miter,
                opacity: 1.0,
                display: true,
            };
            self.fill_polys(&thick_polys, &stroke_style, stroke_bounds, 1.0);
        }
    }

    // ── Paint resolution ─────────────────────────────────────────────

    fn resolve_paint(&self, paint: &Paint, x: f32, y: f32, bounds: Bounds) -> u32 {
        match paint {
            Paint::None => 0,
            Paint::Color(c) => *c,
            Paint::Url(id) => {
                match self.defs.get(id.as_str()) {
                    Some(Def::LinearGradient(g)) => gradient::eval_linear(g, x, y, bounds),
                    Some(Def::RadialGradient(g)) => gradient::eval_radial(g, x, y, bounds),
                    None => 0xFF808080,
                }
            }
        }
    }
}

// ── Stroke expansion ─────────────────────────────────────────────────

/// Build a closed polygon that encloses the stroke of `poly` at half-width `hw`.
fn stroke_expand(poly: &[(f32, f32)], hw: f32, cap: LineCap) -> Vec<(f32, f32)> {
    let n = poly.len();
    if n < 2 { return Vec::new(); }

    let mut left:  Vec<(f32,f32)> = Vec::with_capacity(n + 8);
    let mut right: Vec<(f32,f32)> = Vec::with_capacity(n + 8);

    for i in 0..n {
        let (dx, dy) = seg_normal(poly, i);
        let lx = poly[i].0 + dx * hw;
        let ly = poly[i].1 + dy * hw;
        let rx = poly[i].0 - dx * hw;
        let ry = poly[i].1 - dy * hw;
        left.push((lx, ly));
        right.push((rx, ry));
    }

    // Add end cap
    let end_cap = match cap {
        LineCap::Round  => round_cap(poly[n-1], poly[n-2], hw),
        LineCap::Square => square_cap(poly[n-1], poly[n-2], hw),
        LineCap::Butt   => Vec::new(),
    };
    // Add start cap
    let start_cap = match cap {
        LineCap::Round  => round_cap(poly[0], poly[1], hw),
        LineCap::Square => square_cap(poly[0], poly[1], hw),
        LineCap::Butt   => Vec::new(),
    };

    // Combine: left forward, end cap, right backward, start cap → closed polygon
    let total = left.len() + end_cap.len() + right.len() + start_cap.len() + 1;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&left);
    out.extend(end_cap);
    out.extend(right.iter().rev());
    out.extend(start_cap);
    if let Some(first) = out.first().copied() {
        out.push(first); // close polygon
    }
    out
}

/// Compute a unit normal vector at polyline point `i`.
fn seg_normal(poly: &[(f32, f32)], i: usize) -> (f32, f32) {
    let n = poly.len();
    let (dx, dy) = if i == 0 {
        dir(poly[0], poly[1])
    } else if i == n - 1 {
        dir(poly[n-2], poly[n-1])
    } else {
        // Average of two segment directions
        let (d1x, d1y) = dir(poly[i-1], poly[i]);
        let (d2x, d2y) = dir(poly[i], poly[i+1]);
        let ax = d1x + d2x;
        let ay = d1y + d2y;
        let len = types::libm_sqrt(ax*ax + ay*ay).max(1e-6);
        (ax / len, ay / len)
    };
    // Normal: perpendicular (rotate 90°)
    (-dy, dx)
}

fn dir(a: (f32,f32), b: (f32,f32)) -> (f32,f32) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = types::libm_sqrt(dx*dx + dy*dy).max(1e-6);
    (dx/len, dy/len)
}

fn round_cap(tip: (f32,f32), prev: (f32,f32), hw: f32) -> Vec<(f32,f32)> {
    use crate::types::libm_cos;
    use crate::types::libm_sin;
    use crate::types::libm_atan2;
    let angle = libm_atan2(tip.1 - prev.1, tip.0 - prev.0);
    let n = 8usize;
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let a = angle + core::f32::consts::PI * 0.5
              - core::f32::consts::PI * i as f32 / n as f32;
        pts.push((tip.0 + libm_cos(a) * hw, tip.1 + libm_sin(a) * hw));
    }
    pts
}

fn square_cap(tip: (f32,f32), prev: (f32,f32), hw: f32) -> Vec<(f32,f32)> {
    let (dx, dy) = dir(prev, tip);
    let nx = -dy * hw;
    let ny =  dx * hw;
    let ext_x = tip.0 + dx * hw;
    let ext_y = tip.1 + dy * hw;
    vec![
        (ext_x + nx, ext_y + ny),
        (ext_x - nx, ext_y - ny),
    ]
}

// ── Utility ───────────────────────────────────────────────────────────

/// Compute the axis-aligned bounding box of all polylines.
fn poly_bounds(polys: &[Vec<(f32, f32)>]) -> Bounds {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for poly in polys {
        for &(x, y) in poly {
            if x < min_x { min_x = x; }
            if y < min_y { min_y = y; }
            if x > max_x { max_x = x; }
            if y > max_y { max_y = y; }
        }
    }
    if min_x > max_x {
        Bounds { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }
    } else {
        Bounds { x: min_x, y: min_y, w: max_x - min_x, h: max_y - min_y }
    }
}

/// Blend ARGB8888 `src` over `dst` using the source alpha.
#[inline]
fn alpha_blend(dst: u32, src: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    if sa == 255 { return src; }
    if sa == 0   { return dst; }
    let inv = 255 - sa;
    let blend_ch = |s_shift: u32, d_shift: u32| -> u32 {
        let sc = (src >> s_shift) & 0xFF;
        let dc = (dst >> d_shift) & 0xFF;
        (sc * sa + dc * inv) / 255
    };
    0xFF000000
      | (blend_ch(16, 16) << 16)
      | (blend_ch( 8,  8) <<  8)
      |  blend_ch( 0,  0)
}

/// Scale the alpha channel of an ARGB8888 colour by `factor` ∈ [0, 1].
#[inline]
fn apply_alpha(color: u32, factor: f32) -> u32 {
    if factor >= 1.0 { return color; }
    let a = ((color >> 24) as f32 * factor) as u32;
    (a << 24) | (color & 0x00FFFFFF)
}
