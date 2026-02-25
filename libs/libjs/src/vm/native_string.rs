//! String.prototype methods.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::value::*;
use super::Vm;

// ═══════════════════════════════════════════════════════════
// Helper: get `this` as a String
// ═══════════════════════════════════════════════════════════

fn this_string(vm: &Vm) -> String {
    vm.current_this.to_js_string()
}

/// Collect the string into chars for indexing.
fn chars_vec(s: &str) -> Vec<char> {
    s.chars().collect()
}

/// Resolve a possibly-negative index against a char length.
fn resolve_index(idx: f64, len: usize) -> usize {
    if idx.is_nan() { return 0; }
    if idx < 0.0 {
        let r = len as f64 + idx;
        if r < 0.0 { 0 } else { r as usize }
    } else {
        (idx as usize).min(len)
    }
}

// ═══════════════════════════════════════════════════════════
// String.prototype methods
// ═══════════════════════════════════════════════════════════

pub fn string_char_at(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let chars = chars_vec(&s);
    let idx = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    if idx < chars.len() {
        let mut buf = String::new();
        buf.push(chars[idx]);
        JsValue::String(buf)
    } else {
        JsValue::String(String::new())
    }
}

pub fn string_char_code_at(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let chars = chars_vec(&s);
    let idx = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    if idx < chars.len() {
        JsValue::Number(chars[idx] as u32 as f64)
    } else {
        JsValue::Number(f64::NAN)
    }
}

pub fn string_code_point_at(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let chars = chars_vec(&s);
    let idx = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    if idx < chars.len() {
        JsValue::Number(chars[idx] as u32 as f64)
    } else {
        JsValue::Undefined
    }
}

pub fn string_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let search = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let from = args.get(1).map(|v| v.to_number() as usize).unwrap_or(0);

    if search.is_empty() {
        return JsValue::Number(from.min(s.chars().count()) as f64);
    }

    let s_chars = chars_vec(&s);
    let search_chars = chars_vec(&search);
    let s_len = s_chars.len();
    let search_len = search_chars.len();

    if search_len > s_len { return JsValue::Number(-1.0); }

    for i in from..=s_len.saturating_sub(search_len) {
        if s_chars[i..i + search_len] == search_chars[..] {
            return JsValue::Number(i as f64);
        }
    }
    JsValue::Number(-1.0)
}

pub fn string_last_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let search = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let s_chars = chars_vec(&s);
    let search_chars = chars_vec(&search);
    let s_len = s_chars.len();
    let search_len = search_chars.len();

    if search_len > s_len { return JsValue::Number(-1.0); }

    let from = args.get(1).map(|v| {
        let n = v.to_number();
        if n.is_nan() { s_len } else { (n as usize).min(s_len) }
    }).unwrap_or(s_len);

    let max_start = from.min(s_len.saturating_sub(search_len));
    for i in (0..=max_start).rev() {
        if s_chars[i..i + search_len] == search_chars[..] {
            return JsValue::Number(i as f64);
        }
    }
    JsValue::Number(-1.0)
}

pub fn string_includes(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let search = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let from = args.get(1).map(|v| v.to_number() as usize).unwrap_or(0);

    let s_chars = chars_vec(&s);
    let search_chars = chars_vec(&search);
    let s_len = s_chars.len();
    let search_len = search_chars.len();

    if search_len == 0 { return JsValue::Bool(true); }
    if search_len > s_len { return JsValue::Bool(false); }

    for i in from..=s_len.saturating_sub(search_len) {
        if s_chars[i..i + search_len] == search_chars[..] {
            return JsValue::Bool(true);
        }
    }
    JsValue::Bool(false)
}

pub fn string_starts_with(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let search = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let pos = args.get(1).map(|v| v.to_number() as usize).unwrap_or(0);

    let s_chars = chars_vec(&s);
    let search_chars = chars_vec(&search);

    if pos + search_chars.len() > s_chars.len() {
        return JsValue::Bool(false);
    }
    JsValue::Bool(s_chars[pos..pos + search_chars.len()] == search_chars[..])
}

pub fn string_ends_with(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let search = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let s_chars = chars_vec(&s);
    let search_chars = chars_vec(&search);

    let end_pos = args.get(1).map(|v| (v.to_number() as usize).min(s_chars.len())).unwrap_or(s_chars.len());

    if search_chars.len() > end_pos {
        return JsValue::Bool(false);
    }
    let start = end_pos - search_chars.len();
    JsValue::Bool(s_chars[start..end_pos] == search_chars[..])
}

pub fn string_slice(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let chars = chars_vec(&s);
    let len = chars.len();
    let start = resolve_index(args.first().map(|v| v.to_number()).unwrap_or(0.0), len);
    let end = resolve_index(args.get(1).map(|v| v.to_number()).unwrap_or(len as f64), len);

    if start >= end {
        return JsValue::String(String::new());
    }
    let result: String = chars[start..end].iter().collect();
    JsValue::String(result)
}

