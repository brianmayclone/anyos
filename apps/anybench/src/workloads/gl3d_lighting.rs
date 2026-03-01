//! 3D Benchmark 3 — Phong Lighting.
//!
//! Renders a lit, rotating sphere with per-vertex Gouraud shading for
//! [`GL3D_TEST_MS`] milliseconds.  Stresses vertex transformation, lighting
//! computation, and the full rasterisation pipeline. Returns total lit
//! triangles rendered.

use libanyui_client as anyui;
use libgl_client as gl;
use super::GL3D_TEST_MS;
use super::gl3d_common::*;

const VS_SRC: &str =
"attribute vec3 aPosition;
attribute vec3 aNormal;
attribute vec2 aTexCoord;
uniform mat4 uMVP;
uniform mat4 uModel;
uniform vec3 uLightPos;
uniform vec3 uLightColor;
uniform vec3 uEyePos;
varying vec3 vLighting;
void main() {
    vec4 worldPos = uModel * vec4(aPosition, 1.0);
    vec3 N = normalize((uModel * vec4(aNormal, 0.0)).xyz);
    vec3 V = normalize(uEyePos - worldPos.xyz);
    vec3 L = normalize(uLightPos - worldPos.xyz);
    vec3 ambient = vec3(0.08, 0.08, 0.10);
    float diff = max(dot(N, L), 0.0);
    vec3 diffuse = uLightColor * diff;
    vec3 H = normalize(L + V);
    float spec = pow(max(dot(N, H), 0.0), 32.0);
    vec3 specular = uLightColor * spec;
    vLighting = ambient + diffuse + specular;
    gl_Position = uMVP * vec4(aPosition, 1.0);
}";

const FS_SRC: &str =
"varying vec3 vLighting;
uniform vec4 uMatColor;
void main() {
    gl_FragColor = vec4(vLighting * uMatColor.rgb, 1.0);
}";

/// Phong lighting benchmark (Gouraud-shaded sphere).
pub fn bench_gl3d_lighting(canvas: &anyui::Canvas) -> u64 {
    let w = canvas.get_stride();
    let h = canvas.get_height();
    if !ensure_gl_init(w, h) { return 0; }

    let (program, vs, fs) = match compile_program(VS_SRC, FS_SRC) {
        Some(p) => p,
        None => return 0,
    };
    gl::use_program(program);

    // Generate sphere geometry
    let (sphere_verts, sphere_indices) = generate_sphere(12, 20);
    let num_indices = sphere_indices.len() as i32;
    let tris_per_frame = num_indices / 3;

    let mut vbo = [0u32; 1];
    gl::gen_buffers(1, &mut vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &sphere_verts, gl::GL_STATIC_DRAW);

    let mut ebo = [0u32; 1];
    gl::gen_buffers(1, &mut ebo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, ebo[0]);
    gl::buffer_data_u16(gl::GL_ELEMENT_ARRAY_BUFFER, &sphere_indices, gl::GL_STATIC_DRAW);

    setup_vertex_attribs(program);

    let loc_mvp = gl::get_uniform_location(program, "uMVP");
    let loc_model = gl::get_uniform_location(program, "uModel");
    let loc_light_pos = gl::get_uniform_location(program, "uLightPos");
    let loc_light_color = gl::get_uniform_location(program, "uLightColor");
    let loc_eye_pos = gl::get_uniform_location(program, "uEyePos");
    let loc_mat_color = gl::get_uniform_location(program, "uMatColor");

    let eye = [0.0f32, 0.0, 3.5];
    let aspect = w as f32 / h as f32;
    let proj = mat4_perspective(0.9, aspect, 0.1, 50.0);
    let view = mat4_translate(-eye[0], -eye[1], -eye[2]);

    gl::uniform3f(loc_eye_pos, eye[0], eye[1], eye[2]);
    gl::uniform3f(loc_light_color, 1.0, 0.95, 0.85);
    gl::uniform4f(loc_mat_color, 0.9, 0.55, 0.2, 1.0);

    gl::enable(gl::GL_CULL_FACE);
    gl::clear_color(0.04, 0.04, 0.08, 1.0);

    let mut frame: u32 = 0;
    let mut count: u64 = 0;
    let start = anyos_std::sys::uptime_ms();
    while anyos_std::sys::uptime_ms().wrapping_sub(start) < GL3D_TEST_MS {
        let t = frame as f32 * 0.03;
        frame += 1;

        gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);

        // Animate light
        let lx = gl::sin(t * 0.7) * 3.0;
        let lz = gl::cos(t * 0.7) * 3.0;
        gl::uniform3f(loc_light_pos, lx, 2.0, lz);

        // Rotating sphere
        let model = mat4_rotate_y(t * 0.5);
        let mvp = mat4_mul(&proj, &mat4_mul(&view, &model));
        gl::uniform_matrix4fv(loc_mvp, false, &mvp);
        gl::uniform_matrix4fv(loc_model, false, &model);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, ebo[0]);
        setup_vertex_attribs(program);
        gl::draw_elements(gl::GL_TRIANGLES, num_indices, gl::GL_UNSIGNED_SHORT, 0);

        gl::swap_buffers();
        count += tris_per_frame as u64;
    }

    copy_gl_to_canvas(canvas, w, h);
    gl::delete_buffers(&vbo);
    gl::delete_buffers(&ebo);
    cleanup_program(program, vs, fs);
    count
}
