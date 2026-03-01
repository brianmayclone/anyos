//! QEMU debug console port emulation (port 0x402).
//!
//! SeaBIOS writes debug output to this port one byte at a time via `outb`.
//! The port also serves as a detection mechanism: reading returns 0xE9
//! (Bochs debug port signature) to indicate the debug console is active.
//!
//! # I/O Port
//!
//! | Port | Width | Direction | Description |
//! |------|-------|-----------|-------------|
//! | 0x402 | 8-bit | Write | Debug character output |
//! | 0x402 | 8-bit | Read | Returns 0xE9 (port present) |

use alloc::vec::Vec;
use crate::error::Result;
use crate::io::IoHandler;

/// QEMU debug console port emulation.
///
/// Captures bytes written by the guest BIOS/OS for diagnostic output.
/// The accumulated output can be drained via [`take_output`](DebugPort::take_output).
#[derive(Debug)]
pub struct DebugPort {
    /// Buffered output bytes waiting to be drained by the host.
    output: Vec<u8>,
}

impl DebugPort {
    /// Create a new debug port with an empty output buffer.
    pub fn new() -> Self {
        DebugPort {
            output: Vec::new(),
        }
    }

    /// Drain all buffered output, returning ownership of the buffer.
    ///
    /// After this call, the internal buffer is empty and ready for new data.
    pub fn take_output(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.output)
    }
}

impl IoHandler for DebugPort {
    /// Read from the debug port.
    ///
    /// Returns 0xE9 (Bochs debug port signature) to indicate the port is
    /// active. SeaBIOS checks this before enabling debug output.
    fn read(&mut self, _port: u16, _size: u8) -> Result<u32> {
        Ok(0xE9)
    }

    /// Write a byte to the debug port.
    ///
    /// Appends the low byte of `val` to the output buffer for later
    /// retrieval by the host.
    fn write(&mut self, _port: u16, _size: u8, val: u32) -> Result<()> {
        self.output.push(val as u8);
        Ok(())
    }
}
