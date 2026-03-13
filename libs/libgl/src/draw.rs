//! Draw command dispatch.
//!
//! Dispatches draw calls to either a loaded hardware driver (.drv) or the
//! software rasterizer, depending on whether a driver is active.

use crate::state::GlContext;
use crate::types::*;
use crate::rasterizer;
use crate::drv_loader::{self, DrvAttrib};

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

    (drv.drv_draw_elements)(
        mode, count as u32, type_,
        index_start.as_ptr(),
        vb_bytes.as_ptr(), vertex_stride,
        attribs.as_ptr(), attribs.len() as u32,
    );
}
