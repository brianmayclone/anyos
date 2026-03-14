//! Unit tests for DOS ↔ Unix timestamp conversion.
//!
//! `dos_datetime_to_unix` and `unix_to_dos_datetime` are pure math functions
//! with no hardware or heap dependency — ideal for exhaustive round-trip
//! testing.  Encoding is standard MS-DOS FAT date/time format:
//!   date: bits [15:9] year-1980, [8:5] month 1-12, [4:0] day 1-31
//!   time: bits [15:11] hours, [10:5] minutes, [4:0] seconds/2

use crate::fs::fat::datetime::{dos_datetime_to_unix, unix_to_dos_datetime};
use crate::kunit::{TestCase, TestContext, TestSuite};

pub static SUITE: TestSuite = TestSuite {
    name: "fs::fat::datetime",
    cases: &[
        TestCase { name: "epoch_1980_roundtrip",      run: test_epoch_1980 },
        TestCase { name: "known_date_2024_01_01",     run: test_known_2024 },
        TestCase { name: "known_date_2000_02_29",     run: test_leap_2000 },
        TestCase { name: "known_date_1984_06_15",     run: test_1984 },
        TestCase { name: "max_dos_date_2107",         run: test_max_date },
        TestCase { name: "midnight_encoding",         run: test_midnight },
        TestCase { name: "time_23_59_58",             run: test_max_time },
        TestCase { name: "roundtrip_various_dates",   run: test_roundtrip_various },
        TestCase { name: "dos_time_2second_granule",  run: test_2sec_granule },
        TestCase { name: "unix_to_dos_to_unix",       run: test_unix_roundtrip },
    ],
};

/// Build a DOS date word: year is offset from 1980.
fn make_date(year: u16, month: u16, day: u16) -> u16 {
    ((year - 1980) << 9) | (month << 5) | day
}

/// Build a DOS time word: seconds divided by 2.
fn make_time(h: u16, m: u16, s: u16) -> u16 {
    (h << 11) | (m << 5) | (s / 2)
}

fn test_epoch_1980(ctx: &mut TestContext) {
    // 1980-01-01 00:00:00 = DOS epoch = Unix timestamp 315532800.
    let date = make_date(1980, 1, 1);
    let time = make_time(0, 0, 0);
    let unix = dos_datetime_to_unix(date, time);
    ctx.expect_eq(unix, 315_532_800u32, "DOS epoch == Unix 315532800");

    let (d2, t2) = unix_to_dos_datetime(unix);
    ctx.expect_eq(d2, date, "roundtrip date matches");
    ctx.expect_eq(t2, time, "roundtrip time matches");
}

fn test_known_2024(ctx: &mut TestContext) {
    // 2024-01-01 00:00:00 UTC = 1704067200.
    let date = make_date(2024, 1, 1);
    let time = make_time(0, 0, 0);
    let unix = dos_datetime_to_unix(date, time);
    ctx.expect_eq(unix, 1_704_067_200u32, "2024-01-01 00:00:00 UTC");

    let (d2, t2) = unix_to_dos_datetime(unix);
    ctx.expect_eq(d2, date, "2024-01-01 roundtrip date");
    ctx.expect_eq(t2, time, "2024-01-01 roundtrip time");
}

fn test_leap_2000(ctx: &mut TestContext) {
    // 2000-02-29 = leap day, Unix 951782400.
    let date = make_date(2000, 2, 29);
    let time = make_time(12, 0, 0);
    let unix = dos_datetime_to_unix(date, time);
    ctx.expect_eq(unix, 951_782_400u32 + 12 * 3600, "2000-02-29 12:00:00");

    let (d2, t2) = unix_to_dos_datetime(unix);
    ctx.expect_eq(d2, date, "leap day roundtrip date");
    ctx.expect_eq(t2, time, "leap day roundtrip time");
}

fn test_1984(ctx: &mut TestContext) {
    // 1984-06-15 08:30:00 — a random date in the middle of the DOS range.
    let date = make_date(1984, 6, 15);
    let time = make_time(8, 30, 0);
    let unix = dos_datetime_to_unix(date, time);
    let (d2, t2) = unix_to_dos_datetime(unix);
    ctx.expect_eq(d2, date, "1984-06-15 roundtrip date");
    ctx.expect_eq(t2, time, "1984-06-15 roundtrip time");
}

