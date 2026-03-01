//! 3D Benchmark 5 — Draw Call Overhead.
//!
//! Renders many small objects (cubes) each with an individual draw call and
//! unique MVP uniform for [`GL3D_TEST_MS`] milliseconds.  Measures
//! per-draw-call overhead including uniform updates, buffer binding, and
//! pipeline dispatch. Returns total draw calls executed.

use libanyui_client as anyui;
use libgl_client as gl;
use super::GL3D_TEST_MS;
use super::gl3d_common::*;

const NUM_OBJECTS: u32 = 50;

const VS_SRC: &str =
"attribute vec3 aPosition;
attribute vec3 aNormal;
attribute vec2 aTexCoord;
uniform mat4 uMVP;
varying vec3 vColor;
void main() {
    vec3 lightDir = normalize(vec3(0.5, 1.0, 0.7));
    float diff = max(dot(aNormal, lightDir), 0.0);
    vColor = vec3(0.15) + vec3(0.85) * diff;
    gl_Position = uMVP * vec4(aPosition, 1.0);
}";

const FS_SRC: &str =
"varying vec3 vColor;
uniform vec4 uObjColor;
void main() {
    gl_FragColor = vec4(vColor * uObjColor.rgb, 1.0);
}";

/// Draw-call overhead benchmark (many small objects with unique transforms).
pub fn bench_gl3d_drawcalls(canvas: &anyui::Canvas) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if !ensure_gl_init(w, h) { return 0; }

    let (program, vs, fs) = match compile_program(VS_SRC, FS_SRC) {
        Some(p) => p,
        None => return 0,
    };
    gl::use_program(program);

    // Upload cube geometry once
    let (cube_verts, cube_indices) = generate_cube();
    let num_indices = cube_indices.len() as i32;

    let mut vbo = [0u32; 1];
    gl::gen_buffers(1, &mut vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &cube_verts, gl::GL_STATIC_DRAW);

    let mut ebo = [0u32; 1];
    gl::gen_buffers(1, &mut ebo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, ebo[0]);
    gl::buffer_data_u16(gl::GL_ELEMENT_ARRAY_BUFFER, &cube_indices, gl::GL_STATIC_DRAW);

    setup_vertex_attribs(program);

    let loc_mvp = gl::get_uniform_location(program, "uMVP");
    let loc_color = gl::get_uniform_location(program, "uObjColor");

    let aspect = w as f32 / h as f32;
    let proj = mat4_perspective(1.0, aspect, 0.1, 100.0);
    let view = mat4_translate(0.0, 0.0, -15.0);

    gl::enable(gl::GL_CULL_FACE);
    gl::enable(gl::GL_DEPTH_TEST);
    gl::clear_color(0.04, 0.04, 0.08, 1.0);

    // Pre-compute grid positions and colors for objects
    let grid_side = 8u32; // 8×8 grid (use first NUM_OBJECTS)
    let spacing = 2.8f32;
    let offset = (grid_side as f32 - 1.0) * spacing * 0.5;

    let mut frame: u32 = 0;
    let mut count: u64 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < GL3D_TEST_MS {
        let t = frame as f32 * 0.02;
        frame += 1;

        gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, ebo[0]);
        setup_vertex_attribs(program);

        let mut obj_idx: u32 = 0;
        for row in 0..grid_side {
            for col in 0..grid_side {
                if obj_idx >= NUM_OBJECTS { break; }
                let x = col as f32 * spacing - offset;
                let y = row as f32 * spacing - offset;

                // Each cube rotates at a slightly different speed
                let rot_speed = 0.5 + obj_idx as f32 * 0.03;
                let model = mat4_mul(
                    &mat4_translate(x, y, 0.0),
                    &mat4_mul(
                        &mat4_rotate_y(t * rot_speed),
                        &mat4_scale(0.4, 0.4, 0.4),
                    ),
                );
                let mvp = mat4_mul(&proj, &mat4_mul(&view, &model));
                gl::uniform_matrix4fv(loc_mvp, false, &mvp);

                // Color varies per object
                let cr = 0.3 + 0.7 * (obj_idx as f32 / NUM_OBJECTS as f32);
                let cg = 0.3 + 0.7 * (1.0 - obj_idx as f32 / NUM_OBJECTS as f32);
                let cb = 0.6;
                gl::uniform4f(loc_color, cr, cg, cb, 1.0);

                gl::draw_elements(gl::GL_TRIANGLES, num_indices, gl::GL_UNSIGNED_SHORT, 0);
                count += 1;
                obj_idx += 1;
            }
            if obj_idx >= NUM_OBJECTS { break; }
        }

        gl::swap_buffers();
    }

    copy_gl_to_canvas(canvas, w, h);
    gl::delete_buffers(&vbo);
    gl::delete_buffers(&ebo);
    cleanup_program(program, vs, fs);
    count
}
