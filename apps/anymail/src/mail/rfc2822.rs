// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! RFC 2822 email address and date parsing.

use alloc::string::String;
use alloc::vec::Vec;

/// A parsed email address with optional display name.
#[derive(Clone)]
pub struct EmailAddress {
    pub name: String,    // Display name (may be empty)
    pub address: String, // user@domain
}

impl EmailAddress {
    pub fn new(address: &str) -> Self {
        Self {
            name: String::new(),
            address: String::from(address.trim()),
        }
    }

    pub fn with_name(name: &str, address: &str) -> Self {
        Self {
            name: String::from(name.trim()),
            address: String::from(address.trim()),
        }
    }

    /// Format as `"Name" <address>` or just `address`.
    pub fn to_header_string(&self) -> String {
        if self.name.is_empty() {
            self.address.clone()
        } else {
            let mut s = String::from("\"");
            s.push_str(&self.name);
            s.push_str("\" <");
            s.push_str(&self.address);
            s.push('>');
            s
        }
    }

    /// Short display: name if available, otherwise address.
    pub fn display_short(&self) -> &str {
        if self.name.is_empty() {
            &self.address
        } else {
            &self.name
        }
    }
}

/// Parse a single address from a header value.
/// Handles: `"Name" <addr>`, `Name <addr>`, `<addr>`, `addr`
pub fn parse_address(input: &str) -> EmailAddress {
    let s = input.trim();

    // "Name" <addr> or Name <addr>
    if let Some(lt) = s.find('<') {
        if let Some(gt) = s.find('>') {
            let addr = &s[lt + 1..gt];
            let mut name = s[..lt].trim();
            // Strip quotes
            if name.starts_with('"') && name.ends_with('"') && name.len() >= 2 {
                name = &name[1..name.len() - 1];
            }
            return EmailAddress::with_name(name, addr);
        }
    }

    // Just an address
    EmailAddress::new(s)
}

/// Parse a comma-separated list of addresses.
pub fn parse_address_list(input: &str) -> Vec<EmailAddress> {
    let mut addrs = Vec::new();
    // Simple split on comma (doesn't handle quoted commas, but sufficient for most emails)
    for part in input.split(',') {
        let part = part.trim();
        if !part.is_empty() {
            addrs.push(parse_address(part));
        }
    }
    addrs
}

/// Parse an RFC 2822 date string into a sortable timestamp string.
/// Handles common formats:
///   `Thu, 27 Feb 2026 14:30:00 +0100`
///   `27 Feb 2026 14:30:00 GMT`
pub fn parse_date(input: &str) -> String {
    let s = input.trim();

    // Skip optional day-of-week
    let rest = if let Some(comma) = s.find(',') {
        s[comma + 1..].trim()
    } else {
        s
    };

    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 4 {
        return String::from(s);
    }

    let day: u32 = parts[0].parse().unwrap_or(0);
    let month = month_to_num(parts[1]);
    let year: u32 = parts[2].parse().unwrap_or(0);
    let time = parts[3];

    if day == 0 || month == 0 || year == 0 {
        return String::from(s);
    }

    // Format: YYYY-MM-DD HH:MM:SS
    let mut result = String::new();
    push_padded(&mut result, year, 4);
    result.push('-');
    push_padded(&mut result, month, 2);
    result.push('-');
    push_padded(&mut result, day, 2);
    result.push(' ');
    result.push_str(time);

    result
}

/// Format a date string for display (shorter form).
pub fn format_date_short(date_str: &str) -> String {
    // If already in YYYY-MM-DD HH:MM:SS format, show DD.MM.YYYY HH:MM
    if date_str.len() >= 16 && date_str.as_bytes()[4] == b'-' {
        let mut s = String::new();
        s.push_str(&date_str[8..10]); // DD
        s.push('.');
        s.push_str(&date_str[5..7]); // MM
        s.push('.');
        s.push_str(&date_str[0..4]); // YYYY
        s.push(' ');
        s.push_str(&date_str[11..16]); // HH:MM
        return s;
    }
    String::from(date_str)
}

fn month_to_num(m: &str) -> u32 {
    match m.as_bytes().get(..3) {
        Some(b"Jan") | Some(b"jan") => 1,
        Some(b"Feb") | Some(b"feb") => 2,
        Some(b"Mar") | Some(b"mar") => 3,
        Some(b"Apr") | Some(b"apr") => 4,
        Some(b"May") | Some(b"may") => 5,
        Some(b"Jun") | Some(b"jun") => 6,
        Some(b"Jul") | Some(b"jul") => 7,
        Some(b"Aug") | Some(b"aug") => 8,
        Some(b"Sep") | Some(b"sep") => 9,
        Some(b"Oct") | Some(b"oct") => 10,
        Some(b"Nov") | Some(b"nov") => 11,
        Some(b"Dec") | Some(b"dec") => 12,
        _ => 0,
    }
}

fn push_padded(s: &mut String, val: u32, width: usize) {
    let mut buf = [0u8; 10];
    let mut n = val;
    let mut len = 0;
    if n == 0 {
        buf[0] = b'0';
        len = 1;
    } else {
        while n > 0 {
            buf[len] = b'0' + (n % 10) as u8;
            n /= 10;
            len += 1;
        }
    }
    // Pad with zeros
    for _ in len..width {
        s.push('0');
    }
    // Push digits in reverse
    for i in (0..len).rev() {
        s.push(buf[i] as char);
    }
}

/// Format a size in bytes for display (KB, MB).
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        let mut s = String::new();
        push_u64(&mut s, bytes);
        s.push_str(" B");
        s
    } else if bytes < 1024 * 1024 {
        let kb = bytes / 1024;
        let mut s = String::new();
        push_u64(&mut s, kb);
        s.push_str(" KB");
        s
    } else {
        let mb = bytes / (1024 * 1024);
        let frac = (bytes % (1024 * 1024)) * 10 / (1024 * 1024);
        let mut s = String::new();
        push_u64(&mut s, mb);
        s.push('.');
        push_u64(&mut s, frac);
        s.push_str(" MB");
        s
    }
}

fn push_u64(s: &mut String, val: u64) {
    if val == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut n = val;
    let mut len = 0;
    while n > 0 {
        buf[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    for i in (0..len).rev() {
        s.push(buf[i] as char);
    }
}

/// Extract the domain from an email address.
pub fn domain_of(email: &str) -> &str {
    if let Some(at) = email.find('@') {
        &email[at + 1..]
    } else {
        email
    }
}

/// Extract the local part from an email address.
pub fn local_part_of(email: &str) -> &str {
    if let Some(at) = email.find('@') {
        &email[..at]
    } else {
        email
    }
}
