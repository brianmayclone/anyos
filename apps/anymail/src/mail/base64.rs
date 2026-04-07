// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Base64 encoding and decoding (RFC 4648).

use alloc::vec::Vec;

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_val(c: u8) -> u8 {
    match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => 0xFF,
    }
}

/// Encode bytes to base64 string.
pub fn encode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;

    while i + 3 <= input.len() {
        let b0 = input[i] as u32;
        let b1 = input[i + 1] as u32;
        let b2 = input[i + 2] as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_CHARS[((triple >> 18) & 0x3F) as usize]);
        out.push(B64_CHARS[((triple >> 12) & 0x3F) as usize]);
        out.push(B64_CHARS[((triple >> 6) & 0x3F) as usize]);
        out.push(B64_CHARS[(triple & 0x3F) as usize]);
        i += 3;
    }

    let rem = input.len() - i;
    if rem == 1 {
        let b0 = input[i] as u32;
        out.push(B64_CHARS[((b0 >> 2) & 0x3F) as usize]);
        out.push(B64_CHARS[((b0 << 4) & 0x3F) as usize]);
        out.push(b'=');
        out.push(b'=');
    } else if rem == 2 {
        let b0 = input[i] as u32;
        let b1 = input[i + 1] as u32;
        out.push(B64_CHARS[((b0 >> 2) & 0x3F) as usize]);
        out.push(B64_CHARS[(((b0 << 4) | (b1 >> 4)) & 0x3F) as usize]);
        out.push(B64_CHARS[((b1 << 2) & 0x3F) as usize]);
        out.push(b'=');
    }

    out
}

/// Encode bytes to base64 string (returns String).
pub fn encode_str(input: &[u8]) -> alloc::string::String {
    let bytes = encode(input);
    // Safe: base64 is always ASCII
    unsafe { alloc::string::String::from_utf8_unchecked(bytes) }
}

/// Decode base64 bytes. Skips whitespace and stops at '=' padding.
pub fn decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = [0u8; 4];
    let mut buf_len = 0;

    for &c in input {
        if c == b'\n' || c == b'\r' || c == b' ' || c == b'\t' {
            continue;
        }
        if c == b'=' {
            break;
        }
        let v = b64_val(c);
        if v == 0xFF {
            continue;
        }
        buf[buf_len] = v;
        buf_len += 1;
        if buf_len == 4 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
            out.push((buf[2] << 6) | buf[3]);
            buf_len = 0;
        }
    }

    if buf_len == 2 {
        out.push((buf[0] << 2) | (buf[1] >> 4));
    } else if buf_len == 3 {
        out.push((buf[0] << 2) | (buf[1] >> 4));
        out.push((buf[1] << 4) | (buf[2] >> 2));
    }

    out
}

/// Decode base64 from a &str.
pub fn decode_str(input: &str) -> Vec<u8> {
    decode(input.as_bytes())
}
