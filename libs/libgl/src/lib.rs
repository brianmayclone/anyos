//! libgl — OpenGL ES 2.0 software implementation for anyOS.
//!
//! Built as a `.so` shared library loaded via `dl_open`/`dl_sym`.
//! Provides a software rasterizer with GLSL shader support.
//!
//! # Architecture
//! - State machine in [`state::GlContext`]
//! - GLSL compiler: [`compiler`] (lexer → parser → AST → IR)
//! - Software rasterizer: [`rasterizer`] (vertex → clip → raster → fragment)
//! - Framebuffer: [`framebuffer::SwFramebuffer`] (ARGB color + f32 depth)
//!
//! # Export Convention
//! All public functions are `extern "C"` with `#[no_mangle]` for use via `dl_sym()`.

#![no_std]
#![no_main]
#![allow(unused, dead_code, static_mut_refs)]

extern crate alloc;

pub mod types;
pub mod state;
pub mod buffer;
pub mod texture;
pub mod shader;
pub mod framebuffer;
pub mod draw;
pub mod compiler;
pub mod rasterizer;
pub mod simd;
pub mod fxaa;
pub mod svga3d;
pub mod drv_loader;
pub mod physics;
mod syscall;

use types::*;
use state::GlContext;

// ── Allocator ───────────────────────────────────────────────────────────────

libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);

// ── Panic handler ───────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}

// ── Global GL context ───────────────────────────────────────────────────────

pub(crate) static mut CTX: Option<GlContext> = None;

/// Whether a hardware 3D backend is active (loaded via drv_loader).
pub(crate) static mut USE_HW_BACKEND: bool = false;

/// Fullscreen direct framebuffer state.
static mut FULLSCREEN_FB_PTR: *mut u32 = core::ptr::null_mut();
static mut FULLSCREEN_WIDTH: u32 = 0;
static mut FULLSCREEN_HEIGHT: u32 = 0;
static mut FULLSCREEN_STRIDE: u32 = 0; // in pixels (pitch / 4)

/// Whether the cursor is currently captured (hidden + grabbed).
static mut CURSOR_CAPTURED: bool = false;

/// Frame counter for diagnostic output (first N frames only).
pub(crate) static mut DIAG_FRAME: u32 = 0;

/// Raw pointers to texture state — avoids `&CTX` / `&mut CTX` aliasing UB
/// during rasterization when `real_tex_sample` needs read access while
/// `rasterize_triangle` holds `&mut GlContext`.
pub(crate) static mut TEX_STORE_PTR: *const crate::texture::TextureStore = core::ptr::null();
pub(crate) static mut BOUND_TEXTURES_PTR: *const [u32; crate::state::MAX_TEXTURE_UNITS] = core::ptr::null();

fn ctx() -> &'static mut GlContext {
    unsafe {
        CTX.as_mut().expect("gl_init not called")
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  anyOS Extensions
// ══════════════════════════════════════════════════════════════════════════════

/// Check that the CPU supports the SIMD instruction sets we compiled for.
///
/// Verifies SSE3, SSSE3, SSE4.1, SSE4.2 via CPUID leaf 1 ECX bits.
/// Prints a diagnostic to serial and terminates if any are missing.
/// Only relevant on x86_64 — ARM64 has mandatory NEON.
fn check_cpu_features() {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::__cpuid;

        let leaf1 = unsafe { __cpuid(1) };
        let ecx = leaf1.ecx;

        let sse3   = ecx & (1 << 0) != 0;
        let ssse3  = ecx & (1 << 9) != 0;
        let sse41  = ecx & (1 << 19) != 0;
        let sse42  = ecx & (1 << 20) != 0;

        if !sse3 || !ssse3 || !sse41 || !sse42 {
            serial_println!("[libgl] FATAL: CPU missing required SIMD features:");
            if !sse3  { serial_println!("[libgl]   - SSE3 not supported"); }
            if !ssse3 { serial_println!("[libgl]   - SSSE3 not supported"); }
            if !sse41 { serial_println!("[libgl]   - SSE4.1 not supported"); }
            if !sse42 { serial_println!("[libgl]   - SSE4.2 not supported"); }
            serial_println!("[libgl] Hint: use QEMU flag -cpu qemu64,+sse3,+ssse3,+sse4.1,+sse4.2");
            syscall::exit(1);
        }
    }
}

/// Initialize the GL context with the given framebuffer dimensions.
///
/// Attempts to load a hardware 3D driver (.drv) based on the GPU type.
/// Falls back to the software rasterizer if no driver is available.
#[no_mangle]
pub extern "C" fn gl_init(width: u32, height: u32) {
    check_cpu_features();

    unsafe {
        CTX = Some(GlContext::new(width, height));
    }

    // Try to load a hardware 3D driver based on the GPU type
    let hw_loaded = drv_loader::init(width, height);
    unsafe { USE_HW_BACKEND = hw_loaded; }

    serial_println!("[libgl] gl_init done ({}x{}, hw={})", width, height, hw_loaded);
}

/// Shut down libgl — unloads hardware driver and frees resources.
#[no_mangle]
pub extern "C" fn gl_deinit() {
    drv_loader::deinit();
    unsafe { USE_HW_BACKEND = false; }
    serial_println!("[libgl] gl_deinit done");
}

/// Resize the GL framebuffer without destroying shaders, buffers, or textures.
#[no_mangle]
pub extern "C" fn gl_resize(width: u32, height: u32) {
    let c = ctx();
    c.default_fb.resize(width, height);
}

/// Swap buffers — returns a pointer to the ARGB color buffer.
///
/// When using a hardware 3D driver, calls drv_flush + drv_present.
/// When using the software rasterizer, runs FXAA and returns the buffer pointer.
#[no_mangle]
pub extern "C" fn gl_swap_buffers() -> *const u32 {
    if unsafe { USE_HW_BACKEND } {
        if let Some(drv) = drv_loader::drv() {
            (drv.drv_flush)();
            (drv.drv_present)(0);

            // Read back HW-rendered pixels into the software color buffer
            if let Some(readback) = drv.drv_readback {
                let c = ctx();
                let buf = unsafe {
                    core::slice::from_raw_parts_mut(
                        c.default_fb.color.as_mut_ptr() as *mut u8,
                        c.default_fb.color.len() * 4,
                    )
                };
                readback(buf.as_mut_ptr(), buf.len() as u32);
                return c.default_fb.color.as_ptr();
            }
        }
    }

    let c = ctx();
    if c.fxaa_enabled {
        let w = c.default_fb.width;
        let h = c.default_fb.height;
        fxaa::apply(&mut c.default_fb.color, w, h);
    }
    c.default_fb.color.as_ptr()
}

