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

fn write_cmos(reg: u8, val: u8) {
    unsafe {
        outb(0x70, reg);
        outb(0x71, val);
    }
}

fn bcd_to_binary(bcd: u8) -> u8 {
    (bcd & 0x0F) + ((bcd >> 4) * 10)
}

fn binary_to_bcd(val: u8) -> u8 {
    ((val / 10) << 4) | (val % 10)
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

/// Set the RTC date/time.
///
/// Disables RTC update cycle, writes all registers, re-enables updates.
pub fn set_time(year: u16, month: u8, day: u8, hour: u8, min: u8, sec: u8) {
    // Read register B to determine BCD vs binary mode.
    let reg_b = read_cmos(0x0B);
    let is_bcd = reg_b & 0x04 == 0;

    // Disable RTC updates (set bit 7 of register B).
    write_cmos(0x0B, reg_b | 0x80);

    // Year: RTC stores only 2-digit year (0-99), we assume 21st century.
    let year2 = (year % 100) as u8;

    if is_bcd {
        write_cmos(0x00, binary_to_bcd(sec));
        write_cmos(0x02, binary_to_bcd(min));
        write_cmos(0x04, binary_to_bcd(hour));
        write_cmos(0x07, binary_to_bcd(day));
        write_cmos(0x08, binary_to_bcd(month));
        write_cmos(0x09, binary_to_bcd(year2));
    } else {
        write_cmos(0x00, sec);
        write_cmos(0x02, min);
        write_cmos(0x04, hour);
        write_cmos(0x07, day);
        write_cmos(0x08, month);
        write_cmos(0x09, year2);
    }

    // Re-enable RTC updates (clear bit 7 of register B).
    write_cmos(0x0B, reg_b & !0x80);
}

// ── CMOS NVRAM (registers 0x10–0x7F) ────────────────────────────────────────

/// NVRAM usable range: registers 0x10–0x7F (112 bytes).
/// Registers 0x00–0x0F are reserved for RTC time/alarm/status.
const NVRAM_START: u8 = 0x10;
const NVRAM_END: u8 = 0x7F;

/// Read a single byte from CMOS NVRAM.
/// `addr` is the CMOS register (0x10–0x7F). Returns 0 if out of range.
pub fn nvram_read(addr: u8) -> u8 {
    if addr < NVRAM_START || addr > NVRAM_END {
        return 0;
    }
    read_cmos(addr)
}

/// Write a single byte to CMOS NVRAM.
/// `addr` is the CMOS register (0x10–0x7F). Ignored if out of range.
pub fn nvram_write(addr: u8, val: u8) {
    if addr < NVRAM_START || addr > NVRAM_END {
        return;
    }
    write_cmos(addr, val);
}

/// Read a block of bytes from CMOS NVRAM starting at `start_addr`.
/// Returns the number of bytes read.
pub fn nvram_read_block(start_addr: u8, buf: &mut [u8]) -> usize {
    let mut count = 0;
    let mut addr = start_addr;
    for byte in buf.iter_mut() {
        if addr < NVRAM_START || addr > NVRAM_END {
            break;
        }
        *byte = read_cmos(addr);
        addr = addr.wrapping_add(1);
        count += 1;
    }
    count
}

/// Write a block of bytes to CMOS NVRAM starting at `start_addr`.
/// Returns the number of bytes written.
pub fn nvram_write_block(start_addr: u8, data: &[u8]) -> usize {
    let mut count = 0;
    let mut addr = start_addr;
    for &byte in data {
        if addr < NVRAM_START || addr > NVRAM_END {
            break;
        }
        write_cmos(addr, byte);
        addr = addr.wrapping_add(1);
        count += 1;
    }
    count
}

// ── Initialization ──────────────────────────────────────────────────────────

/// Initialize the RTC driver and log the current date/time.
pub fn init() {
    let time = read_time();
    crate::serial_verbose_println!(
        "[OK] RTC: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        time.year, time.month, time.day,
        time.hours, time.minutes, time.seconds
    );
}
