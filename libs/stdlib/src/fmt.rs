// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Number and hex formatting utilities for `no_std` environments.
//!
//! All buffer-based functions write into caller-provided arrays and return
//! `&str` slices — zero allocation, zero copy.

use alloc::string::String;

// ── Decimal integers ────────────────────────────────────────────────────────

/// Format a `u32` as a decimal string.
pub fn fmt_u32<'a>(buf: &'a mut [u8; 12], val: u32) -> &'a str {
    if val == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut v = val;
    let mut tmp = [0u8; 12];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

/// Format an `i32` as a decimal string.
pub fn fmt_i32<'a>(buf: &'a mut [u8; 14], val: i32) -> &'a str {
    if val < 0 {
        buf[0] = b'-';
        let mut inner = [0u8; 12];
        let s = fmt_u32(&mut inner, (-(val as i64)) as u32);
        buf[1..1 + s.len()].copy_from_slice(s.as_bytes());
        return unsafe { core::str::from_utf8_unchecked(&buf[..1 + s.len()]) };
    }
    let mut inner = [0u8; 12];
    let s = fmt_u32(&mut inner, val as u32);
    buf[..s.len()].copy_from_slice(s.as_bytes());
    unsafe { core::str::from_utf8_unchecked(&buf[..s.len()]) }
}

/// Format a `u64` as a decimal string.
pub fn fmt_u64<'a>(buf: &'a mut [u8; 20], val: u64) -> &'a str {
    if val == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut v = val;
    let mut tmp = [0u8; 20];
    let mut n = 0;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..n]) }
}

/// Format an `i64` as a decimal string.
pub fn fmt_i64<'a>(buf: &'a mut [u8; 21], val: i64) -> &'a str {
    if val < 0 {
        buf[0] = b'-';
        let mut inner = [0u8; 20];
        let s = fmt_u64(&mut inner, (-(val as i128)) as u64);
        buf[1..1 + s.len()].copy_from_slice(s.as_bytes());
        return unsafe { core::str::from_utf8_unchecked(&buf[..1 + s.len()]) };
    }
    let mut inner = [0u8; 20];
    let s = fmt_u64(&mut inner, val as u64);
    buf[..s.len()].copy_from_slice(s.as_bytes());
    unsafe { core::str::from_utf8_unchecked(&buf[..s.len()]) }
}

// ── Floating point ──────────────────────────────────────────────────────────

/// Format an `f64` as a decimal string (allocating).
///
/// Handles NaN, infinity, integers, and up to 10 fractional digits
/// with trailing-zero stripping.
pub fn fmt_f64(val: f64) -> String {
    if val != val {
        return String::from("NaN");
    }
    if val == f64::INFINITY {
        return String::from("inf");
    }
    if val == f64::NEG_INFINITY {
        return String::from("-inf");
    }
    if val == 0.0 {
        return String::from("0");
    }

    let neg = val < 0.0;
    let v = if neg { -val } else { val };

    let int_part = v as u64;
    let frac = v - int_part as f64;

    // Pure integer?
    if frac.abs() < 1e-9 && int_part < 1_000_000_000_000 {
        let mut buf = [0u8; 20];
        let s = fmt_u64(&mut buf, int_part);
        if neg {
            let mut r = String::from("-");
            r.push_str(s);
            return r;
        }
        return String::from(s);
    }

    let mut s = String::new();
    if neg {
        s.push('-');
    }
    let mut buf = [0u8; 20];
    s.push_str(fmt_u64(&mut buf, int_part));
    s.push('.');

    let mut f = frac;
    let mut dec = [0u8; 10];
    for d in dec.iter_mut() {
        f *= 10.0;
        let digit = f as u8;
        *d = digit;
        f -= digit as f64;
    }
    // Strip trailing zeros
    let mut last = 0;
    for i in 0..10 {
        if dec[i] != 0 {
            last = i;
        }
    }
    for i in 0..=last {
        s.push((b'0' + dec[i]) as char);
    }
    s
}

