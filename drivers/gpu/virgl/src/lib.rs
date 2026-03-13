//! VirGL GPU driver (.drv) — translates generic drv_* calls into virgl
//! (Gallium3D) command buffers submitted via SYS_GPU_3D_SUBMIT.
//!
//! Loaded at runtime by libgl's drv_loader when the kernel reports
//! gpu_type = "virgl".
//!
//! Virgl command encoding follows the virglrenderer protocol:
//! each command is a u32 header word encoding (length << 16 | object << 8 | cmd),
//! followed by payload words.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::slice;

// ── Heap allocator ───────────────────────────────────────────────────────

libheap::dll_allocator!(libsyscall::sbrk, libsyscall::mmap, libsyscall::munmap);

// ── Virgl command IDs (VIRGL_CCMD_*) ────────────────────────────────────

const VIRGL_CCMD_NOP: u32                    = 0;
const VIRGL_CCMD_CREATE_OBJECT: u32          = 1;
const VIRGL_CCMD_BIND_OBJECT: u32            = 2;
const VIRGL_CCMD_DESTROY_OBJECT: u32         = 3;
const VIRGL_CCMD_SET_VIEWPORT_STATE: u32     = 4;
const VIRGL_CCMD_SET_FRAMEBUFFER_STATE: u32  = 5;
const VIRGL_CCMD_SET_VERTEX_BUFFERS: u32     = 6;
const VIRGL_CCMD_CLEAR: u32                  = 7;
const VIRGL_CCMD_DRAW_VBO: u32               = 8;
const VIRGL_CCMD_RESOURCE_INLINE_WRITE: u32  = 9;
const VIRGL_CCMD_SET_SAMPLER_VIEWS: u32      = 10;
const VIRGL_CCMD_SET_INDEX_BUFFER: u32       = 11;
const VIRGL_CCMD_SET_CONSTANT_BUFFER: u32    = 12;
const VIRGL_CCMD_SET_STENCIL_REF: u32        = 13;
const VIRGL_CCMD_SET_BLEND_COLOR: u32        = 14;
const VIRGL_CCMD_SET_SCISSOR_STATE: u32      = 15;
const VIRGL_CCMD_BLIT: u32                   = 16;
const VIRGL_CCMD_RESOURCE_COPY_REGION: u32   = 17;
const VIRGL_CCMD_BIND_SAMPLER_STATES: u32    = 18;
const VIRGL_CCMD_BEGIN_QUERY: u32            = 19;
const VIRGL_CCMD_END_QUERY: u32              = 20;
const VIRGL_CCMD_GET_QUERY_RESULT: u32       = 21;
const VIRGL_CCMD_SET_POLYGON_STIPPLE: u32    = 22;
const VIRGL_CCMD_SET_CLIP_STATE: u32         = 23;
const VIRGL_CCMD_SET_SAMPLE_MASK: u32        = 24;
const VIRGL_CCMD_SET_STREAMOUT_TARGETS: u32  = 25;
const VIRGL_CCMD_SET_RENDER_CONDITION: u32   = 26;
const VIRGL_CCMD_SET_UNIFORM_BUFFER: u32     = 27;
const VIRGL_CCMD_SET_SUB_CTX: u32            = 28;
const VIRGL_CCMD_CREATE_SUB_CTX: u32         = 29;
const VIRGL_CCMD_DESTROY_SUB_CTX: u32        = 30;
const VIRGL_CCMD_BIND_SHADER: u32            = 31;

// ── Virgl object types (VIRGL_OBJECT_*) ─────────────────────────────────

const VIRGL_OBJECT_BLEND: u32          = 1;
const VIRGL_OBJECT_RASTERIZER: u32     = 2;
const VIRGL_OBJECT_DSA: u32            = 3;
const VIRGL_OBJECT_SHADER: u32         = 4;
const VIRGL_OBJECT_VERTEX_ELEMENTS: u32 = 5;
const VIRGL_OBJECT_SAMPLER_VIEW: u32   = 6;
const VIRGL_OBJECT_SAMPLER_STATE: u32  = 7;
const VIRGL_OBJECT_SURFACE: u32        = 8;
const VIRGL_OBJECT_QUERY: u32          = 9;
const VIRGL_OBJECT_STREAMOUT_TARGET: u32 = 10;

// ── Gallium pipe constants ──────────────────────────────────────────────

const PIPE_SHADER_VERTEX: u32   = 0;
const PIPE_SHADER_FRAGMENT: u32 = 1;

