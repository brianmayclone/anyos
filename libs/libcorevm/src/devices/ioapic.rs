//! IO-APIC (I/O Advanced Programmable Interrupt Controller) emulation.
//!
//! The IO-APIC routes hardware interrupts from external devices to local
//! APICs in a multi-processor system. It replaces the legacy 8259A PIC
//! for interrupt routing in modern x86 systems.
//!
//! # MMIO Region
//!
//! | Offset | Size | Register |
//! |--------|------|----------|
//! | 0x00 | 4 | IOREGSEL — register select |
//! | 0x10 | 4 | IOWIN — register read/write window |
//!
//! # Indirect Registers (via IOREGSEL/IOWIN)
//!
//! | Index | Register |
//! |-------|----------|
//! | 0x00 | IOAPIC ID |
//! | 0x01 | IOAPIC Version |
//! | 0x02 | IOAPIC Arbitration ID |
//! | 0x10-0x3F | Redirection table (24 entries × 2 dwords each) |

use crate::error::Result;
use crate::memory::mmio::MmioHandler;

/// Number of interrupt redirection entries (IRQ 0-23).
const NUM_REDIR_ENTRIES: usize = 24;

/// IO-APIC device with register-indirect MMIO access.
///
/// The guest selects a register via IOREGSEL (offset 0x00), then
/// reads or writes the value via IOWIN (offset 0x10).
#[derive(Debug)]
pub struct IoApic {
    /// IOREGSEL — selects which internal register to access via IOWIN.
    reg_select: u32,
    /// IOAPIC ID register (bits 27:24 hold the 4-bit APIC ID).
    id: u32,
    /// Redirection table: 24 entries, each 64 bits.
    ///
    /// Each entry controls routing for one interrupt line:
    /// - Bits 7:0   — interrupt vector
    /// - Bits 10:8  — delivery mode (0=Fixed, 1=LowestPri, 2=SMI, etc.)
    /// - Bit 11     — destination mode (0=physical, 1=logical)
    /// - Bit 12     — delivery status (read-only, 0=idle)
    /// - Bit 13     — pin polarity (0=active high, 1=active low)
    /// - Bit 14     — remote IRR (read-only)
    /// - Bit 15     — trigger mode (0=edge, 1=level)
    /// - Bit 16     — mask (1=masked, 0=unmasked)
    /// - Bits 63:56 — destination APIC ID
    redir_table: [u64; NUM_REDIR_ENTRIES],
}

impl IoApic {
    /// Create a new IO-APIC with all interrupts masked.
    pub fn new() -> Self {
        // Default: all entries masked (bit 16 = 1), vector 0.
        let mut redir_table = [0u64; NUM_REDIR_ENTRIES];
        for entry in redir_table.iter_mut() {
            *entry = 1 << 16; // masked
        }

        IoApic {
            reg_select: 0,
            id: 0,
            redir_table,
        }
    }

    /// Read from an indirect register selected by IOREGSEL.
    fn read_reg(&self, index: u32) -> u32 {
        match index {
            // IOAPIC ID: bits 27:24 = ID.
            0x00 => self.id,
            // IOAPIC Version: bits 23:16 = max redir entry (23), bits 7:0 = version (0x20).
            0x01 => ((NUM_REDIR_ENTRIES as u32 - 1) << 16) | 0x20,
            // IOAPIC Arbitration ID.
            0x02 => self.id,
            // Redirection table entries (low dword at even index, high at odd).
            0x10..=0x3F => {
                let entry_index = ((index - 0x10) / 2) as usize;
                if entry_index < NUM_REDIR_ENTRIES {
                    let entry = self.redir_table[entry_index];
                    if (index & 1) == 0 {
                        entry as u32 // low 32 bits
                    } else {
                        (entry >> 32) as u32 // high 32 bits
                    }
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    /// Write to an indirect register selected by IOREGSEL.
    fn write_reg(&mut self, index: u32, val: u32) {
        match index {
            // IOAPIC ID: only bits 27:24 are writable.
            0x00 => self.id = val & 0x0F000000,
            // Version and arbitration ID are read-only.
            0x01 | 0x02 => {}
            // Redirection table entries.
            0x10..=0x3F => {
                let entry_index = ((index - 0x10) / 2) as usize;
                if entry_index < NUM_REDIR_ENTRIES {
                    let entry = &mut self.redir_table[entry_index];
                    if (index & 1) == 0 {
                        // Low 32 bits (preserve high 32).
                        *entry = (*entry & 0xFFFFFFFF_00000000) | (val as u64);
                    } else {
                        // High 32 bits (preserve low 32).
                        *entry = (*entry & 0x00000000_FFFFFFFF) | ((val as u64) << 32);
                    }
                }
            }
            _ => {}
        }
    }
}

impl MmioHandler for IoApic {
    /// Read from IO-APIC MMIO registers.
    fn read(&mut self, offset: u64, _size: u8) -> Result<u64> {
        let val = match offset {
            // IOREGSEL at offset 0x00.
            0x00 => self.reg_select,
            // IOWIN at offset 0x10 — read the selected register.
            0x10 => self.read_reg(self.reg_select),
            _ => 0,
        };
        Ok(val as u64)
    }

    /// Write to IO-APIC MMIO registers.
    fn write(&mut self, offset: u64, _size: u8, val: u64) -> Result<()> {
        match offset {
            // IOREGSEL at offset 0x00 — select a register.
            0x00 => self.reg_select = val as u32,
            // IOWIN at offset 0x10 — write to the selected register.
            0x10 => self.write_reg(self.reg_select, val as u32),
            _ => {}
        }
        Ok(())
    }
}
