//! GPU driver trait and global GPU instance.
//!
//! Provides a unified [`GpuDriver`] trait for GPU drivers (Bochs VGA, VMware SVGA II, etc.)
//! with support for 2D acceleration, hardware cursor, double-buffering, and runtime
//! resolution changes. Drivers are registered dynamically via PCI detection in the HAL.

pub mod bochs_vga;
pub mod vbox_vga;
pub mod virtio_gpu;
pub mod vmware_svga;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use crate::sync::spinlock::Spinlock;

/// Validate a `&dyn GpuDriver` trait object's vtable pointer.
/// Returns false if data or vtable pointer is outside kernel higher-half,
/// indicating heap corruption of the `Box<dyn GpuDriver>`.
#[inline]
fn validate_gpu_vtable(driver: &dyn GpuDriver) -> bool {
    let fat: [usize; 2] = unsafe { core::mem::transmute_copy(&(driver as *const dyn GpuDriver)) };
    let data = fat[0] as u64;
    let vtable = fat[1] as u64;
    const KERNEL_HIGHER_HALF: u64 = 0xFFFF_FFFF_8000_0000;
    if data < KERNEL_HIGHER_HALF || vtable < KERNEL_HIGHER_HALF {
        unsafe {
            use crate::arch::x86::port::{inb, outb};
            let msg = b"\r\n!!! GPU VTABLE CORRUPT vtable=";
            for &c in msg { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
            let mut v = vtable;
            let mut buf = [0u8; 16];
            for i in (0..16).rev() {
                let d = (v & 0xF) as u8;
                buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
                v >>= 4;
            }
            for &c in &buf { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
            let msg2 = b" -- GPU call SKIPPED\r\n";
            for &c in msg2 { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
        }
        return false;
    }
    true
}

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

    /// Transfer a dirty region to the GPU without flushing to display.
    /// Default: falls back to update_rect (transfer+flush combined).
    /// VirtIO GPU overrides this to only TRANSFER_TO_HOST_2D.
    fn transfer_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.update_rect(x, y, w, h);
    }

    /// Flush accumulated transfers to the display as a single
    /// bounding-box region. Default: no-op (drivers that don't
    /// separate transfer/flush already did everything in transfer_rect).
    fn flush_display(&mut self, _x: u32, _y: u32, _w: u32, _h: u32) {}

    /// Synchronize: wait for GPU to process all pending FIFO commands.
    fn sync(&mut self) {}

    /// Total VRAM size in bytes (0 if unknown).
    fn vram_size(&self) -> u32 { 0 }

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
/// Returns None if no GPU driver is registered or if the trait object
/// vtable appears corrupted (prevents RIP=0x3 crash from heap corruption).
pub fn with_gpu<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut dyn GpuDriver) -> R,
{
    let mut gpu = GPU.lock();
    let boxed = gpu.as_mut()?;
    let driver: &mut dyn GpuDriver = boxed.as_mut();
    if !validate_gpu_vtable(driver) {
        return None;
    }
    Some(f(driver))
}

/// Check if a GPU driver is registered.
pub fn is_available() -> bool {
    GPU.lock().is_some()
}

/// Lock-free check if the GPU lock is currently held.
pub fn is_gpu_locked() -> bool {
    GPU.is_locked()
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
            let driver: &mut dyn GpuDriver = g.as_mut();
            if validate_gpu_vtable(driver) {
                driver.move_cursor(new_x as u32, new_y as u32);
            }
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

// ── HAL integration ─────────────────────────────────────────────────────────

use crate::drivers::pci::PciDevice;
use crate::drivers::hal::{Driver, DriverType, DriverError,
    IOCTL_DISPLAY_GET_MODE, IOCTL_DISPLAY_FLIP, IOCTL_DISPLAY_IS_DBLBUF,
    IOCTL_DISPLAY_GET_PITCH, IOCTL_DISPLAY_SET_MODE, IOCTL_DISPLAY_LIST_MODES,
    IOCTL_DISPLAY_HAS_ACCEL, IOCTL_DISPLAY_HAS_HW_CURSOR};

struct GpuHalDriver {
    name: &'static str,
}

impl Driver for GpuHalDriver {
    fn name(&self) -> &str { self.name }
    fn driver_type(&self) -> DriverType { DriverType::Display }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn ioctl(&mut self, cmd: u32, arg: u32) -> Result<u32, DriverError> {
        if !is_available() { return Err(DriverError::NotSupported); }
        match cmd {
            IOCTL_DISPLAY_GET_MODE => {
                with_gpu(|g| {
                    let (w, h, _, _) = g.get_mode();
                    w | (h << 16)
                }).ok_or(DriverError::IoError)
            }
            IOCTL_DISPLAY_FLIP => {
                with_gpu(|g| g.flip());
                Ok(0)
            }
            IOCTL_DISPLAY_IS_DBLBUF => {
                Ok(with_gpu(|g| g.has_double_buffer() as u32).unwrap_or(0))
            }
            IOCTL_DISPLAY_GET_PITCH => {
                with_gpu(|g| {
                    let (_, _, pitch, _) = g.get_mode();
                    pitch
                }).ok_or(DriverError::IoError)
            }
            IOCTL_DISPLAY_SET_MODE => {
                let w = arg & 0xFFFF;
                let h = (arg >> 16) & 0xFFFF;
                with_gpu(|g| {
                    g.set_mode(w, h, 32).map(|(w, h, _, _)| w | (h << 16))
                }).flatten().ok_or(DriverError::IoError)
            }
            IOCTL_DISPLAY_LIST_MODES => {
                Ok(with_gpu(|g| g.supported_modes().len() as u32).unwrap_or(0))
            }
            IOCTL_DISPLAY_HAS_ACCEL => {
                Ok(with_gpu(|g| g.has_accel() as u32).unwrap_or(0))
            }
            IOCTL_DISPLAY_HAS_HW_CURSOR => {
                Ok(with_gpu(|g| g.has_hw_cursor() as u32).unwrap_or(0))
            }
            _ => Err(DriverError::NotSupported),
        }
    }
}

/// Create a HAL Driver wrapper for the GPU subsystem (called from driver probe).
pub(crate) fn create_hal_driver(name: &'static str) -> Option<Box<dyn Driver>> {
    Some(Box::new(GpuHalDriver { name }))
}

/// Auto-detect and initialize VirtualBox GPU (VBoxSVGA vs VBoxVGA based on BAR0).
pub fn vbox_probe(pci: &PciDevice) -> Option<Box<dyn Driver>> {
    if pci.bars[0] & 1 != 0 {
        crate::serial_println!("  GPU: VBoxSVGA detected (SVGA II mode)");
        vmware_svga::init_and_register(pci);
        create_hal_driver("VBoxSVGA")
    } else {
        crate::serial_println!("  GPU: VBoxVGA detected (HGSMI mode)");
        vbox_vga::init_and_register(pci);
        create_hal_driver("VBoxVGA (HGSMI)")
    }
}

/// Probe for Bochs/QEMU VGA (already initialized via VBE during boot).
pub fn bochs_probe(_pci: &PciDevice) -> Option<Box<dyn Driver>> {
    create_hal_driver("Bochs/QEMU VGA")
}

/// Fallback probe for generic VGA-compatible controller.
pub fn generic_vga_probe(_pci: &PciDevice) -> Option<Box<dyn Driver>> {
    create_hal_driver("Generic VGA")
}
