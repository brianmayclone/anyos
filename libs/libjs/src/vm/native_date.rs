//! Date object — simplified, epoch-based.
//!
//! In a no_std/no-OS environment we cannot read the real clock,
//! so Date.now() returns 0 and new Date() creates epoch-zero.
//! The webview layer can override __date_now if a clock is available.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use core::cell::RefCell;

use crate::value::*;
use super::{Vm, native_fn};

// ═══════════════════════════════════════════════════════════
// Date constructor
// ═══════════════════════════════════════════════════════════

/// `new Date()` / `new Date(ms)` / `Date()`
pub fn ctor_date(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let ms = if args.is_empty() {
        // Try to get time from environment
        date_now_ms(vm)
    } else if args.len() == 1 {
        match &args[0] {
            JsValue::Number(n) => *n,
            JsValue::String(s) => parse_date_string(s),
            _ => f64::NAN,
        }
    } else {
        // Date(year, month, day, hours, minutes, seconds, ms)
        let year = args.first().map(|v| v.to_number()).unwrap_or(0.0);
        let month = args.get(1).map(|v| v.to_number()).unwrap_or(0.0);
        let day = args.get(2).map(|v| v.to_number()).unwrap_or(1.0);
        let hours = args.get(3).map(|v| v.to_number()).unwrap_or(0.0);
        let minutes = args.get(4).map(|v| v.to_number()).unwrap_or(0.0);
        let seconds = args.get(5).map(|v| v.to_number()).unwrap_or(0.0);
        let ms = args.get(6).map(|v| v.to_number()).unwrap_or(0.0);
        // Simplified: compute epoch ms from components
        let y = if year >= 0.0 && year < 100.0 { year + 1900.0 } else { year };
        compute_epoch_ms(y, month, day, hours, minutes, seconds, ms)
    };

    let mut obj = JsObject::new();
    obj.internal_tag = Some(String::from("__date__"));
    obj.set(String::from("__ms"), JsValue::Number(ms));
    // Install methods
    obj.set(String::from("getTime"), native_fn("getTime", date_get_time));
    obj.set(String::from("valueOf"), native_fn("valueOf", date_get_time));
    obj.set(String::from("getFullYear"), native_fn("getFullYear", date_get_full_year));
    obj.set(String::from("getMonth"), native_fn("getMonth", date_get_month));
    obj.set(String::from("getDate"), native_fn("getDate", date_get_date));
    obj.set(String::from("getDay"), native_fn("getDay", date_get_day));
    obj.set(String::from("getHours"), native_fn("getHours", date_get_hours));
    obj.set(String::from("getMinutes"), native_fn("getMinutes", date_get_minutes));
    obj.set(String::from("getSeconds"), native_fn("getSeconds", date_get_seconds));
    obj.set(String::from("getMilliseconds"), native_fn("getMilliseconds", date_get_milliseconds));
    obj.set(String::from("setTime"), native_fn("setTime", date_set_time));
    obj.set(String::from("toISOString"), native_fn("toISOString", date_to_iso_string));
    obj.set(String::from("toJSON"), native_fn("toJSON", date_to_iso_string));
    obj.set(String::from("toString"), native_fn("toString", date_to_string));
    obj.set(String::from("toLocaleDateString"), native_fn("toLocaleDateString", date_to_string));
    obj.set(String::from("toLocaleTimeString"), native_fn("toLocaleTimeString", date_to_string));
    obj.set(String::from("toLocaleString"), native_fn("toLocaleString", date_to_string));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// `Date.now()` — returns ms since epoch.
pub fn date_now(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(date_now_ms(vm))
}

/// `Date.parse(string)` — parse a date string.
pub fn date_parse(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    JsValue::Number(parse_date_string(&s))
}

// ═══════════════════════════════════════════════════════════
// Instance methods
// ═══════════════════════════════════════════════════════════

fn get_ms(vm: &Vm) -> f64 {
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(p) = o.properties.get("__ms") {
            return p.value.to_number();
        }
    }
    f64::NAN
}

