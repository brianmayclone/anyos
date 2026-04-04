//! GPU driver loader — dynamically loads a `.drv` shared library based on
//! the active GPU type reported by the kernel.
//!
//! At `gl_init()` time, queries the kernel for the GPU driver type name
//! (e.g. "svga3d", "virgl") and loads `/Drivers/{name}.drv` via `dll_load`.
//! All `drv_*` symbols are resolved via ELF hash lookup and stored as function pointers.
//!
//! This module embeds a minimal ELF dynamic linker (symbol resolution only)
//! to avoid depending on the `dynlink` crate which pulls in `anyos_std`.

use core::mem::transmute_copy;

// ── Inline ELF64 structures ──────────────────────────────────────────────

#[repr(C)]
struct Elf64Ehdr {
    e_ident: [u8; 16],
    e_type: u16,
    _e_machine: u16,
    _e_version: u32,
    _e_entry: u64,
    e_phoff: u64,
    _e_shoff: u64,
    _e_flags: u32,
    _e_ehsize: u16,
    _e_phentsize: u16,
    e_phnum: u16,
    _e_shentsize: u16,
    _e_shnum: u16,
    _e_shstrndx: u16,
}

#[repr(C)]
struct Elf64Phdr {
    p_type: u32,
    _p_flags: u32,
    _p_offset: u64,
    p_vaddr: u64,
    _p_paddr: u64,
    _p_filesz: u64,
    _p_memsz: u64,
    _p_align: u64,
}

#[repr(C)]
struct Elf64Dyn {
    d_tag: i64,
    d_val: u64,
}

#[repr(C)]
struct Elf64Sym {
    st_name: u32,
    _st_info: u8,
    _st_other: u8,
    _st_shndx: u16,
    st_value: u64,
    _st_size: u64,
}

// ── Minimal dynamic linker ───────────────────────────────────────────────

/// Handle to a loaded shared library (ELF symbol lookup state).
struct DlHandle {
    _base: u64,
    symtab: *const Elf64Sym,
    strtab: *const u8,
    buckets: *const u32,
    chains: *const u32,
    nbuckets: u32,
}

unsafe impl Send for DlHandle {}
unsafe impl Sync for DlHandle {}

/// Load a shared library by path using SYS_DLL_LOAD and parse its ELF tables.
fn dl_open(path: &str) -> Option<DlHandle> {
    let base = crate::syscall::dll_load(path.as_bytes());
    if base == 0 {
        return None;
    }

    let ehdr = unsafe { &*(base as *const Elf64Ehdr) };

    // Validate ELF magic
    if ehdr.e_ident[0] != 0x7F || ehdr.e_ident[1] != b'E'
        || ehdr.e_ident[2] != b'L' || ehdr.e_ident[3] != b'F'
    {
        return None;
    }
    if ehdr.e_ident[4] != 2 { return None; } // ELF64
    if ehdr.e_type != 3 { return None; }      // ET_DYN

    let phdr_base = (base + ehdr.e_phoff) as *const Elf64Phdr;
    let mut dynamic_va: u64 = 0;
    let mut link_base: u64 = u64::MAX;

    for i in 0..ehdr.e_phnum as usize {
        let ph = unsafe { &*phdr_base.add(i) };
        if ph.p_type == 1 && ph.p_vaddr < link_base { // PT_LOAD
            link_base = ph.p_vaddr;
        }
        if ph.p_type == 2 { // PT_DYNAMIC
            dynamic_va = ph.p_vaddr;
        }
    }
    if dynamic_va == 0 { return None; }

    let load_bias = if link_base != u64::MAX { base - link_base } else { 0 };
    dynamic_va += load_bias;

    let mut symtab_va: u64 = 0;
    let mut strtab_va: u64 = 0;
    let mut hash_va: u64 = 0;

    let dyn_ptr = dynamic_va as *const Elf64Dyn;
    for i in 0..128 {
        let d = unsafe { &*dyn_ptr.add(i) };
        match d.d_tag {
            6 => symtab_va = d.d_val, // DT_SYMTAB
            5 => strtab_va = d.d_val, // DT_STRTAB
            4 => hash_va = d.d_val,   // DT_HASH
            0 => break,               // DT_NULL
            _ => {}
        }
    }
    if symtab_va == 0 || strtab_va == 0 || hash_va == 0 { return None; }

    let hash_ptr = hash_va as *const u32;
    let nbuckets = unsafe { *hash_ptr };
    let buckets = unsafe { hash_ptr.add(2) };
    let chains = unsafe { buckets.add(nbuckets as usize) };

    Some(DlHandle {
        _base: base,
        symtab: symtab_va as *const Elf64Sym,
        strtab: strtab_va as *const u8,
        buckets,
        chains,
        nbuckets,
    })
}

