//! gldemo — Phong-lit 3D scene with sphere, cube, textures and animated lights.
//!
//! Demonstrates the optimized libgl software rasterizer with:
//! - Per-fragment Phong shading (ambient + diffuse + specular)
//! - Two animated point lights with different colors
//! - Procedural checkerboard texture
//! - UV sphere + textured cube rotating in 3D space
//! - 60fps animation loop via anyui timer

#![no_std]
#![no_main]
#![allow(unused, dead_code)]

anyos_std::entry!(main);

use alloc::vec;
use alloc::vec::Vec;

use libgl_client as gl;

// ── Shaders ──────────────────────────────────────────────────────────────────

/// Vertex shader: transforms to clip space, passes world position, normal, UV.
static VS_SOURCE: &str =
"attribute vec3 aPosition;
attribute vec3 aNormal;
attribute vec2 aTexCoord;
uniform mat4 uMVP;
uniform mat4 uModel;
varying vec3 vNormal;
varying vec3 vWorldPos;
varying vec2 vTexCoord;
void main() {
    vec4 worldPos = uModel * vec4(aPosition, 1.0);
    vWorldPos = worldPos.xyz;
    vec4 tn = uModel * vec4(aNormal, 0.0);
    vNormal = tn.xyz;
    vTexCoord = aTexCoord;
    gl_Position = uMVP * vec4(aPosition, 1.0);
}
";

/// Fragment shader: Blinn-Phong with 2 point lights + texture.
static FS_SOURCE: &str =
"varying vec3 vNormal;
varying vec3 vWorldPos;
varying vec2 vTexCoord;
uniform vec3 uLightPos0;
uniform vec3 uLightColor0;
uniform vec3 uLightPos1;
uniform vec3 uLightColor1;
uniform vec3 uEyePos;
uniform sampler2D uTexture;
uniform vec4 uMatColor;
void main() {
    vec3 N = normalize(vNormal);
    vec3 V = normalize(uEyePos - vWorldPos);
    vec3 ambient = vec3(0.08, 0.08, 0.1);
    vec3 L0 = normalize(uLightPos0 - vWorldPos);
    float diff0 = max(dot(N, L0), 0.0);
    vec3 H0 = normalize(L0 + V);
    float spec0 = pow(max(dot(N, H0), 0.0), 64.0);
    vec3 c0 = uLightColor0 * diff0 + uLightColor0 * spec0;
    vec3 L1 = normalize(uLightPos1 - vWorldPos);
    float diff1 = max(dot(N, L1), 0.0);
    vec3 H1 = normalize(L1 + V);
    float spec1 = pow(max(dot(N, H1), 0.0), 64.0);
    vec3 c1 = uLightColor1 * diff1 + uLightColor1 * spec1;
    vec3 lighting = ambient + c0 + c1;
    vec4 texColor = texture2D(uTexture, vTexCoord);
    vec3 baseColor = texColor.rgb * uMatColor.rgb;
    gl_FragColor = vec4(lighting * baseColor, 1.0);
}
";

// ── Math helpers ─────────────────────────────────────────────────────────────

type Mat4 = [f32; 16];

fn mat4_identity() -> Mat4 {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Column-major mat4 multiply: result = a * b.
fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            r[col * 4 + row] =
                a[0 * 4 + row] * b[col * 4 + 0] +
                a[1 * 4 + row] * b[col * 4 + 1] +
                a[2 * 4 + row] * b[col * 4 + 2] +
                a[3 * 4 + row] * b[col * 4 + 3];
        }
    }
    r
}

fn mat4_translate(tx: f32, ty: f32, tz: f32) -> Mat4 {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
         tx,  ty,  tz, 1.0,
    ]
}

