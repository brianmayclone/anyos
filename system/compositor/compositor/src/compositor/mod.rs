//! Layer-based compositor engine.
//!
//! Manages z-ordered layers, tracks damage regions, and composites
//! visible layers onto a back buffer, then flushes to the framebuffer.

mod blend;
mod compositing;
pub(crate) mod gpu;
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

/// Pixels + geometry for the drag-image overlay rendered during a cross-
/// window drag. The compositor maps the source-allocated SHM read-only and
/// blends `image` under the cursor each compose pass.
pub struct DragImageOverlay {
    /// Source-allocated SHM region holding ARGB8888 pixels. Owned by source.
    pub shm_id: u32,
    /// Read-only mapping of the SHM in compositor address space.
    pub pixels: *const u32,
    pub image_w: u32,
    pub image_h: u32,
    pub hot_x: i32,
    pub hot_y: i32,
    /// Last drawn screen position (top-left); used so we can damage the
    /// previous spot on the next compose.
    pub last_x: i32,
    pub last_y: i32,
    pub last_drawn: bool,
}

/// One output (display) the compositor scans out to.
///
/// Output 0 ("primary") is mirrored by the inline `fb_ptr / fb_width /
/// fb_height / fb_pitch / back_buffer / damage` fields of `Compositor`
/// — that representation is kept untouched so the existing single-output
/// render fast paths keep working without refactoring risk. Outputs ≥ 1
/// live in `Compositor::outputs` and own their own framebuffer mapping
/// (`SYS_DISPLAY_MAP_FB(id)` returns a per-output VA at
/// 0x2000_0000 + id*64 MiB), back buffer, and damage list.
///
/// `virtual_x / virtual_y` is the output's top-left position in the
/// global virtual desktop. Windows live in virtual coordinates; per
/// output we compute the visible sub-rectangle by intersecting each
/// window's bbox with `(virtual_x, virtual_y, fb_width, fb_height)`.
pub struct Output {
    pub id: u32,
    pub virtual_x: i32,
    pub virtual_y: i32,
    pub fb_ptr: *mut u32,
    pub fb_width: u32,
    pub fb_height: u32,
    pub fb_pitch: u32,
    pub back_buffer: Vec<u32>,
    pub damage: Vec<Rect>,
    pub primary: bool,
    pub mirror_of: Option<u32>,
}

impl Output {
    /// Build the Output entry that mirrors the primary fields of the
    /// containing Compositor. Used at construction time so callers don't
    /// have to repeat the field list.
    fn primary_view(fb_ptr: *mut u32, w: u32, h: u32, pitch: u32) -> Self {
        Self {
            id: 0,
            virtual_x: 0,
            virtual_y: 0,
            fb_ptr,
            fb_width: w,
            fb_height: h,
            fb_pitch: pitch,
            // Output 0's back_buffer / damage live in the legacy inline
            // fields for now; this entry's vectors stay empty so we
            // never accidentally render into a duplicate buffer.
            back_buffer: Vec::new(),
            damage: Vec::new(),
            primary: true,
            mirror_of: None,
        }
    }
}

// Output is accessed only from the compositor thread; the raw fb_ptr is
// not Send/Sync by default but the rest of the compositor already
// asserts the same invariant for its own fb_ptr field.
unsafe impl Send for Output {}

pub struct Compositor {
    /// Framebuffer pointer (MMIO VRAM mapped at 0x20000000)
    pub(crate) fb_ptr: *mut u32,
    pub(crate) fb_width: u32,
    pub(crate) fb_height: u32,
    /// Framebuffer pitch in bytes (may differ from width*4)
    pub(crate) fb_pitch: u32,

    /// All display outputs. `outputs[0]` is always the primary and
    /// mirrors the inline `fb_*` fields above. Additional outputs are
    /// appended by `init_secondary_outputs()` after construction.
    pub outputs: Vec<Output>,

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

    /// Drag-image overlay (drawn under the cursor during cross-window
    /// drag-and-drop). Pixels are ARGB8888 in `image`, sized `image_w` ×
    /// `image_h`, and the top-left of the rendered position is
    /// `(cursor_x - hot_x, cursor_y - hot_y)`. `None` while no drag image
    /// is set.
    pub drag_image: Option<DragImageOverlay>,

