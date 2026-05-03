//! GPU driver trait and global GPU instance.
//!
//! Provides a unified [`GpuDriver`] trait for GPU drivers (Bochs VGA, VMware SVGA II, etc.)
//! with support for 2D acceleration, hardware cursor, double-buffering, and runtime
//! resolution changes. Drivers are registered dynamically via PCI detection in the HAL.

pub mod amd_fb;
pub mod bochs_vga;
pub mod intel_fb;
pub mod nvidia_fb;
pub mod output;
pub mod vbox_vga;
pub mod virtio_gpu;
pub mod vmware_svga;

pub use output::{
    DisplayEvent, LayoutError, OutputInfo, OutputLayout, OutputLayoutEntry, OutputMode,
    MAX_OUTPUTS,
};

use crate::sync::mutex::Mutex;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

/// Poisoned flag: set after force_unlock_gpu() to prevent use of potentially
/// corrupted GPU driver state. Cleared only by a full GPU re-init.
static GPU_POISONED: AtomicBool = AtomicBool::new(false);
static LAST_GPU_DRIVER_DATA: AtomicU32 = AtomicU32::new(0);
static LAST_GPU_DRIVER_VTABLE_LO: AtomicU32 = AtomicU32::new(0);
static LAST_GPU_DRIVER_VTABLE_HI: AtomicU32 = AtomicU32::new(0);

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
            for &c in msg {
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, c);
            }
            let mut v = vtable;
            let mut buf = [0u8; 16];
            for i in (0..16).rev() {
                let d = (v & 0xF) as u8;
                buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
                v >>= 4;
            }
            for &c in &buf {
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, c);
            }
            let msg2 = b" -- GPU call SKIPPED\r\n";
            for &c in msg2 {
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, c);
            }
        }
        return false;
    }
    true
}

/// Preferred resolution set via boot params (res=WxH). 0 = not set.
static PREFERRED_WIDTH: AtomicU32 = AtomicU32::new(0);
static PREFERRED_HEIGHT: AtomicU32 = AtomicU32::new(0);

/// Set the preferred resolution from boot params.
pub fn set_preferred_resolution(width: u32, height: u32) {
    PREFERRED_WIDTH.store(width, Ordering::Relaxed);
    PREFERRED_HEIGHT.store(height, Ordering::Relaxed);
}

