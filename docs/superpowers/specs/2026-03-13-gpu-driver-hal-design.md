# GPU Driver HAL: Userspace 3D Driver Architecture

## Summary

Replace the hardcoded SVGA3D backend in libGL with a loadable userspace driver model. libGL queries the kernel for the active GPU type and dynamically loads a matching `.drv` shared library (e.g., `svga3d.drv`, `virgl.drv`) that translates GL operations into device-specific GPU commands. The kernel provides generic 3D syscalls; all hardware-specific translation happens in userspace drivers.

## Architecture

```
App (glDrawArrays, glTexImage2D, ...)
    |
libgl.so  (GL API + Software Rasterizer)
    |-- gl_init() calls gpu_query_type() syscall
    |-- dl_open("/Drivers/{type}.drv")
    |-- calls drv_* functions via dl_sym()
    |
svga3d.drv  |  virgl.drv     (Userspace drivers)
    |-- Translate GL -> device-specific command buffers
    |-- Call generic kernel 3D syscalls
    |
Kernel: generic 3D syscalls
    |-- gpu_3d_submit(opaque command buffer)
    |-- gpu_3d_resource_create/destroy
    |-- gpu_3d_sync()
    |
GpuDriver trait (vmware_svga | virtio_gpu)
```

### Design Decisions

- **Userspace drivers, not kernel drivers**: Shader compilation and state tracking are complex; crashes in userspace don't take down the system. This follows the Windows ICD / Linux Mesa DRI model.
- **Kernel provides generic syscalls**: The command buffer content is opaque to the kernel. The kernel handles resource management, command submission, and synchronization.
- **2D stays unchanged**: `SYS_GPU_COMMAND` for cursor, fill/copy rect, flip, update continues via the existing kernel compositor path. Only 3D acceleration goes through `.drv` files.
- **Software fallback**: If no `.drv` is found or loading fails, libGL falls back to its built-in software rasterizer silently.

## Driver API (.drv Interface)

Each `.drv` exports these `extern "C"` symbols:

### Lifecycle

```rust
drv_init(width: u32, height: u32) -> bool
drv_deinit()
```

### Resource Management

```rust
drv_create_surface(id: u32, w: u32, h: u32, format: u32) -> bool
drv_destroy_surface(id: u32)
drv_surface_upload(id: u32, data: *const u8, len: u32, w: u32, h: u32) -> bool
drv_surface_download(id: u32, buf: *mut u8, len: u32, w: u32, h: u32) -> bool
```

### Shaders

GLSL source is passed directly to the driver. Each driver compiles it to its native format internally (SVGA3D: DX9 bytecode, virgl: TGSI).

```rust
drv_create_shader(id: u32, shader_type: u32, glsl_source: *const u8, len: u32) -> bool
drv_destroy_shader(id: u32)
drv_link_program(program_id: u32, vs_id: u32, fs_id: u32) -> bool
drv_use_program(program_id: u32)
```

### Render State

```rust
drv_set_viewport(x: u32, y: u32, w: u32, h: u32)
drv_set_blend(enabled: bool, src: u32, dst: u32)
drv_set_depth_test(enabled: bool, func: u32)
drv_clear(mask: u32, r: f32, g: f32, b: f32, a: f32, depth: f32)
```

### Uniforms

```rust
drv_set_uniform_f32(location: i32, count: u32, data: *const f32)
drv_set_uniform_mat4(location: i32, data: *const f32)
```

### Drawing

```rust
drv_draw_arrays(mode: u32, first: u32, count: u32,
                vertices: *const u8, vertex_stride: u32,
                attribs: *const DrvAttrib, attrib_count: u32)
drv_draw_elements(mode: u32, count: u32, index_type: u32, indices: *const u8,
                  vertices: *const u8, vertex_stride: u32,
                  attribs: *const DrvAttrib, attrib_count: u32)
```

### Sync & Present

```rust
drv_flush()
drv_finish()          // blocking sync
drv_present(target_surface: u32)  // blit result to scanout
```

### Shared Types

```rust
#[repr(C)]
struct DrvAttrib {
    location: u32,
    components: u32,  // 1-4
    attr_type: u32,   // GL_FLOAT, GL_BYTE, etc.
    offset: u32,      // byte offset within vertex
    normalized: bool,
}
```

## Kernel Changes

### New Syscall: SYS_GPU_QUERY_TYPE

Writes the active GPU driver name as a null-terminated string to a user buffer.

- Returns: `"svga3d"`, `"virgl"`, `"none"`
- libGL uses the result to construct the driver path: `/Drivers/{name}.drv`