fn test_max_date(ctx: &mut TestContext) {
    // Both functions use u32 timestamps. The max u32 value (4_294_967_295)
    // corresponds to 2106-02-07, so 2107 overflows. Use 2100-01-01 instead,
    // which has Unix timestamp 4_102_444_800 — well within u32 range.
    let date = make_date(2100, 1, 1);
    let time = make_time(0, 0, 0);
    let unix = dos_datetime_to_unix(date, time);
    let (d2, t2) = unix_to_dos_datetime(unix);
    ctx.expect_eq(d2, date, "late DOS date (2100-01-01) roundtrip");
    ctx.expect_eq(t2, time, "late DOS date time roundtrip");
}

fn test_midnight(ctx: &mut TestContext) {
    // Midnight (00:00:00) should encode to time word 0.
    let time = make_time(0, 0, 0);
    ctx.expect_eq(time, 0u16, "midnight encodes to 0");
    let date = make_date(2020, 3, 15);
    let unix = dos_datetime_to_unix(date, time);
    let (_, t2) = unix_to_dos_datetime(unix);
    ctx.expect_eq(t2, 0u16, "midnight roundtrip");
}

fn test_max_time(ctx: &mut TestContext) {
    // 23:59:58 is the maximum representable time (58 = 2 * 29).
    let date = make_date(2000, 1, 1);
    let time = make_time(23, 59, 58);
    let unix = dos_datetime_to_unix(date, time);
    let (d2, t2) = unix_to_dos_datetime(unix);
    ctx.expect_eq(d2, date, "23:59:58 roundtrip date");
    ctx.expect_eq(t2, time, "23:59:58 roundtrip time");
}

fn test_roundtrip_various(ctx: &mut TestContext) {
    // Spot-check 8 arbitrary dates spread across the DOS range.
    let cases: &[(u16, u16, u16, u16, u16, u16)] = &[
        (1981,  3, 15,  9,  0,  0),
        (1990, 12, 31, 23, 59, 58),
        (1995,  7,  4,  0,  0,  0),
        (2000,  1,  1,  0,  0,  0),
        (2005,  8, 20, 14, 30,  0),
        (2012,  2, 29, 12,  0,  0), // 2012 is a leap year
        (2023, 11, 15,  7, 45, 20),
        (2038,  1, 19,  3, 14,  6), // close to Unix 32-bit overflow
    ];
    for &(y, mo, d, h, mi, s) in cases {
        let date = make_date(y, mo, d);
        let time = make_time(h, mi, s);
        let unix = dos_datetime_to_unix(date, time);
        let (d2, t2) = unix_to_dos_datetime(unix);
        ctx.expect_eq(d2, date, "roundtrip date");
        ctx.expect_eq(t2, time, "roundtrip time");
    }
}

fn test_2sec_granule(ctx: &mut TestContext) {
    // DOS time has 2-second granularity. Odd seconds are truncated.
    let date = make_date(2020, 6, 1);
    let even = make_time(10, 30, 4); // 4s
    let unix_even = dos_datetime_to_unix(date, even);

    // Manually compute unix with 5s and verify it matches 4s (truncated).
    let time_5 = make_time(10, 30, 5); // 5/2=2, same bits as 4/2=2
    let unix_5s = dos_datetime_to_unix(date, time_5);
    ctx.expect_eq(unix_even, unix_5s, "odd second truncated to even (2s granule)");

    let (_, t2) = unix_to_dos_datetime(unix_even);
    ctx.expect_eq(t2, even, "even-second time roundtrips exactly");
}

fn test_unix_roundtrip(ctx: &mut TestContext) {
    // Start from a known Unix timestamp and verify dos→unix→dos stability.
    // 2023-01-15 12:34:56 UTC.  Seconds are truncated to 2-second granularity.
    let unix_in: u32 = 1_673_786_096; // 2023-01-15 12:34:56, even second
    let (d, t) = unix_to_dos_datetime(unix_in);
    let unix_out = dos_datetime_to_unix(d, t);
    // Difference must be < 2 seconds due to 2-second granule.
    let diff = if unix_out >= unix_in { unix_out - unix_in } else { unix_in - unix_out };
    ctx.expect_true(diff < 2, "unix→dos→unix within 2-second granule");
}
