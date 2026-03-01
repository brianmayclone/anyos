//! 3D Benchmark 4 — Depth Testing.
//!
//! Renders many overlapping triangle layers at different Z depths with
//! `GL_DEPTH_TEST` enabled for [`GL3D_TEST_MS`] milliseconds.  Measures
//! depth-buffer read/write overhead under high overdraw. Returns total
//! depth-tested triangles rendered.

use alloc::vec::Vec;
use libanyui_client as anyui;
use libgl_client as gl;
use super::GL3D_TEST_MS;
use super::gl3d_common::*;

const NUM_LAYERS: u32 = 10;
const TRIS_PER_LAYER: u32 = 100;

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

/// Depth-testing benchmark (high overdraw with depth buffer).
pub fn bench_gl3d_depth(canvas: &anyui::Canvas) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if !ensure_gl_init(w, h) { return 0; }

    let (program, vs, fs) = match compile_program(VS_SRC, FS_SRC) {
        Some(p) => p,
        None => return 0,
    };
    gl::use_program(program);

    // Generate layers of overlapping triangles: pos(3) + color(3)
    let total_tris = NUM_LAYERS * TRIS_PER_LAYER;
    let mut verts: Vec<f32> = Vec::with_capacity((total_tris * 3 * 6) as usize);
    let mut seed: u32 = 123;
    for layer in 0..NUM_LAYERS {
        // Each layer at a different Z depth (-0.1 to -1.0)
        let z = -0.1 - (layer as f32 * 0.9 / NUM_LAYERS as f32);
        // Layer color gradient
        let cr = (layer as f32 + 1.0) / NUM_LAYERS as f32;
        let cg = 0.3 + 0.5 * (1.0 - cr);
        let cb = 0.5;
        for _ in 0..TRIS_PER_LAYER {
            for _ in 0..3 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let x = ((seed >> 16) as f32 / 32768.0) - 1.0;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let y = ((seed >> 16) as f32 / 32768.0) - 1.0;
                verts.push(x);
                verts.push(y);
                verts.push(z);
                verts.push(cr);
                verts.push(cg);
                verts.push(cb);
            }
        }
    }

    let mut vbo = [0u32; 1];
    gl::gen_buffers(1, &mut vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &verts, gl::GL_STATIC_DRAW);
    setup_pos_color_attribs(program);

    let loc_mvp = gl::get_uniform_location(program, "uMVP");
    gl::uniform_matrix4fv(loc_mvp, false, &mat4_identity());

    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::disable(gl::GL_CULL_FACE);
    gl::clear_color(0.05, 0.05, 0.08, 1.0);

    let vertex_count = (total_tris * 3) as i32;
    let mut count: u64 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < GL3D_TEST_MS {
        gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);
        gl::draw_arrays(gl::GL_TRIANGLES, 0, vertex_count);
        gl::swap_buffers();
        count += total_tris as u64;
    }

    copy_gl_to_canvas(canvas, w, h);
    gl::delete_buffers(&vbo);
    cleanup_program(program, vs, fs);
    gl::enable(gl::GL_CULL_FACE);
    count
}
