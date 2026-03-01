//! ATA/IDE disk controller emulation.
//!
//! Emulates a single-channel ATA controller with one drive (master)
//! attached. Supports PIO data transfers used by BIOS INT 13h and
//! early Linux boot (before DMA drivers are loaded).
//!
//! # I/O Ports
//!
//! | Port Range | Description |
//! |------------|-------------|
//! | 0x1F0-0x1F7 | Primary ATA command block |
//! | 0x3F6-0x3F7 | Primary ATA control block |
//!
//! # Supported Commands
//!
//! | Command | Code | Description |
//! |---------|------|-------------|
//! | IDENTIFY DEVICE | 0xEC | Return 512-byte device info |
//! | READ SECTORS | 0x20 | PIO read (28-bit LBA) |
//! | WRITE SECTORS | 0x30 | PIO write (28-bit LBA) |
//! | READ SECTORS EXT | 0x24 | PIO read (48-bit LBA) |
//! | WRITE SECTORS EXT | 0x34 | PIO write (48-bit LBA) |
//! | SET FEATURES | 0xEF | Feature configuration |
//! | FLUSH CACHE | 0xE7 | Flush write cache |
//! | DEVICE RESET | 0x08 | Software reset |

use alloc::vec::Vec;
use crate::error::Result;
use crate::io::IoHandler;

// ── ATA status register bits ──

/// BSY — drive is busy processing a command.
const SR_BSY: u8 = 0x80;
/// DRDY — drive is ready to accept commands.
const SR_DRDY: u8 = 0x40;
/// DF — device fault.
const SR_DF: u8 = 0x20;
/// DSC — drive seek complete.
const SR_DSC: u8 = 0x10;
/// DRQ — data request (ready to transfer data).
const SR_DRQ: u8 = 0x08;
/// ERR — error occurred (see error register).
const SR_ERR: u8 = 0x01;

// ── ATA error register bits ──

/// ABRT — command aborted.
const ER_ABRT: u8 = 0x04;

// ── ATA commands ──

const CMD_IDENTIFY: u8 = 0xEC;
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_READ_SECTORS_EXT: u8 = 0x24;
const CMD_WRITE_SECTORS_EXT: u8 = 0x34;
const CMD_SET_FEATURES: u8 = 0xEF;
const CMD_FLUSH_CACHE: u8 = 0xE7;
const CMD_DEVICE_RESET: u8 = 0x08;
const CMD_INIT_DRIVE_PARAMS: u8 = 0x91;
const CMD_READ_MULTIPLE: u8 = 0xC4;
const CMD_WRITE_MULTIPLE: u8 = 0xC5;
const CMD_SET_MULTIPLE: u8 = 0xC6;
const CMD_NOP: u8 = 0x00;

/// Sector size in bytes.
const SECTOR_SIZE: usize = 512;

/// IDE/ATA disk controller with one attached drive.
///
/// The drive image is stored as a flat `Vec<u8>`. Reads/writes beyond
/// the image size return zeros / are silently ignored.
pub struct Ide {
    // ── Drive image ──

    /// Flat disk image data. Length determines drive capacity.
    disk: Vec<u8>,
    /// Total number of sectors (disk.len() / 512).
    total_sectors: u64,

    // ── Task file registers ──

    /// Error register (read) / Features register (write).
    error: u8,
    features: u8,
    /// Sector count register.
    sector_count: u8,
    /// Sector number / LBA[7:0].
    sector_number: u8,
    /// Cylinder low / LBA[15:8].
    cylinder_low: u8,
    /// Cylinder high / LBA[23:16].
    cylinder_high: u8,
    /// Drive/head register (LBA[27:24] in low nibble, bit 6=LBA mode).
    drive_head: u8,
    /// Status register.
    status: u8,

    // ── 48-bit LBA extension (HOB = High Order Byte) ──

    hob_sector_count: u8,
    hob_sector_number: u8,
    hob_cylinder_low: u8,
    hob_cylinder_high: u8,
    /// Tracks which byte (0 or 1) of the HOB pair is being written.
    hob_toggle: bool,

    // ── Control register ──

    /// Device control register (port 0x3F6). Bit 1 = nIEN, bit 2 = SRST.
    device_control: u8,

    // ── Data transfer state ──

    /// 512-byte sector buffer for PIO transfers.
    buffer: [u8; SECTOR_SIZE],
    /// Current byte offset within the buffer (0..512).
    buffer_offset: usize,
    /// Number of sectors remaining in the current multi-sector transfer.
    sectors_remaining: u32,
    /// True if the current transfer is a write (guest→disk).
    is_write: bool,
    /// True if the drive raises IRQ 14 on command completion.
    irq_pending: bool,
    /// Multiple sector count for READ/WRITE MULTIPLE.
    multiple_count: u8,
}

