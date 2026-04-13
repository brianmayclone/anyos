//! gldemo — Gouraud-lit 3D scene with sphere, cube, textures and animated lights.
//!
//! Demonstrates the optimized libgl software rasterizer with:
//! - Per-vertex Gouraud shading (ambient + diffuse + specular at vertices)
//! - One animated point light
//! - Procedural checkerboard texture
//! - UV sphere + textured cube rotating in 3D space
//! - 60fps animation loop via anyui timer

#![no_std]
#![no_main]
#![allow(unused, dead_code)]

anyos_std::entry!(main);

use alloc::vec;
use alloc::vec::Vec;

use libgl_client as gl;

// ── Shaders ──────────────────────────────────────────────────────────────────

/// Vertex shader: Gouraud shading — computes lighting per-vertex.
///
/// Lighting runs on ~187 vertices (sphere 10×16) instead of ~90K pixels,
/// giving a ~500× reduction in lighting math.
static VS_SOURCE: &str =
"attribute vec3 aPosition;
attribute vec3 aNormal;
attribute vec2 aTexCoord;
uniform mat4 uMVP;
uniform mat4 uModel;
uniform mat4 uLightMVP;
uniform vec3 uLightPos0;
uniform vec3 uLightColor0;
uniform vec3 uEyePos;
varying vec3 vLighting;
varying vec2 vTexCoord;
varying vec4 vShadowCoord;
varying vec3 vWorldNormal;
varying vec3 vWorldPos;
void main() {
    vec4 worldPos = uModel * vec4(aPosition, 1.0);
    vec3 wp = worldPos.xyz;
    vec4 tn = uModel * vec4(aNormal, 0.0);
    vec3 N = normalize(tn.xyz);
    vec3 V = normalize(uEyePos - wp);
    vec3 ambient = vec3(0.08, 0.08, 0.10);
    vec3 L0 = normalize(uLightPos0 - wp);
    float diff0 = max(dot(N, L0), 0.0);
    vec3 H0 = normalize(L0 + V);
    float spec0 = pow(max(dot(N, H0), 0.0), 32.0);
    vec3 c0 = uLightColor0 * diff0 + uLightColor0 * spec0;
    vLighting = ambient + c0;
    vTexCoord = aTexCoord;
    vShadowCoord = uLightMVP * worldPos;
    vWorldNormal = N;
    vWorldPos = wp;
    gl_Position = uMVP * vec4(aPosition, 1.0);
}
";

/// Fragment shader: texture × lighting × shadow visibility.
static FS_SOURCE: &str =
"varying vec3 vLighting;
varying vec2 vTexCoord;
varying vec4 vShadowCoord;
varying vec3 vWorldNormal;
varying vec3 vWorldPos;
uniform sampler2D uTexture;
uniform sampler2D uShadowMap;
uniform vec4 uMatColor;
uniform vec3 uLightPos0;
uniform vec2 uShadowTexelSize;
uniform float uShadowFlipY;
void main() {
    vec4 texColor = texture2D(uTexture, vTexCoord);
    vec3 shadowNdc = vShadowCoord.xyz / max(vShadowCoord.w, 0.0001);
    // The software shadow buffer uses a top-left convention, while the virgl
    // hardware path samples the shadow texture in the usual GL orientation.
    float shadowY = mix(
        shadowNdc.y * 0.5 + 0.5,
        0.5 - shadowNdc.y * 0.5,
        clamp(uShadowFlipY, 0.0, 1.0)
    );
    vec2 shadowUv = vec2(
        shadowNdc.x * 0.5 + 0.5,
        shadowY
    );
    float shadowDepth = shadowNdc.z * 0.5 + 0.5;
    float visibility = 1.0;
    vec3 N = normalize(vWorldNormal);
    vec3 L = normalize(uLightPos0 - vWorldPos);
    float ndotl = max(dot(N, L), 0.0);
    float shadowBias = 0.0007 + (1.0 - ndotl) * 0.0035;
    if (shadowUv.x >= 0.0 && shadowUv.x <= 1.0 &&
        shadowUv.y >= 0.0 && shadowUv.y <= 1.0 &&
        shadowDepth >= 0.0 && shadowDepth <= 1.0) {
        float lit = 0.0;
        float cmpDepth = shadowDepth - shadowBias;
        vec2 sx = vec2(uShadowTexelSize.x * 1.5, 0.0);
        vec2 sy = vec2(0.0, uShadowTexelSize.y * 1.5);
        lit += (cmpDepth <= texture2D(uShadowMap, shadowUv - sx - sy).r ? 1.0 : 0.0) * 0.085;
        lit += (cmpDepth <= texture2D(uShadowMap, shadowUv - sy).r ? 1.0 : 0.0) * 0.120;
        lit += (cmpDepth <= texture2D(uShadowMap, shadowUv + sx - sy).r ? 1.0 : 0.0) * 0.085;
        lit += (cmpDepth <= texture2D(uShadowMap, shadowUv - sx).r ? 1.0 : 0.0) * 0.120;
        lit += (cmpDepth <= texture2D(uShadowMap, shadowUv).r ? 1.0 : 0.0) * 0.180;
        lit += (cmpDepth <= texture2D(uShadowMap, shadowUv + sx).r ? 1.0 : 0.0) * 0.120;
        lit += (cmpDepth <= texture2D(uShadowMap, shadowUv - sx + sy).r ? 1.0 : 0.0) * 0.085;
        lit += (cmpDepth <= texture2D(uShadowMap, shadowUv + sy).r ? 1.0 : 0.0) * 0.120;
        lit += (cmpDepth <= texture2D(uShadowMap, shadowUv + sx + sy).r ? 1.0 : 0.0) * 0.085;
        visibility = 0.30 + 0.70 * lit;
    }
    gl_FragColor = vec4(vLighting * texColor.rgb * uMatColor.rgb * visibility,
                        texColor.a * uMatColor.a);
}
";

static SHADOW_VS_SOURCE: &str =
"attribute vec3 aPosition;
uniform mat4 uLightMVP;
uniform mat4 uModel;
void main() {
    vec4 worldPos = uModel * vec4(aPosition, 1.0);
    gl_Position = uLightMVP * worldPos;
}
";

static SHADOW_FS_SOURCE: &str =
"void main() {
    gl_FragColor = vec4(1.0, 1.0, 1.0, 1.0);
}
";

// ── Math helpers ─────────────────────────────────────────────────────────────

type Mat4 = [f32; 16];

fn mat4_identity() -> Mat4 {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Column-major mat4 multiply: result = a * b.
fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            r[col * 4 + row] =
                a[0 * 4 + row] * b[col * 4 + 0] +
                a[1 * 4 + row] * b[col * 4 + 1] +
                a[2 * 4 + row] * b[col * 4 + 2] +
                a[3 * 4 + row] * b[col * 4 + 3];
        }
    }
    r
}

fn mat4_translate(tx: f32, ty: f32, tz: f32) -> Mat4 {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
         tx,  ty,  tz, 1.0,
    ]
}

