//! Double-buffered window compositor with damage-based recompositing.
//!
//! Manages Z-ordered layers, software and hardware cursors, GPU-accelerated
//! RECT_COPY moves, and optional hardware page flipping.

use alloc::vec::Vec;
use crate::graphics::color::Color;
use crate::graphics::rect::Rect;
use crate::graphics::surface::Surface;

/// Cursor shape for different interaction zones
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Arrow,
    ResizeHorizontal, // ↔ left/right edges
    ResizeVertical,   // ↕ bottom edge
    ResizeNWSE,       // ↘ bottom-right corner
    ResizeNESW,       // ↙ bottom-left corner
}

/// Window layer in the compositor
pub struct Layer {
    pub id: u32,
    pub surface: Surface,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    pub dirty: bool,
}

impl Layer {
    /// Create a new layer at the given position and dimensions.
    pub fn new(id: u32, x: i32, y: i32, width: u32, height: u32) -> Self {
        Layer {
            id,
            surface: Surface::new(width, height),
            x,
            y,
            visible: true,
            dirty: true,
        }
    }

    /// Return the screen-space bounding rectangle of this layer.
    pub fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, self.surface.width, self.surface.height)
    }
}

/// Hint for GPU-accelerated layer move (RECT_COPY optimization).
/// Stored between move_layer() and compose() to avoid redundant VRAM writes.
struct AccelMoveHint {
    layer_id: u32,
    old_bounds: Rect,
    new_bounds: Rect,
}

/// Double-buffered compositor with z-ordered layers
pub struct Compositor {
    /// Back buffer (composited image)
    back_buffer: Surface,
    /// Framebuffer pointer and pitch
    framebuffer_addr: u32,
    framebuffer_pitch: u32,
    /// Screen dimensions
    pub width: u32,
    pub height: u32,
    /// Layers ordered back-to-front (index 0 = bottom)
    layers: Vec<Layer>,
    /// Next layer ID
    next_id: u32,
    /// Damage regions that need recompositing
    damage: Vec<Rect>,
    /// Mouse cursor state
    cursor_x: i32,
    cursor_y: i32,
    cursor_visible: bool,
    /// Hardware double-buffer: flush to HW back page, then flip
    hw_double_buffer: bool,
    /// Previous frame's damage regions (for HW double-buffer page sync)
    prev_damage: Vec<Rect>,
    /// GPU has hardware cursor — skip SW cursor drawing
    hw_cursor_active: bool,
    /// GPU has 2D acceleration
    gpu_accel: bool,
    /// Pending GPU RECT_COPY hint for accelerated window move
    accel_move: Option<AccelMoveHint>,
    /// Resize outline overlay (drawn during compose, zero allocation)
    resize_outline: Option<Rect>,
    /// Current cursor shape (for HW cursor shape switching)
    current_cursor: CursorShape,
}

impl Compositor {
    /// Create a new compositor targeting the given framebuffer.
    pub fn new(width: u32, height: u32, framebuffer_addr: u32, framebuffer_pitch: u32) -> Self {
        Compositor {
            back_buffer: Surface::new_with_color(width, height, Color::MACOS_BG),
            framebuffer_addr,
            framebuffer_pitch,
            width,
            height,
            layers: Vec::new(),
            next_id: 1,
            damage: Vec::new(),
            cursor_x: (width / 2) as i32,
            cursor_y: (height / 2) as i32,
            hw_double_buffer: false,
            prev_damage: Vec::new(),
            cursor_visible: true,
            hw_cursor_active: false,
            gpu_accel: false,
            accel_move: None,
            resize_outline: None,
            current_cursor: CursorShape::Arrow,
        }
    }