const PIPE_PRIM_TRIANGLES: u32      = 4;
const PIPE_PRIM_TRIANGLE_STRIP: u32 = 5;
const PIPE_PRIM_TRIANGLE_FAN: u32   = 6;

const PIPE_CLEAR_COLOR: u32   = 1;
const PIPE_CLEAR_DEPTH: u32   = 2;
const PIPE_CLEAR_STENCIL: u32 = 4;

// Pipe formats (subset)
const PIPE_FORMAT_B8G8R8A8_UNORM: u32 = 1;
const PIPE_FORMAT_Z24_UNORM_S8_UINT: u32 = 36;
const PIPE_FORMAT_R32G32B32A32_FLOAT: u32 = 31;

// Pipe bind flags
const PIPE_BIND_RENDER_TARGET: u32    = 1 << 1;
const PIPE_BIND_DEPTH_STENCIL: u32    = 1 << 2;
const PIPE_BIND_VERTEX_BUFFER: u32    = 1 << 3;
const PIPE_BIND_INDEX_BUFFER: u32     = 1 << 4;
const PIPE_BIND_CONSTANT_BUFFER: u32  = 1 << 5;

// Pipe texture target
const PIPE_TEXTURE_2D: u32   = 2;
const PIPE_BUFFER: u32       = 7;

// ── Attrib descriptor (matches drv_loader::DrvAttrib layout) ─────────────

#[repr(C)]
struct DrvAttrib {
    location: u32,
    components: u32,
    attr_type: u32,
    offset: u32,
    normalized: u32,
}

// ── Command buffer builder ───────────────────────────────────────────────

struct CmdBuf {
    words: Vec<u32>,
}

impl CmdBuf {
    fn new() -> Self {
        Self { words: Vec::with_capacity(1024) }
    }

    /// Encode a virgl command header: (length << 16) | (object << 8) | cmd
    fn push_cmd(&mut self, cmd: u32, object: u32, length: u32) {
        self.words.push((length << 16) | (object << 8) | cmd);
    }

    fn push(&mut self, val: u32) {
        self.words.push(val);
    }

    fn push_f32(&mut self, val: f32) {
        self.words.push(val.to_bits());
    }

    fn submit(&mut self) -> u32 {
        if self.words.is_empty() { return 0; }
        let result = libsyscall::gpu_3d_submit(&self.words);
        self.words.clear();
        result
    }
}

// ── Driver state ─────────────────────────────────────────────────────────

struct VirglState {
    cmd: CmdBuf,
    width: u32,
    height: u32,
    next_handle: u32,

    // Pre-created resource handles
    color_surface_handle: u32,
    depth_surface_handle: u32,
    framebuffer_set: bool,

    // Blend/DSA/rasterizer objects
    blend_handle: u32,
    dsa_handle: u32,
    rast_handle: u32,

    // Current shader bindings
    current_vs: u32,
    current_fs: u32,
}

static mut STATE: Option<VirglState> = None;

fn state() -> &'static mut VirglState {
    unsafe { STATE.as_mut().unwrap() }
}

fn alloc_handle() -> u32 {
    let s = state();
    let h = s.next_handle;
    s.next_handle += 1;
    h
}

