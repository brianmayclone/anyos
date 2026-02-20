//! Hardware Abstraction Layer (HAL).
//!
//! Provides a unified [`Driver`] trait and a central device registry. Includes automatic
//! PCI-to-driver matching for device detection at boot, plus legacy device registration.
//!
//! PCI device-to-driver mappings are in [`super::pci_drivers`].

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;
use crate::drivers::pci::PciDevice;

// ──────────────────────────────────────────────
// Driver trait + types
// ──────────────────────────────────────────────

/// Classification of device drivers by hardware category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverType {
    Block,
    Char,
    Network,
    Display,
    Input,
    Audio,
    Output,  // Speakers, printers, LEDs
    Sensor,  // Temperature, accelerometer, etc.
    Bus,
    Unknown,
}

/// Errors returned by driver operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    NotSupported,
    NotInitialized,
    IoError,
    InvalidArgument,
    Timeout,
    DeviceBusy,
    NoDevice,
}

// ── Ioctl command ranges by device class ──

/// Display ioctl: get current mode as `width | (height << 16)`.
pub const IOCTL_DISPLAY_GET_MODE: u32 = 0x0100;
/// Display ioctl: flip the double buffer.
pub const IOCTL_DISPLAY_FLIP: u32 = 0x0101;
/// Display ioctl: returns 1 if double-buffered.
pub const IOCTL_DISPLAY_IS_DBLBUF: u32 = 0x0102;
/// Display ioctl: returns pitch in bytes.
pub const IOCTL_DISPLAY_GET_PITCH: u32 = 0x0103;
/// Display ioctl: set mode, arg = `width | (height << 16)`.
pub const IOCTL_DISPLAY_SET_MODE: u32 = 0x0104;
/// Display ioctl: returns count of supported modes.
pub const IOCTL_DISPLAY_LIST_MODES: u32 = 0x0105;
/// Display ioctl: returns 1 if 2D acceleration is available.
pub const IOCTL_DISPLAY_HAS_ACCEL: u32 = 0x0106;
/// Display ioctl: returns 1 if hardware cursor is available.
pub const IOCTL_DISPLAY_HAS_HW_CURSOR: u32 = 0x0107;

/// Audio ioctl: get sample rate.
pub const IOCTL_AUDIO_GET_SAMPLE_RATE: u32 = 0x0200;
/// Audio ioctl: set volume level.
pub const IOCTL_AUDIO_SET_VOLUME: u32 = 0x0201;
/// Audio ioctl: get current volume level.
pub const IOCTL_AUDIO_GET_VOLUME: u32 = 0x0202;

/// Network ioctl: get MAC address.
pub const IOCTL_NET_GET_MAC: u32 = 0x0300;
/// Network ioctl: get link status.
pub const IOCTL_NET_GET_LINK: u32 = 0x0301;

/// Sensor ioctl: read sensor value.
pub const IOCTL_SENSOR_READ: u32 = 0x0400;
/// Sensor ioctl: get sensor type identifier.
pub const IOCTL_SENSOR_GET_TYPE: u32 = 0x0401;

/// Output ioctl: get output device status.
pub const IOCTL_OUTPUT_STATUS: u32 = 0x0500;
/// Output ioctl: flush output buffer.
pub const IOCTL_OUTPUT_FLUSH: u32 = 0x0501;

/// Unified device driver interface for the HAL registry.
pub trait Driver: Send {
    /// Human-readable driver name.
    fn name(&self) -> &str;
    /// The category of this driver.
    fn driver_type(&self) -> DriverType;
    /// Initialize the driver hardware. Called once during HAL probe.
    fn init(&mut self) -> Result<(), DriverError>;
    /// Read data from the device at the given byte offset.
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError>;
    /// Write data to the device at the given byte offset.
    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, DriverError>;
    /// Perform a device-specific control operation.
    fn ioctl(&mut self, cmd: u32, arg: u32) -> Result<u32, DriverError>;
}

// ──────────────────────────────────────────────
// HAL Device Registry
// ──────────────────────────────────────────────

struct HalDevice {
    path: String,
    driver: Box<dyn Driver>,
    pci: Option<PciDevice>,
}

struct HalRegistry {
    devices: Vec<HalDevice>,
}

static HAL: Spinlock<Option<HalRegistry>> = Spinlock::new(None);

/// Initialize the HAL registry
pub fn init() {
    let mut hal = HAL.lock();
    *hal = Some(HalRegistry {
        devices: Vec::new(),
    });
    crate::serial_println!("[OK] HAL initialized");
}

