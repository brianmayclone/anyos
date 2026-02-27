//! Software rasterizer pipeline.
//!
//! Orchestrates the full rendering pipeline:
//! vertex assembly → vertex shader → primitive assembly → clipping →
//! perspective divide → viewport transform → rasterization → fragment shader →
//! depth test + blending → framebuffer write.

pub mod math;
pub mod vertex;
pub mod clipper;
pub mod raster;
pub mod fragment;

use alloc::vec::Vec;
use crate::state::GlContext;
use crate::types::*;
use crate::compiler::backend_sw::ShaderExec;

/// A processed vertex after the vertex shader.
#[derive(Clone)]
pub struct ClipVertex {
    /// Clip-space position (before perspective divide).
    pub position: [f32; 4],
    /// Varying values output by the vertex shader.
    pub varyings: Vec<[f32; 4]>,
}

/// Render primitives using the software rasterizer.
pub fn draw(ctx: &mut GlContext, mode: GLenum, first: i32, count: i32) {
    if count <= 0 { return; }
    let prog_id = ctx.current_program;
    let program = match ctx.shaders.get_program(prog_id) {
        Some(p) if p.linked => p,
        _ => return,
    };

    let vs_ir = match &program.vs_ir {
        Some(ir) => ir.clone(),
        None => return,
    };
    let fs_ir = match &program.fs_ir {
        Some(ir) => ir.clone(),
        None => return,
    };
    let num_varyings = program.varying_count;
    let uniforms = collect_uniforms(program);
    let attrib_info: Vec<(i32, i32, GLenum, i32, usize, u32)> = program.attributes.iter().map(|a| {
        let loc = a.location as usize;
        if loc < ctx.attribs.len() && ctx.attribs[loc].enabled {
            let va = &ctx.attribs[loc];
            (a.location, va.size, va.typ, va.stride, va.offset, va.buffer_id)
        } else {
            (a.location, 0, 0, 0, 0, 0)
        }
    }).collect();

    // ── Vertex Processing ───────────────────────────────────────────────
    let mut clip_verts = Vec::new();
    for i in first..(first + count) {
        let attributes = vertex::fetch_attributes(ctx, &attrib_info, i as u32);
        let mut exec = ShaderExec::new(vs_ir.num_regs, num_varyings);
        exec.execute(&vs_ir, &attributes, &uniforms, None, null_tex_sample);
        clip_verts.push(ClipVertex {
            position: exec.position,
            varyings: exec.varyings,
        });
    }

    // ── Primitive Assembly + Rasterization ───────────────────────────────
    let fb_w = ctx.default_fb.width as i32;
    let fb_h = ctx.default_fb.height as i32;

    match mode {
        GL_TRIANGLES => {
            let mut i = 0;
            while i + 2 < clip_verts.len() {
                let tri = [clip_verts[i].clone(), clip_verts[i+1].clone(), clip_verts[i+2].clone()];

                // Frustum clipping
                let clipped = clipper::clip_triangle(&tri);

                for t in clipped.chunks(3) {
                    if t.len() < 3 { continue; }
                    // Perspective divide + viewport transform
                    let screen: Vec<_> = t.iter().map(|v| {
                        let ndc = perspective_divide(&v.position);
                        viewport_transform(&ndc, ctx.viewport_x, ctx.viewport_y,
                                          ctx.viewport_w, ctx.viewport_h)
                    }).collect();

                    // Backface culling
                    if ctx.cull_face {
                        let area = edge_function(&screen[0], &screen[1], &screen[2]);
                        let front = match ctx.front_face {
                            GL_CCW => area > 0.0,
                            _ => area < 0.0,
                        };
                        let cull = match ctx.cull_face_mode {
                            GL_FRONT => front,
                            GL_BACK => !front,
                            GL_FRONT_AND_BACK => true,
                            _ => false,
                        };
                        if cull { continue; }
                    }

                    // Rasterize triangle
                    raster::rasterize_triangle(
                        ctx, &fs_ir, &uniforms,
                        &[&t[0], &t[1], &t[2]],
                        &screen,
                        fb_w, fb_h,
                    );
                }

                i += 3;
            }
        }
        GL_TRIANGLE_STRIP => {
            for i in 0..clip_verts.len().saturating_sub(2) {
                let tri = if i % 2 == 0 {
                    [clip_verts[i].clone(), clip_verts[i+1].clone(), clip_verts[i+2].clone()]
                } else {
                    [clip_verts[i+1].clone(), clip_verts[i].clone(), clip_verts[i+2].clone()]
                };
                let clipped = clipper::clip_triangle(&tri);
                for t in clipped.chunks(3) {
                    if t.len() < 3 { continue; }
                    let screen: Vec<_> = t.iter().map(|v| {
                        let ndc = perspective_divide(&v.position);
                        viewport_transform(&ndc, ctx.viewport_x, ctx.viewport_y,
                                          ctx.viewport_w, ctx.viewport_h)
                    }).collect();
                    raster::rasterize_triangle(
                        ctx, &fs_ir, &uniforms,
                        &[&t[0], &t[1], &t[2]], &screen, fb_w, fb_h,
                    );
                }
            }
        }
        GL_TRIANGLE_FAN => {
            for i in 1..clip_verts.len().saturating_sub(1) {
                let tri = [clip_verts[0].clone(), clip_verts[i].clone(), clip_verts[i+1].clone()];
                let clipped = clipper::clip_triangle(&tri);
                for t in clipped.chunks(3) {
                    if t.len() < 3 { continue; }
                    let screen: Vec<_> = t.iter().map(|v| {
                        let ndc = perspective_divide(&v.position);
                        viewport_transform(&ndc, ctx.viewport_x, ctx.viewport_y,
                                          ctx.viewport_w, ctx.viewport_h)
                    }).collect();
                    raster::rasterize_triangle(
                        ctx, &fs_ir, &uniforms,
                        &[&t[0], &t[1], &t[2]], &screen, fb_w, fb_h,
                    );
                }
            }
        }
        _ => {} // GL_LINES, GL_POINTS — Phase 2
    }
}

