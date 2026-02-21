//! Color blending, blur, and shadow cache computation.
//!
//! Performance-critical: all hot-path divisions replaced with `div255()`
//! bit trick (exact for 0..=65025, which covers all 255*255 products).

use alloc::vec;
use alloc::vec::Vec;

use super::layer::{ShadowCache, SHADOW_SPREAD};

/// Fast exact division by 255 using bit manipulation.
/// Exact for all x in 0..=65025 (255*255), which covers every possible
/// product in alpha blending (channel * alpha where both are 0-255).
#[inline(always)]
pub(crate) fn div255(x: u32) -> u32 {
    (x + 1 + (x >> 8)) >> 8
}

/// Alpha-blend src over dst (both ARGB8888). Division-free.
#[inline]
pub fn alpha_blend(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    if sa == 0 {
        return dst;
    }
    if sa >= 255 {
        return src;
    }
    let inv = 255 - sa;
    let r = div255(((src >> 16) & 0xFF) * sa + ((dst >> 16) & 0xFF) * inv);
    let g = div255(((src >> 8) & 0xFF) * sa + ((dst >> 8) & 0xFF) * inv);
    let b = div255((src & 0xFF) * sa + (dst & 0xFF) * inv);
    let a = sa + div255(((dst >> 24) & 0xFF) * inv);
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Blend a pure-black shadow (R=G=B=0) with alpha onto dst. Division-free.
/// Specialized fast path: skips source RGB extraction (always 0).
#[inline(always)]
pub(crate) fn shadow_blend(alpha: u32, dst: u32) -> u32 {
    if alpha == 0 { return dst; }
    let inv = 255 - alpha;
    let r = div255(((dst >> 16) & 0xFF) * inv);
    let g = div255(((dst >> 8) & 0xFF) * inv);
    let b = div255((dst & 0xFF) * inv);
    let a = alpha + div255(((dst >> 24) & 0xFF) * inv);
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Integer square root (for u32).
#[inline]
pub(crate) fn isqrt_u32(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Signed distance from point (px,py) to a rounded rectangle.
/// Returns negative values inside, positive outside, 0 on the edge.
/// Uses integer arithmetic (no floating point).
#[inline]
pub(crate) fn rounded_rect_sdf(px: i32, py: i32, rx: i32, ry: i32, rw: i32, rh: i32, r: i32) -> i32 {
    // Clamp r to half the smallest dimension
    let r = r.min(rw / 2).min(rh / 2).max(0);

    // Distance from point to the inner rect (shrunk by radius)
    let inner_x0 = rx + r;
    let inner_y0 = ry + r;
    let inner_x1 = rx + rw - r;
    let inner_y1 = ry + rh - r;

    // Compute distance to the inner rect
    let dx = if px < inner_x0 {
        inner_x0 - px
    } else if px >= inner_x1 {
        px - inner_x1 + 1
    } else {
        0
    };
    let dy = if py < inner_y0 {
        inner_y0 - py
    } else if py >= inner_y1 {
        py - inner_y1 + 1
    } else {
        0
    };

    if dx == 0 && dy == 0 {
        // Inside the inner rect — compute distance to edge (negative)
        let to_left = px - rx;
        let to_right = rx + rw - 1 - px;
        let to_top = py - ry;
        let to_bottom = ry + rh - 1 - py;
        let min_edge = to_left.min(to_right).min(to_top).min(to_bottom);
        -min_edge
    } else if dx > 0 && dy > 0 {
        // In a corner region — use Euclidean distance to corner
        isqrt_u32((dx * dx + dy * dy) as u32) as i32 - r
    } else {
        // Along an edge — simple distance
        let d = dx.max(dy);
        d - r
    }
}

/// Pre-compute shadow alpha values for a layer of given dimensions.
/// The result is a bitmap of (layer_w + 2*spread) x (layer_h + 2*spread) alpha values
/// representing the shadow intensity at each pixel, normalized to 0-255.
pub(crate) fn compute_shadow_cache(layer_w: u32, layer_h: u32) -> ShadowCache {
    let spread = SHADOW_SPREAD;
    let lw = layer_w as i32;
    let lh = layer_h as i32;
    let corner_r = 8i32; // must match the window corner radius
    let s = spread as u32;

    let cache_w = (lw + spread * 2) as u32;
    let cache_h = (lh + spread * 2) as u32;
    let mut alphas = vec![0u8; (cache_w * cache_h) as usize];

    // The virtual layer starts at (spread, spread) within the cache bitmap
    let lx = spread;
    let ly = spread;

    for row in 0..cache_h {
        let py = row as i32;
        for col in 0..cache_w {
            let px = col as i32;

            let dist = rounded_rect_sdf(px, py, lx, ly, lw, lh, corner_r);

            if dist >= spread {
                continue;
            }

            if dist <= 0 {
                // Inside the shadow shape: full alpha.
                // The window layer drawn on top will cover the interior;
                // this ensures no gap when the shadow is offset.
                alphas[(row * cache_w + col) as usize] = 255;
            } else {
                let t = dist as u32;
                let inv = s - t;
                // Normalized alpha: quadratic falloff scaled to 0-255
                let a = (255 * inv * inv) / (s * s);
                alphas[(row * cache_w + col) as usize] = a.min(255) as u8;
            }
        }
    }

    ShadowCache {
        alphas,
        cache_w,
        cache_h,
        layer_w,
        layer_h,
    }
}

/// Fast two-pass (H+V) box blur on a rectangular region of a pixel buffer.
/// `passes` iterations: 1=box, 2=triangle, 3~=gaussian.
/// `temp` is a reusable scratch buffer (avoids per-call heap allocation).
pub(crate) fn blur_back_buffer_region(
    bb: &mut [u32], fb_w: u32, fb_h: u32,
    rx: i32, ry: i32, rw: u32, rh: u32,
    radius: u32, passes: u32,
    temp: &mut Vec<u32>,
) {
    if rw == 0 || rh == 0 || radius == 0 || passes == 0 { return; }
    let x0 = rx.max(0) as usize;
    let y0 = ry.max(0) as usize;
    let x1 = ((rx + rw as i32) as usize).min(fb_w as usize);
    let y1 = ((ry + rh as i32) as usize).min(fb_h as usize);
    if x0 >= x1 || y0 >= y1 { return; }
    let w = x1 - x0;
    let h = y1 - y0;
    let stride = fb_w as usize;
    let r = radius as usize;
    let kernel = (2 * r + 1) as u32;

    // Fixed-point reciprocal: replaces `/ kernel` with `* recip >> 16`.
    // Max input: 255 * kernel. For kernel=17: 255*17=4335, 4335*recip=16,711,425 < u32::MAX.
    let recip = ((1u32 << 16) + kernel - 1) / kernel;

    let max_dim = w.max(h);
    if temp.len() < max_dim {
        temp.resize(max_dim, 0);
    }

    for _ in 0..passes {
        // Horizontal pass
        for row in y0..y1 {
            let row_off = row * stride;
            let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
            for i in 0..=(2 * r) {
                let sx = (x0 as i32 + i as i32 - r as i32).max(0).min(fb_w as i32 - 1) as usize;
                let px = bb[row_off + sx];
                sr += (px >> 16) & 0xFF;
                sg += (px >> 8) & 0xFF;
                sb += px & 0xFF;
            }
            for col in 0..w {
                let cx = x0 + col;
                temp[col] = 0xFF000000
                    | (((sr * recip) >> 16) << 16)
                    | (((sg * recip) >> 16) << 8)
                    | ((sb * recip) >> 16);
                let add_x = (cx as i32 + r as i32 + 1).min(fb_w as i32 - 1).max(0) as usize;
                let rem_x = (cx as i32 - r as i32).max(0).min(fb_w as i32 - 1) as usize;
                let add_px = bb[row_off + add_x];
                let rem_px = bb[row_off + rem_x];
                sr += ((add_px >> 16) & 0xFF) - ((rem_px >> 16) & 0xFF);
                sg += ((add_px >> 8) & 0xFF) - ((rem_px >> 8) & 0xFF);
                sb += (add_px & 0xFF) - (rem_px & 0xFF);
            }
            for col in 0..w {
                bb[row_off + x0 + col] = temp[col];
            }
        }
        // Vertical pass
        for col in x0..x1 {
            let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
            for i in 0..=(2 * r) {
                let sy = (y0 as i32 + i as i32 - r as i32).max(0).min(fb_h as i32 - 1) as usize;
                let px = bb[sy * stride + col];
                sr += (px >> 16) & 0xFF;
                sg += (px >> 8) & 0xFF;
                sb += px & 0xFF;
            }
            for row in 0..h {
                let cy = y0 + row;
                temp[row] = 0xFF000000
                    | (((sr * recip) >> 16) << 16)
                    | (((sg * recip) >> 16) << 8)
                    | ((sb * recip) >> 16);
                let add_y = (cy as i32 + r as i32 + 1).min(fb_h as i32 - 1).max(0) as usize;
                let rem_y = (cy as i32 - r as i32).max(0).min(fb_h as i32 - 1) as usize;
                let add_px = bb[add_y * stride + col];
                let rem_px = bb[rem_y * stride + col];
                sr += ((add_px >> 16) & 0xFF) - ((rem_px >> 16) & 0xFF);
                sg += ((add_px >> 8) & 0xFF) - ((rem_px >> 8) & 0xFF);
                sb += (add_px & 0xFF) - (rem_px & 0xFF);
            }
            for row in 0..h {
                bb[(y0 + row) * stride + col] = temp[row];
            }
        }
    }
}
