extern crate alloc;

use alloc::collections::BTreeMap;
use libgl_client as gl;

use crate::mesh::{ChunkMesh, FLOATS_PER_VERTEX};

// ---------------------------------------------------------------------------
// Type alias
// ---------------------------------------------------------------------------
type Mat4 = [f32; 16];

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

const VS_BLOCK: &str = r#"
attribute vec3 aPosition;
attribute vec2 aTexCoord;
attribute vec3 aNormal;
attribute float aLight;

uniform mat4 uMVP;
uniform vec3 uSunDir;
uniform float uAmbient;

varying vec2 vTexCoord;
varying float vLighting;
varying float vDist;

void main() {
    gl_Position = uMVP * vec4(aPosition, 1.0);
    vTexCoord = aTexCoord;
    float sun = max(dot(aNormal, uSunDir), 0.0);
    vLighting = aLight * (uAmbient + (1.0 - uAmbient) * sun);
    vDist = gl_Position.w;
}
"#;

const FS_BLOCK: &str = r#"
precision mediump float;

varying vec2 vTexCoord;
varying float vLighting;
varying float vDist;

uniform sampler2D uTexture;
uniform vec3 uFogColor;
uniform float uFogStart;
uniform float uFogEnd;

void main() {
    vec4 tex = texture2D(uTexture, vTexCoord);
    if (tex.a < 0.1) discard;
    vec3 color = tex.rgb * vLighting;
    float fog = smoothstep(uFogStart, uFogEnd, vDist);
    color = mix(color, uFogColor, fog);
    gl_FragColor = vec4(color, tex.a);
}
"#;

const VS_SKY: &str = r#"
attribute vec2 aPosition;
varying vec2 vPos;

void main() {
    vPos = aPosition;
    gl_Position = vec4(aPosition, 0.999, 1.0);
}
"#;

const FS_SKY: &str = r#"
precision mediump float;

varying vec2 vPos;

uniform vec3 uSkyTop;
uniform vec3 uSkyHorizon;
uniform vec3 uSunDir;

void main() {
    float t = clamp(vPos.y * 0.5 + 0.5, 0.0, 1.0);
    vec3 color = mix(uSkyHorizon, uSkyTop, t);

    // Sun glow
    vec3 dir = normalize(vec3(vPos, 1.0));
    float sun = pow(max(dot(dir, uSunDir), 0.0), 64.0);
    color += vec3(1.0, 0.9, 0.7) * sun;

    gl_FragColor = vec4(color, 1.0);
}
"#;

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

pub struct Renderer {
    pub block_program: u32,
    pub sky_program: u32,
    pub atlas_tex: u32,
    pub sky_vbo: u32,
    // Block shader uniform/attrib locations
    pub u_mvp: i32,
    pub u_sun_dir: i32,
    pub u_ambient: i32,
    pub u_fog_color: i32,
    pub u_fog_start: i32,
    pub u_fog_end: i32,
    pub u_texture: i32,
    pub a_position: i32,
    pub a_texcoord: i32,
    pub a_normal: i32,
    pub a_light: i32,
    // Sky shader locations
    pub u_sky_top: i32,
    pub u_sky_horizon: i32,
    pub u_sky_sun_dir: i32,
    pub a_sky_pos: i32,
    // Chunk VBOs: (vbo_id, vertex_count)
    pub chunk_vbos: BTreeMap<(i32, i32), (u32, u32)>,
    // Camera
    pub yaw: f32,
    pub pitch: f32,
    // Fog
    pub fog_distance: f32,
    pub target_fog_distance: f32,
    // Day/night
    pub time_of_day: f32,
}

