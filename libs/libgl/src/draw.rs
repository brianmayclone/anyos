//! Draw command dispatch.
//!
//! Dispatches draw calls to either the SVGA3D hardware backend or the
//! software rasterizer, depending on whether SVGA3D is active.

use alloc::vec::Vec;
use crate::state::GlContext;
use crate::types::*;
use crate::rasterizer;

/// Execute glDrawArrays.
pub fn draw_arrays(ctx: &mut GlContext, mode: GLenum, first: GLint, count: GLsizei) {
    if unsafe { crate::USE_HW_BACKEND } {
        draw_arrays_hw(ctx, mode, first, count);
    }
    // Always run SW rasterizer for display output.
    // GPU readback (SURFACE_DMA from render target) is not yet implemented,
    // so the compositor still needs the SW framebuffer for compositing.
    rasterizer::draw(ctx, mode, first, count);
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
        // For now, fall back to software for indexed drawing
        // (vertex buffer DMA for indexed access needs additional work)
        rasterizer::draw_elements(ctx, mode, count, type_, offset);
    } else {
        rasterizer::draw_elements(ctx, mode, count, type_, offset);
    }
}

// ── Hardware draw path ──────────────────────────────────

/// Map GL primitive mode to SVGA3D primitive type.
fn gl_mode_to_svga3d(mode: GLenum) -> Option<u32> {
    match mode {
        GL_TRIANGLES => Some(crate::svga3d::SVGA3D_PRIMITIVE_TRIANGLELIST),
        GL_TRIANGLE_STRIP => Some(crate::svga3d::SVGA3D_PRIMITIVE_TRIANGLESTRIP),
        GL_TRIANGLE_FAN => Some(crate::svga3d::SVGA3D_PRIMITIVE_TRIANGLEFAN),
        _ => None,
    }
}

/// Map GL depth function to SVGA3D comparison function.
fn gl_depth_func_to_svga3d(func: GLenum) -> u32 {
    use crate::svga3d::*;
    match func {
        GL_NEVER => SVGA3D_CMP_NEVER,
        GL_LESS => SVGA3D_CMP_LESS,
        GL_EQUAL => SVGA3D_CMP_EQUAL,
        GL_LEQUAL => SVGA3D_CMP_LESSEQUAL,
        GL_GREATER => SVGA3D_CMP_GREATER,
        GL_NOTEQUAL => SVGA3D_CMP_NOTEQUAL,
        GL_GEQUAL => SVGA3D_CMP_GREATEREQUAL,
        GL_ALWAYS => SVGA3D_CMP_ALWAYS,
        _ => SVGA3D_CMP_LESS,
    }
}

/// Map GL blend factor to SVGA3D blend factor.
fn gl_blend_to_svga3d(factor: GLenum) -> u32 {
    use crate::svga3d::*;
    match factor {
        GL_ZERO => SVGA3D_BLEND_ZERO,
        GL_ONE => SVGA3D_BLEND_ONE,
        GL_SRC_COLOR => SVGA3D_BLEND_SRCCOLOR,
        GL_ONE_MINUS_SRC_COLOR => SVGA3D_BLEND_INVSRCCOLOR,
        GL_SRC_ALPHA => SVGA3D_BLEND_SRCALPHA,
        GL_ONE_MINUS_SRC_ALPHA => SVGA3D_BLEND_INVSRCALPHA,
        GL_DST_ALPHA => SVGA3D_BLEND_DESTALPHA,
        GL_ONE_MINUS_DST_ALPHA => SVGA3D_BLEND_INVDESTALPHA,
        GL_DST_COLOR => SVGA3D_BLEND_DESTCOLOR,
        GL_ONE_MINUS_DST_COLOR => SVGA3D_BLEND_INVDESTCOLOR,
        _ => SVGA3D_BLEND_ONE,
    }
}

/// Map GL cull face mode to SVGA3D cull mode.
fn gl_cull_to_svga3d(cull_face: bool, mode: GLenum) -> u32 {
    use crate::svga3d::*;
    if !cull_face {
        SVGA3D_CULL_NONE
    } else {
        match mode {
            GL_FRONT => SVGA3D_CULL_FRONT,
            GL_BACK => SVGA3D_CULL_BACK,
            _ => SVGA3D_CULL_BACK,
        }
    }
}