fn mat4_scale(sx: f32, sy: f32, sz: f32) -> Mat4 {
    [
         sx, 0.0, 0.0, 0.0,
        0.0,  sy, 0.0, 0.0,
        0.0, 0.0,  sz, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn soft_body_scale(body_id: u32, base_scale: f32) -> Mat4 {
    let (sx, sy, sz) = gl::physics_get_deformation_scale(body_id);
    mat4_scale(base_scale * sx, base_scale * sy, base_scale * sz)
}

fn mat4_rotate_y(angle: f32) -> Mat4 {
    let c = gl::cos(angle);
    let s = gl::sin(angle);
    [
          c, 0.0,  -s, 0.0,
        0.0, 1.0, 0.0, 0.0,
          s, 0.0,   c, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn mat4_rotate_x(angle: f32) -> Mat4 {
    let c = gl::cos(angle);
    let s = gl::sin(angle);
    [
        1.0, 0.0, 0.0, 0.0,
        0.0,   c,   s, 0.0,
        0.0,  -s,   c, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Quaternion (w, x, y, z) to 4x4 rotation matrix (column-major).
fn quat_to_mat4(w: f32, x: f32, y: f32, z: f32) -> Mat4 {
    let x2 = x + x; let y2 = y + y; let z2 = z + z;
    let xx = x * x2; let xy = x * y2; let xz = x * z2;
    let yy = y * y2; let yz = y * z2; let zz = z * z2;
    let wx = w * x2; let wy = w * y2; let wz = w * z2;
    [
        1.0 - yy - zz,  xy + wz,        xz - wy,        0.0,  // column 0
        xy - wz,         1.0 - xx - zz,  yz + wx,        0.0,  // column 1
        xz + wy,         yz - wx,        1.0 - xx - yy,  0.0,  // column 2
        0.0,             0.0,            0.0,             1.0,  // column 3
    ]
}

/// Look-at view matrix (column-major).
fn mat4_look_at(eye: &[f32; 3], target: &[f32; 3], up: &[f32; 3]) -> Mat4 {
    let fx = target[0] - eye[0];
    let fy = target[1] - eye[1];
    let fz = target[2] - eye[2];
    let flen = gl::sqrt(fx * fx + fy * fy + fz * fz);
    let (fx, fy, fz) = if flen > 0.0001 { (fx / flen, fy / flen, fz / flen) } else { (0.0, 0.0, -1.0) };

    // s = f × up
    let sx = fy * up[2] - fz * up[1];
    let sy = fz * up[0] - fx * up[2];
    let sz = fx * up[1] - fy * up[0];
    let slen = gl::sqrt(sx * sx + sy * sy + sz * sz);
    let (sx, sy, sz) = if slen > 0.0001 { (sx / slen, sy / slen, sz / slen) } else { (1.0, 0.0, 0.0) };

    // u = s × f
    let ux = sy * fz - sz * fy;
    let uy = sz * fx - sx * fz;
    let uz = sx * fy - sy * fx;

    // Column-major: [col0, col1, col2, col3]
    [
         sx,  ux, -fx, 0.0,  // column 0
         sy,  uy, -fy, 0.0,  // column 1
         sz,  uz, -fz, 0.0,  // column 2
        -(sx * eye[0] + sy * eye[1] + sz * eye[2]),
        -(ux * eye[0] + uy * eye[1] + uz * eye[2]),
         (fx * eye[0] + fy * eye[1] + fz * eye[2]),
        1.0,                  // column 3
    ]
}

/// Perspective projection matrix (column-major).
fn mat4_perspective(fov_rad: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / gl::tan(fov_rad * 0.5);
    let nf = 1.0 / (near - far);
    [
        f / aspect, 0.0,            0.0, 0.0,
              0.0,    f,            0.0, 0.0,
              0.0,  0.0, (far + near) * nf, -1.0,
              0.0,  0.0, 2.0 * far * near * nf, 0.0,
    ]
}

// ── Geometry generation ──────────────────────────────────────────────────────

/// Interleaved vertex: position(3) + normal(3) + texcoord(2) = 8 floats.
const VERTEX_STRIDE: usize = 8;

/// Generate a UV sphere.
/// Returns (vertex_data, index_data).
fn generate_sphere(rings: u32, sectors: u32) -> (Vec<f32>, Vec<u16>) {
    let pi: f32 = 3.14159265;
    let mut verts = Vec::new();
    let mut indices = Vec::new();

    // Vertex 0: north pole
    verts.extend_from_slice(&[0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.5, 0.0]);

    // Middle rings (1..rings-1) — no degenerate poles
    for r in 1..rings {
        let phi = pi * r as f32 / rings as f32;
        let sp = gl::sin(phi);
        let cp = gl::cos(phi);

        for s in 0..sectors {
            let theta = 2.0 * pi * s as f32 / sectors as f32;
            let x = sp * gl::cos(theta);
            let y = cp;
            let z = sp * gl::sin(theta);

            let u = s as f32 / sectors as f32;
            let v = r as f32 / rings as f32;

            // Normal = position for unit sphere
            verts.extend_from_slice(&[x, y, z, x, y, z, u, v]);
        }
    }

    // Last vertex: south pole
    let south_idx = (1 + (rings - 1) * sectors) as u16;
    verts.extend_from_slice(&[0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.5, 1.0]);

    // North pole triangle fan: pole(0) → ring 1
    for s in 0..sectors {
        let next = (s + 1) % sectors;
        indices.extend_from_slice(&[0, 1 + s as u16, 1 + next as u16]);
    }

    // Middle quad strips: ring r → ring r+1
    for r in 0..(rings - 2) {
        let base = 1 + r * sectors;
        let next_base = base + sectors;
        for s in 0..sectors {
            let s1 = (s + 1) % sectors;
            let i0 = (base + s) as u16;
            let i1 = (base + s1) as u16;
            let i2 = (next_base + s) as u16;
            let i3 = (next_base + s1) as u16;

            indices.extend_from_slice(&[i0, i1, i2]);
            indices.extend_from_slice(&[i1, i3, i2]);
        }
    }

    // South pole triangle fan: last ring → pole
    let last_ring_base = 1 + (rings - 2) * sectors;
    for s in 0..sectors {
        let next = (s + 1) % sectors;
        indices.extend_from_slice(&[
            (last_ring_base + s) as u16,
            (last_ring_base + next) as u16,
            south_idx,
        ]);
    }

    (verts, indices)
}

/// Generate a cube with per-face normals.
/// Returns (vertex_data, index_data).
fn generate_cube() -> (Vec<f32>, Vec<u16>) {
    // 6 faces × 4 vertices = 24 vertices
    #[rustfmt::skip]
    let verts: Vec<f32> = vec![
        // Front face (z=+0.5, normal 0,0,1)
        -0.5, -0.5,  0.5,  0.0,  0.0,  1.0,  0.0, 0.0,
         0.5, -0.5,  0.5,  0.0,  0.0,  1.0,  1.0, 0.0,
         0.5,  0.5,  0.5,  0.0,  0.0,  1.0,  1.0, 1.0,
        -0.5,  0.5,  0.5,  0.0,  0.0,  1.0,  0.0, 1.0,
        // Back face (z=-0.5, normal 0,0,-1)
         0.5, -0.5, -0.5,  0.0,  0.0, -1.0,  0.0, 0.0,
        -0.5, -0.5, -0.5,  0.0,  0.0, -1.0,  1.0, 0.0,
        -0.5,  0.5, -0.5,  0.0,  0.0, -1.0,  1.0, 1.0,
         0.5,  0.5, -0.5,  0.0,  0.0, -1.0,  0.0, 1.0,
        // Right face (x=+0.5, normal 1,0,0)
         0.5, -0.5,  0.5,  1.0,  0.0,  0.0,  0.0, 0.0,
         0.5, -0.5, -0.5,  1.0,  0.0,  0.0,  1.0, 0.0,
         0.5,  0.5, -0.5,  1.0,  0.0,  0.0,  1.0, 1.0,
         0.5,  0.5,  0.5,  1.0,  0.0,  0.0,  0.0, 1.0,
        // Left face (x=-0.5, normal -1,0,0)
        -0.5, -0.5, -0.5, -1.0,  0.0,  0.0,  0.0, 0.0,
        -0.5, -0.5,  0.5, -1.0,  0.0,  0.0,  1.0, 0.0,
        -0.5,  0.5,  0.5, -1.0,  0.0,  0.0,  1.0, 1.0,
        -0.5,  0.5, -0.5, -1.0,  0.0,  0.0,  0.0, 1.0,
        // Top face (y=+0.5, normal 0,1,0)
        -0.5,  0.5,  0.5,  0.0,  1.0,  0.0,  0.0, 0.0,
         0.5,  0.5,  0.5,  0.0,  1.0,  0.0,  1.0, 0.0,
         0.5,  0.5, -0.5,  0.0,  1.0,  0.0,  1.0, 1.0,
        -0.5,  0.5, -0.5,  0.0,  1.0,  0.0,  0.0, 1.0,
        // Bottom face (y=-0.5, normal 0,-1,0)
        -0.5, -0.5, -0.5,  0.0, -1.0,  0.0,  0.0, 0.0,
         0.5, -0.5, -0.5,  0.0, -1.0,  0.0,  1.0, 0.0,
         0.5, -0.5,  0.5,  0.0, -1.0,  0.0,  1.0, 1.0,
        -0.5, -0.5,  0.5,  0.0, -1.0,  0.0,  0.0, 1.0,
    ];

    // 6 faces × 2 triangles × 3 indices = 36 indices
    let mut indices = Vec::with_capacity(36);
    for face in 0..6u16 {
        let base = face * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2]);
        indices.extend_from_slice(&[base, base + 2, base + 3]);
    }

    (verts, indices)
}

/// Generate a 64×64 checkerboard texture (RGBA8).
fn generate_checkerboard(size: u32, check_size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let cx = x / check_size;
            let cy = y / check_size;
            let white = (cx + cy) % 2 == 0;
            if white {
                data.extend_from_slice(&[220, 220, 230, 255]);
            } else {
                data.extend_from_slice(&[50, 50, 60, 255]);
            }
        }
    }
    data
}

/// Generate the classic Amiga Boing Ball texture (red & white checkerboard on sphere UV).
/// The pattern has `cols` longitudinal and `rows` latitudinal tiles.
fn generate_boing_texture(size: u32, rows: u32, cols: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / size as f32;
            let col = (u * cols as f32) as u32;
            let row = (v * rows as f32) as u32;
            let is_red = (col + row) % 2 == 0;
            if is_red {
                data.extend_from_slice(&[220, 20, 20, 255]); // red
            } else {
                data.extend_from_slice(&[255, 255, 255, 255]); // white
            }
        }
    }
    data
}

