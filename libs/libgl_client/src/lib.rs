//! libgl_client — Safe Rust wrapper for the libgl shared library.
//!
//! Loads `libgl.so` via `dl_open`/`dl_sym` and provides ergonomic Rust functions
//! for OpenGL ES 2.0 operations.
//!
//! # Usage
//! ```rust
//! libgl_client::init();
//! libgl_client::gl_init(800, 600);
//! libgl_client::clear_color(0.1, 0.1, 0.2, 1.0);
//! libgl_client::clear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
//! ```

#![no_std]
#![allow(unused, dead_code, static_mut_refs)]

extern crate alloc;

use dynlink::{dl_open, dl_sym, DlHandle};

// ── GL Type re-exports ──────────────────────────────────────────────────────

pub type GLenum = u32;
pub type GLboolean = u8;
pub type GLbitfield = u32;
pub type GLint = i32;
pub type GLuint = u32;
pub type GLsizei = i32;
pub type GLfloat = f32;
pub type GLchar = u8;
pub type GLsizeiptr = isize;
pub type GLintptr = isize;

// ── GL Constants ────────────────────────────────────────────────────────────

pub const GL_NO_ERROR: GLenum = 0;
pub const GL_FALSE: GLboolean = 0;
pub const GL_TRUE: GLboolean = 1;
pub const GL_DEPTH_TEST: GLenum = 0x0B71;
pub const GL_BLEND: GLenum = 0x0BE2;
pub const GL_CULL_FACE: GLenum = 0x0B44;
pub const GL_COLOR_BUFFER_BIT: GLbitfield = 0x00004000;
pub const GL_DEPTH_BUFFER_BIT: GLbitfield = 0x00000100;
pub const GL_TRIANGLES: GLenum = 0x0004;
pub const GL_TRIANGLE_STRIP: GLenum = 0x0005;
pub const GL_TRIANGLE_FAN: GLenum = 0x0006;
pub const GL_ARRAY_BUFFER: GLenum = 0x8892;
pub const GL_ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
pub const GL_STATIC_DRAW: GLenum = 0x88E4;
pub const GL_FLOAT: GLenum = 0x1406;
pub const GL_UNSIGNED_SHORT: GLenum = 0x1403;
pub const GL_UNSIGNED_INT: GLenum = 0x1405;
pub const GL_UNSIGNED_BYTE: GLenum = 0x1401;
pub const GL_TEXTURE_2D: GLenum = 0x0DE1;
pub const GL_TEXTURE0: GLenum = 0x84C0;
pub const GL_RGBA: GLenum = 0x1908;
pub const GL_RGB: GLenum = 0x1907;
pub const GL_NEAREST: GLenum = 0x2600;
pub const GL_LINEAR: GLenum = 0x2601;
pub const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub const GL_VERTEX_SHADER: GLenum = 0x8B31;
pub const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
pub const GL_COMPILE_STATUS: GLenum = 0x8B81;
pub const GL_LINK_STATUS: GLenum = 0x8B82;
pub const GL_SRC_ALPHA: GLenum = 0x0302;
pub const GL_ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;
pub const GL_LESS: GLenum = 0x0201;
pub const GL_LEQUAL: GLenum = 0x0203;
pub const GL_BACK: GLenum = 0x0405;
pub const GL_CCW: GLenum = 0x0901;
pub const GL_CW: GLenum = 0x0900;
pub const GL_FRAMEBUFFER: GLenum = 0x8D40;
pub const GL_FRAMEBUFFER_COMPLETE: GLenum = 0x8CD5;
pub const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
pub const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
pub const GL_REPEAT: GLenum = 0x2901;
pub const GL_CLAMP_TO_EDGE: GLenum = 0x812F;
pub const GL_SCISSOR_TEST: GLenum = 0x0C11;
pub const GL_VENDOR: GLenum = 0x1F00;
pub const GL_RENDERER: GLenum = 0x1F01;
pub const GL_VERSION: GLenum = 0x1F02;

