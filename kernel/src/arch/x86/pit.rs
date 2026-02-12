//! 8254 Programmable Interval Timer (PIT) driver.
//!
//! Configures channel 0 in square-wave mode at [`TICK_HZ`] (1000 Hz).
//! Maintains an atomic tick counter for timekeeping. Uses the CPU's TSC
//! (Time Stamp Counter) to recover missed ticks: when interrupts are
//! disabled (scheduler lock, heap lock, etc.), PIT IRQs can be delayed
//! or missed. The TSC runs independently and lets us compute the true
//! elapsed tick count on each IRQ.

use crate::arch::x86::port::outb;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const PIT_CHANNEL0: u16 = 0x40;
const PIT_CMD: u16 = 0x43;
/// Base oscillator frequency of the 8254 PIT in Hz.
const PIT_FREQUENCY: u32 = 1193182;

/// Configured scheduler tick rate in Hz. All code that converts between
/// PIT ticks and wall-clock time must use this constant.
/// 1000 Hz = 1ms tick granularity (up from 100 Hz / 10ms).
pub const TICK_HZ: u32 = 1000;

/// Monotonically increasing tick counter, corrected via TSC when available.
pub static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

/// TSC frequency in Hz (0 = not yet calibrated).
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
/// TSC value at calibration start (anchor point for tick computation).
static TSC_BASE: AtomicU64 = AtomicU64::new(0);
/// TICK_COUNT value at calibration start.
static TICK_BASE: AtomicU32 = AtomicU32::new(0);

/// Read the CPU's Time Stamp Counter.
#[inline(always)]
fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
    }
    (hi as u64) << 32 | lo as u64
}

/// Program the PIT channel 0 to fire at [`TICK_HZ`] interrupts per second.
pub fn init() {
    let divisor = PIT_FREQUENCY / TICK_HZ;

    unsafe {
        // Channel 0, lobyte/hibyte, mode 3 (square wave), binary
        outb(PIT_CMD, 0x36);
        outb(PIT_CHANNEL0, (divisor & 0xFF) as u8);
        outb(PIT_CHANNEL0, ((divisor >> 8) & 0xFF) as u8);
    }
}

/// Calibrate the TSC against the PIT for accurate timekeeping.
///
/// Must be called after PIT and IRQ handlers are initialized, with
/// interrupts enabled. Measures TSC ticks over 50 PIT periods (~50ms
/// at 1000 Hz) to compute TSC frequency.
pub fn calibrate_tsc() {
    // Wait for a PIT tick edge (synchronize to PIT phase)
    let start_tick = TICK_COUNT.load(Ordering::Relaxed);
    while TICK_COUNT.load(Ordering::Relaxed) == start_tick {
        core::hint::spin_loop();
    }

    // Record starting point
    let tsc_start = rdtsc();
    let tick_start = TICK_COUNT.load(Ordering::Relaxed);

    // Wait for 50 PIT ticks (~50ms at 1000 Hz)
    while TICK_COUNT.load(Ordering::Relaxed).wrapping_sub(tick_start) < 50 {
        core::hint::spin_loop();
    }

    let tsc_end = rdtsc();
    let ticks_elapsed = TICK_COUNT.load(Ordering::Relaxed).wrapping_sub(tick_start);

    let tsc_delta = tsc_end - tsc_start;
    // TSC Hz = tsc_delta / (ticks_elapsed / TICK_HZ) = tsc_delta * TICK_HZ / ticks_elapsed
    let tsc_hz_value = tsc_delta * TICK_HZ as u64 / ticks_elapsed as u64;

    // Store calibration anchor
    TSC_BASE.store(tsc_start, Ordering::Release);
    TICK_BASE.store(tick_start, Ordering::Release);
    TSC_HZ.store(tsc_hz_value, Ordering::Release);

    crate::serial_println!(
        "TSC calibrated: {} MHz ({} ticks in {} PIT periods)",
        tsc_hz_value / 1_000_000,
        tsc_delta,
        ticks_elapsed
    );
}

/// Update the tick counter. Called from the PIT IRQ handler.
///
/// When the TSC is calibrated, computes the true tick count from TSC
/// rather than simply incrementing by 1. This recovers ticks lost when
/// interrupts were disabled (scheduler lock, heap lock, etc.).
pub fn tick() {
    let tsc_hz = TSC_HZ.load(Ordering::Relaxed);
    if tsc_hz > 0 {
        // TSC-based accurate tick count
        let tsc_now = rdtsc();
        let tsc_base = TSC_BASE.load(Ordering::Relaxed);
        let tick_base = TICK_BASE.load(Ordering::Relaxed);
        let elapsed_tsc = tsc_now.wrapping_sub(tsc_base);
        let expected_ticks = tick_base + (elapsed_tsc * TICK_HZ as u64 / tsc_hz) as u32;

        // Only advance, never go backwards
        let current = TICK_COUNT.load(Ordering::Relaxed);
        if expected_ticks > current {
            TICK_COUNT.store(expected_ticks, Ordering::Relaxed);
        }
    } else {
        // Pre-calibration: simple increment
        TICK_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

/// Return the current tick count since boot.
pub fn get_ticks() -> u32 {
    TICK_COUNT.load(Ordering::Relaxed)
}
