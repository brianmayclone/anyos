# GPU Driver HAL: Userspace Driver Architecture

## Status: Implemented

This document describes the current GPU driver architecture as implemented. The system uses loadable userspace `.drv` shared libraries for GPU-specific 3D command translation, following the Windows ICD / Linux Mesa DRI model.

## Architecture Overview

```
App (glDrawArrays, glTexImage2D, ...)
    │
libgl.so  (GL API + Software Rasterizer + drv_loader)
    │── gl_init() → SYS_GPU_QUERY_TYPE → "svga3d" / "virgl" / "none"
    │── dl_open("/System/Drivers/gpu/{type}.drv")
    │── resolves all drv_* symbols via ELF hash table
    │
svga3d.drv  │  virgl.drv  │  (future: stdvga.drv)
    │── Translate drv_* calls → device-specific command buffers
    │── Call kernel 3D syscalls (SYS_GPU_3D_SUBMIT, etc.)
    │
Kernel: generic 3D syscalls
    │── submit_3d_commands() on GpuDriver trait
    │── Driver-specific validation (SVGA cmd IDs for svga3d, passthrough for virgl)
    │
GpuDriver trait implementations:
    │── VMware SVGA II → FIFO commands (1040-1099)
    │── VirtIO GPU    → VIRTIO_GPU_CMD_SUBMIT_3D (virgl context)
```

## Design Decisions

- **Userspace drivers, not kernel drivers**: Shader compilation and state tracking are complex; crashes in userspace don't take down the system.
- **Kernel provides generic syscalls**: Command buffer content is opaque to the kernel (except SVGA3D which validates cmd ID range). The kernel handles resource management, submission, and synchronization.
- **2D stays in kernel (for now)**: `SYS_GPU_COMMAND` for cursor, fill/copy rect, flip, update continues via the existing kernel compositor path. Only 3D acceleration goes through `.drv` files. A future redesign will extract 2D operations into the same driver model.
- **Software fallback**: If no `.drv` is found or loading fails, libGL falls back to its built-in software rasterizer silently.
- **Embedded ELF linker**: `drv_loader.rs` embeds a minimal ELF64 dynamic linker (SysV hash table lookup) to avoid depending on the `dynlink` crate, which pulls in `anyos_std`. Uses `libsyscall::dll_load()` (SYS_DLL_LOAD = 80) for the initial mapping.

## Driver Location

Drivers are installed to `sysroot/System/Drivers/gpu/`:

```
System/Drivers/gpu/
    svga3d.drv      VMware SVGA II 3D backend
    virgl.drv       VirtIO GPU virgl/Gallium3D backend
```

The kernel reports the driver type name via `SYS_GPU_QUERY_TYPE` (517), and libGL constructs the path: `/System/Drivers/gpu/{name}.drv`.

## Driver API (.drv Interface)

Each `.drv` exports these 21 `extern "C"` symbols:

### Lifecycle

```c
u32  drv_init(u32 width, u32 height)   // Returns 1 on success, 0 on failure
void drv_deinit()
```

### Resource Management

```c
u32  drv_create_surface(u32 format, u32 flags, u32 width, u32 height)  // Returns surface ID
void drv_destroy_surface(u32 sid)
u32  drv_surface_upload(u32 sid, *const u8 data, u32 len, u32 w, u32 h)
u32  drv_surface_download(u32 sid, *mut u8 buf, u32 len, u32 w, u32 h)
```

### Shaders

```c
u32  drv_create_shader(u32 type, u32 version, *const u8 bytecode, u32 len)  // Returns shader ID
void drv_destroy_shader(u32 shid)
u32  drv_link_program(u32 vs_id, u32 ps_id, u32 flags)  // Returns program ID
void drv_use_program(u32 program_id)
```

Shader bytecode format is driver-specific:
- **svga3d.drv**: DX9 Shader Model 2.0 bytecode (u32 words)
- **virgl.drv**: TGSI tokens (u32 words)

### Render State

```c
void drv_set_viewport(u32 x, u32 y, u32 w, u32 h)
void drv_set_blend(u32 enable, u32 src_factor, u32 dst_factor)
void drv_set_depth_test(u32 enable, u32 func)
void drv_clear(u32 flags, f32 r, f32 g, f32 b, f32 a, f32 depth)
```

### Uniforms

```c
void drv_set_uniform_f32(i32 location, u32 count, *const f32 values)  // count = number of vec4 registers
void drv_set_uniform_mat4(i32 location, *const f32 values)            // 16 floats = 4x4 matrix
```

### Drawing