// ── Function pointer cache ──────────────────────────────────────────────────

struct LibGl {
    _handle: DlHandle,
    // anyOS extensions
    init: extern "C" fn(u32, u32),
    swap_buffers: extern "C" fn() -> *const u32,
    get_backbuffer: extern "C" fn() -> *const u32,
    // State
    get_error: extern "C" fn() -> GLenum,
    get_string: extern "C" fn(GLenum) -> *const u8,
    enable: extern "C" fn(GLenum),
    disable: extern "C" fn(GLenum),
    blend_func: extern "C" fn(GLenum, GLenum),
    blend_func_separate: extern "C" fn(GLenum, GLenum, GLenum, GLenum),
    depth_func: extern "C" fn(GLenum),
    depth_mask: extern "C" fn(GLboolean),
    cull_face: extern "C" fn(GLenum),
    front_face: extern "C" fn(GLenum),
    viewport: extern "C" fn(GLint, GLint, GLsizei, GLsizei),
    clear_color: extern "C" fn(GLfloat, GLfloat, GLfloat, GLfloat),
    clear: extern "C" fn(GLbitfield),
    scissor: extern "C" fn(GLint, GLint, GLsizei, GLsizei),
    line_width: extern "C" fn(GLfloat),
    pixel_storei: extern "C" fn(GLenum, GLint),
    color_mask: extern "C" fn(GLboolean, GLboolean, GLboolean, GLboolean),
    // Buffers
    gen_buffers: extern "C" fn(GLsizei, *mut GLuint),
    delete_buffers: extern "C" fn(GLsizei, *const GLuint),
    bind_buffer: extern "C" fn(GLenum, GLuint),
    buffer_data: extern "C" fn(GLenum, GLsizeiptr, *const u8, GLenum),
    buffer_sub_data: extern "C" fn(GLenum, GLintptr, GLsizeiptr, *const u8),
    // Textures
    gen_textures: extern "C" fn(GLsizei, *mut GLuint),
    delete_textures: extern "C" fn(GLsizei, *const GLuint),
    bind_texture: extern "C" fn(GLenum, GLuint),
    tex_image_2d: extern "C" fn(GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, *const u8),
    tex_sub_image_2d: extern "C" fn(GLenum, GLint, GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *const u8),
    tex_parameteri: extern "C" fn(GLenum, GLenum, GLint),
    active_texture: extern "C" fn(GLenum),
    generate_mipmap: extern "C" fn(GLenum),
    // Shaders
    create_shader: extern "C" fn(GLenum) -> GLuint,
    delete_shader: extern "C" fn(GLuint),
    shader_source: extern "C" fn(GLuint, GLsizei, *const *const GLchar, *const GLint),
    compile_shader: extern "C" fn(GLuint),
    get_shaderiv: extern "C" fn(GLuint, GLenum, *mut GLint),
    get_shader_info_log: extern "C" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar),
    // Programs
    create_program: extern "C" fn() -> GLuint,
    delete_program: extern "C" fn(GLuint),
    attach_shader: extern "C" fn(GLuint, GLuint),
    link_program: extern "C" fn(GLuint),
    use_program: extern "C" fn(GLuint),
    get_programiv: extern "C" fn(GLuint, GLenum, *mut GLint),
    get_program_info_log: extern "C" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar),
    // Uniforms & Attributes
    get_uniform_location: extern "C" fn(GLuint, *const GLchar) -> GLint,
    get_attrib_location: extern "C" fn(GLuint, *const GLchar) -> GLint,
    bind_attrib_location: extern "C" fn(GLuint, GLuint, *const GLchar),
    uniform1i: extern "C" fn(GLint, GLint),
    uniform1f: extern "C" fn(GLint, GLfloat),
    uniform2f: extern "C" fn(GLint, GLfloat, GLfloat),
    uniform3f: extern "C" fn(GLint, GLfloat, GLfloat, GLfloat),
    uniform4f: extern "C" fn(GLint, GLfloat, GLfloat, GLfloat, GLfloat),
    uniform_matrix4fv: extern "C" fn(GLint, GLsizei, GLboolean, *const GLfloat),
    enable_vertex_attrib_array: extern "C" fn(GLuint),
    disable_vertex_attrib_array: extern "C" fn(GLuint),
    vertex_attrib_pointer: extern "C" fn(GLuint, GLint, GLenum, GLboolean, GLsizei, *const u8),
    // Draw
    draw_arrays: extern "C" fn(GLenum, GLint, GLsizei),
    draw_elements: extern "C" fn(GLenum, GLsizei, GLenum, *const u8),
    // Framebuffer
    gen_framebuffers: extern "C" fn(GLsizei, *mut GLuint),
    delete_framebuffers: extern "C" fn(GLsizei, *const GLuint),
    bind_framebuffer: extern "C" fn(GLenum, GLuint),
    framebuffer_texture_2d: extern "C" fn(GLenum, GLenum, GLenum, GLuint, GLint),
    check_framebuffer_status: extern "C" fn(GLenum) -> GLenum,
    read_pixels: extern "C" fn(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *mut u8),
    flush: extern "C" fn(),
    finish: extern "C" fn(),
    // Anti-Aliasing
    set_fxaa: extern "C" fn(u32),
    // Backend selection
    set_hw_backend: extern "C" fn(u32),
    get_hw_backend: extern "C" fn() -> u32,
    has_hw_backend: extern "C" fn() -> u32,
    // Math
    math_sin: extern "C" fn(f32) -> f32,
    math_cos: extern "C" fn(f32) -> f32,
    math_tan: extern "C" fn(f32) -> f32,
    math_sqrt: extern "C" fn(f32) -> f32,
    math_abs: extern "C" fn(f32) -> f32,
    math_pow: extern "C" fn(f32, f32) -> f32,
    math_log2: extern "C" fn(f32) -> f32,
    math_exp2: extern "C" fn(f32) -> f32,
    math_floor: extern "C" fn(f32) -> f32,
    math_ceil: extern "C" fn(f32) -> f32,
    math_clamp: extern "C" fn(f32, f32, f32) -> f32,
    math_lerp: extern "C" fn(f32, f32, f32) -> f32,
}

