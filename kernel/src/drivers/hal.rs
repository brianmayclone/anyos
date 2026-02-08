/// Hardware Abstraction Layer (HAL)
/// Provides a unified interface for device drivers via traits and a central registry.
/// Includes automatic PCI-to-driver matching for device detection at boot.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;
use crate::drivers::pci::PciDevice;

// ──────────────────────────────────────────────
// Driver trait + types
// ──────────────────────────────────────────────

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

// Display ioctls (0x0100-0x01FF)
pub const IOCTL_DISPLAY_GET_MODE: u32 = 0x0100;      // Returns w|(h<<16)
pub const IOCTL_DISPLAY_FLIP: u32 = 0x0101;          // Flip double buffer
pub const IOCTL_DISPLAY_IS_DBLBUF: u32 = 0x0102;     // Returns 1 if double-buffered
pub const IOCTL_DISPLAY_GET_PITCH: u32 = 0x0103;      // Returns pitch in bytes
pub const IOCTL_DISPLAY_SET_MODE: u32 = 0x0104;       // arg = w | (h << 16)
pub const IOCTL_DISPLAY_LIST_MODES: u32 = 0x0105;     // Returns count of supported modes
pub const IOCTL_DISPLAY_HAS_ACCEL: u32 = 0x0106;      // Returns 1 if 2D accel available
pub const IOCTL_DISPLAY_HAS_HW_CURSOR: u32 = 0x0107;  // Returns 1 if HW cursor available

// Audio ioctls (0x0200-0x02FF)
pub const IOCTL_AUDIO_GET_SAMPLE_RATE: u32 = 0x0200;
pub const IOCTL_AUDIO_SET_VOLUME: u32 = 0x0201;
pub const IOCTL_AUDIO_GET_VOLUME: u32 = 0x0202;

// Network ioctls (0x0300-0x03FF)
pub const IOCTL_NET_GET_MAC: u32 = 0x0300;
pub const IOCTL_NET_GET_LINK: u32 = 0x0301;

// Sensor ioctls (0x0400-0x04FF)
pub const IOCTL_SENSOR_READ: u32 = 0x0400;
pub const IOCTL_SENSOR_GET_TYPE: u32 = 0x0401;

// Output ioctls (0x0500-0x05FF)
pub const IOCTL_OUTPUT_STATUS: u32 = 0x0500;
pub const IOCTL_OUTPUT_FLUSH: u32 = 0x0501;

pub trait Driver: Send {
    fn name(&self) -> &str;
    fn driver_type(&self) -> DriverType;
    fn init(&mut self) -> Result<(), DriverError>;
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError>;
    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, DriverError>;
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

/// SATA controller stub
struct SataDriver {
    _pci: PciDevice,
}

impl Driver for SataDriver {
    fn name(&self) -> &str { "SATA Controller (stub)" }
    fn driver_type(&self) -> DriverType { DriverType::Block }
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
    _pci: PciDevice,
}

impl Driver for AudioDriver {
    fn name(&self) -> &str { "Audio Controller (stub)" }
    fn driver_type(&self) -> DriverType { DriverType::Audio }
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

/// USB controller stub
struct UsbDriver {
    _pci: PciDevice,
}

impl Driver for UsbDriver {
    fn name(&self) -> &str { "USB Controller (stub)" }
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
    // Class-based matches
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x01, subclass: 0x01 },
        factory: |pci| Some(Box::new(IdeDriver { pci: pci.clone() })),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x01, subclass: 0x06 },
        factory: |pci| Some(Box::new(SataDriver { _pci: pci.clone() })),
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
        factory: |pci| Some(Box::new(AudioDriver { _pci: pci.clone() })),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x04, subclass: 0x03 },
        factory: |pci| Some(Box::new(AudioDriver { _pci: pci.clone() })),
        specificity: 1,
    },
    PciDriverEntry {
        match_rule: PciMatch::Class { class: 0x0C, subclass: 0x03 },
        factory: |pci| Some(Box::new(UsbDriver { _pci: pci.clone() })),
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
    let mut type_counters = [0usize; 8]; // indexed by DriverType discriminant

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
