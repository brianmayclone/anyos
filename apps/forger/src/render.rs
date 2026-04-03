extern crate alloc;

use alloc::collections::BTreeMap;
use libgl_client as gl;

use crate::mesh::{ChunkMesh, FLOATS_PER_VERTEX};

// ---------------------------------------------------------------------------
// Type alias
// ---------------------------------------------------------------------------
type Mat4 = [f32; 16];

// ---------------------------------------------------------------------------
// Day/Night cycle constants
// ---------------------------------------------------------------------------

/// Full day cycle duration in milliseconds (10 minutes).
const DAY_CYCLE_MS: u32 = 10 * 60 * 1000;

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

const VS_BLOCK: &str =
"attribute vec3 aPosition;
attribute vec2 aTexCoord;
attribute float aLight;
uniform mat4 uMVP;
uniform float uSunBrightness;
varying vec2 vTexCoord;
varying vec3 vLighting;
varying vec3 vDist;
void main() {
    gl_Position = uMVP * vec4(aPosition, 1.0);
    vTexCoord = aTexCoord;
    float L = aLight * uSunBrightness;
    vLighting = vec3(L, L, L);
    float d = gl_Position.w;
    vDist = vec3(d, d, d);
}
";

const FS_BLOCK: &str =
"void main() {
    gl_FragColor = vec4(0.0, 1.0, 0.0, 1.0);
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
uniform vec3 uSkyBottom;
uniform vec3 uSunDir;
uniform vec3 uSunColor;
uniform float uSunSize;
uniform vec3 uCamFwd;
uniform vec3 uCamRight;
uniform vec3 uCamUp;
uniform float uTanHalfFov;
uniform float uAspect;
void main() {
    // Reconstruct view ray from screen position using camera basis vectors
    vec3 ray = normalize(uCamFwd
        + vPos.x * uCamRight * uTanHalfFov * uAspect
        + vPos.y * uCamUp * uTanHalfFov);

    // Sky gradient based on vertical angle
    float elevation = ray.y;
    vec3 color;
    if (elevation > 0.0) {
        // Above horizon: horizon -> top
        float t = clamp(elevation * 2.5, 0.0, 1.0);
        color = mix(uSkyHorizon, uSkyTop, t);
    } else {
        // Below horizon: horizon -> bottom (ground haze)
        float t = clamp(-elevation * 4.0, 0.0, 1.0);
        color = mix(uSkyHorizon, uSkyBottom, t);
    }

    // Sun disc
    float sun_dot = dot(ray, uSunDir);
    if (sun_dot > uSunSize) {
        float t = (sun_dot - uSunSize) / (1.0 - uSunSize);
        color = mix(color, uSunColor, clamp(t * 3.0, 0.0, 1.0));
    }
    // Sun glow (larger soft halo)
    float glow_size = uSunSize - 0.04;
    if (sun_dot > glow_size && glow_size < 1.0) {
        float t = (sun_dot - glow_size) / (1.0 - glow_size);
        color = mix(color, uSunColor, t * t * 0.4);
    }

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
    pub u_sun_brightness: i32,
    pub a_position: i32,
    pub a_texcoord: i32,
    pub a_light: i32,
    // Sky shader locations
    pub u_sky_top: i32,
    pub u_sky_horizon: i32,
    pub u_sky_bottom: i32,
    pub u_sun_dir: i32,
    pub u_sun_color: i32,
    pub u_sun_size: i32,
    pub u_cam_fwd: i32,
    pub u_cam_right: i32,
    pub u_cam_up: i32,
    pub u_tan_half_fov: i32,
    pub u_aspect: i32,
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
        let u_sun_brightness = gl::get_uniform_location(block_program, "uSunBrightness");

        let a_position = gl::get_attrib_location(block_program, "aPosition");
        let a_texcoord = gl::get_attrib_location(block_program, "aTexCoord");
        let a_light = gl::get_attrib_location(block_program, "aLight");

        let u_sky_top = gl::get_uniform_location(sky_program, "uSkyTop");
        let u_sky_horizon = gl::get_uniform_location(sky_program, "uSkyHorizon");
        let u_sky_bottom = gl::get_uniform_location(sky_program, "uSkyBottom");
        let u_sun_dir = gl::get_uniform_location(sky_program, "uSunDir");
        let u_sun_color = gl::get_uniform_location(sky_program, "uSunColor");
        let u_sun_size = gl::get_uniform_location(sky_program, "uSunSize");
        let u_cam_fwd = gl::get_uniform_location(sky_program, "uCamFwd");
        let u_cam_right = gl::get_uniform_location(sky_program, "uCamRight");
        let u_cam_up = gl::get_uniform_location(sky_program, "uCamUp");
        let u_tan_half_fov = gl::get_uniform_location(sky_program, "uTanHalfFov");
        let u_aspect = gl::get_uniform_location(sky_program, "uAspect");
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
            u_sun_brightness,
            a_position,
            a_texcoord,
            a_light,
            u_sky_top,
            u_sky_horizon,
            u_sky_bottom,
            u_sun_dir,
            u_sun_color,
            u_sun_size,
            u_cam_fwd,
            u_cam_right,
            u_cam_up,
            u_tan_half_fov,
            u_aspect,
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
        // ── Day/Night cycle ──────────────────────────────────────────
        let now = anyos_std::sys::uptime_ms();
        let day_progress = (now % DAY_CYCLE_MS) as f32 / DAY_CYCLE_MS as f32;

        // Sun angle: 0.0 = sunrise (east), 0.25 = noon (top), 0.5 = sunset (west), 0.75 = midnight
        let sun_angle = day_progress * 2.0 * gl::PI;
        let sun_elevation = gl::sin(sun_angle);  // -1 to 1, positive = above horizon
        let sun_x = gl::cos(sun_angle);           // East-west component
        let sun_y = sun_elevation;
        let sun_z = 0.3 * gl::sin(sun_angle);     // Slight Z drift for realism
        // Normalize sun direction
        let sun_len = gl::sqrt(sun_x * sun_x + sun_y * sun_y + sun_z * sun_z);
        let sun_dir = [sun_x / sun_len, sun_y / sun_len, sun_z / sun_len];

        // Compute sky colors and lighting based on sun elevation
        let (sky_top, sky_horizon, sky_bottom, sun_color, sun_brightness) =
            compute_sky_colors(sun_elevation);

        // Smooth fog distance
        let fog_speed = 0.02;
        self.fog_distance += (self.target_fog_distance - self.fog_distance) * fog_speed;

        let fog_start = self.fog_distance * 0.6;
        let fog_end = self.fog_distance;

        // -- Clear --
        gl::viewport(0, 0, width as i32, height as i32);
        gl::clear_color(sky_horizon[0], sky_horizon[1], sky_horizon[2], 1.0);
        gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);

        // -- Block pass only (sky disabled for HW debug) --
        let aspect = width as f32 / height as f32;

        gl::enable(gl::GL_DEPTH_TEST);
        gl::depth_func(gl::GL_LESS);

        gl::use_program(self.block_program);

        // Build view-projection matrix for block pass
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
        gl::uniform1f(self.u_sun_brightness, sun_brightness);

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
// Day/Night cycle — sky color computation
// ---------------------------------------------------------------------------

