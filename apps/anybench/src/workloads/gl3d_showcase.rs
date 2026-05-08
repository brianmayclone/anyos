//! 3D room stress scene for the anyBench 3D phase.
//!
//! Inspired by GL Demo: a lit room with physics-driven balls. The scene keeps
//! adding balls until measured throughput drops below 1 FPS, then closes.

use super::gl3d_common::*;
use alloc::format;
use alloc::vec::Vec;
use libanyui_client as anyui;
use libgl_client as gl;

const W: u32 = 720;
const H: u32 = 420;
const ROOM_HALF: f32 = 4.2;
const FLOOR_Y: f32 = -1.7;
const MAX_BALLS: usize = 640;
const ADD_INTERVAL_MS: u32 = 420;
const MIN_RUNTIME_MS: u32 = 1800;

const VS_SRC: &str = "attribute vec3 aPosition;
attribute vec3 aNormal;
attribute vec2 aTexCoord;
uniform mat4 uMVP;
uniform mat4 uModel;
uniform vec3 uLightPos0;
uniform vec3 uLightPos1;
uniform vec3 uEyePos;
varying vec3 vLighting;
varying vec3 vWorldNormal;
void main() {
    vec4 worldPos = uModel * vec4(aPosition, 1.0);
    vec3 N = normalize((uModel * vec4(aNormal, 0.0)).xyz);
    vec3 V = normalize(uEyePos - worldPos.xyz);
    vec3 L0 = normalize(uLightPos0 - worldPos.xyz);
    vec3 L1 = normalize(uLightPos1 - worldPos.xyz);
    float diff0 = max(dot(N, L0), 0.0);
    float diff1 = max(dot(N, L1), 0.0);
    vec3 H0 = normalize(L0 + V);
    float spec0 = pow(max(dot(N, H0), 0.0), 28.0);
    float rim = pow(1.0 - max(dot(N, V), 0.0), 2.0);
    vLighting = vec3(0.10, 0.11, 0.14)
        + diff0 * vec3(0.92, 0.88, 0.74)
        + diff1 * vec3(0.20, 0.34, 0.52)
        + spec0 * vec3(0.9, 0.82, 0.62)
        + rim * vec3(0.08, 0.16, 0.24);
    vWorldNormal = N;
    gl_Position = uMVP * vec4(aPosition, 1.0);
}";

const FS_SRC: &str = "varying vec3 vLighting;
varying vec3 vWorldNormal;
uniform vec4 uMatColor;
void main() {
    vec3 subtle = vec3(0.05 * abs(vWorldNormal.y), 0.035 * abs(vWorldNormal.x), 0.04 * abs(vWorldNormal.z));
    gl_FragColor = vec4((uMatColor.rgb + subtle) * vLighting, uMatColor.a);
}";

#[derive(Clone, Copy)]
struct Ball {
    body: u32,
    radius: f32,
    color: [f32; 4],
}

/// Opens a temporary window, runs the ball-room stress scene, and closes it.
pub fn run_gl3d_showcase_window() {
    let win = anyui::Window::new("anyBench 3D Ball Room", -1, -1, W, H);
    win.set_color(0xFF0D1017);
    win.on_close(|_| {});

    let canvas = anyui::Canvas::new(W, H);
    canvas.set_position(0, 0);
    canvas.set_size(W, H);
    canvas.clear(0xFF0D1017);
    win.add(&canvas);

    let label = anyui::Label::new("3D stress: spawning balls...");
    label.set_position(12, 8);
    label.set_size(420, 22);
    label.set_font_size(13);
    label.set_text_color(0xFFE8EEF7);
    win.add(&label);
    win.set_visible(true);

    run_scene(&canvas, &label);
    win.destroy();
}

