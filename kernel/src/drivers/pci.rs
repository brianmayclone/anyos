//! PCI bus enumeration and configuration space access.
//!
//! Scans all 256 PCI buses to discover devices, reads their BARs and capabilities,
//! and provides lookup functions by class/subclass or vendor/device ID.

use crate::arch::x86::port::{inl, outl};
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

/// PCI configuration address port (write bus/device/function/register here).
const PCI_CONFIG_ADDRESS: u16 = 0x0CF8;
/// PCI configuration data port (read/write the selected register here).
const PCI_CONFIG_DATA: u16 = 0x0CFC;

/// A discovered PCI device
#[derive(Debug, Clone)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision_id: u8,
    pub header_type: u8,
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
    pub bars: [u32; 6],
}

/// Global list of all discovered PCI devices, populated by [`scan_all`].
static PCI_DEVICES: Spinlock<Vec<PciDevice>> = Spinlock::new(Vec::new());

/// Build PCI config address: enable bit | bus | device | function | register (dword-aligned)
fn pci_config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | ((offset as u32) & 0xFC)
}

/// Read a 32-bit value from PCI configuration space.
pub fn pci_config_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let address = pci_config_address(bus, device, function, offset);
    unsafe {
        outl(PCI_CONFIG_ADDRESS, address);
        inl(PCI_CONFIG_DATA)
    }
}

/// Read a 16-bit value from PCI configuration space.
pub fn pci_config_read16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let dword = pci_config_read32(bus, device, function, offset & 0xFC);
    ((dword >> ((offset & 2) * 8)) & 0xFFFF) as u16
}

/// Read an 8-bit value from PCI configuration space.
pub fn pci_config_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let dword = pci_config_read32(bus, device, function, offset & 0xFC);
    ((dword >> ((offset & 3) * 8)) & 0xFF) as u8
}

/// Write a 32-bit value to PCI configuration space.
pub fn pci_config_write32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    let address = pci_config_address(bus, device, function, offset);
    unsafe {
        outl(PCI_CONFIG_ADDRESS, address);
        outl(PCI_CONFIG_DATA, value);
    }
}

fn read_bars(bus: u8, device: u8, function: u8) -> [u32; 6] {
    let mut bars = [0u32; 6];
    for i in 0..6 {
        bars[i] = pci_config_read32(bus, device, function, 0x10 + (i as u8 * 4));
    }
    bars
}

fn probe_function(bus: u8, device: u8, function: u8) -> Option<PciDevice> {
    let vendor_id = pci_config_read16(bus, device, function, 0x00);
    if vendor_id == 0xFFFF {
        return None;
    }

    let device_id = pci_config_read16(bus, device, function, 0x02);
    let revision_id = pci_config_read8(bus, device, function, 0x08);
    let prog_if = pci_config_read8(bus, device, function, 0x09);
    let subclass = pci_config_read8(bus, device, function, 0x0A);
    let class_code = pci_config_read8(bus, device, function, 0x0B);
    let header_type = pci_config_read8(bus, device, function, 0x0E);
    let interrupt_line = pci_config_read8(bus, device, function, 0x3C);
    let interrupt_pin = pci_config_read8(bus, device, function, 0x3D);

    let bars = if header_type & 0x7F == 0x00 {
        read_bars(bus, device, function)
    } else {
        [0u32; 6]
    };

    Some(PciDevice {
        bus,
        device,
        function,
        vendor_id,
        device_id,
        class_code,
        subclass,
        prog_if,
        revision_id,
        header_type,
        interrupt_line,
        interrupt_pin,
        bars,
    })
}

/// Scan all PCI buses and populate the global device list.
pub fn scan_all() {
    let mut devices = PCI_DEVICES.lock();
    devices.clear();

    for bus in 0..=255u16 {
        for device in 0..32u8 {
            let vendor = pci_config_read16(bus as u8, device, 0, 0x00);
            if vendor == 0xFFFF {
                continue;
            }

            if let Some(dev) = probe_function(bus as u8, device, 0) {
                let is_multifunction = dev.header_type & 0x80 != 0;
                devices.push(dev);

                if is_multifunction {
                    for function in 1..8u8 {
                        if let Some(dev) = probe_function(bus as u8, device, function) {
                            devices.push(dev);
                        }
                    }
                }
            }
        }
    }

    crate::serial_println!("[OK] PCI scan: {} device(s) found", devices.len());
}

