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

// Gallium pipe/p_defines.h PIPE_CLEAR_* flags (bit positions):
//   bit 0 = PIPE_CLEAR_DEPTH    = 1
//   bit 1 = PIPE_CLEAR_STENCIL  = 2
//   bit 2 = PIPE_CLEAR_COLOR0   = 4  (used for single render target)
const PIPE_CLEAR_DEPTH: u32    = 1;
const PIPE_CLEAR_STENCIL: u32  = 2;
const PIPE_CLEAR_COLOR: u32    = 4;  // = PIPE_CLEAR_COLOR0

// Pipe formats (subset)
const PIPE_FORMAT_B8G8R8A8_UNORM: u32 = 1;
const PIPE_FORMAT_S8_UINT_Z24_UNORM: u32 = 20;
const PIPE_FORMAT_R32G32B32A32_FLOAT: u32 = 31;
const PIPE_FORMAT_R32_FLOAT: u32       = 47;
const PIPE_FORMAT_R32G32_FLOAT: u32    = 48;
const PIPE_FORMAT_R32G32B32_FLOAT: u32 = 49;
const PIPE_FORMAT_R8_UNORM: u32 = 64;

// Virgl bind flags (VIRGL_BIND_*, NOT Gallium PIPE_BIND_*)
const VIRGL_BIND_DEPTH_STENCIL: u32   = 1 << 0;
const VIRGL_BIND_RENDER_TARGET: u32   = 1 << 1;
const VIRGL_BIND_SAMPLER_VIEW: u32    = 1 << 3;
const VIRGL_BIND_VERTEX_BUFFER: u32   = 1 << 4;
const VIRGL_BIND_INDEX_BUFFER: u32    = 1 << 5;
const VIRGL_BIND_CONSTANT_BUFFER: u32 = 1 << 6;

