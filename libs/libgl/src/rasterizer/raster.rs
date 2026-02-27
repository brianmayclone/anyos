//! Triangle rasterization using incremental edge functions.
//!
//! Scans pixels within the triangle's bounding box using **incremental edge
//! function stepping** — only 3 additions per pixel instead of 6 multiplications.
//! Perspective-correct varyings are pre-divided by clip-space W per vertex so
//! the per-pixel inner loop only does multiply-add chains.
//!
//! **Zero heap allocation**: varying interpolation uses a stack buffer, and the
//! `ShaderExec` is passed in pre-allocated from the draw call.

use crate::state::GlContext;
use crate::types::*;
use crate::compiler::ir::Program as IrProgram;
use crate::compiler::backend_sw::ShaderExec;
use crate::simd::Vec4;
use super::ClipVertex;
use super::fragment;
use super::MAX_VARYINGS;

/// Rasterize a single triangle with incremental edge functions.
///
/// `fs_exec` is a pre-allocated fragment shader execution context, reused
/// across all triangles in a draw call to eliminate per-pixel allocation.
pub fn rasterize_triangle(
    ctx: &mut GlContext,
    fs_ir: &IrProgram,
    uniforms: &[[f32; 4]],
    fs_exec: &mut ShaderExec,
    v0: &ClipVertex,
    v1: &ClipVertex,
    v2: &ClipVertex,
    s0: &[f32; 3],
    s1: &[f32; 3],
    s2: &[f32; 3],
    num_varyings: usize,
    fb_w: i32,
    fb_h: i32,
) {
    // ── Bounding box ─────────────────────────────────────────────────────
    let min_x = min3(s0[0], s1[0], s2[0]).max(0.0) as i32;
    let max_x = (super::math::ceil(max3(s0[0], s1[0], s2[0])) as i32).min(fb_w - 1);
    let min_y = min3(s0[1], s1[1], s2[1]).max(0.0) as i32;
    let max_y = (super::math::ceil(max3(s0[1], s1[1], s2[1])) as i32).min(fb_h - 1);

    if min_x > max_x || min_y > max_y { return; }

    // ── Triangle area + degenerate check ─────────────────────────────────
    let area = edge_fn(s0, s1, s2);
    if area.abs() < 1e-6 { return; }
    let inv_area = 1.0 / area;

    // ── Clip-space W for perspective correction ──────────────────────────
    let w0_clip = v0.position[3];
    let w1_clip = v1.position[3];
    let w2_clip = v2.position[3];

    // Guard against near-zero w values
    if w0_clip.abs() < 1e-6 || w1_clip.abs() < 1e-6 || w2_clip.abs() < 1e-6 {
        return;
    }

    let inv_w0c = 1.0 / w0_clip;
    let inv_w1c = 1.0 / w1_clip;
    let inv_w2c = 1.0 / w2_clip;

    // ── Pre-compute perspective-divided varyings per vertex ──────────────
    // v_persp[vertex][varying] = varying_value / w_clip
    // This moves the per-vertex division out of the per-pixel loop.
    let nv = num_varyings.min(MAX_VARYINGS);
    let mut v0_persp = [[0.0f32; 4]; MAX_VARYINGS];
    let mut v1_persp = [[0.0f32; 4]; MAX_VARYINGS];
    let mut v2_persp = [[0.0f32; 4]; MAX_VARYINGS];
    for vi in 0..nv {
        let iw0 = Vec4::splat(inv_w0c);
        let iw1 = Vec4::splat(inv_w1c);
        let iw2 = Vec4::splat(inv_w2c);
        Vec4::load(&v0.varyings[vi]).mul(iw0).store(&mut v0_persp[vi]);
        Vec4::load(&v1.varyings[vi]).mul(iw1).store(&mut v1_persp[vi]);
        Vec4::load(&v2.varyings[vi]).mul(iw2).store(&mut v2_persp[vi]);
    }

    // ── Depth values ─────────────────────────────────────────────────────
    let z0 = s0[2];
    let z1 = s1[2];
    let z2 = s2[2];

    let fb_width = ctx.default_fb.width;
    let tex_sample = real_tex_sample;

    // ── Incremental edge function setup ──────────────────────────────────
    // Edge function e(px,py) for edge (a→b) evaluated at point p:
    //   e = (b.x - a.x)*(p.y - a.y) - (b.y - a.y)*(p.x - a.x)
    // Stepping right (px+1): delta_x = a.y - b.y
    // Stepping down  (py+1): delta_y = b.x - a.x

    // w0 = edge(v1→v2, p): a=v1, b=v2
    let a12 = s1[1] - s2[1]; // dx step for w0
    let b12 = s2[0] - s1[0]; // dy step for w0

    // w1 = edge(v2→v0, p): a=v2, b=v0
    let a20 = s2[1] - s0[1]; // dx step for w1
    let b20 = s0[0] - s2[0]; // dy step for w1

    // w2 = edge(v0→v1, p): a=v0, b=v1
    let a01 = s0[1] - s1[1]; // dx step for w2
    let b01 = s1[0] - s0[0]; // dy step for w2

    // Initial edge function values at (min_x + 0.5, min_y + 0.5)
    let p0x = min_x as f32 + 0.5;
    let p0y = min_y as f32 + 0.5;
    let mut w0_row = (s2[0] - s1[0]) * (p0y - s1[1]) - (s2[1] - s1[1]) * (p0x - s1[0]);
    let mut w1_row = (s0[0] - s2[0]) * (p0y - s2[1]) - (s0[1] - s2[1]) * (p0x - s2[0]);
    let mut w2_row = (s1[0] - s0[0]) * (p0y - s0[1]) - (s1[1] - s0[1]) * (p0x - s0[0]);

    // Stack-allocated varying interpolation buffer (zero heap alloc)
    let mut varying_buf = [[0.0f32; 4]; MAX_VARYINGS];

    let depth_test_enabled = ctx.depth_test;
    let depth_func = ctx.depth_func;
    let depth_mask = ctx.depth_mask;
    let blend_enabled = ctx.blend;
    let blend_src = ctx.blend_src_rgb;
    let blend_dst = ctx.blend_dst_rgb;

    // ── Scanline loop ────────────────────────────────────────────────────
    for py in min_y..=max_y {
        let mut w0 = w0_row;
        let mut w1 = w1_row;
        let mut w2 = w2_row;

        for px in min_x..=max_x {
            // Inside-triangle test: all edge values must be ≥ 0
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                // Barycentric coordinates
                let bary0 = w0 * inv_area;
                let bary1 = w1 * inv_area;
                let bary2 = w2 * inv_area;

                // Depth interpolation (screen-space linear)
                let depth = bary0 * z0 + bary1 * z1 + bary2 * z2;

                // Early depth test — BEFORE varying interpolation and fragment shader
                let fb_idx = (py as u32 * fb_width + px as u32) as usize;
                if depth_test_enabled {
                    let current_depth = unsafe { *ctx.default_fb.depth.get_unchecked(fb_idx) };
                    if !fragment::depth_test(depth, current_depth, depth_func) {
                        w0 += a12;
                        w1 += a20;
                        w2 += a01;
                        continue;
                    }
                }

                // Perspective-correct interpolation weight
                let inv_w = bary0 * inv_w0c + bary1 * inv_w1c + bary2 * inv_w2c;
                if inv_w.abs() < 1e-10 {
                    w0 += a12;
                    w1 += a20;
                    w2 += a01;
                    continue;
                }
                // Fast reciprocal approximation (1 cycle vs ~20 for division)
                let corr = fast_rcp(inv_w);

                // Interpolate varyings with perspective correction (SIMD)
                let b0 = Vec4::splat(bary0);
                let b1 = Vec4::splat(bary1);
                let b2 = Vec4::splat(bary2);
                let corr_v = Vec4::splat(corr);

                for vi in 0..nv {
                    b0.mul(Vec4::load(&v0_persp[vi]))
                        .add(b1.mul(Vec4::load(&v1_persp[vi])))
                        .add(b2.mul(Vec4::load(&v2_persp[vi])))
                        .mul(corr_v)
                        .store(&mut varying_buf[vi]);
                }

                // Run fragment shader (reusing pre-allocated exec)
                fs_exec.reset_fragment();
                fs_exec.execute(fs_ir, &[], uniforms, Some(&varying_buf[..nv]), tex_sample);
                let fc = fs_exec.frag_color;

                // Convert fragment color [r,g,b,a] to ARGB u32
                let r = (fc[0].clamp(0.0, 1.0) * 255.0) as u32;
                let g = (fc[1].clamp(0.0, 1.0) * 255.0) as u32;
                let b = (fc[2].clamp(0.0, 1.0) * 255.0) as u32;
                let a = (fc[3].clamp(0.0, 1.0) * 255.0) as u32;
                let color = (a << 24) | (r << 16) | (g << 8) | b;

                // Blending
                let final_color = if blend_enabled {
                    let dst = unsafe { *ctx.default_fb.color.get_unchecked(fb_idx) };
                    fragment::blend(color, dst, blend_src, blend_dst)
                } else {
                    color
                };

                // Write to framebuffer
                unsafe {
                    if depth_mask {
                        *ctx.default_fb.depth.get_unchecked_mut(fb_idx) = depth;
                    }
                    *ctx.default_fb.color.get_unchecked_mut(fb_idx) = final_color;
                }
            }

            // Step edge functions right (+1 pixel)
            w0 += a12;
            w1 += a20;
            w2 += a01;
        }

        // Step edge functions down (+1 scanline)
        w0_row += b12;
        w1_row += b20;
        w2_row += b01;
    }
}

