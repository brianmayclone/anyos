//! Compositing — layer blending, shadow rendering, and damage-based recomposition.

use alloc::vec::Vec;

use super::Compositor;
use super::rect::Rect;
use super::layer::{AccelMoveHint, SHADOW_OFFSET_X, SHADOW_OFFSET_Y, SHADOW_SPREAD, SHADOW_ALPHA_FOCUSED, SHADOW_ALPHA_UNFOCUSED};
use super::blend::{alpha_blend, compute_shadow_cache, blur_back_buffer_region};
use super::gpu::{GPU_UPDATE, GPU_FLIP, GPU_RECT_COPY, GPU_SYNC};

impl Compositor {
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

    /// Main compositing function. Composites all dirty regions.
    pub fn compose(&mut self) {
        self.collect_dirty_damage();

        // Check for GPU-accelerated RECT_COPY path (window drag optimization)
        let hint = self.accel_move_hint.take();

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

        // Try GPU RECT_COPY fast path for window drags (requires gpu_accel + valid hint)
        if self.gpu_accel && !self.hw_double_buffer {
            if let Some(ref h) = hint {
                if let Some(moved_idx) = self.layer_index(h.layer_id) {
                    if self.layers[moved_idx].opaque {
                        self.compose_with_rect_copy(&damage, h);
                        return;
                    }
                }
            }
        }

        // Standard SW compositing path (skips VRAM-direct layers)
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

            // VRAM-direct layer overlay: after CPU flush, GPU RECT_COPY from off-screen VRAM.
            // This must happen AFTER flush_region so the background is in place,
            // and BEFORE flush_gpu so RECT_COPY + UPDATE are batched together.
            if self.gpu_accel {
                self.overlay_vram_layers(&damage);
            }
        }

        self.flush_gpu();
    }

    /// GPU-accelerated compositing for window drag (RECT_COPY fast path).
    ///
    /// Key insight: RECT_COPY must execute BEFORE any CPU flush_region writes,
    /// because RECT_COPY reads from the old window position in VRAM. If we
    /// flush_region first, the exposed strips overwrite parts of the source.
    ///
    /// Sequence:
    ///   1. SW composite into back buffer (for future frames)
    ///   2. GPU RECT_COPY old→new position + SYNC (GPU reads pristine VRAM)
    ///   3. CPU flush exposed strips + above-layer fixup to VRAM
    ///   4. GPU UPDATE for all affected regions
    fn compose_with_rect_copy(&mut self, _damage: &[Rect], hint: &AccelMoveHint) {
        let old_b = hint.old_bounds.clip_to_screen(self.fb_width, self.fb_height);
        let new_b = hint.new_bounds.clip_to_screen(self.fb_width, self.fb_height);

        if old_b.is_empty() || new_b.is_empty() {
            return;
        }

        // Step 1: Compute exposed strips (old position minus new position overlap)
        let exposed = super::layer::subtract_rects(&old_b, &new_b);

        // Step 2: SW-composite into back buffer (keeps it in sync for future frames)
        for rect in &exposed {
            if !rect.is_empty() {
                self.composite_rect(rect);
            }
        }
        self.composite_rect(&new_b);

        if let Some(outline) = self.resize_outline {
            self.draw_outline_to_bb(&outline);
        }

        // Step 3: GPU RECT_COPY + SYNC — move window in VRAM BEFORE any CPU writes.
        // RECT_COPY reads from old window position which is still intact in VRAM.
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
        self.gpu_cmds.push([GPU_SYNC, 0, 0, 0, 0, 0, 0, 0, 0]);
        self.flush_gpu(); // GPU executes RECT_COPY, blocks on SYNC until done

        // Step 4: Now safe to CPU-flush exposed strips from back buffer to VRAM
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

        // Step 5: Above-layer fixup — flush intersections of above-layers with the
        // destination from the back buffer (RECT_COPY may have overwritten them with
        // stale dock/menubar pixels from the old position).
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

        // Step 6: UPDATE the destination region
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

    /// Overlay VRAM-direct layers onto the visible framebuffer using GPU RECT_COPY.
    /// Called after the CPU back buffer has been flushed to VRAM.
    /// For each visible VRAM layer that intersects a damage rect, issues a RECT_COPY
    /// from off-screen VRAM (where the app rendered) to the visible region.
    fn overlay_vram_layers(&mut self, damage: &[Rect]) {
        let screen = Rect::new(0, 0, self.fb_width, self.fb_height);

        for li in 0..self.layers.len() {
            if !self.layers[li].visible || !self.layers[li].is_vram {
                continue;
            }
            let layer_rect = self.layers[li].bounds();
            let vram_y = self.layers[li].vram_y;
            let lx = self.layers[li].x;
            let ly = self.layers[li].y;

            for dmg in damage {
                if let Some(overlap) = dmg.intersect(&layer_rect) {
                    let overlap = match overlap.intersect(&screen) {
                        Some(o) => o,
                        None => continue,
                    };
                    // Source coordinates in off-screen VRAM surface:
                    // The surface is at vram_y rows, with stride = pitch_pixels.
                    // Source pixel at (sx, sy) relative to layer origin.
                    let sx = (overlap.x - lx) as u32;
                    let sy = (overlap.y - ly) as u32;
                    let src_x = sx;
                    let src_y = vram_y + sy;

                    self.gpu_cmds.push([
                        GPU_RECT_COPY,
                        src_x,          // source X (within pitch-aligned row)
                        src_y,          // source Y (off-screen VRAM row)
                        overlap.x as u32, // dest X (screen)
                        overlap.y as u32, // dest Y (screen)
                        overlap.width,
                        overlap.height,
                        0, 0,
                    ]);
                    self.gpu_cmds.push([
                        GPU_UPDATE,
                        overlap.x as u32,
                        overlap.y as u32,
                        overlap.width,
                        overlap.height,
                        0, 0, 0, 0,
                    ]);
                }
            }
        }
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

            // Skip VRAM-direct layers — their pixels are in off-screen VRAM,
            // not accessible from CPU. They're overlaid via GPU RECT_COPY later.
            if self.layers[li].is_vram {
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
                        blur_radius, 2, // 2 passes = triangle blur (fast + decent quality)
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
}