    /// Create a new layer and return its ID
    pub fn create_layer(&mut self, x: i32, y: i32, width: u32, height: u32) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let layer = Layer::new(id, x, y, width, height);
        self.damage.push(layer.bounds());
        self.layers.push(layer);
        id
    }

    /// Get a mutable reference to a layer's surface for drawing
    pub fn get_layer_surface(&mut self, id: u32) -> Option<&mut Surface> {
        for layer in self.layers.iter_mut() {
            if layer.id == id {
                layer.dirty = true;
                return Some(&mut layer.surface);
            }
        }
        None
    }

    /// Get a reference to a layer
    pub fn get_layer(&self, id: u32) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// Get a mutable reference to a layer
    pub fn get_layer_mut(&mut self, id: u32) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// Move a layer to a new position
    pub fn move_layer(&mut self, id: u32, x: i32, y: i32) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            let old_bounds = layer.bounds();
            layer.x = x;
            layer.y = y;
            let new_bounds = layer.bounds();

            if self.gpu_accel {
                // GPU accel path: coalesce all moves into one hint (2 damage rects
                // instead of 2*N). compose() will derive damage from the hint.
                let mut coalesced = false;
                if let Some(ref mut hint) = self.accel_move {
                    if hint.layer_id == id {
                        // Same layer moved again this frame — update destination only
                        hint.new_bounds = new_bounds;
                        coalesced = true;
                    }
                }
                if !coalesced {
                    // Flush any previous hint for a different layer
                    if let Some(prev) = self.accel_move.take() {
                        self.damage.push(prev.old_bounds);
                        self.damage.push(prev.new_bounds);
                    }
                    self.accel_move = Some(AccelMoveHint {
                        layer_id: id,
                        old_bounds,
                        new_bounds,
                    });
                }
            } else {
                // No GPU accel — push damage directly
                self.damage.push(old_bounds);
                self.damage.push(new_bounds);
            }
        }
    }

    /// Raise a layer to the top of the z-order
    pub fn raise_layer(&mut self, id: u32) {
        if let Some(pos) = self.layers.iter().position(|l| l.id == id) {
            if pos < self.layers.len() - 1 {
                let layer = self.layers.remove(pos);
                self.damage.push(layer.bounds());
                self.layers.push(layer);
            }
        }
    }

    /// Remove a layer
    pub fn remove_layer(&mut self, id: u32) {
        if let Some(pos) = self.layers.iter().position(|l| l.id == id) {
            let layer = self.layers.remove(pos);
            self.damage.push(layer.bounds());
        }
    }

    /// Set layer visibility
    pub fn set_layer_visible(&mut self, id: u32, visible: bool) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            if layer.visible != visible {
                layer.visible = visible;
                self.damage.push(layer.bounds());
            }
        }
    }

    /// Update cursor position
    pub fn move_cursor(&mut self, x: i32, y: i32) {
        if self.hw_cursor_active {
            // Move hardware cursor — no SW damage needed
            self.cursor_x = x.max(0).min(self.width as i32 - 1);
            self.cursor_y = y.max(0).min(self.height as i32 - 1);
            crate::drivers::gpu::with_gpu(|g| {
                g.move_cursor(self.cursor_x as u32, self.cursor_y as u32);
            });
        } else {
            // Software cursor: damage old position
            self.damage_cursor();
            self.cursor_x = x.max(0).min(self.width as i32 - 1);
            self.cursor_y = y.max(0).min(self.height as i32 - 1);
            // Damage new position
            self.damage_cursor();
        }
    }

    /// Return the current cursor position as (x, y).
    pub fn cursor_position(&self) -> (i32, i32) {
        (self.cursor_x, self.cursor_y)
    }

    fn damage_cursor(&mut self) {
        self.damage.push(Rect::new(
            self.cursor_x - 1,
            self.cursor_y - 1,
            16,
            20,
        ));
    }

    /// Mark entire screen as damaged (full recomposite)
    pub fn invalidate_all(&mut self) {
        self.damage.push(Rect::new(0, 0, self.width, self.height));
        self.accel_move = None;
    }

    /// Set a resize outline overlay (drawn during compose, no allocation)
    pub fn set_resize_outline(&mut self, rect: Rect) {
        // Damage old outline position
        if let Some(old) = self.resize_outline {
            self.damage.push(old);
        }
        self.damage.push(rect);
        self.resize_outline = Some(rect);
    }

    /// Clear the resize outline overlay
    pub fn clear_resize_outline(&mut self) {
        if let Some(old) = self.resize_outline.take() {
            self.damage.push(old);
        }
    }

    /// Mark a layer as dirty
    pub fn invalidate_layer(&mut self, id: u32) {
        if let Some(layer) = self.layers.iter_mut().find(|l| l.id == id) {
            layer.dirty = true;
            self.damage.push(layer.bounds());
        }
    }

    /// Find the topmost layer at a given screen coordinate
    pub fn layer_at(&self, x: i32, y: i32) -> Option<u32> {
        for layer in self.layers.iter().rev() {
            if layer.visible && layer.bounds().contains(x, y) {
                return Some(layer.id);
            }
        }
        None
    }

    /// Maximum individual damage rects before merging to single bounding box
    const MAX_DAMAGE_RECTS: usize = 16;

    /// Composite all layers into the back buffer and flush to the framebuffer.
    /// Uses individual damage regions to minimize work.
    pub fn compose(&mut self) {
        // Derive move damage from accel hint (coalesced: just 2 rects for any number of moves)
        if let Some(ref hint) = self.accel_move {
            self.damage.push(hint.old_bounds);
            self.damage.push(hint.new_bounds);
        }

        // Always add dirty layer bounds to damage list
        for i in 0..self.layers.len() {
            if self.layers[i].dirty {
                let bounds = self.layers[i].bounds();
                self.damage.push(bounds);
            }
        }

        if self.damage.is_empty() {
            // HW double-buffer: still sync prev_damage even with no new work
            if self.hw_double_buffer && !self.prev_damage.is_empty() {
                for rect in &self.prev_damage {
                    self.flush_region(*rect);
                }
                self.prev_damage.clear();
            }
            self.accel_move = None;
            return;
        }

        // Merge overlapping damage rects to avoid duplicate compositing work.
        // During a drag, old_bounds and new_bounds overlap ~90% for small moves;
        // merging halves Phase 1 SW composite and Phase 2 flush cost.
        if self.damage.len() > 1 {
            let mut merged = true;
            while merged {
                merged = false;
                let mut i = 0;
                while i < self.damage.len() {
                    let mut j = i + 1;
                    while j < self.damage.len() {
                        if self.damage[i].intersects(&self.damage[j]) {
                            self.damage[i] = self.damage[i].union(&self.damage[j]);
                            self.damage.swap_remove(j);
                            merged = true;
                        } else {
                            j += 1;
                        }
                    }
                    i += 1;
                }
            }
        }

        // If too many damage rects, merge to single bounding rect
        if self.damage.len() > Self::MAX_DAMAGE_RECTS {
            let mut bounds = self.damage[0];
            for r in &self.damage[1..] {
                bounds = bounds.union(r);
            }
            self.damage.clear();
            self.damage.push(bounds);
            // Merged damage invalidates RECT_COPY optimization
            self.accel_move = None;
        }

        // Clip all damage rects to screen and collect
        let screen_rect = Rect::new(0, 0, self.width, self.height);
        let clipped: Vec<Rect> = self.damage.iter()
            .filter_map(|r| r.intersection(&screen_rect))
            .collect();

        if clipped.is_empty() {
            self.damage.clear();
            self.accel_move = None;
            // HW double-buffer: still need to sync prev_damage to current back page
            if self.hw_double_buffer && !self.prev_damage.is_empty() {
                for rect in &self.prev_damage {
                    self.flush_region(*rect);
                }
                self.prev_damage.clear();
            }
            return;
        }

        // HW double-buffer catch-up: the current back page was the front page last
        // frame, so it's missing the previous frame's updates. Flush them now from
        // the (already-correct) software back buffer.
        if self.hw_double_buffer {
            for rect in &self.prev_damage {
                self.flush_region(*rect);
            }
        }

        // ── Phase 1: SW composite all damage to back buffer ──
        for damage_rect in &clipped {
            // Clear the damaged region with background color
            self.back_buffer.fill_rect(*damage_rect, Color::MACOS_BG);

            // Composite each visible layer that intersects this damage rect
            for layer_idx in 0..self.layers.len() {
                if !self.layers[layer_idx].visible {
                    continue;
                }

                let layer_bounds = self.layers[layer_idx].bounds();
                if let Some(intersection) = layer_bounds.intersection(damage_rect) {
                    // Blit only the clipped region for efficiency
                    let src_rect = Rect::new(
                        intersection.x - self.layers[layer_idx].x,
                        intersection.y - self.layers[layer_idx].y,
                        intersection.width,
                        intersection.height,
                    );
                    self.back_buffer.blit_rect(
                        &self.layers[layer_idx].surface,
                        src_rect,
                        intersection.x,
                        intersection.y,
                    );
                }
            }

            // Draw resize outline if it intersects this damage rect
            if let Some(outline) = self.resize_outline {
                if outline.intersects(damage_rect) {
                    let border = Color::from_u32(0x80FFFFFF);
                    // Top
                    if let Some(r) = Rect::new(outline.x, outline.y, outline.width, 2).intersection(damage_rect) {
                        self.back_buffer.fill_rect(r, border);
                    }
                    // Bottom
                    if let Some(r) = Rect::new(outline.x, outline.bottom() - 2, outline.width, 2).intersection(damage_rect) {
                        self.back_buffer.fill_rect(r, border);
                    }
                    // Left
                    if let Some(r) = Rect::new(outline.x, outline.y, 2, outline.height).intersection(damage_rect) {
                        self.back_buffer.fill_rect(r, border);
                    }
                    // Right
                    if let Some(r) = Rect::new(outline.right() - 2, outline.y, 2, outline.height).intersection(damage_rect) {
                        self.back_buffer.fill_rect(r, border);
                    }
                }
            }

            // Draw software cursor if it intersects this damage rect (skip if HW cursor active)
            if self.cursor_visible && !self.hw_cursor_active {
                let cursor_rect = Rect::new(self.cursor_x - 1, self.cursor_y - 1, 16, 20);
                if cursor_rect.intersects(damage_rect) {
                    self.draw_cursor();
                }
            }
        }

        // ── Phase 2: Flush to framebuffer (with optional RECT_COPY acceleration) ──
        let accel_hint = self.accel_move.take();
        let mut used_accel = false;

        if !self.hw_double_buffer {
            if let Some(ref hint) = accel_hint {
                if let Some(pos) = self.layers.iter().position(|l| l.id == hint.layer_id) {
                    let layer_opaque = self.layers[pos].surface.opaque;
                    // Pure translation: same width/height
                    let same_size = hint.old_bounds.width == hint.new_bounds.width
                        && hint.old_bounds.height == hint.new_bounds.height;

                    if layer_opaque && same_size {
                        // Clip to screen for the actual region
                        let old_r = hint.old_bounds.intersection(&screen_rect);
                        let new_r = hint.new_bounds.intersection(&screen_rect);

                        if let (Some(old_r), Some(new_r)) = (old_r, new_r) {
                            // QEMU's SVGA RECT_COPY uses forward-only iteration,
                            // which corrupts overlapping regions (top-left smear).
                            // Use RECT_COPY only when old and new don't overlap;
                            // otherwise fall back to optimized back-buffer flush.
                            let overlaps = old_r.intersects(&new_r);

                            let did_copy = if !overlaps {
                                let copy_w = old_r.width.min(new_r.width);
                                let copy_h = old_r.height.min(new_r.height);
                                crate::drivers::gpu::with_gpu(|g| {
                                    g.accel_copy_rect(
                                        old_r.x as u32, old_r.y as u32,
                                        new_r.x as u32, new_r.y as u32,
                                        copy_w, copy_h,
                                    )
                                }).unwrap_or(false)
                            } else {
                                false
                            };

                            used_accel = true;

                            if did_copy {
                                // Non-overlapping RECT_COPY succeeded
                                let copy_w = old_r.width.min(new_r.width);
                                let copy_h = old_r.height.min(new_r.height);
                                let copied_dest = Rect::new(new_r.x, new_r.y, copy_w, copy_h);

                                // 1. Flush exposed area (old position minus new position)
                                //    Use GPU fill for background-only exposed strips.
                                let (exposed, exp_count) = old_r.subtract(&new_r);
                                for i in 0..exp_count {
                                    if self.is_background_only(&exposed[i]) {
                                        crate::drivers::gpu::with_gpu(|g| {
                                            g.accel_fill_rect(
                                                exposed[i].x.max(0) as u32,
                                                exposed[i].y.max(0) as u32,
                                                exposed[i].width,
                                                exposed[i].height,
                                                Color::MACOS_BG.to_u32(),
                                            );
                                        });
                                    } else {
                                        self.flush_region(exposed[i]);
                                    }
                                }

                                // 2. Flush parts of new_r not covered by RECT_COPY
                                if copy_w < new_r.width || copy_h < new_r.height {
                                    let (uncovered, unc_count) = new_r.subtract(&copied_dest);
                                    for i in 0..unc_count {
                                        self.flush_region(uncovered[i]);
                                    }
                                }

                                // 3. Correct above-layer artifacts
                                let layer_count = self.layers.len();
                                for li in pos + 1..layer_count {
                                    if self.layers[li].visible {
                                        if let Some(fix_rect) = self.layers[li].bounds().intersection(&copied_dest) {
                                            self.flush_region(fix_rect);
                                        }
                                    }
                                }
                            } else {
                                // Overlapping or RECT_COPY failed: flush from back buffer.
                                // Only flush new position + exposed strips (not full old+new).
                                self.flush_region(new_r);

                                let (exposed, exp_count) = old_r.subtract(&new_r);
                                for i in 0..exp_count {
                                    self.flush_region(exposed[i]);
                                }
                            }

                            // Flush any damage rects not part of the move
                            for rect in &clipped {
                                if !hint.old_bounds.intersects(rect) && !hint.new_bounds.intersects(rect) {
                                    self.flush_region(*rect);
                                }
                            }
                        }
                    }
                }
            }
        }

        if !used_accel {
            for rect in &clipped {
                if self.gpu_accel && self.is_background_only(rect) {
                    // Pure background — GPU RECT_FILL directly, skip memcpy
                    crate::drivers::gpu::with_gpu(|g| {
                        g.accel_fill_rect(
                            rect.x.max(0) as u32,
                            rect.y.max(0) as u32,
                            rect.width,
                            rect.height,
                            Color::MACOS_BG.to_u32(),
                        );
                    });
                } else {
                    self.flush_region(*rect);
                }
            }
        }

        // Notify GPU of all updated regions in a single batched UPDATE.
        // One FIFO command per frame is much faster than one per damage rect.
        if !self.hw_double_buffer && !clipped.is_empty() {
            let mut bounds = clipped[0];
            for r in &clipped[1..] {
                bounds = bounds.union(r);
            }
            crate::drivers::gpu::with_gpu(|g| {
                let x = bounds.x.max(0) as u32;
                let y = bounds.y.max(0) as u32;
                g.update_rect(x, y, bounds.width, bounds.height);
            });
        }

        // Save current damage for next frame's back-page catch-up
        if self.hw_double_buffer {
            self.prev_damage = clipped;
        }

        // Clear damage
        self.damage.clear();
        for layer in &mut self.layers {
            layer.dirty = false;
        }

        // Hardware page flip if double-buffering is active
        if self.hw_double_buffer {
            crate::drivers::gpu::with_gpu(|g| {
                g.flip();
            });
            // Update framebuffer_addr to the new back page
            if let Some(back) = crate::drivers::gpu::with_gpu(|g| g.back_buffer_phys()).flatten() {
                self.framebuffer_addr = back;
            }
        }
    }

    /// Enable hardware double-buffering via GPU driver.
    pub fn enable_hw_double_buffer(&mut self) {
        let back = crate::drivers::gpu::with_gpu(|g| g.back_buffer_phys()).flatten();
        if let Some(back_addr) = back {
            self.framebuffer_addr = back_addr;
            self.hw_double_buffer = true;
            // Do an initial full flush to the back page, then flip
            self.flush_all();
            crate::drivers::gpu::with_gpu(|g| g.flip());
            // Point to the new back page for future rendering
            if let Some(new_back) = crate::drivers::gpu::with_gpu(|g| g.back_buffer_phys()).flatten() {
                self.framebuffer_addr = new_back;
            }
        }
    }

    /// Enable hardware cursor via GPU driver.
    pub fn enable_hw_cursor(&mut self) {
        self.hw_cursor_active = true;
        self.cursor_visible = true;
        // Define cursor shape and position
        crate::drivers::gpu::with_gpu(|g| {
            // Build a simple white arrow cursor with black outline (12x18)
            let mut pixels = [0u32; 12 * 18];
            static CURSOR_BODY: [u16; 18] = [
                0b1000000000000000, 0b1100000000000000, 0b1110000000000000,
                0b1111000000000000, 0b1111100000000000, 0b1111110000000000,
                0b1111111000000000, 0b1111111100000000, 0b1111111110000000,
                0b1111111111000000, 0b1111111111100000, 0b1111110000000000,
                0b1110011000000000, 0b1100011000000000, 0b1000001100000000,
                0b0000001100000000, 0b0000000110000000, 0b0000000000000000,
            ];
            static CURSOR_OUTLINE: [u16; 18] = [
                0b1100000000000000, 0b1010000000000000, 0b1001000000000000,
                0b1000100000000000, 0b1000010000000000, 0b1000001000000000,
                0b1000000100000000, 0b1000000010000000, 0b1000000001000000,
                0b1000000000100000, 0b1000000000010000, 0b1000001110000000,
                0b1001000100000000, 0b1010000100000000, 0b1100000010000000,
                0b0000000010000000, 0b0000000001100000, 0b0000000000000000,
            ];
            for row in 0..18 {
                let body = CURSOR_BODY[row];
                let outline = CURSOR_OUTLINE[row];
                for col in 0..12 {
                    let mask = 0x8000u16 >> col;
                    let idx = row * 12 + col;
                    if outline & mask != 0 {
                        pixels[idx] = 0xFF000000; // black outline, fully opaque
                    } else if body & mask != 0 {
                        pixels[idx] = 0xFFFFFFFF; // white body, fully opaque
                    }
                    // else: transparent (0x00000000)
                }
            }
            g.define_cursor(12, 18, 0, 0, &pixels);
            g.show_cursor(true);
            g.move_cursor(self.cursor_x as u32, self.cursor_y as u32);
        });
    }

    /// Change hardware cursor shape. No-op if shape is already set or no HW cursor.
    pub fn set_cursor_shape(&mut self, shape: CursorShape) {
        if !self.hw_cursor_active || shape == self.current_cursor {
            return;
        }
        self.current_cursor = shape;

        crate::drivers::gpu::with_gpu(|g| {
            match shape {
                CursorShape::Arrow => {
                    // Standard arrow cursor (12x18, hotspot 0,0)
                    let mut pixels = [0u32; 12 * 18];
                    static BODY: [u16; 18] = [
                        0b1000000000000000, 0b1100000000000000, 0b1110000000000000,
                        0b1111000000000000, 0b1111100000000000, 0b1111110000000000,
                        0b1111111000000000, 0b1111111100000000, 0b1111111110000000,
                        0b1111111111000000, 0b1111111111100000, 0b1111110000000000,
                        0b1110011000000000, 0b1100011000000000, 0b1000001100000000,
                        0b0000001100000000, 0b0000000110000000, 0b0000000000000000,
                    ];
                    static OUTLINE: [u16; 18] = [
                        0b1100000000000000, 0b1010000000000000, 0b1001000000000000,
                        0b1000100000000000, 0b1000010000000000, 0b1000001000000000,
                        0b1000000100000000, 0b1000000010000000, 0b1000000001000000,
                        0b1000000000100000, 0b1000000000010000, 0b1000001110000000,
                        0b1001000100000000, 0b1010000100000000, 0b1100000010000000,
                        0b0000000010000000, 0b0000000001100000, 0b0000000000000000,
                    ];
                    for row in 0..18 {
                        for col in 0..12 {
                            let mask = 0x8000u16 >> col;
                            let idx = row * 12 + col;
                            if OUTLINE[row] & mask != 0 {
                                pixels[idx] = 0xFF000000;
                            } else if BODY[row] & mask != 0 {
                                pixels[idx] = 0xFFFFFFFF;
                            }
                        }
                    }
                    g.define_cursor(12, 18, 0, 0, &pixels);
                }
                CursorShape::ResizeHorizontal => {
                    // ↔ horizontal resize cursor (16x10, hotspot 8,5)
                    let mut pixels = [0u32; 16 * 10];
                    // Horizontal double-arrow: two arrows pointing left and right
                    static H_BODY: [u16; 10] = [
                        0b0000000000000000,
                        0b0000100000100000,
                        0b0001100000110000,
                        0b0011111111111000,
                        0b0111111111111100,
                        0b0111111111111100,
                        0b0011111111111000,
                        0b0001100000110000,
                        0b0000100000100000,
                        0b0000000000000000,
                    ];
                    static H_OUTLINE: [u16; 10] = [
                        0b0000100000100000,
                        0b0001010000010000,
                        0b0010011111001000,
                        0b0100000000000100,
                        0b1000000000000010,
                        0b1000000000000010,
                        0b0100000000000100,
                        0b0010011111001000,
                        0b0001010000010000,
                        0b0000100000100000,
                    ];
                    for row in 0..10 {
                        for col in 0..16 {
                            let mask = 0x8000u16 >> col;
                            let idx = row * 16 + col;
                            if H_OUTLINE[row] & mask != 0 {
                                pixels[idx] = 0xFF000000;
                            } else if H_BODY[row] & mask != 0 {
                                pixels[idx] = 0xFFFFFFFF;
                            }
                        }
                    }
                    g.define_cursor(16, 10, 8, 5, &pixels);
                }
                CursorShape::ResizeVertical => {
                    // ↕ vertical resize cursor (10x16, hotspot 5,8)
                    let mut pixels = [0u32; 10 * 16];
                    static V_BODY: [u16; 16] = [
                        0b0000000000000000,
                        0b0001000000000000,
                        0b0011100000000000,
                        0b0111110000000000,
                        0b0011100000000000,
                        0b0011100000000000,
                        0b0011100000000000,
                        0b0011100000000000,
                        0b0011100000000000,
                        0b0011100000000000,
                        0b0011100000000000,
                        0b0111110000000000,
                        0b0011100000000000,
                        0b0001000000000000,
                        0b0000000000000000,
                        0b0000000000000000,
                    ];
                    static V_OUTLINE: [u16; 16] = [
                        0b0001000000000000,
                        0b0010100000000000,
                        0b0100010000000000,
                        0b1000001000000000,
                        0b0100010000000000,
                        0b0100010000000000,
                        0b0100010000000000,
                        0b0100010000000000,
                        0b0100010000000000,
                        0b0100010000000000,
                        0b0100010000000000,
                        0b1000001000000000,
                        0b0100010000000000,
                        0b0010100000000000,
                        0b0001000000000000,
                        0b0000000000000000,
                    ];
                    for row in 0..16 {
                        for col in 0..10 {
                            let mask = 0x8000u16 >> col;
                            let idx = row * 10 + col;
                            if V_OUTLINE[row] & mask != 0 {
                                pixels[idx] = 0xFF000000;
                            } else if V_BODY[row] & mask != 0 {
                                pixels[idx] = 0xFFFFFFFF;
                            }
                        }
                    }
                    g.define_cursor(10, 16, 5, 8, &pixels);
                }
                CursorShape::ResizeNWSE => {
                    // ↘ diagonal NW-SE cursor (14x14, hotspot 7,7)
                    let mut pixels = [0u32; 14 * 14];
                    static NWSE_BODY: [u16; 14] = [
                        0b0000000000000000,
                        0b0111110000000000,
                        0b0011110000000000,
                        0b0001110000000000,
                        0b0001110000000000,
                        0b0000111000000000,
                        0b0000011100000000,
                        0b0000001110000000,
                        0b0000001110000000,
                        0b0000000111000000,
                        0b0000000111000000,
                        0b0000000111000000,
                        0b0000011111000000,
                        0b0000000000000000,
                    ];
                    static NWSE_OUTLINE: [u16; 14] = [
                        0b0111111000000000,
                        0b1000001000000000,
                        0b0100001000000000,
                        0b0010001000000000,
                        0b0010000100000000,
                        0b0001000010000000,
                        0b0000100001000000,
                        0b0000010000100000,
                        0b0000010001000000,
                        0b0000001000100000,
                        0b0000001000100000,
                        0b0000010000100000,
                        0b0000100000100000,
                        0b0000011111100000,
                    ];
                    for row in 0..14 {
                        for col in 0..14 {
                            let mask = 0x8000u16 >> col;
                            let idx = row * 14 + col;
                            if NWSE_OUTLINE[row] & mask != 0 {
                                pixels[idx] = 0xFF000000;
                            } else if NWSE_BODY[row] & mask != 0 {
                                pixels[idx] = 0xFFFFFFFF;
                            }
                        }
                    }
                    g.define_cursor(14, 14, 7, 7, &pixels);
                }
                CursorShape::ResizeNESW => {
                    // ↙ diagonal NE-SW cursor (14x14, hotspot 7,7)
                    // Mirror of NWSE horizontally
                    let mut pixels = [0u32; 14 * 14];
                    static NESW_BODY: [u16; 14] = [
                        0b0000000000000000,
                        0b0000001111100000,
                        0b0000001111000000,
                        0b0000001110000000,
                        0b0000001110000000,
                        0b0000011100000000,
                        0b0000111000000000,
                        0b0001110000000000,
                        0b0001110000000000,
                        0b0011100000000000,
                        0b0011100000000000,
                        0b0011100000000000,
                        0b0011111000000000,
                        0b0000000000000000,
                    ];
                    static NESW_OUTLINE: [u16; 14] = [
                        0b0000011111100000,
                        0b0000010000010000,
                        0b0000010000100000,
                        0b0000010001000000,
                        0b0000100001000000,
                        0b0001000010000000,
                        0b0010000100000000,
                        0b0100001000000000,
                        0b0000101000000000,
                        0b0001000100000000,
                        0b0001000100000000,
                        0b0001000010000000,
                        0b0001000001000000,
                        0b0011111100000000,
                    ];
                    for row in 0..14 {
                        for col in 0..14 {
                            let mask = 0x8000u16 >> col;
                            let idx = row * 14 + col;
                            if NESW_OUTLINE[row] & mask != 0 {
                                pixels[idx] = 0xFF000000;
                            } else if NESW_BODY[row] & mask != 0 {
                                pixels[idx] = 0xFFFFFFFF;
                            }
                        }
                    }
                    g.define_cursor(14, 14, 7, 7, &pixels);
                }
            }
        });
    }

    /// Set GPU acceleration flag.
    pub fn set_gpu_accel(&mut self, enabled: bool) {
        self.gpu_accel = enabled;
    }

    /// Change display resolution via GPU driver.
    /// Reallocates the back buffer and updates framebuffer state.
    /// Returns true on success.
    pub fn change_resolution(&mut self, width: u32, height: u32) -> bool {
        let result = crate::drivers::gpu::with_gpu(|g| {
            g.set_mode(width, height, 32)
        });

        if let Some(Some((new_w, new_h, new_pitch, new_fb))) = result {
            self.width = new_w;
            self.height = new_h;
            self.framebuffer_pitch = new_pitch;
            self.back_buffer = Surface::new_with_color(new_w, new_h, Color::MACOS_BG);

            if self.hw_double_buffer {
                // Re-check if double buffering is still available at the new resolution
                // (may be lost if VRAM is too small for 2x height at larger modes)
                let still_dbl = crate::drivers::gpu::with_gpu(|g| g.has_double_buffer()).unwrap_or(false);
                if still_dbl {
                    if let Some(back) = crate::drivers::gpu::with_gpu(|g| g.back_buffer_phys()).flatten() {
                        self.framebuffer_addr = back;
                    } else {
                        self.framebuffer_addr = new_fb;
                        self.hw_double_buffer = false;
                    }
                } else {
                    self.framebuffer_addr = new_fb;
                    self.hw_double_buffer = false;
                    crate::serial_println!("  Compositor: double-buffering lost at {}x{}", new_w, new_h);
                }
            } else {
                self.framebuffer_addr = new_fb;
                // Check if double buffering became available (switching to smaller mode)
                let now_dbl = crate::drivers::gpu::with_gpu(|g| g.has_double_buffer()).unwrap_or(false);
                if now_dbl {
                    if let Some(back) = crate::drivers::gpu::with_gpu(|g| g.back_buffer_phys()).flatten() {
                        self.framebuffer_addr = back;
                        self.hw_double_buffer = true;
                        // Flush current content to the front page so display is correct
                        // while the back page gets composited in the next frame
                        let front_fb = new_fb;
                        let pitch_u32 = (new_pitch / 4) as usize;
                        let fb = front_fb as *mut u32;
                        for y in 0..new_h {
                            let src_off = (y * new_w) as usize;
                            let dst_off = y as usize * pitch_u32;
                            let row_len = new_w as usize;
                            let src = &self.back_buffer.pixels[src_off..src_off + row_len];
                            let dst = unsafe { core::slice::from_raw_parts_mut(fb.add(dst_off), row_len) };
                            dst.copy_from_slice(src);
                        }
                        crate::drivers::gpu::with_gpu(|g| g.flip());
                        // Update to new back page after flip
                        if let Some(new_back) = crate::drivers::gpu::with_gpu(|g| g.back_buffer_phys()).flatten() {
                            self.framebuffer_addr = new_back;
                        }
                        crate::serial_println!("  Compositor: double-buffering re-enabled at {}x{}", new_w, new_h);
                    }
                }
            }

            // Reset damage tracking
            self.damage.clear();
            self.prev_damage.clear();
            self.accel_move = None;
            self.damage.push(Rect::new(0, 0, new_w, new_h));

            // Clamp cursor to new screen bounds
            self.cursor_x = self.cursor_x.min(new_w as i32 - 1);
            self.cursor_y = self.cursor_y.min(new_h as i32 - 1);

            true
        } else {
            false
        }
    }

    /// Draw a simple arrow cursor
    fn draw_cursor(&mut self) {
        let cx = self.cursor_x;
        let cy = self.cursor_y;

        // Simple arrow cursor (12x18 pixels)
        static CURSOR: [u16; 18] = [
            0b1000000000000000,
            0b1100000000000000,
            0b1110000000000000,
            0b1111000000000000,
            0b1111100000000000,
            0b1111110000000000,
            0b1111111000000000,
            0b1111111100000000,
            0b1111111110000000,
            0b1111111111000000,
            0b1111111111100000,
            0b1111110000000000,
            0b1110011000000000,
            0b1100011000000000,
            0b1000001100000000,
            0b0000001100000000,
            0b0000000110000000,
            0b0000000000000000,
        ];

        // Draw cursor shadow (offset by 1)
        for (row, &bits) in CURSOR.iter().enumerate() {
            for col in 0..12 {
                if bits & (0x8000 >> col) != 0 {
                    self.back_buffer.put_pixel(
                        cx + col + 1,
                        cy + row as i32 + 1,
                        Color::with_alpha(128, 0, 0, 0),
                    );
                }
            }
        }

        // Draw cursor body (white with black outline)
        static CURSOR_OUTLINE: [u16; 18] = [
            0b1100000000000000,
            0b1010000000000000,
            0b1001000000000000,
            0b1000100000000000,
            0b1000010000000000,
            0b1000001000000000,
            0b1000000100000000,
            0b1000000010000000,
            0b1000000001000000,
            0b1000000000100000,
            0b1000000000010000,
            0b1000001110000000,
            0b1001000100000000,
            0b1010000100000000,
            0b1100000010000000,
            0b0000000010000000,
            0b0000000001100000,
            0b0000000000000000,
        ];

        for (row, &bits) in CURSOR.iter().enumerate() {
            let outline_bits = CURSOR_OUTLINE[row];
            for col in 0..12 {
                let mask = 0x8000u16 >> col;
                if outline_bits & mask != 0 {
                    self.back_buffer.put_pixel(cx + col, cy + row as i32, Color::BLACK);
                } else if bits & mask != 0 {
                    self.back_buffer.put_pixel(cx + col, cy + row as i32, Color::WHITE);
                }
            }
        }
    }

    /// Check if a screen region contains only background (no visible layers or overlays).
    /// Used by compose() to decide whether to GPU-fill instead of memcpy from back buffer.
    fn is_background_only(&self, rect: &Rect) -> bool {
        for layer in &self.layers {
            if layer.visible && layer.bounds().intersects(rect) {
                return false;
            }
        }
        if let Some(outline) = self.resize_outline {
            if outline.intersects(rect) {
                return false;
            }
        }
        if self.cursor_visible && !self.hw_cursor_active {
            let cursor_rect = Rect::new(self.cursor_x - 1, self.cursor_y - 1, 16, 20);
            if cursor_rect.intersects(rect) {
                return false;
            }
        }
        true
    }

    /// Flush a region from the back buffer to the physical framebuffer.
    /// Uses u32 row copies — on little-endian x86, our ARGB u32 layout
    /// naturally produces BGRA byte order which is what VBE expects.
    fn flush_region(&self, region: Rect) {
        let fb = self.framebuffer_addr as *mut u32;
        let pitch_u32 = (self.framebuffer_pitch / 4) as usize;

        let x0 = region.x.max(0) as u32;
        let y0 = region.y.max(0) as u32;
        let x1 = (region.right() as u32).min(self.width);
        let y1 = (region.bottom() as u32).min(self.height);

        if x0 >= x1 || y0 >= y1 {
            return;
        }

        let row_len = (x1 - x0) as usize;

        for y in y0..y1 {
            let src_off = (y * self.width + x0) as usize;
            let dst_off = y as usize * pitch_u32 + x0 as usize;
            let src = &self.back_buffer.pixels[src_off..src_off + row_len];
            let dst = unsafe { core::slice::from_raw_parts_mut(fb.add(dst_off), row_len) };
            dst.copy_from_slice(src);
        }
    }

    /// Flush the entire back buffer to the framebuffer
    pub fn flush_all(&self) {
        self.flush_region(Rect::new(0, 0, self.width, self.height));
    }
}
