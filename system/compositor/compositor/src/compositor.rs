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

// ── Shadow Cache ────────────────────────────────────────────────────────────

/// Pre-computed shadow alpha values for a given layer size.
/// Avoids expensive per-pixel SDF + isqrt computation every frame.
struct ShadowCache {
    /// Alpha values (0–255 normalized) for each pixel in the shadow bitmap.
    /// Layout: row-major, width = layer_w + 2*SHADOW_SPREAD.
    alphas: Vec<u8>,
    cache_w: u32,
    cache_h: u32,
    /// Layer dimensions this was computed for (invalidated on resize).
    layer_w: u32,
    layer_h: u32,
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
    /// Blur the back buffer behind this layer before compositing.
    pub blur_behind: bool,
    pub blur_radius: u32,
    /// Cached shadow alpha bitmap (computed lazily, invalidated on resize).
    shadow_cache: Option<ShadowCache>,
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

    /// Bounds including shadow (spread on all sides + vertical offset).
    pub fn shadow_bounds(&self) -> Rect {
        if self.has_shadow {
            let s = SHADOW_SPREAD;
            Rect::new(
                self.x + SHADOW_OFFSET_X - s,
                self.y + SHADOW_OFFSET_Y - s,
                (self.width as i32 + s * 2) as u32,
                (self.height as i32 + s * 2) as u32,
            )
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
const SHADOW_OFFSET_Y: i32 = 6;
/// Total spread (number of concentric rings) for the soft shadow.
const SHADOW_SPREAD: i32 = 16;
/// Shadow alpha for the focused window (innermost ring).
const SHADOW_ALPHA_FOCUSED: u32 = 50;
/// Shadow alpha for unfocused windows (innermost ring).
const SHADOW_ALPHA_UNFOCUSED: u32 = 25;

// ── AccelMoveHint ────────────────────────────────────────────────────────────

/// Tracks a pending GPU-accelerated layer move for RECT_COPY optimization.
/// Multiple move_layer() calls within the same frame are coalesced: keeps the
/// first old_bounds and updates new_bounds on each subsequent call.
#[allow(dead_code)]
struct AccelMoveHint {
    layer_id: u32,
    old_bounds: Rect,
    new_bounds: Rect,
}

/// Compute up to 4 non-overlapping rects covering `a` minus `b`.
/// Returns [top_strip, bottom_strip, left_strip, right_strip] (some may be empty).
#[allow(dead_code)]
fn subtract_rects(a: &Rect, b: &Rect) -> [Rect; 4] {
    let mut result = [Rect::new(0, 0, 0, 0); 4];
    if let Some(overlap) = a.intersect(b) {
        // Top strip (full width of a)
        if overlap.y > a.y {
            result[0] = Rect::new(a.x, a.y, a.width, (overlap.y - a.y) as u32);
        }
        // Bottom strip (full width of a)
        if overlap.bottom() < a.bottom() {
            result[1] = Rect::new(
                a.x,
                overlap.bottom(),
                a.width,
                (a.bottom() - overlap.bottom()) as u32,
            );
        }
        // Left strip (between top and bottom strips)
        if overlap.x > a.x {
            result[2] = Rect::new(
                a.x,
                overlap.y,
                (overlap.x - a.x) as u32,
                overlap.height,
            );
        }
        // Right strip (between top and bottom strips)
        if overlap.right() < a.right() {
            result[3] = Rect::new(
                overlap.right(),
                overlap.y,
                (a.right() - overlap.right()) as u32,
                overlap.height,
            );
        }
    } else {
        // No overlap — entire a is exposed
        result[0] = *a;
    }
    result
}

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

    /// The currently focused layer (gets stronger shadow)
    pub focused_layer_id: Option<u32>,

    /// Pending GPU-accelerated move hint for RECT_COPY optimization
    accel_move_hint: Option<AccelMoveHint>,
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
        if self.damage.len() > 64 {
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

        // Discard accel hint — RECT_COPY disabled (see compose_with_rect_copy)
        self.accel_move_hint = None;

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

        // GPU RECT_COPY disabled: CPU flush_region and GPU FIFO commands lack
        // synchronization — RECT_COPY reads VRAM after CPU already overwrote
        // exposed strips, producing artifacts. The opaque-run optimization in
        // composite_rect() provides the main performance win instead.
        {
            // Standard SW compositing path
            for rect in &damage {
                self.composite_rect(rect);
            }

            if let Some(outline) = self.resize_outline {
                self.draw_outline_to_bb(&outline);
            }

            if self.hw_double_buffer {
                let back_offset = if self.current_page == 0 {
                    self.fb_height
                } else {
                    0
                };
                for rect in &self.prev_damage {
                    self.flush_region(rect, back_offset);
                }
                for rect in &damage {
                    self.flush_region(rect, back_offset);
                }
                self.gpu_cmds.push([GPU_FLIP, 0, 0, 0, 0, 0, 0, 0, 0]);
                self.current_page = 1 - self.current_page;
                self.prev_damage = damage;
            } else {
                for rect in &damage {
                    self.flush_region(rect, 0);
                    self.gpu_cmds
                        .push([GPU_UPDATE, rect.x as u32, rect.y as u32, rect.width, rect.height, 0, 0, 0, 0]);
                }
            }

            self.flush_gpu();
        }
    }

    /// GPU-accelerated compositing for window drag (RECT_COPY fast path).
    ///
    /// Currently disabled: CPU flush_region writes to VRAM before GPU processes
    /// the RECT_COPY command, causing the copy to read partially-overwritten data.
    /// Needs proper SVGA FIFO sync (SYNC register + BUSY poll) to work correctly.
    #[allow(dead_code)]
    fn compose_with_rect_copy(&mut self, _damage: &[Rect], hint: &AccelMoveHint) {
        let old_b = hint.old_bounds.clip_to_screen(self.fb_width, self.fb_height);
        let new_b = hint.new_bounds.clip_to_screen(self.fb_width, self.fb_height);

        if old_b.is_empty() || new_b.is_empty() {
            return;
        }

        // Step 1: Compute exposed strips (old position minus new position overlap)
        let exposed = subtract_rects(&old_b, &new_b);

        // Step 2: SW-composite the exposed strips into back buffer
        for rect in &exposed {
            if !rect.is_empty() {
                self.composite_rect(rect);
            }
        }

        // Step 3: SW-composite new position into back buffer (keep in sync for future frames)
        self.composite_rect(&new_b);

        // Step 4: Draw resize outline if active
        if let Some(outline) = self.resize_outline {
            self.draw_outline_to_bb(&outline);
        }

        // Step 5: Flush exposed strips from back buffer → VRAM
        for rect in &exposed {
            if !rect.is_empty() {
                self.flush_region(rect, 0);
                self.gpu_cmds.push([
                    GPU_UPDATE,
                    rect.x as u32,
                    rect.y as u32,
                    rect.width,
                    rect.height,
                    0, 0, 0, 0,
                ]);
            }
        }

        // Step 6: GPU RECT_COPY — move window pixels within VRAM (hardware accelerated)
        self.gpu_cmds.push([
            GPU_RECT_COPY,
            old_b.x as u32,
            old_b.y as u32,
            new_b.x as u32,
            new_b.y as u32,
            new_b.width,
            new_b.height,
            0, 0,
        ]);

        // Step 7: Above-layer fixup — any layer ABOVE the moved layer that overlaps
        // the destination had its pixels overwritten by RECT_COPY (stale dock/menubar
        // pixels carried from the old position). Flush those intersections from the
        // back buffer which has the correct composited result.
        if let Some(moved_idx) = self.layer_index(hint.layer_id) {
            for li in (moved_idx + 1)..self.layers.len() {
                if !self.layers[li].visible {
                    continue;
                }
                let above_bounds = self.layers[li].damage_bounds();
                if let Some(intersection) = new_b.intersect(&above_bounds) {
                    self.flush_region(&intersection, 0);
                    self.gpu_cmds.push([
                        GPU_UPDATE,
                        intersection.x as u32,
                        intersection.y as u32,
                        intersection.width,
                        intersection.height,
                        0, 0, 0, 0,
                    ]);
                }
            }
        }

        // Step 8: Also UPDATE the new position so GPU displays the RECT_COPY result
        self.gpu_cmds.push([
            GPU_UPDATE,
            new_b.x as u32,
            new_b.y as u32,
            new_b.width,
            new_b.height,
            0, 0, 0, 0,
        ]);

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

            // Blur the back buffer behind this layer (frosted glass effect)
            let blur_behind = self.layers[li].blur_behind;
            let blur_radius = self.layers[li].blur_radius;
            if blur_behind && blur_radius > 0 {
                let lb = self.layers[li].bounds();
                if let Some(blur_area) = rect.intersect(&lb) {
                    blur_back_buffer_region(
                        &mut self.back_buffer, self.fb_width, self.fb_height,
                        blur_area.x, blur_area.y, blur_area.width, blur_area.height,
                        blur_radius, 2, // 2 passes ≈ triangle blur (fast + decent quality)
                    );
                }
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
                    // Alpha-blend path with opaque-run optimization.
                    // Windows with rounded corners have opaque=false, but 95%+ of
                    // pixels are a=255. Detect contiguous opaque runs per row and
                    // bulk-copy them via copy_from_slice (memcpy) instead of
                    // per-pixel branching.
                    for row in 0..overlap.height as usize {
                        let src_off = (sy + row) * lw + sx;
                        let dst_off =
                            (overlap.y as usize + row) * bb_stride + overlap.x as usize;
                        let row_width = overlap.width as usize;
                        let mut col = 0usize;
                        while col < row_width {
                            let si = src_off + col;
                            if si >= lp_len {
                                break;
                            }
                            let src_px = layer_pixels[si];
                            let a = src_px >> 24;
                            if a >= 255 {
                                // Scan ahead for contiguous opaque run
                                let run_start = col;
                                col += 1;
                                while col < row_width {
                                    let si2 = src_off + col;
                                    if si2 >= lp_len {
                                        break;
                                    }
                                    if layer_pixels[si2] >> 24 < 255 {
                                        break;
                                    }
                                    col += 1;
                                }
                                // Bulk copy the opaque run
                                let run_len = col - run_start;
                                let ss = src_off + run_start;
                                let ds = dst_off + run_start;
                                let safe = run_len
                                    .min(lp_len.saturating_sub(ss))
                                    .min(self.back_buffer.len().saturating_sub(ds));
                                if safe > 0 {
                                    self.back_buffer[ds..ds + safe]
                                        .copy_from_slice(&layer_pixels[ss..ss + safe]);
                                }
                            } else if a > 0 {
                                let di = dst_off + col;
                                if di < self.back_buffer.len() {
                                    self.back_buffer[di] =
                                        alpha_blend(src_px, self.back_buffer[di]);
                                }
                                col += 1;
                            } else {
                                // Fully transparent — skip
                                col += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Draw a soft gradient shadow for a layer into the back buffer (within damage rect).
    ///
    /// Uses a cached alpha bitmap (computed lazily per layer size) instead of
    /// per-pixel SDF + isqrt, which was the primary performance bottleneck when
    /// windows overlapped.
    fn draw_shadow_to_bb(&mut self, rect: &Rect, layer_idx: usize) {
        let layer_id = self.layers[layer_idx].id;
        let layer_w = self.layers[layer_idx].width;
        let layer_h = self.layers[layer_idx].height;
        let lx = self.layers[layer_idx].x + SHADOW_OFFSET_X;
        let ly = self.layers[layer_idx].y + SHADOW_OFFSET_Y;
        let spread = SHADOW_SPREAD;

        // Ensure shadow cache exists and matches current layer dimensions
        let needs_recompute = match &self.layers[layer_idx].shadow_cache {
            Some(c) => c.layer_w != layer_w || c.layer_h != layer_h,
            None => true,
        };
        if needs_recompute {
            let cache = compute_shadow_cache(layer_w, layer_h);
            self.layers[layer_idx].shadow_cache = Some(cache);
        }

        // Determine shadow intensity based on focus
        let base_alpha = if self.focused_layer_id == Some(layer_id) {
            SHADOW_ALPHA_FOCUSED
        } else {
            SHADOW_ALPHA_UNFOCUSED
        };

        // The full shadow bounding box
        let shadow_rect = Rect::new(
            lx - spread,
            ly - spread,
            (layer_w as i32 + spread * 2) as u32,
            (layer_h as i32 + spread * 2) as u32,
        );

        if let Some(overlap) = rect.intersect(&shadow_rect) {
            let bb_stride = self.fb_width as usize;
            let shadow_ox = lx - spread;
            let shadow_oy = ly - spread;

            // Split borrow: read cache from layers, write to back_buffer
            let cache = self.layers[layer_idx].shadow_cache.as_ref().unwrap();
            let cache_w = cache.cache_w as usize;
            let cache_alphas = cache.alphas.as_ptr();
            let cache_len = cache.alphas.len();
            let bb = &mut self.back_buffer;

            for row in 0..overlap.height as usize {
                let py = overlap.y + row as i32;
                let cy = (py - shadow_oy) as usize;
                let cache_row_off = cy * cache_w;
                let bb_row_off = py as usize * bb_stride;

                for col in 0..overlap.width as usize {
                    let px = overlap.x + col as i32;
                    let cx = (px - shadow_ox) as usize;

                    let cache_idx = cache_row_off + cx;
                    if cache_idx >= cache_len {
                        break;
                    }
                    let cache_a = unsafe { *cache_alphas.add(cache_idx) } as u32;
                    if cache_a == 0 {
                        continue;
                    }

                    // Scale normalized alpha (0-255) by base_alpha
                    let a = (cache_a * base_alpha + 127) / 255;
                    if a == 0 {
                        continue;
                    }

                    let di = bb_row_off + px as usize;
                    if di < bb.len() {
                        let shadow_px = a << 24; // pure black with computed alpha
                        bb[di] = alpha_blend(shadow_px, bb[di]);
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
            // Ensure all Write-Combining framebuffer stores are committed to VRAM
            // before the GPU processes UPDATE/FLIP commands. Without this barrier,
            // the CPU's WC buffers may not be flushed, causing the GPU to read
            // stale framebuffer data (rendering artifacts / partial updates).
            unsafe { core::arch::asm!("sfence", options(nostack, preserves_flags)); }
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
    }
}

// ── Color Utilities ─────────────────────────────────────────────────────────

/// Pre-compute shadow alpha values for a layer of given dimensions.
/// The result is a bitmap of (layer_w + 2*spread) x (layer_h + 2*spread) alpha values
/// representing the shadow intensity at each pixel, normalized to 0-255.
fn compute_shadow_cache(layer_w: u32, layer_h: u32) -> ShadowCache {
    let spread = SHADOW_SPREAD;
    let lw = layer_w as i32;
    let lh = layer_h as i32;
    let corner_r = 8i32; // must match the window corner radius
    let s = spread as u32;

    let cache_w = (lw + spread * 2) as u32;
    let cache_h = (lh + spread * 2) as u32;
    let mut alphas = vec![0u8; (cache_w * cache_h) as usize];

    // The virtual layer starts at (spread, spread) within the cache bitmap
    let lx = spread;
    let ly = spread;

    for row in 0..cache_h {
        let py = row as i32;
        for col in 0..cache_w {
            let px = col as i32;

            let dist = rounded_rect_sdf(px, py, lx, ly, lw, lh, corner_r);

            if dist >= spread {
                continue;
            }

            if dist <= 0 {
                // Inside the shadow shape: full alpha.
                // The window layer drawn on top will cover the interior;
                // this ensures no gap when the shadow is offset.
                alphas[(row * cache_w + col) as usize] = 255;
            } else {
                let t = dist as u32;
                let inv = s - t;
                // Normalized alpha: quadratic falloff scaled to 0-255
                let a = (255 * inv * inv) / (s * s);
                alphas[(row * cache_w + col) as usize] = a.min(255) as u8;
            }
        }
    }

    ShadowCache {
        alphas,
        cache_w,
        cache_h,
        layer_w,
        layer_h,
    }
}

/// Signed distance from point (px,py) to a rounded rectangle.
/// Returns negative values inside, positive outside, 0 on the edge.
/// Uses integer arithmetic (no floating point).
#[inline]
fn rounded_rect_sdf(px: i32, py: i32, rx: i32, ry: i32, rw: i32, rh: i32, r: i32) -> i32 {
    // Clamp r to half the smallest dimension
    let r = r.min(rw / 2).min(rh / 2).max(0);

    // Distance from point to the inner rect (shrunk by radius)
    let inner_x0 = rx + r;
    let inner_y0 = ry + r;
    let inner_x1 = rx + rw - r;
    let inner_y1 = ry + rh - r;

    // Compute distance to the inner rect
    let dx = if px < inner_x0 {
        inner_x0 - px
    } else if px >= inner_x1 {
        px - inner_x1 + 1
    } else {
        0
    };
    let dy = if py < inner_y0 {
        inner_y0 - py
    } else if py >= inner_y1 {
        py - inner_y1 + 1
    } else {
        0
    };

    if dx == 0 && dy == 0 {
        // Inside the inner rect — compute distance to edge (negative)
        let to_left = px - rx;
        let to_right = rx + rw - 1 - px;
        let to_top = py - ry;
        let to_bottom = ry + rh - 1 - py;
        let min_edge = to_left.min(to_right).min(to_top).min(to_bottom);
        -min_edge
    } else if dx > 0 && dy > 0 {
        // In a corner region — use Euclidean distance to corner
        isqrt_u32((dx * dx + dy * dy) as u32) as i32 - r
    } else {
        // Along an edge — simple distance
        let d = dx.max(dy);
        d - r
    }
}

/// Integer square root (for u32).
#[inline]
fn isqrt_u32(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Fast two-pass (H+V) box blur on a rectangular region of a pixel buffer.
/// `passes` iterations: 1=box, 2=triangle, 3≈gaussian.
fn blur_back_buffer_region(
    bb: &mut [u32], fb_w: u32, fb_h: u32,
    rx: i32, ry: i32, rw: u32, rh: u32,
    radius: u32, passes: u32,
) {
    if rw == 0 || rh == 0 || radius == 0 || passes == 0 { return; }
    let x0 = rx.max(0) as usize;
    let y0 = ry.max(0) as usize;
    let x1 = ((rx + rw as i32) as usize).min(fb_w as usize);
    let y1 = ((ry + rh as i32) as usize).min(fb_h as usize);
    if x0 >= x1 || y0 >= y1 { return; }
    let w = x1 - x0;
    let h = y1 - y0;
    let stride = fb_w as usize;
    let r = radius as usize;
    let kernel = (2 * r + 1) as u32;

    let max_dim = w.max(h);
    let mut temp = vec![0u32; max_dim];

    for _ in 0..passes {
        // Horizontal pass
        for row in y0..y1 {
            let row_off = row * stride;
            let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
            for i in 0..=(2 * r) {
                let sx = (x0 as i32 + i as i32 - r as i32).max(0).min(fb_w as i32 - 1) as usize;
                let px = bb[row_off + sx];
                sr += (px >> 16) & 0xFF;
                sg += (px >> 8) & 0xFF;
                sb += px & 0xFF;
            }
            for col in 0..w {
                let cx = x0 + col;
                temp[col] = 0xFF000000 | ((sr / kernel) << 16) | ((sg / kernel) << 8) | (sb / kernel);
                let add_x = (cx as i32 + r as i32 + 1).min(fb_w as i32 - 1).max(0) as usize;
                let rem_x = (cx as i32 - r as i32).max(0).min(fb_w as i32 - 1) as usize;
                let add_px = bb[row_off + add_x];
                let rem_px = bb[row_off + rem_x];
                sr += ((add_px >> 16) & 0xFF) - ((rem_px >> 16) & 0xFF);
                sg += ((add_px >> 8) & 0xFF) - ((rem_px >> 8) & 0xFF);
                sb += (add_px & 0xFF) - (rem_px & 0xFF);
            }
            for col in 0..w {
                bb[row_off + x0 + col] = temp[col];
            }
        }
        // Vertical pass
        for col in x0..x1 {
            let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
            for i in 0..=(2 * r) {
                let sy = (y0 as i32 + i as i32 - r as i32).max(0).min(fb_h as i32 - 1) as usize;
                let px = bb[sy * stride + col];
                sr += (px >> 16) & 0xFF;
                sg += (px >> 8) & 0xFF;
                sb += px & 0xFF;
            }
            for row in 0..h {
                let cy = y0 + row;
                temp[row] = 0xFF000000 | ((sr / kernel) << 16) | ((sg / kernel) << 8) | (sb / kernel);
                let add_y = (cy as i32 + r as i32 + 1).min(fb_h as i32 - 1).max(0) as usize;
                let rem_y = (cy as i32 - r as i32).max(0).min(fb_h as i32 - 1) as usize;
                let add_px = bb[add_y * stride + col];
                let rem_px = bb[rem_y * stride + col];
                sr += ((add_px >> 16) & 0xFF) - ((rem_px >> 16) & 0xFF);
                sg += ((add_px >> 8) & 0xFF) - ((rem_px >> 8) & 0xFF);
                sb += (add_px & 0xFF) - (rem_px & 0xFF);
            }
            for row in 0..h {
                bb[(y0 + row) * stride + col] = temp[row];
            }
        }
    }
}

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