/// Generate a gradient texture for the sphere (RGBA8).
fn generate_gradient(size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / size as f32;
            // Warm gradient: gold-ish tones
            let r = (200.0 + 55.0 * u) as u8;
            let g = (160.0 + 80.0 * v) as u8;
            let b = (100.0 + 40.0 * u * v) as u8;
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    data
}

/// 5×7 pixel bitmap font for uppercase + digits + dot + space.
/// Each glyph is 5 columns × 7 rows, stored as 7 bytes (one per row, lower 5 bits = pixels).
fn glyph_data(ch: u8) -> [u8; 7] {
    match ch {
        b'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'B' => [0b11110, 0b10001, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110],
        b'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        b'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        b'E' => [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b11111],
        b'F' => [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0b10000],
        b'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        b'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        b'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        b'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        b'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        b'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        b'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        b'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        b'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        b'S' => [0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110],
        b'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        b'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        b'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        b'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        b'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        b'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        b'a' => [0b00000, 0b00000, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111],
        b'n' => [0b00000, 0b00000, 0b10110, 0b11001, 0b10001, 0b10001, 0b10001],
        b'y' => [0b00000, 0b00000, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110],
        b'0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        b'1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        b'3' => [0b01110, 0b10001, 0b00001, 0b00110, 0b00001, 0b10001, 0b01110],
        b'4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        b'5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        b'6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        b'7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        b'8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        b'9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        b'.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100],
        b' ' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000],
        _    => [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111], // box
    }
}