/// Get a pointer to the backbuffer (same as swap_buffers for single-buffered SW).
#[no_mangle]
pub extern "C" fn gl_get_backbuffer() -> *const u32 {
    let c = ctx();
    c.default_fb.color.as_ptr()
}

/// Initialize fullscreen direct framebuffer rendering.
///
/// `fb_ptr` is the mapped framebuffer address (from sys_grant_framebuffer).
/// `width`/`height` are the screen dimensions.
/// `stride` is the row stride in pixels (pitch / 4).
///
/// After calling this, use `gl_swap_buffers_fullscreen()` instead of `gl_swap_buffers()`.
/// The rasterizer still renders to its internal buffer, but swap copies directly
/// to the mapped framebuffer, bypassing the compositor.
#[no_mangle]
pub extern "C" fn gl_init_fullscreen(fb_ptr: *mut u32, width: u32, height: u32, stride: u32) {
    unsafe {
        FULLSCREEN_FB_PTR = fb_ptr;
        FULLSCREEN_WIDTH = width;
        FULLSCREEN_HEIGHT = height;
        FULLSCREEN_STRIDE = stride;
    }

    // Resize the internal framebuffer to match the screen if needed
    let c = ctx();
    if c.default_fb.width != width || c.default_fb.height != height {
        c.default_fb.resize(width, height);
    }

    serial_println!("[libgl] Fullscreen mode: fb_ptr={:#x}, {}x{}, stride={}",
        fb_ptr as usize, width, height, stride);
}

/// Exit fullscreen mode — stop writing to the direct framebuffer.
#[no_mangle]
pub extern "C" fn gl_exit_fullscreen() {
    unsafe {
        FULLSCREEN_FB_PTR = core::ptr::null_mut();
        FULLSCREEN_WIDTH = 0;
        FULLSCREEN_HEIGHT = 0;
        FULLSCREEN_STRIDE = 0;
    }
    serial_println!("[libgl] Fullscreen mode exited");
}

/// Set cursor captured state. When captured, the cursor should be hidden
/// and mouse input delivers relative deltas (infinite movement).
/// Returns the previous captured state (0 or 1).
#[no_mangle]
pub extern "C" fn gl_set_cursor_captured(captured: u32) -> u32 {
    unsafe {
        let prev = if CURSOR_CAPTURED { 1u32 } else { 0 };
        CURSOR_CAPTURED = captured != 0;
        prev
    }
}

/// Query whether the cursor is currently captured.
#[no_mangle]
pub extern "C" fn gl_get_cursor_captured() -> u32 {
    unsafe { if CURSOR_CAPTURED { 1 } else { 0 } }
}

/// Swap buffers in fullscreen mode — copies the rendered frame directly to the
/// mapped framebuffer. Handles stride != width (pitch-aligned rows).
///
/// Returns true if the copy was performed, false if not in fullscreen mode.
#[no_mangle]
pub extern "C" fn gl_swap_buffers_fullscreen() -> u32 {
    let fb_ptr = unsafe { FULLSCREEN_FB_PTR };
    if fb_ptr.is_null() {
        return 0;
    }

    let c = ctx();

    // Apply FXAA if enabled
    if c.fxaa_enabled {
        let w = c.default_fb.width;
        let h = c.default_fb.height;
        fxaa::apply(&mut c.default_fb.color, w, h);
    }

    let width = unsafe { FULLSCREEN_WIDTH } as usize;
    let height = unsafe { FULLSCREEN_HEIGHT } as usize;
    let stride = unsafe { FULLSCREEN_STRIDE } as usize;
    let src = c.default_fb.color.as_ptr();

    // Copy row-by-row if stride != width, or memcpy the whole thing if equal
    unsafe {
        if stride == width {
            core::ptr::copy_nonoverlapping(src, fb_ptr, width * height);
        } else {
            let copy_w = width.min(c.default_fb.width as usize);
            let copy_h = height.min(c.default_fb.height as usize);
            for y in 0..copy_h {
                let src_row = src.add(y * c.default_fb.width as usize);
                let dst_row = fb_ptr.add(y * stride);
                core::ptr::copy_nonoverlapping(src_row, dst_row, copy_w);
            }
        }
    }

    1
}

// ══════════════════════════════════════════════════════════════════════════════
//  State Management
// ══════════════════════════════════════════════════════════════════════════════

/// Get the current error code and clear it.
#[no_mangle]
pub extern "C" fn glGetError() -> GLenum {
    let c = ctx();
    let err = c.error;
    c.error = GL_NO_ERROR;
    err
}

/// Get a string describing the GL implementation.
#[no_mangle]
pub extern "C" fn glGetString(name: GLenum) -> *const u8 {
    match name {
        GL_VENDOR => b"anyOS\0".as_ptr(),
        GL_RENDERER => {
            if unsafe { USE_HW_BACKEND } {
                b"anyOS GPU Accelerated\0".as_ptr()
            } else {
                b"anyOS Software Rasterizer\0".as_ptr()
            }
        }
        GL_VERSION => b"OpenGL ES 2.0 (anyOS libgl 2.0)\0".as_ptr(),
        GL_SHADING_LANGUAGE_VERSION => b"GLSL ES 1.00\0".as_ptr(),
        _ => core::ptr::null(),
    }
}

/// Enable a capability.
#[no_mangle]
pub extern "C" fn glEnable(cap: GLenum) {
    let c = ctx();
    match cap {
        GL_DEPTH_TEST => c.depth_test = true,
        GL_BLEND => c.blend = true,
        GL_CULL_FACE_CAP => c.cull_face = true,
        GL_SCISSOR_TEST => c.scissor_test = true,
        _ => c.set_error(GL_INVALID_ENUM),
    }
}

/// Disable a capability.
#[no_mangle]
pub extern "C" fn glDisable(cap: GLenum) {
    let c = ctx();
    match cap {
        GL_DEPTH_TEST => c.depth_test = false,
        GL_BLEND => c.blend = false,
        GL_CULL_FACE_CAP => c.cull_face = false,
        GL_SCISSOR_TEST => c.scissor_test = false,
        _ => c.set_error(GL_INVALID_ENUM),
    }
}