impl Ide {
    /// Create a new IDE controller with no disk attached.
    pub fn new() -> Self {
        Ide {
            disk: Vec::new(),
            total_sectors: 0,
            error: 0,
            features: 0,
            sector_count: 1,
            sector_number: 1,
            cylinder_low: 0,
            cylinder_high: 0,
            drive_head: 0,
            status: SR_DRDY | SR_DSC,
            hob_sector_count: 0,
            hob_sector_number: 0,
            hob_cylinder_low: 0,
            hob_cylinder_high: 0,
            hob_toggle: false,
            device_control: 0,
            buffer: [0u8; SECTOR_SIZE],
            buffer_offset: 0,
            sectors_remaining: 0,
            is_write: false,
            irq_pending: false,
            multiple_count: 1,
        }
    }

    /// Attach a disk image. The image is a flat sector dump.
    ///
    /// The image length is rounded down to the nearest sector boundary.
    pub fn attach_disk(&mut self, mut image: Vec<u8>) {
        let sectors = image.len() / SECTOR_SIZE;
        image.truncate(sectors * SECTOR_SIZE);
        self.total_sectors = sectors as u64;
        self.disk = image;
        // Update status to indicate drive present and ready.
        self.status = SR_DRDY | SR_DSC;
    }

    /// Detach the current disk image and return it.
    pub fn detach_disk(&mut self) -> Vec<u8> {
        self.total_sectors = 0;
        self.status = 0;
        core::mem::take(&mut self.disk)
    }

    /// Returns true if an IRQ is pending (and nIEN is not set).
    pub fn irq_raised(&self) -> bool {
        self.irq_pending && (self.device_control & 0x02) == 0
    }

    /// Clear the pending IRQ (called after the PIC services it).
    pub fn clear_irq(&mut self) {
        self.irq_pending = false;
    }

    /// Get total disk size in bytes.
    pub fn disk_size(&self) -> u64 {
        self.disk.len() as u64
    }

    // ── Internal helpers ──

    /// Compute the 28-bit LBA from the current task file registers.
    fn lba28(&self) -> u64 {
        let lba = (self.sector_number as u64)
            | ((self.cylinder_low as u64) << 8)
            | ((self.cylinder_high as u64) << 16)
            | (((self.drive_head & 0x0F) as u64) << 24);
        lba
    }

    /// Compute the 48-bit LBA from current + HOB registers.
    fn lba48(&self) -> u64 {
        let lo = (self.sector_number as u64)
            | ((self.cylinder_low as u64) << 8)
            | ((self.cylinder_high as u64) << 16);
        let hi = (self.hob_sector_number as u64)
            | ((self.hob_cylinder_low as u64) << 8)
            | ((self.hob_cylinder_high as u64) << 16);
        lo | (hi << 24)
    }

    /// Read one sector from the disk image into the buffer.
    fn read_sector(&mut self, lba: u64) {
        let offset = (lba as usize) * SECTOR_SIZE;
        if offset + SECTOR_SIZE <= self.disk.len() {
            self.buffer.copy_from_slice(&self.disk[offset..offset + SECTOR_SIZE]);
        } else {
            // Beyond disk — return zeros.
            self.buffer = [0u8; SECTOR_SIZE];
        }
        self.buffer_offset = 0;
    }

    /// Write the buffer contents to the disk image at the given LBA.
    fn write_sector(&mut self, lba: u64) {
        let offset = (lba as usize) * SECTOR_SIZE;
        if offset + SECTOR_SIZE <= self.disk.len() {
            self.disk[offset..offset + SECTOR_SIZE].copy_from_slice(&self.buffer);
        }
        // Writes beyond the disk boundary are silently ignored.
    }

    /// Current LBA for the ongoing transfer, computed from the task file.
    fn current_lba(&self) -> u64 {
        if self.drive_head & 0x40 != 0 {
            // LBA mode
            self.lba28()
        } else {
            // CHS mode — convert to LBA (simplified: treat as LBA anyway)
            self.lba28()
        }
    }

    /// Advance the LBA in the task file registers after one sector.
    fn advance_lba(&mut self) {
        let lba = self.current_lba() + 1;
        self.sector_number = (lba & 0xFF) as u8;
        self.cylinder_low = ((lba >> 8) & 0xFF) as u8;
        self.cylinder_high = ((lba >> 16) & 0xFF) as u8;
        self.drive_head = (self.drive_head & 0xF0) | ((lba >> 24) & 0x0F) as u8;
    }