/// Look up a symbol by name using ELF hash table.
fn dl_sym(handle: &DlHandle, name: &str) -> Option<*const ()> {
    let h = elf_hash(name.as_bytes());
    let bucket_idx = h % handle.nbuckets;
    let mut idx = unsafe { *handle.buckets.add(bucket_idx as usize) };

    while idx != 0 {
        let sym = unsafe { &*handle.symtab.add(idx as usize) };
        if sym.st_value != 0 {
            if unsafe { cstr_eq(handle.strtab.add(sym.st_name as usize), name.as_bytes()) } {
                return Some(sym.st_value as *const ());
            }
        }
        idx = unsafe { *handle.chains.add(idx as usize) };
    }
    None
}

fn elf_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in name {
        h = (h << 4).wrapping_add(b as u32);
        let g = h & 0xF000_0000;
        if g != 0 { h ^= g >> 24; }
        h &= !g;
    }
    h
}

unsafe fn cstr_eq(cstr: *const u8, name: &[u8]) -> bool {
    for (i, &b) in name.iter().enumerate() {
        if unsafe { *cstr.add(i) } != b { return false; }
    }
    unsafe { *cstr.add(name.len()) == 0 }
}

// ── GPU driver interface ─────────────────────────────────────────────────

/// Vertex attribute descriptor passed to drv_draw_arrays / drv_draw_elements.
#[repr(C)]
pub struct DrvAttrib {
    pub location: u32,
    pub components: u32,
    pub attr_type: u32,
    pub offset: u32,
    pub normalized: u32,
}

/// Resolved function pointers from a loaded `.drv` shared library.
pub struct GpuDrv {
    _handle: DlHandle,

    // Lifecycle
    pub drv_init: extern "C" fn(u32, u32) -> u32,
    pub drv_deinit: extern "C" fn(),

    // Resources
    pub drv_create_surface: extern "C" fn(u32, u32, u32, u32) -> u32,
    pub drv_destroy_surface: extern "C" fn(u32),
    pub drv_surface_upload: extern "C" fn(u32, *const u8, u32, u32, u32) -> u32,
    pub drv_surface_download: extern "C" fn(u32, *mut u8, u32, u32, u32) -> u32,

    // Shaders
    pub drv_create_shader: extern "C" fn(u32, u32, *const u8, u32) -> u32,
    pub drv_destroy_shader: extern "C" fn(u32),
    pub drv_link_program: extern "C" fn(u32, u32, u32) -> u32,
    pub drv_use_program: extern "C" fn(u32),

    // State
    pub drv_set_viewport: extern "C" fn(u32, u32, u32, u32),
    pub drv_set_blend: extern "C" fn(u32, u32, u32),
    pub drv_set_depth_test: extern "C" fn(u32, u32),
    pub drv_clear: extern "C" fn(u32, f32, f32, f32, f32, f32),

    // Uniforms
    pub drv_set_uniform_f32: extern "C" fn(i32, u32, *const f32),
    pub drv_set_uniform_mat4: extern "C" fn(i32, *const f32),