static mut LIB: Option<LibGl> = None;

fn lib() -> &'static LibGl {
    unsafe { LIB.as_ref().expect("libgl not loaded — call init() first") }
}

/// Resolve a function pointer from the loaded library.
unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(handle, name)
        .unwrap_or_else(|| panic!("libgl: symbol not found: {}", name));
    unsafe { core::mem::transmute_copy::<*const (), T>(&ptr) }
}

// ── Initialization ──────────────────────────────────────────────────────────

/// Load libgl.so and cache all function pointers. Returns true on success.
pub fn init() -> bool {
    let handle = match dl_open("/Libraries/libgl.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let lib = LibGl {
            init: resolve(&handle, "gl_init"),
            swap_buffers: resolve(&handle, "gl_swap_buffers"),
            get_backbuffer: resolve(&handle, "gl_get_backbuffer"),
            get_error: resolve(&handle, "glGetError"),
            get_string: resolve(&handle, "glGetString"),
            enable: resolve(&handle, "glEnable"),
            disable: resolve(&handle, "glDisable"),
            blend_func: resolve(&handle, "glBlendFunc"),
            blend_func_separate: resolve(&handle, "glBlendFuncSeparate"),
            depth_func: resolve(&handle, "glDepthFunc"),
            depth_mask: resolve(&handle, "glDepthMask"),
            cull_face: resolve(&handle, "glCullFace"),
            front_face: resolve(&handle, "glFrontFace"),
            viewport: resolve(&handle, "glViewport"),
            clear_color: resolve(&handle, "glClearColor"),
            clear: resolve(&handle, "glClear"),
            scissor: resolve(&handle, "glScissor"),
            line_width: resolve(&handle, "glLineWidth"),
            pixel_storei: resolve(&handle, "glPixelStorei"),
            color_mask: resolve(&handle, "glColorMask"),
            gen_buffers: resolve(&handle, "glGenBuffers"),
            delete_buffers: resolve(&handle, "glDeleteBuffers"),
            bind_buffer: resolve(&handle, "glBindBuffer"),
            buffer_data: resolve(&handle, "glBufferData"),
            buffer_sub_data: resolve(&handle, "glBufferSubData"),
            gen_textures: resolve(&handle, "glGenTextures"),
            delete_textures: resolve(&handle, "glDeleteTextures"),
            bind_texture: resolve(&handle, "glBindTexture"),
            tex_image_2d: resolve(&handle, "glTexImage2D"),
            tex_sub_image_2d: resolve(&handle, "glTexSubImage2D"),
            tex_parameteri: resolve(&handle, "glTexParameteri"),
            active_texture: resolve(&handle, "glActiveTexture"),
            generate_mipmap: resolve(&handle, "glGenerateMipmap"),
            create_shader: resolve(&handle, "glCreateShader"),
            delete_shader: resolve(&handle, "glDeleteShader"),
            shader_source: resolve(&handle, "glShaderSource"),
            compile_shader: resolve(&handle, "glCompileShader"),
            get_shaderiv: resolve(&handle, "glGetShaderiv"),
            get_shader_info_log: resolve(&handle, "glGetShaderInfoLog"),
            create_program: resolve(&handle, "glCreateProgram"),
            delete_program: resolve(&handle, "glDeleteProgram"),
            attach_shader: resolve(&handle, "glAttachShader"),
            link_program: resolve(&handle, "glLinkProgram"),
            use_program: resolve(&handle, "glUseProgram"),
            get_programiv: resolve(&handle, "glGetProgramiv"),
            get_program_info_log: resolve(&handle, "glGetProgramInfoLog"),
            get_uniform_location: resolve(&handle, "glGetUniformLocation"),
            get_attrib_location: resolve(&handle, "glGetAttribLocation"),
            bind_attrib_location: resolve(&handle, "glBindAttribLocation"),
            uniform1i: resolve(&handle, "glUniform1i"),
            uniform1f: resolve(&handle, "glUniform1f"),
            uniform2f: resolve(&handle, "glUniform2f"),
            uniform3f: resolve(&handle, "glUniform3f"),
            uniform4f: resolve(&handle, "glUniform4f"),
            uniform_matrix4fv: resolve(&handle, "glUniformMatrix4fv"),
            enable_vertex_attrib_array: resolve(&handle, "glEnableVertexAttribArray"),
            disable_vertex_attrib_array: resolve(&handle, "glDisableVertexAttribArray"),
            vertex_attrib_pointer: resolve(&handle, "glVertexAttribPointer"),
            draw_arrays: resolve(&handle, "glDrawArrays"),
            draw_elements: resolve(&handle, "glDrawElements"),
            gen_framebuffers: resolve(&handle, "glGenFramebuffers"),
            delete_framebuffers: resolve(&handle, "glDeleteFramebuffers"),
            bind_framebuffer: resolve(&handle, "glBindFramebuffer"),
            framebuffer_texture_2d: resolve(&handle, "glFramebufferTexture2D"),
            check_framebuffer_status: resolve(&handle, "glCheckFramebufferStatus"),
            read_pixels: resolve(&handle, "glReadPixels"),
            flush: resolve(&handle, "glFlush"),
            finish: resolve(&handle, "glFinish"),
            set_fxaa: resolve(&handle, "gl_set_fxaa"),
            set_hw_backend: resolve(&handle, "gl_set_hw_backend"),
            get_hw_backend: resolve(&handle, "gl_get_hw_backend"),
            has_hw_backend: resolve(&handle, "gl_has_hw_backend"),
            math_sin: resolve(&handle, "gl_math_sin"),
            math_cos: resolve(&handle, "gl_math_cos"),
            math_tan: resolve(&handle, "gl_math_tan"),
            math_sqrt: resolve(&handle, "gl_math_sqrt"),
            math_abs: resolve(&handle, "gl_math_abs"),
            math_pow: resolve(&handle, "gl_math_pow"),
            math_log2: resolve(&handle, "gl_math_log2"),
            math_exp2: resolve(&handle, "gl_math_exp2"),
            math_floor: resolve(&handle, "gl_math_floor"),
            math_ceil: resolve(&handle, "gl_math_ceil"),
            math_clamp: resolve(&handle, "gl_math_clamp"),
            math_lerp: resolve(&handle, "gl_math_lerp"),
            _handle: handle,
        };
        LIB = Some(lib);
    }
    true
}