    /// Fill the identify buffer with drive information.
    fn fill_identify(&mut self) {
        self.buffer = [0u8; SECTOR_SIZE];
        let w = |buf: &mut [u8; 512], idx: usize, val: u16| {
            let off = idx * 2;
            buf[off] = val as u8;
            buf[off + 1] = (val >> 8) as u8;
        };

        // Word 0: General config — fixed disk, not removable.
        w(&mut self.buffer, 0, 0x0040);

        // Words 1, 3, 6: Legacy CHS geometry.
        let cyls = (self.total_sectors / (16 * 63)).min(16383) as u16;
        w(&mut self.buffer, 1, cyls);         // cylinders
        w(&mut self.buffer, 3, 16);           // heads
        w(&mut self.buffer, 6, 63);           // sectors per track

        // Words 10-19: Serial number (ASCII, swapped bytes).
        let serial = b"COREVM00000000000001";
        for i in 0..10 {
            let hi = serial[i * 2];
            let lo = serial[i * 2 + 1];
            w(&mut self.buffer, 10 + i, ((hi as u16) << 8) | lo as u16);
        }

        // Words 23-26: Firmware revision.
        let fw = b"1.0     ";
        for i in 0..4 {
            let hi = fw[i * 2];
            let lo = fw[i * 2 + 1];
            w(&mut self.buffer, 23 + i, ((hi as u16) << 8) | lo as u16);
        }

        // Words 27-46: Model number.
        let model = b"CoreVM Virtual Disk                     ";
        for i in 0..20 {
            let hi = model[i * 2];
            let lo = model[i * 2 + 1];
            w(&mut self.buffer, 27 + i, ((hi as u16) << 8) | lo as u16);
        }

        // Word 47: Max sectors per READ/WRITE MULTIPLE.
        w(&mut self.buffer, 47, 0x8010); // max 16 sectors

        // Word 49: Capabilities — LBA supported, DMA not supported.
        w(&mut self.buffer, 49, 0x0200); // LBA supported

        // Word 53: Fields validity — words 54-58, 64-70, 88 valid.
        w(&mut self.buffer, 53, 0x0007);

        // Words 54-56: Current CHS (same as logical).
        w(&mut self.buffer, 54, cyls);
        w(&mut self.buffer, 55, 16);
        w(&mut self.buffer, 56, 63);

        // Words 57-58: Current capacity in sectors (CHS).
        let chs_sectors = (cyls as u32) * 16 * 63;
        w(&mut self.buffer, 57, chs_sectors as u16);
        w(&mut self.buffer, 58, (chs_sectors >> 16) as u16);

        // Words 60-61: Total addressable sectors (28-bit LBA).
        let lba28_max = self.total_sectors.min(0x0FFF_FFFF) as u32;
        w(&mut self.buffer, 60, lba28_max as u16);
        w(&mut self.buffer, 61, (lba28_max >> 16) as u16);

        // Word 80: ATA major version — ATA-6.
        w(&mut self.buffer, 80, 0x0040);

        // Word 83: Command set support — 48-bit LBA supported.
        w(&mut self.buffer, 83, 0x0400);

        // Word 86: Command set enabled — 48-bit LBA enabled.
        w(&mut self.buffer, 86, 0x0400);

        // Words 100-103: 48-bit total sectors.
        w(&mut self.buffer, 100, self.total_sectors as u16);
        w(&mut self.buffer, 101, (self.total_sectors >> 16) as u16);
        w(&mut self.buffer, 102, (self.total_sectors >> 32) as u16);
        w(&mut self.buffer, 103, (self.total_sectors >> 48) as u16);

        self.buffer_offset = 0;
    }

