//! GPU command dispatch and framebuffer I/O.

use super::rect::Rect;
use super::Compositor;
use anyos_std::ipc;

// ── GPU Command Types ───────────────────────────────────────────────────────

pub(crate) const GPU_UPDATE: u32 = 1;
#[allow(dead_code)]
pub(crate) const GPU_RECT_FILL: u32 = 2;
pub(crate) const GPU_RECT_COPY: u32 = 3;
pub(crate) const GPU_CURSOR_MOVE: u32 = 4;
pub(crate) const GPU_CURSOR_SHOW: u32 = 5;
pub(crate) const GPU_DEFINE_CURSOR: u32 = 6;
// Multi-monitor cursor commands — the kernel forwards these to the
// per-output cursor methods on the GpuDriver trait.
pub(crate) const GPU_CURSOR_MOVE_OUTPUT: u32 = 11;
pub(crate) const GPU_CURSOR_SHOW_OUTPUT: u32 = 12;
pub(crate) const GPU_FLIP: u32 = 7;
pub(crate) const GPU_SYNC: u32 = 8;

impl Compositor {
    pub fn enable_double_buffer(&mut self) {
        self.hw_double_buffer = true;
        self.current_page = 0;
    }

    pub fn enable_gpu_accel(&mut self) {
        self.gpu_accel = true;
    }

    /// Try to register the back buffer as a GPU GMR for DMA transfers.
    /// If successful, flush_region() skips the CPU memcpy to VRAM.
    pub fn try_enable_gmr(&mut self) {
        let ptr = self.back_buffer.as_ptr() as u32;
        let size = (self.back_buffer.len() * 4) as u32;
        let result = ipc::gpu_register_backbuffer(ptr, size);
        if result == 0 {
            self.gmr_active = true;
        }
    }

    pub fn enable_hw_cursor(&mut self) {
        self.hw_cursor = true;
        self.gpu_cmds
            .push([GPU_CURSOR_SHOW, 1, 0, 0, 0, 0, 0, 0, 0]);
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

    /// Multi-monitor: move the HW cursor on a specific output, with
    /// `x` / `y` in the output's *local* coordinates. The kernel
    /// forwards to `GpuDriver::move_cursor_for_output`.
    pub fn move_hw_cursor_on_output(&mut self, output_id: u32, x: i32, y: i32) {
        if self.hw_cursor {
            self.gpu_cmds.push([
                GPU_CURSOR_MOVE_OUTPUT,
                output_id,
                x as u32,
                y as u32,
                0,
                0,
                0,
                0,
                0,
            ]);
        }
    }

    /// Multi-monitor: show / hide the HW cursor on a specific output.
    /// Used by the compositor to make the cursor "follow" the user
    /// across output boundaries (hide on the previous output, show
    /// on the new one).
    pub fn set_hw_cursor_visible_on_output(&mut self, output_id: u32, visible: bool) {
        if self.hw_cursor {
            self.gpu_cmds.push([
                GPU_CURSOR_SHOW_OUTPUT,
                output_id,
                if visible { 1 } else { 0 },
                0,
                0,
                0,
                0,
                0,
                0,
            ]);
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
            // Only issue sfence when VRAM was actually written (flush_region sets vram_dirty).
            // Without this barrier after VRAM writes, the CPU's WC buffers may not be
            // flushed, causing the GPU to read stale framebuffer data.
            // In GMR mode, back_buffer is in normal cacheable RAM — no sfence needed.
            if self.vram_dirty && !self.gmr_active {
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    core::arch::asm!("sfence", options(nostack, preserves_flags));
                }
                #[cfg(target_arch = "aarch64")]
                unsafe {
                    core::arch::asm!("dsb st", options(nostack, preserves_flags));
                }
                self.vram_dirty = false;
            }
            ipc::gpu_command(&self.gpu_cmds);
            self.gpu_cmds.clear();
        }
    }

    /// Step 1 of the deferred-flush pattern: issue sfence for any VRAM writes done so far.
    ///
    /// Call this under the compositor lock after all compositing work is complete
    /// and before `drain_gpu_cmds()`. The sfence must happen while the lock is held
    /// (while the compositor still owns the VRAM write-combining buffers).
    pub fn prepare_flush(&mut self) {
        if self.vram_dirty && !self.gmr_active {
            #[cfg(target_arch = "x86_64")]
            unsafe {
                core::arch::asm!("sfence", options(nostack, preserves_flags));
            }
            #[cfg(target_arch = "aarch64")]
            unsafe {
                core::arch::asm!("dsb st", options(nostack, preserves_flags));
            }
            self.vram_dirty = false;
        }
    }

    /// Step 2 of the deferred-flush pattern: extract all pending GPU commands.
    ///
    /// Call this under the compositor lock after `prepare_flush()`.
    /// Returns the command batch; the queue is left empty.
    /// Pass the returned batch to `Compositor::submit_cmds()` *outside* the lock.
    pub fn drain_gpu_cmds(&mut self) -> alloc::vec::Vec<[u32; 9]> {
        core::mem::take(&mut self.gpu_cmds)
    }

    /// Step 3 of the deferred-flush pattern: submit pre-extracted GPU commands.
    ///
    /// Call this *outside* the compositor lock. `ipc::gpu_command()` may block
    /// briefly if the VirtIO GPU ring is full; keeping it outside the lock prevents
    /// the management thread from spinning (busy-wait on `acquire_lock()`) during
    /// that wait, which would waste 100% CPU and stall input processing.
    pub fn submit_cmds(cmds: alloc::vec::Vec<[u32; 9]>) {
        if !cmds.is_empty() {
            ipc::gpu_command(&cmds);
        }
    }

    /// Flush a region from back buffer to the visible framebuffer (no offset).
    pub fn flush_region_pub(&mut self, rect: &Rect) {
        self.flush_region(rect, 0);
    }

    /// Flush a region to the currently VISIBLE framebuffer page.
    /// Used for SW cursor: after compose()'s FLIP, the visible page depends on current_page.
    /// For non-double-buffer mode, also sends GPU_UPDATE.
    pub fn flush_cursor_region(&mut self, rect: &Rect) {
        if self.hw_double_buffer {
            // After compose()'s FLIP, current_page was swapped.
            // The page we just wrote to (now visible) is:
            //   current_page==1 -> visible at fb_height, current_page==0 -> visible at 0
            let front_offset = if self.current_page == 1 {
                self.fb_height
            } else {
                0
            };
            self.flush_region(rect, front_offset);
        } else {
            self.flush_region(rect, 0);
            self.gpu_cmds.push([
                GPU_UPDATE,
                rect.x as u32,
                rect.y as u32,
                rect.width,
                rect.height,
                0,
                0,
                0,
                0,
            ]);
            self.flush_gpu();
        }
    }
}
