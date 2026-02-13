//! 8254 Programmable Interval Timer (PIT) driver.
//!
//! Configures channel 0 in square-wave mode at [`TICK_HZ`] (1000 Hz).
//! Maintains an atomic tick counter for timekeeping. Uses per-tick TSC
//! delta measurement to recover ticks lost when interrupts are disabled.
//!
//! Key insight: instead of computing absolute ticks from a boot-time TSC
//! baseline (which drifts because QEMU's TSC frequency changes after boot
//! calibration), we re-anchor the TSC on every PIT IRQ and only measure
//! the delta since the LAST IRQ. This prevents drift accumulation while
//! still recovering missed ticks from interrupt-disabled windows.

use crate::arch::x86::port::outb;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

const PIT_CHANNEL0: u16 = 0x40;
const PIT_CMD: u16 = 0x43;
/// Base oscillator frequency of the 8254 PIT in Hz.
const PIT_FREQUENCY: u32 = 1193182;

/// Configured scheduler tick rate in Hz. All code that converts between
/// PIT ticks and wall-clock time must use this constant.
/// 1000 Hz = 1ms tick granularity (up from 100 Hz / 10ms).
pub const TICK_HZ: u32 = 1000;

/// Monotonically increasing tick counter.
pub static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

/// When true, LAPIC timer on CPU 0 drives TICK_COUNT instead of PIT IRQ 0.
/// Set after TSC calibration is complete. The LAPIC timer is more reliable
/// because it doesn't go through the IOAPIC (edge-triggered, loses edges
/// when interrupts are disabled on CPU 0).
pub static LAPIC_TIMEKEEPING: AtomicBool = AtomicBool::new(false);

/// TSC frequency in Hz (0 = not yet calibrated).
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
/// TSC value at the last PIT IRQ (re-anchored every tick).
static LAST_TSC: AtomicU64 = AtomicU64::new(0);

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
/// are running. Uses a short 100-tick (100ms) window — accuracy only needs
/// to be within ~20% since we use per-tick re-anchoring (not absolute).
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

    let tsc_delta = tsc_end - tsc_start;
    let tsc_hz_value = tsc_delta * TICK_HZ as u64 / ticks_elapsed as u64;

    LAST_TSC.store(tsc_end, Ordering::Relaxed);
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
/// Always increments by 1 for the current IRQ. Then uses TSC delta since
/// the last IRQ to detect missed ticks (from interrupt-disabled windows)
/// and adds bounded catch-up. Re-anchors TSC on every call so measurement
/// errors don't accumulate over time.
pub fn tick() {
    // Always count this IRQ
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);

    // TSC-based catch-up for missed ticks
    let tsc_hz = TSC_HZ.load(Ordering::Relaxed);
    if tsc_hz > 0 {
        let tsc_now = rdtsc();
        let last = LAST_TSC.swap(tsc_now, Ordering::Relaxed);
        if last > 0 {
            let delta = tsc_now.wrapping_sub(last);
            // How many ticks worth of time passed since last IRQ?
            let expected = (delta * TICK_HZ as u64 / tsc_hz) as u32;
            // We already added 1. If expected > 1, we missed (expected-1) ticks.
            if expected > 1 {
                // Bounded catch-up: at most 50 ticks (50ms) per IRQ.
                // This prevents runaway if TSC is wildly off.
                let missed = (expected - 1).min(50);
                TICK_COUNT.fetch_add(missed, Ordering::Relaxed);
            }
        }
    }
}

/// Return the current tick count since boot.
pub fn get_ticks() -> u32 {
    TICK_COUNT.load(Ordering::Relaxed)
}
