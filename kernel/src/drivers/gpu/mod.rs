//! GPU driver trait and global GPU instance.
//!
//! Provides a unified [`GpuDriver`] trait for GPU drivers (Bochs VGA, VMware SVGA II, etc.)
//! with support for 2D acceleration, hardware cursor, double-buffering, and runtime
//! resolution changes. Drivers are registered dynamically via PCI detection in the HAL.

pub mod bochs_vga;
pub mod virtio_gpu;
pub mod vmware_svga;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use crate::sync::spinlock::Spinlock;

/// Common display resolutions supported by QEMU VGA devices
pub static COMMON_MODES: &[(u32, u32)] = &[
    (640, 480),
    (800, 600),
    (1024, 768),
    (1152, 864),
    (1280, 720),
    (1280, 1024),
    (1440, 900),
    (1600, 900),
    (1600, 1200),
    (1920, 1080),
];

/// GPU driver trait — implemented by Bochs VGA, VMware SVGA II, etc.
pub trait GpuDriver: Send {
    /// Human-readable driver name
    fn name(&self) -> &str;

    /// Set display resolution. Returns (width, height, pitch, fb_phys) on success.
    fn set_mode(&mut self, width: u32, height: u32, bpp: u32) -> Option<(u32, u32, u32, u32)>;

    /// Get current mode: (width, height, pitch, fb_phys).
    fn get_mode(&self) -> (u32, u32, u32, u32);

    /// List supported resolutions.
    fn supported_modes(&self) -> &[(u32, u32)] {
        COMMON_MODES
    }

    // ── 2D Acceleration ──────────────────────────────────

    /// Returns true if hardware 2D acceleration is available.
    fn has_accel(&self) -> bool { false }

    /// Hardware-accelerated rectangle fill. Returns true if executed.
    fn accel_fill_rect(&mut self, _x: u32, _y: u32, _w: u32, _h: u32, _color: u32) -> bool {
        false
    }

    /// Hardware-accelerated rectangle copy. Returns true if executed.
    fn accel_copy_rect(&mut self, _sx: u32, _sy: u32, _dx: u32, _dy: u32, _w: u32, _h: u32) -> bool {
        false
    }

    /// Notify the GPU that a screen region has been updated (for SVGA FIFO).
    fn update_rect(&mut self, _x: u32, _y: u32, _w: u32, _h: u32) {}

    // ── Hardware Cursor ──────────────────────────────────

    /// Returns true if hardware cursor is supported.
    fn has_hw_cursor(&self) -> bool { false }

    /// Define cursor bitmap (ARGB8888 pixels).
    fn define_cursor(&mut self, _w: u32, _h: u32, _hotx: u32, _hoty: u32, _pixels: &[u32]) {}

    /// Move hardware cursor to screen position.
    fn move_cursor(&mut self, _x: u32, _y: u32) {}

    /// Show or hide the hardware cursor.
    fn show_cursor(&mut self, _visible: bool) {}

    // ── Double Buffering ─────────────────────────────────

    /// Returns true if hardware double-buffering is available.
    fn has_double_buffer(&self) -> bool { false }

    /// Flip front/back buffers (page flip).
    fn flip(&mut self) {}

    /// Get the physical address of the current back buffer.
    fn back_buffer_phys(&self) -> Option<u32> { None }
}

// ──────────────────────────────────────────────
// Global GPU instance
// ──────────────────────────────────────────────

/// Global GPU driver instance, set during PCI probe.
static GPU: Spinlock<Option<Box<dyn GpuDriver>>> = Spinlock::new(None);

/// Register a GPU driver (called from HAL driver factory during PCI probe).
pub fn register(driver: Box<dyn GpuDriver>) {
    crate::serial_println!("  GPU: registered '{}'", driver.name());
    let mut gpu = GPU.lock();
    *gpu = Some(driver);
}

/// Access the registered GPU driver within a closure.
/// Returns None if no GPU driver is registered.
pub fn with_gpu<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut dyn GpuDriver) -> R,
{
    let mut gpu = GPU.lock();
    gpu.as_mut().map(|g| f(g.as_mut()))
}

/// Check if a GPU driver is registered.
pub fn is_available() -> bool {
    GPU.lock().is_some()
}

