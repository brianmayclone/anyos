//! Shared helpers for 3D (libgl) benchmarks.
//!
//! Provides idempotent GL initialization, shader compilation utilities,
//! matrix math (4×4 column-major), and framebuffer-to-canvas copy.

use alloc::vec::Vec;
use libanyui_client as anyui;
use libgl_client as gl;

// ════════════════════════════════════════════════════════════════════════
//  GL Context Management
// ════════════════════════════════════════════════════════════════════════

static mut GL_LOADED: bool = false;
static mut GL_FB_W: u32 = 0;
static mut GL_FB_H: u32 = 0;

/// Loads libgl.so and initialises the framebuffer (idempotent).
///
/// Returns `true` when the GL context is ready.
pub fn ensure_gl_init(w: u32, h: u32) -> bool {
    unsafe {
        if !GL_LOADED {
            if !gl::init() {
                return false;
            }
            GL_LOADED = true;
        }
        if GL_FB_W != w || GL_FB_H != h {
            gl::gl_init(w, h);
            GL_FB_W = w;
            GL_FB_H = h;
        }
    }
    // Sane defaults for 3D rendering
    gl::viewport(0, 0, w as i32, h as i32);
    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::enable(gl::GL_CULL_FACE);
    gl::cull_face(gl::GL_BACK);
    gl::set_fxaa(false);
    true
}

/// Copies the current GL framebuffer to an anyui Canvas for preview.
pub fn copy_gl_to_canvas(canvas: &anyui::Canvas, w: u32, h: u32) {
    let fb_ptr = gl::swap_buffers();
    if !fb_ptr.is_null() {
        let pixels = unsafe { core::slice::from_raw_parts(fb_ptr, (w * h) as usize) };
        canvas.copy_pixels_from(pixels);
    }
}

// ════════════════════════════════════════════════════════════════════════
//  Shader Helpers
// ════════════════════════════════════════════════════════════════════════

/// Compiles a vertex + fragment shader pair and links them into a program.
///
/// Returns `Some((program, vs, fs))` on success, `None` on failure.
pub fn compile_program(vs_src: &str, fs_src: &str) -> Option<(u32, u32, u32)> {
    let vs = gl::create_shader(gl::GL_VERTEX_SHADER);
    gl::shader_source(vs, vs_src);
    gl::compile_shader(vs);
    if !gl::get_shader_compile_status(vs) {
        gl::delete_shader(vs);
        return None;
    }

    let fs = gl::create_shader(gl::GL_FRAGMENT_SHADER);
    gl::shader_source(fs, fs_src);
    gl::compile_shader(fs);
    if !gl::get_shader_compile_status(fs) {
        gl::delete_shader(vs);
        gl::delete_shader(fs);
        return None;
    }

    let program = gl::create_program();
    gl::attach_shader(program, vs);
    gl::attach_shader(program, fs);
    gl::link_program(program);
    if !gl::get_program_link_status(program) {
        gl::delete_shader(vs);
        gl::delete_shader(fs);
        gl::delete_program(program);
        return None;
    }
    Some((program, vs, fs))
}

/// Deletes a shader program and its attached shaders.
pub fn cleanup_program(program: u32, vs: u32, fs: u32) {
    gl::delete_program(program);
    gl::delete_shader(vs);
    gl::delete_shader(fs);
}

// ════════════════════════════════════════════════════════════════════════
//  4×4 Matrix Math (column-major, f32)
// ════════════════════════════════════════════════════════════════════════

/// 4×4 identity matrix.
pub fn mat4_identity() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Multiplies two 4×4 column-major matrices: result = a × b.
pub fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0f32;
            for k in 0..4 {
                sum += a[k * 4 + row] * b[col * 4 + k];
            }
            r[col * 4 + row] = sum;
        }
    }
    r
}

/// Perspective projection matrix.
pub fn mat4_perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let f = 1.0 / gl::tan(fov_y * 0.5);
    let nf = 1.0 / (near - far);
    let mut m = [0.0f32; 16];
    m[0]  = f / aspect;
    m[5]  = f;
    m[10] = (far + near) * nf;
    m[11] = -1.0;
    m[14] = 2.0 * far * near * nf;
    m
}

/// Translation matrix.
pub fn mat4_translate(x: f32, y: f32, z: f32) -> [f32; 16] {
    let mut m = mat4_identity();
    m[12] = x;
    m[13] = y;
    m[14] = z;
    m
}

