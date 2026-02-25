// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! SVG path data parser and flattener.
//!
//! Parses `d="..."` attribute strings into [`PathCmd`] lists, then flattens
//! bezier curves and arcs into line-segment polylines for rasterization.

use alloc::vec::Vec;
use alloc::vec;
use crate::types::{PathCmd, Transform, libm_sqrt, libm_sin, libm_cos, libm_atan2, libm_ceil, libm_acos};

// ── Path-D parser ────────────────────────────────────────────────────

/// Parse an SVG `d` attribute string into path commands.
///
/// Handles all standard SVG path commands: M, m, L, l, H, h, V, v,
/// C, c, S, s, Q, q, T, t, A, a, Z, z.
pub fn parse_path_d(d: &str) -> Vec<PathCmd> {
    let mut cmds = Vec::new();
    let mut iter = PathTokens::new(d);

    let mut cx = 0.0_f32;  // current point
    let mut cy = 0.0_f32;
    let mut sx = 0.0_f32;  // subpath start (for closepath)
    let mut sy = 0.0_f32;
    let mut last_ctrl_x = 0.0_f32; // last bezier control point (for smooth curves)
    let mut last_ctrl_y = 0.0_f32;
    let mut last_cmd = b'M';

    while let Some(cmd) = iter.next_cmd() {
        match cmd {
            b'M' | b'm' => {
                let rel = cmd == b'm';
                let mut first = true;
                while let Some((ax, ay)) = iter.next_pair() {
                    let (nx, ny) = if rel { (cx + ax, cy + ay) } else { (ax, ay) };
                    if first {
                        cmds.push(PathCmd::MoveTo(nx, ny));
                        sx = nx; sy = ny;
                        first = false;
                    } else {
                        // Subsequent coords after M are implicit LineTo
                        cmds.push(PathCmd::LineTo(nx, ny));
                    }
                    cx = nx; cy = ny;
                    last_ctrl_x = cx; last_ctrl_y = cy;
                    last_cmd = cmd;
                }
            }

            b'Z' | b'z' => {
                cmds.push(PathCmd::ClosePath);
                cx = sx; cy = sy;
                last_ctrl_x = cx; last_ctrl_y = cy;
                last_cmd = cmd;
            }

            b'L' | b'l' => {
                let rel = cmd == b'l';
                while let Some((ax, ay)) = iter.next_pair() {
                    let (nx, ny) = if rel { (cx + ax, cy + ay) } else { (ax, ay) };
                    cmds.push(PathCmd::LineTo(nx, ny));
                    cx = nx; cy = ny;
                    last_ctrl_x = cx; last_ctrl_y = cy;
                    last_cmd = cmd;
                }
            }

            b'H' | b'h' => {
                let rel = cmd == b'h';
                while let Some(ax) = iter.next_f32() {
                    let nx = if rel { cx + ax } else { ax };
                    cmds.push(PathCmd::LineTo(nx, cy));
                    cx = nx;
                    last_ctrl_x = cx; last_ctrl_y = cy;
                    last_cmd = cmd;
                }
            }

            b'V' | b'v' => {
                let rel = cmd == b'v';
                while let Some(ay) = iter.next_f32() {
                    let ny = if rel { cy + ay } else { ay };
                    cmds.push(PathCmd::LineTo(cx, ny));
                    cy = ny;
                    last_ctrl_x = cx; last_ctrl_y = cy;
                    last_cmd = cmd;
                }
            }

            b'C' | b'c' => {
                let rel = cmd == b'c';
                while let Some(vals) = iter.next_n(6) {
                    let (c1x, c1y, c2x, c2y, ex, ey) = if rel {
                        (cx+vals[0], cy+vals[1], cx+vals[2], cy+vals[3], cx+vals[4], cy+vals[5])
                    } else {
                        (vals[0], vals[1], vals[2], vals[3], vals[4], vals[5])
                    };
                    cmds.push(PathCmd::CubicTo(c1x, c1y, c2x, c2y, ex, ey));
                    last_ctrl_x = c2x; last_ctrl_y = c2y;
                    cx = ex; cy = ey;
                    last_cmd = cmd;
                }
            }

            b'S' | b's' => {
                // Smooth cubic: reflect previous control point
                let rel = cmd == b's';
                while let Some(vals) = iter.next_n(4) {
                    let c1x = 2.0 * cx - last_ctrl_x;
                    let c1y = 2.0 * cy - last_ctrl_y;
                    let (c2x, c2y, ex, ey) = if rel {
                        (cx+vals[0], cy+vals[1], cx+vals[2], cy+vals[3])
                    } else {
                        (vals[0], vals[1], vals[2], vals[3])
                    };
                    cmds.push(PathCmd::CubicTo(c1x, c1y, c2x, c2y, ex, ey));
                    last_ctrl_x = c2x; last_ctrl_y = c2y;
                    cx = ex; cy = ey;
                    last_cmd = cmd;
                }
            }

            b'Q' | b'q' => {
                let rel = cmd == b'q';
                while let Some(vals) = iter.next_n(4) {
                    let (qx, qy, ex, ey) = if rel {
                        (cx+vals[0], cy+vals[1], cx+vals[2], cy+vals[3])
                    } else {
                        (vals[0], vals[1], vals[2], vals[3])
                    };
                    cmds.push(PathCmd::QuadTo(qx, qy, ex, ey));
                    last_ctrl_x = qx; last_ctrl_y = qy;
                    cx = ex; cy = ey;
                    last_cmd = cmd;
                }
            }

            b'T' | b't' => {
                // Smooth quadratic
                let rel = cmd == b't';
                while let Some((ax, ay)) = iter.next_pair() {
                    let qx = 2.0 * cx - last_ctrl_x;
                    let qy = 2.0 * cy - last_ctrl_y;
                    let (ex, ey) = if rel { (cx + ax, cy + ay) } else { (ax, ay) };
                    cmds.push(PathCmd::QuadTo(qx, qy, ex, ey));
                    last_ctrl_x = qx; last_ctrl_y = qy;
                    cx = ex; cy = ey;
                    last_cmd = cmd;
                }
            }

            b'A' | b'a' => {
                let rel = cmd == b'a';
                while let Some(vals) = iter.next_n(7) {
                    let (ex, ey) = if rel {
                        (cx + vals[5], cy + vals[6])
                    } else {
                        (vals[5], vals[6])
                    };
                    cmds.push(PathCmd::ArcTo {
                        rx: vals[0], ry: vals[1],
                        x_rot: vals[2],
                        large: vals[3] != 0.0,
                        sweep: vals[4] != 0.0,
                        x: ex, y: ey,
                    });
                    last_ctrl_x = ex; last_ctrl_y = ey;
                    cx = ex; cy = ey;
                    last_cmd = cmd;
                }
            }

            _ => {
                // Unknown command — skip
                last_cmd = cmd;
            }
        }
    }
    cmds
}