/// Non-blocking GPU lock (for use during panic/RSOD where deadlock must be avoided).
pub fn try_lock_gpu() -> Option<crate::sync::spinlock::SpinlockGuard<'static, Option<Box<dyn GpuDriver>>>> {
    GPU.try_lock()
}

// ──────────────────────────────────────────────
// Boot splash cursor: IRQ-time HW cursor updates
// ──────────────────────────────────────────────

static SPLASH_CURSOR_ACTIVE: AtomicBool = AtomicBool::new(false);
static SPLASH_CURSOR_X: AtomicI32 = AtomicI32::new(0);
static SPLASH_CURSOR_Y: AtomicI32 = AtomicI32::new(0);
static SPLASH_SCREEN_W: AtomicU32 = AtomicU32::new(1024);
static SPLASH_SCREEN_H: AtomicU32 = AtomicU32::new(768);

/// Enable boot-splash cursor mode. The mouse IRQ handler will directly
/// update the HW cursor position via GPU I/O registers, bypassing the
/// compositor. This ensures lag-free cursor movement during boot.
pub fn enable_splash_cursor(screen_w: u32, screen_h: u32) {
    SPLASH_SCREEN_W.store(screen_w, Ordering::Relaxed);
    SPLASH_SCREEN_H.store(screen_h, Ordering::Relaxed);
    SPLASH_CURSOR_X.store((screen_w / 2) as i32, Ordering::Relaxed);
    SPLASH_CURSOR_Y.store((screen_h / 2) as i32, Ordering::Relaxed);
    SPLASH_CURSOR_ACTIVE.store(true, Ordering::Release);
}

/// Disable kernel-side HW cursor tracking.
pub fn disable_splash_cursor() {
    SPLASH_CURSOR_ACTIVE.store(false, Ordering::Release);
}

/// Check if kernel-side cursor tracking is active.
pub fn is_splash_cursor_active() -> bool {
    SPLASH_CURSOR_ACTIVE.load(Ordering::Acquire)
}

/// Update the screen dimensions for kernel-side cursor clamping.
/// Called on resolution change so the cursor stays within bounds.
pub fn update_cursor_bounds(screen_w: u32, screen_h: u32) {
    SPLASH_SCREEN_W.store(screen_w, Ordering::Relaxed);
    SPLASH_SCREEN_H.store(screen_h, Ordering::Relaxed);
    // Clamp current position to new bounds
    let x = SPLASH_CURSOR_X.load(Ordering::Relaxed);
    let y = SPLASH_CURSOR_Y.load(Ordering::Relaxed);
    SPLASH_CURSOR_X.store(x.min(screen_w as i32 - 1).max(0), Ordering::Relaxed);
    SPLASH_CURSOR_Y.store(y.min(screen_h as i32 - 1).max(0), Ordering::Relaxed);
}

/// Called from mouse IRQ handler when a complete packet is assembled.
/// Updates the HW cursor position directly at IRQ time if splash mode is active.
/// Returns true if handled (splash active), false otherwise (normal compositor path).
pub fn splash_cursor_move(dx: i32, dy: i32) -> bool {
    if !SPLASH_CURSOR_ACTIVE.load(Ordering::Acquire) {
        return false;
    }
    let sw = SPLASH_SCREEN_W.load(Ordering::Relaxed) as i32;
    let sh = SPLASH_SCREEN_H.load(Ordering::Relaxed) as i32;

    // Atomically update cursor position
    let old_x = SPLASH_CURSOR_X.load(Ordering::Relaxed);
    let old_y = SPLASH_CURSOR_Y.load(Ordering::Relaxed);
    let new_x = (old_x + dx).max(0).min(sw - 1);
    let new_y = (old_y + dy).max(0).min(sh - 1);
    SPLASH_CURSOR_X.store(new_x, Ordering::Relaxed);
    SPLASH_CURSOR_Y.store(new_y, Ordering::Relaxed);

    // Update HW cursor via GPU (try_lock to avoid deadlock from IRQ context)
    if let Some(mut gpu) = GPU.try_lock() {
        if let Some(g) = gpu.as_mut() {
            g.move_cursor(new_x as u32, new_y as u32);
        }
    }
    true
}

/// Get the current splash cursor position (used by compositor on transition).
pub fn splash_cursor_position() -> (i32, i32) {
    (
        SPLASH_CURSOR_X.load(Ordering::Relaxed),
        SPLASH_CURSOR_Y.load(Ordering::Relaxed),
    )
}