```c
void drv_draw_arrays(u32 mode, u32 first, u32 count,
                     *const u8 vertex_data, u32 vertex_data_len,
                     *const DrvAttrib attribs, u32 num_attribs)

void drv_draw_elements(u32 mode, u32 count, u32 index_type,
                       *const u8 vertex_data,
                       *const u8 index_data, u32 index_data_len,
                       *const DrvAttrib attribs, u32 num_attribs)
```

### Sync & Present

```c
void drv_flush()
void drv_finish()
void drv_present(u32 surface_id)  // 0 = default color surface
```

### Shared Types

```c
struct DrvAttrib {  // #[repr(C)], 20 bytes
    u32 location;      // vertex attribute location
    u32 components;    // 1-4
    u32 attr_type;     // GL_FLOAT, GL_BYTE, etc.
    u32 offset;        // byte offset within vertex
    u32 normalized;    // 0 or 1
}
```

## Kernel Support

### Syscalls

| Syscall | Number | Purpose |
|---------|--------|---------|
| `SYS_GPU_QUERY_TYPE` | 517 | Returns GPU driver type name ("svga3d", "virgl", "none") |
| `SYS_GPU_3D_SUBMIT` | 512 | Submit command buffer (validated per driver type) |
| `SYS_GPU_3D_SYNC` | 514 | Wait for GPU completion |
| `SYS_GPU_3D_SURFACE_DMA` | 515 | Upload data to GPU surface |
| `SYS_GPU_3D_SURFACE_DMA_READ` | 516 | Download data from GPU surface |
| `SYS_GPU_3D_QUERY` | 513 | Query 3D capabilities |
| `SYS_DLL_LOAD` | 80 | Load/map .drv shared object into process |

### GpuDriver Trait (3D methods)

```rust
fn driver_type_name(&self) -> &str;                    // "svga3d", "virgl", "none"
fn has_3d(&self) -> bool;
fn hw_version_3d(&self) -> u32;
fn submit_3d_commands(&mut self, words: &[u32]) -> bool;
fn dma_surface_upload(&mut self, sid: u32, data: &[u8], w: u32, h: u32) -> bool;
fn dma_surface_download(&mut self, sid: u32, buf: &mut [u8], w: u32, h: u32) -> bool;
```

### SYS_GPU_3D_SUBMIT Validation

The syscall is generic but applies driver-specific validation:
- **svga3d**: Validates each command ID is in range 1040-1099, checks payload sizes
- **virgl**: Passes raw Gallium command words without structural validation (host renderer validates)
- Cap: 16384 words (64 KiB) per submission

### VirtIO GPU 3D Support

When `VIRTIO_GPU_F_VIRGL` (feature bit 1) is negotiated:
- `driver_type_name()` returns `"virgl"`
- `has_3d()` returns `true`
- `submit_3d_commands()` creates a virgl rendering context on first call, then submits via `VIRTIO_GPU_CMD_SUBMIT_3D`
- 64 KiB DMA buffer allocated at boot for 3D command submission
- Context management: `CTX_CREATE`, `CTX_DESTROY`, `CTX_ATTACH_RESOURCE`
- 3D resource commands: `RESOURCE_CREATE_3D`, `TRANSFER_TO_HOST_3D`, `TRANSFER_FROM_HOST_3D`

QEMU requires `-vga virtio -display gtk,gl=on` for virgl support.

## Implementation: svga3d.drv

**Location**: `drivers/gpu/svga3d/`

Translates drv_* calls into SVGA3D FIFO commands:
- `drv_init`: Creates context (CID=1), color surface (ARGB8888), depth surface (D24S8), binds render targets, sets viewport and default render states
- `drv_draw_arrays/elements`: Creates temporary VB/IB surfaces, uploads vertex/index data via `SYS_GPU_3D_SURFACE_DMA`, issues `CMD_DRAW_PRIMITIVES`, destroys temp surfaces
- `drv_use_program`: Encodes VS+PS shader IDs as `(vs << 16) | ps`, binds via `CMD_SET_SHADER`
- `drv_set_uniform_*`: Sets shader constants via `CMD_SET_SHADER_CONST` (float4 registers)
- `drv_present`: Issues `CMD_PRESENT` with full-screen copy rect

## Implementation: virgl.drv

**Location**: `drivers/gpu/virgl/`