/// Linearly interpolate two RGB colors.
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Compute sky colors, sun color, and block brightness from sun elevation (-1..1).
fn compute_sky_colors(sun_elev: f32) -> ([f32; 3], [f32; 3], [f32; 3], [f32; 3], f32) {
    // Sky palettes for different times of day
    //                 top              horizon          bottom (ground haze)
    let day_top     = [0.25, 0.45, 0.90];
    let day_horiz   = [0.55, 0.65, 0.90];
    let day_bottom  = [0.40, 0.50, 0.55];

    let sunset_top    = [0.15, 0.15, 0.45];
    let sunset_horiz  = [0.85, 0.40, 0.15];
    let sunset_bottom = [0.50, 0.25, 0.10];

    let night_top     = [0.01, 0.01, 0.05];
    let night_horiz   = [0.02, 0.02, 0.08];
    let night_bottom  = [0.01, 0.01, 0.03];

    let sun_color_day    = [1.0, 0.95, 0.8];
    let sun_color_sunset = [1.0, 0.5, 0.1];

    if sun_elev > 0.15 {
        // Full day
        let brightness = 0.7 + 0.3 * sun_elev.min(1.0);
        (day_top, day_horiz, day_bottom, sun_color_day, brightness)
    } else if sun_elev > 0.0 {
        // Sunrise/sunset transition (0.0 .. 0.15)
        let t = sun_elev / 0.15;
        let top = lerp3(sunset_top, day_top, t);
        let horiz = lerp3(sunset_horiz, day_horiz, t);
        let bottom = lerp3(sunset_bottom, day_bottom, t);
        let sun_col = lerp3(sun_color_sunset, sun_color_day, t);
        let brightness = 0.4 + 0.3 * t;
        (top, horiz, bottom, sun_col, brightness)
    } else if sun_elev > -0.15 {
        // Dusk/dawn transition (horizon glow, -0.15 .. 0.0)
        let t = (sun_elev + 0.15) / 0.15; // 0 at -0.15, 1 at 0.0
        let top = lerp3(night_top, sunset_top, t);
        let horiz = lerp3(night_horiz, sunset_horiz, t);
        let bottom = lerp3(night_bottom, sunset_bottom, t);
        let sun_col = lerp3([0.3, 0.1, 0.05], sun_color_sunset, t);
        let brightness = 0.15 + 0.25 * t;
        (top, horiz, bottom, sun_col, brightness)
    } else {
        // Night
        let brightness = 0.15;
        (night_top, night_horiz, night_bottom, [0.1, 0.1, 0.2], brightness)
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

    // Correct up: right x forward
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

