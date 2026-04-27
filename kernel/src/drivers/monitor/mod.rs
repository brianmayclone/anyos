//! Monitor detection, EDID parsing, and device tree registration.
//!
//! Detects connected monitors via GPU driver EDID queries and/or bootloader
//! VBE DDC data. Each monitor is registered in the HAL device tree as
//! `/dev/monitor0`, `/dev/monitor1`, etc.

pub mod edid;

use crate::drivers::hal::{self, Driver, DriverError, DriverType};
use crate::sync::spinlock::Spinlock;
use alloc::boxed::Box;
use alloc::vec::Vec;
use edid::{DisplayMode, ParsedEdid, MAX_MODES};

// ──────────────────────────────────────────────
// MonitorInfo
// ─────────────────────────────��────────────────

/// Runtime information about a connected monitor.
#[derive(Clone)]
pub struct MonitorInfo {
    /// Monitor index (0, 1, 2, ...).
    pub id: u32,
    /// GPU output / scanout index this monitor is connected to.
    pub gpu_output: u32,
    /// Parsed EDID data.
    pub edid: ParsedEdid,
    /// Supported display modes extracted from EDID.
    pub modes: [DisplayMode; MAX_MODES],
    /// Number of valid entries in `modes`.
    pub mode_count: usize,
    /// Physical width in millimeters.
    pub width_mm: u16,
    /// Physical height in millimeters.
    pub height_mm: u16,
    /// `true` if real EDID was successfully read and parsed.
    pub has_edid: bool,
    /// Raw EDID bytes (base block + optional extension).
    pub edid_raw: [u8; 256],
    /// Length of valid data in `edid_raw` (128 or 256).
    pub edid_raw_len: usize,
}

// ──────────────────────────────────────────────
// Registry
// ─────────────��────────────────────────────────

struct MonitorRegistry {
    monitors: Vec<MonitorInfo>,
}

static MONITORS: Spinlock<Option<MonitorRegistry>> = Spinlock::new(None);

/// Bootloader EDID data (set by `store_bootloader_edid` during early boot).
static BOOT_EDID: Spinlock<Option<[u8; 128]>> = Spinlock::new(None);

/// Initialize the monitor registry. Called once during boot.
pub fn init() {
    let mut reg = MONITORS.lock();
    if reg.is_none() {
        *reg = Some(MonitorRegistry {
            monitors: Vec::new(),
        });
    }
}

/// Store EDID data read by the bootloader (from BootInfo).
pub fn store_bootloader_edid(data: [u8; 128]) {
    *BOOT_EDID.lock() = Some(data);
}

/// Register a new monitor. Returns the assigned monitor ID.
pub fn register_monitor(mut info: MonitorInfo) -> u32 {
    let mut reg = MONITORS.lock();
    let reg = reg.as_mut().expect("monitor registry not initialized");
    let id = reg.monitors.len() as u32;
    info.id = id;
    reg.monitors.push(info);
    id
}

/// Get the number of registered monitors.
pub fn monitor_count() -> u32 {
    let reg = MONITORS.lock();
    match reg.as_ref() {
        Some(r) => r.monitors.len() as u32,
        None => 0,
    }
}

/// Get a clone of MonitorInfo for monitor `id`.
pub fn get_monitor(id: u32) -> Option<MonitorInfo> {
    let reg = MONITORS.lock();
    let reg = reg.as_ref()?;
    reg.monitors.get(id as usize).cloned()
}

/// Get all monitors.
pub fn list_monitors() -> Vec<MonitorInfo> {
    let reg = MONITORS.lock();
    match reg.as_ref() {
        Some(r) => r.monitors.clone(),
        None => Vec::new(),
    }
}

// ────────────────────────────────���─────────────
// MonitorInfo constructors
// ──────���───────────────────────────────────────

/// Create a MonitorInfo from raw EDID bytes.
pub fn monitor_from_edid(raw: &[u8], len: usize, gpu_output: u32) -> MonitorInfo {
    let mut edid_raw = [0u8; 256];
    let copy_len = len.min(256);
    edid_raw[..copy_len].copy_from_slice(&raw[..copy_len]);

    // Parse the base block (first 128 bytes).
    let mut base = [0u8; 128];
    base.copy_from_slice(&edid_raw[..128]);
    let parsed = edid::parse_edid_base(&base);
    let (modes, mode_count) = edid::extract_modes(&parsed);

    // Physical size: prefer detailed timing mm, fall back to cm × 10.
    let (wmm, hmm) = preferred_physical_size(&parsed);

    MonitorInfo {
        id: 0, // set by register_monitor
        gpu_output,
        edid: parsed,
        modes,
        mode_count,
        width_mm: wmm,
        height_mm: hmm,
        has_edid: true,
        edid_raw,
        edid_raw_len: copy_len,
    }
}

