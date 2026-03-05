//! System control port 0x61 ("NMI status and control", speaker gate).
//!
//! Linux and bootloaders use this port together with PIT channel 2 for
//! calibration and short delays during very early boot.

use crate::error::Result;
use crate::io::IoHandler;

use super::pit::Pit;

/// Emulation for I/O port 0x61.
///
/// Bits implemented:
/// - bit 0: gate to PIT channel 2
/// - bit 1: speaker data enable (latched only)
/// - bit 4: refresh clock toggle (synthetic, flips on each read)
/// - bit 5: PIT channel 2 output
#[derive(Debug)]
pub struct Port61 {
    pit: *mut Pit,
    control: u8,
    refresh_toggle: bool,
}

impl Port61 {
    /// Create a new port-0x61 device tied to the PIT instance.
    pub fn new(pit: *mut Pit) -> Self {
        // PC reset defaults are effectively bits 0/1 cleared.
        if !pit.is_null() {
            unsafe { (*pit).channels[2].gate = false; }
        }
        Port61 {
            pit,
            control: 0,
            refresh_toggle: false,
        }
    }
}

impl IoHandler for Port61 {
    fn read(&mut self, _port: u16, _size: u8) -> Result<u32> {
        // Bit 5 reflects PIT channel 2 OUT.
        let pit_out = if self.pit.is_null() {
            0
        } else if unsafe { (*self.pit).channels[2].output } {
            1
        } else {
            0
        };
        self.refresh_toggle = !self.refresh_toggle;
        let refresh = if self.refresh_toggle { 1 } else { 0 };
        let v = (self.control & 0x03) | (refresh << 4) | (pit_out << 5);
        Ok(v as u32)
    }

    fn write(&mut self, _port: u16, _size: u8, val: u32) -> Result<()> {
        self.control = (val as u8) & 0x03;
        if !self.pit.is_null() {
            let gate = (self.control & 0x01) != 0;
            unsafe { (*self.pit).channels[2].gate = gate; }
        }
        Ok(())
    }
}