/// Set the blend function.
#[no_mangle]
pub extern "C" fn glBlendFunc(sfactor: GLenum, dfactor: GLenum) {
    let c = ctx();
    c.blend_src_rgb = sfactor;
    c.blend_dst_rgb = dfactor;
    c.blend_src_alpha = sfactor;
    c.blend_dst_alpha = dfactor;
}

/// Set separate blend functions for RGB and alpha.
#[no_mangle]
pub extern "C" fn glBlendFuncSeparate(
    src_rgb: GLenum, dst_rgb: GLenum,
    src_alpha: GLenum, dst_alpha: GLenum,
) {
    let c = ctx();
    c.blend_src_rgb = src_rgb;
    c.blend_dst_rgb = dst_rgb;
    c.blend_src_alpha = src_alpha;
    c.blend_dst_alpha = dst_alpha;
}

/// Set the depth comparison function.
#[no_mangle]
pub extern "C" fn glDepthFunc(func: GLenum) {
    ctx().depth_func = func;
}

/// Enable/disable writing to the depth buffer.
#[no_mangle]
pub extern "C" fn glDepthMask(flag: GLboolean) {
    ctx().depth_mask = flag != 0;
}

/// Set face culling mode.
#[no_mangle]
pub extern "C" fn glCullFace(mode: GLenum) {
    ctx().cull_face_mode = mode;
}

/// Set front-face winding order.
#[no_mangle]
pub extern "C" fn glFrontFace(mode: GLenum) {
    ctx().front_face = mode;
}

/// Set the viewport.
#[no_mangle]
pub extern "C" fn glViewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei) {
    let c = ctx();
    c.viewport_x = x;
    c.viewport_y = y;
    c.viewport_w = width;
    c.viewport_h = height;

    // Update hardware viewport if HW backend is active
    if unsafe { USE_HW_BACKEND } {
        if let Some(drv) = drv_loader::drv() {
            (drv.drv_set_viewport)(x as u32, y as u32, width as u32, height as u32);
        }
    }
}

/// Set the clear color.
#[no_mangle]
pub extern "C" fn glClearColor(red: GLclampf, green: GLclampf, blue: GLclampf, alpha: GLclampf) {
    let c = ctx();
    c.clear_r = red;
    c.clear_g = green;
    c.clear_b = blue;
    c.clear_a = alpha;
}

/// Clear buffers.
#[no_mangle]
pub extern "C" fn glClear(mask: GLbitfield) {
    let c = ctx();

    // Hardware clear via loaded driver
    if unsafe { USE_HW_BACKEND } {
        if let Some(drv) = drv_loader::drv() {
            (drv.drv_clear)(mask, c.clear_r, c.clear_g, c.clear_b, c.clear_a, c.clear_depth);
        }
    }

    // Always clear the software framebuffer too (for state consistency)
    if mask & GL_COLOR_BUFFER_BIT != 0 {
        let r = (c.clear_r.clamp(0.0, 1.0) * 255.0) as u32;
        let g = (c.clear_g.clamp(0.0, 1.0) * 255.0) as u32;
        let b = (c.clear_b.clamp(0.0, 1.0) * 255.0) as u32;
        let a = (c.clear_a.clamp(0.0, 1.0) * 255.0) as u32;
        let argb = (a << 24) | (r << 16) | (g << 8) | b;
        c.default_fb.clear_color(argb);
    }
    if mask & GL_DEPTH_BUFFER_BIT != 0 {
        c.default_fb.clear_depth(c.clear_depth);
    }
}

/// Set scissor rectangle.
#[no_mangle]
pub extern "C" fn glScissor(x: GLint, y: GLint, width: GLsizei, height: GLsizei) {
    let c = ctx();
    c.scissor_x = x;
    c.scissor_y = y;
    c.scissor_w = width;
    c.scissor_h = height;
}

/// Set line width (not fully implemented in SW rasterizer).
#[no_mangle]
pub extern "C" fn glLineWidth(width: GLfloat) {
    ctx().line_width = width;
}

/// Set pixel storage modes.
#[no_mangle]
pub extern "C" fn glPixelStorei(pname: GLenum, param: GLint) {
    let c = ctx();
    match pname {
        GL_UNPACK_ALIGNMENT => c.unpack_alignment = param,
        GL_PACK_ALIGNMENT => c.pack_alignment = param,
        _ => c.set_error(GL_INVALID_ENUM),
    }
}

/// Set color mask.
#[no_mangle]
pub extern "C" fn glColorMask(red: GLboolean, green: GLboolean, blue: GLboolean, alpha: GLboolean) {
    let c = ctx();
    c.color_mask = [red != 0, green != 0, blue != 0, alpha != 0];
}

// ══════════════════════════════════════════════════════════════════════════════
//  Buffer Objects
// ══════════════════════════════════════════════════════════════════════════════

/// Generate buffer names.
#[no_mangle]
pub extern "C" fn glGenBuffers(n: GLsizei, buffers: *mut GLuint) {
    if n <= 0 || buffers.is_null() { return; }
    let ids = unsafe { core::slice::from_raw_parts_mut(buffers, n as usize) };
    ctx().buffers.gen(n, ids);
}

/// Delete buffer objects.
#[no_mangle]
pub extern "C" fn glDeleteBuffers(n: GLsizei, buffers: *const GLuint) {
    if n <= 0 || buffers.is_null() { return; }
    let ids = unsafe { core::slice::from_raw_parts(buffers, n as usize) };
    ctx().buffers.delete(n, ids);
}

/// Bind a buffer to a target.
#[no_mangle]
pub extern "C" fn glBindBuffer(target: GLenum, buffer: GLuint) {
    let c = ctx();
    match target {
        GL_ARRAY_BUFFER => c.bound_array_buffer = buffer,
        GL_ELEMENT_ARRAY_BUFFER => c.bound_element_buffer = buffer,
        _ => c.set_error(GL_INVALID_ENUM),
    }
}