### New GpuDriver Trait Method

```rust
fn driver_type_name(&self) -> &str;
```

Returns the driver identifier used for `.drv` loading. Implementations:
- `VmwareSvgaGpu` -> `"svga3d"`
- `VirtioGpu` (with F_VIRGL) -> `"virgl"`
- `VirtioGpu` (without F_VIRGL) -> `"none"`
- `BochsVga` -> `"none"`

### Existing 3D Syscalls (unchanged)

- `SYS_GPU_3D_SUBMIT` -- opaque command buffer passthrough
- `SYS_GPU_3D_SYNC` -- wait for GPU completion
- `SYS_GPU_3D_SURFACE_DMA` / `DMA_READ` -- resource upload/download
- `SYS_GPU_3D_QUERY` -- has_3d, hw_version

### Virtio-GPU Driver Extensions (for virgl)

- Negotiate `VIRTIO_GPU_F_VIRGL` feature bit
- Implement `VIRTIO_GPU_CMD_SUBMIT_3D` (opaque virgl command buffer)
- Implement `VIRTIO_GPU_CMD_RESOURCE_CREATE_3D` (replaces CREATE_2D for 3D resources)
- `has_3d()` returns true when F_VIRGL negotiated
- `driver_type_name()` returns `"virgl"`

## libGL Changes

### New Module: drv_loader.rs

Uses the existing `dll_exports!` macro pattern to load `.drv` files:

```rust
dynlink::dll_exports! {
    lib_path: dynamic,  // set at runtime based on gpu_query_type()
    lib_struct: GpuDrv,
    symbols: {
        drv_init(width: u32, height: u32) -> bool,
        drv_deinit() -> (),
        drv_create_surface(id: u32, w: u32, h: u32, format: u32) -> bool,
        // ... all drv_* symbols
    }
}
```

### Modified gl_init() Flow

1. Allocate software framebuffer (unchanged)
2. Call `gpu_query_type()` syscall -> e.g., `"svga3d"`
3. `dl_open("/Drivers/svga3d.drv")` -> resolve all `drv_*` symbols
4. Call `drv_init(width, height)`
5. If driver loaded successfully: `HW_BACKEND = true`
6. If loading failed: software rasterizer fallback, no error

### Modified Draw Path

```rust
pub fn glDrawArrays(mode: u32, first: u32, count: u32) {
    if HW_BACKEND {
        drv().drv_draw_arrays(mode, first, count, vertex_data, stride, attribs, n);
    } else {
        single_threaded_draw(...);
    }
}
```

### Code Migration

- Existing `svga3d.rs` module in libGL -> moves into `svga3d.drv` crate
- `static mut SVGA3D: Option<Svga3dGpu>` removed from libGL
- `USE_HW_BACKEND` replaced by `drv_loader` state

### Preserved Public API

- `gl_set_hw_backend(enabled: bool)` -- true: attempt driver load if not loaded; false: disable HW, use software
- `gl_has_hw_backend() -> bool` -- returns whether a .drv is loaded and active

## File Structure

```
libs/
  libgl/src/
    drv_loader.rs     (NEW - dll_exports! for .drv loading)
    lib.rs            (MODIFIED - use drv_loader instead of svga3d)
    svga3d.rs         (REMOVED - code moves to svga3d.drv)
  libdrv_svga3d/      (NEW - svga3d.drv crate)
    Cargo.toml
    src/lib.rs        (drv_* exports, SVGA3D command builder)
  libdrv_virgl/       (NEW - virgl.drv crate)
    Cargo.toml
    src/lib.rs        (drv_* exports, virgl/Gallium command builder)
kernel/src/
  drivers/gpu/mod.rs          (MODIFIED - add driver_type_name() to trait)
  drivers/gpu/virtio_gpu.rs   (MODIFIED - F_VIRGL, SUBMIT_3D, CREATE_3D)
  syscall/handlers/display.rs (MODIFIED - add SYS_GPU_QUERY_TYPE)
```

## Implementation Order

1. **Kernel**: Add `driver_type_name()` to GpuDriver trait + `SYS_GPU_QUERY_TYPE` syscall
2. **libGL**: Add `drv_loader.rs`, refactor `gl_init()` and draw paths to use it
3. **svga3d.drv**: Extract existing `svga3d.rs` code into standalone `.drv` crate
4. **Kernel**: Extend virtio-gpu driver with F_VIRGL + SUBMIT_3D
5. **virgl.drv**: Implement virgl command builder (TGSI shaders, Gallium state)

Steps 1-3 maintain existing SVGA3D functionality through the new architecture. Steps 4-5 add virgl support.