Translates drv_* calls into Gallium3D/virgl command buffers:
- Command encoding: `(length << 16) | (object << 8) | cmd` header word + payload
- `drv_init`: Creates default blend, DSA, rasterizer objects; binds them; sets viewport
- `drv_create_shader`: `VIRGL_CCMD_CREATE_OBJECT(SHADER)` with inline TGSI tokens
- `drv_use_program`: `VIRGL_CCMD_BIND_SHADER` for VS and FS separately
- `drv_draw_arrays/elements`: `VIRGL_CCMD_DRAW_VBO`
- `drv_set_blend/depth_test`: Destroys old state object, creates new one, binds it
- `drv_clear`: `VIRGL_CCMD_CLEAR` with RGBA float + depth
- `drv_set_uniform_*`: `VIRGL_CCMD_SET_CONSTANT_BUFFER`

## libGL Integration

### drv_loader.rs

Embedded minimal ELF64 dynamic linker:
- `dl_open(path)`: Calls `libsyscall::dll_load()`, parses ELF header, extracts `.dynamic` section, builds symbol lookup tables (DT_SYMTAB, DT_STRTAB, DT_HASH)
- `dl_sym(handle, name)`: SysV ELF hash lookup, zero kernel interaction
- `resolve<T>(handle, name)`: Type-safe wrapper using `transmute_copy`

### Loading Flow

```rust
pub fn init(width: u32, height: u32) -> bool {
    let type_name = gpu_query_type();           // "svga3d", "virgl", "none"
    let path = "/System/Drivers/gpu/{type_name}.drv";
    let handle = dl_open(path)?;
    let drv = GpuDrv {
        drv_init: resolve(&handle, "drv_init")?,
        drv_deinit: resolve(&handle, "drv_deinit")?,
        // ... all 21 symbols
    };
    (drv.drv_init)(width, height);
    DRV = Some(drv);
}
```

### GL API Dispatch

```rust
pub fn glDrawArrays(mode, first, count) {
    if let Some(drv) = drv_loader::drv() {
        draw_arrays_hw(drv, mode, first, count);
    } else {
        draw_arrays_sw(mode, first, count);  // software rasterizer
    }
}
```

## Build System

### CMake (cmake/UserPrograms.cmake)

```cmake
function(add_gpu_driver NAME SRC_DIR)
    # Step 1: Cargo -> staticlib (.a)
    # Step 2: anyld -> .drv (ET_DYN shared object)
    # Step 3: Copy to sysroot/System/Drivers/gpu/
endfunction()

add_gpu_driver(svga3d ${CMAKE_SOURCE_DIR}/drivers/gpu/svga3d)
add_gpu_driver(virgl ${CMAKE_SOURCE_DIR}/drivers/gpu/virgl)
```

### Crate Structure

```
drivers/gpu/
    svga3d/
        Cargo.toml          name = "svga3d", staticlib
        exports.def         21 drv_* symbols
        src/lib.rs          SVGA3D command builder
    virgl/
        Cargo.toml          name = "virgl", staticlib
        exports.def         21 drv_* symbols
        src/lib.rs          Gallium/virgl command builder
```

Dependencies: `libheap` (allocator macro) + `libsyscall` (3D syscalls). No `anyos_std`, no `dynlink`.

## File Inventory

### New Files
- `drivers/gpu/svga3d/{Cargo.toml, exports.def, src/lib.rs}` — SVGA3D userspace driver
- `drivers/gpu/virgl/{Cargo.toml, exports.def, src/lib.rs}` — VirGL userspace driver
- `libs/libgl/src/drv_loader.rs` — ELF loader + driver symbol resolution

### Modified Files
- `libs/libgl/src/lib.rs` — Uses drv_loader instead of hardcoded svga3d
- `libs/libgl/src/draw.rs` — Hardware draw path calls drv_* via drv_loader
- `libs/libgl/src/syscall.rs` — Added `dll_load` re-export
- `libs/libsyscall/src/lib.rs` — Added `SYS_GPU_QUERY_TYPE` (517) + `gpu_query_type()`
- `kernel/src/drivers/gpu/mod.rs` — Added `driver_type_name()` to GpuDriver trait
- `kernel/src/drivers/gpu/vmware_svga.rs` — `driver_type_name()` returns "svga3d"
- `kernel/src/drivers/gpu/virtio_gpu.rs` — VIRGL feature negotiation, 3D commands, context management
- `kernel/src/syscall/mod.rs` — Added SYS_GPU_QUERY_TYPE dispatch
- `kernel/src/syscall/handlers/display.rs` — Generic `sys_gpu_3d_submit()`, `sys_gpu_query_type()`
- `cmake/UserPrograms.cmake` — `add_gpu_driver()` function for .drv builds

### Unchanged
- `libs/libgl/src/svga3d.rs` — Still exists as module, no longer called from lib.rs (legacy)
- All 2D GPU paths (`SYS_GPU_COMMAND`, compositor, cursor) — unchanged
