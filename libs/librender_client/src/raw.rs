//! Raw FFI bindings to librender.dlib export table.

/// Base virtual address where librender.dlib is loaded.
const LIBRENDER_BASE: usize = 0x0430_0000;

/// Export function table — must match the DLL's `LibrenderExports` layout exactly.
///
/// ABI layout (x86_64, `#[repr(C)]`):
///   offset   0: magic [u8; 4]
///   offset   4: version u32
///   offset   8: num_exports u32
///   offset  12: _pad u32
///   offset  16: fill_rect fn ptr (8 bytes)
///   offset  24: fill_surface fn ptr
///   offset  32: put_pixel fn ptr
///   offset  40: get_pixel fn ptr
///   offset  48: blit_rect fn ptr
///   offset  56: put_pixel_subpixel fn ptr
///   offset  64: fill_rounded_rect fn ptr
///   offset  72: fill_rounded_rect_aa fn ptr
///   offset  80: fill_circle fn ptr
///   offset  88: fill_circle_aa fn ptr
///   offset  96: draw_line fn ptr
///   offset 104: draw_rect fn ptr
///   offset 112: draw_circle fn ptr
///   offset 120: draw_circle_aa fn ptr
///   offset 128: draw_rounded_rect_aa fn ptr
///   offset 136: fill_gradient_h fn ptr
///   offset 144: fill_gradient_v fn ptr
///   offset 152: blend_color fn ptr
#[repr(C)]
pub struct LibrenderExports {
    pub magic: [u8; 4],
    pub version: u32,
    pub num_exports: u32,
    pub _pad: u32,
    // Surface operations
    pub fill_rect: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u32),
    pub fill_surface: extern "C" fn(*mut u32, u32, u32, u32),
    pub put_pixel: extern "C" fn(*mut u32, u32, u32, i32, i32, u32),
    pub get_pixel: extern "C" fn(*const u32, u32, u32, i32, i32) -> u32,
    pub blit_rect: extern "C" fn(*mut u32, u32, u32, i32, i32, *const u32, u32, u32, i32, i32, u32, u32, u32),
    pub put_pixel_subpixel: extern "C" fn(*mut u32, u32, u32, i32, i32, u8, u8, u8, u32),
    // Renderer primitives
    pub fill_rounded_rect: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, i32, u32),
    pub fill_rounded_rect_aa: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, i32, u32),
    pub fill_circle: extern "C" fn(*mut u32, u32, u32, i32, i32, i32, u32),
    pub fill_circle_aa: extern "C" fn(*mut u32, u32, u32, i32, i32, i32, u32),
    pub draw_line: extern "C" fn(*mut u32, u32, u32, i32, i32, i32, i32, u32),
    pub draw_rect: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u32, u32),
    pub draw_circle: extern "C" fn(*mut u32, u32, u32, i32, i32, i32, u32),
    pub draw_circle_aa: extern "C" fn(*mut u32, u32, u32, i32, i32, i32, u32),
    pub draw_rounded_rect_aa: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, i32, u32),
    pub fill_gradient_h: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u32, u32),
    pub fill_gradient_v: extern "C" fn(*mut u32, u32, u32, i32, i32, u32, u32, u32, u32),
    pub blend_color: extern "C" fn(u32, u32) -> u32,
}

/// Get a reference to the DLL export table at the fixed load address.
pub fn exports() -> &'static LibrenderExports {
    unsafe { &*(LIBRENDER_BASE as *const LibrenderExports) }
}
