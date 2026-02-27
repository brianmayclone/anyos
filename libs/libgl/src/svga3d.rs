//! SVGA3D command buffer builder and GPU resource management.
//!
//! Builds SVGA3D command sequences as `Vec<u32>` word buffers,
//! then submits them to the kernel via `SYS_GPU_3D_SUBMIT`.
//! Each command has the format: `[cmd_id, size_bytes, payload...]`
//! where `size_bytes` is the byte count of the payload only.

use alloc::vec::Vec;

// ── SVGA3D command IDs (must match kernel constants) ──────

const CMD_SURFACE_DEFINE: u32     = 1040;
const CMD_SURFACE_DESTROY: u32    = 1041;
const CMD_SURFACE_DMA: u32        = 1044;
const CMD_CONTEXT_DEFINE: u32     = 1045;
const CMD_CONTEXT_DESTROY: u32    = 1046;
const CMD_SETRENDERSTATE: u32     = 1049;
const CMD_SETRENDERTARGET: u32    = 1050;
const CMD_SETTEXTURESTATE: u32    = 1051;
const CMD_SETVIEWPORT: u32        = 1055;
const CMD_CLEAR: u32              = 1057;
const CMD_PRESENT: u32            = 1058;
const CMD_SHADER_DEFINE: u32      = 1059;
const CMD_SHADER_DESTROY: u32     = 1060;
const CMD_SET_SHADER: u32         = 1061;
const CMD_SET_SHADER_CONST: u32   = 1062;
const CMD_DRAW_PRIMITIVES: u32    = 1063;
const CMD_SETSCISSORRECT: u32     = 1064;
const CMD_PRESENT_READBACK: u32   = 1070;

// ── Surface formats ──────────────────────────────────────

pub const SVGA3D_X8R8G8B8: u32  = 1;
pub const SVGA3D_A8R8G8B8: u32  = 2;
pub const SVGA3D_R5G6B5: u32    = 3;
pub const SVGA3D_Z_D16: u32     = 17;
pub const SVGA3D_Z_D24S8: u32   = 20;
pub const SVGA3D_Z_D24X8: u32   = 23;

// ── Surface flags ────────────────────────────────────────

pub const SVGA3D_SURFACE_HINT_RENDERTARGET: u32 = 1 << 4;
pub const SVGA3D_SURFACE_HINT_DEPTHSTENCIL: u32 = 1 << 5;
pub const SVGA3D_SURFACE_HINT_TEXTURE: u32      = 1 << 0;
pub const SVGA3D_SURFACE_HINT_VERTEXBUFFER: u32 = 1 << 6;
pub const SVGA3D_SURFACE_HINT_INDEXBUFFER: u32  = 1 << 7;

// ── Shader types ─────────────────────────────────────────

pub const SVGA3D_SHADERTYPE_VS: u32 = 1;
pub const SVGA3D_SHADERTYPE_PS: u32 = 2;

// ── Render target types ──────────────────────────────────

pub const SVGA3D_RT_COLOR0: u32  = 0;
pub const SVGA3D_RT_DEPTH: u32   = 1;
pub const SVGA3D_RT_STENCIL: u32 = 2;

// ── Render states ────────────────────────────────────────

pub const SVGA3D_RS_ZENABLE: u32         = 2;
pub const SVGA3D_RS_ZWRITEENABLE: u32    = 14;
pub const SVGA3D_RS_ALPHATESTENABLE: u32 = 15;
pub const SVGA3D_RS_SRCBLEND: u32        = 19;
pub const SVGA3D_RS_DSTBLEND: u32        = 20;
pub const SVGA3D_RS_CULLMODE: u32        = 22;
pub const SVGA3D_RS_ZFUNC: u32           = 23;
pub const SVGA3D_RS_BLENDENABLE: u32     = 27;
pub const SVGA3D_RS_COLORWRITEENABLE: u32 = 168;

// ── Render state values ──────────────────────────────────

// Cull modes
pub const SVGA3D_CULL_NONE: u32  = 1;
pub const SVGA3D_CULL_FRONT: u32 = 2;
pub const SVGA3D_CULL_BACK: u32  = 3;

// Comparison functions (for depth test)
pub const SVGA3D_CMP_NEVER: u32        = 1;
pub const SVGA3D_CMP_LESS: u32         = 2;
pub const SVGA3D_CMP_EQUAL: u32        = 3;
pub const SVGA3D_CMP_LESSEQUAL: u32    = 4;
pub const SVGA3D_CMP_GREATER: u32      = 5;
pub const SVGA3D_CMP_NOTEQUAL: u32     = 6;
pub const SVGA3D_CMP_GREATEREQUAL: u32 = 7;
pub const SVGA3D_CMP_ALWAYS: u32       = 8;