// ══════════════════════════════════════════════════════════════════════════════
//  Public API — thin wrappers around function pointers
// ══════════════════════════════════════════════════════════════════════════════

/// Initialize the GL context with framebuffer dimensions.
pub fn gl_init(width: u32, height: u32) { (lib().init)(width, height); }

/// Swap buffers. Returns pointer to ARGB color data.
pub fn swap_buffers() -> *const u32 { (lib().swap_buffers)() }

/// Get a pointer to the backbuffer.
pub fn get_backbuffer() -> *const u32 { (lib().get_backbuffer)() }

/// Get the current error.
pub fn get_error() -> GLenum { (lib().get_error)() }

/// Enable a capability.
pub fn enable(cap: GLenum) { (lib().enable)(cap); }

/// Disable a capability.
pub fn disable(cap: GLenum) { (lib().disable)(cap); }

/// Set the blend function.
pub fn blend_func(sfactor: GLenum, dfactor: GLenum) { (lib().blend_func)(sfactor, dfactor); }

/// Set the depth function.
pub fn depth_func(func: GLenum) { (lib().depth_func)(func); }

/// Enable/disable depth writes.
pub fn depth_mask(flag: bool) { (lib().depth_mask)(if flag { 1 } else { 0 }); }

/// Set face culling mode.
pub fn cull_face(mode: GLenum) { (lib().cull_face)(mode); }

