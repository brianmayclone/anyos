# anyOS OpenGL Library (libgl) API Reference

The **libgl** shared library provides an OpenGL ES 2.0 compatible 3D graphics engine with a built-in GLSL ES 1.00 shader compiler, software rasterizer, and hardware-accelerated GPU backends via loadable `.drv` drivers. When a compatible GPU is detected (VMware SVGA II or VirtIO GPU with virgl), libgl loads the appropriate userspace driver for hardware-accelerated 3D. Otherwise, it falls back to the built-in software rasterizer, producing an ARGB framebuffer that can be displayed on any anyOS surface (e.g. anyui Canvas).

**Format:** ELF64 shared object (.so), loaded via `dl_open("/Libraries/libgl.so")`
**Exports:** 120
**Client crate:** `libgl_client` (uses `dynlink::dl_open` / `dl_sym`)
**API level:** OpenGL ES 2.0 (Phase 1 subset)
**Shader execution:** JIT-compiled x86_64 SSE (primary) with IR interpreter fallback

---

## Table of Contents

- [Getting Started](#getting-started)
- [Architecture](#architecture)
  - [Rendering Pipeline](#rendering-pipeline)
  - [GLSL Compiler](#glsl-compiler)
  - [Software Rasterizer](#software-rasterizer)
- [Client API (libgl_client)](#client-api-libgl_client)
  - [Initialization](#initialization)
  - [Fullscreen & Cursor](#fullscreen--cursor)
  - [State Management](#state-management)
  - [Buffer Objects](#buffer-objects)
  - [Texture Objects](#texture-objects)
  - [Shader & Program Objects](#shader--program-objects)
  - [Uniforms & Attributes](#uniforms--attributes)
  - [Draw Calls](#draw-calls)
  - [Framebuffer Operations](#framebuffer-operations)
  - [FXAA & Backend Selection](#fxaa--backend-selection)
  - [Shadow Mapping](#shadow-mapping)
  - [Math Library](#math-library)
  - [Physics Engine](#physics-engine)
- [C ABI Exports](#c-abi-exports)
  - [anyOS Extensions (5)](#anyos-extensions-5)
  - [Fullscreen & Cursor (5)](#fullscreen--cursor-5)
  - [State Management (17)](#state-management-17)
  - [Buffer Objects (5)](#buffer-objects-5)
  - [Texture Objects (8)](#texture-objects-8)
  - [Shader Objects (6)](#shader-objects-6)
  - [Program Objects (7)](#program-objects-7)
  - [Uniforms & Attributes (12)](#uniforms--attributes-12)
  - [Draw Calls (2)](#draw-calls-2)
  - [Framebuffer Objects (8)](#framebuffer-objects-8)
  - [FXAA & Backend Selection (4)](#fxaa--backend-selection-4)
  - [Shadow Mapping (5)](#shadow-mapping-5)
  - [Math Library (12)](#math-library-12)
  - [Physics Engine (24)](#physics-engine-24)
- [GLSL ES 1.00 Subset](#glsl-es-100-subset)
  - [Supported Types](#supported-types)
  - [Qualifiers](#qualifiers)
  - [Built-in Functions](#built-in-functions)
  - [Built-in Variables](#built-in-variables)
  - [Operators](#operators)
  - [Limitations (Phase 1)](#limitations-phase-1)
- [Constants Reference](#constants-reference)
- [Constraints](#constraints)
- [Hardware-Accelerated 3D (GPU Driver HAL)](#hardware-accelerated-3d-gpu-driver-hal)
- [Future Roadmap](#future-roadmap)

---

## Getting Started

### Dependencies

Add to your program's `Cargo.toml`:

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
dynlink = { path = "../../libs/dynlink" }
libanyui_client = { path = "../../libs/libanyui_client" }
libgl_client = { path = "../../libs/libgl_client" }
```

### Minimal Example — Colored Triangle

```rust
#![no_std]
#![no_main]

anyos_std::entry!(main);
use libgl_client as gl;

static VS: &str = "
attribute vec3 aPosition;
attribute vec3 aColor;
varying vec3 vColor;
void main() {
    gl_Position = vec4(aPosition, 1.0);
    vColor = aColor;
}
";

static FS: &str = "
precision mediump float;
varying vec3 vColor;
void main() {
    gl_FragColor = vec4(vColor, 1.0);
}
";

fn main() {
    // Set up anyui window + canvas
    libanyui_client::init();
    let window = libanyui_client::Window::new("Triangle", 100, 100, 420, 420);
    let canvas = libanyui_client::Canvas::new(400, 400);
    canvas.set_position(0, 0);
    window.add(&canvas);
    window.set_visible(true);

    let fb_w = canvas.get_stride();
    let fb_h = canvas.get_height();

    // Initialize libgl
    if !gl::init() { return; }
    gl::gl_init(fb_w, fb_h);
    gl::viewport(0, 0, fb_w as i32, fb_h as i32);

    // Compile shaders
    let vs = gl::create_shader(gl::GL_VERTEX_SHADER);
    gl::shader_source(vs, VS);
    gl::compile_shader(vs);

    let fs = gl::create_shader(gl::GL_FRAGMENT_SHADER);
    gl::shader_source(fs, FS);
    gl::compile_shader(fs);

    let prog = gl::create_program();
    gl::attach_shader(prog, vs);
    gl::attach_shader(prog, fs);
    gl::link_program(prog);
    gl::use_program(prog);

    // Triangle vertices: position (x,y,z) + color (r,g,b)
    let verts: [f32; 18] = [
         0.0,  0.5, 0.0,  1.0, 0.0, 0.0,  // top (red)
        -0.5, -0.5, 0.0,  0.0, 1.0, 0.0,  // bottom-left (green)
         0.5, -0.5, 0.0,  0.0, 0.0, 1.0,  // bottom-right (blue)
    ];

    let mut vbo = [0u32; 1];
    gl::gen_buffers(1, &mut vbo);
    gl::bind_buffer(gl::GL_ARRAY_BUFFER, vbo[0]);
    gl::buffer_data_f32(gl::GL_ARRAY_BUFFER, &verts, gl::GL_STATIC_DRAW);

    let loc_pos = gl::get_attrib_location(prog, "aPosition");
    let loc_col = gl::get_attrib_location(prog, "aColor");

    if loc_pos >= 0 {
        gl::enable_vertex_attrib_array(loc_pos as u32);
        gl::vertex_attrib_pointer(loc_pos as u32, 3, gl::GL_FLOAT, false, 24, 0);
    }
    if loc_col >= 0 {
        gl::enable_vertex_attrib_array(loc_col as u32);
        gl::vertex_attrib_pointer(loc_col as u32, 3, gl::GL_FLOAT, false, 24, 12);
    }

    // Render
    gl::clear_color(0.1, 0.1, 0.15, 1.0);
    gl::clear(gl::GL_COLOR_BUFFER_BIT);
    gl::draw_arrays(gl::GL_TRIANGLES, 0, 3);

    // Display
    let fb_ptr = gl::swap_buffers();
    if !fb_ptr.is_null() {
        let pixels = unsafe {
            core::slice::from_raw_parts(fb_ptr, (fb_w * fb_h) as usize)
        };
        canvas.copy_pixels_from(pixels);
    }

    loop { anyos_std::process::sleep(100); }
}
```

---

## Architecture

### Module Structure

```
libs/libgl/src/
  lib.rs                 C ABI exports (120), allocator, global context
  types.rs               GL type aliases and constants
  state.rs               GlContext state machine
  buffer.rs              VBO/EBO storage
  texture.rs             Texture objects with sampling
  shader.rs              Shader/Program objects, link-time JIT compilation
  framebuffer.rs         SwFramebuffer (ARGB color + f32 depth)
  draw.rs                Draw dispatch
  physics.rs             Rigid-body physics engine (sphere, plane, box colliders)
  simd.rs                SSE-accelerated Vec4 (wraps __m128)
  fxaa.rs                FXAA post-processing
  drv_loader.rs          GPU driver HAL — ELF loader + drv_* symbol resolution
  svga3d.rs              SVGA3D GPU command generation (legacy, superseded by svga3d.drv)
  syscall.rs             Minimal syscalls (sbrk, exit, dll_load, gpu_query_type)
  compiler/
    mod.rs               Compile pipeline orchestration
    lexer.rs             GLSL tokenizer (~40 token types)
    ast.rs               AST node definitions
    parser.rs            Recursive-descent parser
    ir.rs                SSA-style intermediate representation (~35 opcodes)
    lower.rs             AST -> IR lowering (with running uniform offset)
    backend_sw.rs        IR interpreter (register file: [f32; 4] per register)
    backend_jit.rs       x86_64 SSE JIT compiler (IR -> native machine code)
    backend_dx9.rs       IR -> DX9 SM 2.0 bytecode (SVGA3D)
  rasterizer/
    mod.rs               Pipeline orchestration, fast-path dispatch
    vertex.rs            Vertex attribute fetching from VBOs
    clipper.rs           Sutherland-Hodgman frustum clipping
    raster.rs            Edge-function rasterization + fast-path rasterizer
    fragment.rs          Depth test, blending
    math.rs              Transcendental functions without libm
```

### Rendering Pipeline

```
glDrawArrays(GL_TRIANGLES, 0, N)
  |
  v
Vertex Assembly
  Read vertex data from VBO according to glVertexAttribPointer layout
  |
  v
Vertex Shader (per vertex)
  JIT-compiled native x86_64 SSE code (preferred)
  Fallback: IR interpreter when JIT unavailable
  Outputs: gl_Position (clip-space), varyings
  |
  v
Primitive Assembly
  Group vertices into triangles (TRIANGLES / TRIANGLE_STRIP / TRIANGLE_FAN)
  |
  v
Frustum Clipping
  Trivial-accept test: skip clipping if all 3 vertices inside frustum
  Slow path: Sutherland-Hodgman against 6 clip planes (may produce 0-N triangles)
  |
  v
Perspective Divide
  xyz /= w for each vertex
  |
  v
Viewport Transform
  NDC [-1,1] -> screen coordinates [0, width/height]
  |
  v
Backface Culling
  Discard triangles facing away from camera (if GL_CULL_FACE enabled)
  |
  v
Rasterization (one of two paths)
  [FAST PATH] Textured + vertex-lit triangles:
    Zero per-pixel function calls; inline texture sampling + color math
    Activated when: FS ≤ 20 instructions, ≥ 2 varyings, no blending, texture bound
  [STANDARD PATH] General fragment shaders:
    JIT-compiled native code (preferred) or IR interpreter fallback
    Incremental edge functions with scanline span clipping
    Perspective-correct varying interpolation (SIMD Vec4)
  |
  v
Per-Fragment Operations
  Early depth test (before fragment shader) -> Blending -> Color write -> SwFramebuffer
```

### GLSL Compiler

The built-in GLSL compiler processes shader source through 4 stages, with 3 execution backends:

#### Compilation Stages

1. **Lexer** (`compiler/lexer.rs`) — Tokenizes GLSL source into ~40 token types (keywords, identifiers, numbers, operators, punctuation). Handles C-style comments, preprocessor lines, hex/float literals.

2. **Parser** (`compiler/parser.rs`) — Recursive-descent parser with precedence climbing. Produces an AST representing declarations (precision, variables, functions), statements (assign, return, if, for, discard), and expressions (binary, unary, ternary, call, swizzle, constructor).

3. **IR Lowering** (`compiler/lower.rs`) — Converts AST to a register-based IR. Each register holds `[f32; 4]`. Allocates registers for attributes, uniforms, varyings, and locals. Lowers built-in functions, type constructors, swizzle operations, and matrix operations to IR instructions. Uses a running uniform offset counter to correctly index mixed mat4/scalar uniforms (mat4 = 4 slots, others = 1 slot).

4. **Link-time Processing** (`shader.rs`) — `link_program()` merges VS and FS IR, resolves uniform/attribute/varying bindings, patches FS `LoadUniform` indices by the VS uniform slot count (since the combined uniform array places VS uniforms first), and JIT-compiles both shaders to native x86_64 code.

#### Execution Backends

**JIT Backend** (`compiler/backend_jit.rs`) — Compiles IR to native x86_64 SSE machine code at `glLinkProgram()` time. The compiled code is cached in `GlProgram.vs_jit` / `GlProgram.fs_jit` and reused every frame. This eliminates the ~15-cycle branch-misprediction penalty per IR instruction from the interpreter's match dispatch.

Architecture:
- Calling convention: `extern "C" fn(ctx: *const JitContext)`
- Register allocation: RBX = register file base, R12 = uniforms, R13 = attributes, R14 = varyings in, R15 = varyings out, RBP = ctx pointer
- Shader virtual registers (up to 128) live in memory at `[RBX + reg * 16]`
- Each instruction does load → SSE operate → store, emitting straight-line code with zero branching
- Supports all ~35 IR opcodes including `TexSample` (calls out to `real_tex_sample`)
- For a typical 70-instruction fragment shader executed ~80K times per frame, eliminates ~84M wasted cycles

**SW Backend** (`compiler/backend_sw.rs`) — Interprets IR instructions against a register file. Fallback when JIT compilation fails. Executes per-vertex (vertex shader) and per-fragment (fragment shader). Uses polynomial math approximations from `rasterizer/math.rs`.

**DX9 Backend** (`compiler/backend_dx9.rs`) — Translates IR to DX9 Shader Model 2.0 bytecode for SVGA3D GPU acceleration (Phase 2).

#### IR Opcodes (~35)

| Category | Instructions |
|----------|-------------|
| Data | `LoadConst`, `Mov`, `Swizzle`, `WriteMask` |
| Arithmetic | `Add`, `Sub`, `Mul`, `Div`, `Neg` |
| Vector | `Dp3`, `Dp4`, `Cross`, `Normalize`, `Length`, `Reflect` |
| Math | `Abs`, `Floor`, `Fract`, `Pow`, `Sqrt`, `Rsqrt`, `Sin`, `Cos` |
| Blend | `Min`, `Max`, `Clamp`, `Mix` |
| Matrix | `MatMul4`, `MatMul3` |
| Compare | `CmpLt`, `CmpEq`, `Select` |
| Convert | `IntToFloat`, `FloatToInt` |
| Texture | `TexSample` |
| I/O | `LoadAttribute`, `LoadUniform`, `LoadVarying`, `StoreVarying`, `StorePosition`, `StoreFragColor`, `StorePointSize` |

### Software Rasterizer

#### Standard Path (raster.rs — `rasterize_triangle`)

The standard rasterizer handles arbitrary fragment shaders via the JIT backend or interpreter:

**Incremental edge functions** — Instead of evaluating 3 edge functions from scratch per pixel (6 multiplications), the rasterizer steps them incrementally: 3 additions per pixel.

**Scanline span clipping** — Instead of testing every pixel in the bounding box, computes the exact x-range per scanline where all 3 edge functions are non-negative. For a sphere with 320 thin triangles, this eliminates ~95% of rejected pixel iterations (from ~7M down to ~50K). The algorithm:
- For each edge function `w(x) = w_row + a * (x - min_x)`:
  - If `a > 0` and `w_row < 0`: left bound at `x = min_x + ceil(-w_row / a)`
  - If `a < 0` and `w_row >= 0`: right bound at `x = min_x + floor(w_row / |a|)`
  - If `a ≈ 0` and `w_row < 0`: entire scanline is outside

**Perspective-correct varying interpolation** — Pre-divides varyings by clip-space W per vertex before the scanline loop. Per pixel: multiply-add chains + one `fast_rcp()` correction. Uses SIMD `Vec4` for 4-wide packed operations.

**Early depth test** — Depth comparison runs before varying interpolation and fragment shader, skipping expensive fragment work for occluded pixels.

**Fast reciprocal** — `fast_rcp()` uses SSE `rcpss` instruction (~4 cycles) with Newton-Raphson refinement, vs ~20 cycles for `divss`. Accuracy is sufficient for perspective correction.

**Winding normalization** — When screen-space triangle area is negative (CW winding from viewport Y-flip), all edge values and increments are negated so the inside test (`>= 0`) works uniformly.

#### Fast Path (raster.rs — `rasterize_triangle_fast`)

A specialized rasterizer for the common "textured + vertex-lit" case that eliminates **all per-pixel function calls**:

**Activation criteria:**
- Fragment shader has ≤ 20 IR instructions
- At least 2 varyings (lighting + texcoord)
- Blending is disabled
- A texture is bound on unit 0

**What it inlines per pixel (zero function call overhead):**
- Texture coordinate wrapping (`GL_REPEAT` via integer truncation)
- Nearest-neighbor texture sampling (direct pointer arithmetic into texture data)
- ARGB unpack/repack
- Color math: `lighting × texel_byte × matColor` → clamped u32

**Varyings layout:**
- Varying 0 = lighting (R, G, B in components [0], [1], [2])
- Varying 1 = texcoord (U, V in components [0], [1])

This matches the output of a Gouraud (per-vertex) lighting vertex shader.

**Pre-resolved texture** — `ResolvedTexture` resolves the bound texture's data pointer, dimensions, and length once before the draw loop, avoiding per-pixel indirection through the texture store.

**`FastPathInfo`** — Holds the resolved texture and pre-extracted material color (matColor RGB), resolved once per draw call by searching for a uniform whose name contains "MatColor".

#### Frustum Clipping

Uses Sutherland-Hodgman against 6 planes in clip space (`-w <= x,y,z <= w`). A single triangle can produce 0 to N output triangles.

**Trivial-accept optimization** — `trivially_inside()` checks if all 3 vertices of a triangle satisfy `-w <= x,y,z <= w`. If so, clipping is skipped entirely — a significant win since clipping involves `Vec` allocations for variable-length output.

#### SIMD Vec4 (simd.rs)

`Vec4` wraps `__m128` (one XMM register, 4 × f32). Used in the rasterizer inner loop for varying interpolation:
- `load` / `store` — Memory transfer (unaligned supported)
- `splat` — Broadcast scalar to all 4 lanes
- `add`, `sub`, `mul` — Packed SSE operations (1 cycle each)
- Replaces 4 scalar operations with 1 SIMD instruction per varying component

#### Math Functions (rasterizer/math.rs)

All implemented without libm (no_std compatible):
- `sqrt` — Quake III fast inverse sqrt + 3 Newton iterations
- `sin` — Parabola approximation with refinement pass
- `pow` — `exp2(exp * log2(base))` with IEEE 754 bit manipulation
- `log2` — IEEE 754 exponent extraction + polynomial for mantissa
- `exp2` — Minimax polynomial approximation
- `ceil` — Integer ceiling via cast + conditional increment

---

## Client API (libgl_client)

The `libgl_client` crate provides ergonomic Rust wrappers around the 120 C ABI exports. All functions are free-standing (no receiver) and operate on the global GL context.

### Initialization

```rust
/// Load libgl.so and resolve all 120 function pointers.
/// Returns false if loading fails.
pub fn init() -> bool;

/// Initialize the GL context with framebuffer dimensions.
pub fn gl_init(width: u32, height: u32);

/// Resize the GL framebuffer (preserves shaders, buffers, textures).
pub fn gl_resize(width: u32, height: u32);

/// Swap buffers. Returns a pointer to the ARGB pixel data.
pub fn swap_buffers() -> *const u32;

/// Get a pointer to the backbuffer.
pub fn get_backbuffer() -> *const u32;
```

### Fullscreen & Cursor

```rust
/// Initialize fullscreen direct framebuffer rendering.
pub fn gl_init_fullscreen(fb_ptr: *mut u32, width: u32, height: u32, stride: u32);

/// Exit fullscreen mode.
pub fn gl_exit_fullscreen();

/// Swap buffers in fullscreen mode (copies directly to mapped framebuffer).
pub fn swap_buffers_fullscreen() -> bool;

/// Set cursor captured state (for FPS-style mouse grab). Returns previous state.
pub fn set_cursor_captured(captured: bool) -> bool;

/// Query whether the cursor is currently captured.
pub fn get_cursor_captured() -> bool;
```

### State Management

```rust
pub fn get_error() -> GLenum;
pub fn enable(cap: GLenum);
pub fn disable(cap: GLenum);
pub fn blend_func(sfactor: GLenum, dfactor: GLenum);
pub fn depth_func(func: GLenum);
pub fn depth_mask(flag: bool);
pub fn cull_face(mode: GLenum);
pub fn front_face(mode: GLenum);
pub fn viewport(x: i32, y: i32, w: i32, h: i32);
pub fn clear_color(r: f32, g: f32, b: f32, a: f32);
pub fn clear(mask: u32);
pub fn color_mask(r: bool, g: bool, b: bool, a: bool);
```

### Buffer Objects

```rust
pub fn gen_buffers(n: i32, ids: &mut [u32]);
pub fn delete_buffers(ids: &[u32]);
pub fn bind_buffer(target: GLenum, buffer: u32);
pub fn buffer_data(target: GLenum, data: &[u8], usage: GLenum);
pub fn buffer_data_f32(target: GLenum, data: &[f32], usage: GLenum);
pub fn buffer_data_u16(target: GLenum, data: &[u16], usage: GLenum);
```

### Texture Objects

```rust
pub fn gen_textures(n: i32, ids: &mut [u32]);
pub fn delete_textures(ids: &[u32]);
pub fn bind_texture(target: GLenum, texture: u32);
pub fn tex_parameteri(target: GLenum, pname: GLenum, param: i32);
pub fn tex_image_2d(target: GLenum, level: i32, internal_format: i32,
                    width: i32, height: i32, border: i32,
                    format: GLenum, type_: GLenum, data: &[u8]);
pub fn active_texture(texture: GLenum);
```

### Shader & Program Objects

```rust
pub fn create_shader(shader_type: GLenum) -> u32;
pub fn delete_shader(shader: u32);
pub fn shader_source(shader: u32, source: &str);
pub fn compile_shader(shader: u32);
pub fn get_shader_compile_status(shader: u32) -> bool;
pub fn get_shader_info_log(shader: u32) -> alloc::string::String;

pub fn create_program() -> u32;
pub fn delete_program(program: u32);
pub fn attach_shader(program: u32, shader: u32);
pub fn link_program(program: u32);
pub fn use_program(program: u32);
pub fn get_program_link_status(program: u32) -> bool;
```

### Uniforms & Attributes

```rust
pub fn get_uniform_location(program: u32, name: &str) -> i32;
pub fn get_attrib_location(program: u32, name: &str) -> i32;
pub fn uniform1i(location: i32, v: i32);
pub fn uniform1f(location: i32, v: f32);
pub fn uniform3f(location: i32, x: f32, y: f32, z: f32);
pub fn uniform4f(location: i32, x: f32, y: f32, z: f32, w: f32);
pub fn uniform_matrix4fv(location: i32, transpose: bool, value: &[f32; 16]);
pub fn enable_vertex_attrib_array(index: u32);
pub fn disable_vertex_attrib_array(index: u32);
pub fn vertex_attrib_pointer(index: u32, size: i32, type_: GLenum, normalized: bool, stride: i32, offset: usize);
```

### Draw Calls

```rust
pub fn draw_arrays(mode: GLenum, first: i32, count: i32);
pub fn draw_elements(mode: GLenum, count: i32, type_: GLenum, offset: usize);
```

### Framebuffer Operations

```rust
pub fn gen_framebuffers(n: i32, fbs: &mut [u32]);
pub fn delete_framebuffers(n: i32, fbs: &[u32]);
pub fn bind_framebuffer(target: u32, framebuffer: u32);
pub fn framebuffer_texture_2d(target: u32, attachment: u32, textarget: u32, texture: u32, level: i32);
pub fn check_framebuffer_status(target: u32) -> u32;
pub fn flush();
pub fn finish();
```

### FXAA & Backend Selection

```rust
/// Enable or disable FXAA post-process anti-aliasing.
pub fn set_fxaa(enabled: bool);

/// Switch between hardware (GPU driver) and software rasterizer.
pub fn set_hw_backend(enabled: bool);

/// Query whether the hardware backend is currently active.
pub fn get_hw_backend() -> bool;

/// Query whether a hardware 3D driver is available (even if not in use).
pub fn has_hw_backend() -> bool;
```

### Shadow Mapping

```rust
/// Begin shadow pass. Returns true if HW shadow is active.
pub fn shadow_pass_begin(lx: f32, ly: f32, lz: f32, tx: f32, ty: f32, tz: f32, radius: f32) -> bool;

/// End shadow pass. Shadow map is now available for the main render.
pub fn shadow_pass_end();

/// Get the light MVP matrix (16 floats). Returns null if no shadow.
pub fn shadow_get_light_mvp() -> *const f32;

/// Whether a shadow map is available this frame.
pub fn shadow_available() -> bool;

/// Texture unit where shadow map is bound (always 7).
pub fn shadow_get_unit() -> u32;
```

### Math Library

```rust
pub const PI: f32 = 3.14159265;

pub fn sin(x: f32) -> f32;       // x87 fsin
pub fn cos(x: f32) -> f32;       // x87 fcos
pub fn tan(x: f32) -> f32;       // x87 fptan
pub fn sqrt(x: f32) -> f32;      // SSE2 sqrtss
pub fn abs(x: f32) -> f32;       // sign-bit clear
pub fn pow(base: f32, exp: f32) -> f32;  // x87 FPU
pub fn log2(x: f32) -> f32;      // x87 fyl2x
pub fn exp2(x: f32) -> f32;      // x87 f2xm1+fscale
pub fn floor(x: f32) -> f32;     // x87 rounding
pub fn ceil(x: f32) -> f32;      // x87 rounding
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32;
pub fn lerp(a: f32, b: f32, t: f32) -> f32;
```

### Physics Engine

```rust
/// Create/reset the physics world.
pub fn physics_create_world();

/// Set the gravity vector (default: 0, -9.81, 0).
pub fn physics_set_gravity(gx: f32, gy: f32, gz: f32);

/// Step the physics simulation by dt seconds.
pub fn physics_step(dt: f32);

/// Add a sphere body. Returns body ID.
pub fn physics_add_sphere(mass: f32, radius: f32, x: f32, y: f32, z: f32) -> u32;

/// Add an infinite plane (static). Returns body ID.
pub fn physics_add_plane(nx: f32, ny: f32, nz: f32, d: f32) -> u32;

/// Add an axis-aligned box body. Returns body ID.
pub fn physics_add_box(mass: f32, hx: f32, hy: f32, hz: f32, x: f32, y: f32, z: f32) -> u32;

/// Set body velocity.
pub fn physics_set_velocity(id: u32, vx: f32, vy: f32, vz: f32);

/// Set body position.
pub fn physics_set_position(id: u32, x: f32, y: f32, z: f32);

/// Get body position. Returns (x, y, z).
pub fn physics_get_position(id: u32) -> (f32, f32, f32);

/// Get body velocity. Returns (vx, vy, vz).
pub fn physics_get_velocity(id: u32) -> (f32, f32, f32);

/// Set coefficient of restitution (bounciness, 0.0..1.0).
pub fn physics_set_restitution(id: u32, e: f32);

/// Set body mass (0.0 = static/immovable).
pub fn physics_set_mass(id: u32, mass: f32);

/// Apply a force to a body (accumulated until next step).
pub fn physics_apply_force(id: u32, fx: f32, fy: f32, fz: f32);

/// Apply an impulse (instant velocity change).
pub fn physics_apply_impulse(id: u32, ix: f32, iy: f32, iz: f32);

/// Set angular velocity around Y axis (rad/s).
pub fn physics_set_angular_vel_y(id: u32, omega: f32);

/// Get current Y rotation angle (rad, extracted from quaternion).
pub fn physics_get_rotation_y(id: u32) -> f32;

/// Set full 3D angular velocity (rad/s).
pub fn physics_set_angular_velocity(id: u32, wx: f32, wy: f32, wz: f32);

/// Get orientation quaternion. Returns (w, x, y, z).
pub fn physics_get_orientation(id: u32) -> (f32, f32, f32, f32);

/// Set angular damping factor (0.0 = none, ~2.0 = moderate, ~5.0 = heavy).
pub fn physics_set_angular_damping(id: u32, damping: f32);

/// Set linear damping factor (air resistance).
pub fn physics_set_linear_damping(id: u32, damping: f32);

/// Set whether body is affected by gravity.
pub fn physics_set_use_gravity(id: u32, use_grav: bool);

/// Set body active/inactive.
pub fn physics_set_active(id: u32, active: bool);

/// Update the distance parameter of a plane collider.
pub fn physics_set_plane_d(id: u32, d: f32);

/// Get number of bodies in the world.
pub fn physics_body_count() -> u32;
```

---

## C ABI Exports

All 120 exported functions use `extern "C"` with `#[no_mangle]`. Strings are null-terminated C strings. Object handles (shaders, programs, buffers, textures) are 1-based unsigned integers; 0 indicates "none" or failure.

### anyOS Extensions (5)

| Export | Signature | Description |
|--------|-----------|-------------|
| `gl_init` | `(u32 width, u32 height)` | Initialize GL context with framebuffer dimensions; loads HW driver if available |
| `gl_deinit` | `()` | Shut down libgl, unload hardware driver, free resources |
| `gl_resize` | `(u32 width, u32 height)` | Resize GL framebuffer without destroying shaders, buffers, or textures |
| `gl_swap_buffers` | `() -> *const u32` | Return pointer to ARGB color buffer (runs FXAA if enabled, HW readback if active) |
| `gl_get_backbuffer` | `() -> *const u32` | Return pointer to backbuffer (alias for single-buffered SW) |

### Fullscreen & Cursor (5)

| Export | Signature | Description |
|--------|-----------|-------------|
| `gl_init_fullscreen` | `(*mut u32 fb_ptr, u32 width, u32 height, u32 stride)` | Initialize fullscreen direct framebuffer rendering; stride is row stride in pixels |
| `gl_exit_fullscreen` | `()` | Exit fullscreen mode, stop writing to direct framebuffer |
| `gl_swap_buffers_fullscreen` | `() -> u32` | Copy rendered frame to mapped framebuffer; returns 1 if performed, 0 if not in fullscreen |
| `gl_set_cursor_captured` | `(u32 captured) -> u32` | Set cursor captured state (hidden + grabbed); returns previous state |
| `gl_get_cursor_captured` | `() -> u32` | Query whether cursor is captured (1) or not (0) |

### State Management (17)

| Export | Signature | Description |
|--------|-----------|-------------|
| `glGetError` | `() -> GLenum` | Get and clear error code |
| `glGetString` | `(GLenum name) -> *const u8` | Get implementation string (VENDOR, RENDERER, VERSION) |
| `glEnable` | `(GLenum cap)` | Enable capability (DEPTH_TEST, BLEND, CULL_FACE, SCISSOR_TEST) |
| `glDisable` | `(GLenum cap)` | Disable capability |
| `glBlendFunc` | `(GLenum sfactor, GLenum dfactor)` | Set blend function |
| `glBlendFuncSeparate` | `(GLenum srcRGB, GLenum dstRGB, GLenum srcA, GLenum dstA)` | Set separate RGB/alpha blend functions |
| `glDepthFunc` | `(GLenum func)` | Set depth comparison function |
| `glDepthMask` | `(GLboolean flag)` | Enable/disable depth buffer writes |
| `glCullFace` | `(GLenum mode)` | Set face culling mode (FRONT, BACK, FRONT_AND_BACK) |
| `glFrontFace` | `(GLenum mode)` | Set front-face winding (CW, CCW) |
| `glViewport` | `(GLint x, GLint y, GLsizei w, GLsizei h)` | Set viewport rectangle |
| `glClearColor` | `(GLclampf r, GLclampf g, GLclampf b, GLclampf a)` | Set clear color |
| `glClear` | `(GLbitfield mask)` | Clear buffers (COLOR_BUFFER_BIT, DEPTH_BUFFER_BIT) |
| `glScissor` | `(GLint x, GLint y, GLsizei w, GLsizei h)` | Set scissor rectangle |
| `glLineWidth` | `(GLfloat width)` | Set line width (stored, not fully implemented) |
| `glPixelStorei` | `(GLenum pname, GLint param)` | Set pixel storage mode (UNPACK/PACK_ALIGNMENT) |
| `glColorMask` | `(GLboolean r, GLboolean g, GLboolean b, GLboolean a)` | Set color write mask |

### Buffer Objects (5)

| Export | Signature | Description |
|--------|-----------|-------------|
| `glGenBuffers` | `(GLsizei n, GLuint *buffers)` | Generate buffer names |
| `glDeleteBuffers` | `(GLsizei n, const GLuint *buffers)` | Delete buffers |
| `glBindBuffer` | `(GLenum target, GLuint buffer)` | Bind buffer to ARRAY_BUFFER or ELEMENT_ARRAY_BUFFER |
| `glBufferData` | `(GLenum target, GLsizeiptr size, const void *data, GLenum usage)` | Upload buffer data |
| `glBufferSubData` | `(GLenum target, GLintptr offset, GLsizeiptr size, const void *data)` | Update buffer sub-region |

### Texture Objects (8)

| Export | Signature | Description |
|--------|-----------|-------------|
| `glGenTextures` | `(GLsizei n, GLuint *textures)` | Generate texture names |
| `glDeleteTextures` | `(GLsizei n, const GLuint *textures)` | Delete textures |
| `glBindTexture` | `(GLenum target, GLuint texture)` | Bind texture to active unit |
| `glTexImage2D` | `(GLenum target, GLint level, GLint internalformat, GLsizei w, GLsizei h, GLint border, GLenum format, GLenum type, const void *data)` | Upload texture image |
| `glTexSubImage2D` | `(GLenum target, GLint level, GLint x, GLint y, GLsizei w, GLsizei h, GLenum format, GLenum type, const void *data)` | Update texture sub-region |
| `glTexParameteri` | `(GLenum target, GLenum pname, GLint param)` | Set texture parameter (filter, wrap mode) |
| `glActiveTexture` | `(GLenum texture)` | Set active texture unit (GL_TEXTURE0 + n) |
| `glGenerateMipmap` | `(GLenum target)` | Generate mipmaps (no-op in Phase 1) |

### Shader Objects (6)

| Export | Signature | Description |
|--------|-----------|-------------|
| `glCreateShader` | `(GLenum type) -> GLuint` | Create shader (VERTEX_SHADER or FRAGMENT_SHADER) |
| `glDeleteShader` | `(GLuint shader)` | Delete shader |
| `glShaderSource` | `(GLuint shader, GLsizei count, const GLchar **string, const GLint *length)` | Set shader source code |
| `glCompileShader` | `(GLuint shader)` | Compile shader (lexer -> parser -> AST -> IR) |
| `glGetShaderiv` | `(GLuint shader, GLenum pname, GLint *params)` | Query shader parameter (COMPILE_STATUS, INFO_LOG_LENGTH) |
| `glGetShaderInfoLog` | `(GLuint shader, GLsizei maxLen, GLsizei *length, GLchar *infoLog)` | Get compilation error log |

### Program Objects (7)

| Export | Signature | Description |
|--------|-----------|-------------|
| `glCreateProgram` | `() -> GLuint` | Create program object |
| `glDeleteProgram` | `(GLuint program)` | Delete program |
| `glAttachShader` | `(GLuint program, GLuint shader)` | Attach compiled shader to program |
| `glLinkProgram` | `(GLuint program)` | Link program (resolves attributes, uniforms, varyings) |
| `glUseProgram` | `(GLuint program)` | Set active program for rendering |
| `glGetProgramiv` | `(GLuint program, GLenum pname, GLint *params)` | Query program parameter (LINK_STATUS) |
| `glGetProgramInfoLog` | `(GLuint program, GLsizei maxLen, GLsizei *length, GLchar *infoLog)` | Get link error log |

### Uniforms & Attributes (12)

| Export | Signature | Description |
|--------|-----------|-------------|
| `glGetUniformLocation` | `(GLuint program, const GLchar *name) -> GLint` | Get uniform location (-1 if not found) |
| `glGetAttribLocation` | `(GLuint program, const GLchar *name) -> GLint` | Get attribute location (-1 if not found) |
| `glBindAttribLocation` | `(GLuint program, GLuint index, const GLchar *name)` | Bind attribute to specific location |
| `glUniform1i` | `(GLint loc, GLint v0)` | Set integer uniform (also sets sampler unit) |
| `glUniform1f` | `(GLint loc, GLfloat v0)` | Set 1-float uniform |
| `glUniform2f` | `(GLint loc, GLfloat v0, GLfloat v1)` | Set 2-float uniform |
| `glUniform3f` | `(GLint loc, GLfloat v0, GLfloat v1, GLfloat v2)` | Set 3-float uniform |
| `glUniform4f` | `(GLint loc, GLfloat v0, GLfloat v1, GLfloat v2, GLfloat v3)` | Set 4-float uniform |
| `glUniformMatrix4fv` | `(GLint loc, GLsizei count, GLboolean transpose, const GLfloat *value)` | Set 4x4 matrix uniform |
| `glEnableVertexAttribArray` | `(GLuint index)` | Enable vertex attribute array |
| `glDisableVertexAttribArray` | `(GLuint index)` | Disable vertex attribute array |
| `glVertexAttribPointer` | `(GLuint index, GLint size, GLenum type, GLboolean normalized, GLsizei stride, const void *pointer)` | Define vertex attribute layout |

### Draw Calls (2)

| Export | Signature | Description |
|--------|-----------|-------------|
| `glDrawArrays` | `(GLenum mode, GLint first, GLsizei count)` | Draw primitives from array data |
| `glDrawElements` | `(GLenum mode, GLsizei count, GLenum type, const void *indices)` | Draw indexed primitives |

**Supported primitive modes:** `GL_TRIANGLES`, `GL_TRIANGLE_STRIP`, `GL_TRIANGLE_FAN`

### Framebuffer Objects (8)

| Export | Signature | Description |
|--------|-----------|-------------|
| `glGenFramebuffers` | `(GLsizei n, GLuint *framebuffers)` | Generate FBO names |
| `glDeleteFramebuffers` | `(GLsizei n, const GLuint *framebuffers)` | Delete FBOs |
| `glBindFramebuffer` | `(GLenum target, GLuint framebuffer)` | Bind framebuffer (0 = default); in HW mode triggers shadow begin/end |
| `glFramebufferTexture2D` | `(GLenum target, GLenum attachment, GLenum textarget, GLuint texture, GLint level)` | Attach texture to FBO (COLOR_ATTACHMENT0, DEPTH_ATTACHMENT) |
| `glCheckFramebufferStatus` | `(GLenum target) -> GLenum` | Check FBO completeness (always returns COMPLETE) |
| `glReadPixels` | `(GLint x, GLint y, GLsizei w, GLsizei h, GLenum format, GLenum type, void *pixels)` | Read pixels from framebuffer (RGBA or RGB) |
| `glFlush` | `()` | Flush pending operations (no-op for SW) |
| `glFinish` | `()` | Finish all operations (no-op for SW) |

### FXAA & Backend Selection (4)

| Export | Signature | Description |
|--------|-----------|-------------|
| `gl_set_fxaa` | `(u32 enabled)` | Enable (non-zero) or disable (0) FXAA post-process anti-aliasing |
| `gl_set_hw_backend` | `(u32 enabled)` | Switch between hardware (non-zero) and software (0) rasterizer at runtime |
| `gl_get_hw_backend` | `() -> u32` | Query whether HW backend is currently active (1 = HW, 0 = SW) |
| `gl_has_hw_backend` | `() -> u32` | Query whether a HW 3D driver is available, regardless of current mode (1/0) |

### Shadow Mapping (5)

Shadow mapping is hardware-only (no-op on SW rasterizer). The shadow pass renders the scene from the light's perspective into a depth-only FBO, then binds the resulting depth texture to sampler unit 7 for the main render pass.

| Export | Signature | Description |
|--------|-----------|-------------|
| `gl_shadow_pass_begin` | `(f32 lx, f32 ly, f32 lz, f32 tx, f32 ty, f32 tz, f32 radius) -> u32` | Begin shadow pass; light at (lx,ly,lz) looking at (tx,ty,tz), radius = shadow volume extent; returns 1 if active |
| `gl_shadow_pass_end` | `()` | End shadow pass; shadow map bound to sampler unit 7 |
| `gl_shadow_get_light_mvp` | `() -> *const f32` | Get light MVP matrix (16 floats, column-major); null if no shadow |
| `gl_shadow_available` | `() -> u32` | Whether a shadow map is available this frame (1/0) |
| `gl_shadow_get_unit` | `() -> u32` | Texture unit where shadow map is bound (always 7) |

**Usage pattern:**
```rust
// 1. Shadow pass — render scene from light's POV
gl::shadow_pass_begin(light_x, light_y, light_z, target_x, target_y, target_z, 20.0);
// ... draw shadow casters with light MVP as the projection ...
gl::shadow_pass_end();

// 2. Main pass — shadow map is automatically on unit 7
let light_mvp = gl::shadow_get_light_mvp();
// ... bind light_mvp as uniform, sample shadow map in fragment shader ...
```

### Math Library (12)

Hardware-accelerated math functions using x87 FPU and SSE2 instructions. Usable by any application linked against libgl, not just for GL rendering.

| Export | Signature | Description |
|--------|-----------|-------------|
| `gl_math_sin` | `(f32 x) -> f32` | Sine via x87 `fsin` (IEEE 754 exact) |
| `gl_math_cos` | `(f32 x) -> f32` | Cosine via x87 `fcos` (IEEE 754 exact) |
| `gl_math_tan` | `(f32 x) -> f32` | Tangent via x87 `fptan` |
| `gl_math_sqrt` | `(f32 x) -> f32` | Square root via SSE2 `sqrtss` (IEEE 754 exact) |
| `gl_math_abs` | `(f32 x) -> f32` | Absolute value via sign-bit clear |
| `gl_math_pow` | `(f32 base, f32 exp) -> f32` | Power function via x87 FPU |
| `gl_math_log2` | `(f32 x) -> f32` | Base-2 logarithm via x87 `fyl2x` |
| `gl_math_exp2` | `(f32 x) -> f32` | Base-2 exponential via x87 `f2xm1` + `fscale` |
| `gl_math_floor` | `(f32 x) -> f32` | Floor via x87 rounding |
| `gl_math_ceil` | `(f32 x) -> f32` | Ceiling via x87 rounding |
| `gl_math_clamp` | `(f32 x, f32 lo, f32 hi) -> f32` | Clamp to [lo, hi] |
| `gl_math_lerp` | `(f32 a, f32 b, f32 t) -> f32` | Linear interpolation: a + (b - a) * t |

### Physics Engine (24)

A rigid-body physics engine with sphere, plane, and box colliders. Supports gravity, mass-based acceleration (F = ma), elastic collision response with restitution, semi-implicit Euler integration, quaternion-based orientation, and angular/linear damping. The physics world holds up to 64 bodies.

| Export | Signature | Description |
|--------|-----------|-------------|
| `gl_physics_create_world` | `() -> u32` | Create/reset the physics world; returns 1 on success |
| `gl_physics_set_gravity` | `(f32 gx, f32 gy, f32 gz)` | Set gravity vector (default: 0, -9.81, 0) |
| `gl_physics_step` | `(f32 dt)` | Step simulation by dt seconds |
| `gl_physics_add_sphere` | `(f32 mass, f32 radius, f32 x, f32 y, f32 z) -> u32` | Add sphere body; returns body ID |
| `gl_physics_add_plane` | `(f32 nx, f32 ny, f32 nz, f32 d) -> u32` | Add infinite static plane (normal + distance); returns body ID |
| `gl_physics_add_box` | `(f32 mass, f32 hx, f32 hy, f32 hz, f32 x, f32 y, f32 z) -> u32` | Add axis-aligned box (half-extents + position); returns body ID |
| `gl_physics_set_position` | `(u32 id, f32 x, f32 y, f32 z)` | Set body position |
| `gl_physics_get_position` | `(u32 id, *mut f32 x, *mut f32 y, *mut f32 z)` | Get body position (writes to output pointers) |
| `gl_physics_set_velocity` | `(u32 id, f32 vx, f32 vy, f32 vz)` | Set body velocity |
| `gl_physics_get_velocity` | `(u32 id, *mut f32 vx, *mut f32 vy, *mut f32 vz)` | Get body velocity (writes to output pointers) |
| `gl_physics_apply_force` | `(u32 id, f32 fx, f32 fy, f32 fz)` | Apply force (accumulated until next step) |
| `gl_physics_apply_impulse` | `(u32 id, f32 ix, f32 iy, f32 iz)` | Apply impulse (instant velocity change: dv = impulse / mass) |
| `gl_physics_set_restitution` | `(u32 id, f32 e)` | Set coefficient of restitution (0.0 = no bounce, 1.0 = perfect) |
| `gl_physics_set_mass` | `(u32 id, f32 mass)` | Set body mass (0.0 = static/immovable) |
| `gl_physics_set_angular_vel_y` | `(u32 id, f32 omega)` | Set angular velocity around Y axis (rad/s) |
| `gl_physics_get_rotation_y` | `(u32 id) -> f32` | Get Y rotation angle (rad, extracted from quaternion) |
| `gl_physics_set_angular_velocity` | `(u32 id, f32 wx, f32 wy, f32 wz)` | Set full 3D angular velocity (rad/s) |
| `gl_physics_get_orientation` | `(u32 id, *mut f32 w, *mut f32 x, *mut f32 y, *mut f32 z)` | Get orientation quaternion (writes to output pointers) |
| `gl_physics_set_angular_damping` | `(u32 id, f32 damping)` | Set angular damping (0.0 = none, ~2.0 = moderate, ~5.0 = heavy) |
| `gl_physics_set_linear_damping` | `(u32 id, f32 damping)` | Set linear damping (air resistance, 0.0 = none, ~0.5 = light drag) |
| `gl_physics_set_use_gravity` | `(u32 id, u32 use_grav)` | Set whether body is affected by gravity (0/non-zero) |
| `gl_physics_set_active` | `(u32 id, u32 active)` | Set body active (non-zero) or inactive (0) |
| `gl_physics_set_plane_d` | `(u32 id, f32 d)` | Update the distance parameter of a plane collider |
| `gl_physics_body_count` | `() -> u32` | Get number of bodies in the world |

**Collider types:**
- **Sphere** — radius-based; sphere-sphere and sphere-plane collisions resolved with positional correction
- **Plane** — infinite static surface defined by normal vector and distance from origin (n.dot(p) = d)
- **Box** — axis-aligned half-extents from center; box-plane collisions resolved via vertex projection

**Integration:** Semi-implicit Euler (velocity updated before position). Angular velocity integrated via quaternion derivative. Collision detection runs N^2 pairwise between active bodies.

---

## GLSL ES 1.00 Subset

### Supported Types

| Type | Description | Components |
|------|-------------|------------|
| `void` | No return value | 0 |
| `float` | 32-bit floating point | 1 |
| `int` | 32-bit integer | 1 |
| `bool` | Boolean | 1 |
| `vec2` | 2-component float vector | 2 |
| `vec3` | 3-component float vector | 3 |
| `vec4` | 4-component float vector | 4 |
| `mat3` | 3x3 float matrix | 9 |
| `mat4` | 4x4 float matrix | 16 |
| `sampler2D` | 2D texture sampler | 1 |

### Qualifiers

| Qualifier | Description |
|-----------|-------------|
| `attribute` | Per-vertex input (vertex shader only) |
| `varying` | Interpolated value passed from VS to FS |
| `uniform` | Constant value set by application |
| `const` | Compile-time constant |
| `precision` | Precision qualifier (parsed, ignored) |

### Built-in Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `texture2D` | `(sampler2D, vec2) -> vec4` | Sample 2D texture |
| `normalize` | `(vecN) -> vecN` | Normalize vector |
| `dot` | `(vecN, vecN) -> float` | Dot product |
| `cross` | `(vec3, vec3) -> vec3` | Cross product |
| `length` | `(vecN) -> float` | Vector length |
| `clamp` | `(T, T, T) -> T` | Clamp to range |
| `mix` | `(T, T, float) -> T` | Linear interpolation |
| `min` | `(T, T) -> T` | Minimum |
| `max` | `(T, T) -> T` | Maximum |
| `abs` | `(T) -> T` | Absolute value |
| `pow` | `(float, float) -> float` | Power |
| `sqrt` | `(float) -> float` | Square root |
| `inversesqrt` | `(float) -> float` | Inverse square root |
| `sin` | `(float) -> float` | Sine |
| `cos` | `(float) -> float` | Cosine |
| `reflect` | `(vecN, vecN) -> vecN` | Reflection vector |
| `floor` | `(T) -> T` | Floor |
| `fract` | `(T) -> T` | Fractional part |

### Built-in Variables

| Variable | Type | Shader | Description |
|----------|------|--------|-------------|
| `gl_Position` | `vec4` | Vertex | Clip-space output position |
| `gl_FragColor` | `vec4` | Fragment | Output fragment color |
| `gl_PointSize` | `float` | Vertex | Point size (stored, not used in Phase 1) |

### Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+` `-` `*` `/` |
| Comparison | `<` `>` `<=` `>=` `==` `!=` |
| Logical | `&&` `\|\|` `!` |
| Ternary | `? :` |
| Swizzle | `.xyzw` / `.rgba` / `.stpq` |
| Assignment | `=` `+=` `-=` `*=` `/=` |

### Limitations (Phase 1)

The following GLSL features are **not yet supported** and will be added in future phases:

- `if/else` statements (parsed but not lowered to IR branches)
- `for` / `while` loops
- `discard` statement
- `struct` types
- User-defined functions (only `main()` is executed)
- Array types and indexing
- `#define` / `#ifdef` preprocessor macros

---

## Constants Reference

### Capabilities (glEnable / glDisable)

| Constant | Value | Description |
|----------|-------|-------------|
| `GL_DEPTH_TEST` | `0x0B71` | Per-fragment depth testing |
| `GL_BLEND` | `0x0BE2` | Color blending |
| `GL_CULL_FACE` | `0x0B44` | Face culling |
| `GL_SCISSOR_TEST` | `0x0C11` | Scissor test |

### Depth Functions

| Constant | Value | Test |
|----------|-------|------|
| `GL_NEVER` | `0x0200` | Never pass |
| `GL_LESS` | `0x0201` | Pass if fragment < buffer |
| `GL_EQUAL` | `0x0202` | Pass if equal |
| `GL_LEQUAL` | `0x0203` | Pass if fragment <= buffer |
| `GL_GREATER` | `0x0204` | Pass if fragment > buffer |
| `GL_NOTEQUAL` | `0x0205` | Pass if not equal |
| `GL_GEQUAL` | `0x0206` | Pass if fragment >= buffer |
| `GL_ALWAYS` | `0x0207` | Always pass |

### Blend Factors

| Constant | Value |
|----------|-------|
| `GL_ZERO` | `0` |
| `GL_ONE` | `1` |
| `GL_SRC_ALPHA` | `0x0302` |
| `GL_ONE_MINUS_SRC_ALPHA` | `0x0303` |
| `GL_DST_ALPHA` | `0x0304` |
| `GL_ONE_MINUS_DST_ALPHA` | `0x0305` |
| `GL_SRC_COLOR` | `0x0300` |
| `GL_ONE_MINUS_SRC_COLOR` | `0x0301` |
| `GL_DST_COLOR` | `0x0306` |
| `GL_ONE_MINUS_DST_COLOR` | `0x0307` |

### Primitive Types

| Constant | Value | Description |
|----------|-------|-------------|
| `GL_TRIANGLES` | `0x0004` | Independent triangles (3 vertices each) |
| `GL_TRIANGLE_STRIP` | `0x0005` | Connected triangle strip |
| `GL_TRIANGLE_FAN` | `0x0006` | Triangle fan from first vertex |

### Pixel Formats

| Constant | Value | Bytes/pixel |
|----------|-------|-------------|
| `GL_ALPHA` | `0x1906` | 1 |
| `GL_RGB` | `0x1907` | 3 |
| `GL_RGBA` | `0x1908` | 4 |
| `GL_LUMINANCE` | `0x1909` | 1 |
| `GL_LUMINANCE_ALPHA` | `0x190A` | 2 |

---

## Constraints

| Resource | Limit | Notes |
|----------|-------|-------|
| Vertex attributes | 16 | Per-vertex attribute slots |
| Texture units | 8 | Bindable texture units (GL_TEXTURE0..7) |
| Uniform registers | 128 | Per-shader uniform [f32; 4] registers |
| Shader registers | 256 | Per-shader general-purpose registers |
| Framebuffer format | ARGB8888 | 32-bit color, 32-bit float depth |
| Max texture size | Memory-limited | No fixed maximum; limited by available heap |
| Primitives | Triangles only | POINTS, LINES not implemented in Phase 1 |
| Vertex data types | FLOAT, BYTE, UNSIGNED_BYTE, SHORT, UNSIGNED_SHORT | Supported in glVertexAttribPointer |

---

## Performance Optimization Summary

The software rasterizer has been tuned for maximum throughput under QEMU TCG emulation (which adds 5–10× overhead, especially for indirect function calls):

| Optimization | Location | Impact |
|-------------|----------|--------|
| JIT compilation | `backend_jit.rs` | Eliminates ~15-cycle branch misprediction per IR instruction |
| Fast-path rasterizer | `raster.rs` | Zero per-pixel function calls for textured+lit geometry |
| Scanline span clipping | `raster.rs` | Eliminates ~95% of rejected pixel iterations |
| Incremental edge functions | `raster.rs` | 3 additions/pixel vs 6 multiplications |
| SIMD Vec4 interpolation | `simd.rs` + `raster.rs` | 1 SSE instruction vs 4 scalar operations per varying |
| SSE fast reciprocal | `raster.rs` | ~4 cycles vs ~20 for division |
| Early depth test | `raster.rs` | Skips fragment shader for occluded pixels |
| Trivial frustum accept | `rasterizer/mod.rs` | Skips clipping (and its Vec allocation) for fully-visible triangles |
| Post-transform vertex cache | `rasterizer/mod.rs` | Avoids re-executing VS for shared vertices in indexed draws |
| Pre-divided varyings | `raster.rs` | Moves per-vertex division out of per-pixel loop |
| Gouraud (per-vertex) shading | Application-side | Lighting at ~187 vertices vs ~90K pixels (500× reduction) |

---

## Hardware-Accelerated 3D (GPU Driver HAL)

libgl supports hardware-accelerated 3D rendering via loadable userspace GPU drivers (`.drv` shared libraries). The driver is selected automatically at init based on the detected GPU hardware.

```
libgl.so
  |-- Software Rasterizer (always available)
  |   |-- JIT Backend (x86_64 SSE native code)
  |   |-- SW Backend (IR interpreter fallback)
  |   |-- Fast-Path Rasterizer (textured + vertex-lit)
  |
  |-- GPU Driver HAL (drv_loader.rs)
        |-- gl_init() → SYS_GPU_QUERY_TYPE → "svga3d" / "virgl" / "none"
        |-- dl_open("/System/Drivers/gpu/{type}.drv")
        |-- Resolves 21 drv_* symbols via ELF hash table
        |
        |-- svga3d.drv (VMware SVGA II)
        |     Translates drv_* → SVGA3D FIFO commands (cmd IDs 1040-1099)
        |     Shader format: DX9 Shader Model 2.0 bytecode
        |
        |-- virgl.drv (VirtIO GPU)
              Translates drv_* → Gallium3D/virgl command buffers
              Shader format: TGSI tokens
              Submitted via VIRTIO_GPU_CMD_SUBMIT_3D
```

### Driver API

Each `.drv` exports 21 `extern "C"` symbols covering lifecycle, resource management, shaders, render state, uniforms, drawing, and sync/present. See `docs/superpowers/specs/2026-03-13-gpu-driver-hal-design.md` for the full driver API reference.

### Driver Location

Drivers are installed to `sysroot/System/Drivers/gpu/`:
- `svga3d.drv` — VMware SVGA II 3D backend (`drivers/gpu/svga3d/`)
- `virgl.drv` — VirtIO GPU virgl backend (`drivers/gpu/virgl/`)

### Kernel Syscalls

| Syscall | Number | Purpose |
|---------|--------|---------|
| `SYS_GPU_QUERY_TYPE` | 517 | Returns GPU driver type name ("svga3d", "virgl", "none") |
| `SYS_GPU_3D_SUBMIT` | 512 | Submit 3D command buffer (up to 64 KiB) |
| `SYS_GPU_3D_SYNC` | 514 | Wait for GPU completion |
| `SYS_GPU_3D_SURFACE_DMA` | 515 | Upload data to GPU surface |
| `SYS_GPU_3D_SURFACE_DMA_READ` | 516 | Download data from GPU surface |
| `SYS_DLL_LOAD` | 80 | Load/map .drv shared object into process |

### QEMU Configuration

- **VMware SVGA3D**: `-vga vmware` (default SVGA II, 3D supported)
- **VirtIO GPU + virgl**: `-vga virtio -display gtk,gl=on`
- **Software only**: `-vga std` (Bochs VGA, no .drv loaded)

## Future Roadmap

- GLSL `if/else`, `for`, `while` with proper control flow (currently parsed but flattened)
- GLSL `discard` statement (currently parsed, no-op)
- User-defined GLSL functions
- `struct` types
- Mipmap generation
- Full FBO render-to-texture
- 2D GPU operations extracted into .drv drivers (stdvga fallback)
