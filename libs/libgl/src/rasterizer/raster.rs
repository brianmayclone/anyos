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
use crate::compiler::backend_jit::{JitFn, JitContext};
use crate::simd::Vec4;
use super::ClipVertex;
use super::fragment;
use super::MAX_VARYINGS;

/// Rasterize a single triangle with incremental edge functions.
///
/// `fs_exec` is a pre-allocated fragment shader execution context, reused
/// across all triangles in a draw call to eliminate per-pixel allocation.
/// `fs_jit` is an optional JIT-compiled fragment shader — if present, it is
/// used instead of the interpreter for a ~10–20× per-pixel speedup.
pub fn rasterize_triangle(
    ctx: &mut GlContext,
    fs_ir: &IrProgram,
    uniforms: &[[f32; 4]],
    fs_exec: &mut ShaderExec,
    fs_jit: Option<JitFn>,
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
    // inv_area must always be positive so barycentric coordinates are correct.
    // When area < 0 (CW screen-space winding from viewport Y-flip), we negate
    // edge values and increments so the inside test (>= 0) works uniformly.
    let inv_area = 1.0 / area.abs();

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
    let tex_sample_addr = real_tex_sample as usize;

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

    // Normalize sign: when area < 0 (CW screen winding), flip all edge
    // values and increments so the inside test (>= 0) works uniformly.
    let (mut a12, mut b12, mut a20, mut b20, mut a01, mut b01) =
        (a12, b12, a20, b20, a01, b01);
    if area < 0.0 {
        w0_row = -w0_row;
        w1_row = -w1_row;
        w2_row = -w2_row;
        a12 = -a12; b12 = -b12;
        a20 = -a20; b20 = -b20;
        a01 = -a01; b01 = -b01;
    }

    // Stack-allocated varying interpolation buffer (zero heap alloc)
    let mut varying_buf = [[0.0f32; 4]; MAX_VARYINGS];

    let depth_test_enabled = ctx.depth_test;
    let depth_func = ctx.depth_func;
    let depth_mask = ctx.depth_mask;
    let blend_enabled = ctx.blend;
    let blend_src = ctx.blend_src_rgb;
    let blend_dst = ctx.blend_dst_rgb;

    // ── Scanline loop with span clipping ─────────────────────────────────
    // Instead of scanning min_x..max_x and testing every pixel, we compute
    // the exact x range where all 3 edge functions are ≥ 0 per scanline.
    // For a sphere with 320 thin triangles, this eliminates ~95% of rejected
    // pixel iterations (from ~7M down to ~50K).

    for py in min_y..=max_y {
        // ── Compute exact x span for this scanline ──────────────────
        // Each edge: w(x) = w_row + a*(x-min_x).
        // a > 0: left bound at x = min_x + ceil(-w_row/a) when w_row < 0
        // a < 0: right bound at x = min_x + floor(w_row/|a|) when w_row >= 0
        // a ≈ 0: whole row in/out depending on w_row sign
        let mut span_left = min_x;
        let mut span_right = max_x;
        let mut empty = false;

        macro_rules! edge_clip {
            ($w:expr, $a:expr) => {
                if !empty {
                    let w_val: f32 = $w;
                    let a_val: f32 = $a;
                    if a_val > 1e-8 {
                        if w_val < 0.0 {
                            let x = min_x + super::math::ceil((-w_val) / a_val) as i32;
                            if x > span_left { span_left = x; }
                        }
                    } else if a_val < -1e-8 {
                        if w_val < 0.0 {
                            empty = true;
                        } else {
                            let x = min_x + (w_val / (-a_val)) as i32;
                            if x < span_right { span_right = x; }
                        }
                    } else if w_val < -1e-8 {
                        empty = true;
                    }
                }
            };
        }

        edge_clip!(w0_row, a12);
        edge_clip!(w1_row, a20);
        edge_clip!(w2_row, a01);

        if !empty && span_left <= span_right {
            // Advance edge functions from min_x to span_left
            let dx = (span_left - min_x) as f32;
            let mut w0 = w0_row + a12 * dx;
            let mut w1 = w1_row + a20 * dx;
            let mut w2 = w2_row + a01 * dx;

            let row_base = py as u32 * fb_width;

            for px in span_left..=span_right {
                // Safety check (float precision at span edges)
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    // Barycentric coordinates
                    let bary0 = w0 * inv_area;
                    let bary1 = w1 * inv_area;
                    let bary2 = w2 * inv_area;

                    // Depth interpolation (screen-space linear)
                    let depth = bary0 * z0 + bary1 * z1 + bary2 * z2;

                    // Early depth test — BEFORE varying interpolation and fragment shader
                    let fb_idx = (row_base + px as u32) as usize;
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

                    // Run fragment shader — JIT path or interpreter fallback
                    fs_exec.frag_color = [0.0, 0.0, 0.0, 1.0];
                    if let Some(jit) = fs_jit {
                        let mut jit_ctx = JitContext {
                            regs: fs_exec.regs.as_mut_ptr() as *mut f32,
                            uniforms: uniforms.as_ptr() as *const f32,
                            attributes: core::ptr::null(),
                            varyings_in: varying_buf.as_ptr() as *const f32,
                            varyings_out: core::ptr::null_mut(),
                            position: core::ptr::null_mut(),
                            frag_color: fs_exec.frag_color.as_mut_ptr(),
                            point_size: core::ptr::null_mut(),
                            tex_sample: tex_sample_addr,
                        };
                        unsafe { jit(&mut jit_ctx); }
                    } else {
                        fs_exec.execute(fs_ir, &[], uniforms, Some(&varying_buf[..nv]), tex_sample);
                    }
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

// ═══════════════════════════════════════════════════════════════════════════
//  Fast-path rasterizer: "textured + vertex-lit" with zero per-pixel calls
// ═══════════════════════════════════════════════════════════════════════════

/// Pre-resolved texture data for the fast-path rasterizer.
///
/// Resolved once before the draw loop to avoid per-pixel indirection
/// through the texture store.
pub struct ResolvedTexture {
    pub data: *const u32,
    pub len: usize,
    pub width: u32,
    pub height: u32,
}

impl ResolvedTexture {
    /// Resolve the currently bound texture on unit 0.
    ///
    /// Returns `None` if no texture is bound or the texture has no data.
    pub fn resolve_unit0() -> Option<Self> {
        unsafe {
            let bound = crate::BOUND_TEXTURES_PTR;
            let store = crate::TEX_STORE_PTR;
            if bound.is_null() || store.is_null() { return None; }
            let tex_id = (*bound)[0];
            if tex_id == 0 { return None; }
            match (*store).get(tex_id) {
                Some(tex) if tex.width > 0 && tex.height > 0 => Some(ResolvedTexture {
                    data: tex.data.as_ptr(),
                    len: tex.data.len(),
                    width: tex.width,
                    height: tex.height,
                }),
                _ => None,
            }
        }
    }
}

/// Fast-path rasterizer for the common "textured + vertex-lit" case.
///
/// Eliminates all per-pixel function calls by inlining:
/// - Texture sampling (direct array read, no function call chain)
/// - Color math (lighting × texColor × matColor)
/// - Depth test (inline compare)
///
/// Varyings layout: [0] = lighting (rgb in xyz), [1] = texcoord (uv in xy).
/// This matches the Gouraud vertex shader output.
pub fn rasterize_triangle_fast(
    ctx: &mut GlContext,
    tex: &ResolvedTexture,
    mat_r: f32, mat_g: f32, mat_b: f32,
    v0: &ClipVertex,
    v1: &ClipVertex,
    v2: &ClipVertex,
    s0: &[f32; 3],
    s1: &[f32; 3],
    s2: &[f32; 3],
    fb_w: i32,
    fb_h: i32,
) {
    // ── Bounding box ─────────────────────────────────────────────────────
    let min_x = min3(s0[0], s1[0], s2[0]).max(0.0) as i32;
    let max_x = (super::math::ceil(max3(s0[0], s1[0], s2[0])) as i32).min(fb_w - 1);
    let min_y = min3(s0[1], s1[1], s2[1]).max(0.0) as i32;
    let max_y = (super::math::ceil(max3(s0[1], s1[1], s2[1])) as i32).min(fb_h - 1);
    if min_x > max_x || min_y > max_y { return; }

    // ── Triangle area ────────────────────────────────────────────────────
    let area = edge_fn(s0, s1, s2);
    if area.abs() < 1e-6 { return; }
    let inv_area = 1.0 / area.abs();

    // ── Clip-space W for perspective correction ──────────────────────────
    let w0_clip = v0.position[3];
    let w1_clip = v1.position[3];
    let w2_clip = v2.position[3];
    if w0_clip.abs() < 1e-6 || w1_clip.abs() < 1e-6 || w2_clip.abs() < 1e-6 { return; }

    let inv_w0c = 1.0 / w0_clip;
    let inv_w1c = 1.0 / w1_clip;
    let inv_w2c = 1.0 / w2_clip;

    // ── Pre-divide varyings by W (lighting rgb + texcoord uv) ────────────
    // Varying 0 = lighting (r,g,b in [0],[1],[2])
    // Varying 1 = texcoord (u,v in [0],[1])
    let v0_lit = [v0.varyings[0][0] * inv_w0c, v0.varyings[0][1] * inv_w0c, v0.varyings[0][2] * inv_w0c];
    let v1_lit = [v1.varyings[0][0] * inv_w1c, v1.varyings[0][1] * inv_w1c, v1.varyings[0][2] * inv_w1c];
    let v2_lit = [v2.varyings[0][0] * inv_w2c, v2.varyings[0][1] * inv_w2c, v2.varyings[0][2] * inv_w2c];

    let v0_uv = [v0.varyings[1][0] * inv_w0c, v0.varyings[1][1] * inv_w0c];
    let v1_uv = [v1.varyings[1][0] * inv_w1c, v1.varyings[1][1] * inv_w1c];
    let v2_uv = [v2.varyings[1][0] * inv_w2c, v2.varyings[1][1] * inv_w2c];

    let z0 = s0[2]; let z1 = s1[2]; let z2 = s2[2];
    let fb_width = ctx.default_fb.width;
    let depth_test = ctx.depth_test;
    let depth_func = ctx.depth_func;
    let depth_mask = ctx.depth_mask;

    let tex_data = tex.data;
    let tex_w = tex.width;
    let tex_h = tex.height;
    let tex_w_f = tex_w as f32;
    let tex_h_f = tex_h as f32;
    let tex_w_max = (tex_w - 1) as i32;
    let tex_h_max = (tex_h - 1) as i32;

    // ── Edge function increments ─────────────────────────────────────────
    let mut a12 = s1[1] - s2[1];
    let mut b12 = s2[0] - s1[0];
    let mut a20 = s2[1] - s0[1];
    let mut b20 = s0[0] - s2[0];
    let mut a01 = s0[1] - s1[1];
    let mut b01 = s1[0] - s0[0];

    let p0x = min_x as f32 + 0.5;
    let p0y = min_y as f32 + 0.5;
    let mut w0_row = (s2[0] - s1[0]) * (p0y - s1[1]) - (s2[1] - s1[1]) * (p0x - s1[0]);
    let mut w1_row = (s0[0] - s2[0]) * (p0y - s2[1]) - (s0[1] - s2[1]) * (p0x - s2[0]);
    let mut w2_row = (s1[0] - s0[0]) * (p0y - s0[1]) - (s1[1] - s0[1]) * (p0x - s0[0]);

    if area < 0.0 {
        w0_row = -w0_row; w1_row = -w1_row; w2_row = -w2_row;
        a12 = -a12; b12 = -b12;
        a20 = -a20; b20 = -b20;
        a01 = -a01; b01 = -b01;
    }

    // ── Scanline loop with span clipping ─────────────────────────────────
    for py in min_y..=max_y {
        let mut span_left = min_x;
        let mut span_right = max_x;
        let mut empty = false;

        macro_rules! edge_clip {
            ($w:expr, $a:expr) => {
                if !empty {
                    let w_val: f32 = $w;
                    let a_val: f32 = $a;
                    if a_val > 1e-8 {
                        if w_val < 0.0 {
                            let x = min_x + super::math::ceil((-w_val) / a_val) as i32;
                            if x > span_left { span_left = x; }
                        }
                    } else if a_val < -1e-8 {
                        if w_val < 0.0 { empty = true; }
                        else {
                            let x = min_x + (w_val / (-a_val)) as i32;
                            if x < span_right { span_right = x; }
                        }
                    } else if w_val < -1e-8 { empty = true; }
                }
            };
        }

        edge_clip!(w0_row, a12);
        edge_clip!(w1_row, a20);
        edge_clip!(w2_row, a01);

        if !empty && span_left <= span_right {
            let dx = (span_left - min_x) as f32;
            let mut w0 = w0_row + a12 * dx;
            let mut w1 = w1_row + a20 * dx;
            let mut w2 = w2_row + a01 * dx;
            let row_base = py as u32 * fb_width;

            for px in span_left..=span_right {
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let bary0 = w0 * inv_area;
                    let bary1 = w1 * inv_area;
                    let bary2 = w2 * inv_area;

                    // Depth
                    let depth = bary0 * z0 + bary1 * z1 + bary2 * z2;
                    let fb_idx = (row_base + px as u32) as usize;

                    if depth_test {
                        let cur = unsafe { *ctx.default_fb.depth.get_unchecked(fb_idx) };
                        if !fragment::depth_test(depth, cur, depth_func) {
                            w0 += a12; w1 += a20; w2 += a01;
                            continue;
                        }
                    }

                    // Perspective correction
                    let inv_w = bary0 * inv_w0c + bary1 * inv_w1c + bary2 * inv_w2c;
                    let corr = fast_rcp(inv_w);

                    // Interpolate lighting (3 components)
                    let lit_r = (bary0 * v0_lit[0] + bary1 * v1_lit[0] + bary2 * v2_lit[0]) * corr;
                    let lit_g = (bary0 * v0_lit[1] + bary1 * v1_lit[1] + bary2 * v2_lit[1]) * corr;
                    let lit_b = (bary0 * v0_lit[2] + bary1 * v1_lit[2] + bary2 * v2_lit[2]) * corr;

                    // Interpolate UV (2 components)
                    let u_raw = (bary0 * v0_uv[0] + bary1 * v1_uv[0] + bary2 * v2_uv[0]) * corr;
                    let v_raw = (bary0 * v0_uv[1] + bary1 * v1_uv[1] + bary2 * v2_uv[1]) * corr;

                    // Inline GL_REPEAT wrap + nearest sample (NO function calls!)
                    let u_f = u_raw - (u_raw as i32) as f32;
                    let u_w = if u_f < 0.0 { u_f + 1.0 } else { u_f };
                    let v_f = v_raw - (v_raw as i32) as f32;
                    let v_w = if v_f < 0.0 { v_f + 1.0 } else { v_f };

                    let tx = ((u_w * tex_w_f) as i32).min(tex_w_max).max(0) as u32;
                    let ty = ((v_w * tex_h_f) as i32).min(tex_h_max).max(0) as u32;
                    let texel = unsafe { *tex_data.add((ty * tex_w + tx) as usize) };

                    // Inline ARGB unpack → multiply → repack
                    let tex_r = ((texel >> 16) & 0xFF) as f32;
                    let tex_g = ((texel >> 8) & 0xFF) as f32;
                    let tex_b = (texel & 0xFF) as f32;

                    // lighting * texColor * matColor, scaled to 0..255
                    let r = (lit_r * tex_r * mat_r).min(255.0).max(0.0) as u32;
                    let g = (lit_g * tex_g * mat_g).min(255.0).max(0.0) as u32;
                    let b = (lit_b * tex_b * mat_b).min(255.0).max(0.0) as u32;

                    let color = 0xFF000000 | (r << 16) | (g << 8) | b;

                    unsafe {
                        if depth_mask {
                            *ctx.default_fb.depth.get_unchecked_mut(fb_idx) = depth;
                        }
                        *ctx.default_fb.color.get_unchecked_mut(fb_idx) = color;
                    }
                }

                w0 += a12; w1 += a20; w2 += a01;
            }
        }

        w0_row += b12; w1_row += b20; w2_row += b01;
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
