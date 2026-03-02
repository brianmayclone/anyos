//! PCI configuration space and system bus emulation.
//!
//! Emulates the PCI Type 1 configuration mechanism used on x86 PCs.
//! The guest writes a 32-bit address to the Configuration Address port
//! (0xCF8) specifying bus, device, function, and register, then reads
//! or writes the Configuration Data port (0xCFC) to access the selected
//! register in the device's 256-byte configuration space.
//!
//! # I/O Ports
//!
//! | Port | Description |
//! |------|-------------|
//! | 0xCF8 | PCI Configuration Address (32-bit write) |
//! | 0xCFC-0xCFF | PCI Configuration Data (32-bit read/write) |
//!
//! # Configuration Address Format (port 0xCF8)
//!
//! ```text
//! Bit 31    : Enable bit (must be 1 for a valid access)
//! Bits 23:16: Bus number (0-255)
//! Bits 15:11: Device number (0-31)
//! Bits 10:8 : Function number (0-7)
//! Bits 7:2  : Register offset (dword-aligned)
//! Bits 1:0  : Must be 0
//! ```
//!
//! # PCI Configuration Space Header (Type 0)
//!
//! | Offset | Size | Field |
//! |--------|------|-------|
//! | 0x00 | 2 | Vendor ID |
//! | 0x02 | 2 | Device ID |
//! | 0x04 | 2 | Command |
//! | 0x06 | 2 | Status |
//! | 0x08 | 1 | Revision ID |
//! | 0x09 | 3 | Class Code (prog IF, subclass, class) |
//! | 0x10-0x27 | 24 | BAR0-BAR5 |
//! | 0x2C | 2 | Subsystem Vendor ID |
//! | 0x2E | 2 | Subsystem Device ID |
//! | 0x3C | 1 | Interrupt Line |
//! | 0x3D | 1 | Interrupt Pin |

use alloc::vec::Vec;
use crate::error::Result;
use crate::io::IoHandler;

/// A single PCI device with a 256-byte configuration space (header type 0).
#[derive(Debug, Clone)]
pub struct PciDevice {
    /// PCI bus number (0-255).
    pub bus: u8,
    /// PCI device number (0-31).
    pub device: u8,
    /// PCI function number (0-7).
    pub function: u8,
    /// 256-byte PCI configuration space.
    pub config_space: [u8; 256],
    /// BAR size masks for each of the 6 BARs. When the guest writes
    /// all-ones to a BAR, the device returns the size mask so the guest
    /// can determine the BAR's required address space size.
    bar_sizes: [u32; 6],
}

impl PciDevice {
    /// Create a new PCI device with the specified identity.
    ///
    /// # Arguments
    /// - `vendor_id`: PCI vendor ID (e.g., 0x8086 for Intel)
    /// - `device_id`: PCI device ID
    /// - `class`: PCI class code (e.g., 0x02 for network controller)
    /// - `subclass`: PCI subclass code
    /// - `prog_if`: programming interface byte
    pub fn new(vendor_id: u16, device_id: u16, class: u8, subclass: u8, prog_if: u8) -> Self {
        let mut config_space = [0u8; 256];

        // Vendor ID (offset 0x00).
        config_space[0x00] = vendor_id as u8;
        config_space[0x01] = (vendor_id >> 8) as u8;

        // Device ID (offset 0x02).
        config_space[0x02] = device_id as u8;
        config_space[0x03] = (device_id >> 8) as u8;

        // Status (offset 0x06): capabilities list not supported.
        config_space[0x06] = 0x00;
        config_space[0x07] = 0x00;

        // Revision ID (offset 0x08).
        config_space[0x08] = 0x01;

        // Class code (offset 0x09-0x0B): prog_if, subclass, class.
        config_space[0x09] = prog_if;
        config_space[0x0A] = subclass;
        config_space[0x0B] = class;

        // Header type (offset 0x0E): type 0 (general device).
        config_space[0x0E] = 0x00;

        PciDevice {
            bus: 0,
            device: 0,
            function: 0,
            config_space,
            bar_sizes: [0; 6],
        }
    }

