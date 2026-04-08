extern crate alloc;

use alloc::collections::BTreeMap;
use libgl_client as gl;

use crate::mesh::{ChunkMesh, FLOATS_PER_VERTEX};

// ---------------------------------------------------------------------------
// Type alias
// ---------------------------------------------------------------------------
type Mat4 = [f32; 16];

fn mat4_identity() -> Mat4 {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

// ---------------------------------------------------------------------------
// Day/Night cycle constants
// ---------------------------------------------------------------------------

/// Full day cycle duration in milliseconds (10 minutes).
const DAY_CYCLE_MS: u32 = 10 * 60 * 1000;
const SUN_VISIBLE_SIZE: f32 = 0.9985;
const SUN_HIDDEN_SIZE: f32 = 2.0;
const SHADOW_WORLD_CENTER_Y: f32 = 64.0;

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

const VS_BLOCK: &str =
"attribute vec3 aPosition;
attribute vec2 aTexCoord;
attribute float aLight;
attribute vec3 aNormal;
attribute float aTranslucency;
uniform mat4 uMVP;
uniform mat4 uLightMVP;
uniform float uSunBrightness;
uniform vec3 uSunDir;
varying vec2 vTexCoord;
varying float vLighting;
varying float vDist;
varying vec4 vShadowCoord;
varying float vTranslucency;
void main() {
    vec4 worldPos = vec4(aPosition, 1.0);
    gl_Position = uMVP * worldPos;
    vTexCoord = aTexCoord;
    vec3 normal = normalize(aNormal);
    vec3 sunDir = normalize(uSunDir);
    float sunDiffuse = max(dot(normal, sunDir), 0.0);
    float sunBack = max(dot(-normal, sunDir), 0.0);
    float ambient = mix(0.24, 0.52, uSunBrightness) + aTranslucency * 0.10;
    float sunTerm = mix(0.08, 0.95, uSunBrightness) * sunDiffuse;
    float transmit = mix(0.04, 0.42, uSunBrightness) * sunBack * aTranslucency;
    vLighting = clamp((ambient + sunTerm + transmit) * aLight, 0.08, 1.25);
    vDist = gl_Position.w;
    vShadowCoord = uLightMVP * worldPos;
    vTranslucency = aTranslucency;
}
";

const FS_BLOCK: &str =
"varying vec2 vTexCoord;
varying float vLighting;
varying float vDist;
varying vec4 vShadowCoord;
varying float vTranslucency;
uniform sampler2D uTexture;
uniform sampler2D uShadowMap;
uniform vec3 uFogColor;
uniform float uFogStart;
uniform float uFogEnd;
uniform vec2 uShadowTexelSize;
uniform float uShadowStrength;
void main() {
    vec4 tex = texture2D(uTexture, vTexCoord);
    float visibility = 1.0;
    if (uShadowStrength > 0.001) {
        vec3 shadowNdc = vShadowCoord.xyz / max(vShadowCoord.w, 0.0001);
        vec2 shadowUv = vec2(
            shadowNdc.x * 0.5 + 0.5,
            0.5 - shadowNdc.y * 0.5
        );
        float shadowDepth = shadowNdc.z * 0.5 + 0.5;
        if (shadowUv.x >= 0.0 && shadowUv.x <= 1.0 &&
            shadowUv.y >= 0.0 && shadowUv.y <= 1.0 &&
            shadowDepth >= 0.0 && shadowDepth <= 1.0) {
            float cmpDepth = shadowDepth - 0.0014;
            vec2 sx = vec2(uShadowTexelSize.x * 1.5, 0.0);
            vec2 sy = vec2(0.0, uShadowTexelSize.y * 1.5);
            float lit = 0.0;
            lit += (cmpDepth <= texture2D(uShadowMap, shadowUv).r ? 1.0 : 0.0) * 0.36;
            lit += (cmpDepth <= texture2D(uShadowMap, shadowUv - sx).r ? 1.0 : 0.0) * 0.16;
            lit += (cmpDepth <= texture2D(uShadowMap, shadowUv + sx).r ? 1.0 : 0.0) * 0.16;
            lit += (cmpDepth <= texture2D(uShadowMap, shadowUv - sy).r ? 1.0 : 0.0) * 0.16;
            lit += (cmpDepth <= texture2D(uShadowMap, shadowUv + sy).r ? 1.0 : 0.0) * 0.16;
            float shadow_blocking = 1.0 - vTranslucency * 0.55;
            visibility = 1.0 - uShadowStrength * shadow_blocking * (1.0 - lit);
        }
    }
    vec3 color = tex.rgb * vLighting * visibility;
    float t = clamp((vDist - uFogStart) / (uFogEnd - uFogStart), 0.0, 1.0);
    color = mix(color, uFogColor, t);
    gl_FragColor = vec4(color, 1.0);
}
";

const VS_SHADOW: &str =
"attribute vec3 aPosition;
uniform mat4 uLightMVP;
void main() {
    gl_Position = uLightMVP * vec4(aPosition, 1.0);
}
";

const FS_SHADOW: &str =
"void main() {
    gl_FragColor = vec4(1.0, 1.0, 1.0, 1.0);
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
uniform float uSunOpacity;
uniform vec3 uCamFwd;
uniform vec3 uCamRight;
uniform vec3 uCamUp;
uniform float uTanHalfFov;
uniform float uAspect;
void main() {
    vec3 ray = normalize(uCamFwd
        + vPos.x * uCamRight * uTanHalfFov * uAspect
        + vPos.y * uCamUp * uTanHalfFov);

    float elevation = ray.y;

    // Branchless sky gradient:
    // t_up = elevation clamped to [0,1] scaled by 2.5 → above horizon blend
    // t_down = -elevation clamped to [0,1] scaled by 4.0 → below horizon blend
    float t_up = clamp(elevation * 2.5, 0.0, 1.0);
    float t_down = clamp(-elevation * 4.0, 0.0, 1.0);
    // When elevation > 0: t_up > 0, t_down = 0 → mix toward top
    // When elevation < 0: t_up = 0, t_down > 0 → mix toward bottom
    vec3 color = mix(uSkyHorizon, uSkyTop, t_up);
    color = mix(color, uSkyBottom, t_down);

    // Branchless sun disc + glow
    float sun_dot = dot(ray, uSunDir);
    float sun_t = clamp((sun_dot - uSunSize) / (1.0 - uSunSize + 0.001) * 3.0, 0.0, 1.0) * uSunOpacity;
    color = mix(color, uSunColor, sun_t);
    float glow_size = uSunSize - 0.04;
    float glow_t = clamp((sun_dot - glow_size) / (1.0 - glow_size + 0.001), 0.0, 1.0) * uSunOpacity;
    color = mix(color, uSunColor, glow_t * glow_t * 0.4);

    gl_FragColor = vec4(color, 1.0);
}
";

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

pub struct Renderer {
    pub block_program: u32,
    pub sky_program: u32,
    pub shadow_program: u32,
    pub atlas_tex: u32,
    pub sky_vbo: u32,
    // Block shader uniform/attrib locations
    pub u_mvp: i32,
    pub u_fog_color: i32,
    pub u_fog_start: i32,
    pub u_fog_end: i32,
    pub u_texture: i32,
    pub u_sun_brightness: i32,
    pub u_block_sun_dir: i32,
    pub u_light_mvp: i32,
    pub u_shadow_map: i32,
    pub u_shadow_texel_size: i32,
    pub u_shadow_strength: i32,
    pub a_position: i32,
    pub a_texcoord: i32,
    pub a_light: i32,
    pub a_normal: i32,
    pub a_translucency: i32,
    pub u_shadow_pass_light_mvp: i32,
    pub a_shadow_position: i32,
    // Sky shader locations
    pub u_sky_top: i32,
    pub u_sky_horizon: i32,
    pub u_sky_bottom: i32,
    pub u_sun_dir: i32,
    pub u_sun_color: i32,
    pub u_sun_size: i32,
    pub u_sun_opacity: i32,
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

struct SunState {
    dir: [f32; 3],
    elevation: f32,
    color: [f32; 3],
    brightness: f32,
    sky_top: [f32; 3],
    sky_horizon: [f32; 3],
    sky_bottom: [f32; 3],
    visible_size: f32,
    visible_opacity: f32,
}

pub struct SunDebugInfo {
    pub day_progress: f32,
    pub hour: u32,
    pub minute: u32,
    pub elevation_deg: f32,
    pub dir: [f32; 3],
}

impl Renderer {
    pub fn init(atlas_data: &[u8], atlas_w: u32, atlas_h: u32) -> Self {
        // -- Block shader program --
        let block_program = compile_program(VS_BLOCK, FS_BLOCK);

        // -- Sky shader program --
        let sky_program = compile_program(VS_SKY, FS_SKY);
        let shadow_program = compile_program(VS_SHADOW, FS_SHADOW);

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
        gl::generate_mipmap(gl::GL_TEXTURE_2D);

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
        let u_block_sun_dir = gl::get_uniform_location(block_program, "uSunDir");
        let u_light_mvp = gl::get_uniform_location(block_program, "uLightMVP");
        let u_shadow_map = gl::get_uniform_location(block_program, "uShadowMap");
        let u_shadow_texel_size = gl::get_uniform_location(block_program, "uShadowTexelSize");
        let u_shadow_strength = gl::get_uniform_location(block_program, "uShadowStrength");

        let a_position = gl::get_attrib_location(block_program, "aPosition");
        let a_texcoord = gl::get_attrib_location(block_program, "aTexCoord");
        let a_light = gl::get_attrib_location(block_program, "aLight");
        let a_normal = gl::get_attrib_location(block_program, "aNormal");
        let a_translucency = gl::get_attrib_location(block_program, "aTranslucency");
        let u_shadow_pass_light_mvp = gl::get_uniform_location(shadow_program, "uLightMVP");
        let a_shadow_position = gl::get_attrib_location(shadow_program, "aPosition");

        let u_sky_top = gl::get_uniform_location(sky_program, "uSkyTop");
        let u_sky_horizon = gl::get_uniform_location(sky_program, "uSkyHorizon");
        let u_sky_bottom = gl::get_uniform_location(sky_program, "uSkyBottom");
        let u_sun_dir = gl::get_uniform_location(sky_program, "uSunDir");
        let u_sun_color = gl::get_uniform_location(sky_program, "uSunColor");
        let u_sun_size = gl::get_uniform_location(sky_program, "uSunSize");
        let u_sun_opacity = gl::get_uniform_location(sky_program, "uSunOpacity");
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
            shadow_program,
            atlas_tex,
            sky_vbo,
            u_mvp,
            u_fog_color,
            u_fog_start,
            u_fog_end,
            u_texture,
            u_sun_brightness,
            u_block_sun_dir,
            u_light_mvp,
            u_shadow_map,
            u_shadow_texel_size,
            u_shadow_strength,
            a_position,
            a_texcoord,
            a_light,
            a_normal,
            a_translucency,
            u_shadow_pass_light_mvp,
            a_shadow_position,
            u_sky_top,
            u_sky_horizon,
            u_sky_bottom,
            u_sun_dir,
            u_sun_color,
            u_sun_size,
            u_sun_opacity,
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

    pub fn render(&mut self, cam_x: f32, cam_y: f32, cam_z: f32, width: u32, height: u32, shadows_enabled: bool) {
        // ── Day/Night cycle ──────────────────────────────────────────
        let now = anyos_std::sys::uptime_ms();
        let day_progress = (now % DAY_CYCLE_MS) as f32 / DAY_CYCLE_MS as f32;
        let sun = compute_sun_state(day_progress);

        // Smooth fog distance
        let fog_speed = 0.02;
        self.fog_distance += (self.target_fog_distance - self.fog_distance) * fog_speed;

        let fog_start = self.fog_distance * 0.6;
        let fog_end = self.fog_distance;
        let shadow_radius = (self.fog_distance * 0.9).max(18.0).min(42.0);
        let shadow_target = [0.0, SHADOW_WORLD_CENTER_Y, 0.0];
        let shadow_distance = shadow_radius + 18.0;
        let shadow_light_x = shadow_target[0] + sun.dir[0] * shadow_distance;
        let shadow_light_y = shadow_target[1] + sun.dir[1] * shadow_distance;
        let shadow_light_z = shadow_target[2] + sun.dir[2] * shadow_distance;

        let mut shadows_ready = false;
        let mut light_mvp = mat4_identity();
        if shadows_enabled
            && sun.elevation > 0.02
            && gl::shadow_pass_begin(
                shadow_light_x,
                shadow_light_y,
                shadow_light_z,
                shadow_target[0],
                shadow_target[1],
                shadow_target[2],
                shadow_radius,
            )
        {
            let light_mvp_ptr = gl::shadow_get_light_mvp();
            if !light_mvp_ptr.is_null() {
                light_mvp = unsafe { *(light_mvp_ptr as *const [f32; 16]) };
                self.render_shadow_chunks(&light_mvp);
                shadows_ready = true;
            }
            gl::shadow_pass_end();
            if shadows_ready {
                let ptr = gl::shadow_get_light_mvp();
                if !ptr.is_null() {
                    light_mvp = unsafe { *(ptr as *const [f32; 16]) };
                }
                shadows_ready = gl::shadow_available();
            }
        }

        // -- Clear --
        gl::viewport(0, 0, width as i32, height as i32);
        gl::clear_color(sun.sky_horizon[0], sun.sky_horizon[1], sun.sky_horizon[2], 1.0);
        gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);

        // -- Sky pass (depth test OFF so sky doesn't write to depth buffer) --
        gl::disable(gl::GL_DEPTH_TEST);
        let aspect = width as f32 / height as f32;

        gl::use_program(self.sky_program);

        gl::uniform3f(self.u_sky_top, sun.sky_top[0], sun.sky_top[1], sun.sky_top[2]);
        gl::uniform3f(self.u_sky_horizon, sun.sky_horizon[0], sun.sky_horizon[1], sun.sky_horizon[2]);
        gl::uniform3f(self.u_sky_bottom, sun.sky_bottom[0], sun.sky_bottom[1], sun.sky_bottom[2]);
        gl::uniform3f(self.u_sun_dir, sun.dir[0], sun.dir[1], sun.dir[2]);
        gl::uniform3f(self.u_sun_color, sun.color[0], sun.color[1], sun.color[2]);
        gl::uniform1f(self.u_sun_size, sun.visible_size);
        gl::uniform1f(self.u_sun_opacity, sun.visible_opacity);

        let fov_rad = 70.0 * gl::PI / 180.0;
        let tan_half_fov = gl::tan(fov_rad * 0.5);
        let cy = gl::cos(self.yaw);
        let sy = gl::sin(self.yaw);
        let cp = gl::cos(self.pitch);
        let sp = gl::sin(self.pitch);
        let fwd = [sy * cp, -sp, -cy * cp];
        let right = [cy, 0.0, sy];
        let up = [
            right[1] * fwd[2] - right[2] * fwd[1],
            right[2] * fwd[0] - right[0] * fwd[2],
            right[0] * fwd[1] - right[1] * fwd[0],
        ];
        gl::uniform3f(self.u_cam_fwd, fwd[0], fwd[1], fwd[2]);
        gl::uniform3f(self.u_cam_right, right[0], right[1], right[2]);
        gl::uniform3f(self.u_cam_up, up[0], up[1], up[2]);
        gl::uniform1f(self.u_tan_half_fov, tan_half_fov);
        gl::uniform1f(self.u_aspect, aspect);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, self.sky_vbo);
        gl::enable_vertex_attrib_array(self.a_sky_pos as u32);
        gl::vertex_attrib_pointer(self.a_sky_pos as u32, 2, gl::GL_FLOAT, false, 8, 0);
        gl::draw_arrays(gl::GL_TRIANGLES, 0, 6);
        gl::disable_vertex_attrib_array(self.a_sky_pos as u32);

        // -- Block pass (depth test ON, draws over sky) --
        gl::enable(gl::GL_DEPTH_TEST);
        gl::depth_func(gl::GL_LESS);
        gl::disable(gl::GL_BLEND);

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
        gl::uniform3f(self.u_fog_color, sun.sky_horizon[0], sun.sky_horizon[1], sun.sky_horizon[2]);
        gl::uniform1f(self.u_fog_start, fog_start);
        gl::uniform1f(self.u_fog_end, fog_end);
        gl::uniform1f(self.u_sun_brightness, sun.brightness);
        gl::uniform3f(self.u_block_sun_dir, sun.dir[0], sun.dir[1], sun.dir[2]);
        gl::uniform_matrix4fv(self.u_light_mvp, false, &light_mvp);
        let shadow_map_size = gl::shadow_get_map_size().max(1) as f32;
        gl::uniform2f(self.u_shadow_texel_size, 1.0 / shadow_map_size, 1.0 / shadow_map_size);
        gl::uniform1f(self.u_shadow_strength, if shadows_ready { 0.72 } else { 0.0 });

        gl::active_texture(gl::GL_TEXTURE0);
        gl::uniform1i(self.u_texture, 0);
        gl::active_texture(gl::GL_TEXTURE0 + gl::shadow_get_unit());
        gl::bind_texture(
            gl::GL_TEXTURE_2D,
            if shadows_ready { gl::shadow_get_texture() } else { 0 },
        );
        gl::uniform1i(self.u_shadow_map, gl::shadow_get_unit() as i32);
        gl::active_texture(gl::GL_TEXTURE0);

        // Stride: FLOATS_PER_VERTEX * 4 bytes = 24 bytes (6 floats)
        let stride = (FLOATS_PER_VERTEX * 4) as i32;

        // Camera forward vector for frustum culling
        let fwd_x = gl::sin(self.yaw);
        let fwd_z = -gl::cos(self.yaw);

        gl::enable_vertex_attrib_array(self.a_position as u32);
        gl::enable_vertex_attrib_array(self.a_texcoord as u32);
        gl::enable_vertex_attrib_array(self.a_light as u32);
        gl::enable_vertex_attrib_array(self.a_normal as u32);
        gl::enable_vertex_attrib_array(self.a_translucency as u32);

        gl::bind_texture(gl::GL_TEXTURE_2D, self.atlas_tex);
        self.draw_chunk_group(cam_x, cam_z, stride, fwd_x, fwd_z, 0.0, f32::MAX);
        gl::disable_vertex_attrib_array(self.a_position as u32);
        gl::disable_vertex_attrib_array(self.a_texcoord as u32);
        gl::disable_vertex_attrib_array(self.a_light as u32);
        gl::disable_vertex_attrib_array(self.a_normal as u32);
        gl::disable_vertex_attrib_array(self.a_translucency as u32);
    }

    fn visible_chunk_delta(&self, cx: i32, cz: i32, cam_x: f32, cam_z: f32) -> Option<(f32, f32, f32)> {
        let chunk_center_x = cx as f32 * 16.0 + 8.0;
        let chunk_center_z = cz as f32 * 16.0 + 8.0;
        let dx = chunk_center_x - cam_x;
        let dz = chunk_center_z - cam_z;
        let dist_sq = dx * dx + dz * dz;
        if dist_sq > self.fog_distance * self.fog_distance {
            None
        } else {
            Some((dx, dz, dist_sq))
        }
    }

    fn render_shadow_chunks(&self, light_mvp: &Mat4) {
        gl::use_program(self.shadow_program);
        gl::uniform_matrix4fv(self.u_shadow_pass_light_mvp, false, light_mvp);
        gl::enable(gl::GL_DEPTH_TEST);
        gl::depth_func(gl::GL_LESS);
        gl::disable(gl::GL_BLEND);

        gl::enable_vertex_attrib_array(self.a_shadow_position as u32);
        let stride = (FLOATS_PER_VERTEX * 4) as i32;
        for (&(cx, cz), &(vbo, vert_count)) in &self.chunk_vbos {
            gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo);
            gl::vertex_attrib_pointer(self.a_shadow_position as u32, 3, gl::GL_FLOAT, false, stride, 0);
            gl::draw_arrays(gl::GL_TRIANGLES, 0, vert_count as i32);
        }
        gl::disable_vertex_attrib_array(self.a_shadow_position as u32);
    }

    fn draw_chunk_group(
        &self,
        cam_x: f32,
        cam_z: f32,
        stride: i32,
        fwd_x: f32,
        fwd_z: f32,
        min_dist_sq: f32,
        max_dist_sq: f32,
    ) {
        for (&(cx, cz), &(vbo, vert_count)) in &self.chunk_vbos {
            let Some((dx, dz, dist_sq)) = self.visible_chunk_delta(cx, cz, cam_x, cam_z) else {
                continue;
            };
            if dist_sq < min_dist_sq || dist_sq >= max_dist_sq {
                continue;
            }
            if dist_sq > 256.0 {
                let dot = dx * fwd_x + dz * fwd_z;
                if dot < -12.0 {
                    continue;
                }
            }

            gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo);
            gl::vertex_attrib_pointer(self.a_position as u32, 3, gl::GL_FLOAT, false, stride, 0);
            gl::vertex_attrib_pointer(self.a_texcoord as u32, 2, gl::GL_FLOAT, false, stride, 12);
            gl::vertex_attrib_pointer(self.a_light as u32, 1, gl::GL_FLOAT, false, stride, 20);
            gl::vertex_attrib_pointer(self.a_normal as u32, 3, gl::GL_FLOAT, false, stride, 24);
            gl::vertex_attrib_pointer(self.a_translucency as u32, 1, gl::GL_FLOAT, false, stride, 36);
            gl::draw_arrays(gl::GL_TRIANGLES, 0, vert_count as i32);
        }
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

fn lerp1(a: f32, b: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    a + (b - a) * t
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge0 == edge1 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn compute_sun_state(day_progress: f32) -> SunState {
    // Follow a clean 180 degree arc in the vertical Y/Z plane:
    // 0.0 = sunrise at the forward horizon, 0.25 = noon overhead,
    // 0.5 = sunset at the rear horizon, 0.75 = midnight below the world.
    let sun_angle = day_progress * 2.0 * gl::PI;
    let elevation = gl::sin(sun_angle);
    let sun_x = 0.0;
    let sun_y = elevation;
    let sun_z = -gl::cos(sun_angle);
    let dir = [sun_x, sun_y, sun_z];
    let (sky_top, sky_horizon, sky_bottom, color, brightness) = compute_sky_colors(elevation);
    let sun_visibility = smoothstep(-0.16, 0.05, elevation);
    let visible_size = lerp1(0.9965, SUN_VISIBLE_SIZE, sun_visibility);
    SunState {
        dir,
        elevation,
        color,
        brightness,
        sky_top,
        sky_horizon,
        sky_bottom,
        visible_size,
        visible_opacity: sun_visibility,
    }
}

pub fn compute_sun_debug(now_ms: u32) -> SunDebugInfo {
    let day_progress = (now_ms % DAY_CYCLE_MS) as f32 / DAY_CYCLE_MS as f32;
    let sun = compute_sun_state(day_progress);

    // Map the cycle to a civil clock where sunrise starts at 06:00.
    let clock_hours = (day_progress * 24.0 + 6.0) % 24.0;
    let total_minutes = (clock_hours * 60.0) as u32;
    let hour = (total_minutes / 60) % 24;
    let minute = total_minutes % 60;
    let sun_angle_deg = day_progress * 360.0;
    let elevation_deg = if sun_angle_deg <= 90.0 {
        sun_angle_deg
    } else if sun_angle_deg <= 270.0 {
        180.0 - sun_angle_deg
    } else {
        sun_angle_deg - 360.0
    };

    SunDebugInfo {
        day_progress,
        hour,
        minute,
        elevation_deg,
        dir: sun.dir,
    }
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
    let twilight = smoothstep(-0.28, 0.12, sun_elev);
    let daylight = smoothstep(-0.02, 0.35, sun_elev);
    let zenith = smoothstep(0.18, 0.92, sun_elev.max(0.0));

    let mut top = lerp3(night_top, sunset_top, twilight);
    let mut horiz = lerp3(night_horiz, sunset_horiz, twilight);
    let mut bottom = lerp3(night_bottom, sunset_bottom, twilight);
    top = lerp3(top, day_top, daylight);
    horiz = lerp3(horiz, day_horiz, daylight);
    bottom = lerp3(bottom, day_bottom, daylight);

    let mut sun_col = lerp3([0.3, 0.1, 0.05], sun_color_sunset, twilight);
    sun_col = lerp3(sun_col, sun_color_day, daylight);

    let brightness = (0.10 + 0.22 * twilight + 0.40 * daylight + 0.28 * zenith).clamp(0.10, 1.0);
    (top, horiz, bottom, sun_col, brightness)
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