/// Set front face winding.
pub fn front_face(mode: GLenum) { (lib().front_face)(mode); }

/// Set the viewport.
pub fn viewport(x: i32, y: i32, w: i32, h: i32) { (lib().viewport)(x, y, w, h); }

/// Set the clear color.
pub fn clear_color(r: f32, g: f32, b: f32, a: f32) { (lib().clear_color)(r, g, b, a); }

/// Clear buffers.
pub fn clear(mask: GLbitfield) { (lib().clear)(mask); }

/// Generate buffer objects.
pub fn gen_buffers(n: i32, ids: &mut [u32]) { (lib().gen_buffers)(n, ids.as_mut_ptr()); }

/// Delete buffer objects.
pub fn delete_buffers(ids: &[u32]) { (lib().delete_buffers)(ids.len() as i32, ids.as_ptr()); }

/// Bind a buffer.
pub fn bind_buffer(target: GLenum, buffer: u32) { (lib().bind_buffer)(target, buffer); }

/// Upload buffer data.
pub fn buffer_data(target: GLenum, data: &[u8], usage: GLenum) {
    (lib().buffer_data)(target, data.len() as isize, data.as_ptr(), usage);
}

/// Upload typed buffer data.
pub fn buffer_data_f32(target: GLenum, data: &[f32], usage: GLenum) {
    let bytes = unsafe {
        core::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4)
    };
    (lib().buffer_data)(target, bytes.len() as isize, bytes.as_ptr(), usage);
}

/// Upload u16 index data.
pub fn buffer_data_u16(target: GLenum, data: &[u16], usage: GLenum) {
    let bytes = unsafe {
        core::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2)
    };
    (lib().buffer_data)(target, bytes.len() as isize, bytes.as_ptr(), usage);
}

/// Generate textures.
pub fn gen_textures(n: i32, ids: &mut [u32]) { (lib().gen_textures)(n, ids.as_mut_ptr()); }

/// Bind a texture.
pub fn bind_texture(target: GLenum, texture: u32) { (lib().bind_texture)(target, texture); }

