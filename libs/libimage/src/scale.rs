// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Bilinear image scaling with multiple fit modes.
//!
//! All arithmetic uses 16.16 fixed-point integers (no floating point).

/// Stretch source to fill destination, ignoring aspect ratio.
pub const MODE_SCALE: u32 = 0;

/// Fit source within destination maintaining aspect ratio; letterbox with
/// transparent black (0x00000000).
pub const MODE_CONTAIN: u32 = 1;

/// Fill destination maintaining aspect ratio; crop any excess.
pub const MODE_COVER: u32 = 2;

/// 16.16 fixed-point shift.
const FP_SHIFT: u32 = 16;
const FP_ONE: u32 = 1 << FP_SHIFT;

/// Scale an image from `src` to `dst` using bilinear interpolation.
///
/// - `src` / `src_w` / `src_h`: source ARGB8888 pixel buffer and dimensions.
/// - `dst` / `dst_w` / `dst_h`: destination ARGB8888 pixel buffer and dimensions.
/// - `mode`: one of `MODE_SCALE`, `MODE_CONTAIN`, or `MODE_COVER`.
///
/// Returns 0 on success, -1 on error (null pointer, zero dimension, or
/// invalid mode).
pub fn scale_image(
    src: *const u32,
    src_w: u32,
    src_h: u32,
    dst: *mut u32,
    dst_w: u32,
    dst_h: u32,
    mode: u32,
) -> i32 {
    if src.is_null() || dst.is_null() {
        return -1;
    }
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return -1;
    }
    if mode > MODE_COVER {
        return -1;
    }

    let src_slice =
        unsafe { core::slice::from_raw_parts(src, (src_w as usize) * (src_h as usize)) };
    let dst_slice =
        unsafe { core::slice::from_raw_parts_mut(dst, (dst_w as usize) * (dst_h as usize)) };

    // Determine the destination viewport and the source sampling window.
    //
    //  vp_x, vp_y, vp_w, vp_h  -- rectangle within dst to write pixels into
    //  crop_x, crop_y, crop_w, crop_h -- rectangle within src to sample from
    //
    // MODE_SCALE:   viewport = full dst,  crop = full src
    // MODE_CONTAIN: viewport = centred fit rect, crop = full src, rest = transparent
    // MODE_COVER:   viewport = full dst,  crop = centred sub-rect of src

    let (vp_x, vp_y, vp_w, vp_h, crop_x, crop_y, crop_w, crop_h) = match mode {
        MODE_SCALE => (0u32, 0u32, dst_w, dst_h, 0u32, 0u32, src_w, src_h),
        MODE_CONTAIN => {
            let (vx, vy, vw, vh) = contain_viewport(src_w, src_h, dst_w, dst_h);
            // Clear entire destination to transparent black.
            for p in dst_slice.iter_mut() {
                *p = 0x00000000;
            }
            (vx, vy, vw, vh, 0, 0, src_w, src_h)
        }
        MODE_COVER => {
            let (cx, cy, cw, ch) = cover_crop(src_w, src_h, dst_w, dst_h);
            (0, 0, dst_w, dst_h, cx, cy, cw, ch)
        }
        _ => return -1,
    };

    if vp_w == 0 || vp_h == 0 || crop_w == 0 || crop_h == 0 {
        return 0;
    }

    // Fixed-point step: how far to advance in *crop* space per dest pixel.
    let step_x = fp_div(crop_w << FP_SHIFT, vp_w << FP_SHIFT);
    let step_y = fp_div(crop_h << FP_SHIFT, vp_h << FP_SHIFT);

    // Clamp limits in source coordinates.
    let max_sx = ((src_w - 1) as u32) << FP_SHIFT;
    let max_sy = ((src_h - 1) as u32) << FP_SHIFT;

    // Starting position in source space (crop origin + half-step for pixel
    // centre sampling).
    let origin_x = (crop_x << FP_SHIFT) + (step_x >> 1);
    let origin_y = (crop_y << FP_SHIFT) + (step_y >> 1);

    let src_stride = src_w as usize;
    let dst_stride = dst_w as usize;

    let mut sy_fp = origin_y;
    for dy in 0..vp_h {
        let csy = clamp(sy_fp, max_sy);
        let sy0 = (csy >> FP_SHIFT) as usize;
        let sy1 = if sy0 + 1 < src_h as usize { sy0 + 1 } else { sy0 };
        let fy = csy & (FP_ONE - 1);

        let dst_row = ((vp_y + dy) as usize) * dst_stride + (vp_x as usize);

        let mut sx_fp = origin_x;
        for dx in 0..vp_w {
            let csx = clamp(sx_fp, max_sx);
            let sx0 = (csx >> FP_SHIFT) as usize;
            let sx1 = if sx0 + 1 < src_w as usize { sx0 + 1 } else { sx0 };
            let fx = csx & (FP_ONE - 1);

            let c00 = src_slice[sy0 * src_stride + sx0];
            let c10 = src_slice[sy0 * src_stride + sx1];
            let c01 = src_slice[sy1 * src_stride + sx0];
            let c11 = src_slice[sy1 * src_stride + sx1];

            dst_slice[dst_row + dx as usize] = bilinear(c00, c10, c01, c11, fx, fy);

            sx_fp = sx_fp.wrapping_add(step_x);
        }
        sy_fp = sy_fp.wrapping_add(step_y);
    }

    0
}

