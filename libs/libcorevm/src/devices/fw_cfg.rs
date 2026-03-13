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
//! | 0x514 | 32-bit | Write | DMA address high (big-endian) |
//! | 0x518 | 32-bit | Write | DMA address low (big-endian), triggers DMA |
//!
//! # Protocol
//!
//! 1. Write a 16-bit selector key to port 0x510 (resets data offset to 0)
//! 2. Read bytes sequentially from port 0x511 (auto-increments offset)
//! 3. Reading past the end of an item returns 0x00
//! 4. DMA: write 64-bit big-endian descriptor address to 0x514/0x518

use alloc::vec;
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

// DMA control bits.
const FW_CFG_DMA_CTL_ERROR: u32 = 0x01;
const FW_CFG_DMA_CTL_READ: u32 = 0x02;
const FW_CFG_DMA_CTL_SKIP: u32 = 0x04;
const FW_CFG_DMA_CTL_SELECT: u32 = 0x08;
const FW_CFG_DMA_CTL_WRITE: u32 = 0x10;

/// QEMU fw_cfg device emulation.
///
/// Provides the minimum configuration needed for SeaBIOS to detect the
/// platform, enumerate RAM, and load VGA option ROMs. Supports both legacy
/// I/O (port 0x510/0x511) and DMA (port 0x514) interfaces.
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
    /// DMA address accumulator (high 32 bits written first).
    dma_addr_high: u32,
    /// Pointer to guest RAM (for DMA operations).
    ram_ptr: *mut u8,
    /// Size of guest RAM in bytes.
    ram_len: usize,
}

// Safety: FwCfg is only used from the single VM thread.
unsafe impl Send for FwCfg {}
unsafe impl Sync for FwCfg {}

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
            dma_addr_high: 0,
            ram_ptr: core::ptr::null_mut(),
            ram_len: 0,
        }
    }

    /// Set the guest RAM pointer for DMA operations.
    ///
    /// Must be called after guest memory is set up. Without this, DMA
    /// operations will fail gracefully (error bit set in descriptor).
    pub fn set_ram(&mut self, ptr: *mut u8, len: usize) {
        self.ram_ptr = ptr;
        self.ram_len = len;
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
                // Feature flags: bit 0 = traditional I/O, bit 1 = DMA.
                3u32.to_le_bytes().to_vec()
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
                // No legacy ACPI tables (use table-loader instead).
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
                // Additional e820 entries. SeaBIOS already gets RAM info from
                // FW_CFG_RAM_SIZE and CMOS, so don't duplicate RAM entries here.
                // Return 0 entries — SeaBIOS builds the full e820 map itself.
                let mut data = Vec::new();
                data.extend_from_slice(&0u32.to_le_bytes()); // count = 0
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

    /// Read `len` bytes from guest physical address `addr`.
    fn guest_read(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
        let a = addr as usize;
        if self.ram_ptr.is_null() || a.checked_add(len)? > self.ram_len {
            return None;
        }
        let mut buf = vec![0u8; len];
        unsafe {
            core::ptr::copy_nonoverlapping(self.ram_ptr.add(a), buf.as_mut_ptr(), len);
        }
        Some(buf)
    }

    /// Write `data` to guest physical address `addr`.
    fn guest_write(&self, addr: u64, data: &[u8]) -> bool {
        let a = addr as usize;
        if self.ram_ptr.is_null() || a.saturating_add(data.len()) > self.ram_len {
            return false;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), self.ram_ptr.add(a), data.len());
        }
        true
    }

    /// Process a DMA transfer request.
    ///
    /// The DMA descriptor at `desc_addr` is a 16-byte big-endian structure:
    ///   [0..4]  control (u32 BE): SELECT|READ|WRITE|SKIP|ERROR flags
    ///   [4..8]  length  (u32 BE): number of bytes
    ///   [8..16] address (u64 BE): guest physical address for data
    fn process_dma(&mut self, desc_addr: u64) {
        // Read the 16-byte DMA descriptor from guest RAM.
        let desc = match self.guest_read(desc_addr, 16) {
            Some(d) => d,
            None => return,
        };

        let control = u32::from_be_bytes([desc[0], desc[1], desc[2], desc[3]]);
        let length = u32::from_be_bytes([desc[4], desc[5], desc[6], desc[7]]) as usize;
        let address = u64::from_be_bytes([desc[8], desc[9], desc[10], desc[11],
                                          desc[12], desc[13], desc[14], desc[15]]);

        // Handle SELECT: change current selector.
        if control & FW_CFG_DMA_CTL_SELECT != 0 {
            let new_sel = (control >> 16) as u16;
            self.selector = new_sel;
            self.offset = 0;
        }

        let mut error = false;

        if control & FW_CFG_DMA_CTL_READ != 0 {
            // READ: copy from fw_cfg file data to guest RAM.
            let data = self.get_item_data();
            let mut dst_off = 0usize;
            let mut remaining = length;
            while remaining > 0 {
                if self.offset < data.len() {
                    let chunk = (data.len() - self.offset).min(remaining);
                    if !self.guest_write(address + dst_off as u64, &data[self.offset..self.offset + chunk]) {
                        error = true;
                        break;
                    }
                    self.offset += chunk;
                    dst_off += chunk;
                    remaining -= chunk;
                } else {
                    // Past end of data: fill with zeros.
                    let zeros = vec![0u8; remaining];
                    if !self.guest_write(address + dst_off as u64, &zeros) {
                        error = true;
                    }
                    self.offset += remaining;
                    break;
                }
            }
        } else if control & FW_CFG_DMA_CTL_SKIP != 0 {
            // SKIP: advance offset without data transfer.
            self.offset += length;
        } else if control & FW_CFG_DMA_CTL_WRITE != 0 {
            // WRITE: guest → fw_cfg (not commonly used, set error).
            error = true;
        }

        // Clear control field (set ERROR if needed) to signal completion.
        let result: u32 = if error { FW_CFG_DMA_CTL_ERROR } else { 0 };
        self.guest_write(desc_addr, &result.to_be_bytes());
    }
}