// ── Path tokenizer ───────────────────────────────────────────────────

struct PathTokens<'a> {
    data: &'a [u8],
    pos: usize,
    pending_cmd: Option<u8>,
}

impl<'a> PathTokens<'a> {
    fn new(d: &'a str) -> Self {
        PathTokens { data: d.as_bytes(), pos: 0, pending_cmd: None }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.data.len()
            && matches!(self.data[self.pos], b' '|b'\t'|b'\n'|b'\r'|b',')
        {
            self.pos += 1;
        }
    }

    fn next_cmd(&mut self) -> Option<u8> {
        self.skip_ws();
        if let Some(c) = self.pending_cmd.take() {
            return Some(c);
        }
        if self.pos >= self.data.len() { return None; }
        let b = self.data[self.pos];
        if b.is_ascii_alphabetic() {
            self.pos += 1;
            Some(b)
        } else {
            None
        }
    }

    fn peek_f32(&self) -> bool {
        let mut p = self.pos;
        while p < self.data.len() && matches!(self.data[p], b' '|b'\t'|b'\n'|b'\r'|b',') {
            p += 1;
        }
        if p >= self.data.len() { return false; }
        let b = self.data[p];
        b.is_ascii_digit() || b == b'-' || b == b'+' || b == b'.'
    }

