//! Texture objects (GL_TEXTURE_2D).
//!
//! Stores texture data as RGBA8 pixels. Supports `glTexImage2D`, `glTexSubImage2D`,
//! `glTexParameteri`, and nearest/linear filtering for the software rasterizer.

use alloc::vec;
use alloc::vec::Vec;
use crate::types::*;

/// A 2D texture object.
pub struct GlTexture {
    /// RGBA8 pixel data (row-major).
    pub data: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub min_filter: GLenum,
    pub mag_filter: GLenum,
    pub wrap_s: GLenum,
    pub wrap_t: GLenum,
    pub internal_format: GLenum,
}

impl GlTexture {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            width: 0,
            height: 0,
            min_filter: GL_NEAREST_MIPMAP_LINEAR,
            mag_filter: GL_LINEAR,
            wrap_s: GL_REPEAT,
            wrap_t: GL_REPEAT,
            internal_format: GL_RGBA,
        }
    }

    /// Sample a texel at (u, v) with nearest-neighbor filtering.
    pub fn sample_nearest(&self, u: f32, v: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        let u = wrap_coord(u, self.wrap_s);
        let v = wrap_coord(v, self.wrap_t);
        let x = ((u * self.width as f32) as i32).clamp(0, self.width as i32 - 1) as u32;
        let y = ((v * self.height as f32) as i32).clamp(0, self.height as i32 - 1) as u32;
        let px = self.data[(y * self.width + x) as usize];
        unpack_rgba(px)
    }

    /// Sample a texel at (u, v) with bilinear filtering.
    pub fn sample_linear(&self, u: f32, v: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        let u = wrap_coord(u, self.wrap_s);
        let v = wrap_coord(v, self.wrap_t);
        let fx = u * self.width as f32 - 0.5;
        let fy = v * self.height as f32 - 0.5;
        let x0 = floor_f32(fx) as i32;
        let y0 = floor_f32(fy) as i32;
        let frac_x = fx - x0 as f32;
        let frac_y = fy - y0 as f32;

        let w = self.width as i32;
        let h = self.height as i32;
        let s00 = self.fetch(x0.clamp(0, w - 1) as u32, y0.clamp(0, h - 1) as u32);
        let s10 = self.fetch((x0 + 1).clamp(0, w - 1) as u32, y0.clamp(0, h - 1) as u32);
        let s01 = self.fetch(x0.clamp(0, w - 1) as u32, (y0 + 1).clamp(0, h - 1) as u32);
        let s11 = self.fetch((x0 + 1).clamp(0, w - 1) as u32, (y0 + 1).clamp(0, h - 1) as u32);

        let mut result = [0.0f32; 4];
        for i in 0..4 {
            let top = s00[i] + (s10[i] - s00[i]) * frac_x;
            let bot = s01[i] + (s11[i] - s01[i]) * frac_x;
            result[i] = top + (bot - top) * frac_y;
        }
        result
    }

    /// Sample using the configured mag filter.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        match self.mag_filter {
            GL_LINEAR => self.sample_linear(u, v),
            _ => self.sample_nearest(u, v),
        }
    }

    fn fetch(&self, x: u32, y: u32) -> [f32; 4] {
        let px = self.data[(y * self.width + x) as usize];
        unpack_rgba(px)
    }
}

/// Storage for all texture objects.
pub struct TextureStore {
    slots: Vec<Option<GlTexture>>,
    next_id: u32,
}

impl TextureStore {
    /// Create an empty texture store.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            next_id: 1,
        }
    }

    /// Generate `n` texture names.
    pub fn gen(&mut self, n: i32, ids: &mut [u32]) {
        for i in 0..(n as usize).min(ids.len()) {
            let id = self.next_id;
            self.next_id += 1;
            while self.slots.len() <= id as usize {
                self.slots.push(None);
            }
            self.slots[id as usize] = Some(GlTexture::new());
            ids[i] = id;
        }
    }

    /// Delete textures by id.
    pub fn delete(&mut self, n: i32, ids: &[u32]) {
        for i in 0..(n as usize).min(ids.len()) {
            let id = ids[i] as usize;
            if id > 0 && id < self.slots.len() {
                self.slots[id] = None;
            }
        }
    }

    /// Get a reference to a texture.
    pub fn get(&self, id: u32) -> Option<&GlTexture> {
        if id == 0 { return None; }
        self.slots.get(id as usize).and_then(|s| s.as_ref())
    }

    /// Get a mutable reference to a texture.
    pub fn get_mut(&mut self, id: u32) -> Option<&mut GlTexture> {
        if id == 0 { return None; }
        self.slots.get_mut(id as usize).and_then(|s| s.as_mut())
    }

    /// Upload pixel data (glTexImage2D).
    pub fn tex_image_2d(
        &mut self,
        id: u32,
        width: u32,
        height: u32,
        format: GLenum,
        data: Option<&[u8]>,
    ) {
        if let Some(tex) = self.get_mut(id) {
            tex.width = width;
            tex.height = height;
            tex.internal_format = format;
            let npixels = (width * height) as usize;
            tex.data = vec![0u32; npixels];

            if let Some(src) = data {
                match format {
                    GL_RGBA => {
                        for i in 0..npixels.min(src.len() / 4) {
                            let r = src[i * 4] as u32;
                            let g = src[i * 4 + 1] as u32;
                            let b = src[i * 4 + 2] as u32;
                            let a = src[i * 4 + 3] as u32;
                            tex.data[i] = (a << 24) | (r << 16) | (g << 8) | b;
                        }
                    }
                    GL_RGB => {
                        for i in 0..npixels.min(src.len() / 3) {
                            let r = src[i * 3] as u32;
                            let g = src[i * 3 + 1] as u32;
                            let b = src[i * 3 + 2] as u32;
                            tex.data[i] = 0xFF000000 | (r << 16) | (g << 8) | b;
                        }
                    }
                    GL_LUMINANCE => {
                        for i in 0..npixels.min(src.len()) {
                            let l = src[i] as u32;
                            tex.data[i] = 0xFF000000 | (l << 16) | (l << 8) | l;
                        }
                    }
                    GL_ALPHA => {
                        for i in 0..npixels.min(src.len()) {
                            let a = src[i] as u32;
                            tex.data[i] = a << 24;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Unpack an ARGB u32 into [r, g, b, a] floats in 0..1.
fn unpack_rgba(px: u32) -> [f32; 4] {
    let a = ((px >> 24) & 0xFF) as f32 / 255.0;
    let r = ((px >> 16) & 0xFF) as f32 / 255.0;
    let g = ((px >> 8) & 0xFF) as f32 / 255.0;
    let b = (px & 0xFF) as f32 / 255.0;
    [r, g, b, a]
}

/// Floor for f32 (no libm).
fn floor_f32(x: f32) -> f32 {
    let i = x as i32;
    if x < 0.0 && x != i as f32 { (i - 1) as f32 } else { i as f32 }
}

/// Wrap a texture coordinate according to the wrap mode.
fn wrap_coord(c: f32, mode: GLenum) -> f32 {
    match mode {
        GL_CLAMP_TO_EDGE => c.clamp(0.0, 1.0),
        GL_MIRRORED_REPEAT => {
            let t = floor_f32(c) as i32;
            let frac = c - floor_f32(c);
            if t & 1 != 0 { 1.0 - frac } else { frac }
        }
        _ => {
            // GL_REPEAT
            let f = c - floor_f32(c);
            if f < 0.0 { f + 1.0 } else { f }
        }
    }
}