/// Upload data to the currently bound buffer.
#[no_mangle]
pub extern "C" fn glBufferData(target: GLenum, size: GLsizeiptr, data: *const GLvoid, usage: GLenum) {
    let c = ctx();
    let id = match target {
        GL_ARRAY_BUFFER => c.bound_array_buffer,
        GL_ELEMENT_ARRAY_BUFFER => c.bound_element_buffer,
        _ => { c.set_error(GL_INVALID_ENUM); return; }
    };
    if id == 0 { c.set_error(GL_INVALID_OPERATION); return; }

    let bytes = if data.is_null() {
        alloc::vec![0u8; size as usize]
    } else {
        let slice = unsafe { core::slice::from_raw_parts(data as *const u8, size as usize) };
        slice.to_vec()
    };
    c.buffers.buffer_data(id, &bytes, usage);
}

/// Update a sub-region of a buffer.
#[no_mangle]
pub extern "C" fn glBufferSubData(target: GLenum, offset: GLintptr, size: GLsizeiptr, data: *const GLvoid) {
    let c = ctx();
    let id = match target {
        GL_ARRAY_BUFFER => c.bound_array_buffer,
        GL_ELEMENT_ARRAY_BUFFER => c.bound_element_buffer,
        _ => { c.set_error(GL_INVALID_ENUM); return; }
    };
    if id == 0 || data.is_null() { return; }
    let slice = unsafe { core::slice::from_raw_parts(data as *const u8, size as usize) };
    c.buffers.buffer_sub_data(id, offset as usize, slice);
}

// ══════════════════════════════════════════════════════════════════════════════
//  Texture Objects
// ══════════════════════════════════════════════════════════════════════════════

/// Generate texture names.
#[no_mangle]
pub extern "C" fn glGenTextures(n: GLsizei, textures: *mut GLuint) {
    if n <= 0 || textures.is_null() { return; }
    let ids = unsafe { core::slice::from_raw_parts_mut(textures, n as usize) };
    ctx().textures.gen(n, ids);
}

/// Delete texture objects.
#[no_mangle]
pub extern "C" fn glDeleteTextures(n: GLsizei, textures: *const GLuint) {
    if n <= 0 || textures.is_null() { return; }
    let ids = unsafe { core::slice::from_raw_parts(textures, n as usize) };
    ctx().textures.delete(n, ids);
}

/// Bind a texture to the active texture unit.
#[no_mangle]
pub extern "C" fn glBindTexture(target: GLenum, texture: GLuint) {
    let c = ctx();
    if target != GL_TEXTURE_2D { c.set_error(GL_INVALID_ENUM); return; }
    let unit = c.active_texture_unit as usize;
    if unit < state::MAX_TEXTURE_UNITS {
        c.bound_textures[unit] = texture;
    }
}

/// Upload texture image data.
#[no_mangle]
pub extern "C" fn glTexImage2D(
    target: GLenum, _level: GLint, internal_format: GLint,
    width: GLsizei, height: GLsizei, _border: GLint,
    format: GLenum, _type: GLenum, data: *const GLvoid,
) {
    let c = ctx();
    if target != GL_TEXTURE_2D { c.set_error(GL_INVALID_ENUM); return; }
    let unit = c.active_texture_unit as usize;
    if unit >= state::MAX_TEXTURE_UNITS { return; }
    let tex_id = c.bound_textures[unit];

    let pixel_size = match format {
        GL_RGBA => 4,
        GL_RGB => 3,
        GL_LUMINANCE => 1,
        GL_ALPHA => 1,
        GL_LUMINANCE_ALPHA => 2,
        _ => 4,
    };
    let data_slice = if data.is_null() {
        None
    } else {
        let len = width as usize * height as usize * pixel_size;
        Some(unsafe { core::slice::from_raw_parts(data as *const u8, len) })
    };

    c.textures.tex_image_2d(tex_id, width as u32, height as u32, format, data_slice);
    let _ = internal_format;
}

/// Update a sub-region of a texture.
#[no_mangle]
pub extern "C" fn glTexSubImage2D(
    target: GLenum, _level: GLint,
    _xoffset: GLint, _yoffset: GLint,
    _width: GLsizei, _height: GLsizei,
    _format: GLenum, _type: GLenum, _data: *const GLvoid,
) {
    if target != GL_TEXTURE_2D {
        ctx().set_error(GL_INVALID_ENUM);
    }
    // TODO: implement sub-image update
}

/// Set texture parameter.
#[no_mangle]
pub extern "C" fn glTexParameteri(target: GLenum, pname: GLenum, param: GLint) {
    let c = ctx();
    if target != GL_TEXTURE_2D { c.set_error(GL_INVALID_ENUM); return; }
    let unit = c.active_texture_unit as usize;
    if unit >= state::MAX_TEXTURE_UNITS { return; }
    let tex_id = c.bound_textures[unit];
    if let Some(tex) = c.textures.get_mut(tex_id) {
        match pname {
            GL_TEXTURE_MIN_FILTER => tex.min_filter = param as u32,
            GL_TEXTURE_MAG_FILTER => tex.mag_filter = param as u32,
            GL_TEXTURE_WRAP_S => tex.wrap_s = param as u32,
            GL_TEXTURE_WRAP_T => tex.wrap_t = param as u32,
            _ => {}
        }
    }
}

/// Set active texture unit.
#[no_mangle]
pub extern "C" fn glActiveTexture(texture: GLenum) {
    let unit = texture.wrapping_sub(GL_TEXTURE0);
    if (unit as usize) < state::MAX_TEXTURE_UNITS {
        ctx().active_texture_unit = unit;
    }
}

/// Generate mipmaps (no-op in Phase 1).
#[no_mangle]
pub extern "C" fn glGenerateMipmap(_target: GLenum) {
    // Mipmap generation not implemented in Phase 1
}

// ══════════════════════════════════════════════════════════════════════════════
//  Shader Objects
// ══════════════════════════════════════════════════════════════════════════════

/// Create a shader object.
#[no_mangle]
pub extern "C" fn glCreateShader(shader_type: GLenum) -> GLuint {
    ctx().shaders.create_shader(shader_type)
}

/// Delete a shader object.
#[no_mangle]
pub extern "C" fn glDeleteShader(shader: GLuint) {
    ctx().shaders.delete_shader(shader);
}