    fn next_f32(&mut self) -> Option<f32> {
        self.skip_ws();
        if self.pos >= self.data.len() { return None; }
        let b = self.data[self.pos];
        // If we hit an alphabetic, it's a new command
        if b.is_ascii_alphabetic() {
            self.pending_cmd = Some(b);
            self.pos += 1;
            return None;
        }
        // Parse number
        let start = self.pos;
        if matches!(b, b'-' | b'+') { self.pos += 1; }
        while self.pos < self.data.len() && (self.data[self.pos].is_ascii_digit() || self.data[self.pos] == b'.') {
            self.pos += 1;
        }
        if self.pos < self.data.len() && matches!(self.data[self.pos], b'e'|b'E') {
            self.pos += 1;
            if self.pos < self.data.len() && matches!(self.data[self.pos], b'+'|b'-') {
                self.pos += 1;
            }
            while self.pos < self.data.len() && self.data[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        if self.pos == start { return None; }
        core::str::from_utf8(&self.data[start..self.pos])
            .ok()
            .and_then(|s| s.parse().ok())
    }

    fn next_pair(&mut self) -> Option<(f32, f32)> {
        if !self.peek_f32() { return None; }
        let x = self.next_f32()?;
        let y = self.next_f32()?;
        Some((x, y))
    }

    fn next_n(&mut self, n: usize) -> Option<Vec<f32>> {
        if !self.peek_f32() { return None; }
        let mut vals = Vec::with_capacity(n);
        for _ in 0..n {
            vals.push(self.next_f32()?);
        }
        Some(vals)
    }
}

// ── Path flattening ──────────────────────────────────────────────────

/// Flatten a list of [`PathCmd`]s into connected polylines.
///
/// Each polyline is a `Vec<(f32, f32)>` in pixel space.  The transform
/// `xform` is applied to every point.  Arc and bezier commands are
/// recursively subdivided until flat enough (tolerance in pixels).
pub fn flatten(cmds: &[PathCmd], xform: &Transform, scale: f32) -> Vec<Vec<(f32, f32)>> {
    let tolerance = 0.25 / scale; // half-pixel tolerance before scaling
    let mut polys: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    let mut cx = 0.0_f32;
    let mut cy = 0.0_f32;
    let mut start_x = 0.0_f32;
    let mut start_y = 0.0_f32;

    for cmd in cmds {
        match *cmd {
            PathCmd::MoveTo(x, y) => {
                if current.len() > 1 {
                    polys.push(core::mem::take(&mut current));
                } else {
                    current.clear();
                }
                cx = x; cy = y;
                start_x = x; start_y = y;
                let p = apply_and_scale(xform, x, y, scale);
                current.push(p);
            }

            PathCmd::LineTo(x, y) => {
                cx = x; cy = y;
                let p = apply_and_scale(xform, x, y, scale);
                current.push(p);
            }

            PathCmd::CubicTo(c1x, c1y, c2x, c2y, ex, ey) => {
                let p0 = (cx, cy);
                let p1 = (c1x, c1y);
                let p2 = (c2x, c2y);
                let p3 = (ex, ey);
                flatten_cubic(p0, p1, p2, p3, &mut current, xform, scale, tolerance, 0);
                cx = ex; cy = ey;
            }

            PathCmd::QuadTo(qx, qy, ex, ey) => {
                // Elevate quadratic to cubic
                let c1x = cx + (qx - cx) * (2.0 / 3.0);
                let c1y = cy + (qy - cy) * (2.0 / 3.0);
                let c2x = ex + (qx - ex) * (2.0 / 3.0);
                let c2y = ey + (qy - ey) * (2.0 / 3.0);
                let p0 = (cx, cy);
                let p3 = (ex, ey);
                flatten_cubic(p0, (c1x,c1y), (c2x,c2y), p3, &mut current, xform, scale, tolerance, 0);
                cx = ex; cy = ey;
            }

            PathCmd::ArcTo { rx, ry, x_rot, large, sweep, x, y } => {
                flatten_arc(cx, cy, rx, ry, x_rot, large, sweep, x, y,
                            &mut current, xform, scale);
                cx = x; cy = y;
            }

            PathCmd::ClosePath => {
                if (cx - start_x).abs() > 0.01 || (cy - start_y).abs() > 0.01 {
                    let p = apply_and_scale(xform, start_x, start_y, scale);
                    current.push(p);
                }
                if current.len() > 1 {
                    let closed = current.clone();
                    polys.push(closed);
                }
                current.clear();
                cx = start_x; cy = start_y;
                let p = apply_and_scale(xform, cx, cy, scale);
                current.push(p);
            }
        }
    }

    if current.len() > 1 {
        polys.push(current);
    }

    polys
}

#[inline]
fn apply_and_scale(xform: &Transform, x: f32, y: f32, scale: f32) -> (f32, f32) {
    let (tx, ty) = xform.apply(x, y);
    (tx * scale, ty * scale)
}

// ── Cubic bezier flattening ──────────────────────────────────────────

const MAX_BEZIER_DEPTH: u32 = 10;

fn flatten_cubic(
    p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32),
    out: &mut Vec<(f32, f32)>,
    xform: &Transform, scale: f32, tolerance: f32, depth: u32,
) {
    if depth >= MAX_BEZIER_DEPTH || is_flat_cubic(p0, p1, p2, p3, tolerance) {
        out.push(apply_and_scale(xform, p3.0, p3.1, scale));
        return;
    }
    // de Casteljau subdivision at t=0.5
    let m01 = mid(p0, p1);
    let m12 = mid(p1, p2);
    let m23 = mid(p2, p3);
    let m012 = mid(m01, m12);
    let m123 = mid(m12, m23);
    let m0123 = mid(m012, m123);
    flatten_cubic(p0, m01, m012, m0123, out, xform, scale, tolerance, depth + 1);
    flatten_cubic(m0123, m123, m23, p3, out, xform, scale, tolerance, depth + 1);
}

fn is_flat_cubic(p0: (f32,f32), p1: (f32,f32), p2: (f32,f32), p3: (f32,f32), tol: f32) -> bool {
    // Test if control points are within `tol` of the chord
    let dx = p3.0 - p0.0;
    let dy = p3.1 - p0.1;
    let d_sq = dx*dx + dy*dy;
    if d_sq < 1e-6 {
        let d1 = (p1.0-p0.0).abs().max((p1.1-p0.1).abs());
        let d2 = (p2.0-p0.0).abs().max((p2.1-p0.1).abs());
        return d1 < tol && d2 < tol;
    }
    let inv_d = 1.0 / d_sq.max(1e-10);
    let cross1 = ((p1.0-p0.0)*dy - (p1.1-p0.1)*dx) * inv_d;
    let cross2 = ((p2.0-p0.0)*dy - (p2.1-p0.1)*dx) * inv_d;
    (cross1*cross1 + cross2*cross2) < tol * tol * inv_d
}

#[inline] fn mid(a: (f32,f32), b: (f32,f32)) -> (f32,f32) {
    ((a.0+b.0)*0.5, (a.1+b.1)*0.5)
}

