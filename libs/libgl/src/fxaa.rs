//! FXAA (Fast Approximate Anti-Aliasing) post-process pass.
//!
//! Operates on the resolved ARGB framebuffer in-place. Detects edges by
//! comparing luminance between neighboring pixels and blends along the
//! detected edge direction.
//!
//! Based on FXAA 1.0 (Timothy Lottes / Nvidia):
//! 1. Compute luminance for each pixel + 4 neighbors (N/S/E/W)
//! 2. Detect edges via contrast threshold
//! 3. Determine edge direction (horizontal or vertical)
//! 4. Blend edge pixels with sub-pixel offset along edge normal

extern crate alloc;

/// Run FXAA on the framebuffer color buffer in-place.
///
/// `color` is ARGB u32 pixels, row-major, top-left origin.
pub fn apply(color: &mut [u32], width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    if w < 3 || h < 3 { return; }

    // We need a copy since we read neighbors while writing.
    // Use the same buffer for output — copy input first.
    let src: alloc::vec::Vec<u32> = color.to_vec();

    let edge_threshold: f32 = 0.0625;     // Minimum contrast to detect edge
    let edge_threshold_min: f32 = 0.0312; // Skip very dark areas

    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let idx = y * w + x;

            // Sample center + 4 neighbors
            let lum_c = luma(src[idx]);
            let lum_n = luma(src[idx - w]);
            let lum_s = luma(src[idx + w]);
            let lum_w = luma(src[idx - 1]);
            let lum_e = luma(src[idx + 1]);

            // Range = max - min of local neighborhood
            let lum_min = min5(lum_c, lum_n, lum_s, lum_w, lum_e);
            let lum_max = max5(lum_c, lum_n, lum_s, lum_w, lum_e);
            let lum_range = lum_max - lum_min;

            // Skip if not enough contrast (not an edge)
            if lum_range < edge_threshold.max(lum_max * edge_threshold_min) {
                continue;
            }

            // Diagonal neighbors for sub-pixel aliasing
            let lum_nw = luma(src[idx - w - 1]);
            let lum_ne = luma(src[idx - w + 1]);
            let lum_sw = luma(src[idx + w - 1]);
            let lum_se = luma(src[idx + w + 1]);

            // Determine edge direction
            let edge_h = ((lum_nw + lum_ne) - 2.0 * lum_n).abs()
                       + ((lum_w + lum_e) - 2.0 * lum_c).abs() * 2.0
                       + ((lum_sw + lum_se) - 2.0 * lum_s).abs();
            let edge_v = ((lum_nw + lum_sw) - 2.0 * lum_w).abs()
                       + ((lum_n + lum_s) - 2.0 * lum_c).abs() * 2.0
                       + ((lum_ne + lum_se) - 2.0 * lum_e).abs();

            let is_horizontal = edge_h >= edge_v;

            // Choose blend direction perpendicular to edge
            let (lum_neg, lum_pos) = if is_horizontal {
                (lum_n, lum_s)
            } else {
                (lum_w, lum_e)
            };

            let grad_neg = (lum_neg - lum_c).abs();
            let grad_pos = (lum_pos - lum_c).abs();

            // Pick the stronger gradient side
            let (step_x, step_y) = if is_horizontal {
                if grad_neg >= grad_pos { (0i32, -1i32) } else { (0, 1) }
            } else {
                if grad_neg >= grad_pos { (-1, 0) } else { (1, 0) }
            };

            // Sub-pixel blend factor based on contrast
            let filter = (2.0 * (lum_n + lum_s + lum_w + lum_e)
                        + (lum_nw + lum_ne + lum_sw + lum_se)) / 12.0;
            let sub_pixel = ((filter - lum_c).abs() / lum_range).min(1.0);
            let sub_pixel = (-2.0 * sub_pixel + 3.0) * sub_pixel * sub_pixel;
            let blend = sub_pixel * sub_pixel * 0.75;

            // Blend with neighbor in edge-perpendicular direction
            let nx = (x as i32 + step_x).clamp(0, (w - 1) as i32) as usize;
            let ny = (y as i32 + step_y).clamp(0, (h - 1) as i32) as usize;
            let neighbor = src[ny * w + nx];

            color[idx] = blend_argb(src[idx], neighbor, blend);
        }
    }
}

/// Compute perceptual luminance from ARGB.
#[inline(always)]
fn luma(argb: u32) -> f32 {
    let r = ((argb >> 16) & 0xFF) as f32;
    let g = ((argb >> 8) & 0xFF) as f32;
    let b = (argb & 0xFF) as f32;
    (0.299 * r + 0.587 * g + 0.114 * b) / 255.0
}

/// Blend two ARGB colors by factor t (0.0 = a, 1.0 = b).
#[inline(always)]
fn blend_argb(a: u32, b: u32, t: f32) -> u32 {
    let inv = 1.0 - t;
    let ra = ((a >> 16) & 0xFF) as f32;
    let ga = ((a >> 8) & 0xFF) as f32;
    let ba = (a & 0xFF) as f32;
    let aa = ((a >> 24) & 0xFF) as f32;

    let rb = ((b >> 16) & 0xFF) as f32;
    let gb = ((b >> 8) & 0xFF) as f32;
    let bb = (b & 0xFF) as f32;
    let ab = ((b >> 24) & 0xFF) as f32;

    let r = (ra * inv + rb * t) as u32;
    let g = (ga * inv + gb * t) as u32;
    let bl = (ba * inv + bb * t) as u32;
    let alpha = (aa * inv + ab * t) as u32;

    (alpha << 24) | (r << 16) | (g << 8) | bl
}

#[inline(always)]
fn min5(a: f32, b: f32, c: f32, d: f32, e: f32) -> f32 {
    a.min(b).min(c).min(d).min(e)
}

#[inline(always)]
fn max5(a: f32, b: f32, c: f32, d: f32, e: f32) -> f32 {
    a.max(b).max(c).max(d).max(e)
}