fn run_scene(canvas: &anyui::Canvas, label: &anyui::Label) {
    if !ensure_gl_init(W, H) {
        label.set_text("3D stress: libgl unavailable");
        anyos_std::process::sleep(900);
        return;
    }

    let (program, vs, fs) = match compile_program(VS_SRC, FS_SRC) {
        Some(p) => p,
        None => {
            label.set_text("3D stress: shader compile failed");
            anyos_std::process::sleep(900);
            return;
        }
    };
    gl::use_program(program);

    let (sphere_verts, sphere_indices) = generate_sphere(18, 28);
    let sphere_index_count = sphere_indices.len() as i32;
    let (cube_verts, cube_indices) = generate_cube();
    let cube_index_count = cube_indices.len() as i32;

    let mut sphere_vbo = [0u32; 1];
    let mut sphere_ebo = [0u32; 1];
    let mut cube_vbo = [0u32; 1];
    let mut cube_ebo = [0u32; 1];

    gl::gen_buffers(1, &mut sphere_vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, sphere_vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &sphere_verts, gl::GL_STATIC_DRAW);
    gl::gen_buffers(1, &mut sphere_ebo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, sphere_ebo[0]);
    gl::buffer_data_u16(
        gl::GL_ELEMENT_ARRAY_BUFFER,
        &sphere_indices,
        gl::GL_STATIC_DRAW,
    );

    gl::gen_buffers(1, &mut cube_vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, cube_vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &cube_verts, gl::GL_STATIC_DRAW);
    gl::gen_buffers(1, &mut cube_ebo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, cube_ebo[0]);
    gl::buffer_data_u16(
        gl::GL_ELEMENT_ARRAY_BUFFER,
        &cube_indices,
        gl::GL_STATIC_DRAW,
    );

    let loc_mvp = gl::get_uniform_location(program, "uMVP");
    let loc_model = gl::get_uniform_location(program, "uModel");
    let loc_light0 = gl::get_uniform_location(program, "uLightPos0");
    let loc_light1 = gl::get_uniform_location(program, "uLightPos1");
    let loc_eye = gl::get_uniform_location(program, "uEyePos");
    let loc_color = gl::get_uniform_location(program, "uMatColor");

    gl::physics_create_world();
    gl::physics_set_gravity(0.0, -9.81, 0.0);
    let floor = gl::physics_add_plane(0.0, 1.0, 0.0, FLOOR_Y);
    gl::physics_set_restitution(floor, 0.72);
    add_room_planes();

    let mut balls: Vec<Ball> = Vec::new();
    let mut seed = 0xB01D_5EEDu32;
    for _ in 0..6 {
        add_ball(&mut balls, &mut seed);
    }

    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::enable(gl::GL_CULL_FACE);
    gl::cull_face(gl::GL_BACK);
    gl::clear_color(0.028, 0.032, 0.046, 1.0);

    let eye = [0.0f32, 3.1, 8.4];
    let view = mat4_look_at(&eye, &[0.0, 0.3, 0.0], &[0.0, 1.0, 0.0]);
    let proj = mat4_perspective(0.82, W as f32 / H as f32, 0.1, 80.0);
    let vp = mat4_mul(&proj, &view);
    gl::uniform3f(loc_eye, eye[0], eye[1], eye[2]);

    let start = anyos_std::sys::uptime_ms();
    let mut last_add = start;
    let mut fps_window = start;
    let mut fps_frames = 0u32;
    let mut fps10 = 600u32;
    let mut frames = 0u32;
    let mut done_below_one = false;

    while balls.len() < MAX_BALLS && !done_below_one {
        let now = anyos_std::sys::uptime_ms();
        let elapsed_total = now.wrapping_sub(start);

        if now.wrapping_sub(last_add) >= ADD_INTERVAL_MS {
            let batch = spawn_batch_size(balls.len());
            for _ in 0..batch {
                if balls.len() >= MAX_BALLS {
                    break;
                }
                add_ball(&mut balls, &mut seed);
            }
            last_add = now;
        }

        gl::physics_step(0.016);
        render_frame(
            canvas,
            program,
            sphere_vbo[0],
            sphere_ebo[0],
            sphere_index_count,
            cube_vbo[0],
            cube_ebo[0],
            cube_index_count,
            loc_mvp,
            loc_model,
            loc_light0,
            loc_light1,
            loc_color,
            &vp,
            &balls,
            frames,
        );
        frames = frames.wrapping_add(1);
        fps_frames = fps_frames.wrapping_add(1);

        let fps_elapsed = now.wrapping_sub(fps_window);
        if fps_elapsed >= 1000 {
            fps10 = fps_frames.saturating_mul(10_000) / fps_elapsed.max(1);
            label.set_text(&format!(
                "3D stress: {} balls  {}.{} FPS",
                balls.len(),
                fps10 / 10,
                fps10 % 10
            ));
            done_below_one = elapsed_total >= MIN_RUNTIME_MS && fps10 < 10;
            fps_frames = 0;
            fps_window = now;
        }

        if fps10 >= 10 {
            anyos_std::process::sleep(1);
        }
    }

    label.set_text(&format!(
        "3D stress complete: {} balls at {}.{} FPS",
        balls.len(),
        fps10 / 10,
        fps10 % 10
    ));
    anyos_std::process::sleep(850);

    gl::delete_buffers(&sphere_vbo);
    gl::delete_buffers(&sphere_ebo);
    gl::delete_buffers(&cube_vbo);
    gl::delete_buffers(&cube_ebo);
    cleanup_program(program, vs, fs);
}