/// Create a fallback MonitorInfo when no EDID is available.
pub fn monitor_fallback(width: u32, height: u32, gpu_output: u32) -> MonitorInfo {
    let mut edid = ParsedEdid::default();
    edid.manufacturer = [b'U', b'N', b'K', 0];
    edid.preferred_width = width;
    edid.preferred_height = height;
    edid.preferred_refresh_100 = 6000; // assume 60 Hz

    let modes = [DisplayMode {
        width,
        height,
        refresh_hz_100: 6000,
        is_preferred: true,
        is_interlaced: false,
    }; 1];
    let mut mode_arr = [DisplayMode::default(); MAX_MODES];
    mode_arr[0] = modes[0];

    MonitorInfo {
        id: 0,
        gpu_output,
        edid,
        modes: mode_arr,
        mode_count: 1,
        width_mm: 0,
        height_mm: 0,
        has_edid: false,
        edid_raw: [0u8; 256],
        edid_raw_len: 0,
    }
}

/// Extract the best physical size (mm) from parsed EDID.
fn preferred_physical_size(edid: &ParsedEdid) -> (u16, u16) {
    // First check detailed timing descriptors (more precise, in mm).
    for dt in &edid.detailed_timings {
        if dt.valid && dt.width_mm > 0 && dt.height_mm > 0 {
            return (dt.width_mm, dt.height_mm);
        }
    }
    // Fall back to header cm values × 10.
    if edid.width_cm > 0 && edid.height_cm > 0 {
        return (edid.width_cm as u16 * 10, edid.height_cm as u16 * 10);
    }
    (0, 0)
}

// ──────────────────────────────────────────────
// HAL Driver wrapper
// ───���─────────────────────��────────────────────

/// HAL driver for a monitor device (`/dev/monitorN`).
/// Reading yields raw EDID bytes; ioctl queries monitor properties.
struct MonitorHalDriver {
    monitor_id: u32,
}

impl Driver for MonitorHalDriver {
    fn name(&self) -> &str {
        "Monitor"
    }

    fn driver_type(&self) -> DriverType {
        DriverType::Monitor
    }

    fn init(&mut self) -> Result<(), DriverError> {
        Ok(())
    }

    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        let info = get_monitor(self.monitor_id).ok_or(DriverError::NoDevice)?;
        let avail = info.edid_raw_len.saturating_sub(offset);
        let copy_len = avail.min(buf.len());
        if copy_len == 0 {
            return Ok(0);
        }
        buf[..copy_len].copy_from_slice(&info.edid_raw[offset..offset + copy_len]);
        Ok(copy_len)
    }

    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }

    fn ioctl(&mut self, cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        let info = get_monitor(self.monitor_id).ok_or(DriverError::NoDevice)?;
        match cmd {
            hal::IOCTL_MONITOR_GET_ID => Ok(info.id),
            hal::IOCTL_MONITOR_GET_NATIVE => {
                Ok(info.edid.preferred_width | (info.edid.preferred_height << 16))
            }
            hal::IOCTL_MONITOR_GET_PHYS_SIZE => {
                Ok(info.width_mm as u32 | ((info.height_mm as u32) << 16))
            }
            hal::IOCTL_MONITOR_HAS_EDID => Ok(info.has_edid as u32),
            hal::IOCTL_MONITOR_MODE_COUNT => Ok(info.mode_count as u32),
            hal::IOCTL_MONITOR_GET_GAMMA => Ok(info.edid.gamma as u32),
            hal::IOCTL_MONITOR_GET_COLOR_DEPTH => Ok(info.edid.color_depth as u32),
            _ => Err(DriverError::NotSupported),
        }
    }
}

/// Register a monitor as a HAL device at `/dev/monitorN`.
fn register_monitor_hal(monitor_id: u32) {
    let path = hal::make_device_path(DriverType::Monitor, monitor_id as usize);
    hal::register_device(&path, Box::new(MonitorHalDriver { monitor_id }), None);
}

// ──────────────���───────────────────────────────
// Detection orchestration
// ──────────────────────────────────────────────

