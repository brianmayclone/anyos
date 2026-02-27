//! Triangle rasterization using edge functions.
//!
//! Scans pixels within the triangle's bounding box, computes barycentric
//! coordinates, interpolates varyings, runs the fragment shader, and writes
//! to the framebuffer.

use alloc::vec::Vec;
use crate::state::GlContext;
use crate::types::*;
use crate::compiler::ir::Program as IrProgram;
use crate::compiler::backend_sw::ShaderExec;
use super::ClipVertex;
use super::fragment;

/// Rasterize a single triangle.
pub fn rasterize_triangle(
    ctx: &mut GlContext,
    fs_ir: &IrProgram,
    uniforms: &[[f32; 4]],
    verts: &[&ClipVertex; 3],
    screen: &[[f32; 3]],
    fb_w: i32,
    fb_h: i32,
) {
    let v0 = &screen[0];
    let v1 = &screen[1];
    let v2 = &screen[2];

    // Bounding box
    let min_x = min3(v0[0], v1[0], v2[0]).max(0.0) as i32;
    let max_x = (super::math::ceil(max3(v0[0], v1[0], v2[0])) as i32).min(fb_w - 1);
    let min_y = min3(v0[1], v1[1], v2[1]).max(0.0) as i32;
    let max_y = (super::math::ceil(max3(v0[1], v1[1], v2[1])) as i32).min(fb_h - 1);

    if min_x > max_x || min_y > max_y { return; }

    // Area of the full triangle
    let area = edge_fn(v0, v1, v2);
    if area.abs() < 1e-6 { return; } // degenerate
    let inv_area = 1.0 / area;

    let w0_clip = verts[0].position[3];
    let w1_clip = verts[1].position[3];
    let w2_clip = verts[2].position[3];

    let fb_width = ctx.default_fb.width;
    let num_varyings = verts[0].varyings.len();

    // Create a texture sampler closure
    let tex_sample = make_tex_sampler(ctx);

    for py in min_y..=max_y {
        for px in min_x..=max_x {
            let p = [px as f32 + 0.5, py as f32 + 0.5];

            // Barycentric coordinates
            let w0 = edge_fn_point(v1, v2, &p) * inv_area;
            let w1 = edge_fn_point(v2, v0, &p) * inv_area;
            let w2 = 1.0 - w0 - w1;

            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 { continue; }

            // Perspective-correct interpolation
            let inv_w = w0 / w0_clip + w1 / w1_clip + w2 / w2_clip;
            if inv_w.abs() < 1e-10 { continue; }
            let corr = 1.0 / inv_w;

            // Interpolate depth
            let depth = w0 * screen[0][2] + w1 * screen[1][2] + w2 * screen[2][2];

            // Depth test
            let fb_idx = (py as u32 * fb_width + px as u32) as usize;
            if ctx.depth_test {
                let current_depth = ctx.default_fb.depth[fb_idx];
                if !fragment::depth_test(depth, current_depth, ctx.depth_func) {
                    continue;
                }
            }

            // Interpolate varyings with perspective correction
            let mut varying_data: Vec<[f32; 4]> = Vec::with_capacity(num_varyings);
            for vi in 0..num_varyings {
                let v0_val = &verts[0].varyings.get(vi).copied().unwrap_or([0.0; 4]);
                let v1_val = &verts[1].varyings.get(vi).copied().unwrap_or([0.0; 4]);
                let v2_val = &verts[2].varyings.get(vi).copied().unwrap_or([0.0; 4]);
                let mut interp = [0.0f32; 4];
                for c in 0..4 {
                    interp[c] = (w0 * v0_val[c] / w0_clip
                               + w1 * v1_val[c] / w1_clip
                               + w2 * v2_val[c] / w2_clip) * corr;
                }
                varying_data.push(interp);
            }

            // Run fragment shader
            let mut exec = ShaderExec::new(fs_ir.num_regs, num_varyings);
            exec.execute(fs_ir, &[], uniforms, Some(&varying_data), tex_sample);
            let frag_color = exec.frag_color;

            // Convert fragment color [r,g,b,a] to ARGB u32
            let r = (frag_color[0].clamp(0.0, 1.0) * 255.0) as u32;
            let g = (frag_color[1].clamp(0.0, 1.0) * 255.0) as u32;
            let b = (frag_color[2].clamp(0.0, 1.0) * 255.0) as u32;
            let a = (frag_color[3].clamp(0.0, 1.0) * 255.0) as u32;
            let color = (a << 24) | (r << 16) | (g << 8) | b;

            // Blending
            let final_color = if ctx.blend {
                let dst = ctx.default_fb.color[fb_idx];
                fragment::blend(color, dst, ctx.blend_src_rgb, ctx.blend_dst_rgb)
            } else {
                color
            };

            // Write to framebuffer
            if ctx.depth_mask {
                ctx.default_fb.depth[fb_idx] = depth;
            }
            ctx.default_fb.color[fb_idx] = final_color;
        }
    }
}

fn edge_fn(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn edge_fn_point(a: &[f32; 3], b: &[f32; 3], p: &[f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn min3(a: f32, b: f32, c: f32) -> f32 {
    let m = if a < b { a } else { b };
    if m < c { m } else { c }
}

fn max3(a: f32, b: f32, c: f32) -> f32 {
    let m = if a > b { a } else { b };
    if m > c { m } else { c }
}

/// Build a texture sampler function pointer for the current GL state.
fn make_tex_sampler(_ctx: &GlContext) -> fn(u32, f32, f32) -> [f32; 4] {
    // For Phase 1, return a simple white-pixel sampler.
    // Full texture lookup requires unsafe static access to ctx.textures;
    // will be improved with a proper context threading model.
    null_tex_sample
}

fn null_tex_sample(_unit: u32, _u: f32, _v: f32) -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}
