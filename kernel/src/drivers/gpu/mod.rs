/// GPU Driver Abstraction
///
/// Provides a unified trait for GPU drivers (Bochs VGA, VMware SVGA II, etc.)
/// with support for 2D acceleration, hardware cursor, double-buffering,
/// and runtime resolution changes.
///
/// GPU drivers are registered dynamically via PCI detection in the HAL.
/// If no GPU driver is registered, the compositor falls back to direct
/// software-only framebuffer rendering.

pub mod bochs_vga;
pub mod vmware_svga;

use alloc::boxed::Box;
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
