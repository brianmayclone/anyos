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
use crate::memory::mmio::MmioHandler;

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
    /// Expansion ROM BAR size mask (offset 0x30). 0 = no ROM present.
    pub rom_bar_size: u32,
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

        // Command (offset 0x04): enable I/O + memory space access.
        config_space[0x04] = 0x03;
        config_space[0x05] = 0x00;

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
            rom_bar_size: 0,
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

    /// Add a 32-bit MSI capability at the given config space offset.
    ///
    /// Sets up the PCI Capabilities List pointer (offset 0x34) and the
    /// MSI capability structure (10 bytes):
    ///   offset+0: Cap ID (0x05) | Next Cap (0x00)
    ///   offset+2: Message Control (0x0000 = 1 vector, 32-bit, disabled)
    ///   offset+4: Message Address (32-bit, guest-writable)
    ///   offset+8: Message Data (16-bit, guest-writable)
    ///
    /// Also sets Status bit 4 (Capabilities List) in the PCI Status register.
    pub fn add_msi_capability(&mut self, offset: usize) {
        // Set Capabilities List bit in Status register (offset 0x06, bit 4)
        self.config_space[0x06] |= 0x10;
        // Capabilities pointer (offset 0x34) → points to our MSI cap
        self.config_space[0x34] = offset as u8;
        // MSI Capability ID = 0x05
        self.config_space[offset] = 0x05;
        // Next capability pointer = 0x00 (end of list)
        self.config_space[offset + 1] = 0x00;
        // Message Control: 0x0000
        //   Bits 3:1 = 000 (1 vector allocated)
        //   Bit 7 = 0 (32-bit address)
        //   Bit 0 = 0 (MSI disabled initially, guest enables it)
        self.config_space[offset + 2] = 0x00;
        self.config_space[offset + 3] = 0x00;
        // Message Address (32-bit): initially 0
        self.config_space[offset + 4] = 0x00;
        self.config_space[offset + 5] = 0x00;
        self.config_space[offset + 6] = 0x00;
        self.config_space[offset + 7] = 0x00;
        // Message Data (16-bit): initially 0
        self.config_space[offset + 8] = 0x00;
        self.config_space[offset + 9] = 0x00;
    }
}

/// PCI system bus holding registered devices.
#[derive(Debug)]
pub struct PciBus {
    /// Last address written to the Configuration Address port (0xCF8).
    pub config_address: u32,
    /// Registered PCI devices.
    pub devices: Vec<PciDevice>,
}

