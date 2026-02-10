//! Export table for librender.dll.

use crate::color::Color;
use crate::rect::Rect;
use crate::surface::RenderSurface;
use crate::renderer::Renderer;

const NUM_EXPORTS: u32 = 18;

/// Export function table — must be first in the binary (`.exports` section).
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

#[link_section = ".exports"]
#[used]
#[no_mangle]
pub static LIBRENDER_EXPORTS: LibrenderExports = LibrenderExports {
    magic: *b"DLIB",
    version: 1,
    num_exports: NUM_EXPORTS,
    _pad: 0,
    fill_rect: export_fill_rect,
    fill_surface: export_fill_surface,
    put_pixel: export_put_pixel,
    get_pixel: export_get_pixel,
    blit_rect: export_blit_rect,
    put_pixel_subpixel: export_put_pixel_subpixel,
    fill_rounded_rect: export_fill_rounded_rect,
    fill_rounded_rect_aa: export_fill_rounded_rect_aa,
    fill_circle: export_fill_circle,
    fill_circle_aa: export_fill_circle_aa,
    draw_line: export_draw_line,
    draw_rect: export_draw_rect,
    draw_circle: export_draw_circle,
    draw_circle_aa: export_draw_circle_aa,
    draw_rounded_rect_aa: export_draw_rounded_rect_aa,
    fill_gradient_h: export_fill_gradient_h,
    fill_gradient_v: export_fill_gradient_v,
    blend_color: export_blend_color,
};

// ── Surface operations ──────────────────────────────────────

extern "C" fn export_fill_rect(
    pixels: *mut u32, width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    surf.fill_rect(Rect::new(x, y, w, h), Color::from_u32(color));
}

extern "C" fn export_fill_surface(pixels: *mut u32, width: u32, height: u32, color: u32) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    surf.fill(Color::from_u32(color));
}

extern "C" fn export_put_pixel(
    pixels: *mut u32, width: u32, height: u32,
    x: i32, y: i32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    surf.put_pixel(x, y, Color::from_u32(color));
}

extern "C" fn export_get_pixel(
    pixels: *const u32, width: u32, height: u32,
    x: i32, y: i32,
) -> u32 {
    let surf = unsafe { RenderSurface::from_raw(pixels as *mut u32, width, height) };
    surf.get_pixel(x, y).to_u32()
}

extern "C" fn export_blit_rect(
    dst: *mut u32, dw: u32, dh: u32, dx: i32, dy: i32,
    src: *const u32, sw: u32, sh: u32,
    sx: i32, sy: i32, cw: u32, ch: u32,
    src_opaque: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(dst, dw, dh) };
    surf.blit_rect(src, sw, sh, Rect::new(sx, sy, cw, ch), dx, dy, src_opaque != 0);
}

extern "C" fn export_put_pixel_subpixel(
    pixels: *mut u32, width: u32, height: u32,
    x: i32, y: i32, r_cov: u8, g_cov: u8, b_cov: u8, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    surf.put_pixel_subpixel(x, y, r_cov, g_cov, b_cov, Color::from_u32(color));
}

// ── Renderer primitives ──────────────────────────────────────

extern "C" fn export_fill_rounded_rect(
    pixels: *mut u32, width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32, radius: i32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).fill_rounded_rect(Rect::new(x, y, w, h), radius, Color::from_u32(color));
}

extern "C" fn export_fill_rounded_rect_aa(
    pixels: *mut u32, width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32, radius: i32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).fill_rounded_rect_aa(Rect::new(x, y, w, h), radius, Color::from_u32(color));
}

extern "C" fn export_fill_circle(
    pixels: *mut u32, width: u32, height: u32,
    cx: i32, cy: i32, radius: i32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).fill_circle(cx, cy, radius, Color::from_u32(color));
}

extern "C" fn export_fill_circle_aa(
    pixels: *mut u32, width: u32, height: u32,
    cx: i32, cy: i32, radius: i32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).fill_circle_aa(cx, cy, radius, Color::from_u32(color));
}

extern "C" fn export_draw_line(
    pixels: *mut u32, width: u32, height: u32,
    x0: i32, y0: i32, x1: i32, y1: i32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).draw_line(x0, y0, x1, y1, Color::from_u32(color));
}

extern "C" fn export_draw_rect(
    pixels: *mut u32, width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32, color: u32, thickness: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).draw_rect(Rect::new(x, y, w, h), Color::from_u32(color), thickness);
}

extern "C" fn export_draw_circle(
    pixels: *mut u32, width: u32, height: u32,
    cx: i32, cy: i32, radius: i32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).draw_circle(cx, cy, radius, Color::from_u32(color));
}

extern "C" fn export_draw_circle_aa(
    pixels: *mut u32, width: u32, height: u32,
    cx: i32, cy: i32, radius: i32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).draw_circle_aa(cx, cy, radius, Color::from_u32(color));
}

extern "C" fn export_draw_rounded_rect_aa(
    pixels: *mut u32, width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32, radius: i32, color: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).draw_rounded_rect_aa(Rect::new(x, y, w, h), radius, Color::from_u32(color));
}

extern "C" fn export_fill_gradient_h(
    pixels: *mut u32, width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32, color_left: u32, color_right: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).fill_gradient_h(
        Rect::new(x, y, w, h),
        Color::from_u32(color_left),
        Color::from_u32(color_right),
    );
}

extern "C" fn export_fill_gradient_v(
    pixels: *mut u32, width: u32, height: u32,
    x: i32, y: i32, w: u32, h: u32, color_top: u32, color_bottom: u32,
) {
    let mut surf = unsafe { RenderSurface::from_raw(pixels, width, height) };
    Renderer::new(&mut surf).fill_gradient_v(
        Rect::new(x, y, w, h),
        Color::from_u32(color_top),
        Color::from_u32(color_bottom),
    );
}

extern "C" fn export_blend_color(src: u32, dst: u32) -> u32 {
    Color::from_u32(src).blend_over(Color::from_u32(dst)).to_u32()
}
