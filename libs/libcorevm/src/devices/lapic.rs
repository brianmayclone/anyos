//! Local APIC (Local Advanced Programmable Interrupt Controller) emulation.
//!
//! The LAPIC is a per-CPU interrupt controller integrated into x86
//! processors. It handles inter-processor interrupts (IPIs), local
//! timer interrupts, and routing of external interrupts from the
//! IO-APIC to the CPU core.
//!
//! # MMIO Region
//!
//! The LAPIC is memory-mapped at physical address `0xFEE00000` (4 KB).
//! All registers are 32-bit aligned at 16-byte boundaries.
//!
//! # Key Registers
//!
//! | Offset | Register | Access |
//! |--------|----------|--------|
//! | 0x020 | LAPIC ID | R/W |
//! | 0x030 | Version | RO |
//! | 0x080 | Task Priority (TPR) | R/W |
//! | 0x0B0 | End of Interrupt (EOI) | WO |
//! | 0x0D0 | Logical Destination | R/W |
//! | 0x0E0 | Destination Format | R/W |
//! | 0x0F0 | Spurious Interrupt Vector | R/W |
//! | 0x300 | ICR Low | R/W |
//! | 0x310 | ICR High | R/W |
//! | 0x320 | LVT Timer | R/W |
//! | 0x350 | LVT LINT0 | R/W |
//! | 0x360 | LVT LINT1 | R/W |
//! | 0x370 | LVT Error | R/W |
//! | 0x380 | Timer Initial Count | R/W |
//! | 0x390 | Timer Current Count | RO |
//! | 0x3E0 | Timer Divide Config | R/W |

use crate::error::Result;
use crate::memory::mmio::MmioHandler;

/// LAPIC version: xAPIC, version 0x14 (20), max LVT entry 5.
const LAPIC_VERSION: u32 = (5 << 16) | 0x14;

/// Local APIC device emulation.
///
/// Provides the minimum LAPIC functionality needed for SeaBIOS and
/// other firmware to detect and configure the APIC. The timer is not
/// actively counting — `current_count` always returns 0.
#[derive(Debug)]
pub struct Lapic {
    /// LAPIC ID register (bits 31:24 = APIC ID).
    id: u32,
    /// Task Priority Register.
    tpr: u32,
    /// Logical Destination Register.
    ldr: u32,
    /// Destination Format Register.
    dfr: u32,
    /// Spurious Interrupt Vector Register.
    /// Bit 8 = APIC software enable; bits 7:0 = spurious vector.
    svr: u32,
    /// Interrupt Command Register — low 32 bits.
    icr_lo: u32,
    /// Interrupt Command Register — high 32 bits (destination field).
    icr_hi: u32,
    /// LVT Timer entry.
    lvt_timer: u32,
    /// LVT LINT0 entry.
    lvt_lint0: u32,
    /// LVT LINT1 entry.
    lvt_lint1: u32,
    /// LVT Error entry.
    lvt_error: u32,
    /// LVT Performance Monitor entry.
    lvt_perf: u32,
    /// LVT Thermal Sensor entry.
    lvt_thermal: u32,
    /// Timer Initial Count.
    timer_init_count: u32,
    /// Timer Current Count.
    timer_cur_count: u32,
    /// Fractional instruction credits toward the next timer tick.
    timer_credit: u64,
    /// Timer Divide Configuration.
    timer_divide: u32,
    /// Error Status Register.
    esr: u32,
    /// In-Service Register (8 × 32-bit = 256 bits).
    isr: [u32; 8],
    /// Trigger Mode Register (8 × 32-bit).
    tmr: [u32; 8],
    /// Interrupt Request Register (8 × 32-bit).
    irr: [u32; 8],
}

impl Lapic {
    /// Create a new LAPIC for the BSP (bootstrap processor, APIC ID 0).
    ///
    /// All LVT entries start masked. The APIC is software-disabled
    /// (SVR bit 8 = 0) until the guest enables it.
    pub fn new() -> Self {
        Lapic {
            id: 0,                 // BSP = APIC ID 0
            tpr: 0,
            ldr: 0,
            dfr: 0xFFFF_FFFF,     // Flat model default
            svr: 0xFF,            // APIC disabled, vector 0xFF
            icr_lo: 0,
            icr_hi: 0,
            lvt_timer: 1 << 16,   // masked
            lvt_lint0: 1 << 16,   // masked
            lvt_lint1: 1 << 16,   // masked
            lvt_error: 1 << 16,   // masked
            lvt_perf: 1 << 16,    // masked
            lvt_thermal: 1 << 16, // masked
            timer_init_count: 0,
            timer_cur_count: 0,
            timer_credit: 0,
            timer_divide: 0,
            esr: 0,
            isr: [0; 8],
            tmr: [0; 8],
            irr: [0; 8],
        }
    }
}

impl Lapic {
    fn timer_divisor(&self) -> u64 {
        // APIC timer divide encoding:
        // 0b0000=2, 0001=4, 0010=8, 0011=16,
        // 1000=32, 1001=64, 1010=128, 1011=1.
        match self.timer_divide & 0xB {
            0x0 => 2,
            0x1 => 4,
            0x2 => 8,
            0x3 => 16,
            0x8 => 32,
            0x9 => 64,
            0xA => 128,
            0xB => 1,
            _ => 2,
        }
    }