    /// The currently focused layer (gets stronger shadow)
    pub focused_layer_id: Option<u32>,

    /// Pending GPU-accelerated move hint for RECT_COPY optimization
    pub(crate) accel_move_hint: Option<AccelMoveHint>,

    /// Off-screen VRAM allocator for VRAM-direct surfaces.
    /// None if GPU accel not available or VRAM too small.
    pub(crate) vram_allocator: Option<VramAllocator>,

    /// Reusable scratch buffer for blur operations (avoids per-frame heap allocation).
    pub(crate) blur_temp: Vec<u32>,

    /// Incremented whenever scene content below blur-behind layers may have changed.
    pub(crate) blur_generation: u64,

    /// Reusable Vec for compositing loop (swap with self.damage to avoid drain+collect alloc).
    pub(crate) compositing_damage: Vec<Rect>,

    /// Tracks whether VRAM was written since the last sfence.
    pub(crate) vram_dirty: bool,

    /// GPU DMA mode: back_buffer is registered as a GMR, no memcpy to VRAM needed.
    pub(crate) gmr_active: bool,
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
            outputs: alloc::vec![Output::primary_view(fb_ptr, width, height, pitch)],
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
            drag_image: None,
            focused_layer_id: None,
            accel_move_hint: None,
            vram_allocator: None,
            blur_temp: Vec::with_capacity(width.max(height) as usize),
            blur_generation: 1,
            compositing_damage: Vec::with_capacity(32),
            vram_dirty: false,
            gmr_active: false,
        }
    }

    pub fn width(&self) -> u32 {
        self.fb_width
    }
    pub fn height(&self) -> u32 {
        self.fb_height
    }

    /// Total virtual-desktop bounding box across every output.
    pub fn virtual_desktop_bounds(&self) -> (i32, i32, i32, i32) {
        if self.outputs.is_empty() {
            return (0, 0, self.fb_width as i32, self.fb_height as i32);
        }
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for o in &self.outputs {
            min_x = min_x.min(o.virtual_x);
            min_y = min_y.min(o.virtual_y);
            max_x = max_x.max(o.virtual_x + o.fb_width as i32);
            max_y = max_y.max(o.virtual_y + o.fb_height as i32);
        }
        (min_x, min_y, max_x, max_y)
    }

    /// Find the output whose rectangle contains `(vx, vy)` in virtual
    /// desktop coordinates. Returns the primary output when no output
    /// covers the point (e.g. a window dragged into a gap).
    pub fn output_at(&self, vx: i32, vy: i32) -> &Output {
        for o in &self.outputs {
            if vx >= o.virtual_x
                && vy >= o.virtual_y
                && vx < o.virtual_x + o.fb_width as i32
                && vy < o.virtual_y + o.fb_height as i32
            {
                return o;
            }
        }
        // Fallback: outputs[0] is always the primary.
        &self.outputs[0]
    }

    /// Discover and map secondary outputs reported by the kernel.
    ///
    /// Called once after `Compositor::new()` and `register_compositor()`.
    /// Walks `SYS_DISPLAY_LIST`; for each connected output beyond index 0
    /// it issues `SYS_DISPLAY_MAP_FB(id)`, places the output to the right
    /// of the previous one in the virtual desktop, and pushes an `Output`
    /// entry. Any failure for a single output (mode not set, mapping
    /// rejected) is logged and skipped — the compositor stays usable
    /// with the outputs it could map.
    ///
    /// Layout policy here is deliberately minimal — "stack to the right
    /// of primary in scanout-id order". The richer policy (persisted
    /// per-EDID layout, drag-to-arrange, primary selection) lives in the
    /// future `displayd` daemon (phase 5); this is what the compositor
    /// uses as a sane bootstrap fallback when no displayd is running yet.
    pub fn init_secondary_outputs(&mut self) {
        let infos = anyos_std::display::list(16);
        if infos.len() <= 1 {
            return;
        }
        // Cursor for stacking: start to the right of output 0.
        let mut next_x = self.fb_width as i32;
        for info in infos.iter().skip(1) {
            if !info.is_connected() {
                continue;
            }
            // The kernel may report current_w/h as 0 for a connected but
            // never-set-up scanout. In that case fall back to the
            // preferred mode; if that's also missing, skip.
            let (w, h) = if info.current_w > 0 && info.current_h > 0 {
                (info.current_w, info.current_h)
            } else if info.preferred_w > 0 && info.preferred_h > 0 {
                (info.preferred_w, info.preferred_h)
            } else {
                anyos_std::println!(
                    "[compositor] output {} reports no usable mode; skipping",
                    info.id
                );
                continue;
            };

            let fb_info = match anyos_std::display::map_fb(info.id) {
                Some(f) => f,
                None => {
                    anyos_std::println!(
                        "[compositor] SYS_DISPLAY_MAP_FB failed for output {} ({}x{})",
                        info.id,
                        w,
                        h
                    );
                    continue;
                }
            };

            // Zero the secondary's framebuffer so we don't show garbage
            // until the first composite reaches it.
            unsafe {
                let pixels = (fb_info.height as usize) * (fb_info.pitch as usize / 4);
                core::ptr::write_bytes(fb_info.fb_addr as *mut u32, 0, pixels);
            }

            self.outputs.push(Output {
                id: info.id,
                virtual_x: next_x,
                virtual_y: 0,
                fb_ptr: fb_info.fb_addr as *mut u32,
                fb_width: fb_info.width,
                fb_height: fb_info.height,
                fb_pitch: fb_info.pitch,
                back_buffer: alloc::vec![0u32; (fb_info.width * fb_info.height) as usize],
                damage: Vec::with_capacity(32),
                primary: false,
                mirror_of: None,
            });

            // Tell the kernel to flush the now-zeroed framebuffer so the
            // host immediately stops showing whatever splash QEMU painted.
            let _ = anyos_std::display::flush(info.id, 0, 0, fb_info.width, fb_info.height);

            anyos_std::println!(
                "[compositor] output {} active at virt=({},{}), {}x{}, fb_va={:#x}",
                info.id,
                next_x,
                0,
                fb_info.width,
                fb_info.height,
                fb_info.fb_addr
            );

            next_x += fb_info.width as i32;
        }
    }

    /// Render every secondary output (id ≥ 1).
    ///
    /// Called after the primary `compose()` pass. For each non-primary
    /// output the function fills its back-buffer with the desktop
    /// background colour, blits any visible layer that overlaps the
    /// output's virtual rect (translated into the output's local
    /// coordinates), copies the back-buffer into the output's
    /// framebuffer, and finally requests a per-output `SYS_DISPLAY_FLUSH`.
    ///
    /// Intentionally minimal: no shadow / blur / hardware cursor support
    /// on secondary outputs yet. Those can land in a follow-up commit
    /// once the basic per-output coordinate flow is verified visually.
    /// Damage tracking is also coarse (full-output rerender every frame
    /// when `force` is set) — fine-grained per-output damage rings will
    /// be wired up in a later phase.
    pub fn render_secondary_outputs(&mut self, force: bool) {
        if self.outputs.len() < 2 || !force {
            return;
        }

        // Desktop-background colour (matches the wallpaper "fill" used on
        // the primary while the wallpaper image is loading). Anything that
        // wants the actual wallpaper to extend across outputs needs the
        // background-layer rendering path duplicated here, which is a
        // separate piece of work.
        const BG: u32 = 0xFF1A1A2E;

        // Visual parity with the primary's composite_rect:
        //
        //   * windows with `has_shadow == true` get a drop shadow drawn
        //     before the layer pixels — a soft falloff matching the
        //     primary's shadow_spread(). The primary uses a baked alpha
        //     cache for performance; on secondary outputs we recompute
        //     per frame because secondary frames are far less frequent
        //     and the simpler code is easier to keep correct.
        //
        //   * the same windows have rounded corners — pixels closer than
        //     CORNER_RADIUS to a corner get an alpha multiplier that
        //     fades to 0 at the corner (rough quarter-circle mask).
        //
        // Blur and the hardware cursor remain primary-only for now;
        // those need a heavier composite_rect refactor (parameterising
        // the destination buffer + stride) to share the existing baked
        // caches between outputs.
        const CORNER_RADIUS: i32 = 8;
        let shadow_spread_px = crate::desktop::theme::scale_i32(16);
        let shadow_offset_y_px = crate::desktop::theme::scale_i32(6);

        // We need read access to layers (immutable borrow on self.layers)
        // and write access to outputs[idx].back_buffer / fb_ptr. Split via
        // indices to avoid the borrow checker fighting us.
        let n_outputs = self.outputs.len();
        let n_layers = self.layers.len();

        for oi in 1..n_outputs {
            let (ox, oy, ow, oh, fb_ptr, fb_pitch) = {
                let o = &self.outputs[oi];
                (
                    o.virtual_x,
                    o.virtual_y,
                    o.fb_width,
                    o.fb_height,
                    o.fb_ptr,
                    o.fb_pitch,
                )
            };
            // Reusable per-output back buffer.
            {
                let bb = &mut self.outputs[oi].back_buffer;
                if bb.len() != (ow * oh) as usize {
                    bb.resize((ow * oh) as usize, 0);
                }
                bb.fill(BG);
            }

            // Output rect in virtual coordinates.
            let ox2 = ox + ow as i32;
            let oy2 = oy + oh as i32;

            // ── Pass 1: drop shadows for has_shadow layers ────────────
            for li in 0..n_layers {
                let layer = &self.layers[li];
                if !layer.visible || !layer.has_shadow {
                    continue;
                }
                let lx = layer.x;
                let ly = layer.y;
                let lw = layer.width as i32;
                let lh = layer.height as i32;
                let is_focused = self.focused_layer_id == Some(layer.id);
                let base_a: i32 = if is_focused { 50 } else { 25 };
                // Shadow band: from spread pixels around the offset
                // window position, falling off linearly to 0.
                let sx0 = lx - shadow_spread_px;
                let sy0 = ly + shadow_offset_y_px - shadow_spread_px;
                let sx1 = lx + lw + shadow_spread_px;
                let sy1 = ly + lh + shadow_offset_y_px + shadow_spread_px;
                // Intersect with output rect.
                let ix = sx0.max(ox);
                let iy = sy0.max(oy);
                let ix2 = sx1.min(ox2);
                let iy2 = sy1.min(oy2);
                if ix >= ix2 || iy >= iy2 {
                    continue;
                }
                let bb = &mut self.outputs[oi].back_buffer;
                let dst_stride = ow as usize;
                let win_y0 = ly + shadow_offset_y_px;
                let win_y1 = ly + lh + shadow_offset_y_px;
                for vy in iy..iy2 {
                    let dy = vy - oy;
                    let row_off = (dy as usize) * dst_stride;
                    // Vertical distance to the offset window rect (0
                    // inside it).
                    let vdist = if vy < win_y0 {
                        win_y0 - vy
                    } else if vy >= win_y1 {
                        vy - win_y1 + 1
                    } else {
                        0
                    };
                    for vx in ix..ix2 {
                        // Skip pixels inside the actual window position
                        // (will be overwritten by the layer pixels in
                        // pass 2). The window itself sits at (lx, ly)
                        // — not the offset y; we still want shadow
                        // visible in the strip below the window
                        // because shadow_offset_y > 0.
                        if vx >= lx && vx < lx + lw && vy >= ly && vy < ly + lh {
                            continue;
                        }
                        let hdist = if vx < lx {
                            lx - vx
                        } else if vx >= lx + lw {
                            vx - lx - lw + 1
                        } else {
                            0
                        };
                        let dist = vdist.max(hdist);
                        if dist > shadow_spread_px {
                            continue;
                        }
                        // Linear falloff from base_a at dist=0 to 0 at
                        // dist=spread.
                        let t = (shadow_spread_px - dist).max(0);
                        let a = (base_a * t / shadow_spread_px) as u32;
                        if a == 0 {
                            continue;
                        }
                        let dx = (vx - ox) as usize;
                        let dst = bb[row_off + dx];
                        let dr = (dst >> 16) & 0xFF;
                        let dg = (dst >> 8) & 0xFF;
                        let db = dst & 0xFF;
                        // shadow colour = pure black, alpha = a (out of
                        // 255). dst' = dst * (255-a) / 255.
                        let inv = 255 - a;
                        let r = dr * inv / 255;
                        let g = dg * inv / 255;
                        let b = db * inv / 255;
                        bb[row_off + dx] = 0xFF000000 | (r << 16) | (g << 8) | b;
                    }
                }
            }

            // ── Pass 2: layer pixels with optional rounded corners ────
            for li in 0..n_layers {
                let layer = &self.layers[li];
                if !layer.visible {
                    continue;
                }
                let lx = layer.x;
                let ly = layer.y;
                let lx2 = lx + layer.width as i32;
                let ly2 = ly + layer.height as i32;

                // Intersect layer with output rect (virtual coords).
                let ix = lx.max(ox);
                let iy = ly.max(oy);
                let ix2 = lx2.min(ox2);
                let iy2 = ly2.min(oy2);
                if ix >= ix2 || iy >= iy2 {
                    continue;
                }
                let layer_w_i = layer.width as i32;
                let layer_h_i = layer.height as i32;
                let rounded = layer.has_shadow;

                // Source row of pixels = SHM-backed for shm layers,
                // owned Vec otherwise.
                let src_w = layer.width as usize;
                let src_pixels: *const u32 = if !layer.shm_ptr.is_null() {
                    layer.shm_ptr as *const u32
                } else if !layer.pixels.is_empty() {
                    layer.pixels.as_ptr()
                } else {
                    continue;
                };

                let bb = &mut self.outputs[oi].back_buffer;
                let dst_stride = ow as usize;
                for vy in iy..iy2 {
                    let layer_local_y = vy - ly; // 0..layer_h
                    let dst_y = (vy - oy) as usize;
                    for vx in ix..ix2 {
                        let layer_local_x = vx - lx; // 0..layer_w
                        let dst_x = (vx - ox) as usize;
                        let dst_idx = dst_y * dst_stride + dst_x;
                        let src_idx = (layer_local_y as usize) * src_w
                            + (layer_local_x as usize);
                        let p = unsafe { core::ptr::read(src_pixels.add(src_idx)) };

                        // Per-pixel alpha multiplier from rounded-corner
                        // mask. 256 = unmodified; 0 = fully transparent.
                        let mut corner_mul: u32 = 256;
                        if rounded {
                            let cr = CORNER_RADIUS;
                            // Distance from each corner in (cx,cy)
                            // measured from the corner's *inner pixel*.
                            let near_left = layer_local_x < cr;
                            let near_right = layer_local_x >= layer_w_i - cr;
                            let near_top = layer_local_y < cr;
                            let near_bottom = layer_local_y >= layer_h_i - cr;
                            if (near_left || near_right) && (near_top || near_bottom) {
                                let cx = if near_left {
                                    cr - 1 - layer_local_x
                                } else {
                                    layer_local_x - (layer_w_i - cr)
                                };
                                let cy = if near_top {
                                    cr - 1 - layer_local_y
                                } else {
                                    layer_local_y - (layer_h_i - cr)
                                };
                                let dist_sq = (cx * cx + cy * cy) as i32;
                                let r_sq = cr * cr;
                                if dist_sq >= r_sq {
                                    corner_mul = 0;
                                } else {
                                    // Soft 1-pixel anti-aliased edge.
                                    let r_inner_sq = (cr - 1) * (cr - 1);
                                    if dist_sq > r_inner_sq {
                                        let frac = ((r_sq - dist_sq) * 256)
                                            / (r_sq - r_inner_sq).max(1);
                                        corner_mul = frac.clamp(0, 256) as u32;
                                    }
                                }
                            }
                        }

                        let a = ((p >> 24) & 0xFF) * corner_mul / 256;
                        bb[dst_idx] = if a == 0 {
                            bb[dst_idx]
                        } else if !layer.opaque && a < 255 {
                            let inv = 255 - a;
                            let dr = (bb[dst_idx] >> 16) & 0xFF;
                            let dg = (bb[dst_idx] >> 8) & 0xFF;
                            let db = bb[dst_idx] & 0xFF;
                            let sr = (p >> 16) & 0xFF;
                            let sg = (p >> 8) & 0xFF;
                            let sb = p & 0xFF;
                            let r = (sr * a + dr * inv) / 255;
                            let g = (sg * a + dg * inv) / 255;
                            let b = (sb * a + db * inv) / 255;
                            0xFF000000 | (r << 16) | (g << 8) | b
                        } else {
                            p | 0xFF000000
                        };
                    }
                }
            }

            // Flush back_buffer to per-output framebuffer (stride conversion).
            unsafe {
                let bb = &self.outputs[oi].back_buffer;
                let dst_stride_px = (fb_pitch / 4) as usize;
                let src_stride = ow as usize;
                for row in 0..(oh as usize) {
                    let src_off = row * src_stride;
                    let dst_off = row * dst_stride_px;
                    core::ptr::copy_nonoverlapping(
                        bb.as_ptr().add(src_off),
                        fb_ptr.add(dst_off),
                        ow as usize,
                    );
                }
            }

            // Tell the GPU to transfer + flush this output.
            let output_id = self.outputs[oi].id;
            let _ = anyos_std::display::flush(output_id, 0, 0, ow, oh);
        }
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
            blur_cache: None,
            shadow_cache: None,
            is_vram: false,
            vram_y: 0,
            dpi_aware: false,
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
            blur_cache: None,
            shadow_cache: None,
            is_vram: false,
            vram_y: 0,
            dpi_aware: false,
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
            blur_cache: None,
            shadow_cache: None,
            is_vram: false,
            vram_y: 0,
            dpi_aware: false,
        });
        id
    }

    /// Remove a layer by ID.
    pub fn remove_layer(&mut self, id: u32) {
        if let Some(idx) = self.layer_index(id) {
            let layer = &self.layers[idx];
            self.damage.push(layer.damage_bounds());
            self.blur_generation = self.blur_generation.wrapping_add(1);
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
    pub fn add_vram_layer(&mut self, x: i32, y: i32, w: u32, h: u32) -> Option<u32> {
        let alloc = self
            .vram_allocator
            .as_mut()?
            .alloc(w, h, self.next_layer_id)?;
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
            blur_cache: None,
            shadow_cache: None,
            is_vram: true,
            vram_y: alloc.vram_y,
            dpi_aware: false,
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
        self.layers
            .iter()
            .find(|l| l.id == layer_id && l.is_vram)
            .map(|l| l.vram_y)
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
        self.layers
            .iter_mut()
            .find(|l| l.id == id)
            .map(|l| &mut l.pixels)
    }

    /// Move a layer to a new position.
    pub fn move_layer(&mut self, id: u32, new_x: i32, new_y: i32) {
        if let Some(idx) = self.layer_index(id) {
            // Give drag/move damage a 1px safety margin. Some window chrome and
            // shadow edges use anti-aliased pixels right at the boundary, and a
            // strict old/new bounds update can otherwise leave thin stale traces
            // behind until another damage pass repaints them.
            let old_bounds = self.layers[idx].damage_bounds().expand(1);
            self.layers[idx].x = new_x;
            self.layers[idx].y = new_y;
            if self.layers[idx].blur_behind {
                self.layers[idx].blur_cache = None;
            } else {
                self.blur_generation = self.blur_generation.wrapping_add(1);
            }
            let new_bounds = self.layers[idx].damage_bounds().expand(1);

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
                self.blur_generation = self.blur_generation.wrapping_add(1);
                self.damage.push(bounds);
            }
        }
    }

    /// Set the focused layer (gets stronger shadow).
    pub fn set_focused_layer(&mut self, id: Option<u32>) {
        if self.focused_layer_id != id {
            self.blur_generation = self.blur_generation.wrapping_add(1);
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
                self.layers[idx].blur_cache = None;
                if !self.layers[idx].blur_behind {
                    self.blur_generation = self.blur_generation.wrapping_add(1);
                }
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

    /// Resize a layer (reallocates pixel buffer, preserving old content).
    pub fn resize_layer(&mut self, id: u32, new_w: u32, new_h: u32) {
        if let Some(idx) = self.layer_index(id) {
            let old_w = self.layers[idx].width;
            let old_h = self.layers[idx].height;

            if old_w == new_w && old_h == new_h {
                return;
            }

            let old_bounds = self.layers[idx].damage_bounds();
            self.damage.push(old_bounds);

            // Preserve old content — copy existing pixels into the new buffer
            // so the previous frame stays visible until the app redraws.
            // Newly exposed regions (right/bottom) are filled with a dark
            // background color to avoid transparent/black gaps.
            let bg = 0xFF1E_1E1E_u32; // dark neutral background
            let mut new_pixels = vec![bg; (new_w * new_h) as usize];
            let copy_w = old_w.min(new_w) as usize;
            let copy_h = old_h.min(new_h) as usize;
            let old_pixels = &self.layers[idx].pixels;
            for y in 0..copy_h {
                let src_off = y * old_w as usize;
                let dst_off = y * new_w as usize;
                new_pixels[dst_off..dst_off + copy_w]
                    .copy_from_slice(&old_pixels[src_off..src_off + copy_w]);
            }

            self.layers[idx].width = new_w;
            self.layers[idx].height = new_h;
            self.layers[idx].pixels = new_pixels;
            self.layers[idx].shadow_cache = None;
            self.layers[idx].blur_cache = None;
            self.blur_generation = self.blur_generation.wrapping_add(1);
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
    /// When GMR DMA mode is active, the GPU reads directly from the back buffer
    /// via DMA — no CPU memcpy needed. The kernel's transfer_rect handles the blit.
    pub(crate) fn flush_region(&mut self, rect: &Rect, y_offset: u32) {
        // In GMR mode, the GPU will DMA-read from back_buffer directly.
        // Skip the CPU memcpy to VRAM entirely.
        if self.gmr_active && y_offset == 0 {
            return;
        }

        let bb_stride = self.fb_width as usize;
        let fb_stride = (self.fb_pitch / 4) as usize;

        let x = rect.x.max(0) as usize;
        let y = rect.y.max(0) as usize;
        let fb_w = self.fb_width as usize;
        let fb_h = self.fb_height as usize;
        if x >= fb_w || y >= fb_h {
            return;
        }
        let w = (rect.width as usize).min(fb_w - x);
        let h = (rect.height as usize).min(fb_h - y);
        if w == 0 || h == 0 {
            return;
        }

        for row in 0..h {
            let src_off = (y + row) * bb_stride + x;
            let dst_off = (y + row + y_offset as usize) * fb_stride + x;
            if src_off + w > self.back_buffer.len() {
                break;
            }
            unsafe {
                core::ptr::copy_nonoverlapping(
                    self.back_buffer.as_ptr().add(src_off),
                    self.fb_ptr.add(dst_off),
                    w,
                );
            }
        }
        self.vram_dirty = true;
    }

    /// Full-screen damage (force recomposition of everything).
    pub fn damage_all(&mut self) {
        self.damage
            .push(Rect::new(0, 0, self.fb_width, self.fb_height));
        self.blur_generation = self.blur_generation.wrapping_add(1);
    }

    /// Resize the compositor for a new screen resolution.
    /// Reallocates the back buffer and updates dimensions. Layers are NOT touched.
    pub fn resize_fb(&mut self, new_width: u32, new_height: u32, new_pitch: u32) {
        self.fb_width = new_width;
        self.fb_height = new_height;
        self.fb_pitch = new_pitch;
        let pixel_count = (new_width * new_height) as usize;
        self.back_buffer = vec![0u32; pixel_count];
        self.blur_temp = Vec::with_capacity(new_width.max(new_height) as usize);
        self.blur_generation = self.blur_generation.wrapping_add(1);
        // GMR must be re-registered since back_buffer was reallocated
        if self.gmr_active {
            self.gmr_active = false;
            self.try_enable_gmr();
        }
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
