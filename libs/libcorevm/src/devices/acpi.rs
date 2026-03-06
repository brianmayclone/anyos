//! ACPI Power Management emulation (ICH9/Q35).
//!
//! Emulates the ACPI PM I/O registers at base address 0xB000 as used by
//! the Q35/ICH9 chipset. The critical register is the **PM Timer** at
//! offset 0x08 — a 32-bit free-running counter that increments at
//! 3.579545 MHz. SeaBIOS uses this for all timing delays (`ndelay`,
//! `usleep`, TSC calibration).
//!
//! # I/O Ports (relative to PMBASE = 0xB000)
//!
//! | Offset | Size | Register | Access |
//! |--------|------|----------|--------|
//! | 0x00 | 2 | PM1a Status | R/W1C |
//! | 0x02 | 2 | PM1a Enable | R/W |
//! | 0x04 | 2 | PM1a Control | R/W |
//! | 0x08 | 4 | PM Timer | RO |

use crate::error::Result;
use crate::io::IoHandler;

/// Approximate guest instructions per PM timer tick.
///
/// The PM timer runs at 3.579545 MHz. On the current host-test workloads we
/// see early Linux execute at roughly 35 MIPS, so a 10:1 ratio yields a
/// workable free-running timer without depending on host wall-clock APIs.
const PM_TIMER_INSTS_PER_TICK: u64 = 10;

/// ACPI Power Management I/O device.
///
/// Covers the PM1 event, PM1 control, and PM timer registers at the
/// ICH9 ACPI I/O base (0xB000). The timer is a free-running 24-bit
/// or 32-bit counter (bit 24 extension supported via FADT, but we
/// report all 32 bits).
#[derive(Debug)]
pub struct AcpiPm {
    /// PM1a Status Register (offset 0x00).
    ///
    /// Bits are write-1-to-clear. Bit 0 = TMR_STS (timer overflow),
    /// Bit 8 = PWRBTN_STS, Bit 10 = RTC_STS.
    pm1_status: u16,
    /// PM1a Enable Register (offset 0x02).
    pm1_enable: u16,
    /// PM1a Control Register (offset 0x04).
    ///
    /// Bit 13 = SLP_EN (triggers sleep), Bits 12:10 = SLP_TYP.
    pm1_control: u16,
    /// PM Timer counter (offset 0x08, 32-bit read-only).
    /// SeaBIOS only cares that it changes between reads.
    timer_count: u32,
    /// Fractional guest instruction credits toward the next PM timer tick.
    instruction_credit: u64,
}

impl AcpiPm {
    /// Create a new ACPI PM device with all registers zeroed.
    pub fn new() -> Self {
        AcpiPm {
            pm1_status: 0,
            pm1_enable: 0,
            pm1_control: 0,
            timer_count: 0,
            instruction_credit: 0,
        }
    }

    /// Advance the free-running PM timer by guest execution progress.
    pub fn advance(&mut self, guest_instructions: u64) {
        self.instruction_credit = self.instruction_credit.saturating_add(guest_instructions);
        if self.instruction_credit < PM_TIMER_INSTS_PER_TICK {
            return;
        }

        let ticks = (self.instruction_credit / PM_TIMER_INSTS_PER_TICK) as u32;
        self.instruction_credit %= PM_TIMER_INSTS_PER_TICK;

        let prev = self.timer_count;
        self.timer_count = self.timer_count.wrapping_add(ticks);

        // PM1_STS.TMR_STS reflects a 24-bit PM timer overflow on ICH-style PM.
        let prev_24 = prev & 0x00FF_FFFF;
        let cur_24 = self.timer_count & 0x00FF_FFFF;
        if cur_24 < prev_24 {
            self.pm1_status |= 1;
        }
    }
}

impl IoHandler for AcpiPm {
    /// Read from ACPI PM I/O registers.
    ///
    /// Port offsets are relative to PMBASE (0xB000):
    /// - 0x00-0x01: PM1a Status
    /// - 0x02-0x03: PM1a Enable
    /// - 0x04-0x05: PM1a Control
    /// - 0x08-0x0B: PM Timer (32-bit, free-running)
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        let offset = port & 0x3F;
        let val = match offset {
            // PM1a Status Register.
            0x00 => self.pm1_status as u32,
            // PM1a Enable Register.
            0x02 => self.pm1_enable as u32,
            // PM1a Control Register.
            0x04 => self.pm1_control as u32,
            // PM Timer — free-running 32-bit counter with byte/word subaccesses.
            0x08..=0x0B => {
                let shift = ((offset - 0x08) * 8) as u32;
                self.timer_count >> shift
            }
            _ => 0,
        };

        // Mask to requested access size.
        let masked = match size {
            1 => val & 0xFF,
            2 => val & 0xFFFF,
            _ => val,
        };
        Ok(masked)
    }

    /// Write to ACPI PM I/O registers.
    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let offset = port & 0x3F;
        match offset {
            // PM1a Status: write-1-to-clear.
            0x00 => self.pm1_status &= !(val as u16),
            // PM1a Enable: writable.
            0x02 => self.pm1_enable = val as u16,
            // PM1a Control: writable (bit 13 SLP_EN triggers sleep).
            0x04 => {
                self.pm1_control = val as u16;
                // SLP_EN (bit 13): guest requested sleep/shutdown.
                // For now, just acknowledge it — the VMD can poll for this.
            }
            // PM Timer is read-only — writes are silently ignored.
            0x08 => {}
            _ => {}
        }
        Ok(())
    }
}
