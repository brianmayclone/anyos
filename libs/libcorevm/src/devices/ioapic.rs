//! IO-APIC (I/O Advanced Programmable Interrupt Controller) emulation.
//!
//! Port of QEMU's `hw/intc/ioapic.c` to Rust.  Key behaviours:
//!
//! - **IRR** tracks pending interrupts per pin, even when masked (level) or
//!   unmasked (edge).
//! - **Level tracking** (`irq_level`): level-triggered IRR is set on assert,
//!   cleared on deassert.  Edge-triggered IRR is only set if pin is unmasked.
//! - **Read-only bits** (Remote IRR, Delivery Status) are preserved on writes.
//! - **`ioapic_fix_edge_remote_irr`**: switching trigger mode to edge clears
//!   Remote IRR (Linux kernel workaround).
//! - **service()** after every redir-table write and EOI.

use crate::error::Result;
use crate::memory::mmio::MmioHandler;

const NUM_REDIR_ENTRIES: usize = 24;

// Redirection entry bit positions
const REDIR_DELIV_MODE_SHIFT: u32 = 8;
const REDIR_DEST_MODE_BIT: u64 = 1 << 11;
const REDIR_DELIV_STATUS: u64 = 1 << 12;
const REDIR_POLARITY_BIT: u64 = 1 << 13;
const REDIR_REMOTE_IRR: u64 = 1 << 14;
const REDIR_LEVEL_TRIGGERED: u64 = 1 << 15;
const REDIR_MASKED: u64 = 1 << 16;

/// Read-only bits that the guest cannot modify via MMIO writes.
/// Matches QEMU's IOAPIC_RO_BITS = REMOTE_IRR | DELIV_STATUS.
const REDIR_RO_BITS: u64 = REDIR_REMOTE_IRR | REDIR_DELIV_STATUS;

#[derive(Debug)]
pub struct IoApic {
    reg_select: u32,
    id: u32,
    redir_table: [u64; NUM_REDIR_ENTRIES],
    /// Interrupt Request Register — one bit per pin.
    irr: u32,
    /// Pin level state for level-triggered handling.
    /// Matches QEMU's `irq_level[IOAPIC_NUM_PINS]`.
    irq_level: [bool; NUM_REDIR_ENTRIES],
    /// Service output buffer — vectors to forward to LAPIC.
    service_out: [(u8, bool); NUM_REDIR_ENTRIES],
    service_out_count: usize,
}

impl IoApic {
    pub fn new() -> Self {
        let mut redir_table = [0u64; NUM_REDIR_ENTRIES];
        for entry in redir_table.iter_mut() {
            *entry = REDIR_MASKED;
        }
        IoApic {
            reg_select: 0,
            id: 0,
            redir_table,
            irr: 0,
            irq_level: [false; NUM_REDIR_ENTRIES],
            service_out: [(0, false); NUM_REDIR_ENTRIES],
            service_out_count: 0,
        }
    }

    // ── Register access ───────────────────────────────────────────────

