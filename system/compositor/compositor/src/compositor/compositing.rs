//! Compositing — layer blending, shadow rendering, and damage-based recomposition.
//!
//! Performance-critical hot path. Key optimizations:
//!   - div255() bit trick replaces all `/ 255` divisions (~10x faster per blend)
//!   - shadow_blend() specialized for R=G=B=0 (halves multiplies)
//!   - Blur uses fixed-point reciprocal instead of `/ kernel`
//!   - Reusable scratch buffers (no per-frame heap allocations)
//!   - Transparent-run scanning in alpha-blend path
//!   - fill() for background clear (LLVM vectorizes to rep stosd)

use super::Compositor;
use super::rect::Rect;
use super::layer::{AccelMoveHint, SHADOW_OFFSET_X, SHADOW_OFFSET_Y, SHADOW_SPREAD};
use super::blend::{alpha_blend, shadow_blend, compute_shadow_cache, blur_back_buffer_region};
use super::gpu::{GPU_UPDATE, GPU_FLIP, GPU_RECT_COPY, GPU_SYNC};

impl Compositor {
    /// Collect damage from all dirty layers.
    /// Any dirty layer (visible or invisible) gets its bounds added as damage.
    /// This ensures that resized, moved, or content-updated layers always
    /// trigger recomposition of their region.
    fn collect_dirty_damage(&mut self) {
        for i in 0..self.layers.len() {
            if self.layers[i].dirty {
                self.damage.push(self.layers[i].damage_bounds());
                self.layers[i].dirty = false;
            }
        }
    }