// Blend factors
pub const SVGA3D_BLEND_ZERO: u32             = 1;
pub const SVGA3D_BLEND_ONE: u32              = 2;
pub const SVGA3D_BLEND_SRCCOLOR: u32         = 3;
pub const SVGA3D_BLEND_INVSRCCOLOR: u32      = 4;
pub const SVGA3D_BLEND_SRCALPHA: u32         = 5;
pub const SVGA3D_BLEND_INVSRCALPHA: u32      = 6;
pub const SVGA3D_BLEND_DESTALPHA: u32        = 7;
pub const SVGA3D_BLEND_INVDESTALPHA: u32     = 8;
pub const SVGA3D_BLEND_DESTCOLOR: u32        = 9;
pub const SVGA3D_BLEND_INVDESTCOLOR: u32     = 10;

// Clear flags
pub const SVGA3D_CLEAR_COLOR: u32   = 1;
pub const SVGA3D_CLEAR_DEPTH: u32   = 2;
pub const SVGA3D_CLEAR_STENCIL: u32 = 4;

// Primitive types
pub const SVGA3D_PRIMITIVE_TRIANGLELIST: u32  = 5;
pub const SVGA3D_PRIMITIVE_TRIANGLESTRIP: u32 = 6;
pub const SVGA3D_PRIMITIVE_TRIANGLEFAN: u32   = 7;

// Vertex declaration types
pub const SVGA3D_DECLTYPE_FLOAT1: u32 = 0;
pub const SVGA3D_DECLTYPE_FLOAT2: u32 = 1;
pub const SVGA3D_DECLTYPE_FLOAT3: u32 = 2;
pub const SVGA3D_DECLTYPE_FLOAT4: u32 = 3;

// Vertex declaration usage
pub const SVGA3D_DECLUSAGE_POSITION: u32   = 0;
pub const SVGA3D_DECLUSAGE_BLENDWEIGHT: u32 = 1;
pub const SVGA3D_DECLUSAGE_NORMAL: u32     = 3;
pub const SVGA3D_DECLUSAGE_COLOR: u32      = 5;
pub const SVGA3D_DECLUSAGE_TEXCOORD: u32   = 7;

// ── GMR special IDs ──────────────────────────────────────

pub const SVGA_GMR_FRAMEBUFFER: u32 = 0xFFFF_FFFE;

// ── Shader constant types ────────────────────────────────

pub const SVGA3D_CONST_TYPE_FLOAT: u32 = 0;

// ══════════════════════════════════════════════════════════
// Command buffer builder
// ══════════════════════════════════════════════════════════

/// SVGA3D command buffer builder.
///
/// Accumulates commands as `Vec<u32>` and submits them to the kernel
/// via `SYS_GPU_3D_SUBMIT` in a single syscall.
pub struct CmdBuf {
    words: Vec<u32>,
}

impl CmdBuf {
    pub fn new() -> Self {
        Self { words: Vec::with_capacity(512) }
    }