/// Set shader source code.
#[no_mangle]
pub extern "C" fn glShaderSource(
    shader: GLuint, count: GLsizei,
    string: *const *const GLchar, length: *const GLint,
) {
    if string.is_null() || count <= 0 { return; }
    let c = ctx();

    let mut source = alloc::string::String::new();
    for i in 0..count as usize {
        let str_ptr = unsafe { *string.add(i) };
        if str_ptr.is_null() { continue; }

        let len = if length.is_null() {
            // Null-terminated
            let mut l = 0;
            unsafe { while *str_ptr.add(l) != 0 { l += 1; } }
            l
        } else {
            let l = unsafe { *length.add(i) };
            if l < 0 {
                let mut l = 0;
                unsafe { while *str_ptr.add(l) != 0 { l += 1; } }
                l
            } else {
                l as usize
            }
        };

        let slice = unsafe { core::slice::from_raw_parts(str_ptr, len) };
        if let Ok(s) = core::str::from_utf8(slice) {
            source.push_str(s);
        }
    }

    if let Some(s) = c.shaders.get_shader_mut(shader) {
        s.source = source;
    }
}

/// Compile a shader.
#[no_mangle]
pub extern "C" fn glCompileShader(shader: GLuint) {
    ctx().shaders.compile_shader(shader);
}

/// Query shader parameters.
#[no_mangle]
pub extern "C" fn glGetShaderiv(shader: GLuint, pname: GLenum, params: *mut GLint) {
    if params.is_null() { return; }
    let c = ctx();
    let val = match c.shaders.get_shader(shader) {
        Some(s) => match pname {
            GL_COMPILE_STATUS => if s.compiled { 1 } else { 0 },
            GL_SHADER_TYPE => s.shader_type as i32,
            GL_INFO_LOG_LENGTH => s.info_log.len() as i32 + 1,
            _ => 0,
        },
        None => 0,
    };
    unsafe { *params = val; }
}

