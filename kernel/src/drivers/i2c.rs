//! I²C abstraction layer built on top of the SMBus host controller.
//!
//! Provides a thin I²C-compatible interface that maps I²C register reads and
//! writes to SMBus Byte Data / Word Data / Block Read transactions.  This works
//! because the SMBus protocol is a strict subset of I²C (same electrical bus,
//! compatible transaction formats for byte and word register accesses).
//!
//! # Usage
//! ```rust
//! let dev = I2cDevice { addr: 0x48 };  // LM75 temperature sensor
//! if let Some(temp_raw) = i2c_read_word_data(&dev, 0x00) { ... }
//! ```

use super::smbus;

// ── Public Types ──────────────────────────────────────────────────────────────

/// Represents an I²C device at a given 7-bit bus address.
pub struct I2cDevice {
    /// 7-bit I²C device address (without R/W bit; range 0x00–0x7F).
    pub addr: u8,
}

// ── I²C Read/Write Operations ─────────────────────────────────────────────────

/// Read a single byte from register `reg` of I²C device `dev`.
///
/// Implemented as an SMBus Byte Data Read.
/// Returns `None` if the device is absent or the transaction fails.
#[inline]
pub fn i2c_read_byte_data(dev: &I2cDevice, reg: u8) -> Option<u8> {
    smbus::read_byte(dev.addr, reg)
}

/// Write a single byte `value` to register `reg` of I²C device `dev`.
///
/// Implemented as an SMBus Byte Data Write.
/// Returns `true` on success.
#[inline]
pub fn i2c_write_byte_data(dev: &I2cDevice, reg: u8, value: u8) -> bool {
    smbus::write_byte(dev.addr, reg, value)
}

/// Read a 16-bit word from register `reg` of I²C device `dev`.
///
/// Implemented as an SMBus Word Data Read.  The returned value is in
/// host (little-endian) byte order: low byte = DAT0, high byte = DAT1.
/// Returns `None` on failure.
#[inline]
pub fn i2c_read_word_data(dev: &I2cDevice, reg: u8) -> Option<u16> {
    smbus::read_word(dev.addr, reg)
}

/// Read a block of up to `buf.len()` bytes from register `reg` of I²C device `dev`.
///
/// Implemented as an SMBus Block Read (first byte returned by the device is
/// the count; subsequent bytes are written into `buf`).
/// Returns the number of bytes actually received, or `None` on failure.
#[inline]
pub fn i2c_read_block(dev: &I2cDevice, reg: u8, buf: &mut [u8]) -> Option<usize> {
    smbus::read_block(dev.addr, reg, buf)
}

// ── Multi-Byte I²C Transfers (for I2C-HID) ─────────────────────────────────

/// Write `tx` bytes then read `rx.len()` bytes in a single I²C transaction.
/// Used by I2C-HID to send a register address and read the response.
/// Returns true on success.
pub fn write_then_read(addr: u8, tx: &[u8], rx: &mut [u8]) -> bool {
    // Use the SMBus block read with the first tx byte as the command
    if tx.is_empty() { return false; }
    if tx.len() == 1 {
        // Simple register read
        if let Some(n) = smbus::read_block(addr, tx[0], rx) {
            return n > 0;
        }
        return false;
    }
    // For 2-byte register addresses (I2C-HID), use the first byte as command
    // and read block data. This is an approximation — full I2C would need
    // a repeated-start transaction which requires direct controller access.
    if let Some(n) = smbus::read_block(addr, tx[0], rx) {
        return n > 0;
    }
    false
}

/// Write raw bytes to an I²C device.
pub fn write(addr: u8, data: &[u8]) -> bool {
    if data.is_empty() { return false; }
    if data.len() == 2 {
        return smbus::write_byte(addr, data[0], data[1]);
    }
    // For longer writes, write byte-by-byte (limited by SMBus)
    for chunk in data.chunks(2) {
        if chunk.len() == 2 {
            if !smbus::write_byte(addr, chunk[0], chunk[1]) { return false; }
        }
    }
    true
}

// ── Device Detection ─────────────────────────────────────────────────────────

/// Probe whether an I²C device is present at 7-bit address `addr`.
///
/// Uses an SMBus Quick Write command (address + W + stop) — if the device
/// ACKs its address, it is considered present.
///
/// Returns `true` if the device acknowledged its address.
#[inline]
pub fn i2c_detect(addr: u8) -> bool {
    smbus::quick_write(addr)
}