    /// Merge damage rects if there are too many (prevents performance explosion).
    fn merge_damage_if_needed(&mut self) {
        if self.damage.len() > 128 {
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
    /// Returns `true` if any damage was processed (screen content changed).
    pub fn compose(&mut self) -> bool {
        self.collect_dirty_damage();

        // Check for GPU-accelerated RECT_COPY path (window drag optimization)
        let hint = self.accel_move_hint.take();

        if self.damage.is_empty() {
            return false;
        }

        self.merge_damage_if_needed();

        // Clip all damage to screen bounds in-place, remove empty rects
        let fb_w = self.fb_width;
        let fb_h = self.fb_height;
        for r in &mut self.damage {
            *r = r.clip_to_screen(fb_w, fb_h);
        }
        self.damage.retain(|r| !r.is_empty());

        if self.damage.is_empty() {
            return false;
        }

        // Swap damage into compositing_damage (avoids drain+collect heap allocation).
        // self.damage keeps its capacity for next frame's pushes.
        core::mem::swap(&mut self.damage, &mut self.compositing_damage);

        // Try GPU RECT_COPY fast path for window drags (requires gpu_accel + valid hint).
        // Works for both opaque and non-opaque layers (decorated windows with rounded corners).
        // For non-opaque layers, corner strips are flushed from back buffer after RECT_COPY.
        // Disabled in GMR mode: RECT_COPY operates on the back buffer (registered as GPU
        // framebuffer), which corrupts freshly composited content. Since flush_region is
        // already a no-op in GMR mode, there's no VRAM memcpy cost to optimize away.
        if self.gpu_accel && !self.hw_double_buffer && !self.gmr_active {
            if let Some(ref h) = hint {
                if let Some(moved_idx) = self.layer_index(h.layer_id) {
                    let layer = &self.layers[moved_idx];
                    if layer.opaque || (layer.width > 16 && layer.height > 16) {
                        self.compose_with_rect_copy(h);
                        return true;
                    }
                }
            }
        }

        // Standard SW compositing path
        let damage_len = self.compositing_damage.len();
        for i in 0..damage_len {
            let rect = self.compositing_damage[i];
            self.composite_rect(&rect);
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
            let prev_len = self.prev_damage.len();
            for i in 0..prev_len {
                let rect = self.prev_damage[i];
                self.flush_region(&rect, back_offset);
            }
            let damage_len = self.compositing_damage.len();
            for i in 0..damage_len {
                let rect = self.compositing_damage[i];
                self.flush_region(&rect, back_offset);
            }
            self.gpu_cmds.push([GPU_FLIP, 0, 0, 0, 0, 0, 0, 0, 0]);
            self.current_page = 1 - self.current_page;
            // Move compositing_damage into prev_damage (swap to reuse allocation)
            core::mem::swap(&mut self.compositing_damage, &mut self.prev_damage);
            self.compositing_damage.clear();
        } else {
            let damage_len = self.compositing_damage.len();
            for i in 0..damage_len {
                let r = self.compositing_damage[i];
                self.flush_region(&r, 0);
                self.gpu_cmds
                    .push([GPU_UPDATE, r.x as u32, r.y as u32, r.width, r.height, 0, 0, 0, 0]);
            }
            self.compositing_damage.clear();
        }

        self.flush_gpu();
        true
    }

    /// GPU-accelerated compositing for window drag (RECT_COPY fast path).
    fn compose_with_rect_copy(&mut self, hint: &AccelMoveHint) {
        let old_b = hint.old_bounds.clip_to_screen(self.fb_width, self.fb_height);
        let new_b = hint.new_bounds.clip_to_screen(self.fb_width, self.fb_height);

        if old_b.is_empty() || new_b.is_empty() {
            self.compositing_damage.clear();
            return;
        }

        let exposed = super::layer::subtract_rects(&old_b, &new_b);

        for rect in &exposed {
            if !rect.is_empty() {
                self.composite_rect(rect);
            }
        }
        self.composite_rect(&new_b);

        if let Some(outline) = self.resize_outline {
            self.draw_outline_to_bb(&outline);
        }

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
        self.flush_gpu();

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

        // Check if any layer above the moved window overlapped the OLD position.
        // RECT_COPY copies raw VRAM from old_b→new_b, which includes alpha-blended
        // pixels from layers above (e.g. a semi-transparent Dock). These artifacts
        // appear at wrong positions in new_b. If detected, flush entire new_b from
        // back_buffer (which has the correct composited result from step above).
        let mut need_full_flush = old_b.width != new_b.width || old_b.height != new_b.height;

        if let Some(moved_idx) = self.layer_index(hint.layer_id) {
            if !need_full_flush {
                for li in (moved_idx + 1)..self.layers.len() {
                    if !self.layers[li].visible { continue; }
                    let above_bounds = self.layers[li].damage_bounds();
                    if old_b.intersect(&above_bounds).is_some()
                        || new_b.intersect(&above_bounds).is_some()
                    {
                        need_full_flush = true;
                        break;
                    }
                }
            }

            if !need_full_flush {
                // No above layers overlap — only fix non-opaque corners
                if !self.layers[moved_idx].opaque {
                    const CORNER_R: u32 = 8;
                    let top_strip = Rect::new(new_b.x, new_b.y, new_b.width, CORNER_R);
                    self.flush_region(&top_strip, 0);
                    self.gpu_cmds.push([GPU_UPDATE, top_strip.x as u32, top_strip.y as u32,
                        top_strip.width, top_strip.height, 0, 0, 0, 0]);
                    let bot_strip = Rect::new(new_b.x, new_b.bottom() - CORNER_R as i32, new_b.width, CORNER_R);
                    self.flush_region(&bot_strip, 0);
                    self.gpu_cmds.push([GPU_UPDATE, bot_strip.x as u32, bot_strip.y as u32,
                        bot_strip.width, bot_strip.height, 0, 0, 0, 0]);
                }
            }
        }

        if need_full_flush {
            // Flush entire new_b from back_buffer to overwrite RECT_COPY artifacts
            self.flush_region(&new_b, 0);
        }

        self.gpu_cmds.push([
            GPU_UPDATE,
            new_b.x as u32,
            new_b.y as u32,
            new_b.width,
            new_b.height,
            0, 0, 0, 0,
        ]);

        self.compositing_damage.clear();
        self.flush_gpu();
    }

    /// Composite all layers within a damage rect into the back buffer.
    fn composite_rect(&mut self, rect: &Rect) {
        let bb_stride = self.fb_width as usize;
        let rx = rect.x as usize;
        let ry = rect.y as usize;
        let rw = rect.width as usize;
        let rh = rect.height as usize;

        // ── Occlusion culling ──
        // Find topmost layer that fully covers this damage rect with opaque pixels.
        // For non-opaque layers (rounded corners): inner rect shrunk by corner radius
        // is fully opaque — if it covers the damage rect, skip everything below.
        let mut base_layer_idx = 0usize;
        let mut skip_bg_clear = false;
        const CORNER_RADIUS: i32 = 8;

        for li in (0..self.layers.len()).rev() {
            if !self.layers[li].visible { continue; }
            let bounds = self.layers[li].bounds();
            if self.layers[li].opaque {
                if bounds.fully_contains(rect) {
                    base_layer_idx = li;
                    skip_bg_clear = true;
                    break;
                }
            } else {
                let inner = bounds.shrink(CORNER_RADIUS);
                if !inner.is_empty() && inner.fully_contains(rect) {
                    base_layer_idx = li;
                    skip_bg_clear = true;
                    break;
                }
            }
        }

        // Background fill — uses fill() which LLVM compiles to rep stosd (vectorized)
        if !skip_bg_clear {
            for row in 0..rh {
                let y = ry + row;
                if y >= self.fb_height as usize {
                    break;
                }
                let off = y * bb_stride + rx;
                let end = (off + rw).min(self.back_buffer.len());
                self.back_buffer[off..end].fill(0xFF1E1E1E);
            }
        }

        // Composite layers from base upward (skip everything below)
        let pitch_stride = (self.fb_pitch / 4) as usize;

        for li in base_layer_idx..self.layers.len() {
            if !self.layers[li].visible {
                continue;
            }

            // Early intersection test: skip layers that don't overlap this damage rect
            let layer_damage = self.layers[li].damage_bounds();
            if rect.intersect(&layer_damage).is_none() {
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
                    // Split borrow: take blur_temp out to avoid &mut self conflict
                    let mut blur_temp = core::mem::take(&mut self.blur_temp);
                    blur_back_buffer_region(
                        &mut self.back_buffer, self.fb_width, self.fb_height,
                        blur_area.x, blur_area.y, blur_area.width, blur_area.height,
                        blur_radius, 2,
                        &mut blur_temp,
                    );
                    self.blur_temp = blur_temp;
                }
            }

            let layer_rect = self.layers[li].bounds();
            let layer_x = self.layers[li].x;
            let layer_y = self.layers[li].y;
            let layer_opaque = self.layers[li].opaque;
            let is_vram = self.layers[li].is_vram;

            let (pixels_ptr, lp_len, lw): (*const u32, usize, usize) = if is_vram {
                let vram_y = self.layers[li].vram_y as usize;
                let ptr = unsafe { self.fb_ptr.add(vram_y * pitch_stride) as *const u32 };
                let len = self.layers[li].height as usize * pitch_stride;
                (ptr, len, pitch_stride)
            } else {
                let ps = self.layers[li].pixel_slice();
                (ps.as_ptr(), ps.len(), self.layers[li].width as usize)
            };

            if let Some(overlap) = rect.intersect(&layer_rect) {
                let sx = (overlap.x - layer_x) as usize;
                let sy = (overlap.y - layer_y) as usize;

                let layer_pixels = unsafe { core::slice::from_raw_parts(pixels_ptr, lp_len) };

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
                    // Alpha-blend path with opaque-run + transparent-run scanning.
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
                                // Fully transparent — scan ahead for transparent run
                                col += 1;
                                while col < row_width {
                                    let si2 = src_off + col;
                                    if si2 >= lp_len { break; }
                                    if layer_pixels[si2] >> 24 != 0 { break; }
                                    col += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Draw a soft gradient shadow for a layer into the back buffer (within damage rect).
    /// Uses pre-baked alpha arrays (focused/unfocused) to skip per-pixel div255 multiply.
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

        // Pick focused or unfocused pre-baked alpha array
        let is_focused = self.focused_layer_id == Some(layer_id);

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

            // Split borrow: read all layer data first, then take mutable ref to back_buffer
            let cache = self.layers[layer_idx].shadow_cache.as_ref().unwrap();
            let cache_w = cache.cache_w as usize;
            let alphas = if is_focused { &cache.focused_alphas } else { &cache.unfocused_alphas };
            let cache_alphas = alphas.as_ptr();
            let cache_len = alphas.len();

            // Interior skip: use the ACTUAL window rect (not the shadow's offset position).
            // With SHADOW_OFFSET_Y=6, using the shadow offset would incorrectly skip
            // the 6px strip below the window where shadow should be visible.
            let win_abs_x0 = self.layers[layer_idx].x;
            let win_abs_x1 = self.layers[layer_idx].x + layer_w as i32;
            let win_abs_y0 = self.layers[layer_idx].y;
            let win_abs_y1 = self.layers[layer_idx].y + layer_h as i32;

            let bb = &mut self.back_buffer;
            let bb_len = bb.len();

            for row in 0..overlap.height as usize {
                let py = overlap.y + row as i32;
                let cy = (py - shadow_oy) as usize;
                let cache_row_off = cy * cache_w;
                let bb_row_off = py as usize * bb_stride;

                let ol_x0 = overlap.x;
                let ol_x1 = overlap.x + overlap.width as i32;
                let in_window_y = py >= win_abs_y0 && py < win_abs_y1;

                if in_window_y {
                    let left_end = win_abs_x0.min(ol_x1);
                    if ol_x0 < left_end {
                        Self::shadow_span(
                            bb, bb_len, bb_row_off,
                            cache_alphas, cache_len, cache_row_off,
                            shadow_ox, ol_x0, left_end,
                        );
                    }
                    let right_start = win_abs_x1.max(ol_x0);
                    if right_start < ol_x1 {
                        Self::shadow_span(
                            bb, bb_len, bb_row_off,
                            cache_alphas, cache_len, cache_row_off,
                            shadow_ox, right_start, ol_x1,
                        );
                    }
                } else {
                    Self::shadow_span(
                        bb, bb_len, bb_row_off,
                        cache_alphas, cache_len, cache_row_off,
                        shadow_ox, ol_x0, ol_x1,
                    );
                }
            }
        }
    }

    /// Process a horizontal span of shadow pixels with pre-baked alpha (no per-pixel div255 multiply).
    #[inline(always)]
    fn shadow_span(
        bb: &mut [u32], bb_len: usize, bb_row_off: usize,
        cache_alphas: *const u8, cache_len: usize, cache_row_off: usize,
        shadow_ox: i32, x_start: i32, x_end: i32,
    ) {
        for px in x_start..x_end {
            let cx = (px - shadow_ox) as usize;
            let cache_idx = cache_row_off + cx;
            if cache_idx >= cache_len { break; }
            let a = unsafe { *cache_alphas.add(cache_idx) } as u32;
            if a == 0 { continue; }
            let di = bb_row_off + px as usize;
            if di < bb_len {
                bb[di] = shadow_blend(a, bb[di]);
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
