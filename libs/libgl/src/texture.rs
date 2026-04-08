//! Texture objects (GL_TEXTURE_2D).
//!
//! Stores texture data as RGBA8 pixels. Supports `glTexImage2D`, `glTexSubImage2D`,
//! `glTexParameteri`, and nearest/linear filtering for the software rasterizer.

use alloc::vec;
use alloc::vec::Vec;
use crate::types::*;

pub const MAX_MIP_LEVELS: usize = 12;

/// A 2D texture object.
pub struct GlTexture {
    /// RGBA8 pixel data (row-major).
    pub data: Vec<u32>,
    /// Linear depth data for depth textures.
    pub depth: Vec<f32>,
    pub width: u32,
    pub height: u32,
    pub min_filter: GLenum,
    pub mag_filter: GLenum,
    pub wrap_s: GLenum,
    pub wrap_t: GLenum,
    pub internal_format: GLenum,
    pub mip_data: [Vec<u32>; MAX_MIP_LEVELS - 1],
    pub mip_depth: [Vec<f32>; MAX_MIP_LEVELS - 1],
    pub mip_widths: [u32; MAX_MIP_LEVELS - 1],
    pub mip_heights: [u32; MAX_MIP_LEVELS - 1],
    pub mip_count: usize,
}

impl GlTexture {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            depth: Vec::new(),
            width: 0,
            height: 0,
            min_filter: GL_NEAREST_MIPMAP_LINEAR,
            mag_filter: GL_LINEAR,
            wrap_s: GL_REPEAT,
            wrap_t: GL_REPEAT,
            internal_format: GL_RGBA,
            mip_data: core::array::from_fn(|_| Vec::new()),
            mip_depth: core::array::from_fn(|_| Vec::new()),
            mip_widths: [0; MAX_MIP_LEVELS - 1],
            mip_heights: [0; MAX_MIP_LEVELS - 1],
            mip_count: 0,
        }
    }

    /// Sample a texel at (u, v) with nearest-neighbor filtering.
    pub fn sample_nearest(&self, u: f32, v: f32) -> [f32; 4] {
        self.sample_nearest_level(0, u, v)
    }

    pub fn sample_nearest_level(&self, level: usize, u: f32, v: f32) -> [f32; 4] {
        let Some((w, h)) = self.level_dims(level) else {
            return [0.0, 0.0, 0.0, 1.0];
        };
        if w == 0 || h == 0 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        if self.width == 0 || self.height == 0 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        let u = wrap_coord(u, self.wrap_s);
        let v = wrap_coord(v, self.wrap_t);
        let x = ((u * w as f32) as i32).clamp(0, w as i32 - 1) as u32;
        let y = ((v * h as f32) as i32).clamp(0, h as i32 - 1) as u32;
        if self.internal_format == GL_DEPTH_COMPONENT {
            let d = self.fetch_depth_level(level, x, y);
            return [d, d, d, 1.0];
        }
        let px = self.fetch_packed_level(level, x, y);
        unpack_rgba(px)
    }

    /// Sample a texel at (u, v) with bilinear filtering.
    pub fn sample_linear(&self, u: f32, v: f32) -> [f32; 4] {
        self.sample_linear_level(0, u, v)
    }

    pub fn sample_linear_level(&self, level: usize, u: f32, v: f32) -> [f32; 4] {
        let Some((w_u32, h_u32)) = self.level_dims(level) else {
            return [0.0, 0.0, 0.0, 1.0];
        };
        if w_u32 == 0 || h_u32 == 0 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        if self.width == 0 || self.height == 0 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        let u = wrap_coord(u, self.wrap_s);
        let v = wrap_coord(v, self.wrap_t);
        let fx = u * w_u32 as f32 - 0.5;
        let fy = v * h_u32 as f32 - 0.5;
        let x0 = floor_f32(fx) as i32;
        let y0 = floor_f32(fy) as i32;
        let frac_x = fx - x0 as f32;
        let frac_y = fy - y0 as f32;

        let w = w_u32 as i32;
        let h = h_u32 as i32;
        if self.internal_format == GL_DEPTH_COMPONENT {
            let s00 = self.fetch_depth_level(level, x0.clamp(0, w - 1) as u32, y0.clamp(0, h - 1) as u32);
            let s10 = self.fetch_depth_level(level, (x0 + 1).clamp(0, w - 1) as u32, y0.clamp(0, h - 1) as u32);
            let s01 = self.fetch_depth_level(level, x0.clamp(0, w - 1) as u32, (y0 + 1).clamp(0, h - 1) as u32);
            let s11 = self.fetch_depth_level(level, (x0 + 1).clamp(0, w - 1) as u32, (y0 + 1).clamp(0, h - 1) as u32);
            let top = s00 + (s10 - s00) * frac_x;
            let bot = s01 + (s11 - s01) * frac_x;
            let depth = top + (bot - top) * frac_y;
            return [depth, depth, depth, 1.0];
        }
        let s00 = self.fetch_level(level, x0.clamp(0, w - 1) as u32, y0.clamp(0, h - 1) as u32);
        let s10 = self.fetch_level(level, (x0 + 1).clamp(0, w - 1) as u32, y0.clamp(0, h - 1) as u32);
        let s01 = self.fetch_level(level, x0.clamp(0, w - 1) as u32, (y0 + 1).clamp(0, h - 1) as u32);
        let s11 = self.fetch_level(level, (x0 + 1).clamp(0, w - 1) as u32, (y0 + 1).clamp(0, h - 1) as u32);

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
        self.sample_lod(u, v, 0.0)
    }

    pub fn sample_lod(&self, u: f32, v: f32, lod: f32) -> [f32; 4] {
        let level = self.pick_mip_level(lod);
        match self.active_level_filter() {
            GL_LINEAR => self.sample_linear_level(level, u, v),
            _ => self.sample_nearest_level(level, u, v),
        }
    }

    fn fetch(&self, x: u32, y: u32) -> [f32; 4] {
        let px = self.data[(y * self.width + x) as usize];
        unpack_rgba(px)
    }

    fn fetch_depth(&self, x: u32, y: u32) -> f32 {
        self.depth[(y * self.width + x) as usize]
    }

    fn fetch_level(&self, level: usize, x: u32, y: u32) -> [f32; 4] {
        unpack_rgba(self.fetch_packed_level(level, x, y))
    }

    fn fetch_packed_level(&self, level: usize, x: u32, y: u32) -> u32 {
        if level == 0 {
            self.data[(y * self.width + x) as usize]
        } else {
            let idx = level - 1;
            let w = self.mip_widths[idx];
            self.mip_data[idx][(y * w + x) as usize]
        }
    }

    fn fetch_depth_level(&self, level: usize, x: u32, y: u32) -> f32 {
        if level == 0 {
            self.depth[(y * self.width + x) as usize]
        } else {
            let idx = level - 1;
            let w = self.mip_widths[idx];
            self.mip_depth[idx][(y * w + x) as usize]
        }
    }

    fn level_dims(&self, level: usize) -> Option<(u32, u32)> {
        if self.mip_count == 0 || level >= self.mip_count {
            None
        } else if level == 0 {
            Some((self.width, self.height))
        } else {
            Some((self.mip_widths[level - 1], self.mip_heights[level - 1]))
        }
    }

    fn active_level_filter(&self) -> GLenum {
        match self.min_filter {
            GL_LINEAR | GL_LINEAR_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_LINEAR => GL_LINEAR,
            _ => GL_NEAREST,
        }
    }

    fn pick_mip_level(&self, lod: f32) -> usize {
        if self.mip_count <= 1 {
            return 0;
        }
        let wants_mips = matches!(
            self.min_filter,
            GL_NEAREST_MIPMAP_NEAREST
                | GL_LINEAR_MIPMAP_NEAREST
                | GL_NEAREST_MIPMAP_LINEAR
                | GL_LINEAR_MIPMAP_LINEAR
        );
        if !wants_mips {
            return 0;
        }
        lod.clamp(0.0, (self.mip_count - 1) as f32) as usize
    }

    pub fn generate_mipmaps(&mut self) {
        self.mip_count = if self.width > 0 && self.height > 0 { 1 } else { 0 };
        for level in 0..(MAX_MIP_LEVELS - 1) {
            self.mip_data[level].clear();
            self.mip_depth[level].clear();
            self.mip_widths[level] = 0;
            self.mip_heights[level] = 0;
        }
        if self.width == 0 || self.height == 0 {
            return;
        }

        let mut src_w = self.width;
        let mut src_h = self.height;
        let mut src_rgba = self.data.clone();
        let mut src_depth = self.depth.clone();

        for level in 1..MAX_MIP_LEVELS {
            if src_w == 1 && src_h == 1 {
                break;
            }
            let dst_w = (src_w / 2).max(1);
            let dst_h = (src_h / 2).max(1);
            let mut dst_rgba = vec![0u32; (dst_w * dst_h) as usize];
            let mut dst_depth = vec![1.0f32; (dst_w * dst_h) as usize];

            for y in 0..dst_h {
                for x in 0..dst_w {
                    let sx0 = (x * 2).min(src_w - 1);
                    let sy0 = (y * 2).min(src_h - 1);
                    let sx1 = (sx0 + 1).min(src_w - 1);
                    let sy1 = (sy0 + 1).min(src_h - 1);
                    let samples = [
                        src_rgba[(sy0 * src_w + sx0) as usize],
                        src_rgba[(sy0 * src_w + sx1) as usize],
                        src_rgba[(sy1 * src_w + sx0) as usize],
                        src_rgba[(sy1 * src_w + sx1) as usize],
                    ];
                    dst_rgba[(y * dst_w + x) as usize] = average_rgba(samples);

                    let depths = [
                        src_depth[(sy0 * src_w + sx0) as usize],
                        src_depth[(sy0 * src_w + sx1) as usize],
                        src_depth[(sy1 * src_w + sx0) as usize],
                        src_depth[(sy1 * src_w + sx1) as usize],
                    ];
                    dst_depth[(y * dst_w + x) as usize] =
                        0.25 * (depths[0] + depths[1] + depths[2] + depths[3]);
                }
            }

            let idx = level - 1;
            self.mip_widths[idx] = dst_w;
            self.mip_heights[idx] = dst_h;
            self.mip_data[idx] = dst_rgba.clone();
            self.mip_depth[idx] = dst_depth.clone();
            self.mip_count = level + 1;

            src_w = dst_w;
            src_h = dst_h;
            src_rgba = dst_rgba;
            src_depth = dst_depth;
        }
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
            tex.mip_count = if width > 0 && height > 0 { 1 } else { 0 };
            let npixels = (width * height) as usize;
            tex.data = vec![0u32; npixels];
            tex.depth = vec![1.0f32; npixels];
            for level in 0..(MAX_MIP_LEVELS - 1) {
                tex.mip_data[level].clear();
                tex.mip_depth[level].clear();
                tex.mip_widths[level] = 0;
                tex.mip_heights[level] = 0;
            }

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
                    // GL_DEPTH_COMPONENT: depth texture (GL_OES_depth_texture).
                    // CPU data is optional; depth is written by the rasterizer or GPU.
                    0x1902 | 0x81A5 | 0x81A6 | 0x81A7 => {
                        for i in 0..npixels.min(src.len()) {
                            let d = src[i] as f32 / 255.0;
                            tex.depth[i] = d;
                            let q = (d * 255.0) as u32;
                            tex.data[i] = 0xFF000000 | (q << 16) | (q << 8) | q;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn generate_mipmaps(&mut self, id: u32) {
        if let Some(tex) = self.get_mut(id) {
            tex.generate_mipmaps();
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

fn average_rgba(samples: [u32; 4]) -> u32 {
    let mut a = 0u32;
    let mut r = 0u32;
    let mut g = 0u32;
    let mut b = 0u32;
    for px in samples {
        a += (px >> 24) & 0xFF;
        r += (px >> 16) & 0xFF;
        g += (px >> 8) & 0xFF;
        b += px & 0xFF;
    }
    ((a / 4) << 24) | ((r / 4) << 16) | ((g / 4) << 8) | (b / 4)
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
