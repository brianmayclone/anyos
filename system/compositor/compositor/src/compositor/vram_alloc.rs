//! Off-screen VRAM allocator for VRAM-direct surfaces.
//!
//! Manages off-screen VRAM (beyond the visible framebuffer) as a simple
//! linear allocator with a free list. Each allocation is pitch-aligned
//! (one row per pitch bytes) so GPU RECT_COPY can address it directly.

use alloc::vec::Vec;

/// A single VRAM allocation (off-screen region for an app surface).
#[derive(Clone, Copy)]
pub struct VramAlloc {
    /// Byte offset from VRAM start.
    pub offset: u32,
    /// Allocation size in bytes.
    pub size: u32,
    /// Y-coordinate in VRAM (row index, for RECT_COPY source).
    pub vram_y: u32,
    /// Surface width in pixels.
    pub width: u32,
    /// Surface height in pixels.
    pub height: u32,
    /// Associated layer ID (0 if freed).
    pub layer_id: u32,
}

/// VRAM off-screen allocator.
pub struct VramAllocator {
    /// Total VRAM size in bytes.
    total_bytes: u32,
    /// Bytes used by the visible framebuffer (pitch * height).
    fb_used_bytes: u32,
    /// Screen pitch in bytes.
    pitch: u32,
    /// Active allocations, sorted by offset.
    allocs: Vec<VramAlloc>,
}

impl VramAllocator {
    /// Create a new allocator. `fb_pitch` is the screen pitch in bytes,
    /// `fb_height` is the visible screen height, `vram_total` is the total VRAM.
    pub fn new(fb_pitch: u32, fb_height: u32, vram_total: u32) -> Self {
        let fb_used = fb_pitch * fb_height;
        VramAllocator {
            total_bytes: vram_total,
            fb_used_bytes: fb_used,
            pitch: fb_pitch,
            allocs: Vec::with_capacity(16),
        }
    }

    /// Available off-screen VRAM in bytes.
    pub fn free_bytes(&self) -> u32 {
        let used: u32 = self.allocs.iter().map(|a| a.size).sum();
        let off_screen = self.total_bytes.saturating_sub(self.fb_used_bytes);
        off_screen.saturating_sub(used)
    }

    /// Total off-screen VRAM in bytes.
    pub fn off_screen_bytes(&self) -> u32 {
        self.total_bytes.saturating_sub(self.fb_used_bytes)
    }

    /// Allocate an off-screen VRAM region for a surface of given dimensions.
    /// The allocation uses `pitch * height` bytes (matching screen stride).
    /// Returns `Some(VramAlloc)` on success, `None` if out of VRAM.
    pub fn alloc(&mut self, width: u32, height: u32, layer_id: u32) -> Option<VramAlloc> {
        if width == 0 || height == 0 || self.pitch == 0 {
            return None;
        }

        // Each row in off-screen VRAM uses the full screen pitch
        let size = self.pitch * height;
        let off_screen_start = self.fb_used_bytes;
        let off_screen_end = self.total_bytes;

        if size > off_screen_end.saturating_sub(off_screen_start) {
            return None;
        }

        // Find the first gap that fits (first-fit)
        // Align start offset to pitch boundary
        let align_to_pitch = |off: u32| -> u32 {
            let rem = off % self.pitch;
            if rem == 0 { off } else { off + self.pitch - rem }
        };

        let mut candidate = align_to_pitch(off_screen_start);

        for alloc in &self.allocs {
            if candidate + size <= alloc.offset {
                // Gap before this allocation
                break;
            }
            // Move past this allocation
            candidate = align_to_pitch(alloc.offset + alloc.size);
        }

        if candidate + size > off_screen_end {
            return None; // No room
        }

        let vram_y = candidate / self.pitch;
        let entry = VramAlloc {
            offset: candidate,
            size,
            vram_y,
            width,
            height,
            layer_id,
        };

        // Insert sorted by offset
        let pos = self.allocs.iter().position(|a| a.offset > candidate)
            .unwrap_or(self.allocs.len());
        self.allocs.insert(pos, entry);

        Some(entry)
    }

    /// Free a VRAM allocation by layer ID.
    pub fn free(&mut self, layer_id: u32) {
        self.allocs.retain(|a| a.layer_id != layer_id);
    }

    /// Look up allocation by layer ID.
    pub fn get(&self, layer_id: u32) -> Option<&VramAlloc> {
        self.allocs.iter().find(|a| a.layer_id == layer_id)
    }

    /// Update screen parameters (e.g., after resolution change).
    /// Frees all allocations since they're no longer valid.
    pub fn update_fb(&mut self, fb_pitch: u32, fb_height: u32, vram_total: u32) {
        self.pitch = fb_pitch;
        self.fb_used_bytes = fb_pitch * fb_height;
        self.total_bytes = vram_total;
        self.allocs.clear();
    }
}
