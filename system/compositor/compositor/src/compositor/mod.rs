//! Layer-based compositor engine.
//!
//! Manages z-ordered layers, tracks damage regions, and composites
//! visible layers onto a back buffer, then flushes to the framebuffer.

mod blend;
mod compositing;
mod gpu;
mod layer;
mod rect;
pub mod vram_alloc;

pub use blend::alpha_blend;
pub use layer::Layer;
pub use rect::Rect;

use alloc::vec;
use alloc::vec::Vec;
use layer::AccelMoveHint;
use vram_alloc::VramAllocator;

// ── Compositor ──────────────────────────────────────────────────────────────

pub struct Compositor {
    /// Framebuffer pointer (MMIO VRAM mapped at 0x20000000)
    pub(crate) fb_ptr: *mut u32,
    pub(crate) fb_width: u32,
    pub(crate) fb_height: u32,
    /// Framebuffer pitch in bytes (may differ from width*4)
    pub(crate) fb_pitch: u32,

    /// Back buffer for compositing (contiguous, stride = fb_width)
    pub back_buffer: Vec<u32>,

    /// Layers in z-order (index 0 = bottom, last = top)
    pub layers: Vec<Layer>,
    pub(crate) next_layer_id: u32,

    /// Damage regions to recompose this frame
    pub(crate) damage: Vec<Rect>,

    /// Hardware double-buffering
    pub(crate) hw_double_buffer: bool,
    pub(crate) current_page: u32,
    pub(crate) prev_damage: Vec<Rect>,

    /// GPU 2D acceleration
    pub(crate) gpu_accel: bool,

    /// GPU command batch
    pub(crate) gpu_cmds: Vec<[u32; 9]>,

    /// Hardware cursor
    pub(crate) hw_cursor: bool,

    /// Resize outline (drawn as overlay during resize operations)
    pub resize_outline: Option<Rect>,

    /// The currently focused layer (gets stronger shadow)
    pub focused_layer_id: Option<u32>,

    /// Pending GPU-accelerated move hint for RECT_COPY optimization
    pub(crate) accel_move_hint: Option<AccelMoveHint>,

    /// Off-screen VRAM allocator for VRAM-direct surfaces.
    /// None if GPU accel not available or VRAM too small.
    pub(crate) vram_allocator: Option<VramAllocator>,

    /// Reusable scratch buffer for blur operations (avoids per-frame heap allocation).
    pub(crate) blur_temp: Vec<u32>,

    /// Reusable Vec for compositing loop (swap with self.damage to avoid drain+collect alloc).
    pub(crate) compositing_damage: Vec<Rect>,
}

impl Compositor {
    /// Create a new compositor with the given framebuffer parameters.
    pub fn new(fb_ptr: *mut u32, width: u32, height: u32, pitch: u32) -> Self {
        let pixel_count = (width * height) as usize;
        Compositor {
            fb_ptr,
            fb_width: width,
            fb_height: height,
            fb_pitch: pitch,
            back_buffer: vec![0u32; pixel_count],
            layers: Vec::with_capacity(32),
            next_layer_id: 1,
            damage: Vec::with_capacity(32),
            hw_double_buffer: false,
            current_page: 0,
            prev_damage: Vec::with_capacity(32),
            gpu_accel: false,
            gpu_cmds: Vec::with_capacity(32),
            hw_cursor: false,
            resize_outline: None,
            focused_layer_id: None,
            accel_move_hint: None,
            vram_allocator: None,
            blur_temp: Vec::with_capacity(width.max(height) as usize),
            compositing_damage: Vec::with_capacity(32),
        }
    }

    pub fn width(&self) -> u32 {
        self.fb_width
    }
    pub fn height(&self) -> u32 {
        self.fb_height
    }

    // ── Layer Management ────────────────────────────────────────────────

    /// Add a new layer at the top of the z-order.
    pub fn add_layer(&mut self, x: i32, y: i32, w: u32, h: u32, opaque: bool) -> u32 {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        let pixels = vec![0u32; (w * h) as usize];
        self.layers.push(Layer {
            id,
            x,
            y,
            width: w,
            height: h,
            pixels,
            shm_ptr: core::ptr::null_mut(),
            shm_id: 0,
            opaque,
            visible: true,
            has_shadow: false,
            dirty: true,
            blur_behind: false,
            blur_radius: 0,
            shadow_cache: None,
            is_vram: false,
            vram_y: 0,
        });
        id
    }

    /// Add a new layer with pre-allocated pixels (avoids allocation under lock).
    pub fn add_layer_with_pixels(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        opaque: bool,
        pixels: Vec<u32>,
    ) -> u32 {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        self.layers.push(Layer {
            id,
            x,
            y,
            width: w,
            height: h,
            pixels,
            shm_ptr: core::ptr::null_mut(),
            shm_id: 0,
            opaque,
            visible: true,
            has_shadow: false,
            dirty: true,
            blur_behind: false,
            blur_radius: 0,
            shadow_cache: None,
            is_vram: false,
            vram_y: 0,
        });
        id
    }