/// Detect all connected monitors and register them in the device tree.
///
/// Called from `kernel_main` after GPU probe and HAL binding.
pub fn detect_and_register() {
    init();

    let mut found = 0u32;

    // 1. Query GPU driver for connected displays and their EDID.
    let display_count = crate::drivers::gpu::with_gpu(|g| g.display_count()).unwrap_or(1);

    for output in 0..display_count {
        // Try GPU-provided EDID first.
        let edid_raw = crate::drivers::gpu::with_gpu(|g| g.read_edid(output)).flatten();

        if let Some(raw) = edid_raw {
            let mut info = monitor_from_edid(&raw, 128, output);

            // QEMU's VirtIO GPU generates EDID with 640x480 as the first detailed
            // timing (VGA default), regardless of the configured xres/yres.
            // The actual desired resolution is in the standard timings list.
            // When the preferred timing is implausibly small, find the best
            // resolution by cross-referencing EDID modes with GPU-supported modes.
            if info.edid.preferred_width < 1024 || info.edid.preferred_height < 768 {
                // Get GPU-supported modes for cross-referencing.
                let gpu_modes = crate::drivers::gpu::with_gpu(|g| {
                    let mut v = alloc::vec::Vec::new();
                    for &(w, h) in g.supported_modes() {
                        v.push((w, h));
                    }
                    v
                })
                .unwrap_or_default();

                // Find the largest EDID mode that the GPU also supports.
                let mut best_w = 0u32;
                let mut best_h = 0u32;
                let mut best_r = 0u32;
                for i in 0..info.mode_count {
                    let m = &info.modes[i];
                    if m.width < 1024 || m.height < 768 {
                        continue;
                    }
                    let in_gpu = gpu_modes
                        .iter()
                        .any(|&(gw, gh)| gw == m.width && gh == m.height);
                    if !in_gpu {
                        continue;
                    }
                    let area = m.width as u64 * m.height as u64;
                    if area > best_w as u64 * best_h as u64 {
                        best_w = m.width;
                        best_h = m.height;
                        best_r = m.refresh_hz_100;
                    }
                }

                // If no cross-match, just pick largest EDID mode ≥ 1024x768.
                if best_w == 0 {
                    for i in 0..info.mode_count {
                        let m = &info.modes[i];
                        if m.width < 1024 || m.height < 768 {
                            continue;
                        }
                        let area = m.width as u64 * m.height as u64;
                        if area > best_w as u64 * best_h as u64 {
                            best_w = m.width;
                            best_h = m.height;
                            best_r = m.refresh_hz_100;
                        }
                    }
                }

                if best_w >= 1024 && best_h >= 768 {
                    info.edid.preferred_width = best_w;
                    info.edid.preferred_height = best_h;
                    info.edid.preferred_refresh_100 = best_r;
                    for i in 0..info.mode_count {
                        let is_best =
                            info.modes[i].width == best_w && info.modes[i].height == best_h;
                        info.modes[i].is_preferred = is_best;
                    }
                }
            }

            let id = register_monitor(info);
            register_monitor_hal(id);
            found += 1;
            if let Some(m) = get_monitor(id) {
                let name = core::str::from_utf8(&m.edid.model_name)
                    .unwrap_or("")
                    .trim_end_matches('\0');
                let mfr = core::str::from_utf8(&m.edid.manufacturer[..3]).unwrap_or("???");
                crate::serial_println!(
                    "[OK] Monitor {}: {} {} ({}x{}, EDID from GPU output {})",
                    id,
                    mfr,
                    name,
                    m.edid.preferred_width,
                    m.edid.preferred_height,
                    output
                );
            }
            continue;
        }

        // For output 0, try bootloader EDID as fallback.
        if output == 0 {
            let boot_edid = BOOT_EDID.lock().clone();
            if let Some(raw) = boot_edid {
                // Validate EDID header before using.
                if raw[0] == 0x00 && raw[1] == 0xFF && raw[7] == 0x00 {
                    let info = monitor_from_edid(&raw, 128, 0);
                    let id = register_monitor(info);
                    register_monitor_hal(id);
                    found += 1;
                    if let Some(m) = get_monitor(id) {
                        let name = core::str::from_utf8(&m.edid.model_name)
                            .unwrap_or("")
                            .trim_end_matches('\0');
                        let mfr = core::str::from_utf8(&m.edid.manufacturer[..3]).unwrap_or("???");
                        crate::serial_println!(
                            "[OK] Monitor {}: {} {} (EDID from bootloader VBE DDC)",
                            id,
                            mfr,
                            name
                        );
                    }
                    continue;
                }
            }
        }

        // Check if this output is enabled (has a display attached) via display_info.
        let disp_info = crate::drivers::gpu::with_gpu(|g| g.display_info(output)).flatten();
        if let Some((w, h, true)) = disp_info {
            let info = monitor_fallback(w, h, output);
            let id = register_monitor(info);
            register_monitor_hal(id);
            found += 1;
            crate::serial_println!(
                "[OK] Monitor {}: Unknown ({}x{}, GPU output {}, no EDID)",
                id,
                w,
                h,
                output
            );
        }
    }

    // 2. If no monitors found from GPU, try bootloader EDID alone.
    if found == 0 {
        let boot_edid = BOOT_EDID.lock().clone();
        if let Some(raw) = boot_edid {
            if raw[0] == 0x00 && raw[1] == 0xFF && raw[7] == 0x00 {
                let info = monitor_from_edid(&raw, 128, 0);
                let id = register_monitor(info);
                register_monitor_hal(id);
                found += 1;
                crate::serial_println!("[OK] Monitor {}: detected via bootloader EDID", id);
            }
        }
    }

    // 3. Final fallback: synthesize from current GPU mode.
    if found == 0 {
        if let Some((w, h, _, _)) = crate::drivers::gpu::with_gpu(|g| g.get_mode()) {
            let info = monitor_fallback(w, h, 0);
            let id = register_monitor(info);
            register_monitor_hal(id);
            found += 1;
            crate::serial_println!(
                "[OK] Monitor {}: fallback ({}x{}, no EDID available)",
                id,
                w,
                h
            );
        }
    }

    crate::serial_println!("[OK] Monitor detection: {} monitor(s) registered", found);

    // Emit system event.
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_MONITOR_DETECTED,
        found,
        0,
        0,
        0,
    ));
}