fn add_room_planes() {
    let left = gl::physics_add_plane(1.0, 0.0, 0.0, -ROOM_HALF);
    let right = gl::physics_add_plane(-1.0, 0.0, 0.0, -ROOM_HALF);
    let front = gl::physics_add_plane(0.0, 0.0, -1.0, -ROOM_HALF);
    let back = gl::physics_add_plane(0.0, 0.0, 1.0, -ROOM_HALF);
    let ceil = gl::physics_add_plane(0.0, -1.0, 0.0, -(FLOOR_Y + ROOM_HALF * 1.45));
    for id in [left, right, front, back, ceil] {
        gl::physics_set_restitution(id, 0.86);
    }
}

fn add_ball(balls: &mut Vec<Ball>, seed: &mut u32) {
    let radius = 0.22 + rand01(seed) * 0.16;
    let x = -2.6 + rand01(seed) * 5.2;
    let y = 2.1 + rand01(seed) * 2.8;
    let z = -2.7 + rand01(seed) * 5.4;
    let body = gl::physics_add_sphere(0.8 + radius * 2.2, radius, x, y, z);
    gl::physics_set_restitution(body, 0.82 + rand01(seed) * 0.13);
    gl::physics_set_angular_damping(body, 0.08);
    gl::physics_set_linear_damping(body, 0.018);
    gl::physics_set_soft_body(body, 0.20 + rand01(seed) * 0.20, 9.0, 0.18);
    gl::physics_set_velocity(
        body,
        -2.2 + rand01(seed) * 4.4,
        -0.3 + rand01(seed) * 1.8,
        -2.2 + rand01(seed) * 4.4,
    );
    gl::physics_set_angular_velocity(
        body,
        -5.0 + rand01(seed) * 10.0,
        -5.0 + rand01(seed) * 10.0,
        -5.0 + rand01(seed) * 10.0,
    );
    balls.push(Ball {
        body,
        radius,
        color: ball_color(balls.len() as u32),
    });
}

