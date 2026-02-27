//! Software framebuffer for the rasterizer.
//!
//! `SwFramebuffer` owns a color buffer (`Vec<u32>` in ARGB) and a depth buffer
//! (`Vec<f32>` with 1.0 = far). Uses SIMD for bulk clears (~4× faster than
//! scalar loop for large framebuffers).

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
    ///
    /// Uses SSE2 128-bit stores to write 4 pixels per instruction.
    pub fn clear_color(&mut self, argb: u32) {
        let len = self.color.len();
        let ptr = self.color.as_mut_ptr();

        unsafe {
            use core::arch::x86_64::*;
            let val = _mm_set1_epi32(argb as i32);
            let aligned_len = len & !3; // round down to multiple of 4

            let mut i = 0;
            while i < aligned_len {
                _mm_storeu_si128(ptr.add(i) as *mut __m128i, val);
                i += 4;
            }
            // Handle remaining pixels
            while i < len {
                *ptr.add(i) = argb;
                i += 1;
            }
        }
    }

    /// Clear the depth buffer to the given value.
    ///
    /// Uses SSE 128-bit stores to write 4 depth values per instruction.
    pub fn clear_depth(&mut self, val: f32) {
        let len = self.depth.len();
        let ptr = self.depth.as_mut_ptr();

        unsafe {
            use core::arch::x86_64::*;
            let v = _mm_set1_ps(val);
            let aligned_len = len & !3;

            let mut i = 0;
            while i < aligned_len {
                _mm_storeu_ps(ptr.add(i), v);
                i += 4;
            }
            while i < len {
                *ptr.add(i) = val;
                i += 1;
            }
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