// Pipe texture target (enum pipe_texture_target)
const PIPE_BUFFER: u32       = 0;
const PIPE_TEXTURE_2D: u32   = 2;

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

    // Render target resources (created via virtio-gpu control plane)
    color_res_id: u32,
    depth_res_id: u32,
    color_surface_handle: u32,
    depth_surface_handle: u32,

    // Blend/DSA/rasterizer objects
    blend_handle: u32,
    dsa_handle: u32,
    rast_handle: u32,

    // Vertex element object
    ve_handle: u32,
    ve_num_attribs: u32,

    // Reusable vertex/index buffer resources
    vbo_res_id: u32,
    vbo_size: u32,
    ibo_res_id: u32,
    ibo_size: u32,

    // Current shader bindings
    current_vs: u32,
    current_fs: u32,

    // CPU-side constant buffer mirror: indexed by location*4 (vec4 slots).
    // Uploaded in one shot before each draw to avoid per-uniform overwrite.
    const_buf: Vec<f32>,
    const_buf_dirty: bool,

    // Texture resource cache: maps GL texture ID → virgl resource ID.
    tex_cache: Vec<(u32, u32)>,

    // Default sampler state object handle (LINEAR filter, REPEAT wrap).
    sampler_state_handle: u32,

    // Per-slot cached sampler view handles (VIRGL_OBJECT_SAMPLER_VIEW).
    sampler_views: [u32; 8],
    // Per-slot cached virgl resource ID (to detect when rebind is needed).
    sampler_view_res: [u32; 8],
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
    let mut next_h = 1u32;
    let mut alloc_h = || { let h = next_h; next_h += 1; h };

    // ── Create render target resources via virtio-gpu control plane ──

    // Color buffer: PIPE_TEXTURE_2D, BGRA, RENDER_TARGET
    let color_res = libsyscall::gpu_3d_resource_create(
        PIPE_TEXTURE_2D, PIPE_FORMAT_B8G8R8A8_UNORM,
        VIRGL_BIND_RENDER_TARGET | VIRGL_BIND_SAMPLER_VIEW,
        width, height,
    );
    libsyscall::serial_println!("[virgl] drv_init: color_res={}", color_res);
    if color_res == u32::MAX { return 0; }

    // Depth/stencil buffer: PIPE_TEXTURE_2D, Z24S8, DEPTH_STENCIL
    let depth_res = libsyscall::gpu_3d_resource_create(
        PIPE_TEXTURE_2D, PIPE_FORMAT_S8_UINT_Z24_UNORM,
        VIRGL_BIND_DEPTH_STENCIL,
        width, height,
    );
    libsyscall::serial_println!("[virgl] drv_init: depth_res={}", depth_res);
    if depth_res == u32::MAX { return 0; }

    // ── Sub-context (must be submitted before CREATE_OBJECT) ──
    cmd.push_cmd(VIRGL_CCMD_CREATE_SUB_CTX, 0, 1);
    cmd.push(1);
    cmd.push_cmd(VIRGL_CCMD_SET_SUB_CTX, 0, 1);
    cmd.push(1);
    cmd.submit();

    // ── Create surface objects (views on resources) ──
    let color_surf_h = alloc_h();
    cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SURFACE, 5);
    cmd.push(color_surf_h);
    cmd.push(color_res);                // resource handle
    cmd.push(PIPE_FORMAT_B8G8R8A8_UNORM);
    cmd.push(0);                        // level = 0
    cmd.push(0);                        // first_layer=0 | (last_layer=0 << 16)

    let depth_surf_h = alloc_h();
    cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SURFACE, 5);
    cmd.push(depth_surf_h);
    cmd.push(depth_res);
    cmd.push(PIPE_FORMAT_S8_UINT_Z24_UNORM);
    cmd.push(0);
    cmd.push(0);

    // ── Set framebuffer state: 1 color buffer + depth ──
    cmd.push_cmd(VIRGL_CCMD_SET_FRAMEBUFFER_STATE, 0, 3);
    cmd.push(1);               // nr_cbufs = 1
    cmd.push(depth_surf_h);    // zsurf_handle
    cmd.push(color_surf_h);    // cbuf[0]

    // ── Create default blend (no blending, colormask=RGBA) ──
    let blend_handle = alloc_h();
    cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_BLEND, 11);
    cmd.push(blend_handle);
    cmd.push(0); // S0
    cmd.push(0); // S1
    cmd.push(0x0F << 27); // S2[0]: colormask RGBA
    for _ in 1..8 { cmd.push(0); }

    // ── Create default DSA (depth test disabled) ──
    let dsa_handle = alloc_h();
    cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_DSA, 5);
    cmd.push(dsa_handle);
    cmd.push(0); cmd.push(0); cmd.push(0); cmd.push(0);

    // ── Create default rasterizer ──
    let rast_handle = alloc_h();
    cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_RASTERIZER, 9);
    cmd.push(rast_handle);
    cmd.push(0x00000002); // depth_clip=1
    cmd.push(0);          // point_size
    cmd.push(0);          // sprite_coord_enable
    cmd.push(0);          // S3
    cmd.push(0x3F800000); // line_width = 1.0f
    cmd.push(0); cmd.push(0); cmd.push(0); // offset

    // ── Create default sampler state (LINEAR filter, REPEAT wrap) ──
    let samp_state_h = alloc_h();
    // Sampler state payload: handle + s0 + lod_bias + min_lod + max_lod + border[4] = 9 words
    cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SAMPLER_STATE, 9);
    cmd.push(samp_state_h);
    // S0 bits: wrap_s[2:0]=0(REPEAT), wrap_t[5:3]=0, wrap_r[8:6]=0,
    //          min_img_filter[10:9]=1(LINEAR), min_mip_filter[12:11]=2(NONE),
    //          mag_img_filter[14:13]=1(LINEAR)
    let s0 = (1u32 << 9) | (2u32 << 11) | (1u32 << 13);
    cmd.push(s0);
    cmd.push_f32(0.0);      // lod_bias
    cmd.push_f32(-1000.0);  // min_lod
    cmd.push_f32(1000.0);   // max_lod
    cmd.push_f32(0.0);      // border_color[0]
    cmd.push_f32(0.0);      // border_color[1]
    cmd.push_f32(0.0);      // border_color[2]
    cmd.push_f32(0.0);      // border_color[3]

    // ── Bind state objects ──
    cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_BLEND, 1);
    cmd.push(blend_handle);
    cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_DSA, 1);
    cmd.push(dsa_handle);
    cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_RASTERIZER, 1);
    cmd.push(rast_handle);

    // ── Set viewport ──
    cmd.push_cmd(VIRGL_CCMD_SET_VIEWPORT_STATE, 0, 7);
    cmd.push(0);
    let half_w = width as f32 / 2.0;
    let half_h = height as f32 / 2.0;
    cmd.push_f32(half_w);
    cmd.push_f32(-half_h);
    cmd.push_f32(0.5);
    cmd.push_f32(half_w);
    cmd.push_f32(half_h);
    cmd.push_f32(0.5);

    let result = cmd.submit();
    if result != 0 { return 0; }

    unsafe {
        STATE = Some(VirglState {
            cmd,
            width,
            height,
            next_handle: next_h,
            color_res_id: color_res,
            depth_res_id: depth_res,
            color_surface_handle: color_surf_h,
            depth_surface_handle: depth_surf_h,
            blend_handle,
            dsa_handle,
            rast_handle,
            ve_handle: 0,
            ve_num_attribs: 0,
            vbo_res_id: 0,
            vbo_size: 0,
            ibo_res_id: 0,
            ibo_size: 0,
            current_vs: 0,
            current_fs: 0,
            const_buf: Vec::new(),
            const_buf_dirty: false,
            tex_cache: Vec::new(),
            sampler_state_handle: samp_state_h,
            sampler_views: [0u32; 8],
            sampler_view_res: [0u32; 8],
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

/// Resize the virgl render target to new dimensions.
///
/// Destroys the old color+depth resources/surfaces and creates new ones.
/// Called when the GL canvas is resized so readback dimensions stay consistent.
#[unsafe(no_mangle)]
pub extern "C" fn drv_resize(width: u32, height: u32) -> u32 {
    if width == 0 || height == 0 { return 0; }
    let s = state();
    if s.width == width && s.height == height { return 1; }

    libsyscall::serial_println!("[virgl] drv_resize: {}x{} -> {}x{}", s.width, s.height, width, height);

    // Destroy old surface objects
    s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_SURFACE, 1);
    s.cmd.push(s.color_surface_handle);
    s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_SURFACE, 1);
    s.cmd.push(s.depth_surface_handle);
    s.cmd.submit();

    // Destroy old GPU resources
    libsyscall::gpu_3d_resource_destroy(s.color_res_id);
    libsyscall::gpu_3d_resource_destroy(s.depth_res_id);
    s.color_res_id = 0;
    s.depth_res_id = 0;

    // Create new color buffer resource
    let color_res = libsyscall::gpu_3d_resource_create(
        PIPE_TEXTURE_2D, PIPE_FORMAT_B8G8R8A8_UNORM,
        VIRGL_BIND_RENDER_TARGET | VIRGL_BIND_SAMPLER_VIEW,
        width, height,
    );
    if color_res == u32::MAX {
        libsyscall::serial_println!("[virgl] drv_resize: color_res failed");
        return 0;
    }

    // Create new depth/stencil resource
    let depth_res = libsyscall::gpu_3d_resource_create(
        PIPE_TEXTURE_2D, PIPE_FORMAT_S8_UINT_Z24_UNORM,
        VIRGL_BIND_DEPTH_STENCIL,
        width, height,
    );
    if depth_res == u32::MAX {
        libsyscall::serial_println!("[virgl] drv_resize: depth_res failed");
        libsyscall::gpu_3d_resource_destroy(color_res);
        return 0;
    }

    // Create new surface objects
    let color_surf_h = alloc_handle();
    s.cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SURFACE, 5);
    s.cmd.push(color_surf_h);
    s.cmd.push(color_res);
    s.cmd.push(PIPE_FORMAT_B8G8R8A8_UNORM);
    s.cmd.push(0);
    s.cmd.push(0);

    let depth_surf_h = alloc_handle();
    s.cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SURFACE, 5);
    s.cmd.push(depth_surf_h);
    s.cmd.push(depth_res);
    s.cmd.push(PIPE_FORMAT_S8_UINT_Z24_UNORM);
    s.cmd.push(0);
    s.cmd.push(0);

    // Rebind framebuffer
    s.cmd.push_cmd(VIRGL_CCMD_SET_FRAMEBUFFER_STATE, 0, 3);
    s.cmd.push(1);               // nr_cbufs = 1
    s.cmd.push(depth_surf_h);
    s.cmd.push(color_surf_h);

    // Update viewport to new dimensions
    let half_w = width as f32 / 2.0;
    let half_h = height as f32 / 2.0;
    s.cmd.push_cmd(VIRGL_CCMD_SET_VIEWPORT_STATE, 0, 7);
    s.cmd.push(0);
    s.cmd.push_f32(half_w);
    s.cmd.push_f32(-half_h);
    s.cmd.push_f32(0.5);
    s.cmd.push_f32(half_w);
    s.cmd.push_f32(half_h);
    s.cmd.push_f32(0.5);

    let r = s.cmd.submit();
    if r != 0 {
        libsyscall::serial_println!("[virgl] drv_resize: submit failed: {}", r);
        return 0;
    }

    s.color_res_id = color_res;
    s.depth_res_id = depth_res;
    s.color_surface_handle = color_surf_h;
    s.depth_surface_handle = depth_surf_h;
    s.width = width;
    s.height = height;

    1
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

    // Virgl shader protocol (TGSI text, num_so=0):
    //   HDR_SIZE(0) = 5 (no SO strides when num_so=0)
    //   [0] handle
    //   [1] type (PIPE_SHADER_VERTEX/FRAGMENT)
    //   [2] offlen = byte length of TGSI text (bit 31=0 for first/only packet)
    //   [3] num_tokens = token array size for tgsi_text_translate
    //   [4] num_so_outputs = 0
    //   [5..] TGSI text packed into u32 words (null-terminated!)
    //
    // Constraints:
    //   - pkt_length (text dwords) = payload_len - HDR_SIZE = must == (offlen+3)/4
    //   - Last 4 bytes of text must contain a '\0' (vrend_shader_assign_tgsi check)
    //   - num_tokens > 0: size of token array for tgsi_text_translate output

    // Ensure null termination: add a NUL byte to text length
    let text_len_with_nul = len + 1;
    let text_words = ((text_len_with_nul as usize) + 3) / 4;
    // num_tokens: estimated token count for tgsi_text_translate output buffer
    // Conservative estimate: ~2 tokens per TGSI instruction line
    let num_tokens = (text_words as u32).max(256);
    let payload_len = 5 + text_words as u32;

    s.cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SHADER, payload_len);
    s.cmd.push(handle);
    s.cmd.push(pipe_type);
    s.cmd.push(text_len_with_nul);  // offlen = byte length including NUL
    s.cmd.push(num_tokens);         // num_tokens = output token array size
    s.cmd.push(0);                  // num_so_outputs = 0

    // Pack text bytes into u32 words (little-endian, zero-padded → null terminated)
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

    // `flags` is the raw GL clear bitfield:
    // GL_DEPTH_BUFFER_BIT   = 0x0100
    // GL_STENCIL_BUFFER_BIT = 0x0400
    // GL_COLOR_BUFFER_BIT   = 0x4000
    let mut pipe_flags = 0u32;
    if flags & 0x4000 != 0 { pipe_flags |= PIPE_CLEAR_COLOR; }
    if flags & 0x0100 != 0 { pipe_flags |= PIPE_CLEAR_DEPTH; }
    if flags & 0x0400 != 0 { pipe_flags |= PIPE_CLEAR_STENCIL; }

    // CLEAR: buffers, color[4], depth, stencil
    s.cmd.push_cmd(VIRGL_CCMD_CLEAR, 0, 8);
    s.cmd.push(pipe_flags);
    s.cmd.push_f32(r);
    s.cmd.push_f32(g);
    s.cmd.push_f32(b);
    s.cmd.push_f32(a);
    // Virgl CLEAR expects depth as f64 (two u32 words, little-endian)
    let depth_bits = (depth as f64).to_bits();
    s.cmd.push((depth_bits & 0xFFFF_FFFF) as u32);       // depth lo
    s.cmd.push(((depth_bits >> 32) & 0xFFFF_FFFF) as u32); // depth hi
    s.cmd.push(0);         // stencil
    s.cmd.submit();
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_uniform_f32(location: i32, count: u32, values: *const f32) {
    let s = state();
    let vals = unsafe { slice::from_raw_parts(values, (count * 4) as usize) };

    // Write into the CPU-side mirror at the correct location offset.
    // The full buffer is uploaded once before the draw call.
    let start = (location as usize) * 4;
    let end = start + vals.len();
    if s.const_buf.len() < end {
        s.const_buf.resize(end, 0.0);
    }
    s.const_buf[start..end].copy_from_slice(vals);
    s.const_buf_dirty = true;
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_set_uniform_mat4(location: i32, values: *const f32) {
    // A mat4 = 4 vec4 registers = 16 floats
    drv_set_uniform_f32(location, 4, values);
}

/// Ensure a VBO resource exists with at least `size` bytes, creating/resizing as needed.
fn ensure_vbo(s: &mut VirglState, size: u32) {
    if s.vbo_res_id != 0 && s.vbo_size >= size {
        return;
    }
    if s.vbo_res_id != 0 {
        libsyscall::gpu_3d_resource_destroy(s.vbo_res_id);
    }
    let res = libsyscall::gpu_3d_resource_create(
        PIPE_BUFFER, PIPE_FORMAT_R8_UNORM, VIRGL_BIND_VERTEX_BUFFER, size, 1,
    );
    s.vbo_res_id = res;
    s.vbo_size = size;
}

/// Ensure an IBO resource exists with at least `size` bytes.
fn ensure_ibo(s: &mut VirglState, size: u32) {
    if s.ibo_res_id != 0 && s.ibo_size >= size {
        return;
    }
    if s.ibo_res_id != 0 {
        libsyscall::gpu_3d_resource_destroy(s.ibo_res_id);
    }
    let res = libsyscall::gpu_3d_resource_create(
        PIPE_BUFFER, PIPE_FORMAT_R8_UNORM, VIRGL_BIND_INDEX_BUFFER, size, 1,
    );
    s.ibo_res_id = res;
    s.ibo_size = size;
}

/// Upload raw bytes to a resource via RESOURCE_INLINE_WRITE.
fn inline_write(cmd: &mut CmdBuf, res_handle: u32, data: &[u8]) {
    let data_words = (data.len() + 3) / 4;
    // 11 header dwords + data
    cmd.push_cmd(VIRGL_CCMD_RESOURCE_INLINE_WRITE, 0, 11 + data_words as u32);
    cmd.push(res_handle);       // resource handle
    cmd.push(0);                // level
    cmd.push(0);                // usage
    cmd.push(0);                // stride (buffer = 0)
    cmd.push(0);                // layer_stride
    cmd.push(0);                // x
    cmd.push(0);                // y
    cmd.push(0);                // z
    cmd.push(data.len() as u32); // width = byte count
    cmd.push(1);                // height
    cmd.push(1);                // depth
    // Pack data into u32 words
    for i in 0..data_words {
        let base = i * 4;
        let mut word = 0u32;
        for j in 0..4 {
            if base + j < data.len() {
                word |= (data[base + j] as u32) << (j * 8);
            }
        }
        cmd.push(word);
    }
}

/// Upload the CPU-side constant buffer mirror to both VS and FS if dirty.
fn flush_const_buf(s: &mut VirglState) {
    if !s.const_buf_dirty || s.const_buf.is_empty() {
        return;
    }
    let data_len = s.const_buf.len() as u32;
    for &shader_type in &[PIPE_SHADER_VERTEX, PIPE_SHADER_FRAGMENT] {
        s.cmd.push_cmd(VIRGL_CCMD_SET_CONSTANT_BUFFER, 0, 2 + data_len);
        s.cmd.push(shader_type);
        s.cmd.push(0); // ubuf index 0
        for &v in &s.const_buf {
            s.cmd.push_f32(v);
        }
    }
    s.const_buf_dirty = false;
}

/// Create/update vertex elements object from attrib descriptors.
fn setup_vertex_elements(s: &mut VirglState, attribs: &[DrvAttrib]) {
    let n = attribs.len() as u32;
    if s.ve_handle != 0 && s.ve_num_attribs == n {
        // Already created with same count — reuse (attrib layout is stable per shader)
        return;
    }
    if s.ve_handle != 0 {
        s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_VERTEX_ELEMENTS, 1);
        s.cmd.push(s.ve_handle);
    }
    let handle = alloc_handle();
    // Payload: handle + 4 dwords per element
    s.cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_VERTEX_ELEMENTS, 1 + n * 4);
    s.cmd.push(handle);
    for attr in attribs {
        let fmt = match attr.components {
            1 => PIPE_FORMAT_R32_FLOAT,
            2 => PIPE_FORMAT_R32G32_FLOAT,
            3 => PIPE_FORMAT_R32G32B32_FLOAT,
            _ => PIPE_FORMAT_R32G32B32A32_FLOAT,
        };
        s.cmd.push(attr.offset);  // src_offset
        s.cmd.push(0);            // instance_divisor
        s.cmd.push(0);            // vertex_buffer_index
        s.cmd.push(fmt);          // src_format based on component count
    }
    s.ve_handle = handle;
    s.ve_num_attribs = n;
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_draw_arrays(
    mode: u32, first: u32, count: u32,
    vertex_data: *const u8, vertex_stride: u32,
    attribs: *const DrvAttrib, num_attribs: u32,
) {
    if vertex_data.is_null() || count == 0 || num_attribs == 0 {
        return;
    }
    let vdata_len = (count * vertex_stride) as usize;
    let vdata = unsafe { slice::from_raw_parts(vertex_data, vdata_len) };
    let attrs = unsafe { slice::from_raw_parts(attribs, num_attribs as usize) };
    let s = state();

    // 0. Upload uniforms (const buffer mirror) if dirty
    flush_const_buf(s);

    // 1. Create/reuse VBO resource and upload vertex data
    ensure_vbo(s, vdata_len as u32);
    inline_write(&mut s.cmd, s.vbo_res_id, vdata);

    // 2. Create vertex elements object
    setup_vertex_elements(s, attrs);

    // 3. Bind vertex elements
    s.cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_VERTEX_ELEMENTS, 1);
    s.cmd.push(s.ve_handle);

    // 4. Set vertex buffer: stride, offset, handle
    s.cmd.push_cmd(VIRGL_CCMD_SET_VERTEX_BUFFERS, 0, 3);
    s.cmd.push(vertex_stride);
    s.cmd.push(0);              // offset
    s.cmd.push(s.vbo_res_id);

    // 5. Draw
    s.cmd.push_cmd(VIRGL_CCMD_DRAW_VBO, 0, 12);
    s.cmd.push(0);              // start (vertex data already offset by libgl)
    s.cmd.push(count);          // count
    s.cmd.push(mode);           // mode
    s.cmd.push(0);              // indexed = false
    s.cmd.push(1);              // instance_count
    s.cmd.push(0);              // index_bias
    s.cmd.push(0);              // start_instance
    s.cmd.push(0);              // primitive_restart
    s.cmd.push(0);              // restart_index
    s.cmd.push(0);              // min_index
    s.cmd.push(count - 1);     // max_index
    s.cmd.push(0);              // cso
    let r = s.cmd.submit();
    if r != 0 { libsyscall::serial_println!("[virgl] draw_arrays submit FAILED: {}", r); }
}

