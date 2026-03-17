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