/// Register a device with the HAL
pub fn register_device(path: &str, driver: Box<dyn Driver>, pci: Option<PciDevice>) {
    let driver_type = driver.driver_type() as u32;
    let mut hal = HAL.lock();
    if let Some(registry) = hal.as_mut() {
        crate::serial_println!("  HAL: registered {} ({})", path, driver.name());
        registry.devices.push(HalDevice {
            path: String::from(path),
            driver,
            pci,
        });
    }
    drop(hal);

    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_DEVICE_ATTACHED, driver_type, 0, 0, 0,
    ));
}

/// Read from a device by path
pub fn device_read(path: &str, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
    let hal = HAL.lock();
    if let Some(registry) = hal.as_ref() {
        if let Some(dev) = registry.devices.iter().find(|d| d.path == path) {
            return dev.driver.read(offset, buf);
        }
    }
    Err(DriverError::NoDevice)
}

/// Write to a device by path
pub fn device_write(path: &str, offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
    let hal = HAL.lock();
    if let Some(registry) = hal.as_ref() {
        if let Some(dev) = registry.devices.iter().find(|d| d.path == path) {
            return dev.driver.write(offset, buf);
        }
    }
    Err(DriverError::NoDevice)
}

/// Send ioctl to a device by path
pub fn device_ioctl(path: &str, cmd: u32, arg: u32) -> Result<u32, DriverError> {
    let mut hal = HAL.lock();
    if let Some(registry) = hal.as_mut() {
        if let Some(dev) = registry.devices.iter_mut().find(|d| d.path == path) {
            return dev.driver.ioctl(cmd, arg);
        }
    }
    Err(DriverError::NoDevice)
}

/// Send ioctl to the first device matching a given DriverType.
pub fn device_ioctl_by_type(dtype: DriverType, cmd: u32, arg: u32) -> Result<u32, DriverError> {
    let mut hal = HAL.lock();
    if let Some(registry) = hal.as_mut() {
        if let Some(dev) = registry.devices.iter_mut().find(|d| d.driver.driver_type() == dtype) {
            return dev.driver.ioctl(cmd, arg);
        }
    }
    Err(DriverError::NoDevice)
}

/// List all registered device paths
pub fn list_devices() -> Vec<(String, String, DriverType)> {
    let hal = HAL.lock();
    let mut result = Vec::new();
    if let Some(registry) = hal.as_ref() {
        for dev in &registry.devices {
            result.push((dev.path.clone(), String::from(dev.driver.name()), dev.driver.driver_type()));
        }
    }
    result
}

/// Count devices of a given type
pub fn count_by_type(dtype: DriverType) -> usize {
    let hal = HAL.lock();
    if let Some(registry) = hal.as_ref() {
        registry.devices.iter().filter(|d| d.driver.driver_type() == dtype).count()
    } else {
        0
    }
}

/// Return all PCI devices that already have bound drivers.
pub fn bound_pci_devices() -> Vec<PciDevice> {
    let hal = HAL.lock();
    let mut result = Vec::new();
    if let Some(registry) = hal.as_ref() {
        for dev in &registry.devices {
            if let Some(ref pci) = dev.pci {
                result.push(pci.clone());
            }
        }
    }
    result
}

/// Print all registered devices to serial
pub fn print_devices() {
    let devices = list_devices();
    crate::serial_println!("  HAL: {} device(s) registered:", devices.len());
    for (path, name, dtype) in &devices {
        crate::serial_println!("    {} - {} ({:?})", path, name, dtype);
    }
}

// ──────────────────────────────────────────────
// Device path generation
// ──────────────────────────────────────────────

/// Auto-generate a device path from driver type and index.
pub fn make_device_path(dtype: DriverType, index: usize) -> String {
    let prefix = match dtype {
        DriverType::Block => "/dev/blk",
        DriverType::Network => "/dev/net",
        DriverType::Display => "/dev/fb",
        DriverType::Audio => "/dev/audio",
        DriverType::Char => "/dev/char",
        DriverType::Bus => "/dev/bus",
        DriverType::Input => "/dev/input",
        DriverType::Output => "/dev/output",
        DriverType::Sensor => "/dev/sensor",
        DriverType::Unknown => "/dev/misc",
    };
    let mut path = String::from(prefix);
    if index < 10 {
        path.push((b'0' + index as u8) as char);
    } else {
        path.push((b'0' + (index / 10) as u8) as char);
        path.push((b'0' + (index % 10) as u8) as char);
    }
    path
}