/// Rotation around the Y axis.
pub fn mat4_rotate_y(angle: f32) -> [f32; 16] {
    let c = gl::cos(angle);
    let s = gl::sin(angle);
    let mut m = mat4_identity();
    m[0]  =  c;
    m[2]  =  s;
    m[8]  = -s;
    m[10] =  c;
    m
}

/// Rotation around the X axis.
pub fn mat4_rotate_x(angle: f32) -> [f32; 16] {
    let c = gl::cos(angle);
    let s = gl::sin(angle);
    let mut m = mat4_identity();
    m[5]  =  c;
    m[6]  =  s;
    m[9]  = -s;
    m[10] =  c;
    m
}

/// Uniform scale matrix.
pub fn mat4_scale(sx: f32, sy: f32, sz: f32) -> [f32; 16] {
    let mut m = [0.0f32; 16];
    m[0]  = sx;
    m[5]  = sy;
    m[10] = sz;
    m[15] = 1.0;
    m
}

// ════════════════════════════════════════════════════════════════════════
//  Geometry Generators
// ════════════════════════════════════════════════════════════════════════

/// Generates a UV sphere with interleaved vertex data (pos3 + normal3 + uv2).
///
/// Returns `(vertices, indices)`.
pub fn generate_sphere(rings: u32, sectors: u32) -> (Vec<f32>, Vec<u16>) {
    let mut verts = Vec::new();
    let mut indices = Vec::new();
    let pi = gl::PI;

    for r in 0..=rings {
        let theta = pi * r as f32 / rings as f32;
        let sin_t = gl::sin(theta);
        let cos_t = gl::cos(theta);
        for s in 0..=sectors {
            let phi = 2.0 * pi * s as f32 / sectors as f32;
            let x = sin_t * gl::cos(phi);
            let y = cos_t;
            let z = sin_t * gl::sin(phi);
            // position
            verts.push(x);
            verts.push(y);
            verts.push(z);
            // normal (unit sphere → normal == position)
            verts.push(x);
            verts.push(y);
            verts.push(z);
            // texcoord
            verts.push(s as f32 / sectors as f32);
            verts.push(r as f32 / rings as f32);
        }
    }

    let row_len = sectors + 1;
    for r in 0..rings {
        for s in 0..sectors {
            let a = r * row_len + s;
            let b = a + row_len;
            indices.push(a as u16);
            indices.push(b as u16);
            indices.push((a + 1) as u16);
            indices.push((a + 1) as u16);
            indices.push(b as u16);
            indices.push((b + 1) as u16);
        }
    }
    (verts, indices)
}