/// Decompose epoch ms into (year, month 0-11, day 1-31, hours, mins, secs, ms).
fn decompose(epoch_ms: f64) -> (i64, u32, u32, u32, u32, u32, u32) {
    if epoch_ms.is_nan() || epoch_ms.is_infinite() {
        return (1970, 0, 1, 0, 0, 0, 0);
    }
    let total_ms = epoch_ms as i64;
    let ms_per_sec: i64 = 1000;
    let ms_per_min: i64 = 60 * ms_per_sec;
    let ms_per_hour: i64 = 60 * ms_per_min;
    let ms_per_day: i64 = 24 * ms_per_hour;

    let mut remaining = total_ms;
    let millis = ((remaining % ms_per_sec) + ms_per_sec) % ms_per_sec;
    remaining /= ms_per_sec;
    let secs = ((remaining % 60) + 60) % 60;
    remaining /= 60;
    let mins = ((remaining % 60) + 60) % 60;
    remaining /= 60;
    let hours = ((remaining % 24) + 24) % 24;
    let mut days = total_ms / ms_per_day;
    if total_ms < 0 && total_ms % ms_per_day != 0 { days -= 1; }

    // Civil date from days since epoch (1970-01-01 = day 0)
    let mut y: i64 = 1970;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if days >= days_in_year {
            days -= days_in_year;
            y += 1;
        } else if days < 0 {
            y -= 1;
            let diy = if is_leap(y) { 366 } else { 365 };
            days += diy;
        } else {
            break;
        }
    }

    let leap = is_leap(y);
    let month_days: [i64; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0u32;
    for (m, &md) in month_days.iter().enumerate() {
        if days < md {
            month = m as u32;
            break;
        }
        days -= md;
    }
    let day = days as u32 + 1;

    (y, month, day, hours as u32, mins as u32, secs as u32, millis as u32)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn date_get_time(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(get_ms(vm))
}

pub fn date_get_full_year(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let (y, ..) = decompose(get_ms(vm));
    JsValue::Number(y as f64)
}

pub fn date_get_month(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let (_, m, ..) = decompose(get_ms(vm));
    JsValue::Number(m as f64)
}

pub fn date_get_date(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let (_, _, d, ..) = decompose(get_ms(vm));
    JsValue::Number(d as f64)
}

pub fn date_get_day(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let ms = get_ms(vm);
    if ms.is_nan() { return JsValue::Number(f64::NAN); }
    // Day 0 (1970-01-01) was Thursday (4)
    let days = super::native_math::floor_f64(ms / 86_400_000.0) as i64;
    let day = ((days % 7 + 4) % 7 + 7) % 7;
    JsValue::Number(day as f64)
}

pub fn date_get_hours(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let (_, _, _, h, ..) = decompose(get_ms(vm));
    JsValue::Number(h as f64)
}

pub fn date_get_minutes(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let (_, _, _, _, m, ..) = decompose(get_ms(vm));
    JsValue::Number(m as f64)
}

pub fn date_get_seconds(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let (_, _, _, _, _, s, _) = decompose(get_ms(vm));
    JsValue::Number(s as f64)
}

pub fn date_get_milliseconds(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let (_, _, _, _, _, _, ms) = decompose(get_ms(vm));
    JsValue::Number(ms as f64)
}

pub fn date_set_time(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let ms = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().set(String::from("__ms"), JsValue::Number(ms));
    }
    JsValue::Number(ms)
}

pub fn date_to_iso_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let ms_val = get_ms(vm);
    if ms_val.is_nan() { return JsValue::String(String::from("Invalid Date")); }
    let (y, mo, d, h, mi, s, ms) = decompose(ms_val);
    let result = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", y, mo + 1, d, h, mi, s, ms);
    JsValue::String(result)
}

pub fn date_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let ms_val = get_ms(vm);
    if ms_val.is_nan() { return JsValue::String(String::from("Invalid Date")); }
    let (y, mo, d, h, mi, s, _) = decompose(ms_val);
    let months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
    let days = ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"];
    let day_of_week = {
        let total_days = super::native_math::floor_f64(ms_val / 86_400_000.0) as i64;
        (((total_days % 7 + 4) % 7 + 7) % 7) as usize
    };
    let result = format!("{} {} {:02} {} {:02}:{:02}:{:02} GMT",
        days[day_of_week], months[mo as usize], d, y, h, mi, s);
    JsValue::String(result)
}

// ═══════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════

fn date_now_ms(vm: &mut Vm) -> f64 {
    // Check if the host provided a __date_now function
    let val = vm.get_global("__date_now");
    if let JsValue::Function(f) = val {
        let kind = f.borrow().kind.clone();
        if let FnKind::Native(native) = kind {
            let result = native(vm, &[]);
            return result.to_number();
        }
    }
    0.0
}

fn parse_date_string(s: &str) -> f64 {
    // Very simplified ISO 8601 parser: "YYYY-MM-DDTHH:MM:SS.mmmZ"
    let s = s.trim();
    if s.is_empty() { return f64::NAN; }

    let bytes = s.as_bytes();
    let mut parts = [0i64; 7]; // year, month, day, hours, mins, secs, ms

    let mut i = 0;
    let mut part = 0;

    // Year
    let neg = i < bytes.len() && bytes[i] == b'-';
    if neg { i += 1; }
    while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' && part == 0 {
        parts[0] = parts[0] * 10 + (bytes[i] - b'0') as i64;
        i += 1;
    }
    if neg { parts[0] = -parts[0]; }

    // Parse remaining with separators
    let separators = [b'-', b'-', b'T', b':', b':', b'.'];
    for (si, &sep) in separators.iter().enumerate() {
        if i < bytes.len() && bytes[i] == sep {
            i += 1;
            part = si + 1;
            if part >= 7 { break; }
            while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
                parts[part] = parts[part] * 10 + (bytes[i] - b'0') as i64;
                i += 1;
            }
        }
    }

    // Default month=1, day=1 for missing parts
    if parts[1] == 0 { parts[1] = 1; }
    if parts[2] == 0 { parts[2] = 1; }

    compute_epoch_ms(
        parts[0] as f64,
        (parts[1] - 1) as f64, // month 0-indexed
        parts[2] as f64,
        parts[3] as f64,
        parts[4] as f64,
        parts[5] as f64,
        parts[6] as f64,
    )
}

fn compute_epoch_ms(year: f64, month: f64, day: f64, hours: f64, minutes: f64, seconds: f64, ms: f64) -> f64 {
    let y = year as i64;
    let m = month as i64; // 0-indexed

    // Days from epoch to start of year
    let mut total_days: i64 = 0;
    if y >= 1970 {
        for yr in 1970..y { total_days += if is_leap(yr) { 366 } else { 365 }; }
    } else {
        for yr in y..1970 { total_days -= if is_leap(yr) { 366 } else { 365 }; }
    }

    // Add days for months
    let leap = is_leap(y);
    let month_days: [i64; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for i in 0..m.min(11) as usize {
        total_days += month_days[i];
    }
    total_days += (day as i64) - 1;

    let total_ms = total_days * 86_400_000
        + (hours as i64) * 3_600_000
        + (minutes as i64) * 60_000
        + (seconds as i64) * 1000
        + ms as i64;

    total_ms as f64
}
