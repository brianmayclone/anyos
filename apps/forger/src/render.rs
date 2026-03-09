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

const VS_BLOCK: &str =
"attribute vec3 aPosition;
attribute vec2 aTexCoord;
attribute float aLight;
uniform mat4 uMVP;
varying vec2 vTexCoord;
varying float vLighting;
varying float vDist;
void main() {
    gl_Position = uMVP * vec4(aPosition, 1.0);
    vTexCoord = aTexCoord;
    vLighting = aLight;
    vDist = gl_Position.w;
}
";

const FS_BLOCK: &str =
"varying vec2 vTexCoord;
varying float vLighting;
varying float vDist;
uniform sampler2D uTexture;
uniform vec3 uFogColor;
uniform float uFogStart;
uniform float uFogEnd;
void main() {
    vec4 tex = texture2D(uTexture, vTexCoord);
    vec3 color = tex.rgb * vLighting;
    float t = clamp((vDist - uFogStart) / (uFogEnd - uFogStart), 0.0, 1.0);
    color = mix(color, uFogColor, t);
    gl_FragColor = vec4(color, 1.0);
}
";

const VS_SKY: &str =
"attribute vec2 aPosition;
varying vec2 vPos;
void main() {
    vPos = aPosition;
    gl_Position = vec4(aPosition, 0.999, 1.0);
}
";