    /// Add a new layer backed by a shared memory region (SHM).
    /// The compositor reads pixels from the SHM pointer during compositing.
    pub fn add_shm_layer(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        opaque: bool,
        shm_id: u32,
        shm_ptr: *mut u32,
    ) -> u32 {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        self.layers.push(Layer {
            id,
            x,
            y,
            width: w,
            height: h,
            pixels: Vec::new(), // empty — not used for SHM layers
            shm_ptr,
            shm_id,
            opaque,
            visible: true,
            has_shadow: false,
            dirty: true,
            blur_behind: false,
            blur_radius: 0,
            shadow_cache: None,
            is_vram: false,
            vram_y: 0,
        });
        id
    }

    /// Remove a layer by ID.
    pub fn remove_layer(&mut self, id: u32) {
        if let Some(idx) = self.layer_index(id) {
            let layer = &self.layers[idx];
            self.damage.push(layer.damage_bounds());
            // Free off-screen VRAM allocation if this was a VRAM-direct layer
            if layer.is_vram {
                if let Some(ref mut alloc) = self.vram_allocator {
                    alloc.free(id);
                }
            }
            self.layers.remove(idx);
        }
    }

    /// Add a new layer backed by VRAM-direct surface.
    /// The app writes directly to off-screen VRAM; compositor uses GPU RECT_COPY
    /// to blit to the visible framebuffer (zero CPU pixel copies for opaque windows).
    /// Returns `Some(layer_id)` on success, `None` if VRAM allocation fails.
    pub fn add_vram_layer(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> Option<u32> {
        let alloc = self.vram_allocator.as_mut()?.alloc(w, h, self.next_layer_id)?;
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        self.layers.push(Layer {
            id,
            x,
            y,
            width: w,
            height: h,
            pixels: Vec::new(), // not used — app writes to VRAM directly
            shm_ptr: core::ptr::null_mut(),
            shm_id: 0,
            opaque: true, // VRAM surfaces are always opaque (GPU RECT_COPY)
            visible: true,
            has_shadow: false,
            dirty: true,
            blur_behind: false,
            blur_radius: 0,
            shadow_cache: None,
            is_vram: true,
            vram_y: alloc.vram_y,
        });
        Some(id)
    }

    /// Initialize the off-screen VRAM allocator (called after GPU accel is enabled).
    pub fn init_vram_allocator(&mut self, vram_total: u32) {
        if vram_total > self.fb_pitch * self.fb_height {
            self.vram_allocator = Some(VramAllocator::new(
                self.fb_pitch,
                self.fb_height,
                vram_total,
            ));
        }
    }

    /// Whether VRAM-direct surfaces are available.
    pub fn has_vram_surfaces(&self) -> bool {
        self.vram_allocator.is_some()
    }

    /// Get the VRAM Y-offset for a layer (for RECT_COPY source).
    pub fn vram_layer_y(&self, layer_id: u32) -> Option<u32> {
        self.layers.iter().find(|l| l.id == layer_id && l.is_vram).map(|l| l.vram_y)
    }

    /// Get layer index by ID.
    pub fn layer_index(&self, id: u32) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// Get immutable reference to a layer.
    pub fn get_layer(&self, id: u32) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// Get mutable reference to a layer.
    pub fn get_layer_mut(&mut self, id: u32) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// Get mutable reference to a layer's pixel buffer.
    pub fn layer_pixels(&mut self, id: u32) -> Option<&mut Vec<u32>> {
        self.layers.iter_mut().find(|l| l.id == id).map(|l| &mut l.pixels)
    }

    /// Move a layer to a new position.
    pub fn move_layer(&mut self, id: u32, new_x: i32, new_y: i32) {
        if let Some(idx) = self.layer_index(id) {
            let old_bounds = self.layers[idx].damage_bounds();
            self.layers[idx].x = new_x;
            self.layers[idx].y = new_y;
            let new_bounds = self.layers[idx].damage_bounds();

            if self.gpu_accel {
                // Coalesce: keep first old_bounds, update last new_bounds
                match &mut self.accel_move_hint {
                    Some(hint) if hint.layer_id == id => {
                        hint.new_bounds = new_bounds;
                    }
                    _ => {
                        self.accel_move_hint = Some(AccelMoveHint {
                            layer_id: id,
                            old_bounds,
                            new_bounds,
                        });
                    }
                }
            }
            // Always add damage (fallback path + merge logic)
            self.damage.push(old_bounds);
            self.damage.push(new_bounds);
        }
    }

    /// Bring a layer to the top of the z-order.
    pub fn raise_layer(&mut self, id: u32) {
        if let Some(idx) = self.layer_index(id) {
            if idx < self.layers.len() - 1 {
                let layer = self.layers.remove(idx);
                let bounds = layer.damage_bounds();
                self.layers.push(layer);
                self.damage.push(bounds);
            }
        }
    }

    /// Set the focused layer (gets stronger shadow).
    pub fn set_focused_layer(&mut self, id: Option<u32>) {
        if self.focused_layer_id != id {
            // Damage old and new focused layers (shadow intensity changed)
            if let Some(old_id) = self.focused_layer_id {
                if let Some(idx) = self.layer_index(old_id) {
                    let bounds = self.layers[idx].damage_bounds();
                    self.damage.push(bounds);
                }
            }
            if let Some(new_id) = id {
                if let Some(idx) = self.layer_index(new_id) {
                    let bounds = self.layers[idx].damage_bounds();
                    self.damage.push(bounds);
                }
            }
            self.focused_layer_id = id;
        }
    }

    /// Set layer visibility.
    pub fn set_layer_visible(&mut self, id: u32, visible: bool) {
        if let Some(idx) = self.layer_index(id) {
            if self.layers[idx].visible != visible {
                self.layers[idx].visible = visible;
                self.damage.push(self.layers[idx].damage_bounds());
            }
        }
    }

    /// Mark a layer as dirty (needs recomposition).
    pub fn mark_layer_dirty(&mut self, id: u32) {
        if let Some(idx) = self.layer_index(id) {
            self.layers[idx].dirty = true;
        }
    }

    /// Resize a layer (reallocates pixel buffer).
    pub fn resize_layer(&mut self, id: u32, new_w: u32, new_h: u32) {
        if let Some(idx) = self.layer_index(id) {
            let old_bounds = self.layers[idx].damage_bounds();
            self.damage.push(old_bounds);

            self.layers[idx].width = new_w;
            self.layers[idx].height = new_h;
            self.layers[idx].pixels = vec![0u32; (new_w * new_h) as usize];
            self.layers[idx].shadow_cache = None;
            self.layers[idx].dirty = true;
        }
    }

    // ── Damage Tracking ─────────────────────────────────────────────────

    /// Add a damage rectangle (region that needs recomposition).
    pub fn add_damage(&mut self, rect: Rect) {
        let clipped = rect.clip_to_screen(self.fb_width, self.fb_height);
        if !clipped.is_empty() {
            self.damage.push(clipped);
        }
    }

    // ── Framebuffer I/O ─────────────────────────────────────────────────

    /// Copy a region from back buffer to the framebuffer (at y_offset for double-buffering).
    pub(crate) fn flush_region(&self, rect: &Rect, y_offset: u32) {
        let bb_stride = self.fb_width as usize;
        let fb_stride = (self.fb_pitch / 4) as usize;

        let x = rect.x.max(0) as usize;
        let y = rect.y.max(0) as usize;
        let w = (rect.width as usize).min(self.fb_width as usize - x);
        let h = (rect.height as usize).min(self.fb_height as usize - y);

        for row in 0..h {
            let src_off = (y + row) * bb_stride + x;
            let dst_off = (y + row + y_offset as usize) * fb_stride + x;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    self.back_buffer.as_ptr().add(src_off),
                    self.fb_ptr.add(dst_off),
                    w,
                );
            }
        }
    }

    /// Full-screen damage (force recomposition of everything).
    pub fn damage_all(&mut self) {
        self.damage
            .push(Rect::new(0, 0, self.fb_width, self.fb_height));
    }

    /// Resize the compositor for a new screen resolution.
    /// Reallocates the back buffer and updates dimensions. Layers are NOT touched.
    pub fn resize_fb(&mut self, new_width: u32, new_height: u32, new_pitch: u32) {
        self.fb_width = new_width;
        self.fb_height = new_height;
        self.fb_pitch = new_pitch;
        let pixel_count = (new_width * new_height) as usize;
        self.back_buffer = vec![0u32; pixel_count];
        // Disable double-buffering — VRAM may be too small for 2x height at new res
        self.hw_double_buffer = false;
        self.current_page = 0;
        self.prev_damage.clear();
        self.damage.clear();
        // Invalidate VRAM allocations — resolution changed so off-screen layout is invalid.
        // Mark all VRAM layers as non-VRAM (they'll fall back to SHM compositing).
        for layer in &mut self.layers {
            if layer.is_vram {
                layer.is_vram = false;
                layer.vram_y = 0;
            }
        }
        if let Some(ref mut alloc) = self.vram_allocator {
            let vram_total = alloc.off_screen_bytes() + new_pitch * new_height;
            alloc.update_fb(new_pitch, new_height, vram_total);
        }
    }
}
