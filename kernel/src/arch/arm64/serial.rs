//! PL011 UART driver for QEMU virt machine (MMIO at 0x0900_0000).
//!
//! Provides early serial output before the memory subsystem is initialized.
//! The UART is identity-mapped by boot.S so we can use physical addresses directly.

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

/// PL011 UART base address on QEMU virt machine.
const PL011_BASE: usize = 0x0900_0000;

/// PL011 register offsets.
const UARTDR: usize = 0x000;   // Data Register
const UARTFR: usize = 0x018;   // Flag Register
const UARTIBRD: usize = 0x024; // Integer Baud Rate Divisor
const UARTFBRD: usize = 0x028; // Fractional Baud Rate Divisor
const UARTLCR_H: usize = 0x02C; // Line Control Register
const UARTCR: usize = 0x030;   // Control Register
const UARTIMSC: usize = 0x038; // Interrupt Mask Set/Clear

/// Flag register bits.
const FR_TXFF: u32 = 1 << 5;   // Transmit FIFO full

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Write a 32-bit value to a PL011 register.
#[inline]
unsafe fn write_reg(offset: usize, val: u32) {
    let addr = PL011_BASE + offset;
    core::ptr::write_volatile(addr as *mut u32, val);
}

/// Read a 32-bit value from a PL011 register.
#[inline]
unsafe fn read_reg(offset: usize) -> u32 {
    let addr = PL011_BASE + offset;
    core::ptr::read_volatile(addr as *const u32)
}

/// Initialize the PL011 UART.
///
/// Configures 115200 baud, 8N1, FIFO enabled.
/// Assumes a 24 MHz reference clock (QEMU virt default).
pub fn init() {
    unsafe {
        // Disable UART while configuring
        write_reg(UARTCR, 0);

        // Set baud rate to 115200 (assuming 24 MHz clock)
        // Divisor = 24_000_000 / (16 * 115200) = 13.0208...
        // Integer part = 13, Fractional part = round(0.0208 * 64) = 1
        write_reg(UARTIBRD, 13);
        write_reg(UARTFBRD, 1);

        // 8 bits, FIFO enabled, no parity, 1 stop bit
        write_reg(UARTLCR_H, (3 << 5) | (1 << 4)); // WLEN=8bit, FEN=1

        // Mask all interrupts
        write_reg(UARTIMSC, 0);

        // Enable UART, TX, RX
        write_reg(UARTCR, (1 << 0) | (1 << 8) | (1 << 9)); // UARTEN, TXE, RXE
    }
    INITIALIZED.store(true, Ordering::Relaxed);
}

/// Write a single byte to the UART, blocking until the TX FIFO has space.
pub fn write_byte(byte: u8) {
    unsafe {
        // Wait until TX FIFO is not full
        while read_reg(UARTFR) & FR_TXFF != 0 {
            core::hint::spin_loop();
        }
        write_reg(UARTDR, byte as u32);
    }
}

/// Write a string to the UART.
pub fn write_str(s: &str) {
    for byte in s.bytes() {
        if byte == b'\n' {
            write_byte(b'\r');
        }
        write_byte(byte);
    }
}

/// Read a byte from the UART (non-blocking, returns None if no data).
pub fn read_byte() -> Option<u8> {
    unsafe {
        let fr = read_reg(UARTFR);
        if fr & (1 << 4) != 0 { // RXFE: receive FIFO empty
            None
        } else {
            Some(read_reg(UARTDR) as u8)
        }
    }
}

/// Check if UART is initialized.
pub fn is_initialized() -> bool {
    INITIALIZED.load(Ordering::Relaxed)
}

/// fmt::Write implementation for PL011.
pub struct Pl011Writer;

impl fmt::Write for Pl011Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        serial::write_str(s);
        Ok(())
    }
}

use super::serial;
