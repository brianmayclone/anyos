//! Draw command dispatch.
//!
//! Dispatches draw calls to either a loaded hardware driver (.drv) or the
//! software rasterizer, depending on whether a driver is active.

use crate::state::GlContext;
use crate::types::*;
use crate::rasterizer;
use crate::drv_loader::{self, DrvAttrib, GpuDrv};

/// Execute glDrawArrays.
pub fn draw_arrays(ctx: &mut GlContext, mode: GLenum, first: GLint, count: GLsizei) {
    if unsafe { crate::USE_HW_BACKEND } {
        draw_arrays_hw(ctx, mode, first, count);
    } else {
        rasterizer::draw(ctx, mode, first, count);
    }
}

/// Execute glDrawElements.
pub fn draw_elements(
    ctx: &mut GlContext,
    mode: GLenum,
    count: GLsizei,
    type_: GLenum,
    offset: usize,
) {
    if unsafe { crate::USE_HW_BACKEND } {
        draw_elements_hw(ctx, mode, count, type_, offset);
    } else {
        rasterizer::draw_elements(ctx, mode, count, type_, offset);
    }
}

// ── Hardware draw path via drv_loader ──────────────────────────────────

/// Bind all active GL textures to the virgl driver's sampler slots.
///
/// Scans the bound program's FS IR for TexSample instructions. If any exist,
/// iterates the GL texture units and uploads + binds each non-zero texture
/// to the corresponding virgl sampler slot.
fn bind_textures_hw(ctx: &GlContext, drv: &drv_loader::GpuDrv) {
    let upload_fn = match drv.drv_upload_texture {
        Some(f) => f,
        None => return,
    };
    let bind_fn = match drv.drv_bind_sampler_view {
        Some(f) => f,
        None => return,
    };

    // Check if the FS IR uses any TexSample instructions
    let prog = match ctx.shaders.get_program(ctx.current_program) {
        Some(p) if p.linked => p,
        _ => return,
    };
    let fs_ir = match &prog.fs_ir {
        Some(ir) => ir,
        None => return,
    };
    let has_tex = fs_ir.instructions.iter().any(|i| {
        matches!(i, crate::compiler::ir::Inst::TexSample(_, _, _))
    });
    if !has_tex { return; }

    // Bind each active texture unit that has a texture bound
    for unit in 0..crate::state::MAX_TEXTURE_UNITS {
        let tex_id = ctx.bound_textures[unit];
        if tex_id == 0 { continue; }
        let tex = match ctx.textures.get(tex_id) {
            Some(t) if !t.data.is_empty() && t.width > 0 => t,
            _ => continue,
        };
        let data_ptr = tex.data.as_ptr() as *const u8;
        let data_len = (tex.data.len() * 4) as u32;
        let res_id = (upload_fn)(tex_id, data_ptr, data_len, tex.width, tex.height);
        if res_id != 0 {
            (bind_fn)(unit as u32, res_id);
        }
    }
}

/// Upload all uniform values to the hardware driver as a constant buffer.
///
/// Packs uniforms into a contiguous vec4 array matching the TGSI slot layout:
/// mat4 = 4 consecutive vec4 slots, mat3 = 3 slots, everything else = 1 slot.
/// Slot offsets are cumulative (same order as the compiler assigns LoadUniform indices).
fn upload_uniforms(ctx: &GlContext, drv: &GpuDrv) {
    let prog_id = ctx.current_program;
    let program = match ctx.shaders.get_program(prog_id) {
        Some(p) if p.linked => p,
        _ => return,
    };
    if program.uniforms.is_empty() { return; }

    // Compute cumulative slot offsets matching the TGSI backend assignment.
    // mat4 = 4 vec4 slots, mat3 = 3 vec4 slots (padded), all others = 1 slot.
    let mut slot_offsets: alloc::vec::Vec<u32> = alloc::vec::Vec::with_capacity(program.uniforms.len());
    let mut total_slots = 0u32;
    for u in &program.uniforms {
        slot_offsets.push(total_slots);
        total_slots += match u.size {
            16 => 4,
            9  => 3,
            _  => 1,
        };
    }

    // Build contiguous float buffer (total_slots * 4 floats)
    let mut buf: alloc::vec::Vec<f32> = alloc::vec![0.0; total_slots as usize * 4];
    for (i, u) in program.uniforms.iter().enumerate() {
        let base = slot_offsets[i] as usize * 4;
        match u.size {
            16 => {
                for j in 0..16 {
                    if base + j < buf.len() { buf[base + j] = u.value[j]; }
                }
            }
            9 => {
                for row in 0..3 {
                    for col in 0..3 {
                        let idx = base + row * 4 + col;
                        if idx < buf.len() { buf[idx] = u.value[row * 3 + col]; }
                    }
                }
            }
            _ => {
                for j in 0..4 {
                    if base + j < buf.len() { buf[base + j] = u.value[j]; }
                }
            }
        }
    }

    (drv.drv_set_uniform_f32)(0, total_slots, buf.as_ptr());
}

