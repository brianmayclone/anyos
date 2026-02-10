//! COM1 serial port driver for debug output.
//!
//! Provides 115200 baud 8N1 serial I/O via port 0x3F8, plus a 32 KiB kernel
//! log ring buffer that captures all serial output for later retrieval.

use crate::arch::x86::port::{inb, outb};
use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

/// COM1 I/O base port address.
const COM1: u16 = 0x3F8;

/// Zero-sized type implementing `fmt::Write` for serial output.
pub struct SerialPort;

static mut SERIAL_INITIALIZED: bool = false;

// ── Kernel log ring buffer (pre-heap, interrupt-safe) ──────────────────────

/// Size of the kernel log ring buffer in bytes.
const LOG_BUF_SIZE: usize = 32 * 1024; // 32 KiB
static mut LOG_BUF: [u8; LOG_BUF_SIZE] = [0u8; LOG_BUF_SIZE];
static LOG_WRITE_POS: AtomicUsize = AtomicUsize::new(0);
static LOG_TOTAL_WRITTEN: AtomicUsize = AtomicUsize::new(0);

fn log_push_byte(byte: u8) {
    let pos = LOG_WRITE_POS.load(Ordering::Relaxed);
    unsafe { LOG_BUF[pos] = byte; }
    LOG_WRITE_POS.store((pos + 1) % LOG_BUF_SIZE, Ordering::Relaxed);
    LOG_TOTAL_WRITTEN.fetch_add(1, Ordering::Relaxed);
}

/// Copy kernel log into `dst`. Returns number of bytes written.
pub fn read_log(dst: &mut [u8]) -> usize {
    let total = LOG_TOTAL_WRITTEN.load(Ordering::Relaxed);
    if total == 0 || dst.is_empty() {
        return 0;
    }
    let available = total.min(LOG_BUF_SIZE);
    let write_pos = LOG_WRITE_POS.load(Ordering::Relaxed);
    let start = if total <= LOG_BUF_SIZE { 0 } else { write_pos };
    let copy_len = available.min(dst.len());

    for i in 0..copy_len {
        let idx = (start + i) % LOG_BUF_SIZE;
        dst[i] = unsafe { LOG_BUF[idx] };
    }
    copy_len
}

/// Initialize COM1 at 115200 baud, 8N1, with FIFO enabled.
pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00); // Disable all interrupts
        outb(COM1 + 3, 0x80); // Enable DLAB (set baud rate divisor)
        outb(COM1 + 0, 0x01); // Set divisor to 1 (115200 baud)
        outb(COM1 + 1, 0x00); //   hi byte
        outb(COM1 + 3, 0x03); // 8 bits, no parity, one stop bit (8N1)
        outb(COM1 + 2, 0xC7); // Enable FIFO, clear them, 14-byte threshold
        outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR set

        SERIAL_INITIALIZED = true;
    }
}

fn is_transmit_empty() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

/// Write a single byte to the serial port, also capturing it in the log ring buffer.
pub fn write_byte(byte: u8) {
    unsafe {
        if !SERIAL_INITIALIZED {
            return;
        }
    }
    // Capture to ring buffer before sending
    log_push_byte(byte);
    while !is_transmit_empty() {
        core::hint::spin_loop();
    }
    unsafe { outb(COM1, byte); }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::drivers::serial::SerialPort, $($arg)*);
    }};
}

#[macro_export]
macro_rules! serial_println {
    () => { $crate::serial_print!("\n") };
    ($($arg:tt)*) => {{
        let _ticks = $crate::arch::x86::pit::TICK_COUNT.load(core::sync::atomic::Ordering::Relaxed);
        let _ms = _ticks as u64 * 10; // PIT at 100 Hz → 10 ms per tick
        $crate::serial_print!("[{}] {}\n", _ms, format_args!($($arg)*));
    }};
}

#[cfg(feature = "debug_verbose")]
#[macro_export]
macro_rules! debug_println {
    () => { $crate::serial_print!("[DBG] \n") };
    ($($arg:tt)*) => { $crate::serial_print!("[DBG] {}\n", format_args!($($arg)*)) };
}

#[cfg(not(feature = "debug_verbose"))]
#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => {};
}