    /// Execute a command written to the command register.
    fn execute_command(&mut self, cmd: u8) {
        // Only drive 0 (master) is present.
        if self.drive_head & 0x10 != 0 {
            // Drive 1 selected — abort.
            self.status = SR_DRDY | SR_ERR;
            self.error = ER_ABRT;
            return;
        }

        match cmd {
            CMD_IDENTIFY => {
                if self.total_sectors == 0 {
                    // No disk attached.
                    self.status = SR_DRDY | SR_ERR;
                    self.error = ER_ABRT;
                    return;
                }
                self.fill_identify();
                self.status = SR_DRDY | SR_DRQ | SR_DSC;
                self.error = 0;
                self.irq_pending = true;
            }

            CMD_READ_SECTORS => {
                let count = if self.sector_count == 0 { 256u32 } else { self.sector_count as u32 };
                let lba = self.lba28();
                self.start_read(lba, count);
            }

            CMD_READ_SECTORS_EXT => {
                let c = ((self.hob_sector_count as u32) << 8) | self.sector_count as u32;
                let count = if c == 0 { 65536u32 } else { c };
                let lba = self.lba48();
                self.start_read(lba, count);
            }

            CMD_WRITE_SECTORS => {
                let count = if self.sector_count == 0 { 256u32 } else { self.sector_count as u32 };
                self.sectors_remaining = count;
                self.is_write = true;
                self.buffer_offset = 0;
                self.status = SR_DRDY | SR_DRQ | SR_DSC;
                self.error = 0;
            }

            CMD_WRITE_SECTORS_EXT => {
                let c = ((self.hob_sector_count as u32) << 8) | self.sector_count as u32;
                let count = if c == 0 { 65536u32 } else { c };
                self.sectors_remaining = count;
                self.is_write = true;
                self.buffer_offset = 0;
                self.status = SR_DRDY | SR_DRQ | SR_DSC;
                self.error = 0;
            }

            CMD_READ_MULTIPLE => {
                let count = if self.sector_count == 0 { 256u32 } else { self.sector_count as u32 };
                let lba = self.lba28();
                self.start_read(lba, count);
            }

            CMD_WRITE_MULTIPLE => {
                let count = if self.sector_count == 0 { 256u32 } else { self.sector_count as u32 };
                self.sectors_remaining = count;
                self.is_write = true;
                self.buffer_offset = 0;
                self.status = SR_DRDY | SR_DRQ | SR_DSC;
                self.error = 0;
            }

            CMD_SET_MULTIPLE => {
                if self.sector_count > 0 && self.sector_count <= 128 {
                    self.multiple_count = self.sector_count;
                    self.status = SR_DRDY | SR_DSC;
                    self.error = 0;
                } else {
                    self.status = SR_DRDY | SR_ERR;
                    self.error = ER_ABRT;
                }
                self.irq_pending = true;
            }

            CMD_SET_FEATURES | CMD_INIT_DRIVE_PARAMS | CMD_NOP => {
                // Accept and do nothing meaningful.
                self.status = SR_DRDY | SR_DSC;
                self.error = 0;
                self.irq_pending = true;
            }

            CMD_FLUSH_CACHE => {
                // No write-back cache to flush.
                self.status = SR_DRDY | SR_DSC;
                self.error = 0;
                self.irq_pending = true;
            }

            CMD_DEVICE_RESET => {
                self.status = SR_DRDY | SR_DSC;
                self.error = 0x01; // Diagnostic code: no error
                self.sector_count = 1;
                self.sector_number = 1;
                self.cylinder_low = 0;
                self.cylinder_high = 0;
                self.drive_head = 0;
                self.irq_pending = true;
            }

            _ => {
                // Unknown command — abort.
                self.status = SR_DRDY | SR_ERR;
                self.error = ER_ABRT;
                self.irq_pending = true;
            }
        }
    }

    /// Begin a PIO read transfer.
    fn start_read(&mut self, lba: u64, count: u32) {
        if lba >= self.total_sectors {
            self.status = SR_DRDY | SR_ERR;
            self.error = ER_ABRT;
            self.irq_pending = true;
            return;
        }
        self.sectors_remaining = count;
        self.is_write = false;
        self.read_sector(lba);
        self.sectors_remaining -= 1;
        self.status = SR_DRDY | SR_DRQ | SR_DSC;
        self.error = 0;
        self.irq_pending = true;
    }

    /// Handle a 16-bit read from the data register (port 0x1F0).
    fn read_data_word(&mut self) -> u16 {
        if self.status & SR_DRQ == 0 {
            return 0xFFFF;
        }

        let off = self.buffer_offset;
        let word = if off + 1 < SECTOR_SIZE {
            (self.buffer[off] as u16) | ((self.buffer[off + 1] as u16) << 8)
        } else {
            0
        };
        self.buffer_offset += 2;

        // End of sector?
        if self.buffer_offset >= SECTOR_SIZE {
            if self.sectors_remaining > 0 {
                // Load next sector.
                self.advance_lba();
                let lba = self.current_lba();
                self.read_sector(lba);
                self.sectors_remaining -= 1;
                self.irq_pending = true;
            } else {
                // Transfer complete.
                self.status = SR_DRDY | SR_DSC;
                self.buffer_offset = 0;
                self.irq_pending = true;
            }
        }

        word
    }

