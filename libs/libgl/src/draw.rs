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
    let mut slot_offsets: alloc::vec::Vec<u32> = alloc::vec::Vec::with_capacity(prog.uniforms.len());
    let mut total_slots = 0u32;
    for u in &prog.uniforms {
        slot_offsets.push(total_slots);
        total_slots += match u.size {
            16 => 4,
            9 => 3,
            _ => 1,
        };
    }

    let mut slot_to_sampler_unit: alloc::vec::Vec<Option<u32>> =
        alloc::vec![None; total_slots as usize];
    for (i, u) in prog.uniforms.iter().enumerate() {
        let base = slot_offsets[i] as usize;
        let slots = match u.size {
            16 => 4,
            9 => 3,
            _ => 1,
        };
        if u.name.contains("Texture") || u.name.contains("ShadowMap") || u.name.contains("Sampler") {
            for j in 0..slots {
                if base + j < slot_to_sampler_unit.len() {
                    slot_to_sampler_unit[base + j] = Some(u.sampler_unit.max(0) as u32);
                }
            }
        }
    }

    let mut reg_to_sampler_unit: alloc::vec::Vec<Option<u32>> =
        alloc::vec![None; fs_ir.num_regs as usize];
    let mut seen_sampler_regs: alloc::vec::Vec<u32> = alloc::vec::Vec::new();

    for inst in &fs_ir.instructions {
        match inst {
            crate::compiler::ir::Inst::LoadUniform(dst, idx) => {
                if let Some(unit) = slot_to_sampler_unit.get(*idx as usize).and_then(|u| *u) {
                    if (*dst as usize) < reg_to_sampler_unit.len() {
                        reg_to_sampler_unit[*dst as usize] = Some(unit);
                    }
                }
            }
            crate::compiler::ir::Inst::Mov(dst, src)
            | crate::compiler::ir::Inst::Swizzle(dst, src, _, _)
            | crate::compiler::ir::Inst::WriteMask(dst, src, _) => {
                if (*dst as usize) < reg_to_sampler_unit.len() && (*src as usize) < reg_to_sampler_unit.len() {
                    reg_to_sampler_unit[*dst as usize] = reg_to_sampler_unit[*src as usize];
                }
            }
            crate::compiler::ir::Inst::TexSample(_, sampler_reg, _) => {
                if seen_sampler_regs.iter().any(|r| r == sampler_reg) {
                    continue;
                }
                let tgsi_slot = seen_sampler_regs.len() as u32;
                seen_sampler_regs.push(*sampler_reg);

                let gl_unit = reg_to_sampler_unit
                    .get(*sampler_reg as usize)
                    .and_then(|u| *u)
                    .unwrap_or(tgsi_slot);
                let tex_unit = gl_unit as usize;
                if tex_unit >= crate::state::MAX_TEXTURE_UNITS {
                    continue;
                }

                let tex_id = ctx.bound_textures[tex_unit];
                if tex_id == 0 {
                    continue;
                }
                if tex_id == ctx.shadow_depth_tex_id {
                    if let Some(bind_shadow) = drv.drv_shadow_bind {
                        bind_shadow(tgsi_slot);
                    }
                    continue;
                }

                let tex = match ctx.textures.get(tex_id) {
                    Some(t) if !t.data.is_empty() && t.width > 0 => t,
                    _ => continue,
                };
                let data_ptr = tex.data.as_ptr() as *const u8;
                let data_len = (tex.data.len() * 4) as u32;
                let res_id = (upload_fn)(tex_id, data_ptr, data_len, tex.width, tex.height);
                if res_id != 0 {
                    (bind_fn)(tgsi_slot, res_id);
                }
            }
            _ => {}
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

    // Upload state
    bind_textures_hw(ctx, drv);
    upload_uniforms(ctx, drv);

    // Try persistent VBO path: if all attribs reference the same VBO and
    // the VBO has matching raw data layout, use the GPU-resident buffer directly.
    let attrib_count = program.attributes.len();
    if false && attrib_count > 0 { // persistent VBO disabled until rendering issues resolved
        let first_loc = program.attributes[0].location as usize;
        if first_loc < ctx.attribs.len() && ctx.attribs[first_loc].enabled {
            let vbo_id = ctx.attribs[first_loc].buffer_id;
            let stride = ctx.attribs[first_loc].stride as u32;

            // Check if all attribs use the same VBO
            let all_same_vbo = vbo_id != 0 && stride > 0 && program.attributes.iter().all(|a| {
                let loc = a.location as usize;
                loc < ctx.attribs.len() && ctx.attribs[loc].enabled
                    && ctx.attribs[loc].buffer_id == vbo_id
            });

            if all_same_vbo {
                if let (Some(upload_fn), Some(draw_fn)) = (drv.drv_upload_vbo, drv.drv_draw_vbo) {
                    // Ensure GPU resource exists for this VBO
                    let buf = ctx.buffers.get_mut(vbo_id);
                    if let Some(buf) = buf {
                        if buf.gpu_res_id == 0 && !buf.data.is_empty() {
                            // Upload once
                            let res = upload_fn(buf.data.as_ptr(), buf.data.len() as u32, stride);
                            buf.gpu_res_id = res;
                            buf.gpu_size = buf.data.len() as u32;
                        }

                        let gpu_res = buf.gpu_res_id;
                        static mut VBO_DBG: u32 = 0;
                        unsafe {
                            if VBO_DBG < 3 {
                                VBO_DBG += 1;
                                crate::serial_println!("[libgl] persistent VBO: id={} gpu_res={} size={} count={}",
                                    vbo_id, gpu_res, buf.gpu_size, count);
                            }
                        }
                        if gpu_res != 0 {
                            // Build attrib descriptors using original offsets from the VBO
                            let mut attribs: alloc::vec::Vec<DrvAttrib> = alloc::vec::Vec::with_capacity(attrib_count);
                            for attr in &program.attributes {
                                let loc = attr.location as usize;
                                let va = &ctx.attribs[loc];
                                attribs.push(DrvAttrib {
                                    location: attr.location as u32,
                                    components: va.size as u32,
                                    attr_type: va.typ,
                                    offset: va.offset as u32,
                                    normalized: if va.normalized { 1 } else { 0 },
                                });
                            }

                            draw_fn(mode, first as u32, count as u32, gpu_res, stride,
                                attribs.as_ptr(), attribs.len() as u32);
                            return;
                        }
                    }
                }
            }
        }
    }

    // Fallback: if all attribs come from the same VBO with interleaved layout,
    // pass the raw VBO data directly (no per-vertex copy). Much faster.
    //
    // Keep this path restricted to vec4-only attributes for now. Virgl has
    // been reliable with the packed vec4 fallback used by indexed draws, but
    // has shown rendering corruption with mixed FLOAT3/FLOAT2 vertex element
    // layouts on glDrawArrays() callers such as the GL demo floor plane.
    let first_loc = if attrib_count > 0 { program.attributes[0].location as usize } else { 0 };
    let vbo_id = if first_loc < ctx.attribs.len() && ctx.attribs[first_loc].enabled {
        ctx.attribs[first_loc].buffer_id
    } else { 0 };
    let raw_stride = if first_loc < ctx.attribs.len() { ctx.attribs[first_loc].stride } else { 0 };

    let all_same_vbo = vbo_id != 0 && raw_stride > 0 && attrib_count > 0
        && program.attributes.iter().all(|a| {
            let loc = a.location as usize;
            loc < ctx.attribs.len() && ctx.attribs[loc].enabled
                && ctx.attribs[loc].buffer_id == vbo_id
        });
    let all_vec4_attribs = attrib_count > 0 && program.attributes.iter().all(|a| {
        let loc = a.location as usize;
        loc < ctx.attribs.len() && ctx.attribs[loc].size == 4 && ctx.attribs[loc].typ == GL_FLOAT
    });

    if all_same_vbo && all_vec4_attribs {
        // Fast path: pass raw interleaved VBO data with real offsets
        let buf = ctx.buffers.get(vbo_id);
        if let Some(buf) = buf {
            let stride = raw_stride as u32;
            let start_byte = first as usize * stride as usize;
            let end_byte = (first + count) as usize * stride as usize;
            if end_byte <= buf.data.len() {
                let vb_bytes = &buf.data[start_byte..end_byte];

                let mut attribs: alloc::vec::Vec<DrvAttrib> = alloc::vec::Vec::with_capacity(attrib_count);
                for attr in &program.attributes {
                    let loc = attr.location as usize;
                    let va = &ctx.attribs[loc];
                    attribs.push(DrvAttrib {
                        location: attr.location as u32,
                        components: va.size as u32,
                        attr_type: va.typ,
                        offset: va.offset as u32,
                        normalized: if va.normalized { 1 } else { 0 },
                    });
                }

                (drv.drv_draw_arrays)(
                    mode, 0, count as u32,
                    vb_bytes.as_ptr(), stride,
                    attribs.as_ptr(), attribs.len() as u32,
                );
                return;
            }
        }
    }

    // Slow fallback: collect vertex data per-attribute (for multi-VBO or non-interleaved)
    let vertex_stride = (attrib_count * 4 * 4) as u32;

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

    bind_textures_hw(ctx, drv);
    upload_uniforms(ctx, drv);

    (drv.drv_draw_elements)(
        mode, count as u32, type_,
        index_start.as_ptr(),
        vb_bytes.as_ptr(), vertex_stride,
        attribs.as_ptr(), attribs.len() as u32,
    );
}
