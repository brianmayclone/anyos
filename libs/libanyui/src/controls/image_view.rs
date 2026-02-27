//! ImageView — displays decoded ARGB pixel data.
//!
//! The client-side library handles image file I/O and decoding via libimage.
//! The server (this DLL) stores and blits the pre-decoded pixel buffer.

use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind};

/// Scale mode for ImageView.
pub const SCALE_NONE: u32 = 0;    // original size, top-left aligned
pub const SCALE_FIT: u32 = 1;     // scale to fit, preserve aspect (letterbox)
pub const SCALE_FILL: u32 = 2;    // scale to fill, preserve aspect (crop)
pub const SCALE_STRETCH: u32 = 3; // stretch to fill control bounds

pub struct ImageView {
    pub(crate) base: ControlBase,
    /// Decoded ARGB pixel buffer (original, unscaled).
    pub(crate) pixels: Vec<u32>,
    /// Original image dimensions.
    pub(crate) img_w: u32,
    pub(crate) img_h: u32,
    /// Scale mode: 0=None, 1=Fit, 2=Fill, 3=Stretch.
    pub(crate) scale_mode: u32,
}

impl ImageView {
    pub fn new(base: ControlBase) -> Self {
        Self {
            base,
            pixels: Vec::new(),
            img_w: 0,
            img_h: 0,
            scale_mode: SCALE_FIT,
        }
    }

    /// Set pixel data from decoded ARGB buffer.
    pub fn set_pixels(&mut self, data: &[u32], w: u32, h: u32) {
        let expected = (w as usize) * (h as usize);
        if data.len() < expected {
            return;
        }
        self.pixels.clear();
        self.pixels.extend_from_slice(&data[..expected]);
        self.img_w = w;
        self.img_h = h;
        self.base.mark_dirty();
    }

    /// Clear pixel data.
    pub fn clear(&mut self) {
        self.pixels.clear();
        self.img_w = 0;
        self.img_h = 0;
        self.base.mark_dirty();
    }
}

impl Control for ImageView {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::ImageView }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let cw = self.base.w;
        let ch = self.base.h;

        if self.pixels.is_empty() || self.img_w == 0 || self.img_h == 0 {
            // No image loaded — draw placeholder
            crate::draw::fill_rect(surface, x, y, cw, ch, crate::theme::colors().placeholder_bg);
            return;
        }

        match self.scale_mode {
            SCALE_NONE => {
                // Blit at original size, top-left aligned
                crate::draw::blit_buffer(surface, x, y, self.img_w, self.img_h, &self.pixels);
            }
            SCALE_FIT => {
                // Scale to fit within control bounds, preserving aspect ratio
                let (dx, dy, dw, dh) = fit_rect(self.img_w, self.img_h, cw, ch);
                // Fill letterbox areas
                if dw < cw || dh < ch {
                    crate::draw::fill_rect(surface, x, y, cw, ch, 0x00000000);
                }
                blit_scaled(surface, x + dx, y + dy, dw, dh, &self.pixels, self.img_w, self.img_h);
            }
            SCALE_FILL => {
                // Scale to fill, preserving aspect ratio (may crop)
                let (sx, sy, sw, sh) = fill_crop(self.img_w, self.img_h, cw, ch);
                blit_scaled_crop(surface, x, y, cw, ch, &self.pixels, self.img_w, sx, sy, sw, sh);
            }
            SCALE_STRETCH | _ => {
                // Stretch to fill control bounds
                blit_scaled(surface, x, y, cw, ch, &self.pixels, self.img_w, self.img_h);
            }
        }
    }
}

/// Calculate destination rect for "fit" mode (scale to fit, preserve aspect).
fn fit_rect(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (i32, i32, u32, u32) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return (0, 0, 0, 0);
    }
    let scale_x = (dst_w as u64) * 1000 / src_w as u64;
    let scale_y = (dst_h as u64) * 1000 / src_h as u64;
    let scale = scale_x.min(scale_y);
    let dw = (src_w as u64 * scale / 1000) as u32;
    let dh = (src_h as u64 * scale / 1000) as u32;
    let dx = (dst_w.saturating_sub(dw) / 2) as i32;
    let dy = (dst_h.saturating_sub(dh) / 2) as i32;
    (dx, dy, dw, dh)
}

