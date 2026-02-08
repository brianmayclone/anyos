use crate::arch::x86::port::outb;
use core::sync::atomic::{AtomicU32, Ordering};

const PIT_CHANNEL0: u16 = 0x40;
const PIT_CMD: u16 = 0x43;
const PIT_FREQUENCY: u32 = 1193182;

pub static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn init(frequency_hz: u32) {
    let divisor = PIT_FREQUENCY / frequency_hz;

    unsafe {
        // Channel 0, lobyte/hibyte, mode 3 (square wave), binary
        outb(PIT_CMD, 0x36);
        outb(PIT_CHANNEL0, (divisor & 0xFF) as u8);
        outb(PIT_CHANNEL0, ((divisor >> 8) & 0xFF) as u8);
    }
}

pub fn tick() {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn get_ticks() -> u32 {
    TICK_COUNT.load(Ordering::Relaxed)
}