/// Get the preferred resolution, if set. Returns (width, height) or None.
pub fn preferred_resolution() -> Option<(u32, u32)> {
    let w = PREFERRED_WIDTH.load(Ordering::Relaxed);
    let h = PREFERRED_HEIGHT.load(Ordering::Relaxed);
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
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

    /// Driver type identifier for userspace .drv loading.
    /// Returns "svga3d", "virgl", "none", etc.
    /// libGL uses this to load `/Drivers/{type}.drv`.
    fn driver_type_name(&self) -> &str {
        "none"
    }

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
    fn has_accel(&self) -> bool {
        false
    }

    /// Hardware-accelerated rectangle fill. Returns true if executed.
    fn accel_fill_rect(&mut self, _x: u32, _y: u32, _w: u32, _h: u32, _color: u32) -> bool {
        false
    }

    /// Hardware-accelerated rectangle copy. Returns true if executed.
    fn accel_copy_rect(
        &mut self,
        _sx: u32,
        _sy: u32,
        _dx: u32,
        _dy: u32,
        _w: u32,
        _h: u32,
    ) -> bool {
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
    fn vram_size(&self) -> u32 {
        0
    }

    // ── Hardware Cursor ──────────────────────────────────

    /// Returns true if hardware cursor is supported.
    fn has_hw_cursor(&self) -> bool {
        false
    }

    /// Define cursor bitmap (ARGB8888 pixels).
    fn define_cursor(&mut self, _w: u32, _h: u32, _hotx: u32, _hoty: u32, _pixels: &[u32]) {}

    /// Move hardware cursor to screen position.
    fn move_cursor(&mut self, _x: u32, _y: u32) {}

    /// Show or hide the hardware cursor.
    fn show_cursor(&mut self, _visible: bool) {}

    // ── DMA Back Buffer (GMR) ───────────────────────────

    /// Register userspace back_buffer physical pages as GPU-accessible memory.
    /// After registration, transfer_rect uses GPU DMA from this buffer
    /// instead of reading VRAM. `sub_page_offset` is the byte offset within
    /// the first page where pixel data starts (buf_ptr & 0xFFF).
    /// Returns true if successful.
    fn register_back_buffer(&mut self, _phys_pages: &[u64], _sub_page_offset: u32) -> bool {
        false
    }

    /// Whether DMA back_buffer mode is active (GMR registered).
    fn has_dma_back_buffer(&self) -> bool {
        false
    }

    // ── Double Buffering ─────────────────────────────────

    /// Returns true if hardware double-buffering is available.
    fn has_double_buffer(&self) -> bool {
        false
    }

    /// Flip front/back buffers (page flip).
    fn flip(&mut self) {}

    /// Get the physical address of the current back buffer.
    fn back_buffer_phys(&self) -> Option<u32> {
        None
    }

    // ── 3D Acceleration ──────────────────────────────────

    /// Returns true if SVGA3D hardware acceleration is available.
    fn has_3d(&self) -> bool {
        false
    }

    /// Get the 3D hardware version (0 if no 3D support).
    fn hw_version_3d(&self) -> u32 {
        0
    }

    /// Submit raw SVGA3D command words to the GPU FIFO.
    /// The buffer must contain correctly formatted 3D command sequences.
    fn submit_3d_commands(&mut self, _words: &[u32]) -> bool {
        false
    }

    /// Upload data from kernel buffer to a GPU surface via DMA (GMR).
    /// `sid`: target surface ID. `data`: raw bytes to upload.
    /// `width`, `height`: surface dimensions (for DMA copy box).
    /// Returns true on success.
    fn dma_surface_upload(&mut self, _sid: u32, _data: &[u8], _width: u32, _height: u32) -> bool {
        false
    }

    /// Download data from a GPU surface to a kernel buffer via DMA (GMR).
    /// `sid`: source surface ID. `buf`: destination buffer for pixel data.
    /// `width`, `height`: surface dimensions (for DMA copy box).
    /// Returns true on success.
    fn dma_surface_download(
        &mut self,
        _sid: u32,
        _buf: &mut [u8],
        _width: u32,
        _height: u32,
    ) -> bool {
        false
    }

    /// Create a 3D resource via the control plane (virgl).
    /// Returns the allocated resource ID, or None on failure.
    fn create_3d_resource(
        &mut self,
        _target: u32,
        _format: u32,
        _bind: u32,
        _width: u32,
        _height: u32,
        _depth: u32,
        _array_size: u32,
        _last_level: u32,
        _nr_samples: u32,
        _flags: u32,
    ) -> Option<u32> {
        None
    }

    /// Destroy a 3D resource. Returns true on success.
    fn destroy_3d_resource(&mut self, _resource_id: u32) -> bool {
        false
    }

    // ── Monitor / EDID ──

    /// Number of display outputs (scanouts) this GPU supports.
    fn display_count(&self) -> u32 {
        1
    }

    /// Read the 128-byte EDID base block for the given output index.
    /// Returns `None` if EDID is not available for this output.
    fn read_edid(&mut self, _output: u32) -> Option<[u8; 128]> {
        None
    }

    /// Read the 128-byte EDID extension block (bytes 128-255) for the given output.
    /// Returns `None` if no extension block is available.
    fn read_edid_ext(&mut self, _output: u32) -> Option<[u8; 128]> {
        None
    }

    /// Query display info for a given output: (width, height, enabled).
    /// Returns `None` if the output index is out of range.
    fn display_info(&self, _output: u32) -> Option<(u32, u32, bool)> {
        None
    }

    /// Re-query display info from hardware (not cached).
    /// Call this after boot has progressed to get up-to-date display dimensions.
    fn refresh_display_info(&mut self) {}

    // ── Per-output mode / framebuffer (multi-monitor) ──
    //
    // The single-output `set_mode`, `get_mode`, `update_rect`, `transfer_rect`,
    // `flush_display` operate on output 0. The `*_for_output` variants take an
    // explicit output index and let drivers expose multiple independent
    // scanouts. Drivers without multi-output support fall through to the
    // single-output path for `output == 0` and report `None` / no-op for
    // other indices.

    /// Activate a mode on output `output_id`. Returns (width, height, pitch,
    /// fb_phys) on success. For output 0, default delegates to `set_mode`.
    fn set_mode_for_output(
        &mut self,
        output_id: u32,
        width: u32,
        height: u32,
        bpp: u32,
    ) -> Option<(u32, u32, u32, u32)> {
        if output_id == 0 {
            self.set_mode(width, height, bpp)
        } else {
            None
        }
    }

    /// Current mode of output `output_id` as (width, height, pitch, fb_phys).
    /// Returns `None` if the output is not active.
    fn mode_for_output(&self, output_id: u32) -> Option<(u32, u32, u32, u32)> {
        if output_id == 0 {
            let (w, h, p, fb) = self.get_mode();
            if w > 0 && h > 0 {
                Some((w, h, p, fb))
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Transfer a dirty region of output `output_id` from guest RAM to GPU.
    /// Default: delegates to `transfer_rect` for output 0.
    fn transfer_rect_for_output(&mut self, output_id: u32, x: u32, y: u32, w: u32, h: u32) {
        if output_id == 0 {
            self.transfer_rect(x, y, w, h);
        }
    }

    /// Flush a region of output `output_id` to the host display.
    /// Default: delegates to `flush_display` for output 0.
    fn flush_for_output(&mut self, output_id: u32, x: u32, y: u32, w: u32, h: u32) {
        if output_id == 0 {
            self.flush_display(x, y, w, h);
        }
    }

    /// Combined transfer + flush for output `output_id` (legacy update_rect
    /// fast-path). Default: delegates to `update_rect` for output 0.
    fn update_rect_for_output(&mut self, output_id: u32, x: u32, y: u32, w: u32, h: u32) {
        if output_id == 0 {
            self.update_rect(x, y, w, h);
        }
    }
}

// ──────────────────────────────────────────────
// Global GPU instance
// ──────────────────────────────────────────────

/// Global GPU driver instance, set during PCI probe.
///
/// Uses a yielding [`Mutex`] (not a spinlock) so that long-running DMA
/// operations inside the driver (VirtIO virtqueue polling, VMware SVGA FIFO
/// sync) do **not** hold interrupts disabled. The timer IRQ keeps firing,
/// other threads keep running, and the scheduler cannot deadlock.
///
/// Rule: never acquire this lock from an interrupt handler. IRQ-context code
/// that needs GPU state must use [`try_lock_gpu`] (non-blocking) and handle
/// the `None` case gracefully.
static GPU: Mutex<Option<Box<dyn GpuDriver>>> = Mutex::new(None);

/// Register a GPU driver (called from HAL driver factory during PCI probe).
/// Clears the poison flag if set from a previous crash.
pub fn register(driver: Box<dyn GpuDriver>) {
    crate::serial_verbose_println!("  GPU: registered '{}'", driver.name());
    GPU_POISONED.store(false, Ordering::Release);
    let mut gpu = GPU.lock();
    *gpu = Some(driver);
}

/// Access the registered GPU driver within a closure.
///
/// Acquires the GPU [`Mutex`] with interrupts **enabled** — safe to hold
/// across long operations (DMA, FIFO sync, VirtIO polling). Returns `None`
/// if no driver is registered, poisoned after a crash, or the vtable appears
/// corrupted.
pub fn with_gpu<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut dyn GpuDriver) -> R,
{
    // Fast-path reject: if the GPU was force-unlocked after a crash, the driver
    // state is potentially corrupt. All further calls are blocked until re-init.
    if GPU_POISONED.load(Ordering::Relaxed) {
        return None;
    }
    let mut gpu = GPU.lock();
    let boxed = gpu.as_mut()?;
    let driver: &mut dyn GpuDriver = boxed.as_mut();
    let fat: [usize; 2] = unsafe { core::mem::transmute_copy(&(driver as *const dyn GpuDriver)) };
    LAST_GPU_DRIVER_DATA.store(fat[0] as u32, Ordering::Relaxed);
    LAST_GPU_DRIVER_VTABLE_LO.store(fat[1] as u32, Ordering::Relaxed);
    LAST_GPU_DRIVER_VTABLE_HI.store((fat[1] >> 32) as u32, Ordering::Relaxed);
    if !validate_gpu_vtable(driver) {
        return None;
    }
    Some(f(driver))
}

/// Last observed GPU trait-object raw pointers for crash diagnostics.
pub fn last_driver_ptrs() -> (u32, u64) {
    let data = LAST_GPU_DRIVER_DATA.load(Ordering::Relaxed);
    let vtable_lo = LAST_GPU_DRIVER_VTABLE_LO.load(Ordering::Relaxed) as u64;
    let vtable_hi = LAST_GPU_DRIVER_VTABLE_HI.load(Ordering::Relaxed) as u64;
    (data, vtable_lo | (vtable_hi << 32))
}

/// Check if a GPU driver is registered and not poisoned.
pub fn is_available() -> bool {
    !GPU_POISONED.load(Ordering::Relaxed) && GPU.lock().is_some()
}

/// Check if the GPU was poisoned after a crash (force-unlock).
/// The compositor can use this to detect that a GPU re-init is needed.
pub fn is_gpu_poisoned() -> bool {
    GPU_POISONED.load(Ordering::Relaxed)
}

/// Check if the GPU mutex is currently held (by any thread).
pub fn is_gpu_locked() -> bool {
    GPU.is_locked()
}

/// Force-release the GPU mutex from a crash/fault handler.
///
/// Poisons the GPU: all subsequent `with_gpu()` calls return `None` until
/// the GPU driver is re-registered. This prevents use of potentially corrupt
/// driver state (partially modified DMA buffers, inconsistent virtqueue, etc.)
/// that caused the original crash.
///
/// # Safety
/// Only call when the current thread is known to hold the GPU mutex and is
/// about to be terminated.
pub unsafe fn force_unlock_gpu() {
    GPU_POISONED.store(true, Ordering::Release);
    GPU.force_unlock();
}

/// Non-blocking GPU access (for use during panic/RSOD where yielding is not safe).
///
/// Returns `Some(guard)` only if the mutex is currently free and not poisoned.
/// Callers **must** handle `None` gracefully — the GPU may be in use by another thread.
pub fn try_lock_gpu() -> Option<crate::sync::mutex::MutexGuard<'static, Option<Box<dyn GpuDriver>>>>
{
    if GPU_POISONED.load(Ordering::Relaxed) {
        return None;
    }
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

use crate::drivers::hal::{
    Driver, DriverError, DriverType, IOCTL_DISPLAY_FLIP, IOCTL_DISPLAY_GET_MODE,
    IOCTL_DISPLAY_GET_PITCH, IOCTL_DISPLAY_HAS_ACCEL, IOCTL_DISPLAY_HAS_HW_CURSOR,
    IOCTL_DISPLAY_IS_DBLBUF, IOCTL_DISPLAY_LIST_MODES, IOCTL_DISPLAY_SET_MODE,
};
use crate::drivers::pci::PciDevice;

struct GpuHalDriver {
    name: &'static str,
}

impl Driver for GpuHalDriver {
    fn name(&self) -> &str {
        self.name
    }
    fn driver_type(&self) -> DriverType {
        DriverType::Display
    }
    fn init(&mut self) -> Result<(), DriverError> {
        Ok(())
    }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn ioctl(&mut self, cmd: u32, arg: u32) -> Result<u32, DriverError> {
        if !is_available() {
            return Err(DriverError::NotSupported);
        }
        match cmd {
            IOCTL_DISPLAY_GET_MODE => with_gpu(|g| {
                let (w, h, _, _) = g.get_mode();
                w | (h << 16)
            })
            .ok_or(DriverError::IoError),
            IOCTL_DISPLAY_FLIP => {
                with_gpu(|g| g.flip());
                Ok(0)
            }
            IOCTL_DISPLAY_IS_DBLBUF => Ok(with_gpu(|g| g.has_double_buffer() as u32).unwrap_or(0)),
            IOCTL_DISPLAY_GET_PITCH => with_gpu(|g| {
                let (_, _, pitch, _) = g.get_mode();
                pitch
            })
            .ok_or(DriverError::IoError),
            IOCTL_DISPLAY_SET_MODE => {
                let w = arg & 0xFFFF;
                let h = (arg >> 16) & 0xFFFF;
                with_gpu(|g| g.set_mode(w, h, 32).map(|(w, h, _, _)| w | (h << 16)))
                    .flatten()
                    .ok_or(DriverError::IoError)
            }
            IOCTL_DISPLAY_LIST_MODES => {
                Ok(with_gpu(|g| g.supported_modes().len() as u32).unwrap_or(0))
            }
            IOCTL_DISPLAY_HAS_ACCEL => Ok(with_gpu(|g| g.has_accel() as u32).unwrap_or(0)),
            IOCTL_DISPLAY_HAS_HW_CURSOR => Ok(with_gpu(|g| g.has_hw_cursor() as u32).unwrap_or(0)),
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
        crate::serial_verbose_println!("  GPU: VBoxSVGA detected (SVGA II mode)");
        vmware_svga::init_and_register(pci);
        create_hal_driver("VBoxSVGA")
    } else {
        crate::serial_verbose_println!("  GPU: VBoxVGA detected (HGSMI mode)");
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