/// Get shader info log.
#[no_mangle]
pub extern "C" fn glGetShaderInfoLog(
    shader: GLuint, max_length: GLsizei,
    length: *mut GLsizei, info_log: *mut GLchar,
) {
    let c = ctx();
    let log = match c.shaders.get_shader(shader) {
        Some(s) => &s.info_log,
        None => return,
    };
    let copy_len = log.len().min((max_length as usize).saturating_sub(1));
    if !info_log.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(log.as_ptr(), info_log, copy_len);
            *info_log.add(copy_len) = 0;
        }
    }
    if !length.is_null() {
        unsafe { *length = copy_len as i32; }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  Program Objects
// ══════════════════════════════════════════════════════════════════════════════

/// Create a program object.
#[no_mangle]
pub extern "C" fn glCreateProgram() -> GLuint {
    ctx().shaders.create_program()
}

/// Delete a program object.
#[no_mangle]
pub extern "C" fn glDeleteProgram(program: GLuint) {
    ctx().shaders.delete_program(program);
}

/// Attach a shader to a program.
#[no_mangle]
pub extern "C" fn glAttachShader(program: GLuint, shader: GLuint) {
    let c = ctx();
    let shader_type = match c.shaders.get_shader(shader) {
        Some(s) => s.shader_type,
        None => return,
    };
    if let Some(prog) = c.shaders.get_program_mut(program) {
        match shader_type {
            GL_VERTEX_SHADER => prog.vertex_shader = shader,
            GL_FRAGMENT_SHADER => prog.fragment_shader = shader,
            _ => {}
        }
    }
}

/// Link a program.
#[no_mangle]
pub extern "C" fn glLinkProgram(program: GLuint) {
    ctx().shaders.link_program(program);
}

/// Use a program for rendering.
#[no_mangle]
pub extern "C" fn glUseProgram(program: GLuint) {
    let c = ctx();
    c.current_program = program;

    if unsafe { USE_HW_BACKEND } {
        if let Some(prog) = c.shaders.get_program(program) {
            if prog.hw_program_id != 0 {
                if let Some(drv) = drv_loader::drv() {
                    (drv.drv_use_program)(prog.hw_program_id);
                }
            }
        }
    }
}

/// Query program parameters.
#[no_mangle]
pub extern "C" fn glGetProgramiv(program: GLuint, pname: GLenum, params: *mut GLint) {
    if params.is_null() { return; }
    let c = ctx();
    let val = match c.shaders.get_program(program) {
        Some(p) => match pname {
            GL_LINK_STATUS => if p.linked { 1 } else { 0 },
            GL_INFO_LOG_LENGTH => p.info_log.len() as i32 + 1,
            _ => 0,
        },
        None => 0,
    };
    unsafe { *params = val; }
}

/// Get program info log.
#[no_mangle]
pub extern "C" fn glGetProgramInfoLog(
    program: GLuint, max_length: GLsizei,
    length: *mut GLsizei, info_log: *mut GLchar,
) {
    let c = ctx();
    let log = match c.shaders.get_program(program) {
        Some(p) => &p.info_log,
        None => return,
    };
    let copy_len = log.len().min((max_length as usize).saturating_sub(1));
    if !info_log.is_null() && copy_len > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(log.as_ptr(), info_log, copy_len);
            *info_log.add(copy_len) = 0;
        }
    }
    if !length.is_null() {
        unsafe { *length = copy_len as i32; }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  Uniforms & Attributes
// ══════════════════════════════════════════════════════════════════════════════

/// Get the location of a uniform variable.
#[no_mangle]
pub extern "C" fn glGetUniformLocation(program: GLuint, name: *const GLchar) -> GLint {
    if name.is_null() { return -1; }
    let name_str = unsafe { cstr_to_str(name) };
    let c = ctx();
    match c.shaders.get_program(program) {
        Some(p) => {
            for u in &p.uniforms {
                if u.name == name_str { return u.location; }
            }
            -1
        }
        None => -1,
    }
}

/// Get the location of an attribute variable.
#[no_mangle]
pub extern "C" fn glGetAttribLocation(program: GLuint, name: *const GLchar) -> GLint {
    if name.is_null() { return -1; }
    let name_str = unsafe { cstr_to_str(name) };
    let c = ctx();
    match c.shaders.get_program(program) {
        Some(p) => {
            for a in &p.attributes {
                if a.name == name_str { return a.location; }
            }
            -1
        }
        None => -1,
    }
}

/// Bind an attribute to a specific location.
#[no_mangle]
pub extern "C" fn glBindAttribLocation(program: GLuint, index: GLuint, name: *const GLchar) {
    if name.is_null() { return; }
    let name_str = unsafe { cstr_to_str(name) };
    let c = ctx();
    if let Some(p) = c.shaders.get_program_mut(program) {
        p.attrib_bindings.push((alloc::string::String::from(name_str), index as i32));
    }
}

/// Set a 1-int uniform (typically for sampler bindings).
#[no_mangle]
pub extern "C" fn glUniform1i(location: GLint, v0: GLint) {
    set_uniform_floats(location, &[v0 as f32, 0.0, 0.0, 0.0]);
    // Also set sampler unit
    let c = ctx();
    let prog_id = c.current_program;
    if let Some(p) = c.shaders.get_program_mut(prog_id) {
        if let Some(u) = p.uniforms.iter_mut().find(|u| u.location == location) {
            u.sampler_unit = v0;
        }
    }
}

/// Set a 1-float uniform.
#[no_mangle]
pub extern "C" fn glUniform1f(location: GLint, v0: GLfloat) {
    set_uniform_floats(location, &[v0, v0, v0, v0]);
}

/// Set a 2-float uniform.
#[no_mangle]
pub extern "C" fn glUniform2f(location: GLint, v0: GLfloat, v1: GLfloat) {
    set_uniform_floats(location, &[v0, v1, 0.0, 0.0]);
}

/// Set a 3-float uniform.
#[no_mangle]
pub extern "C" fn glUniform3f(location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat) {
    set_uniform_floats(location, &[v0, v1, v2, 0.0]);
}

/// Set a 4-float uniform.
#[no_mangle]
pub extern "C" fn glUniform4f(location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat, v3: GLfloat) {
    set_uniform_floats(location, &[v0, v1, v2, v3]);
}

/// Set a 4x4 matrix uniform.
#[no_mangle]
pub extern "C" fn glUniformMatrix4fv(
    location: GLint, _count: GLsizei, _transpose: GLboolean, value: *const GLfloat,
) {
    if value.is_null() { return; }
    let vals = unsafe { core::slice::from_raw_parts(value, 16) };
    let c = ctx();
    let prog_id = c.current_program;
    if let Some(p) = c.shaders.get_program_mut(prog_id) {
        if let Some(u) = p.uniforms.iter_mut().find(|u| u.location == location) {
            u.value[..16].copy_from_slice(vals);
        }
    }
}

/// Enable a vertex attribute array.
#[no_mangle]
pub extern "C" fn glEnableVertexAttribArray(index: GLuint) {
    let c = ctx();
    if (index as usize) < state::MAX_VERTEX_ATTRIBS {
        c.attribs[index as usize].enabled = true;
    }
}

/// Disable a vertex attribute array.
#[no_mangle]
pub extern "C" fn glDisableVertexAttribArray(index: GLuint) {
    let c = ctx();
    if (index as usize) < state::MAX_VERTEX_ATTRIBS {
        c.attribs[index as usize].enabled = false;
    }
}

/// Define a vertex attribute pointer.
#[no_mangle]
pub extern "C" fn glVertexAttribPointer(
    index: GLuint, size: GLint, type_: GLenum,
    normalized: GLboolean, stride: GLsizei, pointer: *const GLvoid,
) {
    let c = ctx();
    if (index as usize) >= state::MAX_VERTEX_ATTRIBS { return; }
    c.attribs[index as usize] = state::VertexAttrib {
        enabled: c.attribs[index as usize].enabled,
        size,
        typ: type_,
        normalized: normalized != 0,
        stride,
        offset: pointer as usize,
        buffer_id: c.bound_array_buffer,
    };
}

// ══════════════════════════════════════════════════════════════════════════════
//  Draw Calls
// ══════════════════════════════════════════════════════════════════════════════

/// Draw primitives from array data.
#[no_mangle]
pub extern "C" fn glDrawArrays(mode: GLenum, first: GLint, count: GLsizei) {
    draw::draw_arrays(ctx(), mode, first, count);
}

/// Draw indexed primitives.
#[no_mangle]
pub extern "C" fn glDrawElements(
    mode: GLenum, count: GLsizei, type_: GLenum, indices: *const GLvoid,
) {
    draw::draw_elements(ctx(), mode, count, type_, indices as usize);
}

// ══════════════════════════════════════════════════════════════════════════════
//  Framebuffer Objects
// ══════════════════════════════════════════════════════════════════════════════

/// Generate framebuffer names.
#[no_mangle]
pub extern "C" fn glGenFramebuffers(n: GLsizei, framebuffers: *mut GLuint) {
    if n <= 0 || framebuffers.is_null() { return; }
    // Phase 1: minimal FBO support — just return sequential IDs
    for i in 0..n as usize {
        unsafe { *framebuffers.add(i) = (i + 1) as u32; }
    }
}

/// Delete framebuffer objects.
#[no_mangle]
pub extern "C" fn glDeleteFramebuffers(_n: GLsizei, _framebuffers: *const GLuint) {
    // Phase 1: no-op
}

/// Bind a framebuffer.
#[no_mangle]
pub extern "C" fn glBindFramebuffer(_target: GLenum, framebuffer: GLuint) {
    ctx().bound_framebuffer = framebuffer;
}

/// Attach a texture to a framebuffer.
#[no_mangle]
pub extern "C" fn glFramebufferTexture2D(
    _target: GLenum, _attachment: GLenum,
    _textarget: GLenum, _texture: GLuint, _level: GLint,
) {
    // Phase 1: minimal — tracked but not rendered to
}

/// Check framebuffer completeness.
#[no_mangle]
pub extern "C" fn glCheckFramebufferStatus(_target: GLenum) -> GLenum {
    GL_FRAMEBUFFER_COMPLETE
}

/// Read pixels from the framebuffer.
#[no_mangle]
pub extern "C" fn glReadPixels(
    x: GLint, y: GLint, width: GLsizei, height: GLsizei,
    format: GLenum, _type: GLenum, pixels: *mut GLvoid,
) {
    if pixels.is_null() { return; }
    let c = ctx();
    let fb_w = c.default_fb.width as i32;
    let dst = pixels as *mut u8;

    let pixel_size: usize = match format {
        GL_RGBA => 4,
        GL_RGB => 3,
        _ => 4,
    };

    for row in 0..height {
        for col in 0..width {
            let sx = x + col;
            let sy = y + row;
            if sx < 0 || sy < 0 || sx >= fb_w || sy >= c.default_fb.height as i32 {
                continue;
            }
            let argb = c.default_fb.color[(sy as u32 * c.default_fb.width + sx as u32) as usize];
            let offset = ((row * width + col) as usize) * pixel_size;
            let r = ((argb >> 16) & 0xFF) as u8;
            let g = ((argb >> 8) & 0xFF) as u8;
            let b = (argb & 0xFF) as u8;
            let a = ((argb >> 24) & 0xFF) as u8;
            unsafe {
                *dst.add(offset) = r;
                *dst.add(offset + 1) = g;
                *dst.add(offset + 2) = b;
                if pixel_size == 4 {
                    *dst.add(offset + 3) = a;
                }
            }
        }
    }
}

/// Flush pending operations (no-op for SW rasterizer).
#[no_mangle]
pub extern "C" fn glFlush() {}

/// Finish all pending operations (no-op for SW rasterizer).
#[no_mangle]
pub extern "C" fn glFinish() {}

// ══════════════════════════════════════════════════════════════════════════════
//  Anti-Aliasing
// ══════════════════════════════════════════════════════════════════════════════

/// Enable or disable FXAA post-process (0 = off, non-zero = on).
#[no_mangle]
pub extern "C" fn gl_set_fxaa(enabled: u32) {
    ctx().fxaa_enabled = enabled != 0;
}

// ══════════════════════════════════════════════════════════════════════════════
//  Backend Selection
// ══════════════════════════════════════════════════════════════════════════════

/// Switch between hardware and software rasterizer at runtime.
/// 0 = software, non-zero = hardware (only if a .drv was loaded).
#[no_mangle]
pub extern "C" fn gl_set_hw_backend(enabled: u32) {
    let want_hw = enabled != 0;
    let has_hw = drv_loader::is_loaded();
    unsafe { USE_HW_BACKEND = want_hw && has_hw; }
}

/// Query whether the hardware backend is currently active.
/// Returns 1 if HW, 0 if SW.
#[no_mangle]
pub extern "C" fn gl_get_hw_backend() -> u32 {
    if unsafe { USE_HW_BACKEND } { 1 } else { 0 }
}

/// Query whether a hardware 3D driver is available (regardless of current mode).
/// Returns 1 if available, 0 if not.
#[no_mangle]
pub extern "C" fn gl_has_hw_backend() -> u32 {
    if drv_loader::is_loaded() { 1 } else { 0 }
}

// ══════════════════════════════════════════════════════════════════════════════
//  Math Functions (FPU/SSE accelerated, usable by any app)
// ══════════════════════════════════════════════════════════════════════════════

/// Sine via x87 `fsin` (IEEE 754 exact).
#[no_mangle]
pub extern "C" fn gl_math_sin(x: f32) -> f32 { rasterizer::math::sin(x) }

/// Cosine via x87 `fcos` (IEEE 754 exact).
#[no_mangle]
pub extern "C" fn gl_math_cos(x: f32) -> f32 { rasterizer::math::cos(x) }

/// Tangent via x87 `fptan`.
#[no_mangle]
pub extern "C" fn gl_math_tan(x: f32) -> f32 { rasterizer::math::tan(x) }

/// Square root via SSE2 `sqrtss` (IEEE 754 exact).
#[no_mangle]
pub extern "C" fn gl_math_sqrt(x: f32) -> f32 { rasterizer::math::sqrt(x) }

/// Absolute value via sign-bit clear.
#[no_mangle]
pub extern "C" fn gl_math_abs(x: f32) -> f32 { rasterizer::math::abs(x) }

/// Power function x^y via x87 FPU.
#[no_mangle]
pub extern "C" fn gl_math_pow(base: f32, exp: f32) -> f32 { rasterizer::math::pow(base, exp) }

/// Base-2 logarithm via x87 `fyl2x`.
#[no_mangle]
pub extern "C" fn gl_math_log2(x: f32) -> f32 { rasterizer::math::log2(x) }

/// Base-2 exponential via x87 `f2xm1` + `fscale`.
#[no_mangle]
pub extern "C" fn gl_math_exp2(x: f32) -> f32 { rasterizer::math::exp2(x) }

/// Floor via x87 rounding.
#[no_mangle]
pub extern "C" fn gl_math_floor(x: f32) -> f32 { rasterizer::math::floor(x) }

/// Ceiling via x87 rounding.
#[no_mangle]
pub extern "C" fn gl_math_ceil(x: f32) -> f32 { rasterizer::math::ceil(x) }

/// Clamp to [lo, hi].
#[no_mangle]
pub extern "C" fn gl_math_clamp(x: f32, lo: f32, hi: f32) -> f32 { rasterizer::math::clamp(x, lo, hi) }

/// Linear interpolation.
#[no_mangle]
pub extern "C" fn gl_math_lerp(a: f32, b: f32, t: f32) -> f32 { rasterizer::math::lerp(a, b, t) }

// ══════════════════════════════════════════════════════════════════════════════
//  Internal Helpers
// ══════════════════════════════════════════════════════════════════════════════

/// Set uniform float values.
fn set_uniform_floats(location: GLint, vals: &[f32]) {
    let c = ctx();
    let prog_id = c.current_program;
    if let Some(p) = c.shaders.get_program_mut(prog_id) {
        if let Some(u) = p.uniforms.iter_mut().find(|u| u.location == location) {
            for (i, &v) in vals.iter().enumerate() {
                if i < 16 { u.value[i] = v; }
            }
        }
    }
}

/// Convert a C string pointer to a &str.
unsafe fn cstr_to_str<'a>(ptr: *const u8) -> &'a str {
    let mut len = 0;
    while *ptr.add(len) != 0 { len += 1; }
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8(slice).unwrap_or("")
}

// ══════════════════════════════════════════════════════════════════════════════
//  Physics Engine
// ══════════════════════════════════════════════════════════════════════════════

static mut PHYSICS_WORLD: Option<physics::PhysicsWorld> = None;

fn phys() -> &'static mut physics::PhysicsWorld {
    unsafe {
        if PHYSICS_WORLD.is_none() {
            PHYSICS_WORLD = Some(physics::PhysicsWorld::new());
        }
        PHYSICS_WORLD.as_mut().unwrap()
    }
}

