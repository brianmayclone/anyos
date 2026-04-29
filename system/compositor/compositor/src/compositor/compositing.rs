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
use super::layer::{AccelMoveHint, BlurCache, SHADOW_OFFSET_X, shadow_offset_y, shadow_spread};
use super::blend::{alpha_blend, shadow_blend, compute_shadow_cache, blur_back_buffer_region};
use super::gpu::{GPU_UPDATE, GPU_FLIP, GPU_RECT_COPY, GPU_SYNC};

impl Compositor {
    /// Collect damage from all dirty layers.
    /// Any dirty layer (visible or invisible) gets its bounds added as damage.
    /// This ensures that resized, moved, or content-updated layers always
    /// trigger recomposition of their region.
    fn collect_dirty_damage(&mut self) {
        let mut behind_blur_changed = false;
        for i in 0..self.layers.len() {
            if self.layers[i].dirty {
                self.damage.push(self.layers[i].damage_bounds());
                if !self.layers[i].blur_behind {
                    behind_blur_changed = true;
                }
                self.layers[i].dirty = false;
            }
        }
        if behind_blur_changed {
            self.blur_generation = self.blur_generation.wrapping_add(1);
        }
    }

    /// Merge damage rects if there are too many (prevents performance explosion).
    /// Uses a smarter two-pass strategy: first coalesce overlapping/adjacent rects,
    /// then fall back to full merge only if count still exceeds threshold.
    fn merge_damage_if_needed(&mut self) {
        if self.damage.len() <= 512 {
            return;
        }
        // Pass 1: Coalesce overlapping and nearby rects (within 32px gap).
        // Sort by Y then X for better spatial locality.
        self.damage.sort_unstable_by(|a, b| {
            a.y.cmp(&b.y).then(a.x.cmp(&b.x))
        });
        let mut merged = alloc::vec::Vec::with_capacity(self.damage.len() / 2);
        if let Some(&first) = self.damage.first() {
            merged.push(first);
        }
        for i in 1..self.damage.len() {
            let r = self.damage[i];
            let last = merged.last_mut().unwrap();
            // Merge if rects overlap or are within 32px of each other
            if r.x <= last.x + last.width as i32 + 32
                && r.y <= last.y + last.height as i32 + 32
                && r.x + r.width as i32 >= last.x - 32
                && r.y >= last.y - 32
            {
                *last = last.union(&r);
            } else {
                merged.push(r);
            }
        }
        self.damage = merged;

        // Pass 2: If still too many, fall back to full union (rare)
        if self.damage.len() > 512 {
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

        // NOTE: The GPU RECT_COPY drag fast path is intentionally disabled for now.
        //
        // On VirtIO GPU this path has proven unstable under rapid window movement:
        // the compositor emits a tight sequence of RECT_COPY/SYNC/UPDATE/FLUSH
        // commands, and the kernel-side GPU path has been observed to crash in
        // SYS_GPU_COMMAND during or immediately after fast drags.
        //
        // Until the lower-level VirtIO GPU instability is fully resolved, prefer
        // the standard software compositing path for correctness and stability.
        let _ = hint;

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

        // Issue sfence for any VRAM writes done above.  The actual GPU command
        // submission (ipc::gpu_command) is intentionally deferred: callers that
        // hold the compositor lock must drain gpu_cmds and submit outside the
        // lock (via Desktop::compose_deferred + Compositor::submit_cmds) so that
        // a potentially-blocking kernel call never stalls the management thread.
        // For callers that do not use the deferred pattern, Desktop::compose()
        // calls flush_gpu() after this returns, which submits all queued commands.
        self.prepare_flush();
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

        // Composite exposed regions (old position not covered by new) and the
        // new position into the back buffer.
        let exposed = super::layer::subtract_rects(&old_b, &new_b);

        for rect in &exposed {
            if !rect.is_empty() {
                self.composite_rect(rect);
            }
        }
        self.composite_rect(&new_b);

        // Also process any OTHER damage rects (e.g. cursor, dock, content updates)
        // that are not covered by old_b/new_b. Without this, content changes from
        // other layers during a drag frame would be silently dropped.
        let damage_len = self.compositing_damage.len();
        for i in 0..damage_len {
            let r = self.compositing_damage[i];
            if r.is_empty() { continue; }
            // Skip rects already covered by old_b or new_b
            if old_b.fully_contains(&r) || new_b.fully_contains(&r) { continue; }
            self.composite_rect(&r);
        }

        if let Some(outline) = self.resize_outline {
            self.draw_outline_to_bb(&outline);
        }

        // GPU RECT_COPY: fast VRAM blit from old position to new.
        // This is a latency optimization — the CPU flush below guarantees
        // correctness regardless of whether the GPU copy succeeds.
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

        // Flush exposed regions from back buffer → VRAM (clears old window position).
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

        // Always flush new_b from back buffer to guarantee correctness.
        // The back buffer has the correctly composited result; this overwrites
        // the GPU RECT_COPY output with verified pixels, preventing artifacts
        // from RECT_COPY edge cases (above-layer overlap, non-opaque corners,
        // clipped boundaries, or GPU emulation quirks).
        self.flush_region(&new_b, 0);

        self.gpu_cmds.push([
            GPU_UPDATE,
            new_b.x as u32,
            new_b.y as u32,
            new_b.width,
            new_b.height,
            0, 0, 0, 0,
        ]);

        // Flush any extra damage rects not covered by old_b/new_b
        for i in 0..damage_len {
            let r = self.compositing_damage[i];
            if r.is_empty() { continue; }
            if old_b.fully_contains(&r) || new_b.fully_contains(&r) { continue; }
            self.flush_region(&r, 0);
            self.gpu_cmds.push([
                GPU_UPDATE, r.x as u32, r.y as u32, r.width, r.height, 0, 0, 0, 0,
            ]);
        }

        self.compositing_damage.clear();
        // Same deferred-flush pattern as compose(): sfence now, submit later outside lock.
        self.prepare_flush();
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
            } else if self.layers[li].has_shadow {
                // Inner-rect optimization: only for decorated windows (has_shadow)
                // where only the rounded corners are transparent and the interior
                // is fully opaque. NOT safe for layers with arbitrary transparency
                // (e.g. the dock, tooltips, overlays) which would skip compositing
                // of layers below, revealing stale back buffer content.
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
                    self.apply_blur_behind(rect, li, lb, blur_area, blur_radius);
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

    fn apply_blur_behind(
        &mut self,
        damage: &Rect,
        layer_idx: usize,
        layer_bounds: Rect,
        blur_area: Rect,
        radius: u32,
    ) {
        if self.copy_blur_cache_to_bb(layer_idx, blur_area) {
            return;
        }

        let layer_clip = layer_bounds.clip_to_screen(self.fb_width, self.fb_height);
        if !layer_clip.is_empty() && damage.fully_contains(&layer_clip) {
            self.rebuild_blur_cache(layer_idx, layer_clip, radius);
            if self.copy_blur_cache_to_bb(layer_idx, blur_area) {
                return;
            }
        }

        // Fallback for partial first paints or changing below-content.
        let mut blur_temp = core::mem::take(&mut self.blur_temp);
        blur_back_buffer_region(
            &mut self.back_buffer, self.fb_width, self.fb_height,
            blur_area.x, blur_area.y, blur_area.width, blur_area.height,
            radius, 2,
            &mut blur_temp,
        );
        self.blur_temp = blur_temp;
    }

    fn copy_blur_cache_to_bb(&mut self, layer_idx: usize, area: Rect) -> bool {
        let generation = self.blur_generation;
        let Some(cache) = self.layers[layer_idx].blur_cache.as_ref() else {
            return false;
        };
        if cache.generation != generation || cache.radius == 0 {
            return false;
        }
        let cache_rect = Rect::new(cache.x, cache.y, cache.width, cache.height);
        let Some(overlap) = area.intersect(&cache_rect) else {
            return false;
        };

        let bb_stride = self.fb_width as usize;
        let cache_stride = cache.width as usize;
        for row in 0..overlap.height as usize {
            let src_y = (overlap.y - cache.y) as usize + row;
            let src_x = (overlap.x - cache.x) as usize;
            let dst_y = overlap.y as usize + row;
            let dst_x = overlap.x as usize;
            let w = overlap.width as usize;
            let src = src_y * cache_stride + src_x;
            let dst = dst_y * bb_stride + dst_x;
            let safe = w
                .min(cache.pixels.len().saturating_sub(src))
                .min(self.back_buffer.len().saturating_sub(dst));
            if safe > 0 {
                self.back_buffer[dst..dst + safe].copy_from_slice(&cache.pixels[src..src + safe]);
            }
        }
        true
    }

    fn rebuild_blur_cache(&mut self, layer_idx: usize, area: Rect, radius: u32) {
        let w = area.width as usize;
        let h = area.height as usize;
        if w == 0 || h == 0 {
            return;
        }

        if radius >= 8 && w.saturating_mul(h) >= 240_000 {
            self.rebuild_blur_cache_downsampled(layer_idx, area, radius);
            return;
        }

        let mut pixels = alloc::vec![0u32; w * h];
        let bb_stride = self.fb_width as usize;
        for row in 0..h {
            let src = (area.y as usize + row) * bb_stride + area.x as usize;
            let dst = row * w;
            let safe = w
                .min(self.back_buffer.len().saturating_sub(src))
                .min(pixels.len().saturating_sub(dst));
            if safe > 0 {
                pixels[dst..dst + safe].copy_from_slice(&self.back_buffer[src..src + safe]);
            }
        }

        let mut blur_temp = core::mem::take(&mut self.blur_temp);
        blur_back_buffer_region(
            &mut pixels, area.width, area.height,
            0, 0, area.width, area.height,
            radius, 2,
            &mut blur_temp,
        );
        self.blur_temp = blur_temp;

        self.layers[layer_idx].blur_cache = Some(BlurCache {
            pixels,
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
            radius,
            generation: self.blur_generation,
        });
    }

    fn rebuild_blur_cache_downsampled(&mut self, layer_idx: usize, area: Rect, radius: u32) {
        let w = area.width as usize;
        let h = area.height as usize;
        let dw = (w + 1) / 2;
        let dh = (h + 1) / 2;
        if dw == 0 || dh == 0 {
            return;
        }

        let mut small = alloc::vec![0u32; dw * dh];
        let bb_stride = self.fb_width as usize;
        for sy in 0..dh {
            for sx in 0..dw {
                let src_x = area.x as usize + sx * 2;
                let src_y = area.y as usize + sy * 2;
                let mut a = 0u32;
                let mut r = 0u32;
                let mut g = 0u32;
                let mut b = 0u32;
                let mut n = 0u32;
                for oy in 0..2 {
                    let y = src_y + oy;
                    if y >= area.y as usize + h || y >= self.fb_height as usize {
                        continue;
                    }
                    for ox in 0..2 {
                        let x = src_x + ox;
                        if x >= area.x as usize + w || x >= self.fb_width as usize {
                            continue;
                        }
                        let idx = y * bb_stride + x;
                        if idx >= self.back_buffer.len() {
                            continue;
                        }
                        let px = self.back_buffer[idx];
                        a += (px >> 24) & 0xFF;
                        r += (px >> 16) & 0xFF;
                        g += (px >> 8) & 0xFF;
                        b += px & 0xFF;
                        n += 1;
                    }
                }
                if n > 0 {
                    small[sy * dw + sx] =
                        ((a / n) << 24) | ((r / n) << 16) | ((g / n) << 8) | (b / n);
                }
            }
        }

        let mut blur_temp = core::mem::take(&mut self.blur_temp);
        blur_back_buffer_region(
            &mut small, dw as u32, dh as u32,
            0, 0, dw as u32, dh as u32,
            (radius + 1) / 2, 2,
            &mut blur_temp,
        );
        self.blur_temp = blur_temp;

        let mut pixels = alloc::vec![0u32; w * h];
        for y in 0..h {
            let sy = (y / 2).min(dh - 1);
            let sy1 = (sy + 1).min(dh - 1);
            let fy = (y & 1) as u32;
            for x in 0..w {
                let sx = (x / 2).min(dw - 1);
                let sx1 = (sx + 1).min(dw - 1);
                let fx = (x & 1) as u32;
                let c00 = small[sy * dw + sx];
                let c10 = small[sy * dw + sx1];
                let c01 = small[sy1 * dw + sx];
                let c11 = small[sy1 * dw + sx1];
                let top = mix_px(c00, c10, fx);
                let bot = mix_px(c01, c11, fx);
                pixels[y * w + x] = mix_px(top, bot, fy);
            }
        }

        self.layers[layer_idx].blur_cache = Some(BlurCache {
            pixels,
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
            radius,
            generation: self.blur_generation,
        });
    }

    /// Draw a soft gradient shadow for a layer into the back buffer (within damage rect).
    /// Uses pre-baked alpha arrays (focused/unfocused) to skip per-pixel div255 multiply.
    fn draw_shadow_to_bb(&mut self, rect: &Rect, layer_idx: usize) {
        let layer_id = self.layers[layer_idx].id;
        let layer_w = self.layers[layer_idx].width;
        let layer_h = self.layers[layer_idx].height;
        let lx = self.layers[layer_idx].x + SHADOW_OFFSET_X;
        let ly = self.layers[layer_idx].y + shadow_offset_y();
        let spread = shadow_spread();

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
            let Some(cache) = self.layers[layer_idx].shadow_cache.as_ref() else {
                return;
            };
            let cache_w = cache.cache_w as usize;
            let alphas = if is_focused { &cache.focused_alphas } else { &cache.unfocused_alphas };
            let cache_alphas = alphas.as_ptr();
            let cache_len = alphas.len();

            // Interior skip: use the ACTUAL window rect (not the shadow's offset position).
            // With shadow_offset_y()>0, using the shadow offset would incorrectly skip
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

    /// Alpha-blend the drag-image overlay onto the back buffer at the given
    /// screen position (pre-clipped to fb bounds; expects ARGB8888 source
    /// pixels). Used by the cross-window drag pipeline. Pixels with alpha=0
    /// are skipped.
    pub(crate) fn blend_drag_image(
        &mut self,
        pixels: *const u32,
        img_w: u32,
        img_h: u32,
        dst_x: i32,
        dst_y: i32,
    ) {
        if pixels.is_null() || img_w == 0 || img_h == 0 {
            return;
        }
        let bb_stride = self.fb_width as usize;
        for sy in 0..img_h as i32 {
            let py = dst_y + sy;
            if py < 0 || (py as u32) >= self.fb_height {
                continue;
            }
            for sx in 0..img_w as i32 {
                let px = dst_x + sx;
                if px < 0 || (px as u32) >= self.fb_width {
                    continue;
                }
                let src_off = (sy as usize) * (img_w as usize) + (sx as usize);
                let src = unsafe { *pixels.add(src_off) };
                let sa = (src >> 24) & 0xFF;
                if sa == 0 {
                    continue;
                }
                let di = py as usize * bb_stride + px as usize;
                if di >= self.back_buffer.len() {
                    continue;
                }
                if sa == 0xFF {
                    self.back_buffer[di] = src;
                } else {
                    let dst = self.back_buffer[di];
                    let inv = 255 - sa;
                    let sr = (src >> 16) & 0xFF;
                    let sg = (src >> 8) & 0xFF;
                    let sb = src & 0xFF;
                    let dr = (dst >> 16) & 0xFF;
                    let dg = (dst >> 8) & 0xFF;
                    let db = dst & 0xFF;
                    let r = ((sr * sa + dr * inv) / 255) & 0xFF;
                    let g = ((sg * sa + dg * inv) / 255) & 0xFF;
                    let b = ((sb * sa + db * inv) / 255) & 0xFF;
                    self.back_buffer[di] = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

fn mix_px(a: u32, b: u32, t: u32) -> u32 {
    if t == 0 {
        return a;
    }
    let ia = 2 - t;
    let aa = ((a >> 24) & 0xFF) * ia + ((b >> 24) & 0xFF) * t;
    let rr = ((a >> 16) & 0xFF) * ia + ((b >> 16) & 0xFF) * t;
    let gg = ((a >> 8) & 0xFF) * ia + ((b >> 8) & 0xFF) * t;
    let bb = (a & 0xFF) * ia + (b & 0xFF) * t;
    ((aa / 2) << 24) | ((rr / 2) << 16) | ((gg / 2) << 8) | (bb / 2)
}