impl PciBus {
    /// Create a new empty PCI bus with no devices.
    pub fn new() -> Self {
        PciBus {
            config_address: 0,
            devices: Vec::new(),
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

    /// Read from PCI config space by explicit BDF and register (MMCONFIG path).
    ///
    /// Returns bytes from the device's 256-byte config space. Registers
    /// beyond offset 255 return all-ones (extended config space not emulated).
    pub fn mmcfg_read(&mut self, bus: u8, device: u8, function: u8, register: usize, size: u8) -> u64 {
        if let Some(dev) = self.find_device(bus, device, function) {
            if register + (size as usize) <= dev.config_space.len() {
                match size {
                    1 => dev.config_space[register] as u64,
                    2 => {
                        (dev.config_space[register] as u64)
                            | ((dev.config_space[register + 1] as u64) << 8)
                    }
                    4 => config_read_u32(&dev.config_space, register) as u64,
                    _ => 0xFFFFFFFF,
                }
            } else {
                // Register offset out of range.
                match size {
                    1 => 0xFF,
                    2 => 0xFFFF,
                    _ => 0xFFFFFFFF,
                }
            }
        } else {
            // No device at this BDF.
            match size {
                1 => 0xFF,
                2 => 0xFFFF,
                _ => 0xFFFFFFFF,
            }
        }
    }

    /// Write to PCI config space by explicit BDF and register (MMCONFIG path).
    ///
    /// Handles BAR size probing and read-only field protection, same as the
    /// CF8/CFC I/O path.
    pub fn mmcfg_write(&mut self, bus: u8, device: u8, function: u8, register: usize, size: u8, val: u64) {
        if let Some(dev) = self.find_device(bus, device, function) {
            if register >= dev.config_space.len() {
                return;
            }

            // BAR size probing (dword writes to BAR region).
            if size == 4 && register >= 0x10 && register <= 0x24 {
                let bar_index = (register - 0x10) / 4;
                if bar_index < 6 {
                    let val32 = val as u32;
                    if val32 == 0xFFFFFFFF {
                        config_write_u32(&mut dev.config_space, register, dev.bar_sizes[bar_index]);
                        return;
                    }
                    let type_bits = dev.config_space[register] & 0x0F;
                    let new_val = (val32 & 0xFFFFFFF0) | (type_bits as u32);
                    config_write_u32(&mut dev.config_space, register, new_val);
                    return;
                }
            }

            // Expansion ROM BAR size probing (offset 0x30).
            if size == 4 && register == 0x30 {
                let val32 = val as u32;
                if val32 == 0xFFFFFFFE || val32 == 0xFFFFFFFF {
                    // Size probe: return size mask (0 if no ROM)
                    config_write_u32(&mut dev.config_space, 0x30, dev.rom_bar_size);
                    return;
                }
                // Normal write: store base address with enable bit
                config_write_u32(&mut dev.config_space, 0x30, val32);
                return;
            }

            // Read-only field protection.
            match register {
                0x00..=0x03 | 0x08..=0x0B => { /* Vendor/Device ID, Class: read-only */ }
                _ => match size {
                    1 => {
                        if register < dev.config_space.len() {
                            dev.config_space[register] = val as u8;
                        }
                    }
                    2 => {
                        if register + 1 < dev.config_space.len() {
                            config_write_u16(&mut dev.config_space, register, val as u16);
                        }
                    }
                    4 => {
                        if register + 3 < dev.config_space.len() {
                            config_write_u32(&mut dev.config_space, register, val as u32);
                        }
                    }
                    _ => {}
                },
            }
        }
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
                    let is_io = dev.config_space[register] & 0x01 != 0;
                    let (type_bits, addr_mask) = if is_io {
                        (dev.config_space[register] & 0x03, 0xFFFF_FFFCu32)
                    } else {
                        (dev.config_space[register] & 0x0F, 0xFFFF_FFF0u32)
                    };
                    let new_val = (val & addr_mask) | (type_bits as u32);
                    config_write_u32(&mut dev.config_space, register, new_val);
                    return;
                }
            }

            // Expansion ROM BAR size probing (offset 0x30).
            if register == 0x30 {
                if val == 0xFFFFFFFE || val == 0xFFFFFFFF {
                    config_write_u32(&mut dev.config_space, 0x30, dev.rom_bar_size);
                    return;
                }
                config_write_u32(&mut dev.config_space, 0x30, val);
                return;
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
    /// - 0xCF8-0xCFB: returns bytes/words/dword of the config address latch
    /// - 0xCFC-0xCFF: reads from PCI configuration data, supports
    ///   byte and word sub-accesses
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        let val = match port {
            0xCF8..=0xCFB => {
                let byte_offset = (port - 0xCF8) as u32;
                let shifted = self.config_address >> (byte_offset * 8);
                match size {
                    1 => shifted & 0xFF,
                    2 => shifted & 0xFFFF,
                    _ => shifted,
                }
            }
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
    /// - 0xCF8-0xCFB: stores the configuration address latch
    /// - 0xCFC-0xCFF: writes to PCI configuration data, supports
    ///   byte and word sub-accesses
    fn write(&mut self, port: u16, size: u8, val: u32) -> Result<()> {
        match port {
            0xCF8..=0xCFB => {
                // Mechanism #1 config-address latch supports byte/word accesses
                // across 0xCF8..0xCFB. Merge sub-accesses into the 32-bit latch.
                let new_addr = if size >= 4 && port == 0xCF8 {
                    val
                } else {
                    let byte_offset = (port - 0xCF8) as u32;
                    let mask = match size {
                        1 => 0xFFu32,
                        2 => 0xFFFFu32,
                        _ => 0xFFFF_FFFFu32,
                    };
                    let shifted_mask = mask << (byte_offset * 8);
                    let shifted_val = (val & mask) << (byte_offset * 8);
                    (self.config_address & !shifted_mask) | shifted_val
                };

                // Bits 1:0 are reserved and read as zero in config-address.
                self.config_address = new_addr & !0x3;
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

// ── MMCONFIG (PCI Express Enhanced Configuration) MMIO handler ──

/// PCI Express Enhanced Configuration Mechanism (MMCONFIG) handler.
///
/// Maps a 256 MiB physical address region into PCI configuration space.
/// Each BDF (bus/device/function) gets 4 KiB of config space. The address
/// layout within the region is:
///
/// ```text
/// offset = (bus << 20) | (device << 15) | (function << 12) | register
/// ```
///
/// SeaBIOS on Q35 reads the PCIEXBAR register from the MCH to discover
/// this region, then uses MMIO reads instead of CF8/CFC port I/O for
/// all PCI config space accesses.
pub struct PciMmcfgHandler {
    /// Raw pointer to the PCI bus (valid for the lifetime of the VM).
    bus_ptr: *mut PciBus,
}

impl PciMmcfgHandler {
    /// Create a new MMCONFIG handler pointing to the given PCI bus.
    pub fn new(bus_ptr: *mut PciBus) -> Self {
        PciMmcfgHandler { bus_ptr }
    }

    /// Decode an MMCONFIG offset into (bus, device, function, register).
    #[inline]
    fn decode_offset(offset: u64) -> (u8, u8, u8, usize) {
        let bus = ((offset >> 20) & 0xFF) as u8;
        let device = ((offset >> 15) & 0x1F) as u8;
        let function = ((offset >> 12) & 0x07) as u8;
        let register = (offset & 0xFFF) as usize;
        (bus, device, function, register)
    }
}

impl MmioHandler for PciMmcfgHandler {
    /// Read from MMCONFIG region.
    ///
    /// Decodes the MMIO offset into a PCI BDF + register and reads from
    /// the device's configuration space. Returns all-ones if no device
    /// exists at the addressed BDF.
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        let (bus, device, function, register) = Self::decode_offset(offset);
        let pci_bus = unsafe { &mut *self.bus_ptr };
        let val = pci_bus.mmcfg_read(bus, device, function, register, size);
        Ok(val)
    }

    /// Write to MMCONFIG region.
    ///
    /// Decodes the MMIO offset into a PCI BDF + register and writes to
    /// the device's configuration space with BAR probing and read-only
    /// field protection.
    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()> {
        let (bus, device, function, register) = Self::decode_offset(offset);
        let pci_bus = unsafe { &mut *self.bus_ptr };
        pci_bus.mmcfg_write(bus, device, function, register, size, val);
        Ok(())
    }
}
