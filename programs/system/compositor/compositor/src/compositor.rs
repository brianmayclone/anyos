//! Layer-based compositor engine.
//!
//! Manages z-ordered layers, tracks damage regions, and composites
//! visible layers onto a back buffer, then flushes to the framebuffer.

use alloc::vec;
use alloc::vec::Vec;
use anyos_std::ipc;

// ── GPU Command Types ───────────────────────────────────────────────────────

const GPU_UPDATE: u32 = 1;
const GPU_RECT_FILL: u32 = 2;
const GPU_RECT_COPY: u32 = 3;
const GPU_CURSOR_MOVE: u32 = 4;
const GPU_CURSOR_SHOW: u32 = 5;
const GPU_DEFINE_CURSOR: u32 = 6;
const GPU_FLIP: u32 = 7;

// ── Rect ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Rect { x, y, width, height }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// Compute intersection of two rectangles. Returns None if no overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        if r > x && b > y {
            Some(Rect::new(x, y, (r - x) as u32, (b - y) as u32))
        } else {
            None
        }
    }

    /// Compute bounding box union of two rectangles.
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = self.right().max(other.right());
        let b = self.bottom().max(other.bottom());
        Rect::new(x, y, (r - x) as u32, (b - y) as u32)
    }

    /// Expand rect by `n` pixels on all sides.
    pub fn expand(&self, n: i32) -> Rect {
        Rect::new(
            self.x - n,
            self.y - n,
            (self.width as i32 + n * 2).max(0) as u32,
            (self.height as i32 + n * 2).max(0) as u32,
        )
    }

    /// Clip rect to screen bounds.
    pub fn clip_to_screen(&self, w: u32, h: u32) -> Rect {
        let x = self.x.max(0);
        let y = self.y.max(0);
        let r = self.right().min(w as i32);
        let b = self.bottom().min(h as i32);
        if r > x && b > y {
            Rect::new(x, y, (r - x) as u32, (b - y) as u32)
        } else {
            Rect::new(0, 0, 0, 0)
        }
    }
}

// ── Layer ───────────────────────────────────────────────────────────────────

pub struct Layer {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Owned pixel buffer (for compositor-managed layers like bg, menubar)
    pub pixels: Vec<u32>,
    /// SHM-backed pixel buffer (for app windows). If non-null, compositing reads from here.
    pub shm_ptr: *mut u32,
    pub shm_id: u32,
    pub opaque: bool,
    pub visible: bool,
    pub has_shadow: bool,
    pub dirty: bool,
}

impl Layer {
    /// Get the pixel slice for compositing. Uses SHM if available, else owned Vec.
    pub fn pixel_slice(&self) -> &[u32] {
        if !self.shm_ptr.is_null() {
            let count = (self.width * self.height) as usize;
            unsafe { core::slice::from_raw_parts(self.shm_ptr, count) }
        } else {
            &self.pixels
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }

    /// Bounds including shadow offset (8px spread, 4px down).
    pub fn shadow_bounds(&self) -> Rect {
        if self.has_shadow {
            Rect::new(self.x - 4, self.y, self.width + 8, self.height + 8)
        } else {
            self.bounds()
        }
    }

    /// Effective bounds for damage (includes shadow if present).
    pub fn damage_bounds(&self) -> Rect {
        if self.has_shadow {
            self.shadow_bounds()
        } else {
            self.bounds()
        }
    }
}

// ── Shadow Constants ────────────────────────────────────────────────────────

const SHADOW_OFFSET_X: i32 = 0;
const SHADOW_OFFSET_Y: i32 = 4;
const SHADOW_SPREAD: i32 = 4;
const SHADOW_COLOR: u32 = 0x40000000; // 25% black

// ── Compositor ──────────────────────────────────────────────────────────────

pub struct Compositor {
    /// Framebuffer pointer (MMIO VRAM mapped at 0x20000000)
    fb_ptr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    /// Framebuffer pitch in bytes (may differ from width*4)
    fb_pitch: u32,

    /// Back buffer for compositing (contiguous, stride = fb_width)
    pub back_buffer: Vec<u32>,

    /// Layers in z-order (index 0 = bottom, last = top)
    pub layers: Vec<Layer>,
    next_layer_id: u32,

    /// Damage regions to recompose this frame
    damage: Vec<Rect>,

