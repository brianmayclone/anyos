//! ATA PIO disk driver for the primary IDE controller.
//!
//! Supports 28-bit LBA sector read/write via I/O ports 0x1F0-0x1F7.
//! Detects the primary master drive via IDENTIFY and provides sector-level access.

use crate::arch::x86::port::{inb, inw, outb};

// ATA PIO ports (primary controller)
/// ATA data register (16-bit read/write).
const ATA_DATA: u16 = 0x1F0;
const ATA_ERROR: u16 = 0x1F1;
const ATA_SECTOR_COUNT: u16 = 0x1F2;
const ATA_LBA_LO: u16 = 0x1F3;
const ATA_LBA_MID: u16 = 0x1F4;
const ATA_LBA_HI: u16 = 0x1F5;
const ATA_DRIVE_HEAD: u16 = 0x1F6;
const ATA_STATUS: u16 = 0x1F7;
const ATA_COMMAND: u16 = 0x1F7;
/// Device Control Register / Alternate Status (write: control, read: alt status).
/// Bit 1 = nIEN: when set, the device does not assert INTRQ after transfers.
/// This is mandatory for pure PIO-polling drivers — without it, VirtualBox
/// stalls subsequent commands on a single CPU because no IRQ handler clears
/// the pending interrupt line.
const ATA_DEV_CTRL: u16 = 0x3F6;

// Status bits
const STATUS_BSY: u8 = 0x80;
const STATUS_DRDY: u8 = 0x40;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;

// Commands
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_IDENTIFY: u8 = 0xEC;

/// Detected ATA drive information (model, sector count, master/slave).
pub struct AtaDrive {
    pub present: bool,
    pub slave: bool,
    pub sectors: u32,
    pub model: [u8; 40],
}

static mut PRIMARY_DRIVE: AtaDrive = AtaDrive {
    present: false,
    slave: false,
    sectors: 0,
    model: [0; 40],
};

fn wait_bsy() -> bool {
    unsafe {
        for _ in 0..1_000_000u32 {
            let s = inb(ATA_STATUS);
            if s == 0xFF {
                return false; // floating bus — no controller
            }
            if s & STATUS_BSY == 0 {
                return true;
            }
            core::hint::spin_loop();
        }
        false // timeout
    }
}

fn wait_drq() -> bool {
    unsafe {
        for _ in 0..1_000_000u32 {
            let s = inb(ATA_STATUS);
            if s == 0xFF {
                return false; // floating bus
            }
            if s & STATUS_ERR != 0 {
                return false;
            }
            if s & STATUS_DRQ != 0 {
                return true;
            }
            core::hint::spin_loop();
        }
        false // timeout
    }
}

/// 400 ns delay required by ATA spec after writing the Command register.
/// Reading the Alternate Status register 4 times costs ≈ 4×100 ns = 400 ns
/// and does NOT clear the interrupt flag (that is only done by reading 0x1F7).
#[inline]
fn ata_delay_400ns() {
    unsafe {
        inb(ATA_DEV_CTRL);
        inb(ATA_DEV_CTRL);
        inb(ATA_DEV_CTRL);
        inb(ATA_DEV_CTRL);
    }
}

