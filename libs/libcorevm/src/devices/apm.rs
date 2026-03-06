//! APM control/status ports (0xB2/0xB3).
//!
//! SeaBIOS uses these ports for chipset-specific SMI handshakes. On QEMU the
//! firmware writes a nonzero status to 0xB3, triggers an APM command via 0xB2,
//! then polls 0xB3 until the platform clears it. We do not emulate SMM itself,
//! but we must provide the handshake so SeaBIOS can continue booting.

use crate::error::Result;
use crate::io::IoHandler;

/// Minimal APM device for firmware handshakes.
pub struct ApmControl {
    control: u8,
    status: u8,
}

impl ApmControl {
    pub fn new() -> Self {
        Self {
            control: 0,
            status: 0,
        }
    }
}

impl IoHandler for ApmControl {
    fn read(&mut self, port: u16, _size: u8) -> Result<u32> {
        let value = match port {
            0xB2 => self.control,
            0xB3 => self.status,
            _ => 0xFF,
        };
        Ok(value as u32)
    }

    fn write(&mut self, port: u16, _size: u8, val: u32) -> Result<()> {
        let val8 = val as u8;
        match port {
            0xB2 => {
                self.control = val8;
                // Complete the pending firmware handshake immediately.
                self.status = 0;
            }
            0xB3 => {
                self.status = val8;
            }
            _ => {}
        }
        Ok(())
    }
}
