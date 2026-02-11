//! COM1 serial port driver for debug output.
//!
//! Provides 115200 baud 8N1 serial I/O via port 0x3F8, plus a 32 KiB kernel
//! log ring buffer that captures all serial output for later retrieval.
//!
//! TX is interrupt-driven (IRQ 4 / THRE): `write_byte()` pushes to a ring
//! buffer and returns immediately.  The UART drains the buffer via the
//! Transmitter Holding Register Empty interrupt, sending up to 16 bytes
//! per ISR invocation (FIFO depth).  During early boot (before
//! `enable_async()` is called) output falls back to blocking polling so
//! that boot messages still appear in order.

use crate::arch::x86::port::{inb, outb};
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// COM1 I/O base port address.
const COM1: u16 = 0x3F8;

/// Zero-sized type implementing `fmt::Write` for serial output.
pub struct SerialPort;

static mut SERIAL_INITIALIZED: bool = false;
/// When true, `write_byte` uses the async TX buffer + IRQ 4 path.
static ASYNC_TX: AtomicBool = AtomicBool::new(false);

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

// ── Async TX ring buffer ──────────────────────────────────────────────────

const TX_BUF_SIZE: usize = 8 * 1024; // 8 KiB
static mut TX_BUF: [u8; TX_BUF_SIZE] = [0u8; TX_BUF_SIZE];
/// Producer (write_byte) writes here.
static TX_HEAD: AtomicUsize = AtomicUsize::new(0);
/// Consumer (ISR) reads from here.
static TX_TAIL: AtomicUsize = AtomicUsize::new(0);

/// Push one byte into the TX ring buffer.  Returns false if full (byte dropped).
#[inline]
fn tx_push(byte: u8) -> bool {
    let head = TX_HEAD.load(Ordering::Relaxed);
    let next = (head + 1) % TX_BUF_SIZE;
    if next == TX_TAIL.load(Ordering::Acquire) {
        return false; // full — drop
    }
    unsafe { TX_BUF[head] = byte; }
    TX_HEAD.store(next, Ordering::Release);
    true
}

/// Pop one byte from the TX ring buffer.  Returns None if empty.
#[inline]
fn tx_pop() -> Option<u8> {
    let tail = TX_TAIL.load(Ordering::Relaxed);
    if tail == TX_HEAD.load(Ordering::Acquire) {
        return None;
    }
    let byte = unsafe { TX_BUF[tail] };
    TX_TAIL.store((tail + 1) % TX_BUF_SIZE, Ordering::Release);
    Some(byte)
}

/// Drain the TX buffer into the UART FIFO (up to `n` bytes).
/// Called from both `kick_tx` (with interrupts disabled) and from the ISR.
fn drain_tx(max: usize) {
    for _ in 0..max {
        if !is_transmit_empty() {
            break;
        }
        match tx_pop() {
            Some(b) => unsafe { outb(COM1, b); },
            None => {
                // Buffer empty — disable THRE interrupt until new data arrives
                unsafe {
                    let ier = inb(COM1 + 1);
                    outb(COM1 + 1, ier & !0x02);
                }
                return;
            }
        }
    }
    // More data remains — ensure THRE interrupt is enabled
    unsafe {
        let ier = inb(COM1 + 1);
        if ier & 0x02 == 0 {
            outb(COM1 + 1, ier | 0x02);
        }
    }
}

/// Kick-start the TX interrupt chain if the UART is idle.
/// Briefly disables interrupts to avoid racing with the ISR.
#[inline]
fn kick_tx() {
    unsafe {
        let flags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags));
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));

        // Send up to 16 bytes into the FIFO right now (non-blocking).
        drain_tx(16);

        core::arch::asm!("push {}; popfq", in(reg) flags, options(nomem, nostack));
    }
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
        outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR set (OUT2 needed for ISA IRQ)

        SERIAL_INITIALIZED = true;
    }
}

/// Switch from blocking to async TX mode.
/// Call once after the IRQ subsystem is ready (IRQ handler registered, IRQ 4 unmasked).
pub fn enable_async() {
    // Register COM1 IRQ handler and unmask IRQ 4
    crate::arch::x86::irq::register_irq(4, serial_irq_handler);

    if crate::arch::x86::apic::is_initialized() {
        crate::arch::x86::ioapic::unmask_irq(4);
    } else {
        crate::arch::x86::pic::unmask(4);
    }

    ASYNC_TX.store(true, Ordering::Release);
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
    // Always capture to log ring buffer
    log_push_byte(byte);

    if ASYNC_TX.load(Ordering::Acquire) {
        // Async path: push to TX buffer, kick UART if idle
        if tx_push(byte) {
            kick_tx();
        }
    } else {
        // Early boot: blocking poll
        while !is_transmit_empty() {
            core::hint::spin_loop();
        }
        unsafe { outb(COM1, byte); }
    }
}

/// COM1 (IRQ 4) interrupt handler — drains the TX buffer into the UART FIFO.
fn serial_irq_handler(_irq: u8) {
    // Read IIR to acknowledge the interrupt source
    let _iir = unsafe { inb(COM1 + 2) };
    // Drain up to 16 bytes (UART FIFO depth) per interrupt
    drain_tx(16);
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