impl IoHandler for FwCfg {
    /// Read from fw_cfg data port (0x511).
    ///
    /// Returns the next byte of the currently selected item. Reading past
    /// the end returns 0x00.
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        if port == 0x511 {
            let data = self.get_item_data();
            // Support multi-byte reads (size 1, 2, or 4)
            let bytes = (size as usize).max(1).min(4);
            let mut val: u32 = 0;
            for i in 0..bytes {
                let b = if self.offset < data.len() {
                    data[self.offset]
                } else {
                    0x00
                };
                val |= (b as u32) << (i * 8);
                self.offset += 1;
            }
            Ok(val)
        } else {
            Ok(0xFF)
        }
    }

    /// Write to fw_cfg selector port (0x510) or DMA port (0x514/0x518).
    ///
    /// Port 0x510: Selects a configuration key and resets the data read offset.
    /// Port 0x514: Sets high 32 bits of DMA descriptor address (big-endian).
    /// Port 0x518: Sets low 32 bits and triggers DMA transfer (big-endian).
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        match port {
            0x510 => {
                let sel = val as u16;
                #[cfg(feature = "linux")]
                {
                    let file_info = if sel >= 0x0020 {
                        self.files.iter().find(|f| f.selector == sel)
                            .map(|f| {
                                let name_end = f.name.iter().position(|&b| b == 0).unwrap_or(56);
                                alloc::format!(" -> '{}' ({} bytes)",
                                    core::str::from_utf8(&f.name[..name_end]).unwrap_or("?"), f.data.len())
                            })
                            .unwrap_or_else(|| alloc::format!(" -> (no file)"))
                    } else if sel == 0x0019 {
                        // Log file directory contents
                        alloc::format!(" [FILE_DIR: {} files: {}]",
                            self.files.len(),
                            self.files.iter().map(|f| {
                                let ne = f.name.iter().position(|&b| b == 0).unwrap_or(56);
                                alloc::format!("{}(sel={:#x},{}b)",
                                    core::str::from_utf8(&f.name[..ne]).unwrap_or("?"), f.selector, f.data.len())
                            }).collect::<alloc::vec::Vec<_>>().join(", "))
                    } else {
                        alloc::string::String::new()
                    };
                    eprintln!("[fw_cfg] select 0x{:04X}{}", sel, file_info);
                }
                #[cfg(all(feature = "anyos", not(any(feature = "linux", feature = "std"))))]
                libsyscall::serial_print(format_args!(
                    "[fw_cfg] select 0x{:04X}\n", sel
                ));
                #[cfg(feature = "std")]
                {
                    use std::io::Write;
                    let path = std::env::var("TEMP")
                        .map(|t| std::format!("{}\\fw_cfg_debug.log", t))
                        .unwrap_or_else(|_| std::string::String::from("fw_cfg_debug.log"));
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        let file_info = if sel >= 0x0020 {
                            self.files.iter().find(|f| f.selector == sel)
                                .map(|f| {
                                    let name_end = f.name.iter().position(|&b| b == 0).unwrap_or(56);
                                    std::format!(" -> file '{}' ({} bytes)", std::string::String::from_utf8_lossy(&f.name[..name_end]), f.data.len())
                                })
                                .unwrap_or_else(|| std::string::String::from(" -> (no file)"))
                        } else {
                            std::string::String::new()
                        };
                        let _ = writeln!(f, "[fw_cfg] select 0x{:04X}{}", sel, file_info);
                    }
                }
                self.selector = sel;
                self.offset = 0;
            }
            0x514 => {
                // DMA address high 32 bits (big-endian).
                self.dma_addr_high = u32::from_be(val);
            }
            0x518 => {
                // DMA address low 32 bits (big-endian), triggers DMA.
                let low = u32::from_be(val);
                let desc_addr = ((self.dma_addr_high as u64) << 32) | (low as u64);
                #[cfg(feature = "linux")]
                eprintln!("[fw_cfg] DMA transfer: desc_addr={:#x} sel={:#x}", desc_addr, self.selector);
                self.process_dma(desc_addr);
                self.dma_addr_high = 0;
            }
            _ => {}
        }
        Ok(())
    }
}