    /// Hardware double-buffering
    hw_double_buffer: bool,
    current_page: u32,
    prev_damage: Vec<Rect>,

    /// GPU 2D acceleration
    gpu_accel: bool,

    /// GPU command batch
    gpu_cmds: Vec<[u32; 9]>,

    /// Hardware cursor
    hw_cursor: bool,

    /// Resize outline (drawn as overlay during resize operations)
    pub resize_outline: Option<Rect>,
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
        });
        id
    }

    /// Remove a layer by ID.
    pub fn remove_layer(&mut self, id: u32) {
        if let Some(idx) = self.layer_index(id) {
            let layer = &self.layers[idx];
            self.damage.push(layer.damage_bounds());
            self.layers.remove(idx);
        }
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

    /// Collect damage from all dirty layers.
    fn collect_dirty_damage(&mut self) {
        for layer in &mut self.layers {
            if layer.dirty && layer.visible {
                // NOTE: can't push to self.damage while iterating self.layers
                // so we just set dirty=false; damage was already added by the caller
                layer.dirty = false;
            }
        }
        // Also collect dirty layers' bounds
        let mut new_damage = Vec::new();
        for layer in &self.layers {
            if layer.dirty {
                new_damage.push(layer.damage_bounds());
            }
        }
        for layer in &mut self.layers {
            layer.dirty = false;
        }
        self.damage.extend(new_damage);
    }

    /// Merge damage rects if there are too many (prevents performance explosion).
    fn merge_damage_if_needed(&mut self) {
        if self.damage.len() > 16 {
            let merged = self.damage.iter().copied().reduce(|a, b| a.union(&b));
            self.damage.clear();
            if let Some(r) = merged {
                let clipped = r.clip_to_screen(self.fb_width, self.fb_height);
                if !clipped.is_empty() {
                    self.damage.push(clipped);
                }
            }
        }
    }

    // ── Compositing ─────────────────────────────────────────────────────

    /// Main compositing function. Composites all dirty regions.
    pub fn compose(&mut self) {
        self.collect_dirty_damage();

        if self.damage.is_empty() {
            return;
        }

        self.merge_damage_if_needed();

        // Clip all damage to screen bounds
        let screen = Rect::new(0, 0, self.fb_width, self.fb_height);
        let damage: Vec<Rect> = self
            .damage
            .drain(..)
            .filter_map(|r| r.intersect(&screen))
            .collect();

        if damage.is_empty() {
            return;
        }

        // Phase 1: Composite layers into back buffer
        for rect in &damage {
            self.composite_rect(rect);
        }

        // Draw resize outline on back buffer (if active)
        if let Some(outline) = self.resize_outline {
            self.draw_outline_to_bb(&outline);
        }

        // Phase 2: Flush to framebuffer
        if self.hw_double_buffer {
            // Double-buffered: flush prev_damage to new back page first,
            // then flush current damage, then flip.
            let back_offset = if self.current_page == 0 {
                self.fb_height
            } else {
                0
            };

            // Flush previous damage to the new back page
            for rect in &self.prev_damage {
                self.flush_region(rect, back_offset);
            }
            // Flush current damage
            for rect in &damage {
                self.flush_region(rect, back_offset);
            }
            // Page flip
            self.gpu_cmds.push([GPU_FLIP, 0, 0, 0, 0, 0, 0, 0, 0]);
            self.current_page = 1 - self.current_page;
            self.prev_damage = damage;
        } else {
            // Single-buffered: copy back buffer to visible FB
            for rect in &damage {
                self.flush_region(rect, 0);
                self.gpu_cmds
                    .push([GPU_UPDATE, rect.x as u32, rect.y as u32, rect.width, rect.height, 0, 0, 0, 0]);
            }
        }

        // Submit GPU commands
        self.flush_gpu();
    }

    /// Composite all layers within a damage rect into the back buffer.
    fn composite_rect(&mut self, rect: &Rect) {
        let bb_stride = self.fb_width as usize;
        let rx = rect.x as usize;
        let ry = rect.y as usize;
        let rw = rect.width as usize;
        let rh = rect.height as usize;

        // Fill with transparent black (background will be drawn by bottom layer)
        for row in 0..rh {
            let y = ry + row;
            if y >= self.fb_height as usize {
                break;
            }
            let off = y * bb_stride + rx;
            let end = (off + rw).min(self.back_buffer.len());
            for p in &mut self.back_buffer[off..end] {
                *p = 0xFF1E1E1E; // Desktop background color as fallback
            }
        }

        // Composite each visible layer (bottom to top)
        for li in 0..self.layers.len() {
            if !self.layers[li].visible {
                continue;
            }

            // Draw shadow before the layer itself
            let has_shadow = self.layers[li].has_shadow;
            if has_shadow {
                self.draw_shadow_to_bb(rect, li);
            }

            let layer_rect = self.layers[li].bounds();
            let layer_x = self.layers[li].x;
            let layer_y = self.layers[li].y;
            let lw = self.layers[li].width as usize;
            let layer_opaque = self.layers[li].opaque;

            if let Some(overlap) = rect.intersect(&layer_rect) {
                // Source coords in layer
                let sx = (overlap.x - layer_x) as usize;
                let sy = (overlap.y - layer_y) as usize;

                let layer_pixels = self.layers[li].pixel_slice();
                let lp_len = layer_pixels.len();

                if layer_opaque {
                    // Fast path: opaque copy
                    for row in 0..overlap.height as usize {
                        let src_off = (sy + row) * lw + sx;
                        let dst_off =
                            (overlap.y as usize + row) * bb_stride + overlap.x as usize;
                        let w = overlap.width as usize;
                        let src_end = (src_off + w).min(lp_len);
                        let dst_end = (dst_off + w).min(self.back_buffer.len());
                        let copy_w = (src_end - src_off).min(dst_end - dst_off);
                        self.back_buffer[dst_off..dst_off + copy_w]
                            .copy_from_slice(&layer_pixels[src_off..src_off + copy_w]);
                    }
                } else {
                    // Alpha-blend path
                    for row in 0..overlap.height as usize {
                        let src_off = (sy + row) * lw + sx;
                        let dst_off =
                            (overlap.y as usize + row) * bb_stride + overlap.x as usize;
                        for col in 0..overlap.width as usize {
                            let si = src_off + col;
                            let di = dst_off + col;
                            if si >= lp_len || di >= self.back_buffer.len() {
                                break;
                            }
                            let src = layer_pixels[si];
                            let a = (src >> 24) & 0xFF;
                            if a >= 255 {
                                self.back_buffer[di] = src;
                            } else if a > 0 {
                                self.back_buffer[di] = alpha_blend(src, self.back_buffer[di]);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Draw shadow for a layer into the back buffer (within damage rect).
    fn draw_shadow_to_bb(&mut self, rect: &Rect, layer_idx: usize) {
        let layer = &self.layers[layer_idx];
        let shadow_rect = Rect::new(
            layer.x + SHADOW_OFFSET_X - SHADOW_SPREAD,
            layer.y + SHADOW_OFFSET_Y,
            layer.width + (SHADOW_SPREAD * 2) as u32,
            layer.height + SHADOW_SPREAD as u32,
        );
        if let Some(overlap) = rect.intersect(&shadow_rect) {
            let bb_stride = self.fb_width as usize;
            for row in 0..overlap.height as usize {
                let y = overlap.y as usize + row;
                for col in 0..overlap.width as usize {
                    let x = overlap.x as usize + col;
                    let di = y * bb_stride + x;
                    if di < self.back_buffer.len() {
                        self.back_buffer[di] = alpha_blend(SHADOW_COLOR, self.back_buffer[di]);
                    }
                }
            }
        }
    }

    /// Draw resize outline rectangle into back buffer.
    fn draw_outline_to_bb(&mut self, outline: &Rect) {
        let bb_stride = self.fb_width as usize;
        let color = 0xFF4A9EFF; // Blue outline
        let thickness = 2i32;

        // Top edge
        for t in 0..thickness {
            let y = outline.y + t;
            if y >= 0 && y < self.fb_height as i32 {
                for x in outline.x.max(0)..outline.right().min(self.fb_width as i32) {
                    let di = y as usize * bb_stride + x as usize;
                    if di < self.back_buffer.len() {
                        self.back_buffer[di] = color;
                    }
                }
            }
        }
        // Bottom edge
        for t in 0..thickness {
            let y = outline.bottom() - 1 - t;
            if y >= 0 && y < self.fb_height as i32 {
                for x in outline.x.max(0)..outline.right().min(self.fb_width as i32) {
                    let di = y as usize * bb_stride + x as usize;
                    if di < self.back_buffer.len() {
                        self.back_buffer[di] = color;
                    }
                }
            }
        }
        // Left edge
        for t in 0..thickness {
            let x = outline.x + t;
            if x >= 0 && x < self.fb_width as i32 {
                for y in outline.y.max(0)..outline.bottom().min(self.fb_height as i32) {
                    let di = y as usize * bb_stride + x as usize;
                    if di < self.back_buffer.len() {
                        self.back_buffer[di] = color;
                    }
                }
            }
        }
        // Right edge
        for t in 0..thickness {
            let x = outline.right() - 1 - t;
            if x >= 0 && x < self.fb_width as i32 {
                for y in outline.y.max(0)..outline.bottom().min(self.fb_height as i32) {
                    let di = y as usize * bb_stride + x as usize;
                    if di < self.back_buffer.len() {
                        self.back_buffer[di] = color;
                    }
                }
            }
        }
    }

    // ── Framebuffer I/O ─────────────────────────────────────────────────

    /// Copy a region from back buffer to the framebuffer (at y_offset for double-buffering).
    fn flush_region(&self, rect: &Rect, y_offset: u32) {
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

    // ── GPU Commands ────────────────────────────────────────────────────

    pub fn enable_double_buffer(&mut self) {
        self.hw_double_buffer = true;
        self.current_page = 0;
    }

    pub fn enable_gpu_accel(&mut self) {
        self.gpu_accel = true;
    }

    pub fn enable_hw_cursor(&mut self) {
        self.hw_cursor = true;
        self.gpu_cmds.push([GPU_CURSOR_SHOW, 1, 0, 0, 0, 0, 0, 0, 0]);
    }

    pub fn has_hw_cursor(&self) -> bool {
        self.hw_cursor
    }

    pub fn move_hw_cursor(&mut self, x: i32, y: i32) {
        if self.hw_cursor {
            self.gpu_cmds
                .push([GPU_CURSOR_MOVE, x as u32, y as u32, 0, 0, 0, 0, 0, 0]);
        }
    }

    /// Define HW cursor shape from ARGB pixel data.
    /// The pixel data must remain valid until flush_gpu() completes.
    pub fn define_hw_cursor(&mut self, w: u32, h: u32, hotx: u32, hoty: u32, pixels: &[u32]) {
        let ptr = pixels.as_ptr() as u64;
        let ptr_lo = ptr as u32;
        let ptr_hi = (ptr >> 32) as u32;
        let count = pixels.len() as u32;
        self.gpu_cmds.push([
            GPU_DEFINE_CURSOR,
            w,
            h,
            hotx,
            hoty,
            ptr_lo,
            ptr_hi,
            count,
            0,
        ]);
    }

    pub fn queue_gpu_update(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.gpu_cmds.push([GPU_UPDATE, x, y, w, h, 0, 0, 0, 0]);
    }

    pub fn flush_gpu(&mut self) {
        if !self.gpu_cmds.is_empty() {
            ipc::gpu_command(&self.gpu_cmds);
            self.gpu_cmds.clear();
        }
    }

    /// Flush a region from back buffer to the visible framebuffer (no offset).
    pub fn flush_region_pub(&self, rect: &Rect) {
        self.flush_region(rect, 0);
    }

    /// Full-screen damage (force recomposition of everything).
    pub fn damage_all(&mut self) {
        self.damage
            .push(Rect::new(0, 0, self.fb_width, self.fb_height));
    }
}

// ── Color Utilities ─────────────────────────────────────────────────────────

/// Alpha-blend src over dst (both ARGB8888).
#[inline]
pub fn alpha_blend(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    if sa == 0 {
        return dst;
    }
    if sa >= 255 {
        return src;
    }
    let inv = 255 - sa;
    let r = (((src >> 16) & 0xFF) * sa + ((dst >> 16) & 0xFF) * inv) / 255;
    let g = (((src >> 8) & 0xFF) * sa + ((dst >> 8) & 0xFF) * inv) / 255;
    let b = ((src & 0xFF) * sa + (dst & 0xFF) * inv) / 255;
    let a = sa + ((dst >> 24) & 0xFF) * inv / 255;
    (a << 24) | (r << 16) | (g << 8) | b
}
