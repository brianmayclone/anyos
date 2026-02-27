//! gldemo — OpenGL ES 2.0 rotating cube demo for anyOS.
//!
//! Renders a colored cube using the libgl software rasterizer,
//! displaying the result in an anyui Canvas window.

#![no_std]
#![no_main]
#![allow(unused, dead_code)]

anyos_std::entry!(main);

use libgl_client as gl;

/// Vertex data for a colored cube: position (x,y,z) + color (r,g,b).
/// 36 vertices (6 faces * 2 triangles * 3 vertices).
#[rustfmt::skip]
static CUBE_VERTICES: [f32; 216] = [
    // Front face (red)
    -0.5, -0.5,  0.5,  1.0, 0.0, 0.0,
     0.5, -0.5,  0.5,  1.0, 0.0, 0.0,
     0.5,  0.5,  0.5,  1.0, 0.0, 0.0,
    -0.5, -0.5,  0.5,  1.0, 0.0, 0.0,
     0.5,  0.5,  0.5,  1.0, 0.0, 0.0,
    -0.5,  0.5,  0.5,  1.0, 0.0, 0.0,
    // Back face (green)
    -0.5, -0.5, -0.5,  0.0, 1.0, 0.0,
    -0.5,  0.5, -0.5,  0.0, 1.0, 0.0,
     0.5,  0.5, -0.5,  0.0, 1.0, 0.0,
    -0.5, -0.5, -0.5,  0.0, 1.0, 0.0,
     0.5,  0.5, -0.5,  0.0, 1.0, 0.0,
     0.5, -0.5, -0.5,  0.0, 1.0, 0.0,
    // Top face (blue)
    -0.5,  0.5, -0.5,  0.0, 0.0, 1.0,
    -0.5,  0.5,  0.5,  0.0, 0.0, 1.0,
     0.5,  0.5,  0.5,  0.0, 0.0, 1.0,
    -0.5,  0.5, -0.5,  0.0, 0.0, 1.0,
     0.5,  0.5,  0.5,  0.0, 0.0, 1.0,
     0.5,  0.5, -0.5,  0.0, 0.0, 1.0,
    // Bottom face (yellow)
    -0.5, -0.5, -0.5,  1.0, 1.0, 0.0,
     0.5, -0.5, -0.5,  1.0, 1.0, 0.0,
     0.5, -0.5,  0.5,  1.0, 1.0, 0.0,
    -0.5, -0.5, -0.5,  1.0, 1.0, 0.0,
     0.5, -0.5,  0.5,  1.0, 1.0, 0.0,
    -0.5, -0.5,  0.5,  1.0, 1.0, 0.0,
    // Right face (magenta)
     0.5, -0.5, -0.5,  1.0, 0.0, 1.0,
     0.5,  0.5, -0.5,  1.0, 0.0, 1.0,
     0.5,  0.5,  0.5,  1.0, 0.0, 1.0,
     0.5, -0.5, -0.5,  1.0, 0.0, 1.0,
     0.5,  0.5,  0.5,  1.0, 0.0, 1.0,
     0.5, -0.5,  0.5,  1.0, 0.0, 1.0,
    // Left face (cyan)
    -0.5, -0.5, -0.5,  0.0, 1.0, 1.0,
    -0.5, -0.5,  0.5,  0.0, 1.0, 1.0,
    -0.5,  0.5,  0.5,  0.0, 1.0, 1.0,
    -0.5, -0.5, -0.5,  0.0, 1.0, 1.0,
    -0.5,  0.5,  0.5,  0.0, 1.0, 1.0,
    -0.5,  0.5, -0.5,  0.0, 1.0, 1.0,
];

/// Vertex shader: MVP transform + pass-through color.
static VS_SOURCE: &str =
"attribute vec3 aPosition;
attribute vec3 aColor;
uniform mat4 uMVP;
varying vec3 vColor;
void main() {
    gl_Position = uMVP * vec4(aPosition, 1.0);
    vColor = aColor;
}
";

/// Fragment shader: output interpolated color.
static FS_SOURCE: &str =
"precision mediump float;
varying vec3 vColor;
void main() {
    gl_FragColor = vec4(vColor, 1.0);
}
";

/// Mutable render state accessed from the timer callback.
struct RenderState {
    canvas: libanyui_client::Canvas,
    fb_w: u32,
    fb_h: u32,
    loc_mvp: i32,
    angle: f32,
    frame: u32,
}

static mut STATE: Option<RenderState> = None;

fn render_frame() {
    let s = unsafe { STATE.as_mut().unwrap() };

    // Build MVP matrix
    let mvp = build_mvp(s.angle);
    if s.loc_mvp >= 0 {
        gl::uniform_matrix4fv(s.loc_mvp, false, &mvp);
    }

    // Render
    gl::clear_color(0.1, 0.1, 0.15, 1.0);
    gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);
    gl::draw_arrays(gl::GL_TRIANGLES, 0, 36);

    // Copy to canvas
    let fb_ptr = gl::swap_buffers();
    if !fb_ptr.is_null() {
        let pixels = unsafe {
            core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize)
        };
        if s.frame == 0 {
            let center = 200 * s.fb_w as usize + 200;
            anyos_std::println!("gldemo: frame0 center={:#010x}", pixels[center]);
        }
        s.canvas.copy_pixels_from(pixels);
    }

    s.angle += 0.02;
    if s.angle > 6.28318 { s.angle -= 6.28318; }
    s.frame += 1;
}

