// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! RFC 2047 encoded-word decoding.
//!
//! Handles `=?charset?encoding?text?=` patterns in email headers.
//! Supports B (base64) and Q (quoted-printable variant) encodings.

use super::base64;
use alloc::string::String;
use alloc::vec::Vec;

/// Decode RFC 2047 encoded words in a header value from raw bytes.
///
/// This function properly handles mixed ASCII and non-ASCII bytes,
/// decoding RFC 2047 encoded words while treating non-encoded non-ASCII
/// bytes as UTF-8 with Latin-1 fallback.
///
/// Example: `=?UTF-8?B?SGVsbG8gV29ybGQ=?=` -> `Hello World`
/// Example: `=?ISO-8859-1?Q?K=F6ln?=` -> `Koeln`
pub fn decode_header_bytes(data: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;

    while i < data.len() {
        // Look for =? start
        if i + 1 < data.len() && data[i] == b'=' && data[i + 1] == b'?' {
            if let Some((decoded, end)) = try_decode_word(&data[i..]) {
                result.push_str(&decoded);
                i += end;
                // Skip whitespace between consecutive encoded words
                while i < data.len() && (data[i] == b' ' || data[i] == b'\t') {
                    i += 1;
                }
                continue;
            }
        }

        // Regular character
        if data[i] < 0x80 {
            // ASCII character
            result.push(data[i] as char);
            i += 1;
        } else {
            // Non-ASCII: try to decode as UTF-8 multi-byte sequence
            let seq_len = utf8_seq_len(data[i]);
            if seq_len > 1 && i + seq_len <= data.len() {
                if let Ok(s) = core::str::from_utf8(&data[i..i + seq_len]) {
                    result.push_str(s);
                    i += seq_len;
                    continue;
                }
            }
            // Fall back to Latin-1: single byte as Unicode code point
            result.push(data[i] as char);
            i += 1;
        }
    }

    result
}

/// Determine UTF-8 sequence length from first byte.
fn utf8_seq_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 0, // Invalid start byte
    }
}

/// Decode RFC 2047 encoded words in a header value.
///
/// Example: `=?UTF-8?B?SGVsbG8gV29ybGQ=?=` -> `Hello World`
/// Example: `=?ISO-8859-1?Q?K=F6ln?=` -> `Koeln`
pub fn decode_header(input: &str) -> String {
    let mut result = String::new();
    let mut i = 0;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        // Look for =? start
        if i + 1 < bytes.len() && bytes[i] == b'=' && bytes[i + 1] == b'?' {
            if let Some((decoded, end)) = try_decode_word(&bytes[i..]) {
                result.push_str(&decoded);
                i += end;
                // Skip whitespace between consecutive encoded words
                let mut j = i;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j < bytes.len() + 1
                    && j + 1 < bytes.len()
                    && bytes[j] == b'='
                    && bytes[j + 1] == b'?'
                {
                    i = j; // skip whitespace between encoded words
                }
                continue;
            }
        }
        // Regular character
        if bytes[i] < 0x80 {
            result.push(bytes[i] as char);
        } else {
            result.push(bytes[i] as char);
        }
        i += 1;
    }

    result
}

/// Try to decode a single encoded word starting at `data`.
/// Returns (decoded_text, bytes_consumed) or None.
fn try_decode_word(data: &[u8]) -> Option<(String, usize)> {
    // Format: =?charset?encoding?encoded_text?=
    if data.len() < 8 || data[0] != b'=' || data[1] != b'?' {
        return None;
    }

    let rest = &data[2..];

    // Find charset end
    let charset_end = rest.iter().position(|&b| b == b'?')?;
    let charset = core::str::from_utf8(&rest[..charset_end]).ok()?;

    let after_charset = &rest[charset_end + 1..];
    if after_charset.is_empty() {
        return None;
    }

    // Get encoding (B or Q)
    let encoding = after_charset[0];
    if after_charset.len() < 2 || after_charset[1] != b'?' {
        return None;
    }

    let encoded_start = &after_charset[2..];

    // Find ?= terminator
    let mut end_pos = None;
    for j in 0..encoded_start.len() {
        if j + 1 < encoded_start.len() && encoded_start[j] == b'?' && encoded_start[j + 1] == b'=' {
            end_pos = Some(j);
            break;
        }
    }
    let end_pos = end_pos?;
    let encoded_text = &encoded_start[..end_pos];
    let total_consumed = 2 + charset_end + 1 + 1 + 1 + end_pos + 2;

    // Decode the content
    let raw_bytes = match encoding {
        b'B' | b'b' => base64::decode(encoded_text),
        b'Q' | b'q' => decode_q_encoding(encoded_text),
        _ => return None,
    };

    // Convert charset to UTF-8
    let text = convert_charset(charset, &raw_bytes);
    Some((text, total_consumed))
}

/// Q-encoding is like quoted-printable but _ means space.
fn decode_q_encoding(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        if input[i] == b'_' {
            out.push(b' ');
            i += 1;
        } else if input[i] == b'=' && i + 2 < input.len() {
            if let (Some(hi), Some(lo)) = (hex_val(input[i + 1]), hex_val(input[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
            } else {
                out.push(input[i]);
                i += 1;
            }
        } else {
            out.push(input[i]);
            i += 1;
        }
    }

    out
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Convert common charsets to UTF-8.
fn convert_charset(charset: &str, data: &[u8]) -> String {
    let cs = charset.to_ascii_uppercase();
    let cs = cs.as_str();

    match cs {
        "UTF-8" | "UTF8" => String::from(core::str::from_utf8(data).unwrap_or("")),
        "ISO-8859-1" | "LATIN1" | "LATIN-1" | "ISO_8859-1" | "ISO-8859-15" => latin1_to_utf8(data),
        "WINDOWS-1252" | "CP1252" => windows1252_to_utf8(data),
        "US-ASCII" | "ASCII" => {
            let mut s = String::new();
            for &b in data {
                s.push(if b < 0x80 { b as char } else { '?' });
            }
            s
        }
        _ => {
            // Unknown charset: try UTF-8, fallback to latin1
            match core::str::from_utf8(data) {
                Ok(s) => String::from(s),
                Err(_) => latin1_to_utf8(data),
            }
        }
    }
}

fn latin1_to_utf8(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push(b as char); // ISO-8859-1 maps directly to Unicode code points
    }
    s
}

fn windows1252_to_utf8(data: &[u8]) -> String {
    // Windows-1252 is like Latin-1 but with special chars in 0x80-0x9F
    static W1252_MAP: [char; 32] = [
        '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}',
        '\u{017D}', '\u{008F}', '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
    ];
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        if b >= 0x80 && b <= 0x9F {
            s.push(W1252_MAP[(b - 0x80) as usize]);
        } else {
            s.push(b as char);
        }
    }
    s
}

/// Helper trait for ASCII uppercase (no_std compatible).
trait AsciiUppercase {
    fn to_ascii_uppercase(&self) -> String;
}

impl AsciiUppercase for str {
    fn to_ascii_uppercase(&self) -> String {
        let mut s = String::with_capacity(self.len());
        for c in self.chars() {
            if c >= 'a' && c <= 'z' {
                s.push((c as u8 - 32) as char);
            } else {
                s.push(c);
            }
        }
        s
    }
}
