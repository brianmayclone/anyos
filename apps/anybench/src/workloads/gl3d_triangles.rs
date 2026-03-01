//! 3D Benchmark 1 — Triangle Throughput.
//!
//! Renders random flat-colored triangles via the libgl software rasterizer
//! for [`GL3D_TEST_MS`] milliseconds.  Measures raw vertex-processing and
//! rasterisation speed. Returns total triangles rendered.

use alloc::vec::Vec;
use libanyui_client as anyui;
use libgl_client as gl;
use super::GL3D_TEST_MS;
use super::gl3d_common::*;

const NUM_TRIANGLES: u32 = 500;

const VS_SRC: &str =
"attribute vec3 aPosition;
attribute vec3 aColor;
uniform mat4 uMVP;
varying vec3 vColor;
void main() {
    vColor = aColor;
    gl_Position = uMVP * vec4(aPosition, 1.0);
}";

const FS_SRC: &str =
"varying vec3 vColor;
void main() {
    gl_FragColor = vec4(vColor, 1.0);
}";

/// Triangle throughput benchmark (flat-colored triangles, no textures).
pub fn bench_gl3d_triangles(canvas: &anyui::Canvas) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if !ensure_gl_init(w, h) { return 0; }

    let (program, vs, fs) = match compile_program(VS_SRC, FS_SRC) {
        Some(p) => p,
        None => return 0,
    };
    gl::use_program(program);

    // Generate random triangles: pos(3) + color(3) per vertex
    let mut verts: Vec<f32> = Vec::with_capacity((NUM_TRIANGLES * 3 * 6) as usize);
    let mut seed: u32 = 42;
    for _ in 0..NUM_TRIANGLES {
        // Random color for this triangle
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let cr = ((seed >> 16) & 0xFF) as f32 / 255.0;
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let cg = ((seed >> 16) & 0xFF) as f32 / 255.0;
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let cb = ((seed >> 16) & 0xFF) as f32 / 255.0;

        for _ in 0..3 {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let x = ((seed >> 16) as f32 / 32768.0) - 1.0; // [-1, 1]
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let y = ((seed >> 16) as f32 / 32768.0) - 1.0;
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let z = -((seed >> 16) as f32 / 65536.0) - 0.5; // [-0.5, -1.5]
            verts.push(x);
            verts.push(y);
            verts.push(z);
            verts.push(cr);
            verts.push(cg);
            verts.push(cb);
        }
    }

    let mut vbo = [0u32; 1];
    gl::gen_buffers(1, &mut vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &verts, gl::GL_STATIC_DRAW);
    setup_pos_color_attribs(program);

    // Ortho-like MVP (identity — vertices already in clip space)
    let mvp = mat4_identity();
    let loc_mvp = gl::get_uniform_location(program, "uMVP");
    gl::uniform_matrix4fv(loc_mvp, false, &mvp);

    gl::disable(gl::GL_CULL_FACE);
    gl::clear_color(0.05, 0.05, 0.08, 1.0);

    let vertex_count = (NUM_TRIANGLES * 3) as i32;
    let mut count: u64 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < GL3D_TEST_MS {
        gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);
        gl::draw_arrays(gl::GL_TRIANGLES, 0, vertex_count);
        gl::swap_buffers();
        count += NUM_TRIANGLES as u64;
    }

    copy_gl_to_canvas(canvas, w, h);
    gl::delete_buffers(&vbo);
    cleanup_program(program, vs, fs);
    gl::enable(gl::GL_CULL_FACE);
    count
}