// ── Elliptical arc flattening ─────────────────────────────────────────

fn flatten_arc(
    x1: f32, y1: f32, rx: f32, ry: f32, x_rot_deg: f32,
    large_arc: bool, sweep: bool,
    x2: f32, y2: f32,
    out: &mut Vec<(f32, f32)>,
    xform: &Transform, scale: f32,
) {
    if (x1 - x2).abs() < 1e-6 && (y1 - y2).abs() < 1e-6 { return; }
    if rx.abs() < 1e-6 || ry.abs() < 1e-6 {
        out.push(apply_and_scale(xform, x2, y2, scale));
        return;
    }

    let phi = x_rot_deg.to_radians();
    let (sin_phi, cos_phi) = (libm_sin(phi), libm_cos(phi));

    // Midpoint method (SVG spec section F.6.5)
    let dx2 = (x1 - x2) * 0.5;
    let dy2 = (y1 - y2) * 0.5;
    let x1p =  cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;

    let mut rx = rx.abs();
    let mut ry = ry.abs();
    let x1p2 = x1p * x1p;
    let y1p2 = y1p * y1p;
    let rx2 = rx * rx;
    let ry2 = ry * ry;

    // Scale radii if too small
    let lambda = x1p2 / rx2 + y1p2 / ry2;
    if lambda > 1.0 {
        let s = libm_sqrt(lambda);
        rx *= s; ry *= s;
        let rx2 = rx * rx;
        let ry2 = ry * ry;
        let _ = (rx2, ry2);
    }
    let rx2 = rx * rx;
    let ry2 = ry * ry;

    let sign = if large_arc == sweep { -1.0_f32 } else { 1.0_f32 };
    let num = rx2 * ry2 - rx2 * y1p2 - ry2 * x1p2;
    let den = rx2 * y1p2 + ry2 * x1p2;
    let sq = if den < 1e-10 { 0.0 } else { libm_sqrt((num / den).max(0.0)) };
    let cxp =  sign * sq * rx * y1p / ry;
    let cyp = -sign * sq * ry * x1p / rx;

    let cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) * 0.5;
    let cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) * 0.5;

    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;

    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut d_theta = angle(ux, uy, vx, vy);

    if !sweep && d_theta > 0.0 {
        d_theta -= 2.0 * core::f32::consts::PI;
    } else if sweep && d_theta < 0.0 {
        d_theta += 2.0 * core::f32::consts::PI;
    }

    // Approximate arc with line segments (about 4 per quarter circle)
    let n_segs = (libm_ceil(d_theta.abs() / (core::f32::consts::PI * 0.5)) as u32).max(1).min(64);
    let dt = d_theta / n_segs as f32;
    let mut theta = theta1;
    for _ in 0..n_segs {
        theta += dt;
        let px = cos_phi * rx * libm_cos(theta) - sin_phi * ry * libm_sin(theta) + cx;
        let py = sin_phi * rx * libm_cos(theta) + cos_phi * ry * libm_sin(theta) + cy;
        out.push(apply_and_scale(xform, px, py, scale));
    }
}