    /// Configure a Base Address Register (BAR).
    ///
    /// # Arguments
    /// - `bar_index`: BAR number (0-5)
    /// - `address`: the base address to program into the BAR
    /// - `size`: the size of the address region (must be a power of 2)
    /// - `is_mmio`: `true` for memory-mapped BAR, `false` for I/O BAR
    pub fn set_bar(&mut self, bar_index: usize, address: u32, size: u32, is_mmio: bool) {
        if bar_index >= 6 {
            return;
        }

        let offset = 0x10 + bar_index * 4;
        let bar_value = if is_mmio {
            address & 0xFFFFFFF0 // bit 0 = 0 for MMIO
        } else {
            (address & 0xFFFFFFFC) | 0x01 // bit 0 = 1 for I/O
        };

        config_write_u32(&mut self.config_space, offset, bar_value);

        // Store the size mask: a BAR of size N returns ~(N-1) when
        // written with all-ones (preserving the type bits).
        if size > 0 {
            let mask = !(size - 1);
            self.bar_sizes[bar_index] = if is_mmio {
                mask & 0xFFFFFFF0
            } else {
                (mask & 0xFFFFFFFC) | 0x01
            };
        }
    }

    /// Set the interrupt line and pin.
    ///
    /// - `line`: interrupt line (IRQ number, 0-255)
    /// - `pin`: interrupt pin (1=INTA, 2=INTB, 3=INTC, 4=INTD; 0=none)
    pub fn set_interrupt(&mut self, line: u8, pin: u8) {
        self.config_space[0x3C] = line;
        self.config_space[0x3D] = pin;
    }

    /// Set the subsystem vendor ID and subsystem device ID.
    pub fn set_subsystem(&mut self, vendor_id: u16, device_id: u16) {
        config_write_u16(&mut self.config_space, 0x2C, vendor_id);
        config_write_u16(&mut self.config_space, 0x2E, device_id);
    }
}

/// PCI system bus holding registered devices.
#[derive(Debug)]
pub struct PciBus {
    /// Last address written to the Configuration Address port (0xCF8).
    pub config_address: u32,
    /// Registered PCI devices.
    pub devices: Vec<PciDevice>,
    /// Diagnostic: number of config address writes logged.
    log_count: u32,
    /// Diagnostic: number of config data reads logged.
    read_log_count: u32,
}

impl PciBus {
    /// Create a new empty PCI bus with no devices.
    pub fn new() -> Self {
        PciBus {
            config_address: 0,
            devices: Vec::new(),
            log_count: 0,
            read_log_count: 0,
        }
    }

    /// Register a PCI device on this bus.
    ///
    /// The device's `bus`, `device`, and `function` fields must be set
    /// before calling this method and must not collide with an existing
    /// device.
    pub fn add_device(&mut self, pci_device: PciDevice) {
        self.devices.push(pci_device);
    }

    /// Find the device matching the bus/device/function from the current
    /// config address.
    fn find_device(&mut self, bus: u8, device: u8, function: u8) -> Option<&mut PciDevice> {
        self.devices.iter_mut().find(|d| {
            d.bus == bus && d.device == device && d.function == function
        })
    }

    /// Read a dword from PCI configuration space for the currently
    /// addressed device.
    fn config_read(&mut self) -> u32 {
        if self.config_address & 0x80000000 == 0 {
            // Enable bit not set — return all-ones (no device).
            return 0xFFFFFFFF;
        }

        let bus = ((self.config_address >> 16) & 0xFF) as u8;
        let device = ((self.config_address >> 11) & 0x1F) as u8;
        let function = ((self.config_address >> 8) & 0x07) as u8;
        let register = (self.config_address & 0xFC) as usize;

        let result = if let Some(dev) = self.find_device(bus, device, function) {
            if register + 3 < dev.config_space.len() {
                config_read_u32(&dev.config_space, register)
            } else {
                0xFFFFFFFF
            }
        } else {
            0xFFFFFFFF
        };

        // Log config reads for devices that exist or for device 2 (VGA).
        if self.read_log_count < 60 && (result != 0xFFFFFFFF || device == 2) {
            self.read_log_count += 1;
            libsyscall::serial_print(format_args!(
                "[pci] CFC read: addr=0x{:08X} bus={} dev={} func={} reg=0x{:02X} => 0x{:08X}\n",
                self.config_address, bus, device, function, register, result
            ));
        }

        result
    }