/// Edge function: signed area of triangle (a, b, c).
#[inline(always)]
fn edge_fn(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

#[inline(always)]
fn min3(a: f32, b: f32, c: f32) -> f32 {
    let m = if a < b { a } else { b };
    if m < c { m } else { c }
}

#[inline(always)]
fn max3(a: f32, b: f32, c: f32) -> f32 {
    let m = if a > b { a } else { b };
    if m > c { m } else { c }
}

/// Fast reciprocal using SSE `rcpss` (~12 bits, refined with Newton-Raphson).
///
/// ~4 cycles total vs ~20 for `divss`. Accuracy is sufficient for
/// perspective correction in rendering.
#[inline(always)]
fn fast_rcp(x: f32) -> f32 {
    unsafe {
        use core::arch::x86_64::*;
        let v = _mm_set_ss(x);
        let approx = _mm_rcp_ss(v);
        // Newton-Raphson refinement: y = y * (2 - x*y)
        let xy = _mm_mul_ss(v, approx);
        let two = _mm_set_ss(2.0);
        let refined = _mm_mul_ss(approx, _mm_sub_ss(two, xy));
        _mm_cvtss_f32(refined)
    }
}

/// Texture sampler using raw pointers to avoid `&CTX` / `&mut CTX` aliasing.
///
/// `TEX_STORE_PTR` and `BOUND_TEXTURES_PTR` are set before each draw call in
/// `rasterizer::draw()` / `draw_elements()`, so they always point at the
/// current context's texture state without creating a second reference.
pub fn real_tex_sample(unit: u32, u: f32, v: f32) -> [f32; 4] {
    unsafe {
        let bound = crate::BOUND_TEXTURES_PTR;
        let store = crate::TEX_STORE_PTR;
        if bound.is_null() || store.is_null() {
            return [1.0, 1.0, 1.0, 1.0];
        }
        let unit_idx = unit as usize;
        if unit_idx >= crate::state::MAX_TEXTURE_UNITS {
            return [1.0, 1.0, 1.0, 1.0];
        }
        let tex_id = (*bound)[unit_idx];
        if tex_id == 0 {
            return [1.0, 1.0, 1.0, 1.0];
        }
        match (*store).get(tex_id) {
            Some(tex) => tex.sample(u, v),
            None => [1.0, 1.0, 1.0, 1.0],
        }
    }
}
