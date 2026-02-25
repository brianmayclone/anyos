// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Gradient colour computation.
//!
//! Evaluates linear and radial gradients at a given (x,y) position,
//! returning ARGB8888.

use crate::types::{
    LinearGradient, RadialGradient, GradientUnits, Spread, GradientStop,
    libm_sqrt, libm_floor, clamp01,
};

/// Bounds of a shape (in pixel space) — needed for objectBoundingBox gradients.
#[derive(Clone, Copy)]
pub struct Bounds {
    pub x: f32, pub y: f32,
    pub w: f32, pub h: f32,
}

// ── Linear gradient ──────────────────────────────────────────────────

/// Evaluate a linear gradient at pixel position `(px, py)`.
pub fn eval_linear(grad: &LinearGradient, px: f32, py: f32, bounds: Bounds) -> u32 {
    let (x1, y1, x2, y2) = if grad.units == GradientUnits::ObjectBoundingBox {
        (
            bounds.x + grad.x1 * bounds.w,
            bounds.y + grad.y1 * bounds.h,
            bounds.x + grad.x2 * bounds.w,
            bounds.y + grad.y2 * bounds.h,
        )
    } else {
        let (gx1, gy1) = grad.xform.apply(grad.x1, grad.y1);
        let (gx2, gy2) = grad.xform.apply(grad.x2, grad.y2);
        (gx1, gy1, gx2, gy2)
    };

    let dx = x2 - x1;
    let dy = y2 - y1;
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-10 {
        0.0
    } else {
        ((px - x1) * dx + (py - y1) * dy) / len2
    };

    let t = apply_spread(t, grad.spread);
    interpolate_stops(&grad.stops, t)
}

// ── Radial gradient ──────────────────────────────────────────────────

/// Evaluate a radial gradient at pixel position `(px, py)`.
pub fn eval_radial(grad: &RadialGradient, px: f32, py: f32, bounds: Bounds) -> u32 {
    let (cx, cy, r, fx, fy) = if grad.units == GradientUnits::ObjectBoundingBox {
        (
            bounds.x + grad.cx * bounds.w,
            bounds.y + grad.cy * bounds.h,
            (grad.r * (bounds.w + bounds.h) * 0.5),
            bounds.x + grad.fx * bounds.w,
            bounds.y + grad.fy * bounds.h,
        )
    } else {
        let (gcx, gcy) = grad.xform.apply(grad.cx, grad.cy);
        let (gfx, gfy) = grad.xform.apply(grad.fx, grad.fy);
        // scale r by the transform scale
        let sx = libm_sqrt(grad.xform.0[0]*grad.xform.0[0] + grad.xform.0[1]*grad.xform.0[1]);
        (gcx, gcy, grad.r * sx, gfx, gfy)
    };

    if r < 1e-6 {
        return grad.stops.last().map(|s| s.1).unwrap_or(0);
    }

    // Distance from focal point to pixel
    let dx = px - fx;
    let dy = py - fy;
    let dfx = fx - cx;
    let dfy = fy - cy;

    // Solve: |P - F + t*(F - C)| = t*r  (SVG spec focal-point formula)
    // Simplified when fx==cx, fy==cy: t = distance/r
    let t = if (dfx * dfx + dfy * dfy) < 1e-6 {
        libm_sqrt(dx * dx + dy * dy) / r
    } else {
        // General focal-point case
        let a = (dx - dfx) * (dx - dfx) + (dy - dfy) * (dy - dfy)
              - r * r * ((dfx * dfx + dfy * dfy) / (r * r));
        let b2 = dx * (dx - dfx) + dy * (dy - dfy);
        let disc = b2 * b2 - a * (dx * dx + dy * dy - r * r);
        if disc < 0.0 { 1.0 }
        else {
            let sq = libm_sqrt(disc);
            let t1 = (b2 + sq) / a;
            let t2 = (b2 - sq) / a;
            let t = if t1 > 0.0 { t1 } else { t2 };
            if t < 0.0 { 0.0 } else { t }
        }
    };

    let t = apply_spread(t, grad.spread);
    interpolate_stops(&grad.stops, t)
}

// ── Stop interpolation ───────────────────────────────────────────────

/// Interpolate between gradient stops at parameter `t` ∈ [0, 1].
pub fn interpolate_stops(stops: &[GradientStop], t: f32) -> u32 {
    if stops.is_empty() { return 0xFF000000; }
    if stops.len() == 1 { return stops[0].1; }

    // Clamp to first/last stop
    if t <= stops[0].0 { return stops[0].1; }
    let last = stops[stops.len() - 1];
    if t >= last.0 { return last.1; }

    // Find surrounding stops
    let mut lo = 0;
    for i in 1..stops.len() {
        if stops[i].0 >= t {
            let hi = i;
            lo = hi - 1;
            let t0 = stops[lo].0;
            let t1 = stops[hi].0;
            let dt = t1 - t0;
            let f = if dt < 1e-10 { 0.0 } else { (t - t0) / dt };
            return lerp_argb(stops[lo].1, stops[hi].1, f);
        }
    }
    last.1
}

fn apply_spread(mut t: f32, spread: Spread) -> f32 {
    match spread {
        Spread::Pad => clamp01(t),
        Spread::Repeat => {
            t = t - libm_floor(t);
            if t < 0.0 { t += 1.0; }
            t
        }
        Spread::Reflect => {
            t = t.abs();
            let it = t as u32;
            let frac = t - it as f32;
            if it % 2 == 0 { frac } else { 1.0 - frac }
        }
    }
}

/// Linear interpolation between two ARGB8888 colours.
#[inline]
pub fn lerp_argb(a: u32, b: u32, t: f32) -> u32 {
    let t256 = (t * 256.0) as u32;
    let inv = 256 - t256;
    let blend_ch = |shift: u32| -> u32 {
        let ca = (a >> shift) & 0xFF;
        let cb = (b >> shift) & 0xFF;
        ((ca * inv + cb * t256) >> 8) & 0xFF
    };
    (blend_ch(24) << 24) | (blend_ch(16) << 16) | (blend_ch(8) << 8) | blend_ch(0)
}
