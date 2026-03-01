//! ARM Generic Timer driver.
//!
//! Uses CNTPCT_EL0 for monotonic tick counting and CNTP_TVAL_EL0 / CNTP_CTL_EL0
//! for periodic timer interrupts (scheduling tick).

use core::sync::atomic::{AtomicU32, Ordering};

/// Timer frequency in Hz (read from CNTFRQ_EL0 at init time).
static TIMER_FREQ: AtomicU32 = AtomicU32::new(0);

/// Tick counter incremented by each timer IRQ.
static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

/// Target timer frequency for scheduling ticks (Hz).
pub const TICK_HZ: u32 = 1000;

/// Initialize the generic timer.
///
/// Reads the counter frequency from CNTFRQ_EL0 and programs the timer
/// for periodic interrupts at TICK_HZ.
pub fn init() {
    let freq: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack));
    }
    TIMER_FREQ.store(freq as u32, Ordering::Relaxed);

    // Program timer for first tick
    let tval = freq / TICK_HZ as u64;
    unsafe {
        // Set timer value (countdown)
        core::arch::asm!("msr cntp_tval_el0, {}", in(reg) tval, options(nostack));
        // Enable timer, unmask interrupt (ENABLE=1, IMASK=0)
        core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) 1u64, options(nostack));
        core::arch::asm!("isb", options(nostack));
    }

    crate::serial_println!("[OK] ARM Generic Timer: freq={}Hz, tick={}Hz", freq, TICK_HZ);
}

/// Timer IRQ handler — reprogram timer and increment tick count.
pub fn irq_handler() {
    // Increment tick counter
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);

    // Reprogram timer for next tick
    let freq = TIMER_FREQ.load(Ordering::Relaxed) as u64;
    let tval = freq / TICK_HZ as u64;
    unsafe {
        core::arch::asm!("msr cntp_tval_el0, {}", in(reg) tval, options(nostack));
    }
}

/// Timer IRQ handler that also triggers a schedule tick.
pub fn irq_handler_with_schedule() {
    irq_handler();
    crate::task::scheduler::schedule_tick();
}

/// Get the current tick count.
#[inline]
pub fn get_ticks() -> u32 {
    TICK_COUNT.load(Ordering::Relaxed)
}

/// Read the raw counter value (CNTPCT_EL0).
#[inline]
pub fn read_counter() -> u64 {
    let cnt: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntpct_el0", out(reg) cnt, options(nomem, nostack));
    }
    cnt
}

/// Get the timer frequency in Hz.
#[inline]
pub fn frequency() -> u32 {
    TIMER_FREQ.load(Ordering::Relaxed)
}

/// Busy-wait for approximately `us` microseconds using the counter.
pub fn delay_us(us: u64) {
    let freq = TIMER_FREQ.load(Ordering::Relaxed) as u64;
    if freq == 0 { return; }
    let target = read_counter() + (freq * us / 1_000_000);
    while read_counter() < target {
        core::hint::spin_loop();
    }
}

/// Busy-wait for approximately `ms` milliseconds.
pub fn delay_ms(ms: u64) {
    delay_us(ms * 1000);
}