// ── Hex formatting ──────────────────────────────────────────────────────────

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// Format a `u64` as `0x` + 16 zero-padded hex digits (allocating).
pub fn hex64(val: u64) -> String {
    let mut buf = [0u8; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        buf[2 + i] = HEX_CHARS[((val >> (60 - i * 4)) & 0xF) as usize];
    }
    String::from(unsafe { core::str::from_utf8_unchecked(&buf) })
}

/// Format a `u32` as `0x` + 8 zero-padded hex digits (allocating).
pub fn hex32(val: u32) -> String {
    let mut buf = [0u8; 10];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        buf[2 + i] = HEX_CHARS[((val >> (28 - i * 4)) & 0xF) as usize];
    }
    String::from(unsafe { core::str::from_utf8_unchecked(&buf) })
}

/// Format a `u8` as 2 zero-padded hex digits (no prefix).
pub fn hex_byte(val: u8) -> [u8; 2] {
    [HEX_CHARS[(val >> 4) as usize], HEX_CHARS[(val & 0xF) as usize]]
}

/// Format a byte slice as space-separated hex bytes (allocating).
pub fn hex_bytes(data: &[u8]) -> String {
    let mut s = String::new();
    for (i, &b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let h = hex_byte(b);
        s.push(h[0] as char);
        s.push(h[1] as char);
    }
    s
}

// ── Composite formatters ────────────────────────────────────────────────────

/// Format a percentage from a fixed-point x10 value (e.g. 155 → "15.5%").
pub fn fmt_pct<'a>(buf: &'a mut [u8; 12], pct_x10: u32) -> &'a str {
    let whole = pct_x10 / 10;
    let frac = pct_x10 % 10;
    let mut p = 0;
    let mut t = [0u8; 12];
    let s = fmt_u32(&mut t, whole);
    buf[p..p + s.len()].copy_from_slice(s.as_bytes());
    p += s.len();
    buf[p] = b'.';
    p += 1;
    buf[p] = b'0' + frac as u8;
    p += 1;
    buf[p] = b'%';
    p += 1;
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

/// Format a page count as human-readable memory size (4 KiB pages → "X.Y M" or "X K").
pub fn fmt_mem_pages<'a>(buf: &'a mut [u8; 16], pages: u32) -> &'a str {
    let kib = pages * 4;
    let mut t = [0u8; 12];
    let mut p = 0;
    if kib >= 1024 {
        let mib = kib / 1024;
        let frac = (kib % 1024) * 10 / 1024;
        let s = fmt_u32(&mut t, mib);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        buf[p] = b'.';
        p += 1;
        buf[p] = b'0' + frac as u8;
        p += 1;
        buf[p] = b'M';
        p += 1;
    } else {
        let s = fmt_u32(&mut t, kib);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        buf[p] = b'K';
        p += 1;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}

/// Format a byte count as human-readable size ("X.Y MiB", "X KiB", "X B").
pub fn fmt_bytes<'a>(buf: &'a mut [u8; 20], bytes: u64) -> &'a str {
    let mut t = [0u8; 12];
    let mut p = 0;
    if bytes >= 1024 * 1024 {
        let mib = bytes / (1024 * 1024);
        let frac = ((bytes % (1024 * 1024)) * 10 / (1024 * 1024)) as u32;
        let s = fmt_u32(&mut t, mib as u32);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        buf[p] = b'.';
        p += 1;
        buf[p] = b'0' + frac as u8;
        p += 1;
        buf[p..p + 4].copy_from_slice(b" MiB");
        p += 4;
    } else if bytes >= 1024 {
        let kib = bytes / 1024;
        let s = fmt_u32(&mut t, kib as u32);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        buf[p..p + 4].copy_from_slice(b" KiB");
        p += 4;
    } else {
        let s = fmt_u32(&mut t, bytes as u32);
        buf[p..p + s.len()].copy_from_slice(s.as_bytes());
        p += s.len();
        buf[p..p + 2].copy_from_slice(b" B");
        p += 2;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..p]) }
}
