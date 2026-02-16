//! ATAPI (ATA Packet Interface) driver for CD-ROM/DVD-ROM devices.
//!
//! Supports SCSI commands over ATA using the PACKET command (0xA0).
//! CD-ROM/DVD-ROM devices use 2048-byte logical blocks.
//! Scans primary and secondary IDE channels for ATAPI devices.

use crate::arch::x86::port::{inb, inw, outb, outw};

// ATA I/O port bases
const PRIMARY_BASE: u16 = 0x1F0;
const PRIMARY_CTRL: u16 = 0x3F6;
const SECONDARY_BASE: u16 = 0x170;
const SECONDARY_CTRL: u16 = 0x376;

// Register offsets from base
const REG_DATA: u16 = 0;
const REG_FEATURES: u16 = 1;
const REG_BYTE_COUNT_LO: u16 = 4;
const REG_BYTE_COUNT_HI: u16 = 5;
const REG_DRIVE_HEAD: u16 = 6;
const REG_STATUS: u16 = 7;
const REG_COMMAND: u16 = 7;

// Status bits
const STATUS_BSY: u8 = 0x80;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;

// Commands
const CMD_IDENTIFY_PACKET: u8 = 0xA1;
const CMD_PACKET: u8 = 0xA0;

// SCSI command opcodes
const SCSI_READ_10: u8 = 0x28;
const SCSI_READ_CAPACITY: u8 = 0x25;

/// CD-ROM logical block size (2048 bytes).
pub const CDROM_SECTOR_SIZE: usize = 2048;

/// Detected ATAPI drive information.
pub struct AtapiDrive {
    pub present: bool,
    base: u16,
    ctrl: u16,
    slave: bool,
    pub model: [u8; 40],
    pub capacity_lba: u32,
}

static mut ATAPI_DRIVE: AtapiDrive = AtapiDrive {
    present: false,
    base: 0,
    ctrl: 0,
    slave: false,
    model: [0; 40],
    capacity_lba: 0,
};

/// Check if an ATAPI drive was detected.
pub fn is_present() -> bool {
    unsafe { ATAPI_DRIVE.present }
}

/// Get the total number of 2048-byte blocks on the disc.
pub fn capacity_lba() -> u32 {
    unsafe { ATAPI_DRIVE.capacity_lba }
}

fn wait_bsy(base: u16) {
    unsafe {
        for _ in 0..500_000u32 {
            if inb(base + REG_STATUS) & STATUS_BSY == 0 {
                return;
            }
            core::hint::spin_loop();
        }
    }
}

fn wait_drq(base: u16) -> bool {
    unsafe {
        for _ in 0..500_000u32 {
            let status = inb(base + REG_STATUS);
            if status & STATUS_ERR != 0 {
                return false;
            }
            if status & STATUS_DRQ != 0 {
                return true;
            }
            core::hint::spin_loop();
        }
    }
    false
}

/// Try to detect an ATAPI device on the given channel.
fn try_identify_atapi(base: u16, ctrl: u16, slave: bool) -> Option<AtapiDrive> {
    unsafe {
        let drive_sel = if slave { 0xB0 } else { 0xA0 };

        // Select drive
        outb(base + REG_DRIVE_HEAD, drive_sel);
        // 400ns delay (read alternate status 4 times)
        for _ in 0..4 { inb(ctrl); }

        // Clear registers
        outb(base + 2, 0); // sector count
        outb(base + 3, 0); // LBA lo
        outb(base + REG_BYTE_COUNT_LO, 0);
        outb(base + REG_BYTE_COUNT_HI, 0);

        // Issue IDENTIFY DEVICE (0xEC) to detect signature
        outb(base + REG_COMMAND, 0xEC);

        // Check if drive exists
        let status = inb(base + REG_STATUS);
        if status == 0 || status == 0xFF {
            return None;
        }

        wait_bsy(base);

        // Check ATAPI signature: LBA_MID=0x14, LBA_HI=0xEB
        let sig_mid = inb(base + REG_BYTE_COUNT_LO);
        let sig_hi = inb(base + REG_BYTE_COUNT_HI);

        if sig_mid != 0x14 || sig_hi != 0xEB {
            return None;
        }

        // It's ATAPI — issue IDENTIFY PACKET DEVICE (0xA1)
        outb(base + REG_COMMAND, CMD_IDENTIFY_PACKET);
        wait_bsy(base);

        if !wait_drq(base) {
            return None;
        }

        // Read 256 words of identify data
        let mut identify = [0u16; 256];
        for word in identify.iter_mut() {
            *word = inw(base + REG_DATA);
        }

        // Parse model string (words 27-46, byte-swapped)
        let mut model = [0u8; 40];
        for i in 0..20 {
            model[i * 2] = (identify[27 + i] >> 8) as u8;
            model[i * 2 + 1] = identify[27 + i] as u8;
        }

        Some(AtapiDrive {
            present: true,
            base,
            ctrl,
            slave,
            model,
            capacity_lba: 0,
        })
    }
}

/// Read the capacity of the CD-ROM (total LBAs and block size).
fn read_capacity(drive: &AtapiDrive) -> Option<(u32, u32)> {
    let mut packet = [0u8; 12];
    packet[0] = SCSI_READ_CAPACITY;

    let mut buf = [0u8; 8];
    if send_packet_read(drive, &packet, &mut buf) {
        let last_lba = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let block_size = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        Some((last_lba.saturating_add(1), block_size))
    } else {
        None
    }
}