/// Render a text string into an RGBA pixel buffer at (ox, oy) with given scale and color.
fn draw_text_into(data: &mut [u8], tex_w: u32, text: &[u8], ox: u32, oy: u32, scale: u32, r: u8, g: u8, b: u8) {
    let glyph_w = 5;
    let glyph_h = 7;
    let spacing = 1; // 1 pixel gap between glyphs
    for (ci, &ch) in text.iter().enumerate() {
        let glyph = glyph_data(ch);
        let base_x = ox + ci as u32 * (glyph_w + spacing) * scale;
        for row in 0..glyph_h {
            for col in 0..glyph_w {
                if glyph[row as usize] & (1 << (glyph_w - 1 - col)) != 0 {
                    // Draw a scale×scale block
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = base_x + col * scale + sx;
                            let py = oy + row * scale + sy;
                            if px < tex_w && py < tex_w {
                                let idx = ((py * tex_w + px) * 4) as usize;
                                if idx + 3 < data.len() {
                                    data[idx] = r;
                                    data[idx + 1] = g;
                                    data[idx + 2] = b;
                                    data[idx + 3] = 255;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Generate a floor texture with checkerboard and "anyOS" logo + version.
fn generate_floor_texture(size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let check_size = size / 8; // 8×8 checkerboard grid

    // Base checkerboard
    for y in 0..size {
        for x in 0..size {
            let cx = x / check_size;
            let cy = y / check_size;
            let white = (cx + cy) % 2 == 0;
            if white {
                data.extend_from_slice(&[180, 185, 195, 255]);
            } else {
                data.extend_from_slice(&[60, 65, 80, 255]);
            }
        }
    }

    // "anyOS" in large text, centered
    let text = b"anyOS";
    let scale = size / 64 * 3; // scale factor based on texture size
    let glyph_w_total = text.len() as u32 * 6 * scale; // (5+1) pixels per glyph
    let glyph_h_total = 7 * scale;
    let tx = (size - glyph_w_total) / 2;
    let ty = size / 2 - glyph_h_total - scale;
    draw_text_into(&mut data, size, text, tx, ty, scale, 40, 140, 255);

    // Version below
    let ver = b"0.3.134";
    let ver_scale = scale * 2 / 3;
    let ver_scale = if ver_scale < 1 { 1 } else { ver_scale };
    let ver_w = ver.len() as u32 * 6 * ver_scale;
    let vx = (size - ver_w) / 2;
    let vy = ty + glyph_h_total + scale;
    draw_text_into(&mut data, size, ver, vx, vy, ver_scale, 30, 110, 200);

    data
}

/// Format a u32 into a decimal string. Returns number of bytes written.
fn fmt_u32(val: u32, buf: &mut [u8; 16]) -> usize {
    let mut tmp = [0u8; 12];
    let s = anyos_std::fmt::fmt_u32(&mut tmp, val);
    let b = s.as_bytes();
    buf[..b.len()].copy_from_slice(b);
    b.len()
}

// ── Render state ─────────────────────────────────────────────────────────────

struct RenderState {
    canvas: libanyui_client::Canvas,
    fb_w: u32,
    fb_h: u32,
    main_program: u32,
    shadow_program: u32,
    // Sphere
    sphere_vbo: u32,
    sphere_ebo: u32,
    sphere_num_indices: i32,
    // Cube
    cube_vbo: u32,
    cube_ebo: u32,
    cube_num_indices: i32,
    // Floor
    floor_vbo: u32,
    // Textures
    checker_tex: u32,
    gradient_tex: u32,
    boing_tex: u32,
    floor_tex: u32,
    // Uniform locations
    loc_main_mvp: i32,
    loc_main_model: i32,
    loc_main_light_pos0: i32,
    loc_main_light_color0: i32,
    loc_main_eye_pos: i32,
    loc_main_texture: i32,
    loc_main_mat_color: i32,
    loc_main_shadow_map: i32,
    loc_main_shadow_texel_size: i32,
    loc_main_shadow_flip_y: i32,
    loc_main_light_mvp: i32,
    loc_shadow_model: i32,
    loc_shadow_light_mvp: i32,
    // Physics body IDs
    phys_sphere: u32,
    phys_cube: u32,
    phys_boing: u32,
    phys_wall_l: u32,
    phys_wall_r: u32,
    phys_wall_f: u32,
    phys_wall_b: u32,
    boing_active: bool,
    // Camera
    camera_dist: f32,
    camera_yaw: f32,
    camera_pitch: f32,
    // Mouse drag state
    drag_button: u32,   // 0=none, 1=left(orbit), 2=right(zoom)
    drag_last_x: i32,
    drag_last_y: i32,
    // Animation
    frame: u32,
    // FPS counter
    fps_label: libanyui_client::Label,
    fps_frame_count: u32,
    fps_last_ms: u32,
    fps_display: u32,
    // Fullscreen state
    fullscreen: bool,
    shadows_enabled: bool,
}

static mut STATE: Option<RenderState> = None;

fn render_frame() {
    let s = unsafe { STATE.as_mut().unwrap() };
    s.frame += 1;

    // ── FPS counter ─────────────────────────────────────────────────────
    s.fps_frame_count += 1;
    let now_ms = anyos_std::sys::uptime_ms();
    let elapsed = now_ms.wrapping_sub(s.fps_last_ms);
    if elapsed >= 1000 {
        s.fps_display = s.fps_frame_count * 1000 / elapsed;
        s.fps_frame_count = 0;
        s.fps_last_ms = now_ms;
        let mut buf = [0u8; 16];
        let n = fmt_u32(s.fps_display, &mut buf);
        let mut fps_str = [0u8; 20];
        fps_str[..n].copy_from_slice(&buf[..n]);
        fps_str[n] = b' ';
        fps_str[n + 1] = b'F';
        fps_str[n + 2] = b'P';
        fps_str[n + 3] = b'S';
        if let Ok(text) = core::str::from_utf8(&fps_str[..n + 4]) {
            s.fps_label.set_text(text);
        }
    }

    // Dynamic resize
    let cur_w = s.canvas.get_stride();
    let cur_h = s.canvas.get_height();
    if cur_w == 0 || cur_h == 0 { return; }
    if cur_w != s.fb_w || cur_h != s.fb_h {
        s.fb_w = cur_w;
        s.fb_h = cur_h;
        gl::gl_resize(cur_w, cur_h);
        gl::viewport(0, 0, cur_w as i32, cur_h as i32);
    }

    let t = s.frame as f32 * 0.02;

    // ── Update wall boundaries based on camera distance ─────────────────
    let wall_d = -(s.camera_dist * 0.55);
    gl::physics_set_plane_d(s.phys_wall_l, wall_d);
    gl::physics_set_plane_d(s.phys_wall_r, wall_d);
    gl::physics_set_plane_d(s.phys_wall_f, wall_d);
    gl::physics_set_plane_d(s.phys_wall_b, wall_d);

    // ── Step physics ────────────────────────────────────────────────────
    gl::physics_step(0.016); // ~60fps timestep

    // ── Setup ────────────────────────────────────────────────────────────
    gl::clear_color(0.05, 0.05, 0.12, 1.0);
    gl::clear(gl::GL_COLOR_BUFFER_BIT | gl::GL_DEPTH_BUFFER_BIT);

    let d = s.camera_dist;
    let eye = [
        d * gl::sin(s.camera_yaw) * gl::cos(s.camera_pitch),
        d * gl::sin(s.camera_pitch),
        d * gl::cos(s.camera_yaw) * gl::cos(s.camera_pitch),
    ];
    let aspect = s.fb_w as f32 / s.fb_h as f32;
    let proj = mat4_perspective(0.9, aspect, 0.1, 50.0);
    // Look-at: camera at `eye`, looking at origin (0,0,0), up = (0,1,0)
    let view = mat4_look_at(&eye, &[0.0, 0.0, 0.0], &[0.0, 1.0, 0.0]);

    // Animated light
    let l0_x = gl::sin(t * 0.7) * 3.0;
    let l0_z = gl::cos(t * 0.7) * 3.0;
    let l0_y = 2.0f32;

    if s.shadows_enabled {
        let shadow_radius = (s.camera_dist * 1.8).max(4.0);
        let ran_shadow = gl::shadow_pass_begin(l0_x, l0_y, l0_z, 0.0, 0.0, 0.0, shadow_radius);
        if ran_shadow {
            let light_mvp_ptr = gl::shadow_get_light_mvp();
            if !light_mvp_ptr.is_null() {
                let light_mvp = unsafe { *(light_mvp_ptr as *const [f32; 16]) };
                gl::use_program(s.shadow_program);
                gl::color_mask(false, false, false, false);
                gl::clear(gl::GL_DEPTH_BUFFER_BIT);
                draw_scene_geometry_shadow(s, &light_mvp);
                gl::color_mask(true, true, true, true);
            }
            gl::shadow_pass_end();
        }
    }

    // ── Main pass ──────────────────────────────────────────────────────
    let camera_vp = mat4_mul(&proj, &view);
    let shadows_ready = s.shadows_enabled && gl::shadow_available();
    let light_mvp = if shadows_ready {
        let ptr = gl::shadow_get_light_mvp();
        if !ptr.is_null() {
            unsafe { *(ptr as *const [f32; 16]) }
        } else {
            mat4_identity()
        }
    } else {
        mat4_identity()
    };

    gl::use_program(s.main_program);
    gl::uniform3f(s.loc_main_light_pos0, l0_x, l0_y, l0_z);
    gl::uniform3f(s.loc_main_light_color0, 1.0, 0.95, 0.8);
    gl::uniform3f(s.loc_main_eye_pos, eye[0], eye[1], eye[2]);
    gl::uniform_matrix4fv(s.loc_main_light_mvp, false, &light_mvp);
    let shadow_map_size = if shadows_ready {
        gl::shadow_get_map_size().max(1) as f32
    } else {
        1.0
    };
    gl::uniform2f(
        s.loc_main_shadow_texel_size,
        1.0 / shadow_map_size,
        1.0 / shadow_map_size,
    );
    gl::uniform1f(
        s.loc_main_shadow_flip_y,
        if gl::get_hw_backend() { 0.0 } else { 1.0 },
    );
    gl::active_texture(gl::GL_TEXTURE0 + gl::shadow_get_unit());
    gl::bind_texture(
        gl::GL_TEXTURE_2D,
        if shadows_ready { gl::shadow_get_texture() } else { 0 },
    );
    gl::uniform1i(s.loc_main_shadow_map, gl::shadow_get_unit() as i32);

    draw_scene_geometry_main(s, &camera_vp);

    render_frame_finish();
}

fn draw_scene_geometry_main(s: &mut RenderState, vp: &Mat4) {
    let d = s.camera_dist;

    // ── Draw sphere (physics-driven with quaternion) ────────────────────
    {
        let (px, py, pz) = gl::physics_get_position(s.phys_sphere);
        let (qw, qx, qy, qz) = gl::physics_get_orientation(s.phys_sphere);
        let rot_mat = quat_to_mat4(qw, qx, qy, qz);
        let model = mat4_mul(
            &mat4_translate(px, py, pz),
            &mat4_mul(&rot_mat, &soft_body_scale(s.phys_sphere, 0.8)),
        );
        let mvp = mat4_mul(vp, &model);

        gl::uniform_matrix4fv(s.loc_main_mvp, false, &mvp);
        gl::uniform_matrix4fv(s.loc_main_model, false, &model);
        gl::uniform4f(s.loc_main_mat_color, 1.0, 1.0, 1.0, 1.0);

        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, s.gradient_tex);
        gl::uniform1i(s.loc_main_texture, 0);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.sphere_vbo);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, s.sphere_ebo);
        setup_vertex_attribs(s.main_program);
        gl::draw_elements(gl::GL_TRIANGLES, s.sphere_num_indices, gl::GL_UNSIGNED_SHORT, 0);
    }

    // ── Draw cube (physics-driven with quaternion orientation) ─────────
    {
        let (px, py, pz) = gl::physics_get_position(s.phys_cube);
        let (qw, qx, qy, qz) = gl::physics_get_orientation(s.phys_cube);
        let rot_mat = quat_to_mat4(qw, qx, qy, qz);
        let model = mat4_mul(
            &mat4_translate(px, py, pz),
            &mat4_mul(&rot_mat, &mat4_scale(0.9, 0.9, 0.9)),
        );
        let mvp = mat4_mul(vp, &model);

        gl::uniform_matrix4fv(s.loc_main_mvp, false, &mvp);
        gl::uniform_matrix4fv(s.loc_main_model, false, &model);
        gl::uniform4f(s.loc_main_mat_color, 1.0, 1.0, 1.0, 1.0);

        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, s.checker_tex);
        gl::uniform1i(s.loc_main_texture, 0);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.cube_vbo);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, s.cube_ebo);
        setup_vertex_attribs(s.main_program);
        gl::draw_elements(gl::GL_TRIANGLES, s.cube_num_indices, gl::GL_UNSIGNED_SHORT, 0);
    }

    // ── Draw Amiga Boing Ball (physics-driven with quaternion) ──────────
    if s.boing_active {
        let (px, py, pz) = gl::physics_get_position(s.phys_boing);
        let (qw, qx, qy, qz) = gl::physics_get_orientation(s.phys_boing);
        let rot_mat = quat_to_mat4(qw, qx, qy, qz);
        let model = mat4_mul(
            &mat4_translate(px, py, pz),
            &mat4_mul(&rot_mat, &soft_body_scale(s.phys_boing, 0.6)),
        );
        let mvp = mat4_mul(vp, &model);

        gl::uniform_matrix4fv(s.loc_main_mvp, false, &mvp);
        gl::uniform_matrix4fv(s.loc_main_model, false, &model);
        gl::uniform4f(s.loc_main_mat_color, 1.0, 1.0, 1.0, 1.0);

        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, s.boing_tex);
        gl::uniform1i(s.loc_main_texture, 0);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.sphere_vbo);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, s.sphere_ebo);
        setup_vertex_attribs(s.main_program);
        gl::draw_elements(gl::GL_TRIANGLES, s.sphere_num_indices, gl::GL_UNSIGNED_SHORT, 0);
    }

    // ── Draw floor plane ───────────────────────────────────────────────
    {
        gl::disable(gl::GL_CULL_FACE); // floor visible from both sides

        let floor_scale = d * 1.2 + 2.0;
        let model = mat4_mul(
            &mat4_translate(0.0, -0.5, 0.0),
            &mat4_scale(floor_scale, 1.0, floor_scale),
        );
        let mvp = mat4_mul(vp, &model);

        gl::uniform_matrix4fv(s.loc_main_mvp, false, &mvp);
        gl::uniform_matrix4fv(s.loc_main_model, false, &model);
        gl::uniform4f(s.loc_main_mat_color, 1.0, 1.0, 1.0, 1.0);

        gl::active_texture(gl::GL_TEXTURE0);
        gl::bind_texture(gl::GL_TEXTURE_2D, s.floor_tex);
        gl::uniform1i(s.loc_main_texture, 0);

        gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.floor_vbo);
        setup_vertex_attribs(s.main_program);
        gl::draw_arrays(gl::GL_TRIANGLES, 0, 6);

        gl::enable(gl::GL_CULL_FACE);
    }
}

