//! gldemo — OpenGL ES 2.0 lit & textured cube demo for anyOS.
//!
//! Renders a Phong-lit cube with a procedural checkerboard texture using
//! the libgl software rasterizer, displayed in an anyui Canvas window.

#![no_std]
#![no_main]
#![allow(unused, dead_code)]

anyos_std::entry!(main);

use libgl_client as gl;

// ── Vertex data: position (3) + normal (3) + texcoord (2) = 8 floats ────────
// 36 vertices (6 faces * 2 triangles * 3 vertices), stride = 32 bytes.

#[rustfmt::skip]
static CUBE_VERTICES: [f32; 288] = [
    // Front face (normal: 0, 0, 1)
    -0.5, -0.5,  0.5,   0.0,  0.0,  1.0,  0.0, 0.0,
     0.5, -0.5,  0.5,   0.0,  0.0,  1.0,  1.0, 0.0,
     0.5,  0.5,  0.5,   0.0,  0.0,  1.0,  1.0, 1.0,
    -0.5, -0.5,  0.5,   0.0,  0.0,  1.0,  0.0, 0.0,
     0.5,  0.5,  0.5,   0.0,  0.0,  1.0,  1.0, 1.0,
    -0.5,  0.5,  0.5,   0.0,  0.0,  1.0,  0.0, 1.0,
    // Back face (normal: 0, 0, -1)
    -0.5, -0.5, -0.5,   0.0,  0.0, -1.0,  1.0, 0.0,
    -0.5,  0.5, -0.5,   0.0,  0.0, -1.0,  1.0, 1.0,
     0.5,  0.5, -0.5,   0.0,  0.0, -1.0,  0.0, 1.0,
    -0.5, -0.5, -0.5,   0.0,  0.0, -1.0,  1.0, 0.0,
     0.5,  0.5, -0.5,   0.0,  0.0, -1.0,  0.0, 1.0,
     0.5, -0.5, -0.5,   0.0,  0.0, -1.0,  0.0, 0.0,
    // Top face (normal: 0, 1, 0)
    -0.5,  0.5, -0.5,   0.0,  1.0,  0.0,  0.0, 0.0,
    -0.5,  0.5,  0.5,   0.0,  1.0,  0.0,  0.0, 1.0,
     0.5,  0.5,  0.5,   0.0,  1.0,  0.0,  1.0, 1.0,
    -0.5,  0.5, -0.5,   0.0,  1.0,  0.0,  0.0, 0.0,
     0.5,  0.5,  0.5,   0.0,  1.0,  0.0,  1.0, 1.0,
     0.5,  0.5, -0.5,   0.0,  1.0,  0.0,  1.0, 0.0,
    // Bottom face (normal: 0, -1, 0)
    -0.5, -0.5, -0.5,   0.0, -1.0,  0.0,  0.0, 1.0,
     0.5, -0.5, -0.5,   0.0, -1.0,  0.0,  1.0, 1.0,
     0.5, -0.5,  0.5,   0.0, -1.0,  0.0,  1.0, 0.0,
    -0.5, -0.5, -0.5,   0.0, -1.0,  0.0,  0.0, 1.0,
     0.5, -0.5,  0.5,   0.0, -1.0,  0.0,  1.0, 0.0,
    -0.5, -0.5,  0.5,   0.0, -1.0,  0.0,  0.0, 0.0,
    // Right face (normal: 1, 0, 0)
     0.5, -0.5, -0.5,   1.0,  0.0,  0.0,  0.0, 0.0,
     0.5,  0.5, -0.5,   1.0,  0.0,  0.0,  0.0, 1.0,
     0.5,  0.5,  0.5,   1.0,  0.0,  0.0,  1.0, 1.0,
     0.5, -0.5, -0.5,   1.0,  0.0,  0.0,  0.0, 0.0,
     0.5,  0.5,  0.5,   1.0,  0.0,  0.0,  1.0, 1.0,
     0.5, -0.5,  0.5,   1.0,  0.0,  0.0,  1.0, 0.0,
    // Left face (normal: -1, 0, 0)
    -0.5, -0.5, -0.5,  -1.0,  0.0,  0.0,  1.0, 0.0,
    -0.5, -0.5,  0.5,  -1.0,  0.0,  0.0,  0.0, 0.0,
    -0.5,  0.5,  0.5,  -1.0,  0.0,  0.0,  0.0, 1.0,
    -0.5, -0.5, -0.5,  -1.0,  0.0,  0.0,  1.0, 0.0,
    -0.5,  0.5,  0.5,  -1.0,  0.0,  0.0,  0.0, 1.0,
    -0.5,  0.5, -0.5,  -1.0,  0.0,  0.0,  1.0, 1.0,
];

// ── Shaders ──────────────────────────────────────────────────────────────────