/// Send a 12-byte SCSI packet and read response data.
fn send_packet_read(drive: &AtapiDrive, packet: &[u8; 12], buf: &mut [u8]) -> bool {
    unsafe {
        let base = drive.base;
        let ctrl = drive.ctrl;
        let drive_sel = if drive.slave { 0xB0 } else { 0xA0 };

        outb(base + REG_DRIVE_HEAD, drive_sel);
        for _ in 0..4 { inb(ctrl); }

        wait_bsy(base);

        // Set max byte count limit
        outb(base + REG_FEATURES, 0);
        outb(base + REG_BYTE_COUNT_LO, 0xFE);
        outb(base + REG_BYTE_COUNT_HI, 0xFF);

        // Issue PACKET command
        outb(base + REG_COMMAND, CMD_PACKET);
        wait_bsy(base);

        if !wait_drq(base) {
            return false;
        }

        // Send 12-byte packet as 6 little-endian words
        for i in 0..6 {
            let word = (packet[i * 2 + 1] as u16) << 8 | packet[i * 2] as u16;
            outw(base + REG_DATA, word);
        }

        // Read data in DRQ phases
        let mut offset = 0usize;
        loop {
            wait_bsy(base);

            let status = inb(base + REG_STATUS);
            if status & STATUS_ERR != 0 {
                return offset > 0; // Partial read OK
            }
            if status & STATUS_DRQ == 0 {
                break; // Done
            }

            let byte_count = (inb(base + REG_BYTE_COUNT_HI) as usize) << 8
                           | inb(base + REG_BYTE_COUNT_LO) as usize;

            let words = byte_count / 2;
            for _ in 0..words {
                let word = inw(base + REG_DATA);
                if offset < buf.len() {
                    buf[offset] = word as u8;
                }
                if offset + 1 < buf.len() {
                    buf[offset + 1] = (word >> 8) as u8;
                }
                offset += 2;
            }
        }

        true
    }
}

/// Initialize ATAPI: scan primary and secondary channels for CD/DVD drives.
pub fn init() {
    let positions: [(u16, u16, bool, &str); 4] = [
        (PRIMARY_BASE, PRIMARY_CTRL, false, "primary master"),
        (PRIMARY_BASE, PRIMARY_CTRL, true, "primary slave"),
        (SECONDARY_BASE, SECONDARY_CTRL, false, "secondary master"),
        (SECONDARY_BASE, SECONDARY_CTRL, true, "secondary slave"),
    ];

    for &(base, ctrl, slave, name) in &positions {
        if let Some(mut drive) = try_identify_atapi(base, ctrl, slave) {
            let model_str = core::str::from_utf8(&drive.model).unwrap_or("???").trim();

            if let Some((total_lba, block_size)) = read_capacity(&drive) {
                drive.capacity_lba = total_lba;
                let size_mb = (total_lba as u64 * block_size as u64) / (1024 * 1024);
                crate::serial_println!(
                    "[OK] ATAPI {}: '{}', {} blocks x {} bytes ({} MiB)",
                    name, model_str, total_lba, block_size, size_mb
                );
            } else {
                crate::serial_println!(
                    "[OK] ATAPI {}: '{}' (no media or capacity unavailable)",
                    name, model_str
                );
            }

            unsafe { ATAPI_DRIVE = drive; }
            return;
        }
    }
}

/// Read 2048-byte logical blocks from the CD-ROM.
///
/// `lba`: starting logical block address (2048-byte blocks)
/// `count`: number of blocks to read
/// `buf`: output buffer (must be >= count * 2048 bytes)
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    let drive = unsafe { &ATAPI_DRIVE };
    if !drive.present {
        return false;
    }

    let total_bytes = count as usize * CDROM_SECTOR_SIZE;
    if buf.len() < total_bytes {
        return false;
    }

    // Read in chunks of up to 32 blocks (64 KiB per transfer)
    let max_blocks: u32 = 32;
    let mut remaining = count;
    let mut cur_lba = lba;
    let mut offset = 0usize;

    while remaining > 0 {
        let batch = remaining.min(max_blocks);
        let batch_bytes = batch as usize * CDROM_SECTOR_SIZE;

        // Build SCSI READ(10) command
        let mut packet = [0u8; 12];
        packet[0] = SCSI_READ_10;
        packet[2] = ((cur_lba >> 24) & 0xFF) as u8;
        packet[3] = ((cur_lba >> 16) & 0xFF) as u8;
        packet[4] = ((cur_lba >> 8) & 0xFF) as u8;
        packet[5] = (cur_lba & 0xFF) as u8;
        packet[7] = ((batch >> 8) & 0xFF) as u8;
        packet[8] = (batch & 0xFF) as u8;

        if !send_packet_read(drive, &packet, &mut buf[offset..offset + batch_bytes]) {
            return false;
        }

        offset += batch_bytes;
        cur_lba += batch;
        remaining -= batch;
    }

    true
}

/// Read data using 512-byte sector addressing (for FAT16 compatibility).
///
/// Translates 512-byte sector addresses to 2048-byte CD-ROM blocks.
/// Allocates a temporary buffer for alignment.
pub fn read_sectors_512(lba_512: u32, count_512: u32, buf: &mut [u8]) -> bool {
    let block_start = lba_512 / 4;
    let start_offset = (lba_512 % 4) as usize * 512;
    let end_sector = lba_512 + count_512;
    let block_end = (end_sector + 3) / 4;
    let block_count = block_end - block_start;

    let total_cd_bytes = block_count as usize * CDROM_SECTOR_SIZE;
    let mut temp = alloc::vec![0u8; total_cd_bytes];

    if !read_sectors(block_start, block_count, &mut temp) {
        return false;
    }

    let needed = count_512 as usize * 512;
    if needed > buf.len() {
        return false;
    }
    buf[..needed].copy_from_slice(&temp[start_offset..start_offset + needed]);
    true
}
