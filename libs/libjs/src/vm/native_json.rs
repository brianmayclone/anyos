//! JSON.parse and JSON.stringify.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::value::*;
use super::Vm;

// ═══════════════════════════════════════════════════════════
// JSON.stringify
// ═══════════════════════════════════════════════════════════

pub fn json_stringify(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let val = args.first().cloned().unwrap_or(JsValue::Undefined);
    let indent = args.get(2).map(|v| {
        match v {
            JsValue::Number(n) => {
                let n = *n as usize;
                let mut s = String::new();
                for _ in 0..n.min(10) { s.push(' '); }
                s
            }
            JsValue::String(s) => {
                let mut out = String::new();
                for (i, c) in s.chars().enumerate() {
                    if i >= 10 { break; }
                    out.push(c);
                }
                out
            }
            _ => String::new(),
        }
    }).unwrap_or_default();

    match stringify_value(&val, &indent, 0) {
        Some(s) => JsValue::String(s),
        None => JsValue::Undefined,
    }
}

fn stringify_value(val: &JsValue, indent: &str, depth: usize) -> Option<String> {
    match val {
        JsValue::Undefined | JsValue::Function(_) => None,
        JsValue::Null => Some(String::from("null")),
        JsValue::Bool(true) => Some(String::from("true")),
        JsValue::Bool(false) => Some(String::from("false")),
        JsValue::Number(n) => {
            if n.is_nan() || n.is_infinite() {
                Some(String::from("null"))
            } else {
                Some(format_number(*n))
            }
        }
        JsValue::String(s) => Some(stringify_string(s)),
        JsValue::Array(arr) => {
            let a = arr.borrow();
            if a.elements.is_empty() {
                return Some(String::from("[]"));
            }
            let has_indent = !indent.is_empty();
            let mut out = String::from("[");
            let new_depth = depth + 1;
            for (i, el) in a.elements.iter().enumerate() {
                if i > 0 { out.push(','); }
                if has_indent {
                    out.push('\n');
                    push_indent(&mut out, indent, new_depth);
                }
                match stringify_value(el, indent, new_depth) {
                    Some(s) => out.push_str(&s),
                    None => out.push_str("null"),
                }
            }
            if has_indent {
                out.push('\n');
                push_indent(&mut out, indent, depth);
            }
            out.push(']');
            Some(out)
        }
        JsValue::Object(obj) => {
            let o = obj.borrow();
            let keys = o.keys();
            if keys.is_empty() {
                return Some(String::from("{}"));
            }
            let has_indent = !indent.is_empty();
            let mut out = String::from("{");
            let new_depth = depth + 1;
            let mut first = true;
            for key in &keys {
                if let Some(prop) = o.properties.get(key) {
                    if let Some(val_str) = stringify_value(&prop.value, indent, new_depth) {
                        if !first { out.push(','); }
                        first = false;
                        if has_indent {
                            out.push('\n');
                            push_indent(&mut out, indent, new_depth);
                        }
                        out.push_str(&stringify_string(key));
                        out.push(':');
                        if has_indent { out.push(' '); }
                        out.push_str(&val_str);
                    }
                }
            }
            if has_indent {
                out.push('\n');
                push_indent(&mut out, indent, depth);
            }
            out.push('}');
            Some(out)
        }
    }
}

fn stringify_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u");
                let n = c as u32;
                out.push(hex_char((n >> 12) as u8));
                out.push(hex_char(((n >> 8) & 0xF) as u8));
                out.push(hex_char(((n >> 4) & 0xF) as u8));
                out.push(hex_char((n & 0xF) as u8));
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn push_indent(out: &mut String, indent: &str, depth: usize) {
    for _ in 0..depth {
        out.push_str(indent);
    }
}

fn hex_char(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'a' + n - 10) as char,
    }
}

// ═══════════════════════════════════════════════════════════
// JSON.parse
// ═══════════════════════════════════════════════════════════

pub fn json_parse(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let bytes = s.as_bytes();
    let mut pos = 0;
    skip_ws(bytes, &mut pos);
    let result = parse_value(vm, bytes, &mut pos);
    result.unwrap_or(JsValue::Undefined)
}

fn skip_ws(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }
}

fn parse_value(vm: &mut Vm, bytes: &[u8], pos: &mut usize) -> Option<JsValue> {
    skip_ws(bytes, pos);
    if *pos >= bytes.len() { return None; }

    match bytes[*pos] {
        b'"' => parse_string_val(bytes, pos),
        b'{' => parse_object(vm, bytes, pos),
        b'[' => parse_array(vm, bytes, pos),
        b't' => parse_literal(bytes, pos, b"true", JsValue::Bool(true)),
        b'f' => parse_literal(bytes, pos, b"false", JsValue::Bool(false)),
        b'n' => parse_literal(bytes, pos, b"null", JsValue::Null),
        _ => parse_number(bytes, pos),
    }
}

fn parse_literal(bytes: &[u8], pos: &mut usize, expected: &[u8], val: JsValue) -> Option<JsValue> {
    if *pos + expected.len() <= bytes.len() && &bytes[*pos..*pos + expected.len()] == expected {
        *pos += expected.len();
        Some(val)
    } else {
        None
    }
}