fn main() {
    anyos_std::println!("gldemo: starting");

    // Initialize anyui
    libanyui_client::init();
    let window = libanyui_client::Window::new("GL Demo", 100, 100, 420, 420);

    let canvas = libanyui_client::Canvas::new(400, 400);
    canvas.set_position(0, 0);
    window.add(&canvas);
    window.set_visible(true);

    let fb_w = canvas.get_stride();
    let fb_h = canvas.get_height();
    anyos_std::println!("gldemo: canvas stride={} height={}", fb_w, fb_h);
    if fb_w == 0 || fb_h == 0 {
        anyos_std::println!("gldemo: canvas dimensions are zero, aborting");
        return;
    }

    // Initialize libgl
    if !gl::init() {
        anyos_std::println!("gldemo: failed to load libgl.so");
        return;
    }
    anyos_std::println!("gldemo: libgl loaded OK");

    gl::gl_init(fb_w, fb_h);
    gl::viewport(0, 0, fb_w as i32, fb_h as i32);
    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::enable(gl::GL_CULL_FACE);
    gl::cull_face(gl::GL_BACK);

    // Compile shaders
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
        anyos_std::println!("gldemo: program link FAILED");
        return;
    }
    gl::use_program(program);
    anyos_std::println!("gldemo: shaders OK");

    // Get locations
    let loc_pos = gl::get_attrib_location(program, "aPosition");
    let loc_col = gl::get_attrib_location(program, "aColor");
    let loc_mvp = gl::get_uniform_location(program, "uMVP");

    // Upload vertex data
    let mut vbo = [0u32; 1];
    gl::gen_buffers(1, &mut vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &CUBE_VERTICES, gl::GL_STATIC_DRAW);

    // Configure vertex attributes (stride = 6 floats * 4 bytes = 24)
    if loc_pos >= 0 {
        gl::enable_vertex_attrib_array(loc_pos as u32);
        gl::vertex_attrib_pointer(loc_pos as u32, 3, gl::GL_FLOAT, false, 24, 0);
    }
    if loc_col >= 0 {
        gl::enable_vertex_attrib_array(loc_col as u32);
        gl::vertex_attrib_pointer(loc_col as u32, 3, gl::GL_FLOAT, false, 24, 12);
    }

    // Store render state for timer callback
    unsafe {
        STATE = Some(RenderState {
            canvas,
            fb_w,
            fb_h,
            loc_mvp,
            angle: 0.0,
            frame: 0,
        });
    }

    // Register 60fps animation timer — anyui event loop handles presentation
    libanyui_client::set_timer(16, || {
        render_frame();
    });

    // Run the anyui event loop (blocks, handles events + timer + compositor)
    libanyui_client::run();
}

/// Build a model-view-projection matrix for the rotating cube.
fn build_mvp(angle: f32) -> [f32; 16] {
    let model = mat4_rotate_y(angle);
    let model = mat4_mul(&mat4_rotate_x(angle * 0.7), &model);
    let view = mat4_translate(0.0, 0.0, -3.0);
    let proj = mat4_perspective(45.0, 1.0, 0.1, 100.0);
    let mv = mat4_mul(&view, &model);
    mat4_mul(&proj, &mv)
}

/// Rotation around Y axis (column-major).
fn mat4_rotate_y(angle: f32) -> [f32; 16] {
    let c = cos_approx(angle);
    let s = sin_approx(angle);
    [
         c,  0.0,   s, 0.0,
        0.0, 1.0, 0.0, 0.0,
        -s,  0.0,   c, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Rotation around X axis (column-major).
fn mat4_rotate_x(angle: f32) -> [f32; 16] {
    let c = cos_approx(angle);
    let s = sin_approx(angle);
    [
        1.0, 0.0, 0.0, 0.0,
        0.0,   c,   s, 0.0,
        0.0,  -s,   c, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Translation matrix (column-major).
fn mat4_translate(x: f32, y: f32, z: f32) -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
          x,   y,   z, 1.0,
    ]
}

/// Perspective projection matrix (column-major).
fn mat4_perspective(fov_deg: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let fov_rad = fov_deg * 3.14159265 / 180.0;
    let f = 1.0 / tan_approx(fov_rad * 0.5);
    let range_inv = 1.0 / (near - far);
    [
        f / aspect, 0.0, 0.0, 0.0,
        0.0, f, 0.0, 0.0,
        0.0, 0.0, (far + near) * range_inv, -1.0,
        0.0, 0.0, 2.0 * far * near * range_inv, 0.0,
    ]
}

/// 4x4 matrix multiply (column-major).
fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
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

// ── Trig approximations (no libm) ──────────────────────────────────────────

fn sin_approx(x: f32) -> f32 {
    let pi = 3.14159265;
    let mut t = x;
    t = t - ((t / (2.0 * pi)).floor_approx()) * 2.0 * pi;
    if t > pi { t -= 2.0 * pi; }
    if t < -pi { t += 2.0 * pi; }
    let abs_t = if t < 0.0 { -t } else { t };
    let y = t * (4.0 / pi - 4.0 / (pi * pi) * abs_t);
    let abs_y = if y < 0.0 { -y } else { y };
    0.225 * (y * abs_y - y) + y
}

fn cos_approx(x: f32) -> f32 {
    sin_approx(x + 3.14159265 * 0.5)
}

fn tan_approx(x: f32) -> f32 {
    let c = cos_approx(x);
    if c.abs() < 1e-10 { return 1e10; }
    sin_approx(x) / c
}

trait FloorApprox { fn floor_approx(self) -> Self; }
impl FloorApprox for f32 {
    fn floor_approx(self) -> f32 {
        let i = self as i32;
        if self < 0.0 && self != i as f32 { (i - 1) as f32 } else { i as f32 }
    }
}

trait AbsApprox { fn abs(self) -> Self; }
impl AbsApprox for f32 {
    fn abs(self) -> f32 { if self < 0.0 { -self } else { self } }
}
