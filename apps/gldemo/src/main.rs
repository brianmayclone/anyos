//! gldemo — OpenGL ES 2.0 lit & textured cube demo for anyOS.
//!
//! Renders a Phong-lit cube with a procedural checkerboard texture using
//! the libgl software rasterizer, displayed in an anyui Canvas window.

#![no_std]
#![no_main]
#![allow(unused, dead_code)]

anyos_std::entry!(main);

use libgl_client as gl;

// ── Vertex data: simple triangle, position only (3 floats per vertex) ────────
// 3 vertices in NDC space, stride = 12 bytes.

#[rustfmt::skip]
static TRI_VERTICES: [f32; 9] = [
    // Large triangle covering most of the viewport
    -0.8, -0.8, 0.0,   // bottom-left
     0.8, -0.8, 0.0,   // bottom-right
     0.0,  0.8, 0.0,   // top-center
];

// ── Shaders ──────────────────────────────────────────────────────────────────

/// Vertex shader: simple passthrough (no MVP, positions already in NDC).
static VS_SOURCE: &str =
"attribute vec3 aPosition;
void main() {
    gl_Position = vec4(aPosition, 1.0);
}
";

/// Fragment shader: solid bright red.
static FS_SOURCE: &str =
"void main() {
    gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);
}
";

// ── Render state ─────────────────────────────────────────────────────────────

struct RenderState {
    canvas: libanyui_client::Canvas,
    fb_w: u32,
    fb_h: u32,
}

static mut STATE: Option<RenderState> = None;

fn render_frame() {
    let s = unsafe { STATE.as_mut().unwrap() };

    // Simple: clear to dark blue, draw one bright red triangle
    gl::clear_color(0.1, 0.1, 0.3, 1.0);
    gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);
    gl::draw_arrays(gl::GL_TRIANGLES, 0, 3);

    // Copy to canvas
    let fb_ptr = gl::swap_buffers();
    if !fb_ptr.is_null() {
        let pixels = unsafe {
            core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize)
        };
        s.canvas.copy_pixels_from(pixels);
    }
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
    // No culling for simple triangle test
    gl::set_fxaa(false);

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

    // Compile simple shaders (passthrough VS + solid red FS)
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

    // Single attribute: aPosition (vec3, stride=12)
    let loc_pos = gl::get_attrib_location(program, "aPosition");
    anyos_std::println!("gldemo: aPosition loc={}", loc_pos);

    // Upload simple triangle vertex data
    let mut vbo = [0u32; 1];
    gl::gen_buffers(1, &mut vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &TRI_VERTICES, gl::GL_STATIC_DRAW);

    if loc_pos >= 0 {
        gl::enable_vertex_attrib_array(loc_pos as u32);
        gl::vertex_attrib_pointer(loc_pos as u32, 3, gl::GL_FLOAT, false, 12, 0);
    }

    anyos_std::println!("gldemo: simple triangle setup done (3 verts, stride=12)");

    // Store render state for timer callback
    unsafe {
        STATE = Some(RenderState {
            canvas,
            fb_w,
            fb_h,
        });
    }

    // Register 60fps animation timer
    libanyui_client::set_timer(16, || {
        render_frame();
    });

    // Run the anyui event loop
    libanyui_client::run();
}