fn mat4_scale(sx: f32, sy: f32, sz: f32) -> Mat4 {
    [
         sx, 0.0, 0.0, 0.0,
        0.0,  sy, 0.0, 0.0,
        0.0, 0.0,  sz, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn mat4_rotate_y(angle: f32) -> Mat4 {
    let c = gl::cos(angle);
    let s = gl::sin(angle);
    [
          c, 0.0,  -s, 0.0,
        0.0, 1.0, 0.0, 0.0,
          s, 0.0,   c, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn mat4_rotate_x(angle: f32) -> Mat4 {
    let c = gl::cos(angle);
    let s = gl::sin(angle);
    [
        1.0, 0.0, 0.0, 0.0,
        0.0,   c,   s, 0.0,
        0.0,  -s,   c, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Perspective projection matrix (column-major).
fn mat4_perspective(fov_rad: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / gl::tan(fov_rad * 0.5);
    let nf = 1.0 / (near - far);
    [
        f / aspect, 0.0,            0.0, 0.0,
              0.0,    f,            0.0, 0.0,
              0.0,  0.0, (far + near) * nf, -1.0,
              0.0,  0.0, 2.0 * far * near * nf, 0.0,
    ]
}

// ── Geometry generation ──────────────────────────────────────────────────────

/// Interleaved vertex: position(3) + normal(3) + texcoord(2) = 8 floats.
const VERTEX_STRIDE: usize = 8;

/// Generate a UV sphere.
/// Returns (vertex_data, index_data).
fn generate_sphere(rings: u32, sectors: u32) -> (Vec<f32>, Vec<u16>) {
    let pi: f32 = 3.14159265;
    let mut verts = Vec::new();
    let mut indices = Vec::new();

    for r in 0..=rings {
        let phi = pi * r as f32 / rings as f32; // 0..PI
        let sp = gl::sin(phi);
        let cp = gl::cos(phi);

        for s in 0..=sectors {
            let theta = 2.0 * pi * s as f32 / sectors as f32; // 0..2PI
            let st = gl::sin(theta);
            let ct = gl::cos(theta);

            // Position (unit sphere)
            let x = sp * ct;
            let y = cp;
            let z = sp * st;

            // Normal = position (unit sphere)
            let nx = x;
            let ny = y;
            let nz = z;

            // UV
            let u = s as f32 / sectors as f32;
            let v = r as f32 / rings as f32;

            verts.extend_from_slice(&[x, y, z, nx, ny, nz, u, v]);
        }
    }

    // Indices (two triangles per quad)
    let row_len = sectors + 1;
    for r in 0..rings {
        for s in 0..sectors {
            let i0 = (r * row_len + s) as u16;
            let i1 = (r * row_len + s + 1) as u16;
            let i2 = ((r + 1) * row_len + s) as u16;
            let i3 = ((r + 1) * row_len + s + 1) as u16;

            indices.extend_from_slice(&[i0, i1, i2]);
            indices.extend_from_slice(&[i1, i3, i2]);
        }
    }

    (verts, indices)
}

/// Generate a cube with per-face normals.
/// Returns (vertex_data, index_data).
fn generate_cube() -> (Vec<f32>, Vec<u16>) {
    // 6 faces × 4 vertices = 24 vertices
    #[rustfmt::skip]
    let verts: Vec<f32> = vec![
        // Front face (z=+0.5, normal 0,0,1)
        -0.5, -0.5,  0.5,  0.0,  0.0,  1.0,  0.0, 0.0,
         0.5, -0.5,  0.5,  0.0,  0.0,  1.0,  1.0, 0.0,
         0.5,  0.5,  0.5,  0.0,  0.0,  1.0,  1.0, 1.0,
        -0.5,  0.5,  0.5,  0.0,  0.0,  1.0,  0.0, 1.0,
        // Back face (z=-0.5, normal 0,0,-1)
         0.5, -0.5, -0.5,  0.0,  0.0, -1.0,  0.0, 0.0,
        -0.5, -0.5, -0.5,  0.0,  0.0, -1.0,  1.0, 0.0,
        -0.5,  0.5, -0.5,  0.0,  0.0, -1.0,  1.0, 1.0,
         0.5,  0.5, -0.5,  0.0,  0.0, -1.0,  0.0, 1.0,
        // Right face (x=+0.5, normal 1,0,0)
         0.5, -0.5,  0.5,  1.0,  0.0,  0.0,  0.0, 0.0,
         0.5, -0.5, -0.5,  1.0,  0.0,  0.0,  1.0, 0.0,
         0.5,  0.5, -0.5,  1.0,  0.0,  0.0,  1.0, 1.0,
         0.5,  0.5,  0.5,  1.0,  0.0,  0.0,  0.0, 1.0,
        // Left face (x=-0.5, normal -1,0,0)
        -0.5, -0.5, -0.5, -1.0,  0.0,  0.0,  0.0, 0.0,
        -0.5, -0.5,  0.5, -1.0,  0.0,  0.0,  1.0, 0.0,
        -0.5,  0.5,  0.5, -1.0,  0.0,  0.0,  1.0, 1.0,
        -0.5,  0.5, -0.5, -1.0,  0.0,  0.0,  0.0, 1.0,
        // Top face (y=+0.5, normal 0,1,0)
        -0.5,  0.5,  0.5,  0.0,  1.0,  0.0,  0.0, 0.0,
         0.5,  0.5,  0.5,  0.0,  1.0,  0.0,  1.0, 0.0,
         0.5,  0.5, -0.5,  0.0,  1.0,  0.0,  1.0, 1.0,
        -0.5,  0.5, -0.5,  0.0,  1.0,  0.0,  0.0, 1.0,
        // Bottom face (y=-0.5, normal 0,-1,0)
        -0.5, -0.5, -0.5,  0.0, -1.0,  0.0,  0.0, 0.0,
         0.5, -0.5, -0.5,  0.0, -1.0,  0.0,  1.0, 0.0,
         0.5, -0.5,  0.5,  0.0, -1.0,  0.0,  1.0, 1.0,
        -0.5, -0.5,  0.5,  0.0, -1.0,  0.0,  0.0, 1.0,
    ];

    // 6 faces × 2 triangles × 3 indices = 36 indices
    let mut indices = Vec::with_capacity(36);
    for face in 0..6u16 {
        let base = face * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2]);
        indices.extend_from_slice(&[base, base + 2, base + 3]);
    }

    (verts, indices)
}

/// Generate a 64×64 checkerboard texture (RGBA8).
fn generate_checkerboard(size: u32, check_size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let cx = x / check_size;
            let cy = y / check_size;
            let white = (cx + cy) % 2 == 0;
            if white {
                data.extend_from_slice(&[220, 220, 230, 255]);
            } else {
                data.extend_from_slice(&[50, 50, 60, 255]);
            }
        }
    }
    data
}

/// Generate a gradient texture for the sphere (RGBA8).
fn generate_gradient(size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / size as f32;
            // Warm gradient: gold-ish tones
            let r = (200.0 + 55.0 * u) as u8;
            let g = (160.0 + 80.0 * v) as u8;
            let b = (100.0 + 40.0 * u * v) as u8;
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    data
}

// ── Render state ─────────────────────────────────────────────────────────────

struct RenderState {
    canvas: libanyui_client::Canvas,
    fb_w: u32,
    fb_h: u32,
    program: u32,
    // Sphere
    sphere_vbo: u32,
    sphere_ebo: u32,
    sphere_num_indices: i32,
    // Cube
    cube_vbo: u32,
    cube_ebo: u32,
    cube_num_indices: i32,
    // Textures
    checker_tex: u32,
    gradient_tex: u32,
    // Uniform locations
    loc_mvp: i32,
    loc_model: i32,
    loc_light_pos0: i32,
    loc_light_color0: i32,
    loc_light_pos1: i32,
    loc_light_color1: i32,
    loc_eye_pos: i32,
    loc_texture: i32,
    loc_mat_color: i32,
    // Animation
    frame: u32,
}

static mut STATE: Option<RenderState> = None;

fn render_frame() {
    let s = unsafe { STATE.as_mut().unwrap() };
    s.frame += 1;

    // Dynamic resize: query actual canvas dimensions each frame
    let cur_w = s.canvas.get_stride();
    let cur_h = s.canvas.get_height();
    if cur_w == 0 || cur_h == 0 { return; }
    if cur_w != s.fb_w || cur_h != s.fb_h {
        s.fb_w = cur_w;
        s.fb_h = cur_h;
        gl::gl_resize(cur_w, cur_h);
        gl::viewport(0, 0, cur_w as i32, cur_h as i32);
    }

    let t = s.frame as f32 * 0.02; // time in pseudo-seconds

    // ── Setup ────────────────────────────────────────────────────────────
    gl::clear_color(0.05, 0.05, 0.12, 1.0);
    gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);

    // Camera — view = Rx(pitch) * T(-eye), positive pitch tilts down
    let eye = [0.0f32, 1.8, 4.5];
    let aspect = s.fb_w as f32 / s.fb_h as f32;
    let proj = mat4_perspective(0.9, aspect, 0.1, 50.0);
    let view = mat4_mul(&mat4_rotate_x(0.38), &mat4_translate(-eye[0], -eye[1], -eye[2]));

    // Animated lights
    let l0_x = gl::sin(t * 0.7) * 3.0;
    let l0_z = gl::cos(t * 0.7) * 3.0;
    let l1_x = gl::sin(t * 0.5 + 2.0) * 2.5;
    let l1_y = 1.5 + gl::sin(t * 0.8) * 1.0;

    gl::uniform3f(s.loc_light_pos0, l0_x, 2.0, l0_z);
    gl::uniform3f(s.loc_light_color0, 0.9, 0.7, 0.4);
    gl::uniform3f(s.loc_light_pos1, l1_x, l1_y, 2.0);
    gl::uniform3f(s.loc_light_color1, 0.3, 0.5, 0.9);
    gl::uniform3f(s.loc_eye_pos, eye[0], eye[1], eye[2]);

    // ── Draw sphere ──────────────────────────────────────────────────────
    {
        let model = mat4_mul(
            &mat4_translate(-1.0, 0.5, 0.0),
            &mat4_mul(&mat4_rotate_y(t * 0.5), &mat4_scale(0.8, 0.8, 0.8)),
        );
        let mvp = mat4_mul(&proj, &mat4_mul(&view, &model));

        gl::uniform_matrix4fv(s.loc_mvp, false, &mvp);
        gl::uniform_matrix4fv(s.loc_model, false, &model);
        gl::uniform4f(s.loc_mat_color, 1.0, 1.0, 1.0, 1.0);

        // Bind sphere texture + VBO/EBO
        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, s.gradient_tex);
        gl::uniform1i(s.loc_texture, 0);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.sphere_vbo);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, s.sphere_ebo);
        setup_vertex_attribs(s.program);

        gl::draw_elements(gl::GL_TRIANGLES, s.sphere_num_indices, gl::GL_UNSIGNED_SHORT, 0);
    }

    // ── Draw cube ────────────────────────────────────────────────────────
    {
        let model = mat4_mul(
            &mat4_translate(1.2, 0.5, 0.0),
            &mat4_mul(
                &mat4_rotate_y(t * 0.8),
                &mat4_mul(&mat4_rotate_x(t * 0.3), &mat4_scale(0.9, 0.9, 0.9)),
            ),
        );
        let mvp = mat4_mul(&proj, &mat4_mul(&view, &model));

        gl::uniform_matrix4fv(s.loc_mvp, false, &mvp);
        gl::uniform_matrix4fv(s.loc_model, false, &model);
        gl::uniform4f(s.loc_mat_color, 1.0, 1.0, 1.0, 1.0);

        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, s.checker_tex);
        gl::uniform1i(s.loc_texture, 0);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.cube_vbo);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, s.cube_ebo);
        setup_vertex_attribs(s.program);

        gl::draw_elements(gl::GL_TRIANGLES, s.cube_num_indices, gl::GL_UNSIGNED_SHORT, 0);
    }

    // ── Draw floor plane ─────────────────────────────────────────────────
    {
        let model = mat4_mul(
            &mat4_translate(0.0, -0.5, 0.0),
            &mat4_scale(5.0, 1.0, 5.0),
        );
        let mvp = mat4_mul(&proj, &mat4_mul(&view, &model));

        gl::uniform_matrix4fv(s.loc_mvp, false, &mvp);
        gl::uniform_matrix4fv(s.loc_model, false, &model);
        gl::uniform4f(s.loc_mat_color, 0.6, 0.6, 0.7, 1.0);

        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, s.checker_tex);

        // Floor: two triangles as GL_TRIANGLES (non-indexed)
        #[rustfmt::skip]
        let floor: [f32; 48] = [
            // pos          normal       uv
            -0.5, 0.0, -0.5,  0.0, 1.0, 0.0,  0.0, 0.0,
             0.5, 0.0, -0.5,  0.0, 1.0, 0.0,  3.0, 0.0,
             0.5, 0.0,  0.5,  0.0, 1.0, 0.0,  3.0, 3.0,
            -0.5, 0.0, -0.5,  0.0, 1.0, 0.0,  0.0, 0.0,
             0.5, 0.0,  0.5,  0.0, 1.0, 0.0,  3.0, 3.0,
            -0.5, 0.0,  0.5,  0.0, 1.0, 0.0,  0.0, 3.0,
        ];

        let mut floor_vbo = [0u32; 1];
        gl::gen_buffers(1, &mut floor_vbo);
        gl::bind_buffer(gl::GL_ARRAY_BUFFER, floor_vbo[0]);
        gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &floor, gl::GL_STATIC_DRAW);
        setup_vertex_attribs(s.program);

        gl::draw_arrays(gl::GL_TRIANGLES, 0, 6);
        gl::delete_buffers(&floor_vbo);
    }

    // ── Swap to canvas ───────────────────────────────────────────────────
    let fb_ptr = gl::swap_buffers();
    if !fb_ptr.is_null() {
        let pixels = unsafe {
            core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize)
        };
        s.canvas.copy_pixels_from(pixels);
    }
}

