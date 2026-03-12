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
/// Optional callback for synchronizing PIT channel 2 gate to the
/// in-kernel PIT on Linux/KVM. Set via `set_gate_sync`.
pub type GateSyncFn = fn(bool);

/// Optional callback for reading PIT channel 2 output from the in-kernel PIT
/// on Linux/KVM. Returns true if channel 2 output pin is high.
pub type PitOutputFn = fn() -> bool;

pub struct Port61 {
    pit: *mut Pit,
    control: u8,
    refresh_toggle: bool,
    gate_sync: Option<GateSyncFn>,
    pit_output: Option<PitOutputFn>,
}

impl core::fmt::Debug for Port61 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Port61")
            .field("control", &self.control)
            .field("refresh_toggle", &self.refresh_toggle)
            .finish()
    }
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
            gate_sync: None,
            pit_output: None,
        }
    }

    /// Set a callback that synchronizes the PIT channel 2 gate to the
    /// hardware backend (e.g., in-kernel KVM PIT).
    pub fn set_gate_sync(&mut self, f: GateSyncFn) {
        self.gate_sync = Some(f);
    }

    /// Set a callback that reads PIT channel 2 output from the hardware
    /// backend (e.g., in-kernel KVM PIT via KVM_GET_PIT2).
    pub fn set_pit_output(&mut self, f: PitOutputFn) {
        self.pit_output = Some(f);
    }
}

impl IoHandler for Port61 {
    fn read(&mut self, _port: u16, _size: u8) -> Result<u32> {
        // Bit 5 reflects PIT channel 2 OUT.
        // On Linux/KVM, read from in-kernel PIT via callback (the userspace PIT
        // isn't programmed since port 0x43/0x42 go to the in-kernel PIT).
        let pit_out = if let Some(f) = self.pit_output {
            if f() { 1 } else { 0 }
        } else if self.pit.is_null() {
            0
        } else if unsafe { (*self.pit).channels[2].output } {
            1
        } else {
            0
        };
        self.refresh_toggle = !self.refresh_toggle;
        let refresh = if self.refresh_toggle { 1 } else { 0 };
        let v = (self.control & 0x03) | (refresh << 4) | (pit_out << 5);
        #[cfg(feature = "host_test")]
        {
            static P61_LOG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let cnt = P61_LOG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if cnt < 5 || (pit_out != 0 && cnt < 20) {
                eprintln!("[port61] read val={:#04x} pit_out={} gate={} cur={} enabled={}",
                    v, pit_out,
                    if self.pit.is_null() { 0 } else { unsafe { (*self.pit).channels[2].gate as u8 } },
                    if self.pit.is_null() { 0 } else { unsafe { (*self.pit).channels[2].count } },
                    if self.pit.is_null() { 0 } else { unsafe { (*self.pit).channels[2].enabled as u8 } },
                );
            }
        }
        Ok(v as u32)
    }

    fn write(&mut self, _port: u16, _size: u8, val: u32) -> Result<()> {
        self.control = (val as u8) & 0x03;
        let gate = (self.control & 0x01) != 0;
        if !self.pit.is_null() {
            unsafe { (*self.pit).channels[2].gate = gate; }
        }
        // Sync gate to in-kernel PIT (Linux/KVM)
        if let Some(sync) = self.gate_sync {
            sync(gate);
        }
        Ok(())
    }
}