fn render_frame(
    canvas: &anyui::Canvas,
    program: u32,
    sphere_vbo: u32,
    sphere_ebo: u32,
    sphere_index_count: i32,
    cube_vbo: u32,
    cube_ebo: u32,
    cube_index_count: i32,
    loc_mvp: i32,
    loc_model: i32,
    loc_light0: i32,
    loc_light1: i32,
    loc_color: i32,
    vp: &[f32; 16],
    balls: &[Ball],
    frame: u32,
) {
    let t = frame as f32 * 0.025;
    gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);
    gl::use_program(program);
    gl::uniform3f(
        loc_light0,
        gl::sin(t * 0.8) * 3.1,
        3.7,
        gl::cos(t * 0.6) * 2.8,
    );
    gl::uniform3f(loc_light1, -3.4, 1.9 + gl::sin(t) * 0.4, 3.2);

    gl::bind_buffer(gl::GL_ARRAY_BUFFER, cube_vbo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, cube_ebo);
    setup_vertex_attribs(program);
    gl::disable(gl::GL_CULL_FACE);
    draw_room(loc_mvp, loc_model, loc_color, vp, cube_index_count);
    gl::enable(gl::GL_CULL_FACE);

    gl::bind_buffer(gl::GL_ARRAY_BUFFER, sphere_vbo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, sphere_ebo);
    setup_vertex_attribs(program);
    for ball in balls {
        let (px, py, pz) = gl::physics_get_position(ball.body);
        let (qw, qx, qy, qz) = gl::physics_get_orientation(ball.body);
        let rot = quat_to_mat4(qw, qx, qy, qz);
        let (sx, sy, sz) = gl::physics_get_deformation_scale(ball.body);
        let scale = mat4_scale(ball.radius * sx, ball.radius * sy, ball.radius * sz);
        let model = mat4_mul(&mat4_translate(px, py, pz), &mat4_mul(&rot, &scale));
        draw_object(
            loc_mvp,
            loc_model,
            loc_color,
            vp,
            &model,
            ball.color,
            sphere_index_count,
        );
    }

    copy_gl_to_canvas(canvas, W, H);
}

fn draw_room(loc_mvp: i32, loc_model: i32, loc_color: i32, vp: &[f32; 16], index_count: i32) {
    let floor = mat4_mul(
        &mat4_translate(0.0, FLOOR_Y - 0.04, 0.0),
        &mat4_scale(ROOM_HALF, 0.04, ROOM_HALF),
    );
    draw_object(
        loc_mvp,
        loc_model,
        loc_color,
        vp,
        &floor,
        [0.46, 0.48, 0.55, 1.0],
        index_count,
    );

    let back = mat4_mul(
        &mat4_translate(0.0, FLOOR_Y + ROOM_HALF * 0.65, -ROOM_HALF - 0.04),
        &mat4_scale(ROOM_HALF, ROOM_HALF * 0.70, 0.04),
    );
    draw_object(
        loc_mvp,
        loc_model,
        loc_color,
        vp,
        &back,
        [0.23, 0.28, 0.38, 1.0],
        index_count,
    );

    let front = mat4_mul(
        &mat4_translate(0.0, FLOOR_Y + ROOM_HALF * 0.65, ROOM_HALF + 0.04),
        &mat4_scale(ROOM_HALF, ROOM_HALF * 0.70, 0.04),
    );
    draw_object(
        loc_mvp,
        loc_model,
        loc_color,
        vp,
        &front,
        [0.20, 0.24, 0.32, 1.0],
        index_count,
    );

    for (x, color) in [
        (-ROOM_HALF - 0.04, [0.30, 0.26, 0.34, 1.0]),
        (ROOM_HALF + 0.04, [0.24, 0.31, 0.34, 1.0]),
    ] {
        let wall = mat4_mul(
            &mat4_translate(x, FLOOR_Y + ROOM_HALF * 0.65, 0.0),
            &mat4_scale(0.04, ROOM_HALF * 0.70, ROOM_HALF),
        );
        draw_object(loc_mvp, loc_model, loc_color, vp, &wall, color, index_count);
    }

    let ceiling = mat4_mul(
        &mat4_translate(0.0, FLOOR_Y + ROOM_HALF * 1.35, 0.0),
        &mat4_scale(ROOM_HALF, 0.035, ROOM_HALF),
    );
    draw_object(
        loc_mvp,
        loc_model,
        loc_color,
        vp,
        &ceiling,
        [0.16, 0.17, 0.22, 1.0],
        index_count,
    );
}