    // Draw
    pub drv_draw_arrays: extern "C" fn(u32, u32, u32, *const u8, u32, *const DrvAttrib, u32),
    pub drv_draw_elements: extern "C" fn(u32, u32, u32, *const u8, *const u8, u32, *const DrvAttrib, u32),

    // Sync & Present
    pub drv_flush: extern "C" fn(),
    pub drv_finish: extern "C" fn(),
    pub drv_present: extern "C" fn(u32),

    // Readback (optional — NULL if not supported)
    pub drv_readback: Option<extern "C" fn(*mut u8, u32) -> u32>,

    // Resize render target (optional — NULL if not supported)
    pub drv_resize: Option<extern "C" fn(u32, u32) -> u32>,

    // Texture upload: (gl_tex_id, data_ptr, byte_len, width, height) -> virgl_res_id
    // Returns 0 on failure. Caches by gl_tex_id (subsequent calls return cached id).
    pub drv_upload_texture: Option<extern "C" fn(u32, *const u8, u32, u32, u32) -> u32>,

    // Sampler view bind: (slot, virgl_res_id) — binds texture resource to FS SAMP[slot]
    pub drv_bind_sampler_view: Option<extern "C" fn(u32, u32)>,

    // Persistent VBO: upload once, draw many times
    // drv_upload_vbo(data_ptr, data_len, stride) → gpu_res_id (0 on failure)
    pub drv_upload_vbo: Option<extern "C" fn(*const u8, u32, u32) -> u32>,
    // drv_draw_vbo(mode, first, count, gpu_res_id, stride, attribs, num_attribs)
    pub drv_draw_vbo: Option<extern "C" fn(u32, u32, u32, u32, u32, *const DrvAttrib, u32)>,
    // drv_destroy_vbo(gpu_res_id)
    pub drv_destroy_vbo: Option<extern "C" fn(u32)>,

    // Shadow mapping (GL_OES_depth_texture / ES 2.0 FBO)
    pub drv_shadow_begin: Option<extern "C" fn(u32, u32) -> u32>,
    pub drv_shadow_end: Option<extern "C" fn()>,
    pub drv_shadow_bind: Option<extern "C" fn(u32)>,
}

static mut DRV: Option<GpuDrv> = None;

/// Get a reference to the loaded GPU driver. Returns `None` if no driver is loaded.
pub fn drv() -> Option<&'static GpuDrv> {
    unsafe { DRV.as_ref() }
}

/// Check if a GPU driver is loaded and active.
pub fn is_loaded() -> bool {
    unsafe { DRV.is_some() }
}

/// Unload the GPU driver (calls drv_deinit).
pub fn deinit() {
    unsafe {
        if let Some(ref d) = DRV {
            (d.drv_deinit)();
        }
        DRV = None;
    }
}

/// Resolve a single symbol from a DlHandle, returning None on failure.
unsafe fn resolve<T>(handle: &DlHandle, name: &str) -> Option<T> {
    let ptr = dl_sym(handle, name)?;
    Some(transmute_copy::<*const (), T>(&ptr))
}

unsafe fn resolve_opt<T>(handle: &DlHandle, name: &str) -> Option<T> {
    let ptr = dl_sym(handle, name)?;
    Some(transmute_copy::<*const (), T>(&ptr))
}

/// Query the kernel for the GPU type and load the corresponding `.drv`.
/// Returns true if a driver was successfully loaded.
pub fn init(width: u32, height: u32) -> bool {
    init_inner(width, height).is_some()
}