    fn read_reg(&self, index: u32) -> u32 {
        match index {
            0x00 => self.id,
            0x01 => ((NUM_REDIR_ENTRIES as u32 - 1) << 16) | 0x20,
            0x02 => self.id,
            0x10..=0x3F => {
                let idx = ((index - 0x10) / 2) as usize;
                if idx < NUM_REDIR_ENTRIES {
                    let entry = self.redir_table[idx];
                    if (index & 1) == 0 { entry as u32 } else { (entry >> 32) as u32 }
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    fn write_reg(&mut self, index: u32, val: u32) {
        match index {
            0x00 => self.id = val & 0x0F000000,
            0x01 | 0x02 => {}
            0x10..=0x3F => {
                let idx = ((index - 0x10) / 2) as usize;
                if idx < NUM_REDIR_ENTRIES {
                    let entry = &mut self.redir_table[idx];
                    // Preserve read-only bits (Remote IRR, Delivery Status)
                    let ro_bits = *entry & REDIR_RO_BITS;
                    if (index & 1) == 0 {
                        // Low 32 bits: merge new value, restore RO bits
                        let new_lo = (val as u64) & !REDIR_RO_BITS;
                        *entry = (*entry & 0xFFFFFFFF_00000000) | new_lo | ro_bits;
                    } else {
                        // High 32 bits (no RO bits in upper half)
                        *entry = (*entry & 0x00000000_FFFFFFFF) | ((val as u64) << 32);
                    }
                    // QEMU: ioapic_fix_edge_remote_irr — if trigger mode is
                    // now edge, clear Remote IRR (Linux kernel workaround for
                    // IOAPICs without EOI register).
                    if (*entry & REDIR_LEVEL_TRIGGERED) == 0 {
                        *entry &= !REDIR_REMOTE_IRR;
                    }
                    #[cfg(feature = "host_test")]
                    {
                        let masked = (*entry >> 16) & 1;
                        let vec = *entry & 0xFF;
                        let dm = (*entry >> 8) & 7;
                        eprintln!("[ioapic] redir write pin={} reg=0x{:02X} val=0x{:08X} -> entry=0x{:016X} masked={} vec=0x{:02X} dm={} irr={:#x}",
                            idx, index, val, *entry, masked, vec, dm, self.irr);
                    }
                    // After every redir-table write, try to deliver pending IRQs.
                    self.service();
                }
            }
            _ => {}
        }
    }

    // ── IRQ assertion / deassertion (QEMU: ioapic_set_irq) ───────────

    /// Assert (level=true) or deassert (level=false) an IRQ pin.
    /// Matches QEMU's `ioapic_set_irq(opaque, vector, level)`.
    pub fn set_irq(&mut self, pin: u8, level: bool) {
        let i = pin as usize;
        if i >= NUM_REDIR_ENTRIES {
            return;
        }
        let entry = self.redir_table[i];
        let is_level = (entry & REDIR_LEVEL_TRIGGERED) != 0;

        if is_level {
            // Level-triggered: track line state
            if level {
                self.irr |= 1 << pin;
                self.irq_level[i] = true;
                // Only service if Remote IRR is clear (QEMU behaviour)
                if (entry & REDIR_REMOTE_IRR) == 0 {
                    self.service();
                }
            } else {
                self.irq_level[i] = false;
                // Note: don't clear IRR on deassert — QEMU clears it in
                // ioapic_set_irq via `s->irr &= ~(1 << vector)`.
                // Actually QEMU DOES clear IRR on deassert for level.
                self.irr &= !(1 << pin);
            }
        } else {
            // Edge-triggered
            if level {
                // QEMU: edge-triggered on masked pin is ignored (not recorded)
                if (entry & REDIR_MASKED) != 0 {
                    return;
                }
                // Set IRR and service
                self.irr |= 1 << pin;
                self.service();
            }
            // Deassertion for edge-triggered is a no-op
        }
    }

    /// Simple assert-only API for backward compatibility.
    /// Calls `set_irq(pin, true)`.
    pub fn assert_irq(&mut self, pin: u8) {
        self.set_irq(pin, true);
    }

    // ── Service loop (QEMU: ioapic_service) ───────────────────────────

    fn service(&mut self) {
        for i in 0..NUM_REDIR_ENTRIES {
            if (self.irr & (1 << i)) == 0 {
                continue;
            }
            let entry = self.redir_table[i];
            if (entry & REDIR_MASKED) != 0 {
                continue;
            }
            let dm = ((entry >> REDIR_DELIV_MODE_SHIFT) & 7) as u8;
            if dm > 1 {
                continue; // only Fixed (0) and LowestPri (1)
            }
            let vector = (entry & 0xFF) as u8;
            // Vectors 0-15 are reserved for CPU exceptions; skip invalid entries
            // (e.g. guest clears entry to 0 before reprogramming).
            if vector < 16 {
                continue;
            }
            let is_level = (entry & REDIR_LEVEL_TRIGGERED) != 0;

            if is_level {
                if (entry & REDIR_REMOTE_IRR) != 0 {
                    continue; // coalesce — waiting for EOI
                }
                self.redir_table[i] |= REDIR_REMOTE_IRR;
                // Level: keep IRR set
            } else {
                // Edge: clear IRR on delivery
                self.irr &= !(1 << i);
            }

            if self.service_out_count < self.service_out.len() {
                self.service_out[self.service_out_count] = (vector, is_level);
                self.service_out_count += 1;
            }
        }
    }

    pub fn take_service_output(&mut self) -> &[(u8, bool)] {
        &self.service_out[..self.service_out_count]
    }

    pub fn clear_service_output(&mut self) {
        self.service_out_count = 0;
    }

    // ── Legacy route_irq API ──────────────────────────────────────────

    /// Route an external IRQ pin.  Asserts the pin and returns the first
    /// queued vector if delivery succeeded.
    pub fn route_irq(&mut self, pin: u8) -> Option<(u8, bool)> {
        self.service_out_count = 0;
        self.set_irq(pin, true);
        if self.service_out_count > 0 {
            let result = self.service_out[0];
            self.service_out_count = 0;
            Some(result)
        } else {
            None
        }
    }

    // ── EOI (QEMU: ioapic_eoi_broadcast) ──────────────────────────────

    /// Handle EOI broadcast from LAPIC.  Clears Remote IRR on matching
    /// level-triggered entries and re-services if IRR is still set.
    pub fn eoi_vector(&mut self, vector: u8) {
        for i in 0..NUM_REDIR_ENTRIES {
            let entry = &mut self.redir_table[i];
            if (*entry & REDIR_LEVEL_TRIGGERED) == 0 {
                continue;
            }
            if (*entry & 0xFF) as u8 != vector {
                continue;
            }
            if (*entry & REDIR_REMOTE_IRR) == 0 {
                continue;
            }
            *entry &= !REDIR_REMOTE_IRR;
        }
        // Re-service: if the line is still asserted, the IRR bit is still
        // set, and now that Remote IRR is cleared, it can be delivered again.
        self.service();
    }

    // ── Diagnostics ───────────────────────────────────────────────────

    pub fn has_route(&self, irq: u8) -> bool {
        let idx = irq as usize;
        if idx >= NUM_REDIR_ENTRIES { return false; }
        let entry = self.redir_table[idx];
        (entry & REDIR_MASKED) == 0 && ((entry >> REDIR_DELIV_MODE_SHIFT) & 7) <= 1
    }

    pub fn diag_entry(&self, irq: u8) -> u64 {
        self.redir_table.get(irq as usize).copied().unwrap_or(0)
    }

    pub fn programmed_vector(&self, irq: u8) -> Option<u8> {
        let idx = irq as usize;
        if idx >= NUM_REDIR_ENTRIES { return None; }
        let vector = (self.redir_table[idx] & 0xFF) as u8;
        if vector == 0 { None } else { Some(vector) }
    }

    pub fn redir_entry(&self, irq: u8) -> u64 {
        if (irq as usize) < NUM_REDIR_ENTRIES {
            self.redir_table[irq as usize]
        } else {
            0
        }
    }

    pub fn diag_irr(&self) -> u32 {
        self.irr
    }

    // ── MMIO helpers ──────────────────────────────────────────────────

    fn read_mmio_register(&self, reg_base: u64) -> u32 {
        match reg_base {
            0x00 => self.reg_select,
            0x10 => self.read_reg(self.reg_select),
            _ => 0,
        }
    }

    fn write_mmio_register(&mut self, reg_base: u64, v: u32) {
        match reg_base {
            0x00 => self.reg_select = v,
            0x10 => self.write_reg(self.reg_select, v),
            _ => {}
        }
    }
}

impl MmioHandler for IoApic {
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        #[cfg(feature = "host_test")]
        {
            static IOAPIC_ACCESS_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let cnt = IOAPIC_ACCESS_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if cnt < 200 {
                eprintln!("[ioapic] MMIO read offset={:#x} size={} regsel={:#x}", offset, size, self.reg_select);
            }
        }
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

    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()> {
        #[cfg(feature = "host_test")]
        {
            static IOAPIC_WCOUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let cnt = IOAPIC_WCOUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if cnt < 200 {
                eprintln!("[ioapic] MMIO write offset={:#x} size={} val={:#x}", offset, size, val);
            }
        }
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