fn draw_scene_geometry_shadow(s: &mut RenderState, light_vp: &Mat4) {
    {
        let (px, py, pz) = gl::physics_get_position(s.phys_sphere);
        let (qw, qx, qy, qz) = gl::physics_get_orientation(s.phys_sphere);
        let rot_mat = quat_to_mat4(qw, qx, qy, qz);
        let model = mat4_mul(
            &mat4_translate(px, py, pz),
            &mat4_mul(&rot_mat, &soft_body_scale(s.phys_sphere, 0.8)),
        );
        gl::uniform_matrix4fv(s.loc_shadow_model, false, &model);
        gl::uniform_matrix4fv(s.loc_shadow_light_mvp, false, light_vp);
        gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.sphere_vbo);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, s.sphere_ebo);
        setup_vertex_attribs(s.shadow_program);
        gl::draw_elements(gl::GL_TRIANGLES, s.sphere_num_indices, gl::GL_UNSIGNED_SHORT, 0);
    }

    {
        let (px, py, pz) = gl::physics_get_position(s.phys_cube);
        let (qw, qx, qy, qz) = gl::physics_get_orientation(s.phys_cube);
        let rot_mat = quat_to_mat4(qw, qx, qy, qz);
        let model = mat4_mul(
            &mat4_translate(px, py, pz),
            &mat4_mul(&rot_mat, &mat4_scale(0.9, 0.9, 0.9)),
        );
        gl::uniform_matrix4fv(s.loc_shadow_model, false, &model);
        gl::uniform_matrix4fv(s.loc_shadow_light_mvp, false, light_vp);
        gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.cube_vbo);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, s.cube_ebo);
        setup_vertex_attribs(s.shadow_program);
        gl::draw_elements(gl::GL_TRIANGLES, s.cube_num_indices, gl::GL_UNSIGNED_SHORT, 0);
    }

    if s.boing_active {
        let (px, py, pz) = gl::physics_get_position(s.phys_boing);
        let (qw, qx, qy, qz) = gl::physics_get_orientation(s.phys_boing);
        let rot_mat = quat_to_mat4(qw, qx, qy, qz);
        let model = mat4_mul(
            &mat4_translate(px, py, pz),
            &mat4_mul(&rot_mat, &soft_body_scale(s.phys_boing, 0.6)),
        );
        gl::uniform_matrix4fv(s.loc_shadow_model, false, &model);
        gl::uniform_matrix4fv(s.loc_shadow_light_mvp, false, light_vp);
        gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.sphere_vbo);
        gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, s.sphere_ebo);
        setup_vertex_attribs(s.shadow_program);
        gl::draw_elements(gl::GL_TRIANGLES, s.sphere_num_indices, gl::GL_UNSIGNED_SHORT, 0);
    }

    gl::disable(gl::GL_CULL_FACE);
    let floor_scale = s.camera_dist * 1.2 + 2.0;
    let model = mat4_mul(
        &mat4_translate(0.0, -0.5, 0.0),
        &mat4_scale(floor_scale, 1.0, floor_scale),
    );
    gl::uniform_matrix4fv(s.loc_shadow_model, false, &model);
    gl::uniform_matrix4fv(s.loc_shadow_light_mvp, false, light_vp);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, s.floor_vbo);
    setup_vertex_attribs(s.shadow_program);
    gl::draw_arrays(gl::GL_TRIANGLES, 0, 6);
    gl::enable(gl::GL_CULL_FACE);
}

fn render_frame_finish() {
    let s = unsafe { STATE.as_mut().unwrap() };

    // ── Swap to canvas or direct framebuffer ────────────────────────────
    if s.fullscreen {
        // Fullscreen mode: copy directly to mapped framebuffer
        gl::swap_buffers_fullscreen();
    } else {
        // Windowed mode: copy to anyui canvas
        let fb_ptr = gl::swap_buffers();
        if !fb_ptr.is_null() {
            let pixels = unsafe {
                core::slice::from_raw_parts(fb_ptr, (s.fb_w * s.fb_h) as usize)
            };
            s.canvas.copy_pixels_from(pixels);
        }
    }
}

