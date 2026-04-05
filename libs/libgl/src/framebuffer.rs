//! Software framebuffer for the rasterizer.
//!
//! `SwFramebuffer` owns a color buffer (`Vec<u32>` in ARGB) and a depth buffer
//! (`Vec<f32>` with 1.0 = far). Uses simple scalar loops for bulk clears.

use alloc::vec;
use alloc::vec::Vec;
use crate::state::GlContext;

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

#[derive(Clone, Copy)]
pub struct TargetInfo {
    pub color_ptr: *mut u32,
    pub depth_ptr: *mut f32,
    pub width: u32,
    pub height: u32,
    pub has_color: bool,
    pub has_depth: bool,
}

pub fn current_target_size(ctx: &GlContext) -> (u32, u32) {
    if ctx.bound_framebuffer == 0 {
        return (ctx.default_fb.width, ctx.default_fb.height);
    }

    if let Some(fbo) = ctx.fbos.iter().find(|f| f.id == ctx.bound_framebuffer) {
        if fbo.width > 0 && fbo.height > 0 {
            return (fbo.width, fbo.height);
        }
    }

    (ctx.default_fb.width, ctx.default_fb.height)
}

pub fn current_target(ctx: &mut GlContext) -> Option<TargetInfo> {
    if ctx.bound_framebuffer == 0 {
        return Some(TargetInfo {
            color_ptr: ctx.default_fb.color.as_mut_ptr(),
            depth_ptr: ctx.default_fb.depth.as_mut_ptr(),
            width: ctx.default_fb.width,
            height: ctx.default_fb.height,
            has_color: true,
            has_depth: true,
        });
    }

    let fbo = ctx.fbos.iter().find(|f| f.id == ctx.bound_framebuffer)?.clone();
    if fbo.width == 0 || fbo.height == 0 {
        return None;
    }

    let color_ptr = if fbo.color_tex != 0 {
        match ctx.textures.get_mut(fbo.color_tex) {
            Some(tex) if tex.width == fbo.width && tex.height == fbo.height => tex.data.as_mut_ptr(),
            _ => core::ptr::null_mut(),
        }
    } else {
        core::ptr::null_mut()
    };

    let depth_ptr = if fbo.depth_tex != 0 {
        match ctx.textures.get_mut(fbo.depth_tex) {
            Some(tex) if tex.width == fbo.width && tex.height == fbo.height => tex.depth.as_mut_ptr(),
            _ => core::ptr::null_mut(),
        }
    } else {
        core::ptr::null_mut()
    };

    Some(TargetInfo {
        color_ptr,
        depth_ptr,
        width: fbo.width,
        height: fbo.height,
        has_color: !color_ptr.is_null(),
        has_depth: !depth_ptr.is_null(),
    })
}

pub fn clear_current(ctx: &mut GlContext, clear_color: Option<u32>, clear_depth: Option<f32>) {
    let Some(target) = current_target(ctx) else { return; };
    let count = (target.width * target.height) as usize;

    unsafe {
        if let Some(argb) = clear_color {
            if target.has_color {
                let color = core::slice::from_raw_parts_mut(target.color_ptr, count);
                for p in color.iter_mut() {
                    *p = argb;
                }
            }
        }

        if let Some(depth_val) = clear_depth {
            if target.has_depth {
                let depth = core::slice::from_raw_parts_mut(target.depth_ptr, count);
                for p in depth.iter_mut() {
                    *p = depth_val;
                }
            }
        }
    }
}
