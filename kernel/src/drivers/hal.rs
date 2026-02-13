//! Hardware Abstraction Layer (HAL).
//!
//! Provides a unified [`Driver`] trait and a central device registry. Includes automatic
//! PCI-to-driver matching for device detection at boot, plus legacy device registration.

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

/// Print all registered devices to serial
pub fn print_devices() {
    let devices = list_devices();
    crate::serial_println!("  HAL: {} device(s) registered:", devices.len());
    for (path, name, dtype) in &devices {
        crate::serial_println!("    {} - {} ({:?})", path, name, dtype);
    }
}

// ──────────────────────────────────────────────
// PCI-to-Driver Matching
// ──────────────────────────────────────────────

enum PciMatch {
    Class { class: u8, subclass: u8 },
    VendorDevice { vendor: u16, device: u16 },
}

struct PciDriverEntry {
    match_rule: PciMatch,
    factory: fn(&PciDevice) -> Option<Box<dyn Driver>>,
    /// Higher = more specific match (vendor/device beats class)
    specificity: u8,
}

/// Auto-generate a device path from driver type and index
fn make_device_path(dtype: DriverType, index: usize) -> String {
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
    // Simple number-to-string without format! to avoid pulling in too much
    if index < 10 {
        path.push((b'0' + index as u8) as char);
    } else {
        path.push((b'0' + (index / 10) as u8) as char);
        path.push((b'0' + (index % 10) as u8) as char);
    }
    path
}

// ──────────────────────────────────────────────
// Stub Drivers
// ──────────────────────────────────────────────

/// GPU display driver (wraps gpu::GpuDriver trait)
/// Used for both Bochs VGA and VMware SVGA II — routes ioctls through gpu::with_gpu()
struct GpuDisplayDriver {
    _pci: PciDevice,
    driver_name: &'static str,
}

impl Driver for GpuDisplayDriver {
    fn name(&self) -> &str { self.driver_name }
    fn driver_type(&self) -> DriverType { DriverType::Display }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn ioctl(&mut self, cmd: u32, arg: u32) -> Result<u32, DriverError> {
        match cmd {
            IOCTL_DISPLAY_GET_MODE => {
                crate::drivers::gpu::with_gpu(|g| {
                    let (w, h, _, _) = g.get_mode();
                    Ok(w | (h << 16))
                }).unwrap_or(Err(DriverError::NotInitialized))
            }
            IOCTL_DISPLAY_FLIP => {
                crate::drivers::gpu::with_gpu(|g| g.flip());
                Ok(0)
            }
            IOCTL_DISPLAY_IS_DBLBUF => {
                Ok(crate::drivers::gpu::with_gpu(|g| g.has_double_buffer() as u32).unwrap_or(0))
            }
            IOCTL_DISPLAY_GET_PITCH => {
                crate::drivers::gpu::with_gpu(|g| {
                    let (_, _, pitch, _) = g.get_mode();
                    Ok(pitch)
                }).unwrap_or(Err(DriverError::NotInitialized))
            }
            IOCTL_DISPLAY_SET_MODE => {
                let w = arg & 0xFFFF;
                let h = (arg >> 16) & 0xFFFF;
                crate::drivers::gpu::with_gpu(|g| {
                    match g.set_mode(w, h, 32) {
                        Some(_) => Ok(0u32),
                        None => Err(DriverError::NotSupported),
                    }
                }).unwrap_or(Err(DriverError::NotInitialized))
            }
            IOCTL_DISPLAY_LIST_MODES => {
                Ok(crate::drivers::gpu::with_gpu(|g| g.supported_modes().len() as u32).unwrap_or(0))
            }
            IOCTL_DISPLAY_HAS_ACCEL => {
                Ok(crate::drivers::gpu::with_gpu(|g| g.has_accel() as u32).unwrap_or(0))
            }
            IOCTL_DISPLAY_HAS_HW_CURSOR => {
                Ok(crate::drivers::gpu::with_gpu(|g| g.has_hw_cursor() as u32).unwrap_or(0))
            }
            _ => Err(DriverError::NotSupported),
        }
    }
}

/// Generic VGA controller stub
struct GenericVgaDriver {
    pci: PciDevice,
}