/// Set texture parameter.
pub fn tex_parameteri(target: GLenum, pname: GLenum, param: i32) {
    (lib().tex_parameteri)(target, pname, param);
}

/// Upload texture image data.
pub fn tex_image_2d(target: GLenum, level: i32, internal_format: i32,
                    width: i32, height: i32, border: i32,
                    format: GLenum, type_: GLenum, data: &[u8]) {
    (lib().tex_image_2d)(target, level, internal_format, width, height, border,
                         format, type_, data.as_ptr());
}

/// Set active texture unit.
pub fn active_texture(texture: GLenum) { (lib().active_texture)(texture); }

/// Create a shader.
pub fn create_shader(shader_type: GLenum) -> u32 { (lib().create_shader)(shader_type) }

/// Delete a shader.
pub fn delete_shader(shader: u32) { (lib().delete_shader)(shader); }

/// Set shader source from a string.
pub fn shader_source(shader: u32, source: &str) {
    let ptr = source.as_ptr();
    let len = source.len() as GLint;
    (lib().shader_source)(shader, 1, &ptr, &len);
}

/// Compile a shader.
pub fn compile_shader(shader: u32) { (lib().compile_shader)(shader); }

/// Get shader compile status.
pub fn get_shader_compile_status(shader: u32) -> bool {
    let mut status: GLint = 0;
    (lib().get_shaderiv)(shader, GL_COMPILE_STATUS, &mut status);
    status != 0
}

/// Get shader info log.
pub fn get_shader_info_log(shader: u32) -> alloc::string::String {
    let mut len: GLsizei = 0;
    let mut buf = [0u8; 512];
    (lib().get_shader_info_log)(shader, 512, &mut len, buf.as_mut_ptr());
    let s = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
    alloc::string::String::from(s)
}

/// Create a program.
pub fn create_program() -> u32 { (lib().create_program)() }

/// Delete a program.
pub fn delete_program(program: u32) { (lib().delete_program)(program); }

/// Attach a shader to a program.
pub fn attach_shader(program: u32, shader: u32) { (lib().attach_shader)(program, shader); }

/// Link a program.
pub fn link_program(program: u32) { (lib().link_program)(program); }

/// Use a program.
pub fn use_program(program: u32) { (lib().use_program)(program); }

/// Get program link status.
pub fn get_program_link_status(program: u32) -> bool {
    let mut status: GLint = 0;
    (lib().get_programiv)(program, GL_LINK_STATUS, &mut status);
    status != 0
}

/// Get uniform location.
pub fn get_uniform_location(program: u32, name: &str) -> i32 {
    let mut buf = [0u8; 64];
    let len = name.len().min(63);
    buf[..len].copy_from_slice(&name.as_bytes()[..len]);
    buf[len] = 0;
    (lib().get_uniform_location)(program, buf.as_ptr())
}

/// Get attribute location.
pub fn get_attrib_location(program: u32, name: &str) -> i32 {
    let mut buf = [0u8; 64];
    let len = name.len().min(63);
    buf[..len].copy_from_slice(&name.as_bytes()[..len]);
    buf[len] = 0;
    (lib().get_attrib_location)(program, buf.as_ptr())
}

/// Set 1-int uniform.
pub fn uniform1i(location: i32, v: i32) { (lib().uniform1i)(location, v); }

/// Set 1-float uniform.
pub fn uniform1f(location: i32, v: f32) { (lib().uniform1f)(location, v); }

/// Set 3-float uniform.
pub fn uniform3f(location: i32, x: f32, y: f32, z: f32) { (lib().uniform3f)(location, x, y, z); }

/// Set 4-float uniform.
pub fn uniform4f(location: i32, x: f32, y: f32, z: f32, w: f32) { (lib().uniform4f)(location, x, y, z, w); }

/// Set 4x4 matrix uniform.
pub fn uniform_matrix4fv(location: i32, transpose: bool, value: &[f32; 16]) {
    (lib().uniform_matrix4fv)(location, 1, if transpose { 1 } else { 0 }, value.as_ptr());
}