/// Create/reset the physics world. Returns 1 on success.
#[no_mangle]
pub extern "C" fn gl_physics_create_world() -> u32 {
    unsafe { PHYSICS_WORLD = Some(physics::PhysicsWorld::new()); }
    1
}

/// Set the gravity vector (default: 0, -9.81, 0).
#[no_mangle]
pub extern "C" fn gl_physics_set_gravity(gx: f32, gy: f32, gz: f32) {
    phys().gravity = physics::Vec3::new(gx, gy, gz);
}

/// Step the simulation by `dt` seconds.
#[no_mangle]
pub extern "C" fn gl_physics_step(dt: f32) {
    phys().step(dt);
}

/// Add a sphere body. Returns body ID.
#[no_mangle]
pub extern "C" fn gl_physics_add_sphere(mass: f32, radius: f32, x: f32, y: f32, z: f32) -> u32 {
    phys().add_sphere(mass, radius, x, y, z)
}

/// Add an infinite plane (static). Returns body ID.
#[no_mangle]
pub extern "C" fn gl_physics_add_plane(nx: f32, ny: f32, nz: f32, d: f32) -> u32 {
    phys().add_plane(nx, ny, nz, d)
}

/// Add an axis-aligned box body. Returns body ID.
#[no_mangle]
pub extern "C" fn gl_physics_add_box(mass: f32, hx: f32, hy: f32, hz: f32, x: f32, y: f32, z: f32) -> u32 {
    phys().add_box(mass, hx, hy, hz, x, y, z)
}