/// Render indexed primitives.
pub fn draw_elements(ctx: &mut GlContext, mode: GLenum, count: i32, type_: GLenum, offset: usize) {
    if count <= 0 { return; }
    let ebo_id = ctx.bound_element_buffer;
    let index_data = match ctx.buffers.get(ebo_id) {
        Some(buf) => buf.data.clone(),
        None => return,
    };

    let prog_id = ctx.current_program;
    let program = match ctx.shaders.get_program(prog_id) {
        Some(p) if p.linked => p,
        _ => return,
    };

    let vs_ir = match &program.vs_ir {
        Some(ir) => ir.clone(),
        None => return,
    };
    let fs_ir = match &program.fs_ir {
        Some(ir) => ir.clone(),
        None => return,
    };
    let num_varyings = program.varying_count;
    let uniforms = collect_uniforms(program);
    let attrib_info: Vec<(i32, i32, GLenum, i32, usize, u32)> = program.attributes.iter().map(|a| {
        let loc = a.location as usize;
        if loc < ctx.attribs.len() && ctx.attribs[loc].enabled {
            let va = &ctx.attribs[loc];
            (a.location, va.size, va.typ, va.stride, va.offset, va.buffer_id)
        } else {
            (a.location, 0, 0, 0, 0, 0)
        }
    }).collect();

    // Fetch indices
    let mut indices = Vec::new();
    for i in 0..count as usize {
        let idx = match type_ {
            GL_UNSIGNED_SHORT => {
                let off = offset + i * 2;
                if off + 1 < index_data.len() {
                    u32::from(index_data[off]) | (u32::from(index_data[off + 1]) << 8)
                } else { 0 }
            }
            GL_UNSIGNED_INT => {
                let off = offset + i * 4;
                if off + 3 < index_data.len() {
                    u32::from(index_data[off])
                    | (u32::from(index_data[off + 1]) << 8)
                    | (u32::from(index_data[off + 2]) << 16)
                    | (u32::from(index_data[off + 3]) << 24)
                } else { 0 }
            }
            GL_UNSIGNED_BYTE => {
                let off = offset + i;
                if off < index_data.len() { index_data[off] as u32 } else { 0 }
            }
            _ => 0,
        };
        indices.push(idx);
    }

    // Process vertices
    let mut clip_verts = Vec::new();
    for &idx in &indices {
        let attributes = vertex::fetch_attributes(ctx, &attrib_info, idx);
        let mut exec = ShaderExec::new(vs_ir.num_regs, num_varyings);
        exec.execute(&vs_ir, &attributes, &uniforms, None, null_tex_sample);
        clip_verts.push(ClipVertex {
            position: exec.position,
            varyings: exec.varyings,
        });
    }

    // Rasterize (same as draw)
    let fb_w = ctx.default_fb.width as i32;
    let fb_h = ctx.default_fb.height as i32;

    if mode == GL_TRIANGLES {
        let mut i = 0;
        while i + 2 < clip_verts.len() {
            let tri = [clip_verts[i].clone(), clip_verts[i+1].clone(), clip_verts[i+2].clone()];
            let clipped = clipper::clip_triangle(&tri);
            for t in clipped.chunks(3) {
                if t.len() < 3 { continue; }
                let screen: Vec<_> = t.iter().map(|v| {
                    let ndc = perspective_divide(&v.position);
                    viewport_transform(&ndc, ctx.viewport_x, ctx.viewport_y,
                                      ctx.viewport_w, ctx.viewport_h)
                }).collect();
                raster::rasterize_triangle(
                    ctx, &fs_ir, &uniforms,
                    &[&t[0], &t[1], &t[2]], &screen, fb_w, fb_h,
                );
            }
            i += 3;
        }
    }
}

/// Collect uniform values from program into a flat array.
fn collect_uniforms(program: &crate::shader::GlProgram) -> Vec<[f32; 4]> {
    let mut unis = Vec::new();
    for u in &program.uniforms {
        if u.size == 16 {
            // mat4: 4 vec4 columns
            for col in 0..4 {
                unis.push([
                    u.value[col * 4],
                    u.value[col * 4 + 1],
                    u.value[col * 4 + 2],
                    u.value[col * 4 + 3],
                ]);
            }
        } else {
            unis.push([u.value[0], u.value[1], u.value[2], u.value[3]]);
        }
    }
    unis
}

/// Perspective divide: xyz /= w.
fn perspective_divide(clip: &[f32; 4]) -> [f32; 3] {
    let w = clip[3];
    if w.abs() < 1e-10 {
        [0.0, 0.0, 0.0]
    } else {
        [clip[0] / w, clip[1] / w, clip[2] / w]
    }
}

/// Transform NDC [-1,1] to screen coordinates.
fn viewport_transform(ndc: &[f32; 3], vx: i32, vy: i32, vw: i32, vh: i32) -> [f32; 3] {
    [
        (ndc[0] + 1.0) * 0.5 * vw as f32 + vx as f32,
        (1.0 - ndc[1]) * 0.5 * vh as f32 + vy as f32,  // flip Y
        (ndc[2] + 1.0) * 0.5,  // depth [0, 1]
    ]
}

/// Signed area of a triangle (positive = CCW).
fn edge_function(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Null texture sampler (returns white).
fn null_tex_sample(_unit: u32, _u: f32, _v: f32) -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}
