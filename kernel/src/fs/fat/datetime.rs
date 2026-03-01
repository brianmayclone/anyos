//! DOS datetime <-> Unix timestamp conversion for FAT filesystems.

fn is_leap_year(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Days from 1970-01-01 to (year, month 1-12, day 1-31).
fn days_from_civil(year: u32, month: u32, day: u32) -> u32 {
    const CUMUL: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let mut days = 0u32;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    if month >= 1 && month <= 12 {
        days += CUMUL[(month - 1) as usize];
    }
    if month > 2 && is_leap_year(year) {
        days += 1;
    }
    days + day - 1
}

/// Convert (days since epoch) -> (year, month 1-12, day 1-31).
fn civil_from_days(total_days: u32) -> (u32, u32, u32) {
    let mut remaining = total_days;
    let mut year = 1970u32;
    loop {
        let dy = if is_leap_year(year) { 366 } else { 365 };
        if remaining < dy { break; }
        remaining -= dy;
        year += 1;
    }
    let leap = is_leap_year(year);
    const DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for m in 0..12u32 {
        let dim = if m == 1 && leap { 29 } else { DAYS[m as usize] };
        if remaining < dim {
            month = m + 1;
            break;
        }
        remaining -= dim;
        if m == 11 { month = 12; }
    }
    (year, month, remaining + 1)
}

/// Convert DOS date+time (FAT format) to Unix timestamp.
pub fn dos_datetime_to_unix(date: u16, time: u16) -> u32 {
    if date == 0 && time == 0 { return 0; }
    let year  = 1980 + ((date >> 9) & 0x7F) as u32;
    let month = ((date >> 5) & 0x0F) as u32;
    let day   = (date & 0x1F) as u32;
    let hours = ((time >> 11) & 0x1F) as u32;
    let mins  = ((time >> 5) & 0x3F) as u32;
    let secs  = ((time & 0x1F) * 2) as u32;
    if month < 1 || month > 12 || day < 1 || day > 31 { return 0; }
    days_from_civil(year, month, day) * 86400 + hours * 3600 + mins * 60 + secs
}

/// Convert Unix timestamp to DOS (date, time) pair.
pub fn unix_to_dos_datetime(ts: u32) -> (u16, u16) {
    if ts == 0 { return (0, 0); }
    let secs_of_day = ts % 86400;
    let total_days = ts / 86400;
    let hours = secs_of_day / 3600;
    let mins = (secs_of_day % 3600) / 60;
    let secs = secs_of_day % 60;
    let (year, month, day) = civil_from_days(total_days);
    let dos_year = if year >= 1980 { year - 1980 } else { 0 };
    let date = ((dos_year as u16) << 9) | ((month as u16) << 5) | (day as u16);
    let time = ((hours as u16) << 11) | ((mins as u16) << 5) | ((secs / 2) as u16);
    (date, time)
}

/// Get current RTC time as DOS (date, time) pair.
#[cfg(target_arch = "x86_64")]
pub(crate) fn current_dos_datetime() -> (u16, u16) {
    let rtc = crate::drivers::rtc::read_time();
    let year = if rtc.year >= 1980 { rtc.year as u32 - 1980 } else { 0 };
    let date = ((year as u16) << 9) | ((rtc.month as u16) << 5) | (rtc.day as u16);
    let time = ((rtc.hours as u16) << 11) | ((rtc.minutes as u16) << 5) | ((rtc.seconds as u16 / 2));
    (date, time)
}

/// ARM64 stub: returns a fixed date (2024-01-01 00:00:00) until an RTC driver is available.
#[cfg(target_arch = "aarch64")]
pub(crate) fn current_dos_datetime() -> (u16, u16) {
    // 2024-01-01 00:00:00 in DOS format
    let date = ((44u16) << 9) | (1u16 << 5) | 1u16; // year=2024-1980=44
    let time = 0u16;
    (date, time)
}
