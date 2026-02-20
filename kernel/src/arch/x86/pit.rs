//! 8254 Programmable Interval Timer (PIT) driver.
//!
//! Configures channel 0 in square-wave mode at [`TICK_HZ`] (1000 Hz).
//!
//! TSC calibration uses PIT **channel 2** with polled readback via port 0x61
//! (no IRQ dependency). This is critical because PIT IRQ 0 delivery through
//! the IOAPIC is unreliable in both BIOS mode (~7% drift) and UEFI/OVMF
//! (~90% tick loss), which would poison TSC and LAPIC timer calibration.

use crate::arch::x86::port::{inb, outb};
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
pub fn rdtsc() -> u64 {
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

/// Calibrate TSC frequency using PIT channel 2 polled readback.
///
/// Uses PIT channel 2 in mode 0 (one-shot), gated via port 0x61. The OUT2
/// pin (port 0x61 bit 5) goes high when the count reaches zero. This is
/// completely independent of IRQ delivery — works identically in BIOS and
/// UEFI mode.
///
/// Can be called early (before sti, before IOAPIC init).
pub fn calibrate_tsc() {
    unsafe {
        let port61_saved = inb(0x61);

        // Gate OFF (bit 0 = 0) to stop channel 2, speaker OFF (bit 1 = 0)
        outb(0x61, port61_saved & 0xFC);

        // Program channel 2: mode 0 (interrupt on terminal count),
        // access lobyte/hibyte, binary counting
        outb(PIT_CMD, 0xB0);

        // Write max count: 65535 ticks ≈ 54.9ms at 1.193182 MHz
        // Longer window = better precision
        outb(0x42, 0xFF); // low byte
        outb(0x42, 0xFF); // high byte

        // Gate ON (rising edge loads count and starts counting)
        // Keep speaker off (bit 1 = 0)
        outb(0x61, (inb(0x61) | 0x01) & !0x02);

        let tsc_start = rdtsc();

        // Wait for OUT2 to go high (bit 5 of port 0x61).
        // In mode 0: OUT starts LOW, goes HIGH when count reaches 0.
        while inb(0x61) & 0x20 == 0 {
            core::hint::spin_loop();
        }

        let tsc_end = rdtsc();

        // Restore port 0x61
        outb(0x61, port61_saved);

        let tsc_delta = tsc_end - tsc_start;
        // PIT ran exactly 65535 cycles at 1,193,182 Hz.
        // TSC_HZ = tsc_delta * PIT_FREQUENCY / 65535
        let tsc_hz_value = tsc_delta * PIT_FREQUENCY as u64 / 0xFFFF;

        // Set TSC_BOOT to the TSC value at calibration end.
        // TICK_COUNT is 0 or nearly 0 at this early stage.
        let current_ticks = TICK_COUNT.load(Ordering::Relaxed);
        let tsc_boot_value = tsc_end - (current_ticks as u64 * tsc_hz_value / TICK_HZ as u64);

        TSC_BOOT.store(tsc_boot_value, Ordering::Relaxed);
        // Release store: all prior writes (TSC_BOOT) must be visible before
        // other CPUs read TSC_HZ != 0 and use TSC-based get_ticks().
        TSC_HZ.store(tsc_hz_value, Ordering::Release);

        let cal_ms = 0xFFFFu64 * 1000 / PIT_FREQUENCY as u64;
        crate::serial_println!(
            "TSC calibrated: {} MHz (PIT ch2 polled, {} TSC cycles in ~{}ms)",
            tsc_hz_value / 1_000_000,
            tsc_delta,
            cal_ms
        );
    }
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

/// Busy-wait for the specified number of milliseconds.
/// Uses tick-based polling so the CPU stays responsive to NMIs.
pub fn delay_ms(ms: u32) {
    let ticks = (ms * TICK_HZ) / 1000;
    let ticks = if ticks == 0 { 1 } else { ticks };
    let start = get_ticks();
    while get_ticks().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

/// PIT IRQ 0 handler (APIC mode): timekeeping + boot spinner animation.
/// Does NOT call the scheduler — LAPIC timer handles scheduling separately.
pub fn irq_handler(_irq: u8) {
    tick();
    crate::drivers::boot_console::tick_spinner();
}

/// PIT IRQ 0 handler (legacy PIC mode): timekeeping, spinner, AND scheduling.
/// In legacy PIC mode, the PIT is the only periodic interrupt source.
pub fn irq_handler_with_schedule(_irq: u8) {
    tick();
    crate::drivers::boot_console::tick_spinner();
    crate::task::scheduler::schedule_tick();
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