/// Generates a cube with per-face normals, interleaved (pos3 + normal3 + uv2).
///
/// Returns `(vertices, indices)`.
pub fn generate_cube() -> (Vec<f32>, Vec<u16>) {
    #[rustfmt::skip]
    let verts: Vec<f32> = alloc::vec![
        // Front face (z = +1)
        -1.0, -1.0,  1.0,  0.0,  0.0,  1.0,  0.0, 0.0,
         1.0, -1.0,  1.0,  0.0,  0.0,  1.0,  1.0, 0.0,
         1.0,  1.0,  1.0,  0.0,  0.0,  1.0,  1.0, 1.0,
        -1.0,  1.0,  1.0,  0.0,  0.0,  1.0,  0.0, 1.0,
        // Back face (z = -1)
         1.0, -1.0, -1.0,  0.0,  0.0, -1.0,  0.0, 0.0,
        -1.0, -1.0, -1.0,  0.0,  0.0, -1.0,  1.0, 0.0,
        -1.0,  1.0, -1.0,  0.0,  0.0, -1.0,  1.0, 1.0,
         1.0,  1.0, -1.0,  0.0,  0.0, -1.0,  0.0, 1.0,
        // Top face (y = +1)
        -1.0,  1.0,  1.0,  0.0,  1.0,  0.0,  0.0, 0.0,
         1.0,  1.0,  1.0,  0.0,  1.0,  0.0,  1.0, 0.0,
         1.0,  1.0, -1.0,  0.0,  1.0,  0.0,  1.0, 1.0,
        -1.0,  1.0, -1.0,  0.0,  1.0,  0.0,  0.0, 1.0,
        // Bottom face (y = -1)
        -1.0, -1.0, -1.0,  0.0, -1.0,  0.0,  0.0, 0.0,
         1.0, -1.0, -1.0,  0.0, -1.0,  0.0,  1.0, 0.0,
         1.0, -1.0,  1.0,  0.0, -1.0,  0.0,  1.0, 1.0,
        -1.0, -1.0,  1.0,  0.0, -1.0,  0.0,  0.0, 1.0,
        // Right face (x = +1)
         1.0, -1.0,  1.0,  1.0,  0.0,  0.0,  0.0, 0.0,
         1.0, -1.0, -1.0,  1.0,  0.0,  0.0,  1.0, 0.0,
         1.0,  1.0, -1.0,  1.0,  0.0,  0.0,  1.0, 1.0,
         1.0,  1.0,  1.0,  1.0,  0.0,  0.0,  0.0, 1.0,
        // Left face (x = -1)
        -1.0, -1.0, -1.0, -1.0,  0.0,  0.0,  0.0, 0.0,
        -1.0, -1.0,  1.0, -1.0,  0.0,  0.0,  1.0, 0.0,
        -1.0,  1.0,  1.0, -1.0,  0.0,  0.0,  1.0, 1.0,
        -1.0,  1.0, -1.0, -1.0,  0.0,  0.0,  0.0, 1.0,
    ];
    #[rustfmt::skip]
    let indices: Vec<u16> = alloc::vec![
         0,  1,  2,   2,  3,  0,   // front
         4,  5,  6,   6,  7,  4,   // back
         8,  9, 10,  10, 11,  8,   // top
        12, 13, 14,  14, 15, 12,   // bottom
        16, 17, 18,  18, 19, 16,   // right
        20, 21, 22,  22, 23, 20,   // left
    ];
    (verts, indices)
}

/// Sets up interleaved vertex attributes (pos3 + normal3 + uv2, stride 32).
pub fn setup_vertex_attribs(program: u32) {
    let loc_pos = gl::get_attrib_location(program, "aPosition");
    let loc_norm = gl::get_attrib_location(program, "aNormal");
    let loc_uv = gl::get_attrib_location(program, "aTexCoord");

    if loc_pos >= 0 {
        gl::enable_vertex_attrib_array(loc_pos as u32);
        gl::vertex_attrib_pointer(loc_pos as u32, 3, gl::GL_FLOAT, false, 32, 0);
    }
    if loc_norm >= 0 {
        gl::enable_vertex_attrib_array(loc_norm as u32);
        gl::vertex_attrib_pointer(loc_norm as u32, 3, gl::GL_FLOAT, false, 32, 12);
    }
    if loc_uv >= 0 {
        gl::enable_vertex_attrib_array(loc_uv as u32);
        gl::vertex_attrib_pointer(loc_uv as u32, 2, gl::GL_FLOAT, false, 32, 24);
    }
}

/// Sets up simple vertex attributes (pos3 + color3, stride 24, no normals/uv).
pub fn setup_pos_color_attribs(program: u32) {
    let loc_pos = gl::get_attrib_location(program, "aPosition");
    let loc_col = gl::get_attrib_location(program, "aColor");

    if loc_pos >= 0 {
        gl::enable_vertex_attrib_array(loc_pos as u32);
        gl::vertex_attrib_pointer(loc_pos as u32, 3, gl::GL_FLOAT, false, 24, 0);
    }
    if loc_col >= 0 {
        gl::enable_vertex_attrib_array(loc_col as u32);
        gl::vertex_attrib_pointer(loc_col as u32, 3, gl::GL_FLOAT, false, 24, 12);
    }
}

/// Sets up vertex attributes (pos3 + uv2, stride 20, no normals/color).
pub fn setup_pos_uv_attribs(program: u32) {
    let loc_pos = gl::get_attrib_location(program, "aPosition");
    let loc_uv = gl::get_attrib_location(program, "aTexCoord");

    if loc_pos >= 0 {
        gl::enable_vertex_attrib_array(loc_pos as u32);
        gl::vertex_attrib_pointer(loc_pos as u32, 3, gl::GL_FLOAT, false, 20, 0);
    }
    if loc_uv >= 0 {
        gl::enable_vertex_attrib_array(loc_uv as u32);
        gl::vertex_attrib_pointer(loc_uv as u32, 2, gl::GL_FLOAT, false, 20, 12);
    }
}
