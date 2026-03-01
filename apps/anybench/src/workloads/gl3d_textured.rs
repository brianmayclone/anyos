//! 3D Benchmark 2 — Textured Rendering.
//!
//! Renders textured quads via the libgl software rasteriser for
//! [`GL3D_TEST_MS`] milliseconds.  Measures texture-sampling and
//! fragment-processing throughput. Returns total textured triangles rendered.

use alloc::vec;
use alloc::vec::Vec;
use libanyui_client as anyui;
use libgl_client as gl;
use super::GL3D_TEST_MS;
use super::gl3d_common::*;

const NUM_QUADS: u32 = 200;

const VS_SRC: &str =
"attribute vec3 aPosition;
attribute vec2 aTexCoord;
uniform mat4 uMVP;
varying vec2 vTexCoord;
void main() {
    vTexCoord = aTexCoord;
    gl_Position = uMVP * vec4(aPosition, 1.0);
}";

const FS_SRC: &str =
"varying vec2 vTexCoord;
uniform sampler2D uTexture;
void main() {
    gl_FragColor = texture2D(uTexture, vTexCoord);
}";

/// Generates a 64×64 RGBA checkerboard texture.
fn generate_checkerboard() -> Vec<u8> {
    let size: u32 = 64;
    let check: u32 = 8;
    let mut data = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let dark = ((x / check) + (y / check)) % 2 == 0;
            let idx = ((y * size + x) * 4) as usize;
            if dark {
                data[idx]     = 50;
                data[idx + 1] = 50;
                data[idx + 2] = 60;
            } else {
                data[idx]     = 220;
                data[idx + 1] = 220;
                data[idx + 2] = 230;
            }
            data[idx + 3] = 255;
        }
    }
    data
}

/// Textured quad throughput benchmark.
pub fn bench_gl3d_textured(canvas: &anyui::Canvas) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if !ensure_gl_init(w, h) { return 0; }

    let (program, vs, fs) = match compile_program(VS_SRC, FS_SRC) {
        Some(p) => p,
        None => return 0,
    };
    gl::use_program(program);

    // Generate random quads: pos(3) + uv(2) per vertex, 6 verts per quad
    let mut verts: Vec<f32> = Vec::with_capacity((NUM_QUADS * 6 * 5) as usize);
    let mut seed: u32 = 77;
    for _ in 0..NUM_QUADS {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let cx = ((seed >> 16) as f32 / 32768.0) - 1.0;
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let cy = ((seed >> 16) as f32 / 32768.0) - 1.0;
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let z = -((seed >> 16) as f32 / 65536.0) - 0.5;
        let sz = 0.15;
        // Two triangles for a quad
        let quad: [[f32; 5]; 6] = [
            [cx - sz, cy - sz, z, 0.0, 0.0],
            [cx + sz, cy - sz, z, 1.0, 0.0],
            [cx + sz, cy + sz, z, 1.0, 1.0],
            [cx - sz, cy - sz, z, 0.0, 0.0],
            [cx + sz, cy + sz, z, 1.0, 1.0],
            [cx - sz, cy + sz, z, 0.0, 1.0],
        ];
        for v in &quad {
            verts.extend_from_slice(v);
        }
    }

    let mut vbo = [0u32; 1];
    gl::gen_buffers(1, &mut vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &verts, gl::GL_STATIC_DRAW);
    setup_pos_uv_attribs(program);

    // Texture
    let tex_data = generate_checkerboard();
    let mut tex = [0u32; 1];
    gl::gen_textures(1, &mut tex);
    gl::bind_texture(gl::GL_TEXTURE_2D, tex[0]);
    gl::tex_image_2d(
        gl::GL_TEXTURE_2D, 0, gl::GL_RGBA as i32, 64, 64, 0,
        gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, &tex_data,
    );
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST as i32);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MIN_FILTER, gl::GL_NEAREST as i32);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_WRAP_S, gl::GL_REPEAT as i32);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_WRAP_T, gl::GL_REPEAT as i32);

    let loc_mvp = gl::get_uniform_location(program, "uMVP");
    let loc_tex = gl::get_uniform_location(program, "uTexture");
    gl::uniform_matrix4fv(loc_mvp, false, &mat4_identity());
    gl::active_texture(gl::GL_TEXTURE0);
    gl::uniform1i(loc_tex, 0);

    gl::disable(gl::GL_CULL_FACE);
    gl::clear_color(0.06, 0.06, 0.10, 1.0);

    let vertex_count = (NUM_QUADS * 6) as i32;
    let tris_per_frame = NUM_QUADS * 2;
    let mut count: u64 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < GL3D_TEST_MS {
        gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);
        gl::draw_arrays(gl::GL_TRIANGLES, 0, vertex_count);
        gl::swap_buffers();
        count += tris_per_frame as u64;
    }

    copy_gl_to_canvas(canvas, w, h);
    gl::delete_buffers(&vbo);
    gl::delete_textures(&tex);
    cleanup_program(program, vs, fs);
    gl::enable(gl::GL_CULL_FACE);
    count
}