/// Configure vertex attribute pointers for the interleaved format:
/// position(3) + normal(3) + texcoord(2), stride=32 bytes.
fn setup_vertex_attribs(program: u32) {
    let loc_pos = gl::get_attrib_location(program, "aPosition");
    let loc_norm = gl::get_attrib_location(program, "aNormal");
    let loc_uv = gl::get_attrib_location(program, "aTexCoord");

    if loc_pos >= 0 {
        gl::enable_vertex_attrib_array(loc_pos as u32);
        gl::vertex_attrib_pointer(loc_pos as u32, 3, gl::GL_FLOAT, false, 32, 0);
    }
    if loc_norm >= 0 {
        gl::enable_vertex_attrib_array(loc_norm as u32);
        gl::vertex_attrib_pointer(loc_norm as u32, 3, gl::GL_FLOAT, false, 32, 12);
    }
    if loc_uv >= 0 {
        gl::enable_vertex_attrib_array(loc_uv as u32);
        gl::vertex_attrib_pointer(loc_uv as u32, 2, gl::GL_FLOAT, false, 32, 24);
    }
}

fn main() {
    anyos_std::println!("gldemo: starting Phong shading demo");

    // Initialize anyui
    libanyui_client::init();
    anyos_std::i18n::init();
    let window = libanyui_client::Window::new(anyos_std::i18n::t("GL Demo - Phong"), 80, 60, 420, 460);

    // Canvas fills the entire window client area
    let canvas = libanyui_client::Canvas::new(400, 400);
    canvas.set_dock(libanyui_client::DOCK_FILL);
    window.add(&canvas);
    window.set_visible(true);

    let fb_w = canvas.get_stride();
    let fb_h = canvas.get_height();
    anyos_std::println!("gldemo: canvas {}x{}", fb_w, fb_h);

    // Initialize libgl
    if !gl::init() {
        anyos_std::println!("gldemo: failed to load libgl.so");
        return;
    }

    // Use actual canvas size (may differ from initial 400x400 due to dock)
    let fb_w = if fb_w > 0 { fb_w } else { 400 };
    let fb_h = if fb_h > 0 { fb_h } else { 400 };

    gl::gl_init(fb_w, fb_h);
    gl::viewport(0, 0, fb_w as i32, fb_h as i32);
    gl::enable(gl::GL_DEPTH_TEST);
    gl::depth_func(gl::GL_LESS);
    gl::enable(gl::GL_CULL_FACE);
    gl::cull_face(gl::GL_BACK);
    gl::set_fxaa(false);

    // HW/SW toggle (overlaid on canvas)
    let hw_available = gl::has_hw_backend();
    let hw_label = libanyui_client::Label::new("HW");
    hw_label.set_position(10, 6);
    hw_label.set_text_color(0xFFCCCCCC);
    hw_label.set_font_size(13);
    window.add(&hw_label);

    let hw_toggle = libanyui_client::Toggle::new(hw_available);
    hw_toggle.set_position(40, 4);
    hw_toggle.on_checked_changed(|e| { gl::set_hw_backend(e.checked); });
    window.add(&hw_toggle);

    let shadow_label = libanyui_client::Label::new("Shadow");
    shadow_label.set_position(80, 6);
    shadow_label.set_text_color(0xFFCCCCCC);
    shadow_label.set_font_size(13);
    window.add(&shadow_label);

    let shadow_toggle = libanyui_client::Toggle::new(true);
    shadow_toggle.set_position(132, 4);
    shadow_toggle.on_checked_changed(|e| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.shadows_enabled = e.checked;
    });
    window.add(&shadow_toggle);

    // ── Compile shaders ──────────────────────────────────────────────────
    let main_vs = gl::create_shader(gl::GL_VERTEX_SHADER);
    gl::shader_source(main_vs, VS_SOURCE);
    gl::compile_shader(main_vs);
    if !gl::get_shader_compile_status(main_vs) {
        anyos_std::println!("gldemo: main VS compile FAILED");
        return;
    }

    let main_fs = gl::create_shader(gl::GL_FRAGMENT_SHADER);
    gl::shader_source(main_fs, FS_SOURCE);
    gl::compile_shader(main_fs);
    if !gl::get_shader_compile_status(main_fs) {
        anyos_std::println!("gldemo: main FS compile FAILED");
        return;
    }

    let main_program = gl::create_program();
    gl::attach_shader(main_program, main_vs);
    gl::attach_shader(main_program, main_fs);
    gl::link_program(main_program);
    if !gl::get_program_link_status(main_program) {
        anyos_std::println!("gldemo: main link FAILED");
        return;
    }
    let shadow_vs = gl::create_shader(gl::GL_VERTEX_SHADER);
    gl::shader_source(shadow_vs, SHADOW_VS_SOURCE);
    gl::compile_shader(shadow_vs);
    if !gl::get_shader_compile_status(shadow_vs) {
        anyos_std::println!("gldemo: shadow VS compile FAILED");
        return;
    }

    let shadow_fs = gl::create_shader(gl::GL_FRAGMENT_SHADER);
    gl::shader_source(shadow_fs, SHADOW_FS_SOURCE);
    gl::compile_shader(shadow_fs);
    if !gl::get_shader_compile_status(shadow_fs) {
        anyos_std::println!("gldemo: shadow FS compile FAILED");
        return;
    }

    let shadow_program = gl::create_program();
    gl::attach_shader(shadow_program, shadow_vs);
    gl::attach_shader(shadow_program, shadow_fs);
    gl::link_program(shadow_program);
    if !gl::get_program_link_status(shadow_program) {
        anyos_std::println!("gldemo: shadow link FAILED");
        return;
    }
    gl::use_program(main_program);
    anyos_std::println!("gldemo: shaders compiled OK");

    // ── Query uniform locations ──────────────────────────────────────────
    let loc_main_mvp = gl::get_uniform_location(main_program, "uMVP");
    let loc_main_model = gl::get_uniform_location(main_program, "uModel");
    let loc_main_light_pos0 = gl::get_uniform_location(main_program, "uLightPos0");
    let loc_main_light_color0 = gl::get_uniform_location(main_program, "uLightColor0");
    let loc_main_eye_pos = gl::get_uniform_location(main_program, "uEyePos");
    let loc_main_texture = gl::get_uniform_location(main_program, "uTexture");
    let loc_main_mat_color = gl::get_uniform_location(main_program, "uMatColor");
    let loc_main_shadow_map = gl::get_uniform_location(main_program, "uShadowMap");
    let loc_main_shadow_texel_size = gl::get_uniform_location(main_program, "uShadowTexelSize");
    let loc_main_shadow_flip_y = gl::get_uniform_location(main_program, "uShadowFlipY");
    let loc_main_light_mvp = gl::get_uniform_location(main_program, "uLightMVP");
    let loc_shadow_model = gl::get_uniform_location(shadow_program, "uModel");
    let loc_shadow_light_mvp = gl::get_uniform_location(shadow_program, "uLightMVP");

    anyos_std::println!("gldemo: uniforms: mvp={} model={} lp0={} eye={} tex={} mat={} shadow={} lmvp={}",
        loc_main_mvp, loc_main_model, loc_main_light_pos0, loc_main_eye_pos, loc_main_texture, loc_main_mat_color, loc_main_shadow_map, loc_main_light_mvp);

    // ── Generate sphere geometry ─────────────────────────────────────────
    let (sphere_verts, sphere_indices) = generate_sphere(24, 32);
    let sphere_num_indices = sphere_indices.len() as i32;

    let mut sphere_vbo = [0u32; 1];
    gl::gen_buffers(1, &mut sphere_vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, sphere_vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &sphere_verts, gl::GL_STATIC_DRAW);

    let mut sphere_ebo = [0u32; 1];
    gl::gen_buffers(1, &mut sphere_ebo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, sphere_ebo[0]);
    gl::buffer_data_u16(gl::GL_ELEMENT_ARRAY_BUFFER, &sphere_indices, gl::GL_STATIC_DRAW);

    anyos_std::println!("gldemo: sphere: {} verts, {} indices",
        sphere_verts.len() / VERTEX_STRIDE, sphere_num_indices);

    // ── Generate cube geometry ───────────────────────────────────────────
    let (cube_verts, cube_indices) = generate_cube();
    let cube_num_indices = cube_indices.len() as i32;

    let mut cube_vbo = [0u32; 1];
    gl::gen_buffers(1, &mut cube_vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, cube_vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &cube_verts, gl::GL_STATIC_DRAW);

    let mut cube_ebo = [0u32; 1];
    gl::gen_buffers(1, &mut cube_ebo);
    gl::bind_buffer(gl::GL_ELEMENT_ARRAY_BUFFER, cube_ebo[0]);
    gl::buffer_data_u16(gl::GL_ELEMENT_ARRAY_BUFFER, &cube_indices, gl::GL_STATIC_DRAW);

    anyos_std::println!("gldemo: cube: {} verts, {} indices",
        cube_verts.len() / VERTEX_STRIDE, cube_num_indices);

    // ── Generate floor geometry (static VBO, created once) ──────────────
    #[rustfmt::skip]
    let floor_data: [f32; 48] = [
        // pos          normal       uv
        -0.5, 0.0, -0.5,  0.0, 1.0, 0.0,  0.0, 0.0,
         0.5, 0.0, -0.5,  0.0, 1.0, 0.0,  1.0, 0.0,
         0.5, 0.0,  0.5,  0.0, 1.0, 0.0,  1.0, 1.0,
        -0.5, 0.0, -0.5,  0.0, 1.0, 0.0,  0.0, 0.0,
         0.5, 0.0,  0.5,  0.0, 1.0, 0.0,  1.0, 1.0,
        -0.5, 0.0,  0.5,  0.0, 1.0, 0.0,  0.0, 1.0,
    ];
    let mut floor_vbo = [0u32; 1];
    gl::gen_buffers(1, &mut floor_vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, floor_vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &floor_data, gl::GL_STATIC_DRAW);

    // ── Generate textures ────────────────────────────────────────────────
    let checker_data = generate_checkerboard(64, 8);
    let mut checker_tex = [0u32; 1];
    gl::gen_textures(1, &mut checker_tex);
    gl::bind_texture(gl::GL_TEXTURE_2D, checker_tex[0]);
    gl::tex_image_2d(gl::GL_TEXTURE_2D, 0, gl::GL_RGBA as i32, 64, 64, 0,
        gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, &checker_data);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST as i32);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_WRAP_S, gl::GL_REPEAT as i32);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_WRAP_T, gl::GL_REPEAT as i32);

    let gradient_data = generate_gradient(64);
    let mut gradient_tex = [0u32; 1];
    gl::gen_textures(1, &mut gradient_tex);
    gl::bind_texture(gl::GL_TEXTURE_2D, gradient_tex[0]);
    gl::tex_image_2d(gl::GL_TEXTURE_2D, 0, gl::GL_RGBA as i32, 64, 64, 0,
        gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, &gradient_data);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_LINEAR as i32);

    // Boing ball texture (red/white checkerboard, 8 cols × 8 rows like the Amiga original)
    let boing_data = generate_boing_texture(64, 8, 8);
    let mut boing_tex = [0u32; 1];
    gl::gen_textures(1, &mut boing_tex);
    gl::bind_texture(gl::GL_TEXTURE_2D, boing_tex[0]);
    gl::tex_image_2d(gl::GL_TEXTURE_2D, 0, gl::GL_RGBA as i32, 64, 64, 0,
        gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, &boing_data);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST as i32);

    // Floor texture with checkerboard + "anyOS" logo
    let floor_tex_size = 256;
    let floor_data = generate_floor_texture(floor_tex_size);
    let mut floor_tex = [0u32; 1];
    gl::gen_textures(1, &mut floor_tex);
    gl::bind_texture(gl::GL_TEXTURE_2D, floor_tex[0]);
    gl::tex_image_2d(gl::GL_TEXTURE_2D, 0, gl::GL_RGBA as i32, floor_tex_size as i32, floor_tex_size as i32, 0,
        gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, &floor_data);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_LINEAR as i32);
    gl::tex_parameteri(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MIN_FILTER, gl::GL_LINEAR as i32);

    anyos_std::println!("gldemo: textures created");

    // ── FPS counter label (top-right area) ──────────────────────────────
    let fps_label = libanyui_client::Label::new("-- FPS");
    fps_label.set_position(172, 6);
    fps_label.set_text_color(0xFF00FF88);
    fps_label.set_font_size(13);
    window.add(&fps_label);

    // ── Initialize physics world ────────────────────────────────────────
    gl::physics_create_world();
    gl::physics_set_gravity(0.0, -9.81, 0.0);

    // Floor plane: y = -0.5, normal pointing up
    let _phys_floor = gl::physics_add_plane(0.0, 1.0, 0.0, -0.5);
    gl::physics_set_restitution(_phys_floor, 0.6);

    // Wall planes (invisible boundaries, scaled with camera distance)
    let wall_d = -2.5;
    let phys_wall_l = gl::physics_add_plane(1.0, 0.0, 0.0, wall_d);  // left wall at x=-2.5
    gl::physics_set_restitution(phys_wall_l, 0.8);
    let phys_wall_r = gl::physics_add_plane(-1.0, 0.0, 0.0, wall_d); // right wall at x=2.5
    gl::physics_set_restitution(phys_wall_r, 0.8);
    let phys_wall_f = gl::physics_add_plane(0.0, 0.0, -1.0, wall_d); // front wall at z=2.5
    gl::physics_set_restitution(phys_wall_f, 0.8);
    let phys_wall_b = gl::physics_add_plane(0.0, 0.0, 1.0, wall_d);  // back wall at z=-2.5
    gl::physics_set_restitution(phys_wall_b, 0.8);

    // Gold sphere: mass=2.0 kg, radius=0.8, resting on floor (y = -0.5 + 0.8 = 0.3)
    let phys_sphere = gl::physics_add_sphere(2.0, 0.8, -1.0, 0.3, 0.0);
    gl::physics_set_restitution(phys_sphere, 0.7);
    gl::physics_set_use_gravity(phys_sphere, true);
    gl::physics_set_angular_damping(phys_sphere, 0.3);
    gl::physics_set_soft_body(phys_sphere, 0.16, 12.0, 0.12);

    // Cube: mass=1.5 kg, half-extents=0.45, starts elevated for a drop
    let phys_cube = gl::physics_add_box(1.5, 0.45, 0.45, 0.45, 1.2, 1.5, 0.0);
    gl::physics_set_restitution(phys_cube, 0.35);
    // Initial tumbling rotation and damping so it settles
    gl::physics_set_angular_velocity(phys_cube, 3.0, 1.5, 1.0);
    gl::physics_set_angular_damping(phys_cube, 0.5);
    gl::physics_set_linear_damping(phys_cube, 0.3);

    // Boing ball: mass=1.0 kg, radius=0.6, starts inactive (off-screen)
    let phys_boing = gl::physics_add_sphere(1.0, 0.6, -5.0, 0.0, 0.5);
    gl::physics_set_restitution(phys_boing, 0.92);
    gl::physics_set_angular_damping(phys_boing, 0.15);
    gl::physics_set_soft_body(phys_boing, 0.42, 8.0, 0.26);
    gl::physics_set_active(phys_boing, false);

    anyos_std::println!("gldemo: physics world created ({} bodies)", gl::physics_body_count());

    // ── Boing Ball button ─────────────────────────────────────────────────
    let boing_btn = libanyui_client::Button::new("Boing!");
    boing_btn.set_position(232, 2);
    boing_btn.set_size(60, 22);
    boing_btn.on_click(move |_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.boing_active = true;

        // Simple PRNG based on uptime for randomness
        let seed = anyos_std::sys::uptime_ms();
        let r1 = ((seed.wrapping_mul(1103515245).wrapping_add(12345)) >> 8) & 0xFFFF;
        let r2 = ((r1.wrapping_mul(1103515245).wrapping_add(12345)) >> 8) & 0xFFFF;
        let r3 = ((r2.wrapping_mul(1103515245).wrapping_add(12345)) >> 8) & 0xFFFF;
        let r4 = ((r3.wrapping_mul(1103515245).wrapping_add(12345)) >> 8) & 0xFFFF;

        // Random start edge: 0=left, 1=right, 2=back
        let edge = (r1 % 3) as i32;
        let (start_x, start_z) = match edge {
            0 => (-2.4f32, (r2 as f32 / 65535.0) * 4.0 - 2.0),  // left wall
            1 => (2.4f32, (r2 as f32 / 65535.0) * 4.0 - 2.0),   // right wall
            _ => ((r2 as f32 / 65535.0) * 4.0 - 2.0, -2.4f32),  // back wall
        };
        let start_y = 1.0 + (r3 as f32 / 65535.0) * 2.0; // height 1.0..3.0

        // Velocity aimed roughly toward center, with randomness
        let dir_x = -start_x * 0.5 + (r3 as f32 / 65535.0 - 0.5) * 1.5;
        let dir_z = -start_z * 0.5 + (r4 as f32 / 65535.0 - 0.5) * 1.5;
        let speed = 2.0 + (r4 as f32 / 65535.0) * 3.0; // speed 2.0..5.0
        let len = gl::sqrt(dir_x * dir_x + dir_z * dir_z);
        let vx = if len > 0.01 { dir_x / len * speed } else { speed };
        let vz = if len > 0.01 { dir_z / len * speed } else { 0.0 };
        let vy = (r1 as f32 / 65535.0) * 2.0; // slight upward 0..2

        gl::physics_set_active(s.phys_boing, true);
        gl::physics_set_position(s.phys_boing, start_x, start_y, start_z);
        gl::physics_set_velocity(s.phys_boing, vx, vy, vz);
        // Give the boing ball visible 3D rotation (like the classic Amiga demo)
        let spin = speed * 0.8;
        gl::physics_set_angular_velocity(s.phys_boing, spin * 0.3, spin, spin * 0.2);
    });
    window.add(&boing_btn);

    // ── Zoom buttons ──────────────────────────────────────────────────────
    let zoom_in_btn = libanyui_client::Button::new("+");
    zoom_in_btn.set_position(302, 2);
    zoom_in_btn.set_size(28, 22);
    zoom_in_btn.on_click(move |_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.camera_dist = (s.camera_dist - 0.5).max(1.5);
    });
    window.add(&zoom_in_btn);

    let zoom_out_btn = libanyui_client::Button::new("-");
    zoom_out_btn.set_position(334, 2);
    zoom_out_btn.set_size(28, 22);
    zoom_out_btn.on_click(move |_| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.camera_dist = (s.camera_dist + 0.5).min(12.0);
    });
    window.add(&zoom_out_btn);

    // ── Store render state ───────────────────────────────────────────────
    unsafe {
        STATE = Some(RenderState {
            canvas,
            fb_w,
            fb_h,
            main_program,
            shadow_program,
            sphere_vbo: sphere_vbo[0],
            sphere_ebo: sphere_ebo[0],
            sphere_num_indices,
            cube_vbo: cube_vbo[0],
            cube_ebo: cube_ebo[0],
            cube_num_indices,
            floor_vbo: floor_vbo[0],
            checker_tex: checker_tex[0],
            gradient_tex: gradient_tex[0],
            boing_tex: boing_tex[0],
            floor_tex: floor_tex[0],
            loc_main_mvp,
            loc_main_model,
            loc_main_light_pos0,
            loc_main_light_color0,
            loc_main_eye_pos,
            loc_main_texture,
            loc_main_mat_color,
            loc_main_shadow_map,
            loc_main_shadow_texel_size,
            loc_main_shadow_flip_y,
            loc_main_light_mvp,
            loc_shadow_model,
            loc_shadow_light_mvp,
            phys_sphere,
            phys_cube,
            phys_boing,
            phys_wall_l,
            phys_wall_r,
            phys_wall_f,
            phys_wall_b,
            boing_active: false,
            camera_dist: 4.5,
            camera_yaw: 0.0,
            camera_pitch: 0.38,
            drag_button: 0,
            drag_last_x: 0,
            drag_last_y: 0,
            frame: 0,
            fps_label,
            fps_frame_count: 0,
            fps_last_ms: anyos_std::sys::uptime_ms(),
            fps_display: 0,
            fullscreen: false,
            shadows_enabled: true,
        });
    }

    // ── Mouse camera controls ────────────────────────────────────────────
    // Left-drag = orbit, right-drag = zoom
    let canvas_handle = canvas;
    canvas_handle.on_mouse_down(|x, y, button| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.drag_button = button;
        s.drag_last_x = x;
        s.drag_last_y = y;
    });
    canvas_handle.on_mouse_up(|_x, _y, _button| {
        let s = unsafe { STATE.as_mut().unwrap() };
        s.drag_button = 0;
    });
    canvas_handle.on_mouse_move(|x, y| {
        let s = unsafe { STATE.as_mut().unwrap() };
        if s.drag_button == 0 { return; }
        let dx = x - s.drag_last_x;
        let dy = y - s.drag_last_y;
        s.drag_last_x = x;
        s.drag_last_y = y;
        if s.drag_button == 1 {
            // Left-drag: orbit
            s.camera_yaw += dx as f32 * 0.01;
            s.camera_pitch += dy as f32 * 0.01;
            // Clamp pitch to avoid flipping
            if s.camera_pitch < 0.05 { s.camera_pitch = 0.05; }
            if s.camera_pitch > 1.5 { s.camera_pitch = 1.5; }
        } else if s.drag_button == 2 {
            // Right-drag: zoom (drag up = zoom in, drag down = zoom out)
            s.camera_dist += dy as f32 * 0.03;
            if s.camera_dist < 1.5 { s.camera_dist = 1.5; }
            if s.camera_dist > 12.0 { s.camera_dist = 12.0; }
        }
    });

    // ── Fullscreen support ────────────────────────────────────────────────
    // Register as fullscreen-capable (Alt+Enter will toggle fullscreen)
    window.set_fullscreen_capable(false);

    window.on_fullscreen_enter(|_| {
        if let Some(info) = libanyui_client::get_fullscreen_info() {
            anyos_std::println!("gldemo: fullscreen ENTER {}x{} stride={} fb={:#x}",
                info.width, info.height, info.stride, info.fb_ptr);
            let s = unsafe { STATE.as_mut().unwrap() };
            s.fullscreen = true;
            s.fb_w = info.width;
            s.fb_h = info.height;
            gl::gl_resize(info.width, info.height);
            gl::viewport(0, 0, info.width as i32, info.height as i32);
            if info.fb_ptr != 0 {
                // Direct framebuffer access mode
                gl::gl_init_fullscreen(
                    info.fb_ptr as *mut u32,
                    info.width,
                    info.height,
                    info.stride,
                );
            }
        }
    });

    window.on_fullscreen_exit(|_| {
        anyos_std::println!("gldemo: fullscreen EXIT");
        let s = unsafe { STATE.as_mut().unwrap() };
        s.fullscreen = false;
        gl::gl_exit_fullscreen();
        // Restore canvas size
        let w = s.canvas.get_stride();
        let h = s.canvas.get_height();
        if w > 0 && h > 0 {
            s.fb_w = w;
            s.fb_h = h;
            gl::gl_resize(w, h);
            gl::viewport(0, 0, w as i32, h as i32);
        }
    });

    // ── 60fps animation timer ────────────────────────────────────────────
    libanyui_client::set_timer(16, || {
        render_frame();
    });

    anyos_std::println!("gldemo: entering event loop");
    libanyui_client::run();
}