    /// Write a dword to PCI configuration space for the currently
    /// addressed device.
    fn config_write(&mut self, val: u32) {
        if self.config_address & 0x80000000 == 0 {
            return;
        }

        let bus = ((self.config_address >> 16) & 0xFF) as u8;
        let device_num = ((self.config_address >> 11) & 0x1F) as u8;
        let function = ((self.config_address >> 8) & 0x07) as u8;
        let register = (self.config_address & 0xFC) as usize;

        if let Some(dev) = self.find_device(bus, device_num, function) {
            // Handle BAR writes: when the guest writes 0xFFFFFFFF to a BAR,
            // it is probing the BAR size. Return the size mask instead.
            if register >= 0x10 && register <= 0x24 {
                let bar_index = (register - 0x10) / 4;
                if bar_index < 6 {
                    if val == 0xFFFFFFFF {
                        // Size probe — store the size mask.
                        config_write_u32(&mut dev.config_space, register, dev.bar_sizes[bar_index]);
                        return;
                    }
                    // Normal BAR write: preserve type bits (bit 0 for I/O,
                    // bits 0-3 for MMIO).
                    let type_bits = dev.config_space[register] & 0x0F;
                    let new_val = (val & 0xFFFFFFF0) | (type_bits as u32);
                    config_write_u32(&mut dev.config_space, register, new_val);
                    return;
                }
            }

            // General config space write.
            if register + 3 < dev.config_space.len() {
                // Some fields are read-only (vendor/device ID, class code, etc.).
                // For simplicity, allow writes to all offsets except identity fields.
                match register {
                    0x00..=0x03 => { /* Vendor/Device ID: read-only */ }
                    0x08..=0x0B => { /* Revision/Class: read-only */ }
                    _ => {
                        config_write_u32(&mut dev.config_space, register, val);
                    }
                }
            }
        }
    }
}

impl IoHandler for PciBus {
    /// Read from PCI bus I/O ports.
    ///
    /// - 0xCF8: returns the last config address written
    /// - 0xCFC-0xCFF: reads from PCI configuration data, supports
    ///   byte and word sub-accesses
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        let val = match port {
            0xCF8 => self.config_address,
            0xCFC..=0xCFF => {
                let dword = self.config_read();
                let byte_offset = (port - 0xCFC) as u32;
                let shifted = dword >> (byte_offset * 8);
                match size {
                    1 => shifted & 0xFF,
                    2 => shifted & 0xFFFF,
                    _ => shifted,
                }
            }
            _ => 0xFFFFFFFF,
        };
        Ok(val)
    }

    /// Write to PCI bus I/O ports.
    ///
    /// - 0xCF8: stores the configuration address
    /// - 0xCFC-0xCFF: writes to PCI configuration data, supports
    ///   byte and word sub-accesses
    fn write(&mut self, port: u16, size: u8, val: u32) -> Result<()> {
        match port {
            0xCF8 => {
                // Log the first few config address writes to diagnose PCI scanning.
                if self.log_count < 100 {
                    self.log_count += 1;
                    let enable = val & 0x80000000 != 0;
                    let bus = (val >> 16) & 0xFF;
                    let dev = (val >> 11) & 0x1F;
                    let func = (val >> 8) & 0x07;
                    let reg = val & 0xFC;
                    libsyscall::serial_print(format_args!(
                        "[pci] CF8 write: val=0x{:08X} size={} enable={} bus={} dev={} func={} reg=0x{:02X}\n",
                        val, size, enable, bus, dev, func, reg
                    ));
                }
                self.config_address = val;
            }
            0xCFC..=0xCFF => {
                // Read-modify-write for sub-dword accesses.
                let current = self.config_read();
                let byte_offset = (port - 0xCFC) as u32;
                let mask = match size {
                    1 => 0xFFu32,
                    2 => 0xFFFFu32,
                    _ => 0xFFFF_FFFFu32,
                };
                let shifted_mask = mask << (byte_offset * 8);
                let shifted_val = (val & mask) << (byte_offset * 8);
                let new_val = (current & !shifted_mask) | shifted_val;
                self.config_write(new_val);
            }
            _ => {}
        }
        Ok(())
    }
}

// ── Helper functions for little-endian config space access ──

/// Read a little-endian u32 from a byte array at the given offset.
#[inline]
fn config_read_u32(data: &[u8], offset: usize) -> u32 {
    (data[offset] as u32)
        | ((data[offset + 1] as u32) << 8)
        | ((data[offset + 2] as u32) << 16)
        | ((data[offset + 3] as u32) << 24)
}

/// Write a little-endian u32 to a byte array at the given offset.
#[inline]
fn config_write_u32(data: &mut [u8], offset: usize, val: u32) {
    data[offset] = val as u8;
    data[offset + 1] = (val >> 8) as u8;
    data[offset + 2] = (val >> 16) as u8;
    data[offset + 3] = (val >> 24) as u8;
}

/// Write a little-endian u16 to a byte array at the given offset.
#[inline]
fn config_write_u16(data: &mut [u8], offset: usize, val: u16) {
    data[offset] = val as u8;
    data[offset + 1] = (val >> 8) as u8;
}