/// Detect and identify the primary master ATA drive.
pub fn init() {
    // Try to identify the primary master drive
    unsafe {
        // Quick floating-bus check before touching any controller registers.
        // If there's no IDE controller (e.g. VirtualBox in AHCI mode), all
        // I/O ports return 0xFF. Detect this early to avoid sending commands
        // into the void and then hanging in wait_bsy().
        let probe = inb(ATA_STATUS);
        if probe == 0xFF {
            crate::serial_println!("  ATA: No IDE controller (floating bus)");
            return;
        }

        // Disable IRQ generation for PIO mode (nIEN = 1).
        // Without this, the ATA controller asserts INTRQ after every transfer.
        // On a single CPU with no IRQ 14 handler registered, VirtualBox refuses
        // to assert DRQ for the next command because the previous IRQ is still
        // pending — causing wait_drq() to spin forever on the 2nd+ read.
        outb(ATA_DEV_CTRL, 0x02); // nIEN = 1

        outb(ATA_DRIVE_HEAD, 0xA0); // Select master
        outb(ATA_SECTOR_COUNT, 0);
        outb(ATA_LBA_LO, 0);
        outb(ATA_LBA_MID, 0);
        outb(ATA_LBA_HI, 0);
        outb(ATA_COMMAND, CMD_IDENTIFY);

        // Check if drive exists
        let status = inb(ATA_STATUS);
        if status == 0 || status == 0xFF {
            crate::serial_println!("  ATA: No primary master drive detected");
            return;
        }

        if !wait_bsy() {
            crate::serial_println!("  ATA: IDENTIFY timed out (BSY stuck)");
            return;
        }

        // Check for non-ATA drives
        if inb(ATA_LBA_MID) != 0 || inb(ATA_LBA_HI) != 0 {
            crate::serial_println!("  ATA: Non-ATA device on primary master");
            return;
        }

        if !wait_drq() {
            crate::serial_println!("  ATA: IDENTIFY failed (no DRQ)");
            return;
        }

        // Read 256 words of identify data
        let mut identify = [0u16; 256];
        for word in identify.iter_mut() {
            *word = inw(ATA_DATA);
        }

        // Parse model string (words 27-46, swapped byte order)
        let mut model = [0u8; 40];
        for i in 0..20 {
            model[i * 2] = (identify[27 + i] >> 8) as u8;
            model[i * 2 + 1] = identify[27 + i] as u8;
        }

        // Get sector count (LBA28: word 60-61)
        let sectors = (identify[61] as u32) << 16 | identify[60] as u32;

        PRIMARY_DRIVE = AtaDrive {
            present: true,
            slave: false,
            sectors,
            model,
        };

        let model_str = core::str::from_utf8(&model).unwrap_or("???").trim();
        crate::serial_println!(
            "[OK] ATA drive: '{}', {} sectors ({} MiB)",
            model_str,
            sectors,
            sectors / 2048
        );
    }
}

/// Read sectors from the primary drive using LBA28 PIO
pub fn read_sectors(lba: u32, count: u8, buf: &mut [u8]) -> bool {
    if !unsafe { PRIMARY_DRIVE.present } {
        return false;
    }
    if buf.len() < (count as usize) * 512 {
        return false;
    }

    unsafe {
        if !wait_bsy() { return false; }

        outb(ATA_DRIVE_HEAD, 0xE0 | ((lba >> 24) & 0x0F) as u8); // LBA mode, master
        outb(ATA_SECTOR_COUNT, count);
        outb(ATA_LBA_LO, (lba & 0xFF) as u8);
        outb(ATA_LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ATA_LBA_HI, ((lba >> 16) & 0xFF) as u8);
        outb(ATA_COMMAND, CMD_READ_SECTORS);

        // ATA spec: wait ≥ 400 ns after writing the Command register
        // before polling the Status register for the first time.
        ata_delay_400ns();

        for sector in 0..count as usize {
            if !wait_bsy() { return false; }
            if !wait_drq() { return false; }

            if inb(ATA_STATUS) & STATUS_ERR != 0 {
                return false;
            }

            let offset = sector * 512;
            for i in (0..512).step_by(2) {
                let word = inw(ATA_DATA);
                buf[offset + i] = word as u8;
                buf[offset + i + 1] = (word >> 8) as u8;
            }

            // Reading the Status register (0x1F7) after the data clears the
            // interrupt condition inside the device, allowing it to assert DRQ
            // for the next sector even with nIEN=1.
            let _ = inb(ATA_STATUS);
        }
    }

    true
}

/// Write sectors to the primary drive using LBA28 PIO
pub fn write_sectors(lba: u32, count: u8, buf: &[u8]) -> bool {
    if !unsafe { PRIMARY_DRIVE.present } {
        return false;
    }
    if buf.len() < (count as usize) * 512 {
        return false;
    }

    unsafe {
        if !wait_bsy() { return false; }

        outb(ATA_DRIVE_HEAD, 0xE0 | ((lba >> 24) & 0x0F) as u8);
        outb(ATA_SECTOR_COUNT, count);
        outb(ATA_LBA_LO, (lba & 0xFF) as u8);
        outb(ATA_LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ATA_LBA_HI, ((lba >> 16) & 0xFF) as u8);
        outb(ATA_COMMAND, CMD_WRITE_SECTORS);

        ata_delay_400ns();

        for sector in 0..count as usize {
            if !wait_bsy() { return false; }
            if !wait_drq() { return false; }

            let offset = sector * 512;
            for i in (0..512).step_by(2) {
                let word = (buf[offset + i + 1] as u16) << 8 | buf[offset + i] as u16;
                crate::arch::x86::port::outw(ATA_DATA, word);
            }

            let _ = inb(ATA_STATUS);
        }

        // Flush cache
        outb(ATA_COMMAND, 0xE7);
        ata_delay_400ns();
        let _ = wait_bsy();
    }

    true
}