/// Enable a vertex attribute array.
pub fn enable_vertex_attrib_array(index: u32) { (lib().enable_vertex_attrib_array)(index); }

/// Disable a vertex attribute array.
pub fn disable_vertex_attrib_array(index: u32) { (lib().disable_vertex_attrib_array)(index); }

/// Set vertex attribute pointer.
pub fn vertex_attrib_pointer(
    index: u32, size: i32, type_: GLenum, normalized: bool, stride: i32, offset: usize,
) {
    (lib().vertex_attrib_pointer)(
        index, size, type_,
        if normalized { 1 } else { 0 },
        stride, offset as *const u8,
    );
}

/// Draw arrays.
pub fn draw_arrays(mode: GLenum, first: i32, count: i32) { (lib().draw_arrays)(mode, first, count); }

/// Draw elements.
pub fn draw_elements(mode: GLenum, count: i32, type_: GLenum, offset: usize) {
    (lib().draw_elements)(mode, count, type_, offset as *const u8);
}

/// Flush.
pub fn flush() { (lib().flush)(); }

/// Finish.
pub fn finish() { (lib().finish)(); }

// ══════════════════════════════════════════════════════════════════════════════
//  Anti-Aliasing
// ══════════════════════════════════════════════════════════════════════════════

/// Enable or disable FXAA post-process anti-aliasing.
pub fn set_fxaa(enabled: bool) { (lib().set_fxaa)(if enabled { 1 } else { 0 }); }

// ══════════════════════════════════════════════════════════════════════════════
//  Backend Selection
// ══════════════════════════════════════════════════════════════════════════════

/// Switch between hardware (SVGA3D) and software rasterizer.
pub fn set_hw_backend(enabled: bool) { (lib().set_hw_backend)(if enabled { 1 } else { 0 }); }

/// Query whether the hardware backend is currently active.
pub fn get_hw_backend() -> bool { (lib().get_hw_backend)() != 0 }

/// Query whether SVGA3D hardware is available (even if not currently in use).
pub fn has_hw_backend() -> bool { (lib().has_hw_backend)() != 0 }

// ══════════════════════════════════════════════════════════════════════════════
//  Math Functions (FPU/SSE accelerated via libgl)
// ══════════════════════════════════════════════════════════════════════════════

/// Pi constant.
pub const PI: f32 = 3.14159265;

/// Sine via x87 `fsin` (IEEE 754 exact).
pub fn sin(x: f32) -> f32 { (lib().math_sin)(x) }

/// Cosine via x87 `fcos` (IEEE 754 exact).
pub fn cos(x: f32) -> f32 { (lib().math_cos)(x) }

/// Tangent via x87 `fptan`.
pub fn tan(x: f32) -> f32 { (lib().math_tan)(x) }

/// Square root via SSE2 `sqrtss` (IEEE 754 exact).
pub fn sqrt(x: f32) -> f32 { (lib().math_sqrt)(x) }

/// Absolute value.
pub fn abs(x: f32) -> f32 { (lib().math_abs)(x) }

/// Power function x^y via x87 FPU.
pub fn pow(base: f32, exp: f32) -> f32 { (lib().math_pow)(base, exp) }

/// Base-2 logarithm via x87 `fyl2x`.
pub fn log2(x: f32) -> f32 { (lib().math_log2)(x) }

/// Base-2 exponential via x87 `f2xm1` + `fscale`.
pub fn exp2(x: f32) -> f32 { (lib().math_exp2)(x) }

/// Floor via x87 rounding.
pub fn floor(x: f32) -> f32 { (lib().math_floor)(x) }

/// Ceiling via x87 rounding.
pub fn ceil(x: f32) -> f32 { (lib().math_ceil)(x) }

/// Clamp to [lo, hi].
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 { (lib().math_clamp)(x, lo, hi) }

/// Linear interpolation.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 { (lib().math_lerp)(a, b, t) }
