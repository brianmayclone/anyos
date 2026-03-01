//! CMOS RTC and NVRAM emulation.
//!
//! Emulates the Motorola MC146818 (or compatible) CMOS real-time clock
//! and 128 bytes of battery-backed NVRAM found in the IBM PC/AT.
//!
//! # I/O Ports
//!
//! | Port | Description |
//! |------|-------------|
//! | 0x70 | Index register (write selects CMOS address; bit 7 controls NMI) |
//! | 0x71 | Data register (read/write the selected CMOS address) |
//!
//! # NVRAM Layout
//!
//! - `0x00-0x09`: RTC time fields
//! - `0x0A-0x0D`: Status registers A-D
//! - `0x0F`: Shutdown status
//! - `0x10`: Floppy drive types
//! - `0x14`: Equipment byte
//! - `0x15-0x16`: Base memory size (KB)
//! - `0x17-0x18`: Extended memory size above 1 MB (KB)
//! - `0x30-0x31`: Extended memory above 1 MB (KB, duplicate)
//! - `0x34-0x35`: Extended memory above 16 MB (64 KB units)

use crate::error::Result;
use crate::io::IoHandler;

/// CMOS RTC and NVRAM controller.
#[derive(Debug)]
pub struct Cmos {
    /// Currently selected CMOS address (low 7 bits of port 0x70 write).
    pub index: u8,
    /// 128 bytes of NVRAM contents.
    pub data: [u8; 128],
    /// NMI disable flag (bit 7 of port 0x70).
    pub nmi_disabled: bool,
}

impl Cmos {
    /// Create a new CMOS device pre-populated with memory size information.
    ///
    /// `ram_size_bytes` is the total guest RAM size. The constructor populates
    /// the base memory (640 KB), extended memory (above 1 MB), and extended
    /// memory above 16 MB fields in the NVRAM.
    pub fn new(ram_size_bytes: usize) -> Self {
        let mut data = [0u8; 128];

        // Status Register A: divider = 010 (32.768 kHz), rate = 0110 (1024 Hz).
        data[0x0A] = 0x26;
        // Status Register B: 24-hour mode, binary (not BCD), no interrupts.
        data[0x0B] = 0x02 | 0x04; // bit 1 = 24h, bit 2 = binary
        // Status Register C: no interrupt flags pending.
        data[0x0C] = 0x00;
        // Status Register D: RTC valid (battery OK).
        data[0x0D] = 0x80;

        // Shutdown status: normal POST.
        data[0x0F] = 0x00;

        // Floppy drive types: none installed.
        data[0x10] = 0x00;

        // Equipment byte: bit 1 = math coprocessor present.
        data[0x14] = 0x02;

        // Base memory: 640 KB (0x0280).
        data[0x15] = 0x80; // low byte
        data[0x16] = 0x02; // high byte

        // Extended memory above 1 MB (in KB), capped at 64 MB - 1 MB = 63 MB.
        // Both register pairs (0x17-0x18 and 0x30-0x31) hold the same value.
        let extended_kb = if ram_size_bytes > 1024 * 1024 {
            let ext = (ram_size_bytes - 1024 * 1024) / 1024;
            // Cap at 0xFFFF (65535 KB = ~64 MB) for the 16-bit field.
            if ext > 0xFFFF { 0xFFFF } else { ext as u16 }
        } else {
            0
        };
        data[0x17] = extended_kb as u8;
        data[0x18] = (extended_kb >> 8) as u8;
        data[0x30] = extended_kb as u8;
        data[0x31] = (extended_kb >> 8) as u8;

        // Extended memory above 16 MB (in 64 KB units).
        let above_16mb = if ram_size_bytes > 16 * 1024 * 1024 {
            let units = (ram_size_bytes - 16 * 1024 * 1024) / (64 * 1024);
            if units > 0xFFFF { 0xFFFF } else { units as u16 }
        } else {
            0
        };
        data[0x34] = above_16mb as u8;
        data[0x35] = (above_16mb >> 8) as u8;

        Cmos {
            index: 0,
            data,
            nmi_disabled: false,
        }
    }
}

impl IoHandler for Cmos {
    /// Read from CMOS ports.
    ///
    /// - Port 0x70: not readable (returns 0xFF)
    /// - Port 0x71: returns the NVRAM byte at the currently selected index.
    ///   Reading status register C (0x0C) clears all interrupt flags.
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        let val = match port {
            0x71 => {
                let idx = (self.index & 0x7F) as usize;
                let v = self.data[idx];
                // Reading status register C clears all interrupt flags.
                if idx == 0x0C {
                    self.data[0x0C] = 0x00;
                }
                v
            }
            _ => 0xFF,
        };
        Ok(val as u32)
    }

    /// Write to CMOS ports.
    ///
    /// - Port 0x70: selects the CMOS address (bits 0-6) and NMI disable
    ///   flag (bit 7).
    /// - Port 0x71: writes a byte to the NVRAM at the currently selected
    ///   index. Status registers C and D are read-only and writes are
    ///   ignored.
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let byte = val as u8;
        match port {
            0x70 => {
                self.index = byte & 0x7F;
                self.nmi_disabled = byte & 0x80 != 0;
            }
            0x71 => {
                let idx = (self.index & 0x7F) as usize;
                // Status register C (0x0C) and D (0x0D) are read-only.
                if idx != 0x0C && idx != 0x0D {
                    self.data[idx] = byte;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