impl Renderer {
    pub fn init(atlas_data: &[u8], atlas_w: u32, atlas_h: u32) -> Self {
        // -- Block shader program --
        let block_program = compile_program(VS_BLOCK, FS_BLOCK);

        // -- Sky shader program --
        let sky_program = compile_program(VS_SKY, FS_SKY);

        // -- Atlas texture --
        let mut tex_ids = [0u32; 1];
        gl::gen_textures(1, &mut tex_ids);
        let atlas_tex = tex_ids[0];
        gl::bind_texture(gl::GL_TEXTURE_2D, atlas_tex);
        gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MIN_FILTER, gl::GL_NEAREST as i32);
        gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST as i32);
        gl::tex_image_2d(
            gl::GL_TEXTURE_2D,
            0,
            gl::GL_RGBA as i32,
            atlas_w as i32,
            atlas_h as i32,
            0,
            gl::GL_RGBA,
            gl::GL_UNSIGNED_BYTE,
            atlas_data,
        );

        // -- Sky quad VBO --
        let sky_verts: [f32; 12] = [
            -1.0, -1.0,
             1.0, -1.0,
             1.0,  1.0,
            -1.0, -1.0,
             1.0,  1.0,
            -1.0,  1.0,
        ];
        let mut vbo_ids = [0u32; 1];
        gl::gen_buffers(1, &mut vbo_ids);
        let sky_vbo = vbo_ids[0];
        gl::bind_buffer(gl::GL_ARRAY_BUFFER, sky_vbo);
        gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &sky_verts, gl::GL_STATIC_DRAW);

        // -- Query locations --
        let u_mvp = gl::get_uniform_location(block_program, "uMVP");
        let u_sun_dir = gl::get_uniform_location(block_program, "uSunDir");
        let u_ambient = gl::get_uniform_location(block_program, "uAmbient");
        let u_fog_color = gl::get_uniform_location(block_program, "uFogColor");
        let u_fog_start = gl::get_uniform_location(block_program, "uFogStart");
        let u_fog_end = gl::get_uniform_location(block_program, "uFogEnd");
        let u_texture = gl::get_uniform_location(block_program, "uTexture");

        let a_position = gl::get_attrib_location(block_program, "aPosition");
        let a_texcoord = gl::get_attrib_location(block_program, "aTexCoord");
        let a_normal = gl::get_attrib_location(block_program, "aNormal");
        let a_light = gl::get_attrib_location(block_program, "aLight");

        let u_sky_top = gl::get_uniform_location(sky_program, "uSkyTop");
        let u_sky_horizon = gl::get_uniform_location(sky_program, "uSkyHorizon");
        let u_sky_sun_dir = gl::get_uniform_location(sky_program, "uSunDir");
        let a_sky_pos = gl::get_attrib_location(sky_program, "aPosition");

        Renderer {
            block_program,
            sky_program,
            atlas_tex,
            sky_vbo,
            u_mvp,
            u_sun_dir,
            u_ambient,
            u_fog_color,
            u_fog_start,
            u_fog_end,
            u_texture,
            a_position,
            a_texcoord,
            a_normal,
            a_light,
            u_sky_top,
            u_sky_horizon,
            u_sky_sun_dir,
            a_sky_pos,
            chunk_vbos: BTreeMap::new(),
            yaw: 0.0,
            pitch: 0.0,
            fog_distance: 96.0,
            target_fog_distance: 96.0,
            time_of_day: 0.25,
        }
    }

    pub fn upload_chunk(&mut self, cx: i32, cz: i32, mesh: &ChunkMesh) {
        let key = (cx, cz);

        if mesh.vertices.is_empty() {
            // Remove existing VBO if any
            if let Some((vbo, _)) = self.chunk_vbos.remove(&key) {
                gl::delete_buffers(&[vbo]);
            }
            return;
        }

        let vertex_count = (mesh.vertices.len() / FLOATS_PER_VERTEX) as u32;

        let vbo = if let Some((existing_vbo, _)) = self.chunk_vbos.get(&key) {
            *existing_vbo
        } else {
            let mut ids = [0u32; 1];
            gl::gen_buffers(1, &mut ids);
            ids[0]
        };

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo);
        gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &mesh.vertices, gl::GL_STATIC_DRAW);

        self.chunk_vbos.insert(key, (vbo, vertex_count));
    }

    pub fn render(&mut self, cam_x: f32, cam_y: f32, cam_z: f32, width: u32, height: u32) {
        // -- Time-of-day calculations --
        let sun_angle = self.time_of_day * 2.0 * gl::PI;
        let sun_y = gl::cos(sun_angle);
        let sun_z = gl::sin(sun_angle);
        let sun_dir = [0.0f32, sun_y, sun_z];

        // Day factor: 1.0 at noon, 0.0 at night
        let day_factor = (sun_y * 2.0).clamp(0.0, 1.0);

        let sky_top = [
            lerp(0.01, 0.3, day_factor),
            lerp(0.01, 0.5, day_factor),
            lerp(0.05, 0.9, day_factor),
        ];
        let sky_horizon = [
            lerp(0.02, 0.6, day_factor),
            lerp(0.02, 0.7, day_factor),
            lerp(0.05, 0.9, day_factor),
        ];
        let ambient = lerp(0.15, 0.4, day_factor);

        // Smooth fog distance
        let fog_speed = 0.02;
        self.fog_distance += (self.target_fog_distance - self.fog_distance) * fog_speed;

        let fog_start = self.fog_distance * 0.6;
        let fog_end = self.fog_distance;

        // -- Clear --
        gl::viewport(0, 0, width as i32, height as i32);
        gl::clear_color(sky_horizon[0], sky_horizon[1], sky_horizon[2], 1.0);
        gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);

        // -- Sky pass --
        gl::disable(gl::GL_DEPTH_TEST);
        gl::use_program(self.sky_program);

        gl::uniform3f(self.u_sky_top, sky_top[0], sky_top[1], sky_top[2]);
        gl::uniform3f(self.u_sky_horizon, sky_horizon[0], sky_horizon[1], sky_horizon[2]);
        gl::uniform3f(self.u_sky_sun_dir, sun_dir[0], sun_dir[1], sun_dir[2]);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, self.sky_vbo);
        gl::enable_vertex_attrib_array(self.a_sky_pos as u32);
        gl::vertex_attrib_pointer(self.a_sky_pos as u32, 2, gl::GL_FLOAT, false, 8, 0);
        gl::draw_arrays(gl::GL_TRIANGLES, 0, 6);
        gl::disable_vertex_attrib_array(self.a_sky_pos as u32);

        // -- Block pass --
        gl::enable(gl::GL_DEPTH_TEST);
        gl::depth_func(gl::GL_LESS);
        gl::enable(gl::GL_BLEND);
        gl::blend_func(gl::GL_SRC_ALPHA, gl::GL_ONE_MINUS_SRC_ALPHA);

        gl::use_program(self.block_program);

        // Build matrices
        let aspect = width as f32 / height as f32;
        let proj = perspective(70.0, aspect, 0.1, 1000.0);
        let view = look_matrix(cam_x, cam_y, cam_z, self.yaw, self.pitch);
        let mvp = mat4_mul(&proj, &view);

        gl::uniform_matrix4fv(self.u_mvp, false, &mvp);
        gl::uniform3f(self.u_sun_dir, sun_dir[0], sun_dir[1], sun_dir[2]);
        gl::uniform1f(self.u_ambient, ambient);
        gl::uniform3f(self.u_fog_color, sky_horizon[0], sky_horizon[1], sky_horizon[2]);
        gl::uniform1f(self.u_fog_start, fog_start);
        gl::uniform1f(self.u_fog_end, fog_end);

        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, self.atlas_tex);
        gl::uniform1i(self.u_texture, 0);

        // Stride: FLOATS_PER_VERTEX * 4 bytes = 36 bytes (9 floats)
        let stride = (FLOATS_PER_VERTEX * 4) as i32;

        for (&(cx, cz), &(vbo, vert_count)) in &self.chunk_vbos {
            // Rough distance check for fog culling
            let chunk_center_x = cx as f32 * 16.0 + 8.0;
            let chunk_center_z = cz as f32 * 16.0 + 8.0;
            let dx = chunk_center_x - cam_x;
            let dz = chunk_center_z - cam_z;
            let dist_sq = dx * dx + dz * dz;
            if dist_sq > self.fog_distance * self.fog_distance {
                continue;
            }

            gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo);

            // aPosition: vec3 at offset 0
            gl::enable_vertex_attrib_array(self.a_position as u32);
            gl::vertex_attrib_pointer(self.a_position as u32, 3, gl::GL_FLOAT, false, stride, 0);

            // aTexCoord: vec2 at offset 12
            gl::enable_vertex_attrib_array(self.a_texcoord as u32);
            gl::vertex_attrib_pointer(self.a_texcoord as u32, 2, gl::GL_FLOAT, false, stride, 12);

            // aNormal: vec3 at offset 20
            gl::enable_vertex_attrib_array(self.a_normal as u32);
            gl::vertex_attrib_pointer(self.a_normal as u32, 3, gl::GL_FLOAT, false, stride, 20);

            // aLight: float at offset 32
            gl::enable_vertex_attrib_array(self.a_light as u32);
            gl::vertex_attrib_pointer(self.a_light as u32, 1, gl::GL_FLOAT, false, stride, 32);

            gl::draw_arrays(gl::GL_TRIANGLES, 0, vert_count as i32);
        }

        gl::disable_vertex_attrib_array(self.a_position as u32);
        gl::disable_vertex_attrib_array(self.a_texcoord as u32);
        gl::disable_vertex_attrib_array(self.a_normal as u32);
        gl::disable_vertex_attrib_array(self.a_light as u32);

        gl::disable(gl::GL_BLEND);
    }

    pub fn adapt_view_distance(&mut self, fps: f32) {
        if fps < 50.0 {
            self.target_fog_distance = (self.target_fog_distance - 8.0).max(64.0);
        } else if fps > 55.0 {
            self.target_fog_distance = (self.target_fog_distance + 8.0).min(192.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Shader compilation helpers
// ---------------------------------------------------------------------------

fn compile_program(vs_src: &str, fs_src: &str) -> u32 {
    let vs = gl::create_shader(gl::GL_VERTEX_SHADER);
    gl::shader_source(vs, vs_src);
    gl::compile_shader(vs);

    let fs = gl::create_shader(gl::GL_FRAGMENT_SHADER);
    gl::shader_source(fs, fs_src);
    gl::compile_shader(fs);

    let program = gl::create_program();
    gl::attach_shader(program, vs);
    gl::attach_shader(program, fs);
    gl::link_program(program);

    program
}

// ---------------------------------------------------------------------------
// Matrix math (column-major)
// ---------------------------------------------------------------------------

fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0f32;
            for k in 0..4 {
                sum += a[k * 4 + row] * b[col * 4 + k];
            }
            out[col * 4 + row] = sum;
        }
    }
    out
}

