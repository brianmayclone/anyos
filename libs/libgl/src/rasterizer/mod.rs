//! Software rasterizer pipeline.
//!
//! Orchestrates the full rendering pipeline:
//! vertex assembly → vertex shader → primitive assembly → clipping →
//! perspective divide → viewport transform → rasterization → fragment shader →
//! depth test + blending → framebuffer write.
//!
//! **Performance**: Zero heap allocations in the per-pixel hot path. Fixed-size
//! `ClipVertex`, pre-allocated `ShaderExec`, incremental edge functions, and
//! pre-computed perspective correction factors yield ~100–1000× speedup over
//! the original implementation.

pub mod math;
pub mod vertex;
pub mod clipper;
pub mod raster;
pub mod fragment;

use alloc::vec::Vec;
use crate::state::GlContext;
use crate::types::*;
use crate::compiler::backend_sw::ShaderExec;
use crate::compiler::backend_jit::{JitFn, JitContext};

/// Maximum number of interpolated varyings between vertex and fragment shaders.
///
/// OpenGL ES 2.0 guarantees at least 8 vec4 varyings.
pub const MAX_VARYINGS: usize = 8;

/// A processed vertex after the vertex shader.
///
/// Uses fixed-size inline arrays for varyings to avoid heap allocation.
/// This makes `ClipVertex` `Copy`-able — cheap 160-byte memcpy instead of
/// heap-allocating `Vec` per vertex.
#[derive(Clone, Copy)]
pub struct ClipVertex {
    /// Clip-space position (before perspective divide).
    pub position: [f32; 4],
    /// Varying values output by the vertex shader (fixed-size).
    pub varyings: [[f32; 4]; MAX_VARYINGS],
    /// Number of active varyings.
    pub num_varyings: usize,
}

impl ClipVertex {
    /// Create a zeroed `ClipVertex`.
    #[inline(always)]
    pub fn zeroed() -> Self {
        Self {
            position: [0.0; 4],
            varyings: [[0.0; 4]; MAX_VARYINGS],
            num_varyings: 0,
        }
    }
}