/// Vertex shader: MVP transform, pass normal/texcoord/world position as varyings.
static VS_SOURCE: &str =
"attribute vec3 aPosition;
attribute vec3 aNormal;
attribute vec2 aTexCoord;
uniform mat4 uMVP;
uniform mat4 uModel;
varying vec3 vNormal;
varying vec2 vTexCoord;
varying vec3 vWorldPos;
void main() {
    gl_Position = uMVP * vec4(aPosition, 1.0);
    vec4 worldNorm = uModel * vec4(aNormal, 0.0);
    vNormal = worldNorm.xyz;
    vTexCoord = aTexCoord;
    vec4 worldPos = uModel * vec4(aPosition, 1.0);
    vWorldPos = worldPos.xyz;
}
";

/// Fragment shader: Phong lighting with texture.
static FS_SOURCE: &str =
"precision mediump float;
uniform vec3 uLightDir;
uniform vec3 uViewPos;
uniform sampler2D uTexture;
varying vec3 vNormal;
varying vec2 vTexCoord;
varying vec3 vWorldPos;
void main() {
    vec3 N = normalize(vNormal);
    vec3 L = normalize(uLightDir);
    float ambient = 0.15;
    float diff = max(dot(N, L), 0.0);
    vec3 V = normalize(uViewPos - vWorldPos);
    vec3 R = reflect(-L, N);
    float spec = pow(max(dot(R, V), 0.0), 32.0);
    vec4 texColor = texture2D(uTexture, vTexCoord);
    vec3 lit = texColor.rgb * (ambient + diff) + vec3(1.0, 1.0, 1.0) * spec * 0.5;
    gl_FragColor = vec4(clamp(lit, 0.0, 1.0), 1.0);
}
";

// ── Render state ─────────────────────────────────────────────────────────────

struct RenderState {
    canvas: libanyui_client::Canvas,
    fb_w: u32,
    fb_h: u32,
    loc_mvp: i32,
    loc_model: i32,
    loc_light_dir: i32,
    loc_view_pos: i32,
    loc_texture: i32,
    angle: f32,
}

static mut STATE: Option<RenderState> = None;

fn render_frame() {
    let s = unsafe { STATE.as_mut().unwrap() };

    // Build model matrix (rotation)
    let model = mat4_rotate_y(s.angle);
    let model = mat4_mul(&mat4_rotate_x(s.angle * 0.7), &model);

    // Build MVP
    let view = mat4_translate(0.0, 0.0, -3.0);
    let proj = mat4_perspective(45.0, 1.0, 0.1, 100.0);
    let mv = mat4_mul(&view, &model);
    let mvp = mat4_mul(&proj, &mv);

    // Upload uniforms
    if s.loc_mvp >= 0 {
        gl::uniform_matrix4fv(s.loc_mvp, false, &mvp);
    }
    if s.loc_model >= 0 {
        gl::uniform_matrix4fv(s.loc_model, false, &model);
    }
    if s.loc_light_dir >= 0 {
        // Directional light from upper-right-front
        gl::uniform3f(s.loc_light_dir, 0.5, 0.7, 1.0);
    }
    if s.loc_view_pos >= 0 {
        // Camera at (0, 0, 3) looking at origin
        gl::uniform3f(s.loc_view_pos, 0.0, 0.0, 3.0);
    }

    // Render
    gl::clear_color(0.08, 0.08, 0.12, 1.0);
    gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);
    gl::draw_arrays(gl::GL_TRIANGLES, 0, 36);

    // Copy to canvas
    let fb_ptr = gl::swap_buffers();
    if !fb_ptr.is_null() {
        let pixels = unsafe {
            core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize)
        };
        s.canvas.copy_pixels_from(pixels);
    }

    s.angle += 0.02;
    if s.angle > 2.0 * gl::PI { s.angle -= 2.0 * gl::PI; }
}