fn draw_object(
    loc_mvp: i32,
    loc_model: i32,
    loc_color: i32,
    vp: &[f32; 16],
    model: &[f32; 16],
    color: [f32; 4],
    index_count: i32,
) {
    let mvp = mat4_mul(vp, model);
    gl::uniform_matrix4fv(loc_mvp, false, &mvp);
    gl::uniform_matrix4fv(loc_model, false, model);
    gl::uniform4f(loc_color, color[0], color[1], color[2], color[3]);
    gl::draw_elements(gl::GL_TRIANGLES, index_count, gl::GL_UNSIGNED_SHORT, 0);
}

fn spawn_batch_size(current: usize) -> usize {
    if current < 24 {
        2
    } else if current < 96 {
        4
    } else if current < 256 {
        8
    } else {
        16
    }
}

fn rand01(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    ((*seed >> 8) & 0xFFFF) as f32 / 65535.0
}

fn ball_color(index: u32) -> [f32; 4] {
    match index % 8 {
        0 => [0.95, 0.18, 0.16, 1.0],
        1 => [0.98, 0.95, 0.86, 1.0],
        2 => [0.22, 0.58, 0.96, 1.0],
        3 => [0.25, 0.86, 0.48, 1.0],
        4 => [0.96, 0.68, 0.22, 1.0],
        5 => [0.74, 0.42, 0.96, 1.0],
        6 => [0.90, 0.34, 0.68, 1.0],
        _ => [0.42, 0.92, 0.90, 1.0],
    }
}

fn quat_to_mat4(w: f32, x: f32, y: f32, z: f32) -> [f32; 16] {
    let x2 = x + x;
    let y2 = y + y;
    let z2 = z + z;
    let xx = x * x2;
    let xy = x * y2;
    let xz = x * z2;
    let yy = y * y2;
    let yz = y * z2;
    let zz = z * z2;
    let wx = w * x2;
    let wy = w * y2;
    let wz = w * z2;
    [
        1.0 - yy - zz,
        xy + wz,
        xz - wy,
        0.0,
        xy - wz,
        1.0 - xx - zz,
        yz + wx,
        0.0,
        xz + wy,
        yz - wx,
        1.0 - xx - yy,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn mat4_look_at(eye: &[f32; 3], target: &[f32; 3], up: &[f32; 3]) -> [f32; 16] {
    let fx = target[0] - eye[0];
    let fy = target[1] - eye[1];
    let fz = target[2] - eye[2];
    let flen = gl::sqrt(fx * fx + fy * fy + fz * fz).max(0.0001);
    let fx = fx / flen;
    let fy = fy / flen;
    let fz = fz / flen;

    let sx = fy * up[2] - fz * up[1];
    let sy = fz * up[0] - fx * up[2];
    let sz = fx * up[1] - fy * up[0];
    let slen = gl::sqrt(sx * sx + sy * sy + sz * sz).max(0.0001);
    let sx = sx / slen;
    let sy = sy / slen;
    let sz = sz / slen;

    let ux = sy * fz - sz * fy;
    let uy = sz * fx - sx * fz;
    let uz = sx * fy - sy * fx;

    [
        sx,
        ux,
        -fx,
        0.0,
        sy,
        uy,
        -fy,
        0.0,
        sz,
        uz,
        -fz,
        0.0,
        -(sx * eye[0] + sy * eye[1] + sz * eye[2]),
        -(ux * eye[0] + uy * eye[1] + uz * eye[2]),
        fx * eye[0] + fy * eye[1] + fz * eye[2],
        1.0,
    ]
}