/// Configure vertex attribute pointers for the interleaved format:
/// position(3) + normal(3) + texcoord(2), stride=32 bytes.
fn setup_vertex_attribs(program: u32) {
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

fn main() {
    anyos_std::println!("gldemo: starting Phong shading demo");

    // Initialize anyui
    libanyui_client::init();
    let window = libanyui_client::Window::new("GL Demo - Phong", 80, 60, 420, 460);

    // Canvas fills the entire window client area
    let canvas = libanyui_client::Canvas::new(400, 400);
    canvas.set_dock(libanyui_client::DOCK_FILL);
    window.add(&canvas);
    window.set_visible(true);

    let fb_w = canvas.get_stride();
    let fb_h = canvas.get_height();
    anyos_std::println!("gldemo: canvas {}x{}", fb_w, fb_h);

    // Initialize libgl
    if !gl::init() {
        anyos_std::println!("gldemo: failed to load libgl.so");
        return;
    }

    // Use actual canvas size (may differ from initial 400x400 due to dock)
    let fb_w = if fb_w > 0 { fb_w } else { 400 };
    let fb_h = if fb_h > 0 { fb_h } else { 400 };

    gl::gl_init(fb_w, fb_h);
    gl::viewport(0, 0, fb_w as i32, fb_h as i32);
    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::enable(gl::GL_CULL_FACE);
    gl::cull_face(gl::GL_BACK);
    gl::set_fxaa(false);

    // HW/SW toggle (overlaid on canvas)
    let hw_available = gl::has_hw_backend();
    let hw_label = libanyui_client::Label::new("HW");
    hw_label.set_position(10, 6);
    hw_label.set_text_color(0xFFCCCCCC);
    hw_label.set_font_size(13);
    window.add(&hw_label);

    let hw_toggle = libanyui_client::Toggle::new(hw_available);
    hw_toggle.set_position(40, 4);
    hw_toggle.on_checked_changed(|e| { gl::set_hw_backend(e.checked); });
    window.add(&hw_toggle);

    // ── Compile shaders ──────────────────────────────────────────────────
    let vs = gl::create_shader(gl::GL_VERTEX_SHADER);
    gl::shader_source(vs, VS_SOURCE);
    gl::compile_shader(vs);
    if !gl::get_shader_compile_status(vs) {
        anyos_std::println!("gldemo: VS compile FAILED");
        return;
    }

    let fs = gl::create_shader(gl::GL_FRAGMENT_SHADER);
    gl::shader_source(fs, FS_SOURCE);
    gl::compile_shader(fs);
    if !gl::get_shader_compile_status(fs) {
        anyos_std::println!("gldemo: FS compile FAILED");
        return;
    }

    let program = gl::create_program();
    gl::attach_shader(program, vs);
    gl::attach_shader(program, fs);
    gl::link_program(program);
    if !gl::get_program_link_status(program) {
        anyos_std::println!("gldemo: link FAILED");
        return;
    }
    gl::use_program(program);
    anyos_std::println!("gldemo: shaders compiled OK");

    // ── Query uniform locations ──────────────────────────────────────────
    let loc_mvp = gl::get_uniform_location(program, "uMVP");
    let loc_model = gl::get_uniform_location(program, "uModel");
    let loc_light_pos0 = gl::get_uniform_location(program, "uLightPos0");
    let loc_light_color0 = gl::get_uniform_location(program, "uLightColor0");
    let loc_light_pos1 = gl::get_uniform_location(program, "uLightPos1");
    let loc_light_color1 = gl::get_uniform_location(program, "uLightColor1");
    let loc_eye_pos = gl::get_uniform_location(program, "uEyePos");
    let loc_texture = gl::get_uniform_location(program, "uTexture");
    let loc_mat_color = gl::get_uniform_location(program, "uMatColor");

    anyos_std::println!("gldemo: uniforms: mvp={} model={} lp0={} lc0={} lp1={} lc1={} eye={} tex={} mat={}",
        loc_mvp, loc_model, loc_light_pos0, loc_light_color0,
        loc_light_pos1, loc_light_color1, loc_eye_pos, loc_texture, loc_mat_color);

    // ── Generate sphere geometry ─────────────────────────────────────────
    let (sphere_verts, sphere_indices) = generate_sphere(10, 16);
    let sphere_num_indices = sphere_indices.len() as i32;

    let mut sphere_vbo = [0u32; 1];
    gl::gen_buffers(1, &mut sphere_vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, sphere_vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &sphere_verts, gl::GL_STATIC_DRAW);

    let mut sphere_ebo = [0u32; 1];
    gl::gen_buffers(1, &mut sphere_ebo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, sphere_ebo[0]);
    gl::buffer_data_u16(gl::GL_ELEMENT_ARRAY_BUFFER, &sphere_indices, gl::GL_STATIC_DRAW);

    anyos_std::println!("gldemo: sphere: {} verts, {} indices",
        sphere_verts.len() / VERTEX_STRIDE, sphere_num_indices);

    // ── Generate cube geometry ───────────────────────────────────────────
    let (cube_verts, cube_indices) = generate_cube();
    let cube_num_indices = cube_indices.len() as i32;

    let mut cube_vbo = [0u32; 1];
    gl::gen_buffers(1, &mut cube_vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, cube_vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &cube_verts, gl::GL_STATIC_DRAW);

    let mut cube_ebo = [0u32; 1];
    gl::gen_buffers(1, &mut cube_ebo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, cube_ebo[0]);
    gl::buffer_data_u16(gl::GL_ELEMENT_ARRAY_BUFFER, &cube_indices, gl::GL_STATIC_DRAW);

    anyos_std::println!("gldemo: cube: {} verts, {} indices",
        cube_verts.len() / VERTEX_STRIDE, cube_num_indices);

    // ── Generate textures ────────────────────────────────────────────────
    let checker_data = generate_checkerboard(64, 8);
    let mut checker_tex = [0u32; 1];
    gl::gen_textures(1, &mut checker_tex);
    gl::bind_texture(gl::GL_TEXTURE_2D, checker_tex[0]);
    gl::tex_image_2d(gl::GL_TEXTURE_2D, 0, gl::GL_RGBA as i32, 64, 64, 0,
        gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, &checker_data);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST as i32);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_WRAP_S, gl::GL_REPEAT as i32);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_WRAP_T, gl::GL_REPEAT as i32);

    let gradient_data = generate_gradient(64);
    let mut gradient_tex = [0u32; 1];
    gl::gen_textures(1, &mut gradient_tex);
    gl::bind_texture(gl::GL_TEXTURE_2D, gradient_tex[0]);
    gl::tex_image_2d(gl::GL_TEXTURE_2D, 0, gl::GL_RGBA as i32, 64, 64, 0,
        gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, &gradient_data);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_LINEAR as i32);

    anyos_std::println!("gldemo: textures created");

    // ── Store render state ───────────────────────────────────────────────
    unsafe {
        STATE = Some(RenderState {
            canvas,
            fb_w,
            fb_h,
            program,
            sphere_vbo: sphere_vbo[0],
            sphere_ebo: sphere_ebo[0],
            sphere_num_indices,
            cube_vbo: cube_vbo[0],
            cube_ebo: cube_ebo[0],
            cube_num_indices,
            checker_tex: checker_tex[0],
            gradient_tex: gradient_tex[0],
            loc_mvp,
            loc_model,
            loc_light_pos0,
            loc_light_color0,
            loc_light_pos1,
            loc_light_color1,
            loc_eye_pos,
            loc_texture,
            loc_mat_color,
            frame: 0,
        });
    }

    // ── 60fps animation timer ────────────────────────────────────────────
    libanyui_client::set_timer(16, || {
        render_frame();
    });

    anyos_std::println!("gldemo: entering event loop");
    libanyui_client::run();
}
