//! SVGA3D GPU driver (.drv) — translates generic drv_* calls into SVGA3D
//! command buffer submissions via SYS_GPU_3D_SUBMIT.
//!
//! Loaded at runtime by libgl's drv_loader when the kernel reports
//! gpu_type = "svga3d".

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::slice;

// ── Heap allocator (required for alloc in no_std .so) ────────────────────

libheap::dll_allocator!(libsyscall::sbrk, libsyscall::mmap, libsyscall::munmap);

// ── SVGA3D constants ─────────────────────────────────────────────────────

const CMD_SURFACE_DEFINE: u32     = 1040;
const CMD_SURFACE_DESTROY: u32    = 1041;
const CMD_CONTEXT_DEFINE: u32     = 1045;
const CMD_CONTEXT_DESTROY: u32    = 1046;
const CMD_SETRENDERSTATE: u32     = 1049;
const CMD_SETRENDERTARGET: u32    = 1050;
const CMD_SETVIEWPORT: u32        = 1055;
const CMD_CLEAR: u32              = 1057;
const CMD_PRESENT: u32            = 1058;
const CMD_SHADER_DEFINE: u32      = 1059;
const CMD_SHADER_DESTROY: u32     = 1060;
const CMD_SET_SHADER: u32         = 1061;
const CMD_SET_SHADER_CONST: u32   = 1062;
const CMD_DRAW_PRIMITIVES: u32    = 1063;

// Surface formats
const SVGA3D_A8R8G8B8: u32 = 2;
const SVGA3D_Z_D24S8: u32  = 20;

// Surface flags
const SVGA3D_SURFACE_HINT_RENDERTARGET: u32 = 1 << 4;
const SVGA3D_SURFACE_HINT_DEPTHSTENCIL: u32 = 1 << 5;

// Render target types
const SVGA3D_RT_COLOR0: u32 = 0;
const SVGA3D_RT_DEPTH: u32  = 1;

// Render state IDs
const SVGA3D_RS_ZENABLE: u32         = 2;
const SVGA3D_RS_ZWRITEENABLE: u32    = 14;
const SVGA3D_RS_SRCBLEND: u32        = 19;
const SVGA3D_RS_DSTBLEND: u32        = 20;
const SVGA3D_RS_ZFUNC: u32           = 23;
const SVGA3D_RS_BLENDENABLE: u32     = 27;
const SVGA3D_RS_COLORWRITEENABLE: u32 = 168;

// Shader types
const SVGA3D_SHADERTYPE_VS: u32 = 1;
const SVGA3D_SHADERTYPE_PS: u32 = 2;

// Shader constant type
const SVGA3D_CONST_TYPE_FLOAT: u32 = 0;

// Clear flags
const SVGA3D_CLEAR_COLOR: u32   = 1;
const SVGA3D_CLEAR_DEPTH: u32   = 2;
const SVGA3D_CLEAR_STENCIL: u32 = 4;

// Primitive types
const SVGA3D_PRIMITIVE_TRIANGLELIST: u32  = 5;
const SVGA3D_PRIMITIVE_TRIANGLESTRIP: u32 = 6;
const SVGA3D_PRIMITIVE_TRIANGLEFAN: u32   = 7;

// Vertex declaration types
const SVGA3D_DECLTYPE_FLOAT1: u32 = 0;
const SVGA3D_DECLTYPE_FLOAT2: u32 = 1;
const SVGA3D_DECLTYPE_FLOAT3: u32 = 2;
const SVGA3D_DECLTYPE_FLOAT4: u32 = 3;

// ── Attrib descriptor (matches drv_loader::DrvAttrib layout) ─────────────

#[repr(C)]
struct DrvAttrib {
    location: u32,
    components: u32,
    attr_type: u32,
    offset: u32,
    normalized: u32,
}

// ── Driver state ─────────────────────────────────────────────────────────

struct Svga3dState {
    cmd: Vec<u32>,
    context_id: u32,
    color_sid: u32,
    depth_sid: u32,
    next_sid: u32,
    next_shid: u32,
    width: u32,
    height: u32,
    // Current shader bindings
    current_vs: u32,
    current_ps: u32,
}