fn angle(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
    let dot = ux * vx + uy * vy;
    let len = libm_sqrt((ux*ux + uy*uy) * (vx*vx + vy*vy));
    let clamped = { let v = dot / len.max(1e-10); if v < -1.0 { -1.0 } else if v > 1.0 { 1.0 } else { v } };
    let a = libm_acos(clamped);
    if ux * vy - uy * vx < 0.0 { -a } else { a }
}

// ── Rect → polygon ───────────────────────────────────────────────────

/// Convert a rounded rectangle to a flat polyline.
pub fn rect_to_poly(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Vec<(f32, f32)> {
    let rx = rx.min(w * 0.5);
    let ry = ry.min(h * 0.5);
    if rx < 0.5 && ry < 0.5 {
        return vec![
            (x, y), (x+w, y), (x+w, y+h), (x, y+h), (x, y),
        ];
    }
    // Approximate rounded corners with bezier segments
    let mut d = alloc::string::String::new();
    use alloc::format;
    d.push_str(&format!(
        "M {},{} H {} Q {},{} {},{} V {} Q {},{} {},{} H {} Q {},{} {},{} V {} Q {},{} {},{} Z",
        x+rx, y,
        x+w-rx, x+w, y, x+w, y+ry,
        y+h-ry, x+w, y+h, x+w-rx, y+h,
        x+rx, x, y+h, x, y+h-ry,
        y+ry, x, y, x+rx, y,
    ));
    let cmds = parse_path_d(&d);
    let t = Transform::identity();
    let polys = flatten(&cmds, &t, 1.0);
    polys.into_iter().next().unwrap_or_default()
}

/// Convert a circle to a flat polygon.
pub fn circle_to_poly(cx: f32, cy: f32, r: f32, xform: &Transform, scale: f32) -> Vec<(f32, f32)> {
    ellipse_to_poly(cx, cy, r, r, xform, scale)
}

/// Convert an ellipse to a flat polygon.
pub fn ellipse_to_poly(cx: f32, cy: f32, rx: f32, ry: f32, xform: &Transform, scale: f32) -> Vec<(f32, f32)> {
    let n = (libm_ceil(rx.max(ry) * scale * 2.0 * core::f32::consts::PI) as usize)
        .max(8)
        .min(256);
    let mut pts = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let angle = 2.0 * core::f32::consts::PI * i as f32 / n as f32;
        let x = cx + rx * libm_cos(angle);
        let y = cy + ry * libm_sin(angle);
        pts.push(apply_and_scale(xform, x, y, scale));
    }
    pts
}