// ── Helpers ───────────────────────────────────────────────

/// Clamp a 16.16 value to [0, max].
#[inline(always)]
fn clamp(v: u32, max: u32) -> u32 {
    if v > max { max } else { v }
}

/// Bilinear blend of four ARGB8888 pixels.
///
/// `fx` and `fy` are the 16.16 fractional parts (0 .. FP_ONE-1).
#[inline(always)]
fn bilinear(c00: u32, c10: u32, c01: u32, c11: u32, fx: u32, fy: u32) -> u32 {
    let inv_fx = FP_ONE - fx;
    let inv_fy = FP_ONE - fy;

    // Reduce to 8.8 before multiplying so the products fit in 32 bits.
    let w00 = (inv_fx >> 8) * (inv_fy >> 8);
    let w10 = (fx >> 8) * (inv_fy >> 8);
    let w01 = (inv_fx >> 8) * (fy >> 8);
    let w11 = (fx >> 8) * (fy >> 8);
    let w_sum = w00 + w10 + w01 + w11;

    if w_sum == 0 {
        return c00;
    }

    let blend = |shift: u32| -> u32 {
        let v00 = (c00 >> shift) & 0xFF;
        let v10 = (c10 >> shift) & 0xFF;
        let v01 = (c01 >> shift) & 0xFF;
        let v11 = (c11 >> shift) & 0xFF;
        let sum = v00 * w00 + v10 * w10 + v01 * w01 + v11 * w11;
        (sum + (w_sum >> 1)) / w_sum
    };

    (blend(24) << 24) | (blend(16) << 16) | (blend(8) << 8) | blend(0)
}

/// Fixed-point division: `a / b` (both 16.16) -> 16.16 result.
#[inline(always)]
fn fp_div(a: u32, b: u32) -> u32 {
    if b == 0 {
        return 0;
    }
    (((a as u64) << FP_SHIFT) / (b as u64)) as u32
}

/// Compute the viewport rectangle for CONTAIN mode.
///
/// Returns `(x, y, w, h)` in destination coordinates -- the largest rect that
/// fits within `(dst_w, dst_h)` while preserving the source aspect ratio.
fn contain_viewport(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, u32, u32) {
    // scale = min(dst_w / src_w, dst_h / src_h)
    let scale_x = fp_div(dst_w << FP_SHIFT, src_w << FP_SHIFT);
    let scale_y = fp_div(dst_h << FP_SHIFT, src_h << FP_SHIFT);
    let scale = if scale_x < scale_y { scale_x } else { scale_y };

    let vw = (((src_w as u64) * (scale as u64)) >> FP_SHIFT) as u32;
    let vh = (((src_h as u64) * (scale as u64)) >> FP_SHIFT) as u32;
    let vw = if vw > dst_w { dst_w } else { vw };
    let vh = if vh > dst_h { dst_h } else { vh };

    let vx = (dst_w - vw) / 2;
    let vy = (dst_h - vh) / 2;
    (vx, vy, vw, vh)
}

/// Compute the source crop rectangle for COVER mode.
///
/// Returns `(x, y, w, h)` in source coordinates -- the largest centred
/// sub-rect of the source that has the same aspect ratio as the destination.
fn cover_crop(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, u32, u32) {
    // scale = max(dst_w / src_w, dst_h / src_h)
    // crop_w = dst_w / scale,  crop_h = dst_h / scale
    //
    // Equivalent: if src_w/src_h > dst_w/dst_h (source wider), crop width.
    //             Cross-multiply to avoid division:
    //               src_w * dst_h > dst_w * src_h  =>  source is wider

    let lhs = (src_w as u64) * (dst_h as u64);
    let rhs = (dst_w as u64) * (src_h as u64);

    if lhs > rhs {
        // Source is wider -- crop horizontally.
        // crop_h = src_h, crop_w = src_h * dst_w / dst_h
        let cw = ((src_h as u64) * (dst_w as u64) / (dst_h as u64)) as u32;
        let cw = if cw > src_w { src_w } else { cw };
        let cx = (src_w - cw) / 2;
        (cx, 0, cw, src_h)
    } else {
        // Source is taller (or exact match) -- crop vertically.
        // crop_w = src_w, crop_h = src_w * dst_h / dst_w
        let ch = ((src_w as u64) * (dst_h as u64) / (dst_w as u64)) as u32;
        let ch = if ch > src_h { src_h } else { ch };
        let cy = (src_h - ch) / 2;
        (0, cy, src_w, ch)
    }
}