/// Hardware-accelerated draw arrays via SVGA3D.
fn draw_arrays_hw(ctx: &mut GlContext, mode: GLenum, first: GLint, count: GLsizei) {
    use crate::svga3d::*;
    use crate::compiler::backend_dx9;

    static mut DRAW_DBG: u32 = 0;
    if count <= 0 { return; }
    let dbg = unsafe { DRAW_DBG < 3 };
    if dbg {
        crate::serial_println!("[libgl] draw_arrays_hw: mode={} first={} count={}", mode, first, count);
    }

    let prim_type = match gl_mode_to_svga3d(mode) {
        Some(pt) => pt,
        None => {
            // Unsupported primitive, fall back to software
            rasterizer::draw(ctx, mode, first, count);
            return;
        }
    };

    let svga = match unsafe { crate::SVGA3D.as_mut() } {
        Some(s) => s,
        None => return,
    };

    let prog_id = ctx.current_program;
    let program = match ctx.shaders.get_program(prog_id) {
        Some(p) if p.linked => p,
        _ => return,
    };

    // 1. Compile shaders to DX9 bytecode
    let vs_ir = match &program.vs_ir {
        Some(ir) => ir,
        None => return,
    };
    let fs_ir = match &program.fs_ir {
        Some(ir) => ir,
        None => return,
    };

    let (vs_bytecode, vs_consts) = backend_dx9::compile(vs_ir, true);
    let (fs_bytecode, fs_consts) = backend_dx9::compile(fs_ir, false);

    // 2. Allocate and upload shaders
    let vs_id = svga.alloc_shader();
    let fs_id = svga.alloc_shader();

    svga.cmd.shader_define(svga.context_id, vs_id, SVGA3D_SHADERTYPE_VS, &vs_bytecode);
    svga.cmd.shader_define(svga.context_id, fs_id, SVGA3D_SHADERTYPE_PS, &fs_bytecode);
    svga.cmd.set_shader(svga.context_id, SVGA3D_SHADERTYPE_VS, vs_id);
    svga.cmd.set_shader(svga.context_id, SVGA3D_SHADERTYPE_PS, fs_id);

    // 3. Upload uniforms as shader constants
    let uniforms = rasterizer::collect_uniforms(program);
    for (i, u) in uniforms.iter().enumerate() {
        svga.cmd.set_shader_const_f(svga.context_id, i as u32, SVGA3D_SHADERTYPE_VS, u);
        svga.cmd.set_shader_const_f(svga.context_id, i as u32, SVGA3D_SHADERTYPE_PS, u);
    }

    // Upload inline constants (from LoadConst instructions) to VS
    for &(creg, vals) in &vs_consts {
        svga.cmd.set_shader_const_f(svga.context_id, creg, SVGA3D_SHADERTYPE_VS, &vals);
    }
    // Upload inline constants to PS
    for &(creg, vals) in &fs_consts {
        svga.cmd.set_shader_const_f(svga.context_id, creg, SVGA3D_SHADERTYPE_PS, &vals);
    }

    // 4. Set render states from GL context
    let cid = svga.context_id;
    svga.cmd.set_render_states(cid, &[
        (SVGA3D_RS_ZENABLE, ctx.depth_test as u32),
        (SVGA3D_RS_ZWRITEENABLE, ctx.depth_mask as u32),
        (SVGA3D_RS_ZFUNC, gl_depth_func_to_svga3d(ctx.depth_func)),
        (SVGA3D_RS_BLENDENABLE, ctx.blend as u32),
        (SVGA3D_RS_SRCBLEND, gl_blend_to_svga3d(ctx.blend_src_rgb)),
        (SVGA3D_RS_DSTBLEND, gl_blend_to_svga3d(ctx.blend_dst_rgb)),
        (SVGA3D_RS_CULLMODE, gl_cull_to_svga3d(ctx.cull_face, ctx.cull_face_mode)),
    ]);

    // 5. Create a vertex buffer surface and upload vertex data via DMA
    //
    // Collect vertex data into a tightly-packed interleaved buffer.
    // For each vertex: attribute0[float4] + attribute1[float4] + ...
    let attrib_count = program.attributes.len();
    let vertex_stride = (attrib_count * 4 * 4) as u32; // 4 floats * 4 bytes per attrib
    let total_bytes = vertex_stride * count as u32;

    // Pack vertex data into a contiguous buffer
    let mut vertex_data: Vec<f32> = Vec::with_capacity((count as usize) * attrib_count * 4);
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

    // Create a vertex buffer surface
    let vb_sid = svga.alloc_surface();
    let vb_width_pixels = total_bytes / 4; // X8R8G8B8 = 4 bytes per pixel
    svga.cmd.surface_define(vb_sid, SVGA3D_SURFACE_HINT_VERTEXBUFFER, SVGA3D_X8R8G8B8, vb_width_pixels, 1);

    // Submit surface definition before DMA (kernel DMA needs the surface to exist)
    svga.cmd.submit();

    // DMA vertex data to the surface via kernel GMR
    let vb_bytes: &[u8] = unsafe {
        core::slice::from_raw_parts(
            vertex_data.as_ptr() as *const u8,
            vertex_data.len() * 4,
        )
    };
    let dma_result = crate::syscall::gpu_3d_surface_dma(vb_sid, vb_bytes, vb_width_pixels, 1);
    if dbg {
        crate::serial_println!("[libgl] VB DMA: sid={} bytes={} w={} result={}", vb_sid, vb_bytes.len(), vb_width_pixels, dma_result);
    }
    if dma_result != 0 {
        // DMA failed — clean up and bail
        svga.cmd.surface_destroy(vb_sid);
        svga.cmd.submit();
        return;
    }
    // Build vertex declaration array
    let mut vertex_decl_words: Vec<u32> = Vec::new();
    for (ai, attr) in program.attributes.iter().enumerate() {
        let offset_in_vertex = (ai * 16) as u32; // 4 floats * 4 bytes = 16 bytes per attribute

        // SVGA3dVertexDecl: {
        //   identity { type, method, usage, usageIndex },
        //   array { surfaceId, offset, stride },
        //   rangeHint { first, last }
        // }
        // = 9 u32s per declaration
        vertex_decl_words.push(SVGA3D_DECLTYPE_FLOAT4); // type
        vertex_decl_words.push(0);                       // method (default)
        vertex_decl_words.push(if ai == 0 { SVGA3D_DECLUSAGE_POSITION } else { SVGA3D_DECLUSAGE_TEXCOORD }); // usage
        vertex_decl_words.push(if ai == 0 { 0 } else { (ai - 1) as u32 }); // usageIndex
        vertex_decl_words.push(vb_sid);                  // array.surfaceId
        vertex_decl_words.push(offset_in_vertex);        // array.offset
        vertex_decl_words.push(vertex_stride);           // array.stride
        vertex_decl_words.push(0);                       // rangeHint.first
        vertex_decl_words.push((count - 1) as u32);      // rangeHint.last
    }

    // Build primitive range
    let prim_count = match prim_type {
        SVGA3D_PRIMITIVE_TRIANGLELIST => (count / 3) as u32,
        SVGA3D_PRIMITIVE_TRIANGLESTRIP | SVGA3D_PRIMITIVE_TRIANGLEFAN => (count - 2) as u32,
        _ => count as u32,
    };

    // SVGA3dPrimitiveRange: {
    //   primType, primitiveCount,
    //   indexArray { surfaceId, offset, stride },
    //   indexWidth, indexBias
    // }
    // = 7 u32s
    let prim_range_words = [
        prim_type,
        prim_count,
        0, // indexArray.surfaceId (0 = no index buffer, sequential)
        0, // indexArray.offset
        0, // indexArray.stride
        0, // indexWidth (0 = non-indexed)
        0, // indexBias
    ];

    svga.cmd.draw_primitives(
        cid,
        attrib_count as u32,
        1, // 1 primitive range
        &vertex_decl_words,
        &prim_range_words,
    );

    // Submit all commands
    let draw_result = svga.cmd.submit();
    if dbg {
        crate::serial_println!("[libgl] DRAW_PRIMITIVES: attribs={} prims={} submit={}", attrib_count, prim_count, draw_result);
        unsafe { DRAW_DBG += 1; }
    }

    // Clean up: destroy vertex buffer and shaders
    svga.cmd.surface_destroy(vb_sid);
    svga.cmd.shader_destroy(cid, vs_id, SVGA3D_SHADERTYPE_VS);
    svga.cmd.shader_destroy(cid, fs_id, SVGA3D_SHADERTYPE_PS);
    svga.cmd.set_shader(cid, SVGA3D_SHADERTYPE_VS, 0); // unbind
    svga.cmd.set_shader(cid, SVGA3D_SHADERTYPE_PS, 0);
    svga.cmd.submit();
}
