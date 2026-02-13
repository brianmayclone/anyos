//! 8254 Programmable Interval Timer (PIT) driver.
//!
//! Configures channel 0 in square-wave mode at [`TICK_HZ`] (1000 Hz).
//! PIT IRQ 0 is used during boot for LAPIC and TSC calibration.
//!
//! After TSC calibration, `get_ticks()` computes elapsed ticks directly
//! from the TSC — no dependency on interrupt delivery. This avoids both
//! PIT IRQ edge loss through the IOAPIC (~2.7% drift) and unstable LAPIC
//! timer calibration in QEMU (up to 34% variation between runs).

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

/// PIT IRQ-based tick counter. Used during boot (before TSC calibration).
/// After calibration, `get_ticks()` uses TSC instead.
pub static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

/// TSC frequency in Hz (0 = not yet calibrated).
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
/// TSC value that corresponds to tick 0 (set during calibration so that
/// TSC-based ticks seamlessly continue from PIT-based TICK_COUNT).
static TSC_BOOT: AtomicU64 = AtomicU64::new(0);

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

/// Calibrate TSC frequency against the PIT. Called once after PIT and IRQs
/// are running. After this, `get_ticks()` returns TSC-derived ticks.
pub fn calibrate_tsc() {
    // Wait for PIT tick edge (synchronize to PIT phase)
    let start_tick = TICK_COUNT.load(Ordering::Relaxed);
    while TICK_COUNT.load(Ordering::Relaxed) == start_tick {
        core::hint::spin_loop();
    }

    let tsc_start = rdtsc();
    let tick_start = TICK_COUNT.load(Ordering::Relaxed);

    // Wait 100 PIT ticks (~100ms at 1000 Hz)
    while TICK_COUNT.load(Ordering::Relaxed).wrapping_sub(tick_start) < 100 {
        core::hint::spin_loop();
    }

    let tsc_end = rdtsc();
    let ticks_elapsed = TICK_COUNT.load(Ordering::Relaxed).wrapping_sub(tick_start);
    let current_ticks = TICK_COUNT.load(Ordering::Relaxed);

    let tsc_delta = tsc_end - tsc_start;
    let tsc_hz_value = tsc_delta * TICK_HZ as u64 / ticks_elapsed as u64;

    // Compute the virtual TSC boot point so that TSC-based get_ticks()
    // returns `current_ticks` right now (seamless transition from PIT).
    // boot = tsc_end - current_ticks * tsc_hz / TICK_HZ
    let tsc_boot_value = tsc_end - (current_ticks as u64 * tsc_hz_value / TICK_HZ as u64);

    TSC_BOOT.store(tsc_boot_value, Ordering::Relaxed);
    // Release store: all prior writes (TSC_BOOT) must be visible before
    // other CPUs read TSC_HZ != 0 and use TSC-based get_ticks().
    TSC_HZ.store(tsc_hz_value, Ordering::Release);

    crate::serial_println!(
        "TSC calibrated: {} MHz ({} ticks in {} PIT periods)",
        tsc_hz_value / 1_000_000,
        tsc_delta,
        ticks_elapsed
    );
}

/// Update the PIT IRQ tick counter. Called from the PIT IRQ handler.
/// Only meaningful during boot (before TSC calibration).
pub fn tick() {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Return the current tick count since boot.
///
/// Before TSC calibration: returns PIT IRQ counter (may drift ~2.7%).
/// After TSC calibration: computes from TSC (drift-free, no IRQ dependency).
#[inline]
pub fn get_ticks() -> u32 {
    let tsc_hz = TSC_HZ.load(Ordering::Acquire);
    if tsc_hz > 0 {
        let boot = TSC_BOOT.load(Ordering::Relaxed);
        let now = rdtsc();
        let delta = now.wrapping_sub(boot);
        // delta * 1000 / tsc_hz — safe from overflow for ~213 days at 1 GHz
        (delta / (tsc_hz / TICK_HZ as u64)) as u32
    } else {
        TICK_COUNT.load(Ordering::Relaxed)
    }
}

/// Return the calibrated TSC frequency in Hz. Used by debug output
/// to compute real elapsed time from TSC deltas.
#[inline]
pub fn tsc_hz() -> u64 {
    TSC_HZ.load(Ordering::Relaxed)
}

/// Return real elapsed milliseconds since boot, computed from TSC.
/// Independent calculation from `get_ticks()` — useful for cross-checking.
#[inline]
pub fn real_ms_since_boot() -> u64 {
    let tsc_hz = TSC_HZ.load(Ordering::Acquire);
    if tsc_hz > 0 {
        let boot = TSC_BOOT.load(Ordering::Relaxed);
        let now = rdtsc();
        let delta = now.wrapping_sub(boot);
        delta * 1000 / tsc_hz
    } else {
        TICK_COUNT.load(Ordering::Relaxed) as u64
    }
}