/// Hardware-accelerated draw arrays via loaded .drv.
fn draw_arrays_hw(ctx: &mut GlContext, mode: GLenum, first: GLint, count: GLsizei) {
    if count <= 0 { return; }

    let drv = match drv_loader::drv() {
        Some(d) => d,
        None => {
            rasterizer::draw(ctx, mode, first, count);
            return;
        }
    };

    let prog_id = ctx.current_program;
    let program = match ctx.shaders.get_program(prog_id) {
        Some(p) if p.linked => p,
        _ => return,
    };

    // Collect vertex data into a contiguous buffer
    let attrib_count = program.attributes.len();
    let vertex_stride = (attrib_count * 4 * 4) as u32; // 4 floats * 4 bytes per attrib

    let mut vertex_data: alloc::vec::Vec<f32> = alloc::vec::Vec::with_capacity(
        (count as usize) * attrib_count * 4
    );
    for vi in first..(first + count) {
        for attr in &program.attributes {
            let loc = attr.location as usize;
            if loc < ctx.attribs.len() && ctx.attribs[loc].enabled {
                let va = &ctx.attribs[loc];
                let fetched = crate::rasterizer::vertex::fetch_single_attribute(
                    ctx, va.size, va.typ, va.stride, va.offset, va.buffer_id, vi as u32,
                );
                vertex_data.extend_from_slice(&fetched);
            } else {
                vertex_data.extend_from_slice(&[0.0, 0.0, 0.0, 1.0]);
            }
        }
    }

    // Build DrvAttrib descriptors
    let mut attribs: alloc::vec::Vec<DrvAttrib> = alloc::vec::Vec::with_capacity(attrib_count);
    for (ai, attr) in program.attributes.iter().enumerate() {
        attribs.push(DrvAttrib {
            location: attr.location as u32,
            components: 4,
            attr_type: GL_FLOAT,
            offset: (ai * 16) as u32,
            normalized: 0,
        });
    }

    let vb_bytes = unsafe {
        core::slice::from_raw_parts(vertex_data.as_ptr() as *const u8, vertex_data.len() * 4)
    };

    // During shadow pass: skip texture binding, upload per-object MVP for depth shader
    if ctx.shadow_pass_active {
        // The app sets loc_mvp (uniform 0) = light_vp * model per object.
        // Upload that as CONST[0..3] for the depth-only shader.
        let prog_id = ctx.current_program;
        if let Some(p) = ctx.shaders.get_program(prog_id) {
            if let Some(u) = p.uniforms.first() {
                // Uniform 0 = MVP (mat4 = 16 floats = 4 vec4 slots)
                (drv.drv_set_uniform_f32)(0, 4, u.value.as_ptr());
            }
        }
    } else {
        bind_textures_hw(ctx, drv);
        upload_uniforms(ctx, drv);
    }

    (drv.drv_draw_arrays)(
        mode, first as u32, count as u32,
        vb_bytes.as_ptr(), vertex_stride,
        attribs.as_ptr(), attribs.len() as u32,
    );
}

/// Hardware-accelerated indexed draw via loaded .drv.
fn draw_elements_hw(
    ctx: &mut GlContext,
    mode: GLenum,
    count: GLsizei,
    type_: GLenum,
    offset: usize,
) {
    if count <= 0 { return; }

    let drv = match drv_loader::drv() {
        Some(d) => d,
        None => {
            rasterizer::draw_elements(ctx, mode, count, type_, offset);
            return;
        }
    };

    let prog_id = ctx.current_program;
    let program = match ctx.shaders.get_program(prog_id) {
        Some(p) if p.linked => p,
        _ => return,
    };

    // Read index buffer
    let ebo_id = ctx.bound_element_buffer;
    let index_data: &[u8] = if ebo_id != 0 {
        if let Some(buf) = ctx.buffers.get(ebo_id) {
            &buf.data
        } else {
            return;
        }
    } else {
        return;
    };

    // Find max index to know how many vertices to fetch
    let mut max_index: u32 = 0;
    for i in 0..count as usize {
        let idx = crate::rasterizer::read_index(index_data, type_, offset, i);
        if idx > max_index { max_index = idx; }
    }

    let attrib_count = program.attributes.len();
    let vertex_stride = (attrib_count * 4 * 4) as u32;
    let num_verts = (max_index + 1) as usize;

    // Fetch all referenced vertices
    let mut vertex_data: alloc::vec::Vec<f32> = alloc::vec::Vec::with_capacity(
        num_verts * attrib_count * 4
    );
    for vi in 0..num_verts {
        for attr in &program.attributes {
            let loc = attr.location as usize;
            if loc < ctx.attribs.len() && ctx.attribs[loc].enabled {
                let va = &ctx.attribs[loc];
                let fetched = crate::rasterizer::vertex::fetch_single_attribute(
                    ctx, va.size, va.typ, va.stride, va.offset, va.buffer_id, vi as u32,
                );
                vertex_data.extend_from_slice(&fetched);
            } else {
                vertex_data.extend_from_slice(&[0.0, 0.0, 0.0, 1.0]);
            }
        }
    }

    // Build DrvAttrib descriptors
    let mut attribs: alloc::vec::Vec<DrvAttrib> = alloc::vec::Vec::with_capacity(attrib_count);
    for (ai, attr) in program.attributes.iter().enumerate() {
        attribs.push(DrvAttrib {
            location: attr.location as u32,
            components: 4,
            attr_type: GL_FLOAT,
            offset: (ai * 16) as u32,
            normalized: 0,
        });
    }

    let vb_bytes = unsafe {
        core::slice::from_raw_parts(vertex_data.as_ptr() as *const u8, vertex_data.len() * 4)
    };

    // Pass index data starting at the offset
    let index_start = &index_data[offset..];

    if ctx.shadow_pass_active {
        let prog_id = ctx.current_program;
        if let Some(p) = ctx.shaders.get_program(prog_id) {
            if let Some(u) = p.uniforms.first() {
                (drv.drv_set_uniform_f32)(0, 4, u.value.as_ptr());
            }
        }
    } else {
        bind_textures_hw(ctx, drv);
        upload_uniforms(ctx, drv);
    }

    (drv.drv_draw_elements)(
        mode, count as u32, type_,
        index_start.as_ptr(),
        vb_bytes.as_ptr(), vertex_stride,
        attribs.as_ptr(), attribs.len() as u32,
    );
}
