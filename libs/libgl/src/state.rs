//! OpenGL ES 2.0 state machine.
//!
//! `GlContext` holds all mutable GL state: viewport, clear color, bound objects,
//! capability flags, blend/depth/cull configuration, and vertex attribute pointers.

use alloc::vec::Vec;
use crate::types::*;
use crate::buffer::BufferStore;
use crate::texture::TextureStore;
use crate::shader::ShaderStore;
use crate::framebuffer::SwFramebuffer;

/// Maximum vertex attribute slots (OpenGL ES 2.0 guarantees at least 8).
pub const MAX_VERTEX_ATTRIBS: usize = 16;

/// Maximum texture units.
pub const MAX_TEXTURE_UNITS: usize = 8;

/// Per-attribute pointer configuration set by `glVertexAttribPointer`.
#[derive(Clone, Copy)]
pub struct VertexAttrib {
    /// Whether this attribute array is enabled.
    pub enabled: bool,
    /// Number of components (1, 2, 3, or 4).
    pub size: i32,
    /// Data type (GL_FLOAT, GL_BYTE, etc.).
    pub typ: GLenum,
    /// Whether fixed-point values are normalized.
    pub normalized: bool,
    /// Byte offset between consecutive attributes.
    pub stride: i32,
    /// Byte offset of the first component in the bound buffer.
    pub offset: usize,
    /// VBO that was bound to GL_ARRAY_BUFFER when this was set.
    pub buffer_id: u32,
}

impl Default for VertexAttrib {
    fn default() -> Self {
        Self {
            enabled: false,
            size: 4,
            typ: GL_FLOAT,
            normalized: false,
            stride: 0,
            offset: 0,
            buffer_id: 0,
        }
    }
}

/// Complete GL context state.
pub struct GlContext {
    // ── Viewport & Scissor ──────────────────────────────────────────────
    pub viewport_x: i32,
    pub viewport_y: i32,
    pub viewport_w: i32,
    pub viewport_h: i32,
    pub scissor_x: i32,
    pub scissor_y: i32,
    pub scissor_w: i32,
    pub scissor_h: i32,

    // ── Clear State ─────────────────────────────────────────────────────
    pub clear_r: f32,
    pub clear_g: f32,
    pub clear_b: f32,
    pub clear_a: f32,
    pub clear_depth: f32,

    // ── Capability Flags ────────────────────────────────────────────────
    pub depth_test: bool,
    pub blend: bool,
    pub cull_face: bool,
    pub scissor_test: bool,

    // ── Depth State ─────────────────────────────────────────────────────
    pub depth_func: GLenum,
    pub depth_mask: bool,

    // ── Blend State ─────────────────────────────────────────────────────
    pub blend_src_rgb: GLenum,
    pub blend_dst_rgb: GLenum,
    pub blend_src_alpha: GLenum,
    pub blend_dst_alpha: GLenum,

    // ── Cull State ──────────────────────────────────────────────────────
    pub cull_face_mode: GLenum,
    pub front_face: GLenum,

    // ── Color Mask ──────────────────────────────────────────────────────
    pub color_mask: [bool; 4],

    // ── Bound Objects ───────────────────────────────────────────────────
    pub bound_array_buffer: u32,
    pub bound_element_buffer: u32,
    pub active_texture_unit: u32,
    pub bound_textures: [u32; MAX_TEXTURE_UNITS],
    pub current_program: u32,
    pub bound_framebuffer: u32,

    // ── Vertex Attributes ───────────────────────────────────────────────
    pub attribs: [VertexAttrib; MAX_VERTEX_ATTRIBS],

    // ── Pixel Store ─────────────────────────────────────────────────────
    pub unpack_alignment: i32,
    pub pack_alignment: i32,

    // ── Line Width ──────────────────────────────────────────────────────
    pub line_width: f32,

    // ── Object Stores ───────────────────────────────────────────────────
    pub buffers: BufferStore,
    pub textures: TextureStore,
    pub shaders: ShaderStore,

    // ── Framebuffers ────────────────────────────────────────────────────
    pub default_fb: SwFramebuffer,
    pub fbo_color_tex: Vec<(u32, u32)>,

    // ── Error State ─────────────────────────────────────────────────────
    pub error: GLenum,
}

impl GlContext {
    /// Create a new context with default OpenGL ES 2.0 state.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            viewport_x: 0,
            viewport_y: 0,
            viewport_w: width as i32,
            viewport_h: height as i32,
            scissor_x: 0,
            scissor_y: 0,
            scissor_w: width as i32,
            scissor_h: height as i32,

            clear_r: 0.0,
            clear_g: 0.0,
            clear_b: 0.0,
            clear_a: 0.0,
            clear_depth: 1.0,

            depth_test: false,
            blend: false,
            cull_face: false,
            scissor_test: false,

            depth_func: GL_LESS,
            depth_mask: true,

            blend_src_rgb: GL_ONE,
            blend_dst_rgb: GL_ZERO,
            blend_src_alpha: GL_ONE,
            blend_dst_alpha: GL_ZERO,

            cull_face_mode: GL_BACK,
            front_face: GL_CCW,

            color_mask: [true; 4],

            bound_array_buffer: 0,
            bound_element_buffer: 0,
            active_texture_unit: 0,
            bound_textures: [0; MAX_TEXTURE_UNITS],
            current_program: 0,
            bound_framebuffer: 0,

            attribs: [VertexAttrib::default(); MAX_VERTEX_ATTRIBS],

            unpack_alignment: 4,
            pack_alignment: 4,

            line_width: 1.0,

            buffers: BufferStore::new(),
            textures: TextureStore::new(),
            shaders: ShaderStore::new(),

            default_fb: SwFramebuffer::new(width, height),
            fbo_color_tex: Vec::new(),

            error: GL_NO_ERROR,
        }
    }

    /// Record an error (only the first error is kept until glGetError clears it).
    pub fn set_error(&mut self, err: GLenum) {
        if self.error == GL_NO_ERROR {
            self.error = err;
        }
    }
}