const FS_SKY: &str =
"varying vec2 vPos;
uniform vec3 uSkyTop;
uniform vec3 uSkyHorizon;
void main() {
    float t = clamp(vPos.y * 0.5 + 0.5, 0.0, 1.0);
    vec3 color = mix(uSkyHorizon, uSkyTop, t);
    gl_FragColor = vec4(color, 1.0);
}
";

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
    pub u_fog_color: i32,
    pub u_fog_start: i32,
    pub u_fog_end: i32,
    pub u_texture: i32,
    pub a_position: i32,
    pub a_texcoord: i32,
    pub a_light: i32,
    // Sky shader locations
    pub u_sky_top: i32,
    pub u_sky_horizon: i32,
    pub a_sky_pos: i32,
    // Chunk VBOs: (vbo_id, vertex_count)
    pub chunk_vbos: BTreeMap<(i32, i32), (u32, u32)>,
    // Camera
    pub yaw: f32,
    pub pitch: f32,
    // Fog
    pub fog_distance: f32,
    pub target_fog_distance: f32,
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
        let u_fog_color = gl::get_uniform_location(block_program, "uFogColor");
        let u_fog_start = gl::get_uniform_location(block_program, "uFogStart");
        let u_fog_end = gl::get_uniform_location(block_program, "uFogEnd");
        let u_texture = gl::get_uniform_location(block_program, "uTexture");

        let a_position = gl::get_attrib_location(block_program, "aPosition");
        let a_texcoord = gl::get_attrib_location(block_program, "aTexCoord");
        let a_light = gl::get_attrib_location(block_program, "aLight");

        let u_sky_top = gl::get_uniform_location(sky_program, "uSkyTop");
        let u_sky_horizon = gl::get_uniform_location(sky_program, "uSkyHorizon");
        let a_sky_pos = gl::get_attrib_location(sky_program, "aPosition");

        anyos_std::println!("forger: block prog={} sky prog={}", block_program, sky_program);
        anyos_std::println!("forger: u_mvp={} u_fog_c={} u_fog_s={} u_fog_e={} u_tex={}", u_mvp, u_fog_color, u_fog_start, u_fog_end, u_texture);
        anyos_std::println!("forger: a_pos={} a_uv={} a_light={}", a_position, a_texcoord, a_light);

        Renderer {
            block_program,
            sky_program,
            atlas_tex,
            sky_vbo,
            u_mvp,
            u_fog_color,
            u_fog_start,
            u_fog_end,
            u_texture,
            a_position,
            a_texcoord,
            a_light,
            u_sky_top,
            u_sky_horizon,
            a_sky_pos,
            chunk_vbos: BTreeMap::new(),
            yaw: 0.0,
            pitch: 0.0,
            fog_distance: 32.0,
            target_fog_distance: 32.0,
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
        // Fixed noon lighting (no day/night cycle for performance)
        let sky_top = [0.3f32, 0.5, 0.9];
        let sky_horizon = [0.6f32, 0.7, 0.9];

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
        gl::depth_mask(false);
        gl::use_program(self.sky_program);

        gl::uniform3f(self.u_sky_top, sky_top[0], sky_top[1], sky_top[2]);
        gl::uniform3f(self.u_sky_horizon, sky_horizon[0], sky_horizon[1], sky_horizon[2]);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, self.sky_vbo);
        gl::enable_vertex_attrib_array(self.a_sky_pos as u32);
        gl::vertex_attrib_pointer(self.a_sky_pos as u32, 2, gl::GL_FLOAT, false, 8, 0);
        gl::draw_arrays(gl::GL_TRIANGLES, 0, 6);
        gl::disable_vertex_attrib_array(self.a_sky_pos as u32);

        // -- Block pass (no blending — all fragments output alpha=1.0) --
        gl::depth_mask(true);
        gl::enable(gl::GL_DEPTH_TEST);
        gl::depth_func(gl::GL_LESS);
        gl::disable(gl::GL_BLEND);

        gl::use_program(self.block_program);

        // Build matrices
        let aspect = width as f32 / height as f32;
        let proj = perspective(70.0, aspect, 0.1, 1000.0);
        let view = look_matrix(cam_x, cam_y, cam_z, self.yaw, self.pitch);
        let mvp = mat4_mul(&proj, &view);

        // Debug: print first draw call info once
        static mut DBG_ONCE: bool = true;
        unsafe {
            if DBG_ONCE {
                DBG_ONCE = false;
                anyos_std::println!("forger: block_prog={} sky_prog={}", self.block_program, self.sky_program);
                anyos_std::println!("forger: u_mvp={} a_pos={} a_uv={} a_light={}", self.u_mvp, self.a_position, self.a_texcoord, self.a_light);
                anyos_std::println!("forger: mvp diag: {},{},{},{}", mvp[0] as i32, mvp[5] as i32, mvp[10] as i32, mvp[15] as i32);
                anyos_std::println!("forger: cam=({},{},{}) fog_s={} fog_e={}", cam_x as i32, cam_y as i32, cam_z as i32, fog_start as i32, fog_end as i32);
                let mut total_verts = 0u32;
                let mut drawn_chunks = 0u32;
                for (&(cx, cz), &(_, vc)) in &self.chunk_vbos {
                    let ccx = cx as f32 * 16.0 + 8.0;
                    let ccz = cz as f32 * 16.0 + 8.0;
                    let dx = ccx - cam_x;
                    let dz = ccz - cam_z;
                    let d = dx * dx + dz * dz;
                    if d <= self.fog_distance * self.fog_distance {
                        total_verts += vc;
                        drawn_chunks += 1;
                    }
                }
                anyos_std::println!("forger: drawing {} chunks, {} verts (fog_dist={})", drawn_chunks, total_verts, self.fog_distance as i32);
            }
        }
        gl::uniform_matrix4fv(self.u_mvp, false, &mvp);
        gl::uniform3f(self.u_fog_color, sky_horizon[0], sky_horizon[1], sky_horizon[2]);
        gl::uniform1f(self.u_fog_start, fog_start);
        gl::uniform1f(self.u_fog_end, fog_end);

        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, self.atlas_tex);
        gl::uniform1i(self.u_texture, 0);

        // Stride: FLOATS_PER_VERTEX * 4 bytes = 24 bytes (6 floats)
        let stride = (FLOATS_PER_VERTEX * 4) as i32;

        // Camera forward vector for frustum culling
        let fwd_x = gl::sin(self.yaw);
        let fwd_z = -gl::cos(self.yaw);

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

            // Frustum culling: skip chunks behind camera (with margin for nearby chunks)
            if dist_sq > 256.0 {
                let dot = dx * fwd_x + dz * fwd_z;
                if dot < -12.0 {
                    continue;
                }
            }

            gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo);

            // aPosition: vec3 at offset 0
            gl::enable_vertex_attrib_array(self.a_position as u32);
            gl::vertex_attrib_pointer(self.a_position as u32, 3, gl::GL_FLOAT, false, stride, 0);

            // aTexCoord: vec2 at offset 12
            gl::enable_vertex_attrib_array(self.a_texcoord as u32);
            gl::vertex_attrib_pointer(self.a_texcoord as u32, 2, gl::GL_FLOAT, false, stride, 12);

            // aLight: float at offset 20
            gl::enable_vertex_attrib_array(self.a_light as u32);
            gl::vertex_attrib_pointer(self.a_light as u32, 1, gl::GL_FLOAT, false, stride, 20);

            gl::draw_arrays(gl::GL_TRIANGLES, 0, vert_count as i32);
        }

        gl::disable_vertex_attrib_array(self.a_position as u32);
        gl::disable_vertex_attrib_array(self.a_texcoord as u32);
        gl::disable_vertex_attrib_array(self.a_light as u32);
    }

    pub fn adapt_view_distance(&mut self, fps: f32) {
        if fps < 8.0 {
            self.target_fog_distance = (self.target_fog_distance - 8.0).max(24.0);
        } else if fps > 12.0 {
            self.target_fog_distance = (self.target_fog_distance + 4.0).min(64.0);
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
    if !gl::get_shader_compile_status(vs) {
        let log = gl::get_shader_info_log(vs);
        anyos_std::println!("forger: VS compile FAILED: {}", log);
    }

    let fs = gl::create_shader(gl::GL_FRAGMENT_SHADER);
    gl::shader_source(fs, fs_src);
    gl::compile_shader(fs);
    if !gl::get_shader_compile_status(fs) {
        let log = gl::get_shader_info_log(fs);
        anyos_std::println!("forger: FS compile FAILED: {}", log);
    }

    let program = gl::create_program();
    gl::attach_shader(program, vs);
    gl::attach_shader(program, fs);
    gl::link_program(program);
    if !gl::get_program_link_status(program) {
        anyos_std::println!("forger: program link FAILED");
    }

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