pub fn string_substring(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let chars = chars_vec(&s);
    let len = chars.len();

    let raw_start = args.first().map(|v| v.to_number()).unwrap_or(0.0);
    let raw_end = args.get(1).map(|v| v.to_number()).unwrap_or(len as f64);

    let s1 = if raw_start.is_nan() || raw_start < 0.0 { 0 } else { (raw_start as usize).min(len) };
    let s2 = if raw_end.is_nan() || raw_end < 0.0 { 0 } else { (raw_end as usize).min(len) };

    let (start, end) = if s1 <= s2 { (s1, s2) } else { (s2, s1) };
    let result: String = chars[start..end].iter().collect();
    JsValue::String(result)
}

pub fn string_to_lower_case(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for lc in c.to_lowercase() {
            out.push(lc);
        }
    }
    JsValue::String(out)
}

pub fn string_to_upper_case(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for uc in c.to_uppercase() {
            out.push(uc);
        }
    }
    JsValue::String(out)
}

pub fn string_trim(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from(this_string(vm).trim()))
}

pub fn string_trim_start(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from(this_string(vm).trim_start()))
}

pub fn string_trim_end(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from(this_string(vm).trim_end()))
}

pub fn string_split(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let sep = args.first().map(|v| v.to_js_string());
    let limit = args.get(1).map(|v| v.to_number() as usize).unwrap_or(usize::MAX);

    let parts: Vec<JsValue> = match sep {
        None => {
            vec![JsValue::String(s)]
        }
        Some(ref sep_val) if sep_val == "undefined" => {
            vec![JsValue::String(s)]
        }
        Some(ref sep_str) if sep_str.is_empty() => {
            // Split into individual characters
            s.chars().take(limit).map(|c| {
                let mut buf = String::new();
                buf.push(c);
                JsValue::String(buf)
            }).collect()
        }
        Some(ref sep_str) => {
            let mut result = Vec::new();
            let mut remaining = s.as_str();
            let mut count = 0;
            while count + 1 < limit {
                if let Some(idx) = remaining.find(sep_str.as_str()) {
                    result.push(JsValue::String(String::from(&remaining[..idx])));
                    remaining = &remaining[idx + sep_str.len()..];
                    count += 1;
                } else {
                    break;
                }
            }
            result.push(JsValue::String(String::from(remaining)));
            result
        }
    };

    JsValue::new_array(parts)
}

pub fn string_replace(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let search = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let replacement = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();

    // Replace first occurrence only
    if let Some(idx) = s.find(&search) {
        let mut result = String::with_capacity(s.len());
        result.push_str(&s[..idx]);
        result.push_str(&replacement);
        result.push_str(&s[idx + search.len()..]);
        JsValue::String(result)
    } else {
        JsValue::String(s)
    }
}

pub fn string_replace_all(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let search = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let replacement = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();

    if search.is_empty() {
        // Insert replacement between every character (and at start/end)
        let mut result = String::new();
        result.push_str(&replacement);
        for c in s.chars() {
            result.push(c);
            result.push_str(&replacement);
        }
        return JsValue::String(result);
    }

    let mut result = String::new();
    let mut remaining = s.as_str();
    loop {
        if let Some(idx) = remaining.find(search.as_str()) {
            result.push_str(&remaining[..idx]);
            result.push_str(&replacement);
            remaining = &remaining[idx + search.len()..];
        } else {
            result.push_str(remaining);
            break;
        }
    }
    JsValue::String(result)
}

pub fn string_repeat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let count = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let mut result = String::with_capacity(s.len() * count);
    for _ in 0..count {
        result.push_str(&s);
    }
    JsValue::String(result)
}

pub fn string_pad_start(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let target_len = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let pad_str = args.get(1).map(|v| v.to_js_string()).unwrap_or_else(|| String::from(" "));

    let chars = chars_vec(&s);
    if chars.len() >= target_len || pad_str.is_empty() {
        return JsValue::String(s);
    }

    let pad_chars = chars_vec(&pad_str);
    let needed = target_len - chars.len();
    let mut result = String::new();
    let mut i = 0;
    while result.chars().count() < needed {
        result.push(pad_chars[i % pad_chars.len()]);
        i += 1;
    }
    result.push_str(&s);
    JsValue::String(result)
}

pub fn string_pad_end(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let target_len = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let pad_str = args.get(1).map(|v| v.to_js_string()).unwrap_or_else(|| String::from(" "));

    let chars = chars_vec(&s);
    if chars.len() >= target_len || pad_str.is_empty() {
        return JsValue::String(s);
    }

    let pad_chars = chars_vec(&pad_str);
    let needed = target_len - chars.len();
    let mut result = s;
    let mut i = 0;
    while i < needed {
        result.push(pad_chars[i % pad_chars.len()]);
        i += 1;
    }
    JsValue::String(result)
}

pub fn string_at(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = this_string(vm);
    let chars = chars_vec(&s);
    let idx = args.first().map(|v| v.to_number() as i64).unwrap_or(0);
    let len = chars.len() as i64;
    let actual = if idx < 0 { len + idx } else { idx };
    if actual >= 0 && actual < len {
        let mut buf = String::new();
        buf.push(chars[actual as usize]);
        JsValue::String(buf)
    } else {
        JsValue::Undefined
    }
}

pub fn string_concat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut s = this_string(vm);
    for arg in args {
        s.push_str(&arg.to_js_string());
    }
    JsValue::String(s)
}

pub fn string_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(this_string(vm))
}