#[unsafe(no_mangle)]
pub extern "C" fn drv_draw_elements(
    mode: u32, count: u32, index_type: u32,
    index_data: *const u8,
    vertex_data: *const u8, vertex_stride: u32,
    attribs: *const DrvAttrib, num_attribs: u32,
) {
    if vertex_data.is_null() || index_data.is_null() || count == 0 || num_attribs == 0 {
        return;
    }
    let attrs = unsafe { slice::from_raw_parts(attribs, num_attribs as usize) };
    let s = state();

    // 0. Upload uniforms (const buffer mirror) if dirty
    flush_const_buf(s);

    // Index element size from GL type
    let index_size = match index_type {
        0x1401 /* GL_UNSIGNED_BYTE */ => 1u32,
        0x1403 /* GL_UNSIGNED_SHORT */ => 2,
        0x1405 /* GL_UNSIGNED_INT */ => 4,
        _ => 2,
    };
    let index_data_len = count * index_size;
    let idata = unsafe { slice::from_raw_parts(index_data, index_data_len as usize) };

    // Find max index to determine vertex data extent
    let mut max_idx = 0u32;
    for i in 0..count as usize {
        let idx = match index_size {
            1 => idata[i] as u32,
            2 => u16::from_le_bytes([idata[i*2], idata[i*2+1]]) as u32,
            4 => u32::from_le_bytes([idata[i*4], idata[i*4+1], idata[i*4+2], idata[i*4+3]]),
            _ => 0,
        };
        if idx > max_idx { max_idx = idx; }
    }
    let vbo_bytes = (max_idx + 1) * vertex_stride;
    let vdata = unsafe { slice::from_raw_parts(vertex_data, vbo_bytes as usize) };

    // 1. Upload vertex data
    ensure_vbo(s, vbo_bytes);
    inline_write(&mut s.cmd, s.vbo_res_id, vdata);

    // 2. Upload index data
    ensure_ibo(s, index_data_len);
    inline_write(&mut s.cmd, s.ibo_res_id, idata);

    // 3. Setup vertex elements + bind
    setup_vertex_elements(s, attrs);
    s.cmd.push_cmd(VIRGL_CCMD_BIND_OBJECT, VIRGL_OBJECT_VERTEX_ELEMENTS, 1);
    s.cmd.push(s.ve_handle);

    // 4. Set vertex buffer
    s.cmd.push_cmd(VIRGL_CCMD_SET_VERTEX_BUFFERS, 0, 3);
    s.cmd.push(vertex_stride);
    s.cmd.push(0);
    s.cmd.push(s.vbo_res_id);

    // 5. Set index buffer: handle, index_size, offset
    s.cmd.push_cmd(VIRGL_CCMD_SET_INDEX_BUFFER, 0, 3);
    s.cmd.push(s.ibo_res_id);
    s.cmd.push(index_size);
    s.cmd.push(0); // offset

    // 6. Draw indexed
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
    s.cmd.push(max_idx);        // max_index
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

/// Read back the rendered color buffer into user-provided BGRA pixel buffer.
/// Called by gl_swap_buffers to get HW-rendered pixels into the software framebuffer.
#[unsafe(no_mangle)]
pub extern "C" fn drv_readback(buf: *mut u8, buf_len: u32) -> u32 {
    let s = state();
    if s.color_res_id == 0 || buf.is_null() || buf_len == 0 {
        return u32::MAX;
    }
    let out = unsafe { slice::from_raw_parts_mut(buf, buf_len as usize) };
    libsyscall::gpu_3d_surface_dma_read(s.color_res_id, out, s.width, s.height)
}

/// Upload a GL texture to a virgl resource and cache it.
/// Returns the virgl resource ID, or 0 on failure.
/// Subsequent calls with the same gl_tex_id return the cached resource ID.
#[unsafe(no_mangle)]
pub extern "C" fn drv_upload_texture(
    gl_tex_id: u32,
    data: *const u8,
    len: u32,
    width: u32,
    height: u32,
) -> u32 {
    if gl_tex_id == 0 || data.is_null() || len == 0 || width == 0 || height == 0 {
        return 0;
    }
    let s = state();

    // Return cached resource if already uploaded
    if let Some(&(_, res_id)) = s.tex_cache.iter().find(|&&(id, _)| id == gl_tex_id) {
        return res_id;
    }

    // Create a PIPE_TEXTURE_2D resource for sampling
    let res = libsyscall::gpu_3d_resource_create(
        PIPE_TEXTURE_2D, PIPE_FORMAT_B8G8R8A8_UNORM,
        VIRGL_BIND_SAMPLER_VIEW,
        width, height,
    );
    if res == u32::MAX {
        libsyscall::serial_println!("[virgl] drv_upload_texture: resource_create failed");
        return 0;
    }

    // Upload texture data via DMA transfer
    let tex_data = unsafe { slice::from_raw_parts(data, len as usize) };
    let r = libsyscall::gpu_3d_surface_dma(res, tex_data, width, height);
    if r != 0 {
        libsyscall::serial_println!("[virgl] drv_upload_texture: DMA failed: {}", r);
        libsyscall::gpu_3d_resource_destroy(res);
        return 0;
    }

    libsyscall::serial_println!("[virgl] drv_upload_texture: gl_id={} -> res={} ({}x{})", gl_tex_id, res, width, height);
    s.tex_cache.push((gl_tex_id, res));
    res
}

/// Bind a virgl texture resource to a fragment shader sampler slot.
///
/// Creates a VIRGL_OBJECT_SAMPLER_VIEW for the resource (cached per slot),
/// then issues SET_SAMPLER_VIEWS + BIND_SAMPLER_STATES for the FS stage.
#[unsafe(no_mangle)]
pub extern "C" fn drv_bind_sampler_view(slot: u32, virgl_res_id: u32) {
    if virgl_res_id == 0 || slot >= 8 { return; }
    let s = state();

    // If the same resource is already bound to this slot, just re-issue the bind commands
    // (SET_SAMPLER_VIEWS must be re-issued before every draw in virgl).
    let sv_handle = if s.sampler_view_res[slot as usize] == virgl_res_id
        && s.sampler_views[slot as usize] != 0
    {
        s.sampler_views[slot as usize]
    } else {
        // Destroy old sampler view for this slot if any
        let old_sv = s.sampler_views[slot as usize];
        if old_sv != 0 {
            s.cmd.push_cmd(VIRGL_CCMD_DESTROY_OBJECT, VIRGL_OBJECT_SAMPLER_VIEW, 1);
            s.cmd.push(old_sv);
        }

        // Create new VIRGL_OBJECT_SAMPLER_VIEW (6 payload words)
        let sv = alloc_handle();
        s.cmd.push_cmd(VIRGL_CCMD_CREATE_OBJECT, VIRGL_OBJECT_SAMPLER_VIEW, 6);
        s.cmd.push(sv);                            // handle
        s.cmd.push(virgl_res_id);                  // resource handle
        s.cmd.push(PIPE_FORMAT_B8G8R8A8_UNORM);    // format
        s.cmd.push(0);                             // val0: first_level=0 | first_layer=0
        s.cmd.push(0);                             // val1: last_level=0 | last_layer=0
        // swizzle: R=0, G=1, B=2, A=3 packed into bits [11:0]; tex_target at bits [31:24]
        let swizzle = 0u32 | (1 << 3) | (2 << 6) | (3 << 9);
        s.cmd.push(swizzle | (PIPE_TEXTURE_2D << 24));

        s.sampler_views[slot as usize] = sv;
        s.sampler_view_res[slot as usize] = virgl_res_id;
        sv
    };

    // SET_SAMPLER_VIEWS: shader, start_slot, handle[...]
    s.cmd.push_cmd(VIRGL_CCMD_SET_SAMPLER_VIEWS, 0, 3);
    s.cmd.push(PIPE_SHADER_FRAGMENT);
    s.cmd.push(slot);
    s.cmd.push(sv_handle);

    // BIND_SAMPLER_STATES: shader, start_slot, state_handle[...]
    s.cmd.push_cmd(VIRGL_CCMD_BIND_SAMPLER_STATES, 0, 3);
    s.cmd.push(PIPE_SHADER_FRAGMENT);
    s.cmd.push(slot);
    s.cmd.push(s.sampler_state_handle);

    s.cmd.submit();
    libsyscall::serial_println!("[virgl] drv_bind_sampler_view: slot={} res={} sv={}", slot, virgl_res_id, sv_handle);
}

// ── Panic handler ────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