    pub fn reset(&mut self) {
        self.words.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Submit the accumulated commands to the GPU and clear the buffer.
    /// Returns 0 on success.
    pub fn submit(&mut self) -> u32 {
        if self.words.is_empty() { return 0; }
        let result = crate::syscall::gpu_3d_submit(&self.words);
        self.words.clear();
        result
    }

    /// Append a raw command: `[cmd_id, size_bytes, payload...]`
    fn push_cmd(&mut self, cmd_id: u32, payload: &[u32]) {
        self.words.push(cmd_id);
        self.words.push((payload.len() * 4) as u32);
        self.words.extend_from_slice(payload);
    }

    // ── Context management ───────────────────────────────

    pub fn context_define(&mut self, cid: u32) {
        self.push_cmd(CMD_CONTEXT_DEFINE, &[cid]);
    }

    pub fn context_destroy(&mut self, cid: u32) {
        self.push_cmd(CMD_CONTEXT_DESTROY, &[cid]);
    }

    // ── Surface management ───────────────────────────────

    /// Define a 3D surface (color buffer, depth buffer, texture, or vertex buffer).
    /// Format: SVGA3dSurfaceFormat, flags: SVGA3D_SURFACE_HINT_*
    pub fn surface_define(&mut self, sid: u32, flags: u32, format: u32, w: u32, h: u32) {
        // SVGA3dCmdDefineSurface:
        //   sid, surfaceFlags, format,
        //   face[0].numMipLevels, face[1..5] = 0,
        //   SVGA3dSize { width, height, depth }
        self.push_cmd(CMD_SURFACE_DEFINE, &[
            sid, flags, format,
            1, 0, 0, 0, 0, 0, // face[0].numMipLevels=1, face[1..5]=0
            w, h, 1,           // mipSize: width, height, depth=1
        ]);
    }

    pub fn surface_destroy(&mut self, sid: u32) {
        self.push_cmd(CMD_SURFACE_DESTROY, &[sid]);
    }

    /// DMA transfer between guest memory (GMR) and a GPU surface.
    /// `guest_ptr` = `(gmr_id, offset_in_bytes)`.
    /// `box_` = `(x, y, z, w, h, d)` defining the transfer region.
    /// `suffix_flags`: SVGA3D_WRITE_HOST_VRAM=1, SVGA3D_READ_HOST_VRAM=2
    pub fn surface_dma(
        &mut self,
        guest_gmr_id: u32, guest_offset: u32,
        sid: u32, face: u32, mipmap: u32,
        transfer_flags: u32,
        box_x: u32, box_y: u32, box_w: u32, box_h: u32,
        src_x: u32, src_y: u32,
        src_pitch: u32,
    ) {
        // SVGA3dCmdSurfaceDMA:
        //   SVGAGuestImage guest { SVGAGuestPtr { gmr, offset }, pitch }
        //   SVGA3dSurfaceImageId host { sid, face, mipmap }
        //   SVGA3dTransferType (1=WRITE to host, 2=READ from host)
        // Suffix: one or more SVGA3dCopyBox
        //   SVGA3dCopyBox { x, y, z, w, h, d, srcx, srcy, srcz }
        // Then: SVGA3dCmdSurfaceDMASuffix { suffixSize, maximumOffset, flags }

        let mut payload = Vec::with_capacity(20);

        // guest image: { gmr_id, offset, pitch }
        payload.push(guest_gmr_id);
        payload.push(guest_offset);
        payload.push(src_pitch);

        // host image: { sid, face, mipmap }
        payload.push(sid);
        payload.push(face);
        payload.push(mipmap);

        // transfer type
        payload.push(transfer_flags);

        // 1 copy box: { x, y, z, w, h, d, srcx, srcy, srcz }
        payload.push(box_x);
        payload.push(box_y);
        payload.push(0); // z
        payload.push(box_w);
        payload.push(box_h);
        payload.push(1); // d
        payload.push(src_x);
        payload.push(src_y);
        payload.push(0); // srcz

        // suffix: { suffixSize, maximumOffset, flags }
        let suffix_size = 12u32; // 3 u32s = 12 bytes
        payload.push(suffix_size);
        payload.push(box_h * src_pitch); // maximumOffset (approx upper bound)
        payload.push(0); // flags (0 = default discard behavior)

        self.push_cmd(CMD_SURFACE_DMA, &payload);
    }

    // ── Render targets ───────────────────────────────────

    /// Set a render target (color or depth) for a context.
    pub fn set_render_target(&mut self, cid: u32, rt_type: u32, sid: u32) {
        // SVGA3dCmdSetRenderTarget:
        //   cid, type, target { sid, face=0, mipmap=0 }
        self.push_cmd(CMD_SETRENDERTARGET, &[cid, rt_type, sid, 0, 0]);
    }

    // ── Render state ─────────────────────────────────────

    /// Set a single render state value.
    pub fn set_render_state(&mut self, cid: u32, state: u32, value: u32) {
        // SVGA3dCmdSetRenderState:
        //   cid, SVGA3dRenderState[] { state, uintValue }
        self.push_cmd(CMD_SETRENDERSTATE, &[cid, state, value]);
    }

    /// Set multiple render states in one command.
    pub fn set_render_states(&mut self, cid: u32, states: &[(u32, u32)]) {
        let mut payload = Vec::with_capacity(1 + states.len() * 2);
        payload.push(cid);
        for &(state, value) in states {
            payload.push(state);
            payload.push(value);
        }
        self.push_cmd(CMD_SETRENDERSTATE, &payload);
    }

    // ── Viewport ─────────────────────────────────────────

    pub fn set_viewport(&mut self, cid: u32, x: f32, y: f32, w: f32, h: f32, min_depth: f32, max_depth: f32) {
        // SVGA3dCmdSetViewport:
        //   cid, SVGA3dViewport { float x, y, width, height, minDepth, maxDepth }
        self.push_cmd(CMD_SETVIEWPORT, &[
            cid,
            x.to_bits(), y.to_bits(), w.to_bits(), h.to_bits(),
            min_depth.to_bits(), max_depth.to_bits(),
        ]);
    }

    // ── Scissor ──────────────────────────────────────────

    pub fn set_scissor_rect(&mut self, cid: u32, x: u32, y: u32, w: u32, h: u32) {
        self.push_cmd(CMD_SETSCISSORRECT, &[cid, x, y, w, h]);
    }

    // ── Clear ────────────────────────────────────────────

    /// Clear color and/or depth buffer.
    pub fn clear(&mut self, cid: u32, flags: u32, color: u32, depth: f32, stencil: u32,
                 rects: &[(u32, u32, u32, u32)]) {
        // SVGA3dCmdClear:
        //   cid, clearFlag, color, depth(float), stencil,
        //   SVGA3dRect[] { x, y, w, h }
        let mut payload = Vec::with_capacity(5 + rects.len() * 4);
        payload.push(cid);
        payload.push(flags);
        payload.push(color);
        payload.push(depth.to_bits());
        payload.push(stencil);
        for &(x, y, w, h) in rects {
            payload.push(x);
            payload.push(y);
            payload.push(w);
            payload.push(h);
        }
        self.push_cmd(CMD_CLEAR, &payload);
    }

    // ── Shaders ──────────────────────────────────────────

    /// Define a shader (upload DX9 SM 2.0 bytecode).
    pub fn shader_define(&mut self, cid: u32, shid: u32, shader_type: u32, bytecode: &[u32]) {
        let mut payload = Vec::with_capacity(3 + bytecode.len());
        payload.push(cid);
        payload.push(shid);
        payload.push(shader_type);
        payload.extend_from_slice(bytecode);
        self.push_cmd(CMD_SHADER_DEFINE, &payload);
    }

    /// Destroy a shader.
    pub fn shader_destroy(&mut self, cid: u32, shid: u32, shader_type: u32) {
        self.push_cmd(CMD_SHADER_DESTROY, &[cid, shid, shader_type]);
    }

    /// Bind a shader to a context.
    pub fn set_shader(&mut self, cid: u32, shader_type: u32, shid: u32) {
        self.push_cmd(CMD_SET_SHADER, &[cid, shader_type, shid]);
    }

    /// Set a single float4 shader constant register.
    pub fn set_shader_const_f(&mut self, cid: u32, reg: u32, shader_type: u32, values: &[f32; 4]) {
        self.push_cmd(CMD_SET_SHADER_CONST, &[
            cid,
            reg,
            shader_type,
            SVGA3D_CONST_TYPE_FLOAT,
            values[0].to_bits(),
            values[1].to_bits(),
            values[2].to_bits(),
            values[3].to_bits(),
        ]);
    }

    // ── Draw primitives ──────────────────────────────────

    /// Issue a draw primitives command.
    ///
    /// `vertex_decls`: array of `SVGA3dVertexDecl` structs (each 4 u32s):
    ///   `[identity_stride, rangeHint_first, rangeHint_count, array_data_offset,
    ///     array_data_stride, array_data_surfaceId, declType, method, usage, usageIndex]`
    ///
    /// `prim_ranges`: array of `SVGA3dPrimitiveRange` (each 4 u32s):
    ///   `[primType, primitiveCount, indexArray_offset, indexArray_stride,
    ///     indexArray_surfaceId, indexWidth, indexBias]`
    pub fn draw_primitives(
        &mut self,
        cid: u32,
        num_vertex_decls: u32,
        num_ranges: u32,
        vertex_decl_words: &[u32],
        prim_range_words: &[u32],
    ) {
        let mut payload = Vec::with_capacity(3 + vertex_decl_words.len() + prim_range_words.len());
        payload.push(cid);
        payload.push(num_vertex_decls);
        payload.push(num_ranges);
        payload.extend_from_slice(vertex_decl_words);
        payload.extend_from_slice(prim_range_words);
        self.push_cmd(CMD_DRAW_PRIMITIVES, &payload);
    }

    // ── Present (display on screen) ──────────────────────

    /// Present a surface to the screen. Copies the rendered surface to display.
    pub fn present(&mut self, sid: u32, rects: &[(u32, u32, u32, u32)]) {
        // SVGA3dCmdPresent:
        //   sid, SVGA3dCopyRect[] { x, y, srcx, srcy, w, h }
        let mut payload = Vec::with_capacity(1 + rects.len() * 6);
        payload.push(sid);
        for &(x, y, w, h) in rects {
            payload.push(x);    // dest x
            payload.push(y);    // dest y
            payload.push(x);    // src x (same as dest for simple present)
            payload.push(y);    // src y
            payload.push(w);    // w
            payload.push(h);    // h
        }
        self.push_cmd(CMD_PRESENT, &payload);
    }

    /// Force GPU to flush render target contents so they can be read back via DMA.
    pub fn present_readback(&mut self) {
        self.push_cmd(CMD_PRESENT_READBACK, &[]);
    }

    // ── Texture state ────────────────────────────────────

    pub fn set_texture_state(&mut self, cid: u32, states: &[(u32, u32, u32)]) {
        // SVGA3dCmdSetTextureState:
        //   cid, SVGA3dTextureState[] { stage, name, value }
        let mut payload = Vec::with_capacity(1 + states.len() * 3);
        payload.push(cid);
        for &(stage, name, value) in states {
            payload.push(stage);
            payload.push(name);
            payload.push(value);
        }
        self.push_cmd(CMD_SETTEXTURESTATE, &payload);
    }
}

// ══════════════════════════════════════════════════════════
// GPU resource management
// ══════════════════════════════════════════════════════════

/// Simple monotonic ID allocator for GPU resources.
struct IdAllocator {
    next: u32,
}

impl IdAllocator {
    fn new(start: u32) -> Self { Self { next: start } }
    fn alloc(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }
}

/// Per-process SVGA3D state tracked in libgl.
pub struct Svga3dState {
    pub cmd: CmdBuf,
    pub context_id: u32,
    pub color_sid: u32,
    pub depth_sid: u32,
    next_surface_id: IdAllocator,
    next_shader_id: IdAllocator,
    pub width: u32,
    pub height: u32,
    pub initialized: bool,
}

impl Svga3dState {
    pub fn new() -> Self {
        Self {
            cmd: CmdBuf::new(),
            context_id: 0,
            color_sid: 0,
            depth_sid: 0,
            next_surface_id: IdAllocator::new(1),
            next_shader_id: IdAllocator::new(1),
            width: 0,
            height: 0,
            initialized: false,
        }
    }

