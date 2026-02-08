//! 8254 Programmable Interval Timer (PIT) driver.
//!
//! Configures channel 0 in square-wave mode at a requested frequency
//! (typically 100 Hz). Maintains an atomic tick counter for timekeeping
//! and LAPIC timer calibration.

use crate::arch::x86::port::outb;
use core::sync::atomic::{AtomicU32, Ordering};

const PIT_CHANNEL0: u16 = 0x40;
const PIT_CMD: u16 = 0x43;
/// Base oscillator frequency of the 8254 PIT in Hz.
const PIT_FREQUENCY: u32 = 1193182;

/// Monotonically increasing tick counter, incremented by the IRQ 0 handler.
pub static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

/// Program the PIT channel 0 to fire at `frequency_hz` interrupts per second.
pub fn init(frequency_hz: u32) {
    let divisor = PIT_FREQUENCY / frequency_hz;

    unsafe {
        // Channel 0, lobyte/hibyte, mode 3 (square wave), binary
        outb(PIT_CMD, 0x36);
        outb(PIT_CHANNEL0, (divisor & 0xFF) as u8);
        outb(PIT_CHANNEL0, ((divisor >> 8) & 0xFF) as u8);
    }
}

/// Increment the tick counter. Called from the PIT IRQ handler.
pub fn tick() {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Return the current tick count since boot.
pub fn get_ticks() -> u32 {
    TICK_COUNT.load(Ordering::Relaxed)
}