fn perspective(fov_deg: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let fov_rad = fov_deg * gl::PI / 180.0;
    let f = 1.0 / gl::tan(fov_rad * 0.5);
    let nf = 1.0 / (near - far);

    let mut m = [0.0f32; 16];
    m[0] = f / aspect;
    m[5] = f;
    m[10] = (far + near) * nf;
    m[11] = -1.0;
    m[14] = 2.0 * far * near * nf;
    m
}

fn look_matrix(x: f32, y: f32, z: f32, yaw: f32, pitch: f32) -> Mat4 {
    let cy = gl::cos(yaw);
    let sy = gl::sin(yaw);
    let cp = gl::cos(pitch);
    let sp = gl::sin(pitch);

    // Forward vector
    let fx = sy * cp;
    let fy = -sp;
    let fz = -cy * cp;

    // Right vector (forward x world_up)
    let rx = cy;
    let ry = 0.0;
    let rz = sy;

    // Up vector (right x forward)
    let ux = -sy * sp;
    let uy = -cp; // Note: negated because we look along -Z convention
    let uz = cy * sp;

    // Correct up: should be right x forward
    let ux2 = ry * fz - rz * fy;
    let uy2 = rz * fx - rx * fz;
    let uz2 = rx * fy - ry * fx;

    // Column-major view matrix: rotation then translation
    let mut m = [0.0f32; 16];
    m[0] = rx;
    m[1] = ux2;
    m[2] = -fx;
    m[3] = 0.0;

    m[4] = ry;
    m[5] = uy2;
    m[6] = -fy;
    m[7] = 0.0;

    m[8] = rz;
    m[9] = uz2;
    m[10] = -fz;
    m[11] = 0.0;

    m[12] = -(rx * x + ry * y + rz * z);
    m[13] = -(ux2 * x + uy2 * y + uz2 * z);
    m[14] = -(-fx * x + -fy * y + -fz * z);
    m[15] = 1.0;

    m
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