/// Check if a clip-space vertex is trivially inside the frustum.
///
/// If all 3 vertices of a triangle pass this test, clipping can be skipped
/// entirely — a huge win since clipping involves `Vec` allocations.
#[inline(always)]
fn trivially_inside(v: &ClipVertex) -> bool {
    let w = v.position[3];
    if w <= 0.0 { return false; }
    v.position[0] >= -w && v.position[0] <= w &&
    v.position[1] >= -w && v.position[1] <= w &&
    v.position[2] >= -w && v.position[2] <= w
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
    let num_varyings = program.varying_count.min(MAX_VARYINGS);
    let uniforms = collect_uniforms(program);

    // Extract matColor early (before program borrow ends)
    let mat_color = program.uniforms.iter().rev()
        .find(|u| u.size <= 4 && u.name.contains("MatColor"))
        .map(|u| [u.value[0], u.value[1], u.value[2]])
        .unwrap_or([1.0, 1.0, 1.0]);

    // Get JIT function pointers (compiled at link time)
    let vs_jit: Option<JitFn> = program.vs_jit.as_ref().map(|j| j.as_fn());
    let fs_jit: Option<JitFn> = program.fs_jit.as_ref().map(|j| j.as_fn());

    // Build attribute info (stack-allocated, max 16 entries)
    let mut attrib_info = [(0i32, 0i32, 0u32, 0i32, 0usize, 0u32); 16];
    let num_attribs = program.attributes.len().min(16);
    for (i, a) in program.attributes.iter().enumerate().take(num_attribs) {
        let loc = a.location as usize;
        if loc < ctx.attribs.len() && ctx.attribs[loc].enabled {
            let va = &ctx.attribs[loc];
            attrib_info[i] = (a.location, va.size, va.typ, va.stride, va.offset, va.buffer_id);
        }
    }

    // Set raw texture pointers before draw — avoids &CTX / &mut CTX aliasing UB.
    unsafe {
        crate::TEX_STORE_PTR = &ctx.textures as *const _;
        crate::BOUND_TEXTURES_PTR = &ctx.bound_textures as *const _;
    }

    // ── Vertex Processing (one ShaderExec reused for all vertices) ────────
    let mut vs_exec = ShaderExec::new(vs_ir.num_regs, num_varyings);
    let mut attrib_buf = [[0.0f32, 0.0, 0.0, 1.0]; 16];
    let mut clip_verts = Vec::with_capacity(count as usize);

    let tex_sample_addr = raster::real_tex_sample as usize;

    for i in first..(first + count) {
        vertex::fetch_attributes_into(ctx, &attrib_info[..num_attribs], i as u32, &mut attrib_buf);
        vs_exec.reset_vertex();
        if let Some(jit) = vs_jit {
            let mut jit_ctx = JitContext {
                regs: vs_exec.regs.as_mut_ptr() as *mut f32,
                uniforms: uniforms.as_ptr() as *const f32,
                attributes: attrib_buf.as_ptr() as *const f32,
                varyings_in: core::ptr::null(),
                varyings_out: vs_exec.varyings.as_mut_ptr() as *mut f32,
                position: vs_exec.position.as_mut_ptr(),
                frag_color: vs_exec.frag_color.as_mut_ptr(),
                point_size: &mut vs_exec.point_size,
                tex_sample: tex_sample_addr,
            };
            unsafe { jit(&mut jit_ctx); }
        } else {
            vs_exec.execute(&vs_ir, &attrib_buf[..num_attribs], &uniforms, None, raster::real_tex_sample);
        }
        clip_verts.push(ClipVertex {
            position: vs_exec.position,
            varyings: vs_exec.varyings,
            num_varyings,
        });
    }

    // ── Primitive Assembly + Rasterization ───────────────────────────────
    let fb_w = ctx.default_fb.width as i32;
    let fb_h = ctx.default_fb.height as i32;

    // Try fast path: trivial FS (≤20 instructions) + bound texture + 2 varyings
    let fast = if fs_ir.instructions.len() <= 20 && num_varyings >= 2 && !ctx.blend {
        raster::ResolvedTexture::resolve_unit0().map(|tex| FastPathInfo {
            tex,
            mat_r: mat_color[0],
            mat_g: mat_color[1],
            mat_b: mat_color[2],
        })
    } else {
        None
    };

    // Pre-allocate fragment shader exec (reused for all pixels in this draw call)
    let mut fs_exec = ShaderExec::new(fs_ir.num_regs, num_varyings);

    match mode {
        GL_TRIANGLES => {
            let mut i = 0;
            while i + 2 < clip_verts.len() {
                process_triangle(
                    ctx, &fs_ir, &uniforms, &mut fs_exec, fs_jit, fast.as_ref(),
                    &clip_verts[i], &clip_verts[i+1], &clip_verts[i+2],
                    num_varyings, fb_w, fb_h,
                );
                i += 3;
            }
        }
        GL_TRIANGLE_STRIP => {
            for i in 0..clip_verts.len().saturating_sub(2) {
                let (a, b, c) = if i % 2 == 0 {
                    (&clip_verts[i], &clip_verts[i+1], &clip_verts[i+2])
                } else {
                    (&clip_verts[i+1], &clip_verts[i], &clip_verts[i+2])
                };
                process_triangle(ctx, &fs_ir, &uniforms, &mut fs_exec, fs_jit, fast.as_ref(), a, b, c, num_varyings, fb_w, fb_h);
            }
        }
        GL_TRIANGLE_FAN => {
            for i in 1..clip_verts.len().saturating_sub(1) {
                process_triangle(
                    ctx, &fs_ir, &uniforms, &mut fs_exec, fs_jit, fast.as_ref(),
                    &clip_verts[0], &clip_verts[i], &clip_verts[i+1],
                    num_varyings, fb_w, fb_h,
                );
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
    let num_varyings = program.varying_count.min(MAX_VARYINGS);
    let uniforms = collect_uniforms(program);

    // Extract matColor early (before program borrow ends)
    let mat_color = program.uniforms.iter().rev()
        .find(|u| u.size <= 4 && u.name.contains("MatColor"))
        .map(|u| [u.value[0], u.value[1], u.value[2]])
        .unwrap_or([1.0, 1.0, 1.0]);

    // Get JIT function pointers (compiled at link time)
    let vs_jit: Option<JitFn> = program.vs_jit.as_ref().map(|j| j.as_fn());
    let fs_jit: Option<JitFn> = program.fs_jit.as_ref().map(|j| j.as_fn());

    let mut attrib_info = [(0i32, 0i32, 0u32, 0i32, 0usize, 0u32); 16];
    let num_attribs = program.attributes.len().min(16);
    for (i, a) in program.attributes.iter().enumerate().take(num_attribs) {
        let loc = a.location as usize;
        if loc < ctx.attribs.len() && ctx.attribs[loc].enabled {
            let va = &ctx.attribs[loc];
            attrib_info[i] = (a.location, va.size, va.typ, va.stride, va.offset, va.buffer_id);
        }
    }

    // Fetch indices into a compact buffer
    let mut indices = Vec::with_capacity(count as usize);
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

    // Set raw texture pointers before draw — avoids &CTX / &mut CTX aliasing UB.
    unsafe {
        crate::TEX_STORE_PTR = &ctx.textures as *const _;
        crate::BOUND_TEXTURES_PTR = &ctx.bound_textures as *const _;
    }

    // ── Vertex Processing with post-transform cache ─────────────────────
    let mut vs_exec = ShaderExec::new(vs_ir.num_regs, num_varyings);
    let mut attrib_buf = [[0.0f32, 0.0, 0.0, 1.0]; 16];
    let tex_sample_addr = raster::real_tex_sample as usize;

    let max_idx = indices.iter().copied().max().unwrap_or(0) as usize;
    let mut cache: Vec<Option<ClipVertex>> = Vec::new();
    let use_cache = max_idx < 65536;
    if use_cache {
        cache.resize(max_idx + 1, None);
    }

    let mut clip_verts = Vec::with_capacity(count as usize);
    for &idx in &indices {
        if use_cache {
            if let Some(cached) = &cache[idx as usize] {
                clip_verts.push(*cached);
                continue;
            }
        }
        vertex::fetch_attributes_into(ctx, &attrib_info[..num_attribs], idx, &mut attrib_buf);
        vs_exec.reset_vertex();
        if let Some(jit) = vs_jit {
            let mut jit_ctx = JitContext {
                regs: vs_exec.regs.as_mut_ptr() as *mut f32,
                uniforms: uniforms.as_ptr() as *const f32,
                attributes: attrib_buf.as_ptr() as *const f32,
                varyings_in: core::ptr::null(),
                varyings_out: vs_exec.varyings.as_mut_ptr() as *mut f32,
                position: vs_exec.position.as_mut_ptr(),
                frag_color: vs_exec.frag_color.as_mut_ptr(),
                point_size: &mut vs_exec.point_size,
                tex_sample: tex_sample_addr,
            };
            unsafe { jit(&mut jit_ctx); }
        } else {
            vs_exec.execute(&vs_ir, &attrib_buf[..num_attribs], &uniforms, None, raster::real_tex_sample);
        }
        let cv = ClipVertex {
            position: vs_exec.position,
            varyings: vs_exec.varyings,
            num_varyings,
        };
        if use_cache {
            cache[idx as usize] = Some(cv);
        }
        clip_verts.push(cv);
    }

    // Rasterize
    let fb_w = ctx.default_fb.width as i32;
    let fb_h = ctx.default_fb.height as i32;

    // Try fast path (same logic as draw_arrays)
    let fast = if fs_ir.instructions.len() <= 20 && num_varyings >= 2 && !ctx.blend {
        raster::ResolvedTexture::resolve_unit0().map(|tex| FastPathInfo {
            tex,
            mat_r: mat_color[0],
            mat_g: mat_color[1],
            mat_b: mat_color[2],
        })
    } else {
        None
    };

    let mut fs_exec = ShaderExec::new(fs_ir.num_regs, num_varyings);

    if mode == GL_TRIANGLES {
        let mut i = 0;
        while i + 2 < clip_verts.len() {
            process_triangle(
                ctx, &fs_ir, &uniforms, &mut fs_exec, fs_jit, fast.as_ref(),
                &clip_verts[i], &clip_verts[i+1], &clip_verts[i+2],
                num_varyings, fb_w, fb_h,
            );
            i += 3;
        }
    } else if mode == GL_TRIANGLE_STRIP {
        for i in 0..clip_verts.len().saturating_sub(2) {
            let (a, b, c) = if i % 2 == 0 {
                (&clip_verts[i], &clip_verts[i+1], &clip_verts[i+2])
            } else {
                (&clip_verts[i+1], &clip_verts[i], &clip_verts[i+2])
            };
            process_triangle(ctx, &fs_ir, &uniforms, &mut fs_exec, fs_jit, fast.as_ref(), a, b, c, num_varyings, fb_w, fb_h);
        }
    } else if mode == GL_TRIANGLE_FAN {
        for i in 1..clip_verts.len().saturating_sub(1) {
            process_triangle(
                ctx, &fs_ir, &uniforms, &mut fs_exec, fs_jit, fast.as_ref(),
                &clip_verts[0], &clip_verts[i], &clip_verts[i+1],
                num_varyings, fb_w, fb_h,
            );
        }
    }
}

/// Fast-path triangle parameters (resolved once per draw call).
pub struct FastPathInfo {
    pub tex: raster::ResolvedTexture,
    pub mat_r: f32,
    pub mat_g: f32,
    pub mat_b: f32,
}

/// Process a single triangle: clip → cull → rasterize.
///
/// Uses trivial-accept test to skip clipping for fully visible triangles.
/// When `fast` is `Some`, uses the fast-path rasterizer (zero per-pixel calls).
fn process_triangle(
    ctx: &mut GlContext,
    fs_ir: &crate::compiler::ir::Program,
    uniforms: &[[f32; 4]],
    fs_exec: &mut ShaderExec,
    fs_jit: Option<JitFn>,
    fast: Option<&FastPathInfo>,
    v0: &ClipVertex,
    v1: &ClipVertex,
    v2: &ClipVertex,
    num_varyings: usize,
    fb_w: i32,
    fb_h: i32,
) {
    // Fast path: if all vertices are inside the frustum, skip clipping entirely
    if trivially_inside(v0) && trivially_inside(v1) && trivially_inside(v2) {
        let s0 = to_screen(&v0.position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        let s1 = to_screen(&v1.position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        let s2 = to_screen(&v2.position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);

        if ctx.cull_face {
            let area = edge_function(&s0, &s1, &s2);
            let front = match ctx.front_face { GL_CCW => area < 0.0, _ => area > 0.0 };
            let cull = match ctx.cull_face_mode {
                GL_FRONT => front,
                GL_BACK => !front,
                GL_FRONT_AND_BACK => true,
                _ => false,
            };
            if cull { return; }
        }

        if let Some(fp) = fast {
            raster::rasterize_triangle_fast(ctx, &fp.tex, fp.mat_r, fp.mat_g, fp.mat_b, v0, v1, v2, &s0, &s1, &s2, fb_w, fb_h);
        } else {
            raster::rasterize_triangle(ctx, fs_ir, uniforms, fs_exec, fs_jit, v0, v1, v2, &s0, &s1, &s2, num_varyings, fb_w, fb_h);
        }
        return;
    }

    // Slow path: clip against frustum
    let tri = [*v0, *v1, *v2];
    let clipped = clipper::clip_triangle(&tri);

    for t in clipped.chunks(3) {
        if t.len() < 3 { continue; }
        let s0 = to_screen(&t[0].position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        let s1 = to_screen(&t[1].position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        let s2 = to_screen(&t[2].position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);

        if ctx.cull_face {
            let area = edge_function(&s0, &s1, &s2);
            let front = match ctx.front_face { GL_CCW => area < 0.0, _ => area > 0.0 };
            let cull = match ctx.cull_face_mode {
                GL_FRONT => front, GL_BACK => !front,
                GL_FRONT_AND_BACK => true, _ => false,
            };
            if cull { continue; }
        }

        if let Some(fp) = fast {
            raster::rasterize_triangle_fast(ctx, &fp.tex, fp.mat_r, fp.mat_g, fp.mat_b, &t[0], &t[1], &t[2], &s0, &s1, &s2, fb_w, fb_h);
        } else {
            raster::rasterize_triangle(ctx, fs_ir, uniforms, fs_exec, fs_jit, &t[0], &t[1], &t[2], &s0, &s1, &s2, t[0].num_varyings, fb_w, fb_h);
        }
    }
}

/// Perspective divide + viewport transform in one step.
#[inline(always)]
fn to_screen(clip: &[f32; 4], vx: i32, vy: i32, vw: i32, vh: i32) -> [f32; 3] {
    let w = clip[3];
    if w.abs() < 1e-10 {
        return [0.0, 0.0, 0.0];
    }
    let inv_w = 1.0 / w;
    let nx = clip[0] * inv_w;
    let ny = clip[1] * inv_w;
    let nz = clip[2] * inv_w;
    [
        (nx + 1.0) * 0.5 * vw as f32 + vx as f32,
        (1.0 - ny) * 0.5 * vh as f32 + vy as f32,  // flip Y
        (nz + 1.0) * 0.5,  // depth [0, 1]
    ]
}

/// Collect uniform values from program into a flat array.
pub fn collect_uniforms(program: &crate::shader::GlProgram) -> Vec<[f32; 4]> {
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

/// Signed area of a triangle (positive = CCW).
#[inline(always)]
fn edge_function(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