fn main() {
    anyos_std::println!("gldemo: starting");

    // Initialize anyui
    libanyui_client::init();
    let window = libanyui_client::Window::new("GL Demo", 100, 100, 420, 460);

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

    gl::gl_init(fb_w, fb_h);
    gl::viewport(0, 0, fb_w as i32, fb_h as i32);
    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::enable(gl::GL_CULL_FACE);
    gl::cull_face(gl::GL_BACK);
    gl::set_fxaa(true);

    // HW/SW renderer toggle (after gl_init so we can query HW availability)
    let hw_available = gl::has_hw_backend();
    let hw_label = libanyui_client::Label::new("HW");
    hw_label.set_position(10, 406);
    hw_label.set_text_color(0xFFCCCCCC);
    hw_label.set_font_size(13);
    window.add(&hw_label);

    let hw_toggle = libanyui_client::Toggle::new(hw_available);
    hw_toggle.set_position(40, 404);
    hw_toggle.on_checked_changed(|e| {
        gl::set_hw_backend(e.checked);
    });
    window.add(&hw_toggle);

    // FXAA toggle row
    let fxaa_label = libanyui_client::Label::new("FXAA");
    fxaa_label.set_position(110, 406);
    fxaa_label.set_text_color(0xFFCCCCCC);
    fxaa_label.set_font_size(13);
    window.add(&fxaa_label);

    let fxaa_toggle = libanyui_client::Toggle::new(true);
    fxaa_toggle.set_position(160, 404);
    fxaa_toggle.on_checked_changed(|e| {
        gl::set_fxaa(e.checked);
    });
    window.add(&fxaa_toggle);

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

    // Get attribute locations
    let loc_pos = gl::get_attrib_location(program, "aPosition");
    let loc_norm = gl::get_attrib_location(program, "aNormal");
    let loc_tex = gl::get_attrib_location(program, "aTexCoord");

    // Get uniform locations
    let loc_mvp = gl::get_uniform_location(program, "uMVP");
    let loc_model = gl::get_uniform_location(program, "uModel");
    let loc_light_dir = gl::get_uniform_location(program, "uLightDir");
    let loc_view_pos = gl::get_uniform_location(program, "uViewPos");
    let loc_texture = gl::get_uniform_location(program, "uTexture");

    // Upload vertex data
    let mut vbo = [0u32; 1];
    gl::gen_buffers(1, &mut vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &CUBE_VERTICES, gl::GL_STATIC_DRAW);

    // Configure vertex attributes (stride = 8 floats * 4 bytes = 32)
    if loc_pos >= 0 {
        gl::enable_vertex_attrib_array(loc_pos as u32);
        gl::vertex_attrib_pointer(loc_pos as u32, 3, gl::GL_FLOAT, false, 32, 0);
    }
    if loc_norm >= 0 {
        gl::enable_vertex_attrib_array(loc_norm as u32);
        gl::vertex_attrib_pointer(loc_norm as u32, 3, gl::GL_FLOAT, false, 32, 12);
    }
    if loc_tex >= 0 {
        gl::enable_vertex_attrib_array(loc_tex as u32);
        gl::vertex_attrib_pointer(loc_tex as u32, 2, gl::GL_FLOAT, false, 32, 24);
    }

    // Create procedural checkerboard texture (64x64 RGBA)
    let tex_size: u32 = 64;
    let mut tex_data = [0u8; 64 * 64 * 4];
    for y in 0..tex_size {
        for x in 0..tex_size {
            let checker = ((x / 8) + (y / 8)) % 2 == 0;
            let offset = ((y * tex_size + x) * 4) as usize;
            if checker {
                // Light warm color
                tex_data[offset] = 230;     // R
                tex_data[offset + 1] = 200; // G
                tex_data[offset + 2] = 160; // B
                tex_data[offset + 3] = 255; // A
            } else {
                // Dark cool color
                tex_data[offset] = 60;      // R
                tex_data[offset + 1] = 80;  // G
                tex_data[offset + 2] = 120; // B
                tex_data[offset + 3] = 255; // A
            }
        }
    }

    let mut tex = [0u32; 1];
    gl::gen_textures(1, &mut tex);
    gl::bind_texture(gl::GL_TEXTURE_2D, tex[0]);
    gl::tex_image_2d(
        gl::GL_TEXTURE_2D, 0, gl::GL_RGBA as i32,
        tex_size as i32, tex_size as i32, 0,
        gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, &tex_data,
    );
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MIN_FILTER, gl::GL_LINEAR as i32);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_LINEAR as i32);

    // Bind texture to unit 0 and set sampler uniform
    gl::active_texture(gl::GL_TEXTURE0);
    gl::bind_texture(gl::GL_TEXTURE_2D, tex[0]);
    if loc_texture >= 0 {
        gl::uniform1i(loc_texture, 0);
    }

    anyos_std::println!("gldemo: texture uploaded ({}x{} checkerboard)", tex_size, tex_size);

    // Store render state for timer callback
    unsafe {
        STATE = Some(RenderState {
            canvas,
            fb_w,
            fb_h,
            loc_mvp,
            loc_model,
            loc_light_dir,
            loc_view_pos,
            loc_texture,
            angle: 0.0,
        });
    }

    // Register 60fps animation timer
    libanyui_client::set_timer(16, || {
        render_frame();
    });

    // Run the anyui event loop
    libanyui_client::run();
}

// ── Matrix math ──────────────────────────────────────────────────────────────

/// Rotation around Y axis (column-major).
fn mat4_rotate_y(angle: f32) -> [f32; 16] {
    let c = gl::cos(angle);
    let s = gl::sin(angle);
    [
         c,  0.0,   s, 0.0,
        0.0, 1.0, 0.0, 0.0,
        -s,  0.0,   c, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Rotation around X axis (column-major).
fn mat4_rotate_x(angle: f32) -> [f32; 16] {
    let c = gl::cos(angle);
    let s = gl::sin(angle);
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
    let fov_rad = fov_deg * gl::PI / 180.0;
    let f = 1.0 / gl::tan(fov_rad * 0.5);
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

