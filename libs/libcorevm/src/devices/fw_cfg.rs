//! QEMU fw_cfg (Firmware Configuration) device emulation.
//!
//! SeaBIOS uses this interface to discover platform configuration including
//! RAM size, CPU count, and option ROM files. Without it, SeaBIOS falls back
//! to legacy detection which may miss features like VGA BIOS loading.
//!
//! # I/O Ports
//!
//! | Port | Width | Direction | Description |
//! |------|-------|-----------|-------------|
//! | 0x510 | 16-bit | Write | Selector register (chooses config item) |
//! | 0x511 | 8-bit | Read | Data register (sequential byte reads) |
//!
//! # Protocol
//!
//! 1. Write a 16-bit selector key to port 0x510 (resets data offset to 0)
//! 2. Read bytes sequentially from port 0x511 (auto-increments offset)
//! 3. Reading past the end of an item returns 0x00

use alloc::vec::Vec;
use crate::error::Result;
use crate::io::IoHandler;

// Well-known fw_cfg selector keys.
const FW_CFG_SIGNATURE: u16 = 0x0000;
const FW_CFG_ID: u16 = 0x0001;
const FW_CFG_UUID: u16 = 0x0002;
const FW_CFG_RAM_SIZE: u16 = 0x0003;
const FW_CFG_NOGRAPHIC: u16 = 0x0004;
const FW_CFG_NB_CPUS: u16 = 0x0005;
const FW_CFG_MAX_CPUS: u16 = 0x000F;
const FW_CFG_NUMA: u16 = 0x000D;
const FW_CFG_BOOT_MENU: u16 = 0x000E;
const FW_CFG_FILE_DIR: u16 = 0x0019;

// x86-specific keys.
const FW_CFG_ACPI_TABLES: u16 = 0x8000;
const FW_CFG_SMBIOS_ENTRIES: u16 = 0x8001;
const FW_CFG_IRQ0_OVERRIDE: u16 = 0x8002;
const FW_CFG_E820_TABLE: u16 = 0x8003;

/// A named file entry in the fw_cfg file directory.
struct FwCfgFile {
    /// File data content.
    data: Vec<u8>,
    /// Selector key assigned to this file (>= 0x0020).
    selector: u16,
    /// NUL-terminated file name (max 55 chars + NUL).
    name: [u8; 56],
}

/// QEMU fw_cfg device emulation.
///
/// Provides the minimum configuration needed for SeaBIOS to detect the
/// platform, enumerate RAM, and load VGA option ROMs.
#[derive(Debug)]
pub struct FwCfg {
    /// Currently selected configuration key.
    selector: u16,
    /// Current read offset within the selected item's data.
    offset: usize,
    /// Guest RAM size in bytes.
    ram_size: u64,
    /// File directory entries.
    files: Vec<FwCfgFileEntry>,
}

/// Simplified file entry for internal storage.
#[derive(Debug)]
struct FwCfgFileEntry {
    /// File data.
    data: Vec<u8>,
    /// Selector key (>= 0x0020).
    selector: u16,
    /// File name (NUL-padded to 56 bytes).
    name: [u8; 56],
}

impl FwCfg {
    /// Create a new fw_cfg device with the given RAM size.
    pub fn new(ram_size: u64) -> Self {
        FwCfg {
            selector: 0,
            offset: 0,
            ram_size,
            files: Vec::new(),
        }
    }

    /// Add a named file to the fw_cfg file directory.
    ///
    /// The file will be assigned the next available selector key (starting at 0x0020).
    pub fn add_file(&mut self, name: &str, data: Vec<u8>) {
        let selector = 0x0020 + self.files.len() as u16;
        let mut name_buf = [0u8; 56];
        let copy_len = name.len().min(55);
        name_buf[..copy_len].copy_from_slice(&name.as_bytes()[..copy_len]);
        self.files.push(FwCfgFileEntry {
            data,
            selector,
            name: name_buf,
        });
    }