static mut STATE: Option<Svga3dState> = None;

fn state() -> &'static mut Svga3dState {
    unsafe { STATE.as_mut().unwrap() }
}

// ── Command buffer helpers ───────────────────────────────────────────────

fn push_cmd(words: &mut Vec<u32>, cmd_id: u32, payload: &[u32]) {
    words.push(cmd_id);
    words.push((payload.len() * 4) as u32);
    words.extend_from_slice(payload);
}

fn submit(words: &mut Vec<u32>) -> u32 {
    if words.is_empty() { return 0; }
    let result = libsyscall::gpu_3d_submit(words);
    words.clear();
    result
}

// ── Exported drv_* API ───────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn drv_init(width: u32, height: u32) -> u32 {
    let mut cmd = Vec::with_capacity(512);
    let context_id = 1u32;
    let color_sid = 1u32;
    let depth_sid = 2u32;

    // Create context
    push_cmd(&mut cmd, CMD_CONTEXT_DEFINE, &[context_id]);

    // Create color render target
    push_cmd(&mut cmd, CMD_SURFACE_DEFINE, &[
        color_sid, SVGA3D_SURFACE_HINT_RENDERTARGET, SVGA3D_A8R8G8B8,
        1, 0, 0, 0, 0, 0, // face[0].numMipLevels=1, rest=0
        width, height, 1,
    ]);

    // Create depth buffer
    push_cmd(&mut cmd, CMD_SURFACE_DEFINE, &[
        depth_sid, SVGA3D_SURFACE_HINT_DEPTHSTENCIL, SVGA3D_Z_D24S8,
        1, 0, 0, 0, 0, 0,
        width, height, 1,
    ]);

    // Bind render targets
    push_cmd(&mut cmd, CMD_SETRENDERTARGET, &[context_id, SVGA3D_RT_COLOR0, color_sid, 0, 0]);
    push_cmd(&mut cmd, CMD_SETRENDERTARGET, &[context_id, SVGA3D_RT_DEPTH, depth_sid, 0, 0]);

    // Set viewport
    push_cmd(&mut cmd, CMD_SETVIEWPORT, &[
        context_id,
        0f32.to_bits(), 0f32.to_bits(),
        (width as f32).to_bits(), (height as f32).to_bits(),
        0f32.to_bits(), 1f32.to_bits(),
    ]);

    // Default render states
    push_cmd(&mut cmd, CMD_SETRENDERSTATE, &[context_id, SVGA3D_RS_COLORWRITEENABLE, 0xF]);

    let result = submit(&mut cmd);
    if result != 0 { return 0; }

    unsafe {
        STATE = Some(Svga3dState {
            cmd,
            context_id,
            color_sid,
            depth_sid,
            next_sid: 3, // 1=color, 2=depth
            next_shid: 1,
            width,
            height,
            current_vs: 0,
            current_ps: 0,
        });
    }

    1 // success
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_deinit() {
    unsafe {
        if let Some(ref mut s) = STATE {
            push_cmd(&mut s.cmd, CMD_SURFACE_DESTROY, &[s.depth_sid]);
            push_cmd(&mut s.cmd, CMD_SURFACE_DESTROY, &[s.color_sid]);
            push_cmd(&mut s.cmd, CMD_CONTEXT_DESTROY, &[s.context_id]);
            submit(&mut s.cmd);
        }
        STATE = None;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_create_surface(format: u32, flags: u32, width: u32, height: u32) -> u32 {
    let s = state();
    let sid = s.next_sid;
    s.next_sid += 1;
    push_cmd(&mut s.cmd, CMD_SURFACE_DEFINE, &[
        sid, flags, format,
        1, 0, 0, 0, 0, 0,
        width, height, 1,
    ]);
    submit(&mut s.cmd);
    sid
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_destroy_surface(sid: u32) {
    let s = state();
    push_cmd(&mut s.cmd, CMD_SURFACE_DESTROY, &[sid]);
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_surface_upload(sid: u32, data: *const u8, len: u32, width: u32, height: u32) -> u32 {
    libsyscall::gpu_3d_surface_dma(sid, unsafe { slice::from_raw_parts(data, len as usize) }, width, height)
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_surface_download(sid: u32, data: *mut u8, len: u32, width: u32, height: u32) -> u32 {
    libsyscall::gpu_3d_surface_dma_read(sid, unsafe { slice::from_raw_parts_mut(data, len as usize) }, width, height)
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_create_shader(shader_type: u32, _version: u32, bytecode: *const u8, len: u32) -> u32 {
    let s = state();
    let shid = s.next_shid;
    s.next_shid += 1;

    let svga_type = if shader_type == 0 { SVGA3D_SHADERTYPE_VS } else { SVGA3D_SHADERTYPE_PS };

    // Bytecode is u32 words
    let word_count = (len as usize) / 4;
    let words = unsafe { slice::from_raw_parts(bytecode as *const u32, word_count) };

    let mut payload = Vec::with_capacity(3 + word_count);
    payload.push(s.context_id);
    payload.push(shid);
    payload.push(svga_type);
    payload.extend_from_slice(words);
    push_cmd(&mut s.cmd, CMD_SHADER_DEFINE, &payload);
    submit(&mut s.cmd);
    shid
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_destroy_shader(shid: u32) {
    let s = state();
    // Destroy both VS and PS with this ID (one will be a no-op)
    push_cmd(&mut s.cmd, CMD_SHADER_DESTROY, &[s.context_id, shid, SVGA3D_SHADERTYPE_VS]);
    push_cmd(&mut s.cmd, CMD_SHADER_DESTROY, &[s.context_id, shid, SVGA3D_SHADERTYPE_PS]);
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_link_program(vs_id: u32, ps_id: u32, _flags: u32) -> u32 {
    // SVGA3D doesn't have a "program" concept — shaders are set individually.
    // Return a synthetic program ID encoding both shader IDs.
    (vs_id << 16) | ps_id
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_use_program(program_id: u32) {
    let s = state();
    let vs_id = program_id >> 16;
    let ps_id = program_id & 0xFFFF;
    if vs_id != s.current_vs {
        push_cmd(&mut s.cmd, CMD_SET_SHADER, &[s.context_id, SVGA3D_SHADERTYPE_VS, vs_id]);
        s.current_vs = vs_id;
    }
    if ps_id != s.current_ps {
        push_cmd(&mut s.cmd, CMD_SET_SHADER, &[s.context_id, SVGA3D_SHADERTYPE_PS, ps_id]);
        s.current_ps = ps_id;
    }
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_viewport(x: u32, y: u32, w: u32, h: u32) {
    let s = state();
    push_cmd(&mut s.cmd, CMD_SETVIEWPORT, &[
        s.context_id,
        (x as f32).to_bits(), (y as f32).to_bits(),
        (w as f32).to_bits(), (h as f32).to_bits(),
        0f32.to_bits(), 1f32.to_bits(),
    ]);
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_blend(enable: u32, src_factor: u32, dst_factor: u32) {
    let s = state();
    push_cmd(&mut s.cmd, CMD_SETRENDERSTATE, &[s.context_id, SVGA3D_RS_BLENDENABLE, enable]);
    if enable != 0 {
        push_cmd(&mut s.cmd, CMD_SETRENDERSTATE, &[s.context_id, SVGA3D_RS_SRCBLEND, src_factor]);
        push_cmd(&mut s.cmd, CMD_SETRENDERSTATE, &[s.context_id, SVGA3D_RS_DSTBLEND, dst_factor]);
    }
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_depth_test(enable: u32, func: u32) {
    let s = state();
    push_cmd(&mut s.cmd, CMD_SETRENDERSTATE, &[s.context_id, SVGA3D_RS_ZENABLE, enable]);
    push_cmd(&mut s.cmd, CMD_SETRENDERSTATE, &[s.context_id, SVGA3D_RS_ZWRITEENABLE, enable]);
    if enable != 0 {
        push_cmd(&mut s.cmd, CMD_SETRENDERSTATE, &[s.context_id, SVGA3D_RS_ZFUNC, func]);
    }
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_clear(flags: u32, r: f32, g: f32, b: f32, a: f32, depth: f32) {
    let s = state();

    // Convert RGBA floats to ARGB u32 color
    let ri = (r * 255.0) as u32;
    let gi = (g * 255.0) as u32;
    let bi = (b * 255.0) as u32;
    let ai = (a * 255.0) as u32;
    let color = (ai << 24) | (ri << 16) | (gi << 8) | bi;

    let mut svga_flags = 0u32;
    if flags & 1 != 0 { svga_flags |= SVGA3D_CLEAR_COLOR; }
    if flags & 2 != 0 { svga_flags |= SVGA3D_CLEAR_DEPTH; }
    if flags & 4 != 0 { svga_flags |= SVGA3D_CLEAR_STENCIL; }

    // Clear with full-screen rect
    push_cmd(&mut s.cmd, CMD_CLEAR, &[
        s.context_id, svga_flags, color, depth.to_bits(), 0,
        0, 0, s.width, s.height,
    ]);
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_uniform_f32(location: i32, count: u32, values: *const f32) {
    let s = state();
    let vals = unsafe { slice::from_raw_parts(values, (count * 4) as usize) };

    // Set as VS constants (SVGA3D uses register-based constants, 4 floats per register)
    let num_regs = count;
    for i in 0..num_regs as usize {
        let reg = location as u32 + i as u32;
        let base = i * 4;
        let v = if base + 4 <= vals.len() {
            [vals[base], vals[base+1], vals[base+2], vals[base+3]]
        } else {
            let mut v = [0.0f32; 4];
            for j in 0..(vals.len() - base).min(4) {
                v[j] = vals[base + j];
            }
            v
        };
        push_cmd(&mut s.cmd, CMD_SET_SHADER_CONST, &[
            s.context_id, reg, SVGA3D_SHADERTYPE_VS, SVGA3D_CONST_TYPE_FLOAT,
            v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), v[3].to_bits(),
        ]);
    }
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_uniform_mat4(location: i32, values: *const f32) {
    let s = state();
    let mat = unsafe { slice::from_raw_parts(values, 16) };

    // A 4x4 matrix uses 4 consecutive float4 registers
    for row in 0..4u32 {
        let reg = location as u32 + row;
        let base = (row * 4) as usize;
        push_cmd(&mut s.cmd, CMD_SET_SHADER_CONST, &[
            s.context_id, reg, SVGA3D_SHADERTYPE_VS, SVGA3D_CONST_TYPE_FLOAT,
            mat[base].to_bits(), mat[base+1].to_bits(),
            mat[base+2].to_bits(), mat[base+3].to_bits(),
        ]);
    }
    submit(&mut s.cmd);
}

/// Map GL primitive type to SVGA3D primitive type.
fn gl_prim_to_svga(mode: u32) -> u32 {
    match mode {
        4 => SVGA3D_PRIMITIVE_TRIANGLELIST,  // GL_TRIANGLES
        5 => SVGA3D_PRIMITIVE_TRIANGLESTRIP, // GL_TRIANGLE_STRIP
        6 => SVGA3D_PRIMITIVE_TRIANGLEFAN,   // GL_TRIANGLE_FAN
        _ => SVGA3D_PRIMITIVE_TRIANGLELIST,
    }
}

/// Map component count to SVGA3D decl type.
fn components_to_decltype(components: u32) -> u32 {
    match components {
        1 => SVGA3D_DECLTYPE_FLOAT1,
        2 => SVGA3D_DECLTYPE_FLOAT2,
        3 => SVGA3D_DECLTYPE_FLOAT3,
        4 => SVGA3D_DECLTYPE_FLOAT4,
        _ => SVGA3D_DECLTYPE_FLOAT4,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_draw_arrays(
    mode: u32, first: u32, count: u32,
    vertex_data: *const u8, vertex_data_len: u32,
    attribs: *const DrvAttrib, num_attribs: u32,
) {
    let s = state();
    let attrib_slice = unsafe { slice::from_raw_parts(attribs, num_attribs as usize) };

    // Upload vertex data to a temporary surface via DMA
    let vb_sid = s.next_sid;
    s.next_sid += 1;

    // Create vertex buffer surface
    push_cmd(&mut s.cmd, CMD_SURFACE_DEFINE, &[
        vb_sid, 1 << 6, // SVGA3D_SURFACE_HINT_VERTEXBUFFER
        1, // SVGA3D_BUFFER (format=1 for buffer surfaces)
        1, 0, 0, 0, 0, 0,
        vertex_data_len, 1, 1,
    ]);
    submit(&mut s.cmd);

    // Upload vertex data
    let vdata = unsafe { slice::from_raw_parts(vertex_data, vertex_data_len as usize) };
    libsyscall::gpu_3d_surface_dma(vb_sid, vdata, vertex_data_len, 1);

    // Build vertex declarations (10 u32s each for SVGA3dVertexDecl)
    // Calculate stride from attribs
    let stride = if num_attribs > 0 {
        let mut max_end = 0u32;
        for a in attrib_slice {
            let end = a.offset + a.components * 4; // float = 4 bytes
            if end > max_end { max_end = end; }
        }
        max_end
    } else { 0 };

    let mut vertex_decl_words = Vec::with_capacity(num_attribs as usize * 10);
    for a in attrib_slice {
        // SVGA3dVertexDecl: { identity { type, method, usage, usageIndex },
        //                     array { surfaceId, offset, stride },
        //                     rangeHint { first, last } }
        let decl_type = components_to_decltype(a.components);
        let usage = a.location; // location maps to usage index

        vertex_decl_words.push(decl_type);  // identity.type
        vertex_decl_words.push(0);          // identity.method (default)
        vertex_decl_words.push(0);          // identity.usage (POSITION=0)
        vertex_decl_words.push(usage);      // identity.usageIndex

        vertex_decl_words.push(vb_sid);     // array.surfaceId
        vertex_decl_words.push(a.offset);   // array.offset
        vertex_decl_words.push(stride);     // array.stride

        vertex_decl_words.push(first);      // rangeHint.first
        vertex_decl_words.push(first + count); // rangeHint.last
        vertex_decl_words.push(0);          // padding
    }

    // Build primitive range (7 u32s for SVGA3dPrimitiveRange)
    let prim_type = gl_prim_to_svga(mode);
    let prim_count = match prim_type {
        SVGA3D_PRIMITIVE_TRIANGLELIST => count / 3,
        SVGA3D_PRIMITIVE_TRIANGLESTRIP | SVGA3D_PRIMITIVE_TRIANGLEFAN => {
            if count >= 3 { count - 2 } else { 0 }
        }
        _ => count / 3,
    };

    let prim_range_words = [
        prim_type,      // primType
        prim_count,     // primitiveCount
        0,              // indexArray.offset
        0,              // indexArray.stride
        0,              // indexArray.surfaceId (no index buffer)
        0,              // indexWidth
        0,              // indexBias
    ];

    // Issue draw
    let mut payload = Vec::with_capacity(3 + vertex_decl_words.len() + prim_range_words.len());
    payload.push(s.context_id);
    payload.push(num_attribs);
    payload.push(1); // numRanges
    payload.extend_from_slice(&vertex_decl_words);
    payload.extend_from_slice(&prim_range_words);
    push_cmd(&mut s.cmd, CMD_DRAW_PRIMITIVES, &payload);
    submit(&mut s.cmd);

    // Clean up temp vertex buffer
    push_cmd(&mut s.cmd, CMD_SURFACE_DESTROY, &[vb_sid]);
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_draw_elements(
    mode: u32, count: u32, _index_type: u32,
    vertex_data: *const u8,
    index_data: *const u8, index_data_len: u32,
    attribs: *const DrvAttrib, num_attribs: u32,
) {
    let s = state();
    let attrib_slice = unsafe { slice::from_raw_parts(attribs, num_attribs as usize) };

    // Calculate total vertex data size from attribs
    let stride = if num_attribs > 0 {
        let mut max_end = 0u32;
        for a in attrib_slice {
            let end = a.offset + a.components * 4;
            if end > max_end { max_end = end; }
        }
        max_end
    } else { 0 };

    // Find max index to determine vertex count
    let indices = unsafe { slice::from_raw_parts(index_data as *const u16, count as usize) };
    let mut max_idx = 0u16;
    for &idx in indices {
        if idx > max_idx { max_idx = idx; }
    }
    let vertex_count = max_idx as u32 + 1;
    let vb_size = vertex_count * stride;

    // Create and upload vertex buffer
    let vb_sid = s.next_sid;
    s.next_sid += 1;
    push_cmd(&mut s.cmd, CMD_SURFACE_DEFINE, &[
        vb_sid, 1 << 6, 1,
        1, 0, 0, 0, 0, 0,
        vb_size, 1, 1,
    ]);
    submit(&mut s.cmd);

    let vdata = unsafe { slice::from_raw_parts(vertex_data, vb_size as usize) };
    libsyscall::gpu_3d_surface_dma(vb_sid, vdata, vb_size, 1);

    // Create and upload index buffer
    let ib_sid = s.next_sid;
    s.next_sid += 1;
    push_cmd(&mut s.cmd, CMD_SURFACE_DEFINE, &[
        ib_sid, 1 << 7, 1, // SVGA3D_SURFACE_HINT_INDEXBUFFER
        1, 0, 0, 0, 0, 0,
        index_data_len, 1, 1,
    ]);
    submit(&mut s.cmd);

    let idata = unsafe { slice::from_raw_parts(index_data, index_data_len as usize) };
    libsyscall::gpu_3d_surface_dma(ib_sid, idata, index_data_len, 1);

    // Build vertex declarations
    let mut vertex_decl_words = Vec::with_capacity(num_attribs as usize * 10);
    for a in attrib_slice {
        let decl_type = components_to_decltype(a.components);
        vertex_decl_words.push(decl_type);
        vertex_decl_words.push(0);
        vertex_decl_words.push(0);
        vertex_decl_words.push(a.location);
        vertex_decl_words.push(vb_sid);
        vertex_decl_words.push(a.offset);
        vertex_decl_words.push(stride);
        vertex_decl_words.push(0);
        vertex_decl_words.push(vertex_count);
        vertex_decl_words.push(0);
    }

    // Build primitive range with index buffer
    let prim_type = gl_prim_to_svga(mode);
    let prim_count = match prim_type {
        SVGA3D_PRIMITIVE_TRIANGLELIST => count / 3,
        SVGA3D_PRIMITIVE_TRIANGLESTRIP | SVGA3D_PRIMITIVE_TRIANGLEFAN => {
            if count >= 3 { count - 2 } else { 0 }
        }
        _ => count / 3,
    };

    let prim_range_words = [
        prim_type,
        prim_count,
        0,          // indexArray.offset
        2,          // indexArray.stride (u16 = 2 bytes)
        ib_sid,     // indexArray.surfaceId
        2,          // indexWidth (u16)
        0,          // indexBias
    ];

    let mut payload = Vec::with_capacity(3 + vertex_decl_words.len() + prim_range_words.len());
    payload.push(s.context_id);
    payload.push(num_attribs);
    payload.push(1);
    payload.extend_from_slice(&vertex_decl_words);
    payload.extend_from_slice(&prim_range_words);
    push_cmd(&mut s.cmd, CMD_DRAW_PRIMITIVES, &payload);
    submit(&mut s.cmd);

    // Clean up temp buffers
    push_cmd(&mut s.cmd, CMD_SURFACE_DESTROY, &[ib_sid]);
    push_cmd(&mut s.cmd, CMD_SURFACE_DESTROY, &[vb_sid]);
    submit(&mut s.cmd);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_flush() {
    let s = state();
    submit(&mut s.cmd);
    libsyscall::gpu_3d_sync();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_finish() {
    let s = state();
    submit(&mut s.cmd);
    libsyscall::gpu_3d_sync();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_present(sid: u32) {
    let s = state();
    let target_sid = if sid == 0 { s.color_sid } else { sid };
    let w = s.width;
    let h = s.height;

    // Present: sid, then CopyRect { x, y, srcx, srcy, w, h }
    push_cmd(&mut s.cmd, CMD_PRESENT, &[target_sid, 0, 0, 0, 0, w, h]);
    submit(&mut s.cmd);
}

// ── Panic handler (required for no_std) ──────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