// ── Exported drv_* API ───────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn drv_init(width: u32, height: u32) -> u32 {
    let mut cmd = CmdBuf::new();

    let color_handle = 1u32;
    let depth_handle = 2u32;
    let blend_handle = 3u32;
    let dsa_handle = 4u32;
    let rast_handle = 5u32;

    // Create color surface (VIRGL_OBJECT_SURFACE referencing the scanout resource)
    // For virgl, surfaces are views on resources. The host creates the actual
    // GL framebuffer. We just need to set framebuffer state.

    // Create and activate sub-context (required before any object creation)
    // Must be submitted separately — virglrenderer needs sub-ctx active
    // before processing any CREATE_OBJECT commands in the same batch.
    cmd.push_cmd(VIRGL_CCMD_CREATE_SUB_CTX, 0, 1);
    cmd.push(1); // sub-context ID
    cmd.push_cmd(VIRGL_CCMD_SET_SUB_CTX, 0, 1);
    cmd.push(1);
    cmd.submit();

    // Create default blend state (no blending)
    // Virgl blend layout: handle(1) + S0(1) + S1(1) + S2[0..7](8) = 11 words
    // S2 per-RT: blend_enable[0] | rgb_func[3:1] | rgb_src[8:4] | rgb_dst[13:9]
    //            | alpha_func[16:14] | alpha_src[21:17] | alpha_dst[26:22] | colormask[30:27]
    cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_BLEND, 11);
    cmd.push(blend_handle);
    cmd.push(0); // S0: independent_blend_enable=0, logicop=0, dither=0
    cmd.push(0); // S1: logicop_func=0
    // S2[0]: RT0 — colormask=0xF (RGBA) at bits 27:30
    cmd.push(0x0F << 27);
    // S2[1..7]: RT1–RT7 disabled
    for _ in 1..8 {
        cmd.push(0);
    }

    // Create default DSA state (depth test disabled)
    // CREATE_OBJECT(DSA): handle, S0 (depth), S1 (stencil)
    cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_DSA, 5);
    cmd.push(dsa_handle);
    cmd.push(0); // S0: depth.enabled=0, depth.writemask=0, depth.func=0
    cmd.push(0); // S1: stencil[0]
    cmd.push(0); // S1: stencil[1]
    cmd.push(0); // alpha ref

    // Create default rasterizer state
    // Layout: handle, S0, point_size, sprite_coord_enable, S3, line_width,
    //         offset_units, offset_scale, offset_clamp
    cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_RASTERIZER, 9);
    cmd.push(rast_handle);
    cmd.push(0x00000002); // S0: flatshade=0, depth_clip=1 (bit 1)
    cmd.push(0);          // point_size (float)
    cmd.push(0);          // sprite_coord_enable
    cmd.push(0);          // S3: line_stipple_pattern/factor/clip_plane_enable
    cmd.push(0x3F800000); // line_width = 1.0f
    cmd.push(0);          // offset_units (float)
    cmd.push(0);          // offset_scale (float)
    cmd.push(0);          // offset_clamp (float)

    // Bind blend, DSA, rasterizer
    cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_BLEND, 1);
    cmd.push(blend_handle);

    cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_DSA, 1);
    cmd.push(dsa_handle);

    cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_RASTERIZER, 1);
    cmd.push(rast_handle);

    // Set viewport
    cmd.push_cmd(VIRGL_CCMD_SET_VIEWPORT_STATE, 0, 7);
    cmd.push(0); // start_slot
    let half_w = width as f32 / 2.0;
    let half_h = height as f32 / 2.0;
    cmd.push_f32(half_w);   // scale.x
    cmd.push_f32(-half_h);  // scale.y (flip Y for GL)
    cmd.push_f32(0.5);      // scale.z
    cmd.push_f32(half_w);   // translate.x
    cmd.push_f32(half_h);   // translate.y
    cmd.push_f32(0.5);      // translate.z

    let result = cmd.submit();
    if result != 0 { return 0; }

    unsafe {
        STATE = Some(VirglState {
            cmd,
            width,
            height,
            next_handle: 6, // 1-5 are pre-allocated
            color_surface_handle: color_handle,
            depth_surface_handle: depth_handle,
            framebuffer_set: false,
            blend_handle,
            dsa_handle,
            rast_handle,
            current_vs: 0,
            current_fs: 0,
        });
    }

    1 // success
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_deinit() {
    unsafe {
        if let Some(ref mut s) = STATE {
            // Destroy objects
            s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_RASTERIZER, 1);
            s.cmd.push(s.rast_handle);
            s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_DSA, 1);
            s.cmd.push(s.dsa_handle);
            s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_BLEND, 1);
            s.cmd.push(s.blend_handle);
            s.cmd.submit();
        }
        STATE = None;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_create_surface(format: u32, _flags: u32, _width: u32, _height: u32) -> u32 {
    let _ = format;
    alloc_handle()
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_destroy_surface(_sid: u32) {
    // virgl surfaces are managed by host
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
pub extern "C" fn drv_create_shader(shader_type: u32, _version: u32, text: *const u8, len: u32) -> u32 {
    let s = state();
    let handle = alloc_handle();

    let pipe_type = if shader_type == 0 { PIPE_SHADER_VERTEX } else { PIPE_SHADER_FRAGMENT };
    let text_bytes = unsafe { slice::from_raw_parts(text, len as usize) };

    // Virgl shader protocol (TGSI text):
    //   [1] handle
    //   [2] type (PIPE_SHADER_VERTEX/FRAGMENT)
    //   [3] offlen = byte length of TGSI text (bit 31=0 for first/only packet)
    //   [4] num_tokens = 0 (text mode, not binary tokens)
    //   [5] num_so_outputs = 0
    //   [6..] TGSI text packed into u32 words
    let text_words = (len as usize + 3) / 4;
    let payload_len = 5 + text_words as u32;

    s.cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SHADER, payload_len);
    s.cmd.push(handle);
    s.cmd.push(pipe_type);
    s.cmd.push(len);        // offlen = text byte length (bit 31=0: not continuation)
    s.cmd.push(0);          // num_tokens = 0 (TGSI text, not binary)
    s.cmd.push(0);          // num_so_outputs = 0

    // Pack text bytes into u32 words (little-endian)
    for i in 0..text_words {
        let base = i * 4;
        let mut word = 0u32;
        for j in 0..4 {
            if base + j < text_bytes.len() {
                word |= (text_bytes[base + j] as u32) << (j * 8);
            }
        }
        s.cmd.push(word);
    }
    s.cmd.submit();

    handle
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_destroy_shader(shid: u32) {
    let s = state();
    s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_SHADER, 1);
    s.cmd.push(shid);
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_link_program(vs_id: u32, ps_id: u32, _flags: u32) -> u32 {
    // Virgl doesn't have linked programs — shaders are bound individually.
    // Return packed ID.
    (vs_id << 16) | ps_id
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_use_program(program_id: u32) {
    let s = state();
    let vs_id = program_id >> 16;
    let ps_id = program_id & 0xFFFF;

    if vs_id != s.current_vs {
        s.cmd.push_cmd(VIRGL_CCMD_BIND_SHADER, 0, 2);
        s.cmd.push(vs_id);
        s.cmd.push(PIPE_SHADER_VERTEX);
        s.current_vs = vs_id;
    }
    if ps_id != s.current_fs {
        s.cmd.push_cmd(VIRGL_CCMD_BIND_SHADER, 0, 2);
        s.cmd.push(ps_id);
        s.cmd.push(PIPE_SHADER_FRAGMENT);
        s.current_fs = ps_id;
    }
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_viewport(x: u32, y: u32, w: u32, h: u32) {
    let s = state();
    let half_w = w as f32 / 2.0;
    let half_h = h as f32 / 2.0;

    s.cmd.push_cmd(VIRGL_CCMD_SET_VIEWPORT_STATE, 0, 7);
    s.cmd.push(0); // start_slot
    s.cmd.push_f32(half_w);
    s.cmd.push_f32(-half_h);
    s.cmd.push_f32(0.5);
    s.cmd.push_f32(x as f32 + half_w);
    s.cmd.push_f32(y as f32 + half_h);
    s.cmd.push_f32(0.5);
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_blend(enable: u32, src_factor: u32, dst_factor: u32) {
    let s = state();

    // Destroy old blend, create new
    s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_BLEND, 1);
    s.cmd.push(s.blend_handle);

    let new_handle = alloc_handle();
    // Blend: handle(1) + S0(1) + S1(1) + S2[0..7](8) = 11 words
    s.cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_BLEND, 11);
    s.cmd.push(new_handle);
    s.cmd.push(0); // S0: independent_blend=0
    s.cmd.push(0); // S1: logicop_func=0
    // S2[0]: RT0 — colormask at bits 27:30, blend state at lower bits
    if enable != 0 {
        // S2 per-RT: blend_enable[0] | rgb_func[3:1] | rgb_src[8:4] | rgb_dst[13:9]
        //            | alpha_func[16:14] | alpha_src[21:17] | alpha_dst[26:22] | colormask[30:27]
        // func = ADD(1 in Gallium PIPE_BLEND_ADD)
        let rt0 = 1 // blend_enable
            | (1 << 1) // rgb_func = PIPE_BLEND_ADD
            | (src_factor << 4)
            | (dst_factor << 9)
            | (1 << 14) // alpha_func = PIPE_BLEND_ADD
            | (src_factor << 17)
            | (dst_factor << 22)
            | (0x0F << 27); // colormask RGBA
        s.cmd.push(rt0);
    } else {
        s.cmd.push(0x0F << 27); // just colormask RGBA, no blend
    }
    // S2[1..7]: disabled
    for _ in 1..8 {
        s.cmd.push(0);
    }

    s.cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_BLEND, 1);
    s.cmd.push(new_handle);

    s.blend_handle = new_handle;
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_depth_test(enable: u32, func: u32) {
    let s = state();

    s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_DSA, 1);
    s.cmd.push(s.dsa_handle);

    let new_handle = alloc_handle();
    s.cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_DSA, 5);
    s.cmd.push(new_handle);
    if enable != 0 {
        // S0: depth_enabled=1, writemask=1, func
        let s0 = 1 | (1 << 1) | (func << 2);
        s.cmd.push(s0);
    } else {
        s.cmd.push(0);
    }
    s.cmd.push(0); // stencil[0]
    s.cmd.push(0); // stencil[1]
    s.cmd.push(0); // alpha ref

    s.cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_DSA, 1);
    s.cmd.push(new_handle);

    s.dsa_handle = new_handle;
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_clear(flags: u32, r: f32, g: f32, b: f32, a: f32, depth: f32) {
    let s = state();

    let mut pipe_flags = 0u32;
    if flags & 1 != 0 { pipe_flags |= PIPE_CLEAR_COLOR; }
    if flags & 2 != 0 { pipe_flags |= PIPE_CLEAR_DEPTH; }
    if flags & 4 != 0 { pipe_flags |= PIPE_CLEAR_STENCIL; }

    // CLEAR: buffers, color[4], depth, stencil
    s.cmd.push_cmd(VIRGL_CCMD_CLEAR, 0, 8);
    s.cmd.push(pipe_flags);
    s.cmd.push_f32(r);
    s.cmd.push_f32(g);
    s.cmd.push_f32(b);
    s.cmd.push_f32(a);
    s.cmd.push_f32(depth); // depth (as float bits in u64, but virgl uses f64... use f32 approx)
    s.cmd.push(0);         // depth high bits
    s.cmd.push(0);         // stencil
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_uniform_f32(location: i32, count: u32, values: *const f32) {
    let s = state();
    let vals = unsafe { slice::from_raw_parts(values, (count * 4) as usize) };

    // SET_CONSTANT_BUFFER: shader_type, index, [data...]
    let data_len = vals.len() as u32;
    s.cmd.push_cmd(VIRGL_CCMD_SET_CONSTANT_BUFFER, 0, 2 + data_len);
    s.cmd.push(PIPE_SHADER_VERTEX);
    s.cmd.push(location as u32); // constant buffer index
    for &v in vals {
        s.cmd.push_f32(v);
    }
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_uniform_mat4(location: i32, values: *const f32) {
    // A mat4 = 4 vec4 registers = 16 floats
    drv_set_uniform_f32(location, 4, values);
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_draw_arrays(
    mode: u32, first: u32, count: u32,
    vertex_data: *const u8, vertex_data_len: u32,
    attribs: *const DrvAttrib, num_attribs: u32,
) {
    let s = state();
    let _ = (vertex_data, vertex_data_len, attribs, num_attribs);

    // DRAW_VBO: mode, start, count, ...
    s.cmd.push_cmd(VIRGL_CCMD_DRAW_VBO, 0, 12);
    s.cmd.push(first);          // start
    s.cmd.push(count);          // count
    s.cmd.push(mode);           // mode
    s.cmd.push(0);              // indexed = false
    s.cmd.push(1);              // instance_count
    s.cmd.push(0);              // index_bias
    s.cmd.push(0);              // start_instance
    s.cmd.push(0);              // primitive_restart
    s.cmd.push(0);              // restart_index
    s.cmd.push(0);              // min_index
    s.cmd.push(first + count);  // max_index
    s.cmd.push(0);              // cso (vertex elements handle, 0=default)
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_draw_elements(
    mode: u32, count: u32, _index_type: u32,
    _vertex_data: *const u8,
    _index_data: *const u8, _index_data_len: u32,
    _attribs: *const DrvAttrib, _num_attribs: u32,
) {
    let s = state();

    // DRAW_VBO with indexed=true
    s.cmd.push_cmd(VIRGL_CCMD_DRAW_VBO, 0, 12);
    s.cmd.push(0);              // start
    s.cmd.push(count);          // count
    s.cmd.push(mode);           // mode
    s.cmd.push(1);              // indexed = true
    s.cmd.push(1);              // instance_count
    s.cmd.push(0);              // index_bias
    s.cmd.push(0);              // start_instance
    s.cmd.push(0);              // primitive_restart
    s.cmd.push(0);              // restart_index
    s.cmd.push(0);              // min_index
    s.cmd.push(count);          // max_index
    s.cmd.push(0);              // cso
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_flush() {
    let s = state();
    s.cmd.submit();
    libsyscall::gpu_3d_sync();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_finish() {
    let s = state();
    s.cmd.submit();
    libsyscall::gpu_3d_sync();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_present(_sid: u32) {
    let s = state();
    s.cmd.submit();
    libsyscall::gpu_3d_sync();
}

// ── Panic handler ────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
