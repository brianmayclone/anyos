//! CMOS Real-Time Clock (RTC) driver.
//!
//! Reads the current date and time from the MC146818 RTC via I/O ports 0x70/0x71.
//! Handles BCD-to-binary conversion and 12-hour to 24-hour format.

use crate::arch::x86::port::{inb, outb};

/// Time from RTC
#[derive(Debug, Clone, Copy)]
pub struct RtcTime {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub day: u8,
    pub month: u8,
    pub year: u16,
}

fn read_cmos(reg: u8) -> u8 {
    unsafe {
        outb(0x70, reg);
        inb(0x71)
    }
}

fn bcd_to_binary(bcd: u8) -> u8 {
    (bcd & 0x0F) + ((bcd >> 4) * 10)
}

fn is_updating() -> bool {
    unsafe {
        outb(0x70, 0x0A);
        inb(0x71) & 0x80 != 0
    }
}

/// Read the current time from the RTC
pub fn read_time() -> RtcTime {
    // Wait for any update to finish
    while is_updating() {}

    let mut seconds = read_cmos(0x00);
    let mut minutes = read_cmos(0x02);
    let mut hours = read_cmos(0x04);
    let mut day = read_cmos(0x07);
    let mut month = read_cmos(0x08);
    let mut year = read_cmos(0x09) as u16;

    // Read register B to check format
    let reg_b = read_cmos(0x0B);

    // Convert BCD to binary if needed
    if reg_b & 0x04 == 0 {
        seconds = bcd_to_binary(seconds);
        minutes = bcd_to_binary(minutes);
        hours = bcd_to_binary(hours & 0x7F) | (hours & 0x80);
        day = bcd_to_binary(day);
        month = bcd_to_binary(month);
        year = bcd_to_binary(year as u8) as u16;
    }

    // Convert 12-hour to 24-hour if needed
    if reg_b & 0x02 == 0 && hours & 0x80 != 0 {
        hours = ((hours & 0x7F) + 12) % 24;
    }

    // Assume 21st century
    year += 2000;

    RtcTime {
        seconds,
        minutes,
        hours,
        day,
        month,
        year,
    }
}

/// Read datetime as a tuple: (year, month, day, hour, minute, second)
pub fn read_datetime() -> (u16, u8, u8, u8, u8, u8) {
    let t = read_time();
    (t.year, t.month, t.day, t.hours, t.minutes, t.seconds)
}

/// Initialize the RTC driver and log the current date/time.
pub fn init() {
    let time = read_time();
    crate::serial_println!(
        "[OK] RTC: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        time.year, time.month, time.day,
        time.hours, time.minutes, time.seconds
    );
}
