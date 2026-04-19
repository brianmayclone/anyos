//! In-memory index support for libdb.
//!
//! Index definitions themselves are persisted as part of the table schema,
//! while the lookup maps are rebuilt in memory on open or schema changes.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::types::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RowLocation {
    pub page_num: u32,
    pub offset: usize,
}

pub type IndexMap = BTreeMap<String, Vec<RowLocation>>;

#[derive(Clone)]
pub struct TableIndexState {
    pub maps: Vec<IndexMap>,
}

pub fn encode_index_key(value: &Value) -> String {
    match value {
        Value::Null => String::from("n:"),
        Value::Integer(v) => {
            let mut out = String::from("i:");
            push_i64(&mut out, *v);
            out
        }
        Value::Text(s) => {
            let mut out = String::from("t:");
            for b in s.bytes() {
                out.push((if b.is_ascii_uppercase() { b.to_ascii_lowercase() } else { b }) as char);
            }
            out
        }
        Value::Blob(blob) => {
            let mut out = String::from("b:");
            for byte in blob {
                out.push(hex_digit(byte >> 4));
                out.push(hex_digit(byte & 0x0f));
            }
            out
        }
    }
}

fn hex_digit(nibble: u8) -> char {
    match nibble & 0x0f {
        0..=9 => (b'0' + (nibble & 0x0f)) as char,
        10..=15 => (b'a' + ((nibble & 0x0f) - 10)) as char,
        _ => '0',
    }
}

fn push_i64(out: &mut String, value: i64) {
    if value == 0 {
        out.push('0');
        return;
    }
    let (negative, abs) = if value < 0 {
        (true, (-(value + 1)) as u64 + 1)
    } else {
        (false, value as u64)
    };
    if negative {
        out.push('-');
    }
    let mut buf = [0u8; 20];
    let mut used = 0usize;
    let mut remaining = abs;
    while remaining > 0 {
        buf[used] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        used += 1;
    }
    for idx in (0..used).rev() {
        out.push(buf[idx] as char);
    }
}
