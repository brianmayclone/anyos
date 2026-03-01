//! Memory inspection and hex dump formatting.

use alloc::string::String;

/// Format a 16-byte-aligned hex dump line.
///
/// Format: "ADDR  HH HH HH HH HH HH HH HH  HH HH HH HH HH HH HH HH  |ASCII...........|"
pub fn format_hex_line(addr: u64, data: &[u8]) -> String {
    use crate::util::format::{hex64, hex_byte};

    let mut line = hex64(addr);
    line.push_str("  ");

    for i in 0..16 {
        if i == 8 {
            line.push(' ');
        }
        if i < data.len() {
            let h = hex_byte(data[i]);
            line.push(h[0] as char);
            line.push(h[1] as char);
        } else {
            line.push_str("  ");
        }
        line.push(' ');
    }

    line.push_str(" |");
    for i in 0..16 {
        if i < data.len() {
            let c = data[i];
            if c >= 0x20 && c < 0x7F {
                line.push(c as char);
            } else {
                line.push('.');
            }
        } else {
            line.push(' ');
        }
    }
    line.push('|');

    line
}

/// Format a multi-line hex dump.
pub fn format_hex_dump(base_addr: u64, data: &[u8]) -> String {
    let mut result = String::new();
    let mut offset = 0;

    while offset < data.len() {
        let line_addr = base_addr + offset as u64;
        let end = (offset + 16).min(data.len());
        let line = format_hex_line(line_addr, &data[offset..end]);
        result.push_str(&line);
        result.push('\n');
        offset += 16;
    }

    result
}