    /// Allocate a new surface ID.
    pub fn alloc_surface(&mut self) -> u32 {
        self.next_surface_id.alloc()
    }

    /// Allocate a new shader ID.
    pub fn alloc_shader(&mut self) -> u32 {
        self.next_shader_id.alloc()
    }

    /// Initialize SVGA3D context, color surface, and depth surface.
    pub fn init(&mut self, width: u32, height: u32) -> bool {
        self.width = width;
        self.height = height;

        // Allocate resources
        self.context_id = 1;
        self.color_sid = self.alloc_surface();
        self.depth_sid = self.alloc_surface();

        // Create context
        self.cmd.context_define(self.context_id);

        // Create render target surfaces
        self.cmd.surface_define(
            self.color_sid,
            SVGA3D_SURFACE_HINT_RENDERTARGET,
            SVGA3D_A8R8G8B8,
            width, height,
        );
        self.cmd.surface_define(
            self.depth_sid,
            SVGA3D_SURFACE_HINT_DEPTHSTENCIL,
            SVGA3D_Z_D24S8,
            width, height,
        );

        // Bind render targets
        self.cmd.set_render_target(self.context_id, SVGA3D_RT_COLOR0, self.color_sid);
        self.cmd.set_render_target(self.context_id, SVGA3D_RT_DEPTH, self.depth_sid);

        // Set viewport (SVGA3D uses floats for viewport dimensions)
        self.cmd.set_viewport(self.context_id, 0.0, 0.0, width as f32, height as f32, 0.0, 1.0);

        // Set default render states
        self.cmd.set_render_states(self.context_id, &[
            (SVGA3D_RS_COLORWRITEENABLE, 0xF), // Write all RGBA channels
        ]);

        let result = self.cmd.submit();
        self.initialized = result == 0;
        self.initialized
    }

    /// Tear down SVGA3D resources.
    pub fn destroy(&mut self) {
        if !self.initialized { return; }
        self.cmd.surface_destroy(self.depth_sid);
        self.cmd.surface_destroy(self.color_sid);
        self.cmd.context_destroy(self.context_id);
        self.cmd.submit();
        self.initialized = false;
    }
}