/// Set body velocity.
#[no_mangle]
pub extern "C" fn gl_physics_set_velocity(id: u32, vx: f32, vy: f32, vz: f32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.velocity = physics::Vec3::new(vx, vy, vz);
    }
}

/// Set body position.
#[no_mangle]
pub extern "C" fn gl_physics_set_position(id: u32, x: f32, y: f32, z: f32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.position = physics::Vec3::new(x, y, z);
    }
}

/// Get body position. Writes to out_x/y/z pointers.
#[no_mangle]
pub extern "C" fn gl_physics_get_position(id: u32, out_x: *mut f32, out_y: *mut f32, out_z: *mut f32) {
    if let Some(b) = phys().bodies.get(id as usize) {
        if !out_x.is_null() { unsafe { *out_x = b.position.x; } }
        if !out_y.is_null() { unsafe { *out_y = b.position.y; } }
        if !out_z.is_null() { unsafe { *out_z = b.position.z; } }
    }
}

/// Get body velocity. Writes to out pointers.
#[no_mangle]
pub extern "C" fn gl_physics_get_velocity(id: u32, out_x: *mut f32, out_y: *mut f32, out_z: *mut f32) {
    if let Some(b) = phys().bodies.get(id as usize) {
        if !out_x.is_null() { unsafe { *out_x = b.velocity.x; } }
        if !out_y.is_null() { unsafe { *out_y = b.velocity.y; } }
        if !out_z.is_null() { unsafe { *out_z = b.velocity.z; } }
    }
}

/// Set coefficient of restitution (bounciness, 0.0..1.0).
#[no_mangle]
pub extern "C" fn gl_physics_set_restitution(id: u32, e: f32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.restitution = e;
    }
}

/// Set body mass (0 = static/immovable).
#[no_mangle]
pub extern "C" fn gl_physics_set_mass(id: u32, mass: f32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.mass = mass;
        b.inv_mass = if mass <= 0.0 { 0.0 } else { 1.0 / mass };
        b.use_gravity = mass > 0.0;
    }
}

/// Apply a force to a body (accumulated until next step).
#[no_mangle]
pub extern "C" fn gl_physics_apply_force(id: u32, fx: f32, fy: f32, fz: f32) {
    phys().apply_force(id, fx, fy, fz);
}

/// Apply an impulse (instant velocity change).
#[no_mangle]
pub extern "C" fn gl_physics_apply_impulse(id: u32, ix: f32, iy: f32, iz: f32) {
    phys().apply_impulse(id, ix, iy, iz);
}

/// Set angular velocity around Y axis (backward compat).
#[no_mangle]
pub extern "C" fn gl_physics_set_angular_vel_y(id: u32, omega: f32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.angular_vel.y = omega;
    }
}

/// Get current Y rotation angle (extracted from quaternion).
#[no_mangle]
pub extern "C" fn gl_physics_get_rotation_y(id: u32) -> f32 {
    phys().bodies.get(id as usize).map_or(0.0, |b| b.orientation.rotation_y())
}

/// Set full 3D angular velocity (rad/s).
#[no_mangle]
pub extern "C" fn gl_physics_set_angular_velocity(id: u32, wx: f32, wy: f32, wz: f32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.angular_vel = physics::Vec3::new(wx, wy, wz);
    }
}

/// Get orientation quaternion. Writes (w, x, y, z) to output pointers.
#[no_mangle]
pub extern "C" fn gl_physics_get_orientation(id: u32, out_w: *mut f32, out_x: *mut f32, out_y: *mut f32, out_z: *mut f32) {
    if let Some(b) = phys().bodies.get(id as usize) {
        if !out_w.is_null() { unsafe { *out_w = b.orientation.w; } }
        if !out_x.is_null() { unsafe { *out_x = b.orientation.x; } }
        if !out_y.is_null() { unsafe { *out_y = b.orientation.y; } }
        if !out_z.is_null() { unsafe { *out_z = b.orientation.z; } }
    }
}

/// Set angular damping factor (0.0 = no damping, ~2.0 = moderate, ~5.0 = heavy).
#[no_mangle]
pub extern "C" fn gl_physics_set_angular_damping(id: u32, damping: f32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.angular_damping = damping;
    }
}

/// Set linear damping factor (air resistance).
#[no_mangle]
pub extern "C" fn gl_physics_set_linear_damping(id: u32, damping: f32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.linear_damping = damping;
    }
}

/// Set whether body is affected by gravity.
#[no_mangle]
pub extern "C" fn gl_physics_set_use_gravity(id: u32, use_grav: u32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.use_gravity = use_grav != 0;
    }
}

/// Set body active/inactive.
#[no_mangle]
pub extern "C" fn gl_physics_set_active(id: u32, active: u32) {
    if let Some(b) = phys().bodies.get_mut(id as usize) {
        b.active = active != 0;
    }
}

/// Update the distance parameter of a plane collider.
#[no_mangle]
pub extern "C" fn gl_physics_set_plane_d(id: u32, d: f32) {
    phys().set_plane_d(id, d);
}

/// Get number of bodies in the world.
#[no_mangle]
pub extern "C" fn gl_physics_body_count() -> u32 {
    phys().bodies.len() as u32
}
