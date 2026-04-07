// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Quoted-Printable encoding and decoding (RFC 2045).

use alloc::vec::Vec;

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Decode quoted-printable encoded data.
pub fn decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        if input[i] == b'=' {
            if i + 2 < input.len() {
                // Soft line break: =\r\n or =\n
                if input[i + 1] == b'\r' && i + 2 < input.len() && input[i + 2] == b'\n' {
                    i += 3;
                    continue;
                }
                if input[i + 1] == b'\n' {
                    i += 2;
                    continue;
                }
                // Hex-encoded byte: =XX
                if let (Some(hi), Some(lo)) = (hex_val(input[i + 1]), hex_val(input[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                    continue;
                }
            } else if i + 1 < input.len() && input[i + 1] == b'\n' {
                // Soft line break at end
                i += 2;
                continue;
            }
            // Malformed: pass through
            out.push(input[i]);
            i += 1;
        } else {
            out.push(input[i]);
            i += 1;
        }
    }

    out
}

/// Encode data as quoted-printable.
pub fn encode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 2);
    let mut col = 0;

    for &b in input {
        if b == b'\r' || b == b'\n' {
            out.push(b);
            col = 0;
            continue;
        }

        let needs_encode = b == b'=' || b < 0x20 && b != b'\t' || b > 0x7E;

        if needs_encode {
            // Check if we need soft line break (76 char limit)
            if col + 3 > 75 {
                out.push(b'=');
                out.push(b'\r');
                out.push(b'\n');
                col = 0;
            }
            out.push(b'=');
            out.push(hex_char(b >> 4));
            out.push(hex_char(b & 0x0F));
            col += 3;
        } else {
            if col + 1 > 75 {
                out.push(b'=');
                out.push(b'\r');
                out.push(b'\n');
                col = 0;
            }
            out.push(b);
            col += 1;
        }
    }

    out
}

fn hex_char(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0' + nibble
    } else {
        b'A' + nibble - 10
    }
}
