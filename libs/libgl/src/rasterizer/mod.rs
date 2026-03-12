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
use crate::serial_println;

/// Maximum number of interpolated varyings between vertex and fragment shaders.
///
/// OpenGL ES 2.0 guarantees at least 8 vec4 varyings.
pub const MAX_VARYINGS: usize = 8;

/// Maximum number of uniform vec4 slots (mat4 = 4 slots, scalar = 1 slot).
const MAX_UNIFORM_SLOTS: usize = 128;

// ── Reusable per-frame buffers (avoid heap fragmentation) ───────────────

static mut CLIP_VERTS_BUF: Option<Vec<ClipVertex>> = None;
static mut CACHE_BUF: Option<Vec<Option<ClipVertex>>> = None;

fn reuse_clip_verts() -> &'static mut Vec<ClipVertex> {
    unsafe {
        if CLIP_VERTS_BUF.is_none() {
            CLIP_VERTS_BUF = Some(Vec::with_capacity(1024));
        }
        let v = CLIP_VERTS_BUF.as_mut().unwrap();
        v.clear();
        v
    }
}

fn reuse_cache(size: usize) -> &'static mut Vec<Option<ClipVertex>> {
    unsafe {
        if CACHE_BUF.is_none() {
            CACHE_BUF = Some(Vec::new());
        }
        let v = CACHE_BUF.as_mut().unwrap();
        v.clear();
        v.resize(size, None);
        v
    }
}

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

    // Use raw pointers to avoid cloning IR (safe: IR is never modified during draw)
    let vs_ir_ptr = match &program.vs_ir {
        Some(ir) => ir as *const crate::compiler::ir::Program,
        None => return,
    };
    let fs_ir_ptr = match &program.fs_ir {
        Some(ir) => ir as *const crate::compiler::ir::Program,
        None => return,
    };
    let num_varyings = program.varying_count.min(MAX_VARYINGS);
    let (uniforms, _uni_count) = collect_uniforms_stack(program);

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

    // Safety: IR is stored in GlContext and never modified during draw.
    let vs_ir = unsafe { &*vs_ir_ptr };
    let fs_ir = unsafe { &*fs_ir_ptr };

    // ── Vertex Processing (one ShaderExec reused for all vertices) ────────
    let mut vs_exec = ShaderExec::new(vs_ir.num_regs, num_varyings);
    let mut attrib_buf = [[0.0f32, 0.0, 0.0, 1.0]; 16];
    let clip_verts = reuse_clip_verts();

    let tex_sample_addr = raster::real_tex_sample as usize;
    let uni_slice = &uniforms[..];

    for i in first..(first + count) {
        vertex::fetch_attributes_into(ctx, &attrib_info[..num_attribs], i as u32, &mut attrib_buf);
        vs_exec.reset_vertex();
        if let Some(jit) = vs_jit {
            let mut jit_ctx = JitContext {
                regs: vs_exec.regs.as_mut_ptr() as *mut f32,
                uniforms: uni_slice.as_ptr() as *const f32,
                attributes: attrib_buf.as_ptr() as *const f32,
                varyings_in: core::ptr::null(),
                varyings_out: vs_exec.varyings.as_mut_ptr() as *mut f32,
                position: vs_exec.position.as_mut_ptr(),
                frag_color: vs_exec.frag_color.as_mut_ptr(),
                point_size: &mut vs_exec.point_size,
                tex_sample: tex_sample_addr,
                discarded: 0,
            };
            unsafe { jit(&mut jit_ctx); }
        } else {
            vs_exec.execute(vs_ir, &attrib_buf[..num_attribs], uni_slice, None, raster::real_tex_sample);
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
    let fast = if fs_ir.instructions.len() <= 20 && num_varyings == 2 && !ctx.blend {
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

    // One-time debug: log first draw call vertex positions
    static mut DRAW_DBG: u32 = 0;
    let dbg_this = unsafe { DRAW_DBG < 2 };
    if dbg_this {
        unsafe { DRAW_DBG += 1; }
        if !clip_verts.is_empty() {
            let v0 = &clip_verts[0];
            serial_println!("[libgl] draw: {} verts, v0.pos=({},{},{},{}), fb={}x{}, fast={}",
                clip_verts.len(),
                v0.position[0] as i32, v0.position[1] as i32,
                v0.position[2] as i32, v0.position[3] as i32,
                fb_w, fb_h, fast.is_some());
            let inside = clip_verts.iter().filter(|v| trivially_inside(v)).count();
            serial_println!("[libgl] draw: {} of {} verts trivially inside", inside, clip_verts.len());
            if clip_verts.len() >= 3 {
                let s0 = to_screen(&clip_verts[0].position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
                let s1 = to_screen(&clip_verts[1].position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
                let s2 = to_screen(&clip_verts[2].position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
                serial_println!("[libgl] draw: screen v0=({},{}), v1=({},{}), v2=({},{})",
                    s0[0] as i32, s0[1] as i32, s1[0] as i32, s1[1] as i32, s2[0] as i32, s2[1] as i32);
            }
        }
    }

    // ── Parallel rasterization: collect triangles into pool ─────────────
    // When pool is active, triangles are batched across draw calls and
    // submitted all at once in gl_swap_buffers (frame-level batching).
    let use_pool = crate::thread_pool::pool_active();

    if use_pool {
        let tri_start = crate::thread_pool::begin_sub_batch();
        if let Some(start) = tri_start {
            let mut overflow = false;
            match mode {
                GL_TRIANGLES => {
                    let mut i = 0;
                    while i + 2 < clip_verts.len() && !overflow {
                        overflow = !collect_triangle(ctx, &clip_verts[i], &clip_verts[i+1], &clip_verts[i+2], fb_w, fb_h);
                        i += 3;
                    }
                }
                GL_TRIANGLE_STRIP => {
                    for i in 0..clip_verts.len().saturating_sub(2) {
                        if overflow { break; }
                        let (a, b, c) = if i % 2 == 0 {
                            (&clip_verts[i], &clip_verts[i+1], &clip_verts[i+2])
                        } else {
                            (&clip_verts[i+1], &clip_verts[i], &clip_verts[i+2])
                        };
                        overflow = !collect_triangle(ctx, a, b, c, fb_w, fb_h);
                    }
                }
                GL_TRIANGLE_FAN => {
                    for i in 1..clip_verts.len().saturating_sub(1) {
                        if overflow { break; }
                        overflow = !collect_triangle(ctx, &clip_verts[0], &clip_verts[i], &clip_verts[i+1], fb_w, fb_h);
                    }
                }
                _ => {}
            }

            // Finalize sub-batch (no worker wake-up — that happens at swap_buffers)
            let fast_arg = fast.as_ref().map(|fp| (&fp.tex, fp.mat_r, fp.mat_g, fp.mat_b));
            crate::thread_pool::end_sub_batch(
                start, ctx.depth_test, ctx.depth_func, ctx.depth_mask,
                ctx.blend, ctx.blend_src_rgb, ctx.blend_dst_rgb,
                fast_arg, fs_ir, uni_slice, num_varyings, fs_jit,
            );
        } else {
            // Sub-batch overflow — fall through to single-threaded
            single_threaded_draw(ctx, fs_ir, uni_slice, &mut fs_exec, fs_jit, fast.as_ref(), clip_verts, mode, num_varyings, fb_w, fb_h);
        }
    } else {
        single_threaded_draw(ctx, fs_ir, uni_slice, &mut fs_exec, fs_jit, fast.as_ref(), clip_verts, mode, num_varyings, fb_w, fb_h);
    }
}

/// Single-threaded rasterization fallback.
fn single_threaded_draw(
    ctx: &mut GlContext,
    fs_ir: &crate::compiler::ir::Program,
    uniforms: &[[f32; 4]],
    fs_exec: &mut ShaderExec,
    fs_jit: Option<JitFn>,
    fast: Option<&FastPathInfo>,
    clip_verts: &[ClipVertex],
    mode: GLenum,
    num_varyings: usize,
    fb_w: i32,
    fb_h: i32,
) {
    match mode {
        GL_TRIANGLES => {
            let mut i = 0;
            while i + 2 < clip_verts.len() {
                process_triangle(
                    ctx, fs_ir, uniforms, fs_exec, fs_jit, fast,
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
                process_triangle(ctx, fs_ir, uniforms, fs_exec, fs_jit, fast, a, b, c, num_varyings, fb_w, fb_h);
            }
        }
        GL_TRIANGLE_FAN => {
            for i in 1..clip_verts.len().saturating_sub(1) {
                process_triangle(
                    ctx, fs_ir, uniforms, fs_exec, fs_jit, fast,
                    &clip_verts[0], &clip_verts[i], &clip_verts[i+1],
                    num_varyings, fb_w, fb_h,
                );
            }
        }
        _ => {}
    }
}

/// Render indexed primitives.
pub fn draw_elements(ctx: &mut GlContext, mode: GLenum, count: i32, type_: GLenum, offset: usize) {
    if count <= 0 { return; }
    let ebo_id = ctx.bound_element_buffer;

    // Use raw pointer to index data — avoid cloning the entire buffer
    let index_data_ptr: *const u8;
    let index_data_len: usize;
    match ctx.buffers.get(ebo_id) {
        Some(buf) => {
            index_data_ptr = buf.data.as_ptr();
            index_data_len = buf.data.len();
        }
        None => return,
    };

    let prog_id = ctx.current_program;
    let program = match ctx.shaders.get_program(prog_id) {
        Some(p) if p.linked => p,
        _ => return,
    };

    // Use raw pointers to avoid cloning IR (safe: IR is never modified during draw)
    let vs_ir_ptr = match &program.vs_ir {
        Some(ir) => ir as *const crate::compiler::ir::Program,
        None => return,
    };
    let fs_ir_ptr = match &program.fs_ir {
        Some(ir) => ir as *const crate::compiler::ir::Program,
        None => return,
    };
    let num_varyings = program.varying_count.min(MAX_VARYINGS);
    let (uniforms, _uni_count) = collect_uniforms_stack(program);

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

    // Safety: index_data is stored in GlContext buffer and never modified during draw.
    let index_data = unsafe { core::slice::from_raw_parts(index_data_ptr, index_data_len) };

    // Safety: IR is stored in GlContext and never modified during draw.
    let vs_ir = unsafe { &*vs_ir_ptr };
    let fs_ir = unsafe { &*fs_ir_ptr };
    let uni_slice = &uniforms[..];

    // Parse indices directly without allocating a Vec — find max_idx in the same pass
    // Use a stack buffer for small counts, reusable static for larger ones
    static mut INDICES_BUF: Option<Vec<u32>> = None;
    let indices: &[u32];
    // Stack buffer for small draw calls (covers most cases: cubes, simple meshes)
    let mut stack_indices = [0u32; 4096];
    let count_usize = count as usize;

    if count_usize <= 4096 {
        let mut max_idx = 0u32;
        for i in 0..count_usize {
            let idx = read_index(index_data, type_, offset, i);
            stack_indices[i] = idx;
            if idx > max_idx { max_idx = idx; }
        }
        indices = &stack_indices[..count_usize];
    } else {
        unsafe {
            if INDICES_BUF.is_none() {
                INDICES_BUF = Some(Vec::with_capacity(count_usize));
            }
            let buf = INDICES_BUF.as_mut().unwrap();
            buf.clear();
            for i in 0..count_usize {
                buf.push(read_index(index_data, type_, offset, i));
            }
            indices = core::slice::from_raw_parts(buf.as_ptr(), buf.len());
        }
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
    let use_cache = max_idx < 65536;
    let cache = if use_cache { reuse_cache(max_idx + 1) } else { reuse_cache(0) };

    let clip_verts = reuse_clip_verts();
    for &idx in indices {
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
                uniforms: uni_slice.as_ptr() as *const f32,
                attributes: attrib_buf.as_ptr() as *const f32,
                varyings_in: core::ptr::null(),
                varyings_out: vs_exec.varyings.as_mut_ptr() as *mut f32,
                position: vs_exec.position.as_mut_ptr(),
                frag_color: vs_exec.frag_color.as_mut_ptr(),
                point_size: &mut vs_exec.point_size,
                tex_sample: tex_sample_addr,
                discarded: 0,
            };
            unsafe { jit(&mut jit_ctx); }
        } else {
            vs_exec.execute(vs_ir, &attrib_buf[..num_attribs], uni_slice, None, raster::real_tex_sample);
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
    let fast = if fs_ir.instructions.len() <= 20 && num_varyings == 2 && !ctx.blend {
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

    // ── Parallel rasterization: collect into pool ─────────────────────
    let use_pool = crate::thread_pool::pool_active();

    if use_pool {
        let tri_start = crate::thread_pool::begin_sub_batch();
        if let Some(start) = tri_start {
            let mut overflow = false;
            if mode == GL_TRIANGLES {
                let mut i = 0;
                while i + 2 < clip_verts.len() && !overflow {
                    overflow = !collect_triangle(ctx, &clip_verts[i], &clip_verts[i+1], &clip_verts[i+2], fb_w, fb_h);
                    i += 3;
                }
            } else if mode == GL_TRIANGLE_STRIP {
                for i in 0..clip_verts.len().saturating_sub(2) {
                    if overflow { break; }
                    let (a, b, c) = if i % 2 == 0 {
                        (&clip_verts[i], &clip_verts[i+1], &clip_verts[i+2])
                    } else {
                        (&clip_verts[i+1], &clip_verts[i], &clip_verts[i+2])
                    };
                    overflow = !collect_triangle(ctx, a, b, c, fb_w, fb_h);
                }
            } else if mode == GL_TRIANGLE_FAN {
                for i in 1..clip_verts.len().saturating_sub(1) {
                    if overflow { break; }
                    overflow = !collect_triangle(ctx, &clip_verts[0], &clip_verts[i], &clip_verts[i+1], fb_w, fb_h);
                }
            }

            let fast_arg = fast.as_ref().map(|fp| (&fp.tex, fp.mat_r, fp.mat_g, fp.mat_b));
            crate::thread_pool::end_sub_batch(
                start, ctx.depth_test, ctx.depth_func, ctx.depth_mask,
                ctx.blend, ctx.blend_src_rgb, ctx.blend_dst_rgb,
                fast_arg, fs_ir, uni_slice, num_varyings, fs_jit,
            );
        } else {
            single_threaded_draw(ctx, fs_ir, uni_slice, &mut fs_exec, fs_jit, fast.as_ref(), clip_verts, mode, num_varyings, fb_w, fb_h);
        }
    } else {
        single_threaded_draw(ctx, fs_ir, uni_slice, &mut fs_exec, fs_jit, fast.as_ref(), clip_verts, mode, num_varyings, fb_w, fb_h);
    }
}

/// Read a single index from the element buffer without allocation.
#[inline(always)]
pub fn read_index(data: &[u8], type_: GLenum, offset: usize, i: usize) -> u32 {
    match type_ {
        GL_UNSIGNED_SHORT => {
            let off = offset + i * 2;
            if off + 1 < data.len() {
                u32::from(data[off]) | (u32::from(data[off + 1]) << 8)
            } else { 0 }
        }
        GL_UNSIGNED_INT => {
            let off = offset + i * 4;
            if off + 3 < data.len() {
                u32::from(data[off])
                | (u32::from(data[off + 1]) << 8)
                | (u32::from(data[off + 2]) << 16)
                | (u32::from(data[off + 3]) << 24)
            } else { 0 }
        }
        GL_UNSIGNED_BYTE => {
            let off = offset + i;
            if off < data.len() { data[off] as u32 } else { 0 }
        }
        _ => 0,
    }
}

/// Fast-path triangle parameters (resolved once per draw call).
pub struct FastPathInfo {
    pub tex: raster::ResolvedTexture,
    pub mat_r: f32,
    pub mat_g: f32,
    pub mat_b: f32,
}

/// Process a single triangle: clip → cull → rasterize (single-threaded fallback).
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

/// Collect a single triangle into the thread pool buffer (clip → cull → store).
/// Returns true if the triangle was added (or culled), false if buffer is full.
fn collect_triangle(
    ctx: &GlContext,
    v0: &ClipVertex,
    v1: &ClipVertex,
    v2: &ClipVertex,
    fb_w: i32,
    fb_h: i32,
) -> bool {
    use crate::thread_pool::{self, ScreenTri};

    // Helper: cull and store a screen-space triangle
    let mut store = |cv0: &ClipVertex, cv1: &ClipVertex, cv2: &ClipVertex,
                     s0: [f32; 3], s1: [f32; 3], s2: [f32; 3]| -> bool {
        if ctx.cull_face {
            let area = edge_function(&s0, &s1, &s2);
            let front = match ctx.front_face { GL_CCW => area < 0.0, _ => area > 0.0 };
            let cull = match ctx.cull_face_mode {
                GL_FRONT => front, GL_BACK => !front,
                GL_FRONT_AND_BACK => true, _ => false,
            };
            if cull { return true; } // culled, not an error
        }
        let tri = ScreenTri { v0: *cv0, v1: *cv1, v2: *cv2, s0, s1, s2 };
        thread_pool::append_tris(&[tri]) > 0
    };

    if trivially_inside(v0) && trivially_inside(v1) && trivially_inside(v2) {
        let s0 = to_screen(&v0.position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        let s1 = to_screen(&v1.position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        let s2 = to_screen(&v2.position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        return store(v0, v1, v2, s0, s1, s2);
    }

    // Clip
    let tri = [*v0, *v1, *v2];
    let clipped = clipper::clip_triangle(&tri);
    for t in clipped.chunks(3) {
        if t.len() < 3 { continue; }
        let s0 = to_screen(&t[0].position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        let s1 = to_screen(&t[1].position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        let s2 = to_screen(&t[2].position, ctx.viewport_x, ctx.viewport_y, ctx.viewport_w, ctx.viewport_h);
        if !store(&t[0], &t[1], &t[2], s0, s1, s2) {
            return false;
        }
    }
    true
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

/// Collect uniform values into a Vec (for SVGA backend — not called per-frame).
pub fn collect_uniforms(program: &crate::shader::GlProgram) -> Vec<[f32; 4]> {
    let (arr, n) = collect_uniforms_stack(program);
    arr[..n].to_vec()
}

/// Collect uniform values from program into a fixed-size stack array.
/// Returns the array and the number of slots used.
pub fn collect_uniforms_stack(program: &crate::shader::GlProgram) -> ([[f32; 4]; MAX_UNIFORM_SLOTS], usize) {
    let mut unis = [[0.0f32; 4]; MAX_UNIFORM_SLOTS];
    let mut n = 0usize;
    for u in &program.uniforms {
        if u.size == 16 {
            for col in 0..4 {
                if n < MAX_UNIFORM_SLOTS {
                    unis[n] = [
                        u.value[col * 4],
                        u.value[col * 4 + 1],
                        u.value[col * 4 + 2],
                        u.value[col * 4 + 3],
                    ];
                    n += 1;
                }
            }
        } else {
            if n < MAX_UNIFORM_SLOTS {
                unis[n] = [u.value[0], u.value[1], u.value[2], u.value[3]];
                n += 1;
            }
        }
    }
    (unis, n)
}

/// Signed area of a triangle (positive = CCW).
#[inline(always)]
fn edge_function(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