// ──────────────────────────────────────────────
// PCI probe (delegates to pci_drivers table)
// ──────────────────────────────────────────────

/// Probe all PCI devices and bind matching drivers.
/// Skips bridges (class 0x06) since they don't need a user-facing driver.
pub fn probe_and_bind_all() {
    use crate::drivers::pci_drivers::{PCI_DRIVER_TABLE, matches_pci};

    let pci_devices = crate::drivers::pci::devices();
    let mut bound = 0u32;
    let mut type_counters = [0usize; 10]; // indexed by DriverType discriminant (10 variants)

    crate::serial_println!("  HAL: Probing {} PCI device(s) for drivers...", pci_devices.len());

    for pci_dev in &pci_devices {
        // Skip bridges — they don't need a user-facing driver
        if pci_dev.class_code == 0x06 {
            continue;
        }

        // Find best matching driver entry (highest specificity wins)
        let mut best: Option<&crate::drivers::pci_drivers::PciDriverEntry> = None;
        for entry in PCI_DRIVER_TABLE {
            if matches_pci(&entry.match_rule, pci_dev) {
                if best.is_none() || entry.specificity > best.unwrap().specificity {
                    best = Some(entry);
                }
            }
        }

        if let Some(entry) = best {
            if let Some(mut driver) = (entry.factory)(pci_dev) {
                let dtype = driver.driver_type();
                let type_idx = dtype as usize;
                let dev_index = type_counters[type_idx];
                type_counters[type_idx] += 1;

                let path = make_device_path(dtype, dev_index);

                // Initialize the driver
                if let Err(e) = driver.init() {
                    crate::serial_println!(
                        "  HAL: WARN - driver '{}' init failed: {:?}",
                        driver.name(), e
                    );
                }

                register_device(&path, driver, Some(pci_dev.clone()));
                bound += 1;
            }
        } else {
            crate::debug_println!(
                "  HAL: no driver for PCI {:02x}:{:02x}.{} ({:04x}:{:04x} class {:02x}:{:02x})",
                pci_dev.bus, pci_dev.device, pci_dev.function,
                pci_dev.vendor_id, pci_dev.device_id,
                pci_dev.class_code, pci_dev.subclass
            );
        }
    }

    crate::serial_println!("  HAL: Bound {} PCI driver(s)", bound);
}

// ──────────────────────────────────────────────
// Legacy (non-PCI) device drivers
// ──────────────────────────────────────────────

struct Ps2KeyboardDriver;

impl Driver for Ps2KeyboardDriver {
    fn name(&self) -> &str { "PS/2 Keyboard" }
    fn driver_type(&self) -> DriverType { DriverType::Input }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

struct Ps2MouseDriver;

impl Driver for Ps2MouseDriver {
    fn name(&self) -> &str { "PS/2 Mouse" }
    fn driver_type(&self) -> DriverType { DriverType::Input }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

struct SerialDriver;

impl Driver for SerialDriver {
    fn name(&self) -> &str { "Serial Port (COM1)" }
    fn driver_type(&self) -> DriverType { DriverType::Char }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        for &b in buf {
            crate::drivers::serial::write_byte(b);
        }
        Ok(buf.len())
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

/// ATAPI CD-ROM/DVD-ROM driver wrapper
struct AtapiDriver;

impl Driver for AtapiDriver {
    fn name(&self) -> &str { "ATAPI CD/DVD-ROM" }
    fn driver_type(&self) -> DriverType { DriverType::Block }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        let lba = offset / 2048;
        let blocks = (buf.len() + 2047) / 2048;
        if crate::drivers::storage::atapi::read_sectors(lba as u32, blocks as u32, buf) {
            Ok(blocks * 2048)
        } else {
            Err(DriverError::IoError)
        }
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported) // Read-only
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

/// Register legacy (non-PCI) devices that are always present on x86
pub fn register_legacy_devices() {
    register_device("/dev/kbd", Box::new(Ps2KeyboardDriver), None);
    register_device("/dev/mouse", Box::new(Ps2MouseDriver), None);
    register_device("/dev/ttyS0", Box::new(SerialDriver), None);

    // Register ATAPI CD/DVD-ROM if detected
    if crate::drivers::storage::atapi::is_present() {
        register_device("/dev/cdrom0", Box::new(AtapiDriver), None);
    }
}