    /// Advance the LAPIC timer by a number of guest instructions.
    ///
    /// Returns the timer interrupt vector when the counter expires and the
    /// timer is unmasked. For periodic mode, the counter reloads.
    pub fn advance(&mut self, guest_instructions: u64) -> Option<u8> {
        // APIC software enable (SVR bit 8) must be set.
        if (self.svr & (1 << 8)) == 0 {
            return None;
        }
        // Timer masked?
        if (self.lvt_timer & (1 << 16)) != 0 {
            return None;
        }
        if self.timer_cur_count == 0 {
            return None;
        }

        let div = self.timer_divisor();
        self.timer_credit = self.timer_credit.saturating_add(guest_instructions);
        if self.timer_credit < div {
            return None;
        }

        let dec = (self.timer_credit / div) as u32;
        self.timer_credit %= div;

        if dec < self.timer_cur_count {
            self.timer_cur_count -= dec;
            return None;
        }

        // Counter expired.
        let vec = (self.lvt_timer & 0xFF) as u8;
        let periodic = (self.lvt_timer & (1 << 17)) != 0;
        if periodic && self.timer_init_count != 0 {
            self.timer_cur_count = self.timer_init_count;
        } else {
            self.timer_cur_count = 0;
        }
        Some(vec)
    }

    /// Read the raw 32-bit value of a register by its 16-byte-aligned offset.
    fn read_register(&self, reg_base: u32) -> u32 {
        match reg_base {
            0x020 => self.id,
            0x030 => LAPIC_VERSION,
            0x080 => self.tpr,
            0x090 | 0x0A0 => 0, // APR / PPR (not implemented)
            0x0D0 => self.ldr,
            0x0E0 => self.dfr,
            0x0F0 => self.svr,
            // In-Service Register (ISR): 8 × 32-bit at 0x100-0x170.
            off @ 0x100..=0x170 => self.isr[((off - 0x100) >> 4) as usize],
            // Trigger Mode Register (TMR): 0x180-0x1F0.
            off @ 0x180..=0x1F0 => self.tmr[((off - 0x180) >> 4) as usize],
            // Interrupt Request Register (IRR): 0x200-0x270.
            off @ 0x200..=0x270 => self.irr[((off - 0x200) >> 4) as usize],
            0x280 => self.esr,
            0x300 => self.icr_lo,
            0x310 => self.icr_hi,
            0x320 => self.lvt_timer,
            0x330 => self.lvt_thermal,
            0x340 => self.lvt_perf,
            0x350 => self.lvt_lint0,
            0x360 => self.lvt_lint1,
            0x370 => self.lvt_error,
            0x380 => self.timer_init_count,
            0x390 => self.timer_cur_count,
            0x3E0 => self.timer_divide,
            _ => 0,
        }
    }

    /// Write a 32-bit value to a register by its 16-byte-aligned offset.
    fn write_register(&mut self, reg_base: u32, v: u32) {
        match reg_base {
            // LAPIC ID: bits 31:24 are writable.
            0x020 => self.id = v & 0xFF00_0000,
            0x080 => self.tpr = v & 0xFF,
            // EOI: any write signals end-of-interrupt.
            0x0B0 => {
                // Clear the highest-priority bit in ISR.
                // Simplified: just clear all ISR bits.
                for r in self.isr.iter_mut() {
                    *r = 0;
                }
            }
            0x0D0 => self.ldr = v,
            0x0E0 => self.dfr = v,
            0x0F0 => self.svr = v,
            0x280 => self.esr = 0, // Writing clears ESR
            0x300 => {
                // ICR Low — triggers IPI. For single-CPU, just store.
                self.icr_lo = v & !0x1000; // clear delivery status bit
            }
            0x310 => self.icr_hi = v,
            0x320 => self.lvt_timer = v,
            0x330 => self.lvt_thermal = v,
            0x340 => self.lvt_perf = v,
            0x350 => self.lvt_lint0 = v,
            0x360 => self.lvt_lint1 = v,
            0x370 => self.lvt_error = v,
            0x380 => {
                self.timer_init_count = v;
                self.timer_cur_count = v;
                self.timer_credit = 0;
            }
            0x3E0 => self.timer_divide = v,
            _ => {}
        }
    }
}

impl MmioHandler for Lapic {
    /// Read from LAPIC MMIO register.
    ///
    /// LAPIC registers are 32-bit wide at 16-byte-aligned offsets.
    /// Bytes 0-3 of each slot hold the register; bytes 4-15 are reserved.
    /// Sub-dword accesses extract the correct byte(s) from the register.
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        let byte_in_slot = (offset & 0xF) as u32;
        // Bytes 4-15 within each 16-byte register slot are reserved.
        if byte_in_slot >= 4 {
            return Ok(0);
        }
        let reg_base = (offset & !0xF) as u32;
        let reg_val = self.read_register(reg_base);
        // Shift to the requested byte position and mask to access size.
        let shifted = (reg_val >> (byte_in_slot * 8)) as u64;
        let bits = (size as u32).min(4) * 8;
        let mask = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
        Ok(shifted & mask)
    }

    /// Write to LAPIC MMIO register.
    ///
    /// Sub-dword writes perform a read-modify-write to merge partial bytes.
    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()> {
        let byte_in_slot = (offset & 0xF) as u32;
        if byte_in_slot >= 4 {
            return Ok(());
        }
        let reg_base = (offset & !0xF) as u32;
        // Build the full 32-bit value to write.
        let v = if byte_in_slot == 0 && size >= 4 {
            // Standard 32-bit aligned write (the expected / common case).
            val as u32
        } else {
            // Sub-dword or byte-offset write: read-modify-write.
            let old = self.read_register(reg_base);
            let shift = byte_in_slot * 8;
            let bits = (size as u32).min(4) * 8;
            let mask = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
            (old & !(mask << shift)) | (((val as u32) & mask) << shift)
        };
        self.write_register(reg_base, v);
        Ok(())
    }
}