    /// Get the data for the currently selected key.
    fn get_item_data(&self) -> Vec<u8> {
        match self.selector {
            FW_CFG_SIGNATURE => {
                // "QEMU" in ASCII.
                Vec::from(*b"QEMU")
            }
            FW_CFG_ID => {
                // Feature flags: bit 0 = traditional I/O, bit 1 = DMA (not supported).
                1u32.to_le_bytes().to_vec()
            }
            FW_CFG_UUID => {
                // Return zero UUID.
                Vec::from([0u8; 16])
            }
            FW_CFG_RAM_SIZE => {
                // Total RAM in bytes (u64 LE).
                self.ram_size.to_le_bytes().to_vec()
            }
            FW_CFG_NOGRAPHIC => {
                // 0 = graphics mode (VGA enabled).
                0u16.to_le_bytes().to_vec()
            }
            FW_CFG_NB_CPUS => {
                // 1 CPU.
                1u16.to_le_bytes().to_vec()
            }
            FW_CFG_MAX_CPUS => {
                // Max 1 APIC ID.
                1u16.to_le_bytes().to_vec()
            }
            FW_CFG_BOOT_MENU => {
                // No boot menu.
                0u16.to_le_bytes().to_vec()
            }
            FW_CFG_NUMA => {
                // No NUMA: count = 0 (u64 LE).
                0u64.to_le_bytes().to_vec()
            }
            FW_CFG_ACPI_TABLES => {
                // No ACPI tables: count = 0 (u16 LE).
                0u16.to_le_bytes().to_vec()
            }
            FW_CFG_SMBIOS_ENTRIES => {
                // No SMBIOS entries: count = 0 (u16 LE).
                0u16.to_le_bytes().to_vec()
            }
            FW_CFG_IRQ0_OVERRIDE => {
                // IRQ0 override active.
                1u32.to_le_bytes().to_vec()
            }
            FW_CFG_E820_TABLE => {
                // E820 memory map: two entries (low RAM + extended RAM).
                let mut data = Vec::new();
                // Entry count (u32 LE).
                data.extend_from_slice(&2u32.to_le_bytes());
                // Entry 0: conventional memory 0x00000-0x9FFFF (640 KB).
                data.extend_from_slice(&0u64.to_le_bytes());       // address
                data.extend_from_slice(&0xA0000u64.to_le_bytes()); // length
                data.extend_from_slice(&1u32.to_le_bytes());       // type = RAM
                // Entry 1: extended memory 0x100000 - (ram_size).
                let ext_start = 0x10_0000u64;
                let ext_len = if self.ram_size > ext_start {
                    self.ram_size - ext_start
                } else {
                    0
                };
                data.extend_from_slice(&ext_start.to_le_bytes()); // address
                data.extend_from_slice(&ext_len.to_le_bytes());   // length
                data.extend_from_slice(&1u32.to_le_bytes());      // type = RAM
                data
            }
            FW_CFG_FILE_DIR => {
                // File directory with big-endian count and entries.
                let count = self.files.len() as u32;
                let mut data = Vec::new();
                data.extend_from_slice(&count.to_be_bytes()); // count (big-endian)
                for f in &self.files {
                    let size = f.data.len() as u32;
                    data.extend_from_slice(&size.to_be_bytes());      // size (BE)
                    data.extend_from_slice(&f.selector.to_be_bytes()); // selector (BE)
                    data.extend_from_slice(&0u16.to_be_bytes());      // reserved
                    data.extend_from_slice(&f.name);                  // name (56 bytes)
                }
                data
            }
            sel if sel >= 0x0020 => {
                // Dynamic file: look up by selector.
                for f in &self.files {
                    if f.selector == sel {
                        return f.data.clone();
                    }
                }
                Vec::new()
            }
            _ => {
                // Unknown key: return empty.
                Vec::new()
            }
        }
    }
}

impl IoHandler for FwCfg {
    /// Read from fw_cfg data port (0x511).
    ///
    /// Returns the next byte of the currently selected item. Reading past
    /// the end returns 0x00.
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        if port == 0x511 {
            let data = self.get_item_data();
            let val = if self.offset < data.len() {
                data[self.offset]
            } else {
                0x00
            };
            self.offset += 1;
            Ok(val as u32)
        } else {
            Ok(0xFF)
        }
    }

    /// Write to fw_cfg selector port (0x510).
    ///
    /// Selects a configuration key and resets the data read offset to 0.
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        if port == 0x510 {
            let sel = val as u16;
            // Log selector accesses for boot diagnostics.
            libsyscall::serial_print(format_args!(
                "[fw_cfg] select 0x{:04X}\n", sel
            ));
            self.selector = sel;
            self.offset = 0;
        }
        Ok(())
    }
}