    /// Handle a 16-bit write to the data register (port 0x1F0).
    fn write_data_word(&mut self, val: u16) {
        if self.status & SR_DRQ == 0 || !self.is_write {
            return;
        }

        let off = self.buffer_offset;
        if off + 1 < SECTOR_SIZE {
            self.buffer[off] = val as u8;
            self.buffer[off + 1] = (val >> 8) as u8;
        }
        self.buffer_offset += 2;

        // End of sector?
        if self.buffer_offset >= SECTOR_SIZE {
            // Write this sector to disk.
            let lba = self.current_lba();
            self.write_sector(lba);
            self.sectors_remaining -= 1;

            if self.sectors_remaining > 0 {
                // Prepare for next sector.
                self.advance_lba();
                self.buffer_offset = 0;
                self.irq_pending = true;
            } else {
                // Transfer complete.
                self.status = SR_DRDY | SR_DSC;
                self.is_write = false;
                self.buffer_offset = 0;
                self.irq_pending = true;
            }
        }
    }
}

impl IoHandler for Ide {
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        match port {
            // Data register — 16-bit PIO reads.
            0x1F0 => {
                if size >= 2 {
                    Ok(self.read_data_word() as u32)
                } else {
                    // 8-bit read from data port — return low byte.
                    let w = self.read_data_word();
                    Ok((w & 0xFF) as u32)
                }
            }
            // Error register (read).
            0x1F1 => Ok(self.error as u32),
            // Sector count.
            0x1F2 => Ok(self.sector_count as u32),
            // Sector number / LBA low.
            0x1F3 => Ok(self.sector_number as u32),
            // Cylinder low / LBA mid.
            0x1F4 => Ok(self.cylinder_low as u32),
            // Cylinder high / LBA high.
            0x1F5 => Ok(self.cylinder_high as u32),
            // Drive/head.
            0x1F6 => Ok(self.drive_head as u32),
            // Status register — reading clears pending IRQ.
            0x1F7 => {
                self.irq_pending = false;
                Ok(self.status as u32)
            }
            // Alternate status (port 0x3F6) — does NOT clear IRQ.
            0x3F6 => Ok(self.status as u32),
            // Drive address register (legacy, mostly unused).
            0x3F7 => Ok(0xFF),
            _ => Ok(0xFF),
        }
    }

    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let v = val as u8;
        match port {
            // Data register — 16-bit PIO writes.
            0x1F0 => {
                self.write_data_word(val as u16);
            }
            // Features register (write).
            0x1F1 => {
                self.features = v;
            }
            // Sector count — with HOB toggling for 48-bit mode.
            0x1F2 => {
                if self.hob_toggle {
                    self.hob_sector_count = v;
                } else {
                    self.sector_count = v;
                }
            }
            // Sector number / LBA[7:0].
            0x1F3 => {
                if self.hob_toggle {
                    self.hob_sector_number = v;
                } else {
                    self.sector_number = v;
                }
            }
            // Cylinder low / LBA[15:8].
            0x1F4 => {
                if self.hob_toggle {
                    self.hob_cylinder_low = v;
                } else {
                    self.cylinder_low = v;
                }
            }
            // Cylinder high / LBA[23:16].
            0x1F5 => {
                if self.hob_toggle {
                    self.hob_cylinder_high = v;
                } else {
                    self.cylinder_high = v;
                }
            }
            // Drive/head register.
            0x1F6 => {
                self.drive_head = v;
                // Reset HOB toggle on drive/head write.
                self.hob_toggle = false;
            }
            // Command register — execute command.
            0x1F7 => {
                self.hob_toggle = false;
                self.execute_command(v);
            }
            // Device control register (port 0x3F6).
            0x3F6 => {
                let old = self.device_control;
                self.device_control = v;
                // SRST (bit 2): rising edge triggers software reset.
                if v & 0x04 != 0 && old & 0x04 == 0 {
                    self.status = SR_BSY;
                }
                // SRST clear: complete reset.
                if v & 0x04 == 0 && old & 0x04 != 0 {
                    self.status = SR_DRDY | SR_DSC;
                    self.error = 0x01;
                    self.sector_count = 1;
                    self.sector_number = 1;
                    self.cylinder_low = 0;
                    self.cylinder_high = 0;
                    self.drive_head = 0;
                }
                // Bit 7 = HOB (high order byte) — allows reading back HOB registers.
                if v & 0x80 != 0 {
                    self.hob_toggle = true;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
