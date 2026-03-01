//! Software framebuffer for the rasterizer.
//!
//! `SwFramebuffer` owns a color buffer (`Vec<u32>` in ARGB) and a depth buffer
//! (`Vec<f32>` with 1.0 = far). Uses simple scalar loops for bulk clears.

use alloc::vec;
use alloc::vec::Vec;

/// Software framebuffer with color and depth.
pub struct SwFramebuffer {
    /// ARGB pixel buffer (row-major, top-left origin).
    pub color: Vec<u32>,
    /// Depth buffer (0.0 = near, 1.0 = far).
    pub depth: Vec<f32>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl SwFramebuffer {
    /// Allocate a new framebuffer. All pixels cleared to 0, depth to 1.0.
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height) as usize;
        Self {
            color: vec![0u32; size],
            depth: vec![1.0f32; size],
            width,
            height,
        }
    }

    /// Clear the color buffer to the given ARGB value.
    pub fn clear_color(&mut self, argb: u32) {
        for p in self.color.iter_mut() {
            *p = argb;
        }
    }

    /// Clear the depth buffer to the given value.
    pub fn clear_depth(&mut self, val: f32) {
        for p in self.depth.iter_mut() {
            *p = val;
        }
    }

    /// Resize the framebuffer (re-allocates and clears).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        let size = (width * height) as usize;
        self.color = vec![0u32; size];
        self.depth = vec![1.0f32; size];
    }
}