/// Calculate source crop for "fill" mode (scale to fill, crop excess).
fn fill_crop(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, u32, u32) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return (0, 0, src_w, src_h);
    }
    let scale_x = (dst_w as u64) * 1000 / src_w as u64;
    let scale_y = (dst_h as u64) * 1000 / src_h as u64;
    let scale = scale_x.max(scale_y);
    // Source rect needed to fill dst at this scale
    let sw = (dst_w as u64 * 1000 / scale).min(src_w as u64) as u32;
    let sh = (dst_h as u64 * 1000 / scale).min(src_h as u64) as u32;
    let sx = (src_w.saturating_sub(sw)) / 2;
    let sy = (src_h.saturating_sub(sh)) / 2;
    (sx, sy, sw, sh)
}

/// Blit source pixels to destination with nearest-neighbor scaling.
fn blit_scaled(surface: &crate::draw::Surface, x: i32, y: i32, dw: u32, dh: u32, src: &[u32], sw: u32, sh: u32) {
    if dw == 0 || dh == 0 || sw == 0 || sh == 0 || src.is_empty() { return; }
    let surf_w = surface.width as i32;
    let surf_h = surface.height as i32;
    let clip_x0 = surface.clip_x.max(0);
    let clip_y0 = surface.clip_y.max(0);
    let clip_x1 = (surface.clip_x + surface.clip_w as i32).min(surf_w);
    let clip_y1 = (surface.clip_y + surface.clip_h as i32).min(surf_h);

    for dy in 0..dh as i32 {
        let py = y + dy;
        if py < clip_y0 || py >= clip_y1 { continue; }
        let sy = (dy as u64 * sh as u64 / dh as u64) as usize;
        if sy >= sh as usize { continue; }
        let src_row = sy * sw as usize;

        let x0 = x.max(clip_x0);
        let x1 = (x + dw as i32).min(clip_x1);
        if x0 >= x1 { continue; }

        let dst_row = py as usize * surf_w as usize;
        for px in x0..x1 {
            let dx = (px - x) as u64;
            let sx = (dx * sw as u64 / dw as u64) as usize;
            if sx >= sw as usize { continue; }
            let pixel = src[src_row + sx];
            let a = pixel >> 24;
            if a >= 255 {
                unsafe { *surface.pixels.add(dst_row + px as usize) = pixel; }
            } else if a > 0 {
                let dst = unsafe { *surface.pixels.add(dst_row + px as usize) };
                unsafe { *surface.pixels.add(dst_row + px as usize) = alpha_blend(pixel, dst); }
            }
        }
    }
}

/// Blit a cropped+scaled region of source pixels.
fn blit_scaled_crop(
    surface: &crate::draw::Surface,
    x: i32, y: i32, dw: u32, dh: u32,
    src: &[u32], src_stride: u32,
    cx: u32, cy: u32, cw: u32, ch: u32,
) {
    if dw == 0 || dh == 0 || cw == 0 || ch == 0 { return; }
    let surf_w = surface.width as i32;
    let surf_h = surface.height as i32;
    let clip_x0 = surface.clip_x.max(0);
    let clip_y0 = surface.clip_y.max(0);
    let clip_x1 = (surface.clip_x + surface.clip_w as i32).min(surf_w);
    let clip_y1 = (surface.clip_y + surface.clip_h as i32).min(surf_h);

    for dy in 0..dh as i32 {
        let py = y + dy;
        if py < clip_y0 || py >= clip_y1 { continue; }
        let sy = cy as usize + (dy as u64 * ch as u64 / dh as u64) as usize;
        let src_row = sy * src_stride as usize;

        let x0 = x.max(clip_x0);
        let x1 = (x + dw as i32).min(clip_x1);
        if x0 >= x1 { continue; }

        let dst_row = py as usize * surf_w as usize;
        for px in x0..x1 {
            let dx_frac = (px - x) as u64;
            let sx = cx as usize + (dx_frac * cw as u64 / dw as u64) as usize;
            let idx = src_row + sx;
            if idx >= src.len() { continue; }
            let pixel = src[idx];
            let a = pixel >> 24;
            if a >= 255 {
                unsafe { *surface.pixels.add(dst_row + px as usize) = pixel; }
            } else if a > 0 {
                let dst = unsafe { *surface.pixels.add(dst_row + px as usize) };
                unsafe { *surface.pixels.add(dst_row + px as usize) = alpha_blend(pixel, dst); }
            }
        }
    }
}

/// Simple alpha blend: src over dst.
#[inline]
fn alpha_blend(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let inv_a = 255 - sa;
    let r = (sr * sa + dr * inv_a) / 255;
    let g = (sg * sa + dg * inv_a) / 255;
    let b = (sb * sa + db * inv_a) / 255;
    0xFF000000 | (r << 16) | (g << 8) | b
}
