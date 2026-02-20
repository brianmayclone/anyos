//! Layer and shadow data structures for the compositor.

use alloc::vec::Vec;
use super::rect::Rect;

// ── Shadow Constants ────────────────────────────────────────────────────────

pub(crate) const SHADOW_OFFSET_X: i32 = 0;
pub(crate) const SHADOW_OFFSET_Y: i32 = 6;
/// Total spread (number of concentric rings) for the soft shadow.
pub(crate) const SHADOW_SPREAD: i32 = 16;
/// Shadow alpha for the focused window (innermost ring).
pub(crate) const SHADOW_ALPHA_FOCUSED: u32 = 50;
/// Shadow alpha for unfocused windows (innermost ring).
pub(crate) const SHADOW_ALPHA_UNFOCUSED: u32 = 25;

// ── Shadow Cache ────────────────────────────────────────────────────────────

/// Pre-computed shadow alpha values for a given layer size.
/// Avoids expensive per-pixel SDF + isqrt computation every frame.
pub(crate) struct ShadowCache {
    /// Alpha values (0-255 normalized) for each pixel in the shadow bitmap.
    /// Layout: row-major, width = layer_w + 2*SHADOW_SPREAD.
    pub(crate) alphas: Vec<u8>,
    pub(crate) cache_w: u32,
    pub(crate) cache_h: u32,
    /// Layer dimensions this was computed for (invalidated on resize).
    pub(crate) layer_w: u32,
    pub(crate) layer_h: u32,
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
    pub(crate) shadow_cache: Option<ShadowCache>,
    /// VRAM-direct surface: app writes directly to off-screen VRAM, compositor
    /// uses GPU RECT_COPY instead of CPU pixel copy during compositing.
    pub is_vram: bool,
    /// Y-coordinate of this layer's surface in off-screen VRAM (for RECT_COPY source).
    pub vram_y: u32,
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

// ── AccelMoveHint ────────────────────────────────────────────────────────────

/// Tracks a pending GPU-accelerated layer move for RECT_COPY optimization.
/// Multiple move_layer() calls within the same frame are coalesced: keeps the
/// first old_bounds and updates new_bounds on each subsequent call.
#[allow(dead_code)]
pub(crate) struct AccelMoveHint {
    pub(crate) layer_id: u32,
    pub(crate) old_bounds: Rect,
    pub(crate) new_bounds: Rect,
}

/// Compute up to 4 non-overlapping rects covering `a` minus `b`.
/// Returns [top_strip, bottom_strip, left_strip, right_strip] (some may be empty).
#[allow(dead_code)]
pub(crate) fn subtract_rects(a: &Rect, b: &Rect) -> [Rect; 4] {
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