/// Human-readable name for a PCI class/subclass
pub fn class_name(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (0x00, 0x00) => "Non-VGA unclassified device",
        (0x00, 0x01) => "VGA-compatible unclassified device",
        (0x01, 0x00) => "SCSI bus controller",
        (0x01, 0x01) => "IDE controller",
        (0x01, 0x02) => "Floppy disk controller",
        (0x01, 0x05) => "ATA controller",
        (0x01, 0x06) => "SATA controller",
        (0x01, 0x08) => "NVMe controller",
        (0x02, 0x00) => "Ethernet controller",
        (0x02, 0x80) => "Other network controller",
        (0x03, 0x00) => "VGA-compatible controller",
        (0x03, 0x01) => "XGA controller",
        (0x04, 0x00) => "Multimedia video controller",
        (0x04, 0x01) => "Multimedia audio controller",
        (0x04, 0x03) => "Audio device (HD Audio)",
        (0x05, 0x00) => "RAM controller",
        (0x06, 0x00) => "Host bridge",
        (0x06, 0x01) => "ISA bridge",
        (0x06, 0x04) => "PCI-to-PCI bridge",
        (0x06, 0x80) => "Other bridge",
        (0x07, 0x00) => "Serial controller",
        (0x07, 0x01) => "Parallel controller",
        (0x08, 0x00) => "PIC",
        (0x08, 0x01) => "DMA controller",
        (0x08, 0x02) => "Timer",
        (0x08, 0x03) => "RTC controller",
        (0x0C, 0x00) => "FireWire controller",
        (0x0C, 0x03) => "USB controller",
        (0x0C, 0x05) => "SMBus controller",
        _ => "Unknown device",
    }
}

/// Print all discovered devices to serial output.
pub fn print_devices() {
    let devices = PCI_DEVICES.lock();
    for dev in devices.iter() {
        crate::serial_println!(
            "  PCI {:02x}:{:02x}.{} - {:04x}:{:04x} - {} (class {:02x}:{:02x})",
            dev.bus, dev.device, dev.function,
            dev.vendor_id, dev.device_id,
            class_name(dev.class_code, dev.subclass),
            dev.class_code, dev.subclass
        );

        #[cfg(feature = "debug_verbose")]
        {
            crate::serial_println!(
                "    prog_if={:02x} rev={:02x} hdr={:02x} IRQ={} PIN={}",
                dev.prog_if, dev.revision_id, dev.header_type,
                dev.interrupt_line, dev.interrupt_pin
            );
            for (i, bar) in dev.bars.iter().enumerate() {
                if *bar != 0 {
                    if bar & 1 == 0 {
                        crate::serial_println!("    BAR{}: MMIO {:#010x}", i, bar & 0xFFFFFFF0);
                    } else {
                        crate::serial_println!("    BAR{}: I/O  {:#06x}", i, bar & 0xFFFFFFFC);
                    }
                }
            }
        }
    }
}

/// Get a clone of the device list
pub fn devices() -> Vec<PciDevice> {
    PCI_DEVICES.lock().clone()
}

/// Find a PCI device by class and subclass
pub fn find_by_class(class: u8, subclass: u8) -> Option<PciDevice> {
    PCI_DEVICES.lock()
        .iter()
        .find(|d| d.class_code == class && d.subclass == subclass)
        .cloned()
}

/// Find a PCI device by vendor and device ID
pub fn find_by_id(vendor_id: u16, device_id: u16) -> Option<PciDevice> {
    PCI_DEVICES.lock()
        .iter()
        .find(|d| d.vendor_id == vendor_id && d.device_id == device_id)
        .cloned()
}

/// Enable bus mastering for a PCI device (needed for DMA)
pub fn enable_bus_master(dev: &PciDevice) {
    let cmd = pci_config_read16(dev.bus, dev.device, dev.function, 0x04);
    pci_config_write32(dev.bus, dev.device, dev.function, 0x04, (cmd | 0x04) as u32);
}