impl Driver for GenericVgaDriver {
    fn name(&self) -> &str { "Generic VGA" }
    fn driver_type(&self) -> DriverType { DriverType::Display }
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

/// IDE controller driver (wraps existing ata.rs)
struct IdeDriver {
    pci: PciDevice,
}

impl Driver for IdeDriver {
    fn name(&self) -> &str { "IDE Controller" }
    fn driver_type(&self) -> DriverType { DriverType::Block }
    fn init(&mut self) -> Result<(), DriverError> {
        // Already initialized via ata::init() at boot
        Ok(())
    }
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        let lba = offset / 512;
        let sectors = (buf.len() + 511) / 512;
        if sectors > 255 { return Err(DriverError::InvalidArgument); }
        if crate::drivers::storage::ata::read_sectors(lba as u32, sectors as u8, buf) {
            Ok(sectors * 512)
        } else {
            Err(DriverError::IoError)
        }
    }
    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        let lba = offset / 512;
        let sectors = (buf.len() + 511) / 512;
        if sectors > 255 { return Err(DriverError::InvalidArgument); }
        if crate::drivers::storage::ata::write_sectors(lba as u32, sectors as u8, buf) {
            Ok(sectors * 512)
        } else {
            Err(DriverError::IoError)
        }
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

/// AHCI/SATA controller driver (wraps ahci.rs DMA backend)
struct SataDriver {
    _pci: PciDevice,
}

impl Driver for SataDriver {
    fn name(&self) -> &str { "AHCI SATA Controller" }
    fn driver_type(&self) -> DriverType { DriverType::Block }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        let lba = offset / 512;
        let sectors = (buf.len() + 511) / 512;
        if crate::drivers::storage::read_sectors(lba as u32, sectors as u32, buf) {
            Ok(sectors * 512)
        } else {
            Err(DriverError::IoError)
        }
    }
    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        let lba = offset / 512;
        let sectors = (buf.len() + 511) / 512;
        if crate::drivers::storage::write_sectors(lba as u32, sectors as u32, buf) {
            Ok(sectors * 512)
        } else {
            Err(DriverError::IoError)
        }
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

/// Ethernet controller driver (wraps e1000.rs)
struct EthernetDriver {
    _pci: PciDevice,
}

impl Driver for EthernetDriver {
    fn name(&self) -> &str { "Intel E1000 Ethernet" }
    fn driver_type(&self) -> DriverType { DriverType::Network }
    fn init(&mut self) -> Result<(), DriverError> {
        // Actual init is done via e1000::init() in main.rs
        Ok(())
    }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        if crate::drivers::network::e1000::transmit(buf) {
            Ok(buf.len())
        } else {
            Err(DriverError::IoError)
        }
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

/// Audio controller stub
struct AudioDriver {
    pci: PciDevice,
    initialized: bool,
}

impl Driver for AudioDriver {
    fn name(&self) -> &str {
        if self.initialized { "Intel AC'97 Audio" } else { "Audio Controller (no device)" }
    }
    fn driver_type(&self) -> DriverType { DriverType::Audio }
    fn init(&mut self) -> Result<(), DriverError> {
        crate::drivers::audio::ac97::init_from_pci(&self.pci);
        self.initialized = crate::drivers::audio::ac97::is_available();
        Ok(())
    }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        if !self.initialized { return Err(DriverError::NotSupported); }
        Ok(crate::drivers::audio::write_pcm(buf))
    }
    fn ioctl(&mut self, cmd: u32, arg: u32) -> Result<u32, DriverError> {
        if !self.initialized { return Err(DriverError::NotSupported); }
        match cmd {
            IOCTL_AUDIO_GET_SAMPLE_RATE => Ok(48000),
            IOCTL_AUDIO_SET_VOLUME => {
                crate::drivers::audio::set_volume(arg as u8);
                Ok(0)
            }
            IOCTL_AUDIO_GET_VOLUME => Ok(crate::drivers::audio::get_volume() as u32),
            _ => Err(DriverError::NotSupported),
        }
    }
}

/// USB host controller driver — dispatches to UHCI/EHCI based on prog_if.
struct UsbControllerDriver {
    _pci: PciDevice,
    controller_name: &'static str,
}

impl Driver for UsbControllerDriver {
    fn name(&self) -> &str { self.controller_name }
    fn driver_type(&self) -> DriverType { DriverType::Bus }
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

/// SMBus controller stub
struct SmbusDriver {
    _pci: PciDevice,
}

impl Driver for SmbusDriver {
    fn name(&self) -> &str { "SMBus Controller (stub)" }
    fn driver_type(&self) -> DriverType { DriverType::Bus }
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

// ──────────────────────────────────────────────
// Legacy (non-PCI) device drivers
// ──────────────────────────────────────────────

/// PS/2 Keyboard driver wrapper
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

/// PS/2 Mouse driver wrapper
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

/// Serial port driver wrapper
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

// ──────────────────────────────────────────────
// PCI driver table + probe
// ──────────────────────────────────────────────

static PCI_DRIVER_TABLE: &[PciDriverEntry] = &[
    // Most specific: vendor/device match
    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x1234, device: 0x1111 },
        factory: |pci| Some(Box::new(GpuDisplayDriver { _pci: pci.clone(), driver_name: "Bochs/QEMU VGA" })),
        specificity: 2,
    },
    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x15AD, device: 0x0405 },
        factory: |pci| {
            // Initialize VMware SVGA II GPU driver
            crate::drivers::gpu::vmware_svga::init_and_register(pci);
            Some(Box::new(GpuDisplayDriver { _pci: pci.clone(), driver_name: "VMware SVGA II" }))
        },
        specificity: 2,
    },
    PciDriverEntry {
        match_rule: PciMatch::VendorDevice { vendor: 0x1AF4, device: 0x1050 },
        factory: |pci| {
            // Initialize VirtIO GPU driver
            crate::drivers::gpu::virtio_gpu::init_and_register(pci);
            Some(Box::new(GpuDisplayDriver { _pci: pci.clone(), driver_name: "VirtIO GPU" }))
        },
        specificity: 2,
    },
    // Class-based matches
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x01, subclass: 0x01 },
        factory: |pci| Some(Box::new(IdeDriver { pci: pci.clone() })),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x01, subclass: 0x06 },
        factory: |pci| {
            // Initialize AHCI SATA controller
            crate::drivers::storage::ahci::init_and_register(pci);
            Some(Box::new(SataDriver { _pci: pci.clone() }))
        },
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x02, subclass: 0x00 },
        factory: |pci| Some(Box::new(EthernetDriver { _pci: pci.clone() })),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x03, subclass: 0x00 },
        factory: |pci| Some(Box::new(GenericVgaDriver { pci: pci.clone() })),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x04, subclass: 0x01 },
        factory: |pci| Some(Box::new(AudioDriver { pci: pci.clone(), initialized: false })),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x04, subclass: 0x03 },
        factory: |pci| Some(Box::new(AudioDriver { pci: pci.clone(), initialized: false })),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x0C, subclass: 0x03 },
        factory: |pci| {
            crate::drivers::usb::init(pci);
            let name = match pci.prog_if {
                0x00 => "UHCI USB 1.1",
                0x20 => "EHCI USB 2.0",
                0x10 => "OHCI USB 1.1",
                0x30 => "xHCI USB 3.0",
                _ => "USB Controller",
            };
            Some(Box::new(UsbControllerDriver { _pci: pci.clone(), controller_name: name }))
        },
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x0C, subclass: 0x05 },
        factory: |pci| Some(Box::new(SmbusDriver { _pci: pci.clone() })),
        specificity: 1,
    },
];

fn matches_pci(rule: &PciMatch, dev: &PciDevice) -> bool {
    match rule {
        PciMatch::Class { class, subclass } => {
            dev.class_code == *class && dev.subclass == *subclass
        }
        PciMatch::VendorDevice { vendor, device } => {
            dev.vendor_id == *vendor && dev.device_id == *device
        }
    }
}

/// Probe all PCI devices and bind matching drivers.
/// Skips bridges (class 0x06) since they don't need a user-facing driver.
pub fn probe_and_bind_all() {
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
        let mut best: Option<&PciDriverEntry> = None;
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

/// Register legacy (non-PCI) devices that are always present on x86
pub fn register_legacy_devices() {
    register_device("/dev/kbd", Box::new(Ps2KeyboardDriver), None);
    register_device("/dev/mouse", Box::new(Ps2MouseDriver), None);
    register_device("/dev/ttyS0", Box::new(SerialDriver), None);
}
