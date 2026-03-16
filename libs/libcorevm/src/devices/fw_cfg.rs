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
    /// Guest RAM size below PCI hole (reported via FW_CFG_RAM_SIZE).
    ram_size: u64,
    /// Guest RAM above 4 GB (relocated from PCI hole region).
    ram_above_4g: u64,
    /// File directory entries.
    files: Vec<FwCfgFileEntry>,
    /// DMA address accumulator (high 32 bits written first).
    dma_addr_high: u32,
    /// Pointer to guest RAM (for DMA operations).
    ram_ptr: *mut u8,
    /// Size of guest RAM in bytes.
    ram_len: usize,
    // --- Legacy kernel boot data (selectors 0x07-0x18) ---
    kernel_addr: u32,
    kernel_size: u32,
    kernel_data: Vec<u8>,
    initrd_addr: u32,
    initrd_size: u32,
    initrd_data: Vec<u8>,
    cmdline_data: Vec<u8>,
    setup_addr: u32,
    setup_size: u32,
    setup_data: Vec<u8>,
    kernel_entry: u32,
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
    ///
    /// `ram_size` is the total guest RAM in bytes. If it exceeds the PCI hole
    /// start (0xE0000000 = 3.5 GB), FW_CFG_RAM_SIZE reports only the below-4G
    /// portion and the excess is reported via e820 above 4 GB.
    pub fn new(ram_size: u64) -> Self {
        const PCI_HOLE_START: u64 = 0xE000_0000;
        const PCI_HOLE_END: u64   = 0x1_0000_0000;

        // RAM visible below the PCI hole.
        let ram_below = if ram_size > PCI_HOLE_START { PCI_HOLE_START } else { ram_size };
        // RAM relocated above 4 GB.
        let ram_above_4g = if ram_size > PCI_HOLE_START { ram_size - PCI_HOLE_START } else { 0 };

        let mut fw = FwCfg {
            selector: 0,
            offset: 0,
            ram_size: ram_below,
            ram_above_4g,
            files: Vec::new(),
            dma_addr_high: 0,
            ram_ptr: core::ptr::null_mut(),
            ram_len: 0,
            kernel_addr: 0,
            kernel_size: 0,
            kernel_data: Vec::new(),
            initrd_addr: 0,
            initrd_size: 0,
            initrd_data: Vec::new(),
            cmdline_data: Vec::new(),
            setup_addr: 0,
            setup_size: 0,
            setup_data: Vec::new(),
            kernel_entry: 0,
        };

        // Tell SeaBIOS about RAM above 4 GB.
        if ram_above_4g > 0 {
            fw.add_file("etc/ram-size-over-4g", ram_above_4g.to_le_bytes().to_vec());
        }

        fw
    }

    /// Set the guest RAM pointer for DMA operations.
    ///
    /// Must be called after guest memory is set up. Without this, DMA
    /// operations will fail gracefully (error bit set in descriptor).
    pub fn set_ram(&mut self, ptr: *mut u8, len: usize) {
        self.ram_ptr = ptr;
        self.ram_len = len;
    }

    /// Set up direct kernel boot via legacy fw_cfg selectors.
    ///
    /// Parses a Linux bzImage, splits it into setup header and protected-mode
    /// kernel, computes load addresses, and populates both legacy fw_cfg
    /// selectors (0x07-0x18) and the `etc/linuxboot` file for linuxboot_dma.bin.
    pub fn set_kernel(&mut self, bzimage: &[u8], initrd: &[u8], cmdline: &[u8]) {
        if bzimage.len() < 0x202 {
            return; // Too small to be a bzImage
        }

        // Parse the Linux boot protocol header.
        // Offset 0x1F1: setup_sects (number of 512-byte setup sectors)
        let setup_sects = bzimage[0x1F1] as u32;
        let setup_sects = if setup_sects == 0 { 4 } else { setup_sects };
        let setup_size = (setup_sects + 1) * 512;

        // Split bzImage into setup header and protected-mode kernel.
        let setup_end = (setup_size as usize).min(bzimage.len());
        self.setup_data = bzimage[..setup_end].to_vec();
        self.setup_size = setup_end as u32;
        self.kernel_data = bzimage[setup_end..].to_vec();
        self.kernel_size = self.kernel_data.len() as u32;

        // Standard load addresses for bzImage boot.
        self.setup_addr = 0x10000;  // Real-mode setup at 64KB
        self.kernel_addr = 0x100000; // Protected-mode kernel at 1MB
        self.kernel_entry = 0x100000;

        // Command line at 0x20000 (128KB).
        let mut cmdline_buf = cmdline.to_vec();
        if !cmdline_buf.ends_with(&[0]) {
            cmdline_buf.push(0);
        }
        self.cmdline_data = cmdline_buf;

        // Initrd: load at top of low RAM minus initrd size, page-aligned.
        self.initrd_data = initrd.to_vec();
        self.initrd_size = initrd.len() as u32;
        if !initrd.is_empty() {
            let ram_top = self.ram_size as u32;
            let initrd_addr = (ram_top - self.initrd_size) & !0xFFF;
            self.initrd_addr = initrd_addr;
        }

        // Patch the setup header with initrd info and cmdline pointer.
        // Offset 0x218: ramdisk_image (4 bytes LE)
        // Offset 0x21C: ramdisk_size (4 bytes LE)
        // Offset 0x228: cmd_line_ptr (4 bytes LE)
        if self.setup_data.len() >= 0x230 {
            let initrd_addr_bytes = self.initrd_addr.to_le_bytes();
            self.setup_data[0x218..0x21C].copy_from_slice(&initrd_addr_bytes);
            let initrd_size_bytes = self.initrd_size.to_le_bytes();
            self.setup_data[0x21C..0x220].copy_from_slice(&initrd_size_bytes);
            let cmdline_addr: u32 = 0x20000;
            self.setup_data[0x228..0x22C].copy_from_slice(&cmdline_addr.to_le_bytes());

            // Set type_of_loader to 0xFF (unknown bootloader).
            self.setup_data[0x210] = 0xFF;
            // Set loadflags: LOADED_HIGH (bit 0) = 1.
            self.setup_data[0x211] |= 0x01;
        }

        // Create the etc/linuxboot file for linuxboot_dma.bin.
        // Format: 8 x u32 LE (kernel_addr, kernel_size, setup_addr, setup_size,
        //                       cmdline_addr, cmdline_size, initrd_addr, initrd_size)
        let mut lb = Vec::with_capacity(32);
        lb.extend_from_slice(&self.kernel_addr.to_le_bytes());
        lb.extend_from_slice(&self.kernel_size.to_le_bytes());
        lb.extend_from_slice(&self.setup_addr.to_le_bytes());
        lb.extend_from_slice(&self.setup_size.to_le_bytes());
        lb.extend_from_slice(&0x20000u32.to_le_bytes()); // cmdline_addr
        lb.extend_from_slice(&(self.cmdline_data.len() as u32).to_le_bytes());
        lb.extend_from_slice(&self.initrd_addr.to_le_bytes());
        lb.extend_from_slice(&self.initrd_size.to_le_bytes());
        self.add_file("etc/linuxboot", lb);

        #[cfg(feature = "linux")]
        eprintln!("[fw_cfg] kernel: setup={}B@{:#x} kernel={}B@{:#x} initrd={}B@{:#x} cmdline={}B",
            self.setup_size, self.setup_addr,
            self.kernel_size, self.kernel_addr,
            self.initrd_size, self.initrd_addr,
            self.cmdline_data.len());
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
                // Additional e820 entries for RAM above 4 GB.
                // SeaBIOS builds the below-4G e820 map from FW_CFG_RAM_SIZE and CMOS.
                let mut data = Vec::new();
                if self.ram_above_4g > 0 {
                    // One entry: RAM starting at 4 GB.
                    data.extend_from_slice(&1u32.to_le_bytes()); // count = 1
                    // Each entry: address (u64 LE), length (u64 LE), type (u32 LE)
                    data.extend_from_slice(&0x1_0000_0000u64.to_le_bytes()); // address = 4 GB
                    data.extend_from_slice(&self.ram_above_4g.to_le_bytes()); // length
                    data.extend_from_slice(&1u32.to_le_bytes()); // type 1 = RAM
                } else {
                    data.extend_from_slice(&0u32.to_le_bytes()); // count = 0
                }
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
            // Legacy kernel boot selectors (populated by set_kernel).
            0x07 => self.kernel_addr.to_le_bytes().to_vec(), // FW_CFG_KERNEL_ADDR
            0x08 => self.kernel_size.to_le_bytes().to_vec(), // FW_CFG_KERNEL_SIZE
            0x09 => self.cmdline_data.clone(),               // FW_CFG_KERNEL_CMDLINE
            0x0A => self.initrd_addr.to_le_bytes().to_vec(), // FW_CFG_INITRD_ADDR
            0x0B => self.initrd_size.to_le_bytes().to_vec(), // FW_CFG_INITRD_SIZE
            0x10 => self.kernel_entry.to_le_bytes().to_vec(),// FW_CFG_KERNEL_ENTRY
            0x11 => self.kernel_data.clone(),                // FW_CFG_KERNEL_DATA
            0x12 => self.initrd_data.clone(),                // FW_CFG_INITRD_DATA
            0x13 => 0x20000u32.to_le_bytes().to_vec(),       // FW_CFG_CMDLINE_ADDR
            0x14 => (self.cmdline_data.len() as u32).to_le_bytes().to_vec(), // FW_CFG_CMDLINE_SIZE
            0x15 => self.cmdline_data.clone(),               // FW_CFG_CMDLINE_DATA
            0x16 => self.setup_addr.to_le_bytes().to_vec(),  // FW_CFG_SETUP_ADDR
            0x17 => self.setup_size.to_le_bytes().to_vec(),  // FW_CFG_SETUP_SIZE
            0x18 => self.setup_data.clone(),                 // FW_CFG_SETUP_DATA
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
                    let _ = file_info; // suppress warning
                }
                #[cfg(all(feature = "anyos", not(any(feature = "linux", feature = "std"))))]
                libsyscall::serial_print(format_args!(
                    "[fw_cfg] select 0x{:04X}\n", sel
                ));
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
                // DMA transfer
                self.process_dma(desc_addr);
                self.dma_addr_high = 0;
            }
            _ => {}
        }
        Ok(())
    }
}
