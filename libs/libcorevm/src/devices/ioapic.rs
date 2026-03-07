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
const REDIR_REMOTE_IRR: u64 = 1 << 14;
const REDIR_LEVEL_TRIGGERED: u64 = 1 << 15;

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

impl IoApic {
    fn routed_entry(&self, irq: u8) -> Option<(u8, bool)> {
        let idx = irq as usize;
        if idx >= NUM_REDIR_ENTRIES {
            return None;
        }
        let entry = self.redir_table[idx];
        // Bit 16: mask (1 = masked).
        if (entry & (1 << 16)) != 0 {
            return None;
        }
        // Bits 10:8: delivery mode (support fixed for now).
        let delivery_mode = ((entry >> 8) & 0x7) as u8;
        if delivery_mode != 0 {
            return None;
        }
        let vector = (entry & 0xFF) as u8;
        let level_triggered = ((entry >> 15) & 1) != 0;
        Some((vector, level_triggered))
    }

    /// Whether the given IRQ line is actively routed through the IO-APIC.
    pub fn has_route(&self, irq: u8) -> bool {
        self.routed_entry(irq).is_some()
    }

    /// Route an external IRQ pin through the redirection table.
    ///
    /// Returns `(vector, level_triggered)` when the entry is unmasked and
    /// uses fixed delivery mode (0). Unsupported delivery modes are ignored.
    pub fn route_irq(&mut self, irq: u8) -> Option<(u8, bool)> {
        let idx = irq as usize;
        let (vector, level_triggered) = self.routed_entry(irq)?;
        if level_triggered {
            let entry = &mut self.redir_table[idx];
            if (*entry & REDIR_REMOTE_IRR) != 0 {
                return None;
            }
            *entry |= REDIR_REMOTE_IRR;
        }
        Some((vector, level_triggered))
    }

    /// Complete delivery of a level-triggered vector after LAPIC EOI.
    pub fn eoi_vector(&mut self, vector: u8) {
        for entry in self.redir_table.iter_mut() {
            if (*entry & REDIR_LEVEL_TRIGGERED) == 0 {
                continue;
            }
            if (*entry & 0xFF) as u8 == vector {
                *entry &= !REDIR_REMOTE_IRR;
            }
        }
    }

    /// Return a raw redirection table entry for diagnostics.
    pub fn redir_entry(&self, irq: u8) -> u64 {
        let idx = irq as usize;
        if idx < NUM_REDIR_ENTRIES {
            self.redir_table[idx]
        } else {
            0
        }
    }

    /// Read the raw 32-bit value of an MMIO register by its base offset.
    fn read_mmio_register(&self, reg_base: u64) -> u32 {
        match reg_base {
            0x00 => self.reg_select,
            0x10 => self.read_reg(self.reg_select),
            _ => 0,
        }
    }

    /// Write a 32-bit value to an MMIO register by its base offset.
    fn write_mmio_register(&mut self, reg_base: u64, v: u32) {
        match reg_base {
            0x00 => self.reg_select = v,
            0x10 => self.write_reg(self.reg_select, v),
            _ => {}
        }
    }
}

impl MmioHandler for IoApic {
    /// Read from IO-APIC MMIO registers.
    ///
    /// IOREGSEL (offset 0x00) and IOWIN (offset 0x10) are both 32-bit
    /// registers. Sub-dword accesses extract the correct byte(s).
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        // IOREGSEL is at 0x00-0x03, IOWIN at 0x10-0x13.
        // Determine which register and the byte offset within it.
        let (reg_base, byte_off) = match offset {
            0x00..=0x03 => (0x00u64, (offset & 0x3) as u32),
            0x10..=0x13 => (0x10u64, (offset & 0x3) as u32),
            _ => return Ok(0),
        };
        let reg_val = self.read_mmio_register(reg_base);
        let shifted = (reg_val >> (byte_off * 8)) as u64;
        let bits = (size as u32).min(4) * 8;
        let mask = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
        Ok(shifted & mask)
    }

    /// Write to IO-APIC MMIO registers.
    ///
    /// Sub-dword writes perform a read-modify-write to merge partial bytes.
    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()> {
        let (reg_base, byte_off) = match offset {
            0x00..=0x03 => (0x00u64, (offset & 0x3) as u32),
            0x10..=0x13 => (0x10u64, (offset & 0x3) as u32),
            _ => return Ok(()),
        };
        let v = if byte_off == 0 && size >= 4 {
            val as u32
        } else {
            let old = self.read_mmio_register(reg_base);
            let shift = byte_off * 8;
            let bits = (size as u32).min(4) * 8;
            let mask = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
            (old & !(mask << shift)) | (((val as u32) & mask) << shift)
        };
        self.write_mmio_register(reg_base, v);
        Ok(())
    }
}
