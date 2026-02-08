use crate::arch::x86::port::{inb, inw, outb};

// ATA PIO ports (primary controller)
const ATA_DATA: u16 = 0x1F0;
const ATA_ERROR: u16 = 0x1F1;
const ATA_SECTOR_COUNT: u16 = 0x1F2;
const ATA_LBA_LO: u16 = 0x1F3;
const ATA_LBA_MID: u16 = 0x1F4;
const ATA_LBA_HI: u16 = 0x1F5;
const ATA_DRIVE_HEAD: u16 = 0x1F6;
const ATA_STATUS: u16 = 0x1F7;
const ATA_COMMAND: u16 = 0x1F7;

// Status bits
const STATUS_BSY: u8 = 0x80;
const STATUS_DRDY: u8 = 0x40;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;

// Commands
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_IDENTIFY: u8 = 0xEC;

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

fn wait_bsy() {
    unsafe {
        while inb(ATA_STATUS) & STATUS_BSY != 0 {
            core::hint::spin_loop();
        }
    }
}

fn wait_drq() {
    unsafe {
        while inb(ATA_STATUS) & STATUS_DRQ == 0 {
            if inb(ATA_STATUS) & STATUS_ERR != 0 {
                return;
            }
            core::hint::spin_loop();
        }
    }
}

pub fn init() {
    // Try to identify the primary master drive
    unsafe {
        outb(ATA_DRIVE_HEAD, 0xA0); // Select master
        outb(ATA_SECTOR_COUNT, 0);
        outb(ATA_LBA_LO, 0);
        outb(ATA_LBA_MID, 0);
        outb(ATA_LBA_HI, 0);
        outb(ATA_COMMAND, CMD_IDENTIFY);

        // Check if drive exists
        let status = inb(ATA_STATUS);
        if status == 0 {
            crate::serial_println!("  ATA: No primary master drive detected");
            return;
        }

        wait_bsy();

        // Check for non-ATA drives
        if inb(ATA_LBA_MID) != 0 || inb(ATA_LBA_HI) != 0 {
            crate::serial_println!("  ATA: Non-ATA device on primary master");
            return;
        }

        wait_drq();

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
        wait_bsy();

        outb(ATA_DRIVE_HEAD, 0xE0 | ((lba >> 24) & 0x0F) as u8); // LBA mode, master
        outb(ATA_SECTOR_COUNT, count);
        outb(ATA_LBA_LO, (lba & 0xFF) as u8);
        outb(ATA_LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ATA_LBA_HI, ((lba >> 16) & 0xFF) as u8);
        outb(ATA_COMMAND, CMD_READ_SECTORS);

        for sector in 0..count as usize {
            wait_bsy();
            wait_drq();

            if inb(ATA_STATUS) & STATUS_ERR != 0 {
                return false;
            }

            let offset = sector * 512;
            for i in (0..512).step_by(2) {
                let word = inw(ATA_DATA);
                buf[offset + i] = word as u8;
                buf[offset + i + 1] = (word >> 8) as u8;
            }
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
        wait_bsy();

        outb(ATA_DRIVE_HEAD, 0xE0 | ((lba >> 24) & 0x0F) as u8);
        outb(ATA_SECTOR_COUNT, count);
        outb(ATA_LBA_LO, (lba & 0xFF) as u8);
        outb(ATA_LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ATA_LBA_HI, ((lba >> 16) & 0xFF) as u8);
        outb(ATA_COMMAND, CMD_WRITE_SECTORS);

        for sector in 0..count as usize {
            wait_bsy();
            wait_drq();

            let offset = sector * 512;
            for i in (0..512).step_by(2) {
                let word = (buf[offset + i + 1] as u16) << 8 | buf[offset + i] as u16;
                crate::arch::x86::port::outw(ATA_DATA, word);
            }
        }

        // Flush cache
        outb(ATA_COMMAND, 0xE7);
        wait_bsy();
    }

    true
}