fn init_inner(width: u32, height: u32) -> Option<()> {
    // Query kernel for GPU driver type
    let mut buf = [0u8; 32];
    let len = crate::syscall::gpu_query_type(&mut buf);
    if len == 0 {
        return None;
    }

    // Extract type name (null-terminated string)
    let name_end = buf.iter().position(|&b| b == 0).unwrap_or(len as usize);
    let type_name = core::str::from_utf8(&buf[..name_end]).unwrap_or("none");

    if type_name == "none" {
        return None;
    }

    // Build driver path: "/System/Drivers/gpu/{type_name}.drv"
    let mut path_buf = [0u8; 80];
    let prefix = b"/System/Drivers/gpu/";
    let suffix = b".drv";
    let total_len = prefix.len() + type_name.len() + suffix.len();
    if total_len >= path_buf.len() {
        return None;
    }
    path_buf[..prefix.len()].copy_from_slice(prefix);
    path_buf[prefix.len()..prefix.len() + type_name.len()].copy_from_slice(type_name.as_bytes());
    path_buf[prefix.len() + type_name.len()..total_len].copy_from_slice(suffix);
    let path = core::str::from_utf8(&path_buf[..total_len]).unwrap_or("");

    crate::serial_println!("[libgl] Loading GPU driver: {}", path);

    // Load the .drv shared library
    let handle = match dl_open(path) {
        Some(h) => h,
        None => {
            crate::serial_println!("[libgl] Failed to load {}", path);
            return None;
        }
    };

    // Resolve all drv_* symbols
    unsafe {
        let drv = GpuDrv {
            drv_init:             resolve(&handle, "drv_init")?,
            drv_deinit:           resolve(&handle, "drv_deinit")?,
            drv_create_surface:   resolve(&handle, "drv_create_surface")?,
            drv_destroy_surface:  resolve(&handle, "drv_destroy_surface")?,
            drv_surface_upload:   resolve(&handle, "drv_surface_upload")?,
            drv_surface_download: resolve(&handle, "drv_surface_download")?,
            drv_create_shader:    resolve(&handle, "drv_create_shader")?,
            drv_destroy_shader:   resolve(&handle, "drv_destroy_shader")?,
            drv_link_program:     resolve(&handle, "drv_link_program")?,
            drv_use_program:      resolve(&handle, "drv_use_program")?,
            drv_set_viewport:     resolve(&handle, "drv_set_viewport")?,
            drv_set_blend:        resolve(&handle, "drv_set_blend")?,
            drv_set_depth_test:   resolve(&handle, "drv_set_depth_test")?,
            drv_clear:            resolve(&handle, "drv_clear")?,
            drv_set_uniform_f32:  resolve(&handle, "drv_set_uniform_f32")?,
            drv_set_uniform_mat4: resolve(&handle, "drv_set_uniform_mat4")?,
            drv_draw_arrays:      resolve(&handle, "drv_draw_arrays")?,
            drv_draw_elements:    resolve(&handle, "drv_draw_elements")?,
            drv_flush:            resolve(&handle, "drv_flush")?,
            drv_finish:           resolve(&handle, "drv_finish")?,
            drv_present:          resolve(&handle, "drv_present")?,
            drv_readback:         resolve_opt(&handle, "drv_readback"),
            drv_resize:           resolve_opt(&handle, "drv_resize"),
            drv_upload_texture:   resolve_opt(&handle, "drv_upload_texture"),
            drv_bind_sampler_view: resolve_opt(&handle, "drv_bind_sampler_view"),
            drv_upload_vbo:       resolve_opt(&handle, "drv_upload_vbo"),
            drv_draw_vbo:         resolve_opt(&handle, "drv_draw_vbo"),
            drv_destroy_vbo:      resolve_opt(&handle, "drv_destroy_vbo"),
            drv_shadow_begin:     resolve_opt(&handle, "drv_shadow_begin"),
            drv_shadow_end:       resolve_opt(&handle, "drv_shadow_end"),
            drv_shadow_bind:      resolve_opt(&handle, "drv_shadow_bind"),
            _handle: handle,
        };

        // Initialize the driver
        let result = (drv.drv_init)(width, height);
        if result == 0 {
            crate::serial_println!("[libgl] drv_init failed");
            return None;
        }

        DRV = Some(drv);
    }

    crate::serial_println!("[libgl] GPU driver '{}' loaded successfully", type_name);
    Some(())
}
