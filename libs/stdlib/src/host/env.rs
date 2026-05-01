// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Host-mode environment variables — delegates to std::env.

pub fn set(key: &str, value: &str) -> u32 {
    std::env::set_var(key, value);
    0
}

pub fn get(key: &str, buf: &mut [u8]) -> u32 {
    if let Ok(val) = std::env::var(key) {
        let bytes = val.as_bytes();
        let len = bytes.len().min(buf.len() - 1);
        buf[..len].copy_from_slice(&bytes[..len]);
        buf[len] = 0;
        len as u32
    } else {
        u32::MAX
    }
}

pub fn unset(key: &str) -> u32 {
    std::env::remove_var(key);
    0
}

pub fn list(buf: &mut [u8]) -> u32 {
    let mut offset = 0;
    for (key, val) in std::env::vars() {
        let entry = alloc::format!("{}={}\0", key, val);
        let bytes = entry.as_bytes();
        if offset + bytes.len() > buf.len() {
            break;
        }
        buf[offset..offset + bytes.len()].copy_from_slice(bytes);
        offset += bytes.len();
    }
    offset as u32
}