fn parse_string_val(bytes: &[u8], pos: &mut usize) -> Option<JsValue> {
    parse_string_raw(bytes, pos).map(JsValue::String)
}

fn parse_string_raw(bytes: &[u8], pos: &mut usize) -> Option<String> {
    if *pos >= bytes.len() || bytes[*pos] != b'"' { return None; }
    *pos += 1;
    let mut s = String::new();
    while *pos < bytes.len() {
        let b = bytes[*pos];
        if b == b'"' {
            *pos += 1;
            return Some(s);
        }
        if b == b'\\' {
            *pos += 1;
            if *pos >= bytes.len() { return None; }
            match bytes[*pos] {
                b'"' => s.push('"'),
                b'\\' => s.push('\\'),
                b'/' => s.push('/'),
                b'n' => s.push('\n'),
                b'r' => s.push('\r'),
                b't' => s.push('\t'),
                b'b' => s.push('\x08'),
                b'f' => s.push('\x0C'),
                b'u' => {
                    *pos += 1;
                    let mut code: u32 = 0;
                    for _ in 0..4 {
                        if *pos >= bytes.len() { return None; }
                        let d = match bytes[*pos] {
                            b'0'..=b'9' => (bytes[*pos] - b'0') as u32,
                            b'a'..=b'f' => (bytes[*pos] - b'a' + 10) as u32,
                            b'A'..=b'F' => (bytes[*pos] - b'A' + 10) as u32,
                            _ => return None,
                        };
                        code = code * 16 + d;
                        *pos += 1;
                    }
                    if let Some(c) = char::from_u32(code) {
                        s.push(c);
                    }
                    continue; // don't increment pos again
                }
                _ => s.push(bytes[*pos] as char),
            }
        } else {
            // Handle UTF-8 multi-byte sequences
            if b < 0x80 {
                s.push(b as char);
            } else {
                // Read full UTF-8 char
                let start = *pos;
                let width = if b & 0xE0 == 0xC0 { 2 }
                    else if b & 0xF0 == 0xE0 { 3 }
                    else if b & 0xF8 == 0xF0 { 4 }
                    else { 1 };
                *pos += width;
                if let Ok(ch) = core::str::from_utf8(&bytes[start..*pos]) {
                    s.push_str(ch);
                }
                continue;
            }
        }
        *pos += 1;
    }
    None // unterminated string
}

fn parse_number(bytes: &[u8], pos: &mut usize) -> Option<JsValue> {
    let start = *pos;
    if *pos < bytes.len() && bytes[*pos] == b'-' { *pos += 1; }

    let mut has_digits = false;
    while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' {
        *pos += 1;
        has_digits = true;
    }
    if *pos < bytes.len() && bytes[*pos] == b'.' {
        *pos += 1;
        while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' {
            *pos += 1;
            has_digits = true;
        }
    }
    if *pos < bytes.len() && (bytes[*pos] == b'e' || bytes[*pos] == b'E') {
        *pos += 1;
        if *pos < bytes.len() && (bytes[*pos] == b'+' || bytes[*pos] == b'-') { *pos += 1; }
        while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' { *pos += 1; }
    }

    if !has_digits { return None; }

    let s = core::str::from_utf8(&bytes[start..*pos]).ok()?;
    let n = parse_js_float(s);
    Some(JsValue::Number(n))
}

fn parse_object(vm: &mut Vm, bytes: &[u8], pos: &mut usize) -> Option<JsValue> {
    *pos += 1; // skip '{'
    skip_ws(bytes, pos);

    let mut obj = JsObject::new();

    if *pos < bytes.len() && bytes[*pos] == b'}' {
        *pos += 1;
        return Some(JsValue::Object(Rc::new(RefCell::new(obj))));
    }

    loop {
        skip_ws(bytes, pos);
        let key = parse_string_raw(bytes, pos)?;
        skip_ws(bytes, pos);
        if *pos >= bytes.len() || bytes[*pos] != b':' { return None; }
        *pos += 1;
        let value = parse_value(vm, bytes, pos)?;
        obj.set(key, value);

        skip_ws(bytes, pos);
        if *pos >= bytes.len() { return None; }
        if bytes[*pos] == b'}' {
            *pos += 1;
            return Some(JsValue::Object(Rc::new(RefCell::new(obj))));
        }
        if bytes[*pos] == b',' {
            *pos += 1;
        } else {
            return None;
        }
    }
}

fn parse_array(vm: &mut Vm, bytes: &[u8], pos: &mut usize) -> Option<JsValue> {
    *pos += 1; // skip '['
    skip_ws(bytes, pos);

    let mut elements = Vec::new();

    if *pos < bytes.len() && bytes[*pos] == b']' {
        *pos += 1;
        return Some(JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(elements)))));
    }

    loop {
        let value = parse_value(vm, bytes, pos)?;
        elements.push(value);

        skip_ws(bytes, pos);
        if *pos >= bytes.len() { return None; }
        if bytes[*pos] == b']' {
            *pos += 1;
            return Some(JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(elements)))));
        }
        if bytes[*pos] == b',' {
            *pos += 1;
        } else {
            return None;
        }
    }
}
