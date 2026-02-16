//! Client library for librender.dlib — 2D rendering primitives.
//!
//! Provides safe Rust wrappers around the raw DLL export functions.
//! All operations work on caller-provided ARGB8888 pixel buffers.

#![no_std]

pub mod raw;

/// An ARGB8888 pixel surface descriptor for rendering operations.
///
/// Does NOT own the pixel data. The caller must ensure the buffer
/// remains valid for the lifetime of this struct.
pub struct Surface {
    pub pixels: *mut u32,
    pub width: u32,
    pub height: u32,
}

impl Surface {
    /// Create a surface descriptor from a raw pixel buffer.
    ///
    /// # Safety
    /// `pixels` must point to a buffer of at least `width * height` u32 elements.
    pub unsafe fn from_raw(pixels: *mut u32, width: u32, height: u32) -> Self {
        Surface { pixels, width, height }
    }

    /// Fill the entire surface with a color.
    pub fn fill(&mut self, color: u32) {
        (raw::exports().fill_surface)(self.pixels, self.width, self.height, color);
    }

    /// Fill a rectangle with a color.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: u32) {
        (raw::exports().fill_rect)(self.pixels, self.width, self.height, x, y, w, h, color);
    }

    /// Set a pixel with alpha blending.
    pub fn put_pixel(&mut self, x: i32, y: i32, color: u32) {
        (raw::exports().put_pixel)(self.pixels, self.width, self.height, x, y, color);
    }

    /// Get a pixel value.
    pub fn get_pixel(&self, x: i32, y: i32) -> u32 {
        (raw::exports().get_pixel)(self.pixels as *const u32, self.width, self.height, x, y)
    }

    /// Blit a rectangular region from `src` onto this surface.
    pub fn blit_rect(
        &mut self,
        dx: i32,
        dy: i32,
        src: *const u32,
        sw: u32,
        sh: u32,
        sx: i32,
        sy: i32,
        cw: u32,
        ch: u32,
        src_opaque: bool,
    ) {
        (raw::exports().blit_rect)(
            self.pixels, self.width, self.height, dx, dy,
            src, sw, sh, sx, sy, cw, ch,
            if src_opaque { 1 } else { 0 },
        );
    }

    /// Set a pixel with LCD subpixel rendering.
    pub fn put_pixel_subpixel(&mut self, x: i32, y: i32, r_cov: u8, g_cov: u8, b_cov: u8, color: u32) {
        (raw::exports().put_pixel_subpixel)(self.pixels, self.width, self.height, x, y, r_cov, g_cov, b_cov, color);
    }

    /// Fill a rounded rectangle.
    pub fn fill_rounded_rect(&mut self, x: i32, y: i32, w: u32, h: u32, radius: i32, color: u32) {
        (raw::exports().fill_rounded_rect)(self.pixels, self.width, self.height, x, y, w, h, radius, color);
    }

    /// Fill a rounded rectangle with anti-aliased edges.
    pub fn fill_rounded_rect_aa(&mut self, x: i32, y: i32, w: u32, h: u32, radius: i32, color: u32) {
        (raw::exports().fill_rounded_rect_aa)(self.pixels, self.width, self.height, x, y, w, h, radius, color);
    }

    /// Fill a solid circle.
    pub fn fill_circle(&mut self, cx: i32, cy: i32, radius: i32, color: u32) {
        (raw::exports().fill_circle)(self.pixels, self.width, self.height, cx, cy, radius, color);
    }

    /// Fill a circle with anti-aliased edges.
    pub fn fill_circle_aa(&mut self, cx: i32, cy: i32, radius: i32, color: u32) {
        (raw::exports().fill_circle_aa)(self.pixels, self.width, self.height, cx, cy, radius, color);
    }

    /// Draw a line between two points.
    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        (raw::exports().draw_line)(self.pixels, self.width, self.height, x0, y0, x1, y1, color);
    }

    /// Draw a rectangular outline.
    pub fn draw_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: u32, thickness: u32) {
        (raw::exports().draw_rect)(self.pixels, self.width, self.height, x, y, w, h, color, thickness);
    }

    /// Draw a circle outline.
    pub fn draw_circle(&mut self, cx: i32, cy: i32, radius: i32, color: u32) {
        (raw::exports().draw_circle)(self.pixels, self.width, self.height, cx, cy, radius, color);
    }

    /// Draw an anti-aliased circle outline.
    pub fn draw_circle_aa(&mut self, cx: i32, cy: i32, radius: i32, color: u32) {
        (raw::exports().draw_circle_aa)(self.pixels, self.width, self.height, cx, cy, radius, color);
    }

    /// Draw a 1px rounded rectangle outline with anti-aliased corners.
    pub fn draw_rounded_rect_aa(&mut self, x: i32, y: i32, w: u32, h: u32, radius: i32, color: u32) {
        (raw::exports().draw_rounded_rect_aa)(self.pixels, self.width, self.height, x, y, w, h, radius, color);
    }

    /// Fill a horizontal gradient.
    pub fn fill_gradient_h(&mut self, x: i32, y: i32, w: u32, h: u32, color_left: u32, color_right: u32) {
        (raw::exports().fill_gradient_h)(self.pixels, self.width, self.height, x, y, w, h, color_left, color_right);
    }

    /// Fill a vertical gradient.
    pub fn fill_gradient_v(&mut self, x: i32, y: i32, w: u32, h: u32, color_top: u32, color_bottom: u32) {
        (raw::exports().fill_gradient_v)(self.pixels, self.width, self.height, x, y, w, h, color_top, color_bottom);
    }
}

/// Alpha-blend two ARGB8888 colors (src over dst).
pub fn blend_color(src: u32, dst: u32) -> u32 {
    (raw::exports().blend_color)(src, dst)
}
