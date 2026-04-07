// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Host-mode system functions.

pub fn random(buf: &mut [u8]) -> u32 {
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(buf).is_ok() {
            return buf.len() as u32;
        }
    }
    // Fallback: fill with pseudo-random
    for b in buf.iter_mut() {
        *b = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() & 0xFF) as u8;
    }
    buf.len() as u32
}

pub fn time(buf: &mut [u8; 8]) -> u32 {
    // Stub: return zeros
    buf.fill(0);
    0
}

pub fn set_time(_buf: &[u8; 8]) -> u32 { u32::MAX }

pub fn uptime() -> u32 { 0 }

pub fn tick_hz() -> u32 { 1000 }

pub fn uptime_ms() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u32
}
