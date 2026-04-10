//! String.prototype methods.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::Vm;
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// Helper: get `this` as a String
// ═══════════════════════════════════════════════════════════

fn is_regexp_like(vm: &mut Vm, search: &JsValue) -> Option<bool> {
    match search {
        JsValue::Object(obj) => {
            let o = obj.borrow();
            if o.internal_tag.as_deref() == Some(super::native_regexp::REGEXP_TAG) {
                return Some(true);
            }
            drop(o);
            match vm.get_property_invoking_getter(search, super::native_symbol::WELL_KNOWN_MATCH) {
                JsValue::Undefined => Some(false),
                JsValue::Bool(b) => Some(b),
                JsValue::Null => Some(false),
                _ if vm.pending_exception.is_some() => None,
                _ => Some(true),
            }
        }
        _ => Some(false),
    }
}

/// Per ECMA: `String.prototype.{split,replace,replaceAll}` must first call
/// GetMethod on the relevant well-known symbol on their first argument.
/// If a callable is returned, dispatch to it; otherwise the caller falls
/// through to the basic string-based implementation. Errors from the getter
/// or the dispatched call are propagated through `pending_exception`.
pub(crate) fn dispatch_string_well_known(
    vm: &mut Vm,
    args: &[JsValue],
    sym_key: &str,
) -> Option<JsValue> {
    let arg = args.first()?;
    if matches!(arg, JsValue::Null | JsValue::Undefined) {
        return None;
    }
    let arg_clone = arg.clone();
    let method = vm.get_property_invoking_getter(&arg_clone, sym_key);
    if vm.pending_exception.is_some() {
        return Some(JsValue::Undefined);
    }
    if !matches!(method, JsValue::Function(_)) {
        return None;
    }
    let this_str = match this_string_checked(vm) {
        Some(s) => JsValue::String(s),
        None => return Some(JsValue::Undefined),
    };
    // Forward all original args after the search value (e.g. replacement,
    // limit) to the user-defined symbol method, with the search value as
    // `this`.
    let mut call_args: Vec<JsValue> = Vec::with_capacity(args.len());
    call_args.push(this_str);
    for extra in args.iter().skip(1) {
        call_args.push(extra.clone());
    }
    let result = vm.call_value(&method, &call_args, arg_clone);
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
        return Some(JsValue::Undefined);
    }
    if vm.pending_exception.is_some() {
        return Some(JsValue::Undefined);
    }
    Some(result)
}

fn reject_regexp_search(vm: &mut Vm, args: &[JsValue]) -> bool {
    if let Some(search) = args.first() {
        let Some(is_regexp) = is_regexp_like(vm, search) else {
            return true;
        };
        if is_regexp {
            let err = vm.make_type_error("The method doesn't accept regular expressions");
            vm.throw_native(err);
            return true;
        }
    }
    false
}

fn this_string(vm: &Vm) -> String {
    match &vm.current_this {
        JsValue::String(s) => s.clone(),
        JsValue::Object(obj) => {
            let o = obj.borrow();
            match o.primitive_value.as_deref() {
                Some(JsValue::String(s)) => s.clone(),
                _ => vm.current_this.to_js_string(),
            }
        }
        _ => vm.current_this.to_js_string(),
    }
}

/// RequireObjectCoercible + ToString. Throws TypeError for null/undefined.
/// For Objects, calls Symbol.toPrimitive if present, otherwise falls back to basic coercion.
/// Returns None if an exception was thrown.
pub fn this_string_checked(vm: &mut Vm) -> Option<String> {
    let this = vm.current_this.clone();
    match &this {
        JsValue::Null | JsValue::Undefined => {
            let err = vm.make_type_error("Cannot convert undefined or null to object");
            vm.throw_native(err);
            None
        }
        JsValue::String(s) => {
            if super::is_symbol_value(&this) {
                let err = vm.make_type_error("Cannot convert a Symbol value to a string");
                vm.throw_native(err);
                None
            } else {
                Some(s.clone())
            }
        }
        JsValue::Number(_) | JsValue::Bool(_) | JsValue::BigInt(_) => Some(this.to_js_string()),
        JsValue::Object(obj) => {
            if let Some(prim) = obj.borrow().primitive_value.as_deref() {
                if super::is_symbol_value(prim) {
                    let err = vm.make_type_error("Cannot convert a Symbol value to a string");
                    vm.throw_native(err);
                    return None;
                }
                return Some(prim.to_js_string());
            }
            let prim = vm.to_primitive_for_op(this, "string");
            if vm.pending_exception.is_some() {
                return None;
            }
            if super::is_symbol_value(&prim) {
                let err = vm.make_type_error("Cannot convert a Symbol value to a string");
                vm.throw_native(err);
                return None;
            }
            Some(prim.to_js_string())
        }
        JsValue::Array(_) | JsValue::Function(_) => {
            let prim = vm.to_primitive_for_op(this, "string");
            if vm.pending_exception.is_some() {
                return None;
            }
            if super::is_symbol_value(&prim) {
                let err = vm.make_type_error("Cannot convert a Symbol value to a string");
                vm.throw_native(err);
                return None;
            }
            Some(prim.to_js_string())
        }
        _ => Some(this_string(vm)),
    }
}

fn this_string_value(vm: &mut Vm) -> Option<String> {
    let this = vm.current_this.clone();
    match this {
        JsValue::String(s) => Some(s),
        JsValue::Object(obj) => {
            let primitive = {
                let borrowed = obj.borrow();
                borrowed.primitive_value.clone()
            };
            match primitive {
                Some(prim) => match *prim {
                    JsValue::String(s) => Some(s),
                    _ => {
                        let err =
                            vm.make_type_error("String.prototype method called on incompatible receiver");
                        vm.throw_native(err);
                        None
                    }
                },
                _ => {
                    let err =
                        vm.make_type_error("String.prototype method called on incompatible receiver");
                    vm.throw_native(err);
                    None
                }
            }
        }
        _ => {
            let err = vm.make_type_error("String.prototype method called on incompatible receiver");
            vm.throw_native(err);
            None
        }
    }
}

fn arg_to_number_checked(vm: &mut Vm, value: &JsValue) -> Option<f64> {
    let n = crate::vm::native_array::to_number_vm(vm, value);
    if vm.pending_exception.is_some() {
        None
    } else {
        Some(n)
    }
}

fn arg_to_string_checked(vm: &mut Vm, value: &JsValue) -> Option<String> {
    if super::is_symbol_value(value) {
        let err = vm.make_type_error("Cannot convert a Symbol value to a string");
        vm.throw_native(err);
        return None;
    }
    match value {
        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => {
            let prim = vm.to_primitive_for_op(value.clone(), "string");
            if vm.pending_exception.is_some() {
                return None;
            }
            if super::is_symbol_value(&prim) {
                let err = vm.make_type_error("Cannot convert a Symbol value to a string");
                vm.throw_native(err);
                return None;
            }
            Some(prim.to_js_string())
        }
        _ => Some(value.to_js_string()),
    }
}

fn to_integer_or_infinity_for_index(vm: &mut Vm, value: Option<&JsValue>) -> Option<f64> {
    let Some(value) = value else {
        return Some(0.0);
    };
    let n = arg_to_number_checked(vm, value)?;
    if n.is_nan() || n == 0.0 {
        Some(0.0)
    } else if !n.is_finite() {
        Some(n)
    } else {
        Some((n as i64) as f64)
    }
}

fn search_string_arg_or_undefined(vm: &mut Vm, args: &[JsValue]) -> Option<String> {
    match args.first() {
        Some(v) => arg_to_string_checked(vm, v),
        None => Some(String::from("undefined")),
    }
}

/// Collect the string into chars for indexing.
fn chars_vec(s: &str) -> Vec<char> {
    s.chars().collect()
}

/// Resolve a possibly-negative index against a char length.
fn resolve_index(idx: f64, len: usize) -> usize {
    if idx.is_nan() {
        return 0;
    }
    if idx < 0.0 {
        let r = len as f64 + idx;
        if r < 0.0 {
            0
        } else {
            r as usize
        }
    } else {
        (idx as usize).min(len)
    }
}

/// Safely convert an f64 to usize, clamping NaN/negative to 0 and Infinity/huge to `cap`.
#[inline]
fn to_usize_clamped(n: f64, cap: usize) -> usize {
    if n.is_nan() || n < 0.0 {
        return 0;
    }
    if !n.is_finite() || n >= cap as f64 {
        return cap;
    }
    n as usize
}

// ═══════════════════════════════════════════════════════════
// String.prototype methods
// ═══════════════════════════════════════════════════════════

pub fn string_char_at(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let chars = chars_vec(&s);
    let idx = match to_integer_or_infinity_for_index(vm, args.first()) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    if idx.is_nan() || idx < 0.0 || !idx.is_finite() {
        return JsValue::String(String::new());
    }
    let idx = idx as usize;
    if idx < chars.len() {
        let mut buf = String::new();
        buf.push(chars[idx]);
        JsValue::String(buf)
    } else {
        JsValue::String(String::new())
    }
}

pub fn string_char_code_at(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let chars = chars_vec(&s);
    let idx = match to_integer_or_infinity_for_index(vm, args.first()) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    if idx.is_nan() || idx < 0.0 || !idx.is_finite() {
        return JsValue::Number(f64::NAN);
    }
    let idx = idx as usize;
    if idx < chars.len() {
        JsValue::Number(chars[idx] as u32 as f64)
    } else {
        JsValue::Number(f64::NAN)
    }
}

pub fn string_code_point_at(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let chars = chars_vec(&s);
    let idx = match to_integer_or_infinity_for_index(vm, args.first()) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    if idx.is_nan() || idx < 0.0 || !idx.is_finite() {
        return JsValue::Undefined;
    }
    let idx = idx as usize;
    if idx < chars.len() {
        JsValue::Number(chars[idx] as u32 as f64)
    } else {
        JsValue::Undefined
    }
}

pub fn string_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let search = match search_string_arg_or_undefined(vm, args) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let s_chars = chars_vec(&s);
    let from = match to_integer_or_infinity_for_index(vm, args.get(1)) {
        Some(n) => to_usize_clamped(n, s_chars.len()),
        None => return JsValue::Undefined,
    };

    if search.is_empty() {
        return JsValue::Number(from.min(s_chars.len()) as f64);
    }

    let search_chars = chars_vec(&search);
    let s_len = s_chars.len();
    let search_len = search_chars.len();

    if search_len > s_len {
        return JsValue::Number(-1.0);
    }

    for i in from..=s_len.saturating_sub(search_len) {
        if s_chars[i..i + search_len] == search_chars[..] {
            return JsValue::Number(i as f64);
        }
    }
    JsValue::Number(-1.0)
}

pub fn string_last_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let search = match search_string_arg_or_undefined(vm, args) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let s_chars = chars_vec(&s);
    let search_chars = chars_vec(&search);
    let s_len = s_chars.len();
    let search_len = search_chars.len();

    if search_len > s_len {
        return JsValue::Number(-1.0);
    }

    let from = match args.get(1) {
        Some(v) => {
            let n = match arg_to_number_checked(vm, v) {
                Some(n) => n,
                None => return JsValue::Undefined,
            };
            if n.is_nan() {
                s_len
            } else {
                to_usize_clamped((n as i64) as f64, s_len)
            }
        }
        None => s_len,
    };

    let max_start = from.min(s_len.saturating_sub(search_len));
    for i in (0..=max_start).rev() {
        if s_chars[i..i + search_len] == search_chars[..] {
            return JsValue::Number(i as f64);
        }
    }
    JsValue::Number(-1.0)
}

pub fn string_includes(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if reject_regexp_search(vm, args) {
        return JsValue::Undefined;
    }
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let search = match search_string_arg_or_undefined(vm, args) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };

    let s_chars = chars_vec(&s);
    let from = match to_integer_or_infinity_for_index(vm, args.get(1)) {
        Some(n) => to_usize_clamped(n, s_chars.len()),
        None => return JsValue::Undefined,
    };
    let search_chars = chars_vec(&search);
    let s_len = s_chars.len();
    let search_len = search_chars.len();

    if search_len == 0 {
        return JsValue::Bool(true);
    }
    if search_len > s_len {
        return JsValue::Bool(false);
    }

    for i in from..=s_len.saturating_sub(search_len) {
        if s_chars[i..i + search_len] == search_chars[..] {
            return JsValue::Bool(true);
        }
    }
    JsValue::Bool(false)
}

pub fn string_starts_with(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if reject_regexp_search(vm, args) {
        return JsValue::Undefined;
    }
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let search = match search_string_arg_or_undefined(vm, args) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };

    let s_chars = chars_vec(&s);
    let search_chars = chars_vec(&search);
    let pos = match to_integer_or_infinity_for_index(vm, args.get(1)) {
        Some(n) => to_usize_clamped(n, s_chars.len()),
        None => return JsValue::Undefined,
    };

    if search_chars.len() > s_chars.len() || pos > s_chars.len() - search_chars.len() {
        return JsValue::Bool(false);
    }
    JsValue::Bool(s_chars[pos..pos + search_chars.len()] == search_chars[..])
}

pub fn string_ends_with(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if reject_regexp_search(vm, args) {
        return JsValue::Undefined;
    }
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let search = match search_string_arg_or_undefined(vm, args) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let s_chars = chars_vec(&s);
    let search_chars = chars_vec(&search);

    let end_pos = match args.get(1) {
        Some(v) => match arg_to_number_checked(vm, v) {
            Some(n) if n.is_nan() => s_chars.len(),
            Some(n) => to_usize_clamped((n as i64) as f64, s_chars.len()),
            None => return JsValue::Undefined,
        },
        None => s_chars.len(),
    };

    if search_chars.len() > end_pos {
        return JsValue::Bool(false);
    }
    let start = end_pos - search_chars.len();
    JsValue::Bool(s_chars[start..end_pos] == search_chars[..])
}

pub fn string_slice(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let chars = chars_vec(&s);
    let len = chars.len();
    let start = match args.first() {
        Some(v) => match arg_to_number_checked(vm, v) {
            Some(n) => resolve_index(n, len),
            None => return JsValue::Undefined,
        },
        None => 0,
    };
    let end = match args.get(1) {
        Some(v) => match arg_to_number_checked(vm, v) {
            Some(n) => resolve_index(n, len),
            None => return JsValue::Undefined,
        },
        None => len,
    };

    if start >= end {
        return JsValue::String(String::new());
    }
    let result: String = chars[start..end].iter().collect();
    JsValue::String(result)
}

pub fn string_substring(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let chars = chars_vec(&s);
    let len = chars.len();

    let raw_start = match args.first() {
        Some(v) => match arg_to_number_checked(vm, v) {
            Some(n) => n,
            None => return JsValue::Undefined,
        },
        None => 0.0,
    };
    let raw_end = match args.get(1) {
        Some(v) => match arg_to_number_checked(vm, v) {
            Some(n) => n,
            None => return JsValue::Undefined,
        },
        None => len as f64,
    };

    let s1 = if raw_start.is_nan() || raw_start < 0.0 {
        0
    } else {
        (raw_start as usize).min(len)
    };
    let s2 = if raw_end.is_nan() || raw_end < 0.0 {
        0
    } else {
        (raw_end as usize).min(len)
    };

    let (start, end) = if s1 <= s2 { (s1, s2) } else { (s2, s1) };
    let result: String = chars[start..end].iter().collect();
    JsValue::String(result)
}

pub fn string_to_lower_case(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for lc in c.to_lowercase() {
            out.push(lc);
        }
    }
    JsValue::String(out)
}

pub fn string_to_upper_case(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for uc in c.to_uppercase() {
            out.push(uc);
        }
    }
    JsValue::String(out)
}

pub fn string_trim(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    JsValue::String(String::from(s.trim()))
}

pub fn string_trim_start(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    JsValue::String(String::from(s.trim_start()))
}

pub fn string_trim_end(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    JsValue::String(String::from(s.trim_end()))
}

pub fn string_split(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Per ES §22.1.3.21: GetMethod(separator, @@split) — propagates getter exceptions.
    if let Some(r) = dispatch_string_well_known(vm, args, super::native_symbol::WELL_KNOWN_SPLIT) {
        return r;
    }
    // Check if separator is a RegExp
    if let Some(sep) = args.first() {
        if let JsValue::Object(obj) = sep {
            if obj.borrow().internal_tag.as_deref() == Some(super::native_regexp::REGEXP_TAG) {
                let result = super::native_regexp::string_split_regexp(vm, args);
                if !result.is_undefined() {
                    return result;
                }
            }
        }
    }

    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let sep_val = args.first().cloned();
    let limit = match args.get(1) {
        None | Some(JsValue::Undefined) => usize::MAX,
        Some(v) => {
            // ES §22.1.3.21 step 6: Let lim be ToUint32(limit).
            // Must invoke valueOf()/Symbol.toPrimitive and propagate exceptions.
            let n = match arg_to_number_checked(vm, v) {
                Some(n) => n,
                None => return JsValue::Undefined,
            };
            if n.is_nan() || n < 0.0 {
                0
            } else if !n.is_finite() {
                usize::MAX
            } else {
                n as usize
            }
        }
    };

    // limit === 0 → empty array (ES2023 §22.1.3.21 step 11)
    if limit == 0 {
        return JsValue::new_array(Vec::new());
    }

    let parts: Vec<JsValue> = match &sep_val {
        None | Some(JsValue::Undefined) => {
            vec![JsValue::String(s)]
        }
        Some(sep_jv) => {
            let sep_str = match arg_to_string_checked(vm, sep_jv) {
                Some(s) => s,
                None => return JsValue::Undefined,
            };
            if sep_str.is_empty() {
                // Split into individual characters
                s.chars()
                    .take(limit)
                    .map(|c| {
                        let mut buf = String::new();
                        buf.push(c);
                        JsValue::String(buf)
                    })
                    .collect()
            } else {
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
                if result.len() < limit {
                    result.push(JsValue::String(String::from(remaining)));
                }
                result
            }
        }
    };

    JsValue::new_array(parts)
}

pub fn string_replace(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Per ES §22.1.3.17: GetMethod(searchValue, @@replace).
    if let Some(r) = dispatch_string_well_known(vm, args, super::native_symbol::WELL_KNOWN_REPLACE) {
        return r;
    }
    // Check if search is a RegExp
    if let Some(search_val) = args.first() {
        if let JsValue::Object(obj) = search_val {
            if obj.borrow().internal_tag.as_deref() == Some(super::native_regexp::REGEXP_TAG) {
                let result = super::native_regexp::string_replace_regexp(vm, args);
                if !result.is_undefined() {
                    return result;
                }
            }
        }
    }

    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let search = match args.first() {
        Some(v) => match arg_to_string_checked(vm, v) {
            Some(s) => s,
            None => return JsValue::Undefined,
        },
        None => String::new(),
    };
    let replacement = match args.get(1) {
        Some(v) => match arg_to_string_checked(vm, v) {
            Some(s) => s,
            None => return JsValue::Undefined,
        },
        None => String::new(),
    };

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
    // Per ES §22.1.3.18: GetMethod(searchValue, @@replace).
    if let Some(r) = dispatch_string_well_known(vm, args, super::native_symbol::WELL_KNOWN_REPLACE) {
        return r;
    }
    // Check if search is a RegExp (must have global flag)
    if let Some(search_val) = args.first() {
        if let JsValue::Object(obj) = search_val {
            if obj.borrow().internal_tag.as_deref() == Some(super::native_regexp::REGEXP_TAG) {
                let result = super::native_regexp::string_replace_regexp(vm, args);
                if !result.is_undefined() {
                    return result;
                }
            }
        }
    }

    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let search = match args.first() {
        Some(v) => match arg_to_string_checked(vm, v) {
            Some(s) => s,
            None => return JsValue::Undefined,
        },
        None => String::new(),
    };

    // §22.1.3.18 step 5: functionalReplace = IsCallable(replaceValue).
    let replace_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let functional = matches!(replace_arg, JsValue::Function(_));
    let static_replacement = if functional {
        String::new()
    } else {
        match arg_to_string_checked(vm, &replace_arg) {
            Some(s) => s,
            None => return JsValue::Undefined,
        }
    };

    // Helper: produce the replacement string for a match at byte index `pos`.
    let make_replacement = |vm: &mut Vm, pos: usize| -> Option<String> {
        if !functional {
            return Some(static_replacement.clone());
        }
        let call_args = [
            JsValue::String(search.clone()),
            JsValue::Number(pos as f64),
            JsValue::String(s.clone()),
        ];
        let result = vm.call_value(&replace_arg, &call_args, JsValue::Undefined);
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
            return None;
        }
        if vm.pending_exception.is_some() {
            return None;
        }
        arg_to_string_checked(vm, &result)
    };

    if search.is_empty() {
        // Insert replacement between every character (and at start/end).
        let mut result = String::new();
        let first = match make_replacement(vm, 0) {
            Some(r) => r,
            None => return JsValue::Undefined,
        };
        result.push_str(&first);
        let mut byte_pos = 0usize;
        for c in s.chars() {
            byte_pos += c.len_utf8();
            result.push(c);
            let r = match make_replacement(vm, byte_pos) {
                Some(r) => r,
                None => return JsValue::Undefined,
            };
            result.push_str(&r);
        }
        return JsValue::String(result);
    }

    let mut result = String::new();
    let mut remaining_offset = 0usize;
    loop {
        let remaining = &s[remaining_offset..];
        if let Some(idx) = remaining.find(search.as_str()) {
            result.push_str(&remaining[..idx]);
            let match_pos = remaining_offset + idx;
            let r = match make_replacement(vm, match_pos) {
                Some(r) => r,
                None => return JsValue::Undefined,
            };
            result.push_str(&r);
            remaining_offset = match_pos + search.len();
        } else {
            result.push_str(remaining);
            break;
        }
    }
    JsValue::String(result)
}

pub fn string_repeat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let n = args.first().map(|v| v.to_number()).unwrap_or(0.0);
    // §22.1.3.16: throw RangeError if count is negative or Infinity
    if n < 0.0 || !n.is_finite() {
        let err = vm.make_range_error("Invalid count value");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let count = n as usize;
    if s.is_empty() || count == 0 {
        return JsValue::String(String::new());
    }
    // Limit allocation to prevent OOM (16 MiB)
    let total = s.len().saturating_mul(count);
    if total > 16 * 1024 * 1024 {
        let err = vm.make_range_error("Invalid count value");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    JsValue::String(s.repeat(count))
}

pub fn string_pad_start(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let target_len = match args.first() {
        Some(v) => match arg_to_number_checked(vm, v) {
            Some(n) => to_usize_clamped(n, 1 << 20),
            None => return JsValue::Undefined,
        },
        None => 0,
    };
    let pad_str = match args.get(1) {
        Some(v) => match arg_to_string_checked(vm, v) {
            Some(s) => s,
            None => return JsValue::Undefined,
        },
        None => String::from(" "),
    };

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
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let target_len = match args.first() {
        Some(v) => match arg_to_number_checked(vm, v) {
            Some(n) => to_usize_clamped(n, 1 << 20),
            None => return JsValue::Undefined,
        },
        None => 0,
    };
    let pad_str = match args.get(1) {
        Some(v) => match arg_to_string_checked(vm, v) {
            Some(s) => s,
            None => return JsValue::Undefined,
        },
        None => String::from(" "),
    };

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
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let chars = chars_vec(&s);
    let idx = match args.first() {
        Some(v) => match arg_to_number_checked(vm, v) {
            Some(n) => n as i64,
            None => return JsValue::Undefined,
        },
        None => 0,
    };
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
    let mut s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    for arg in args {
        let Some(arg_s) = arg_to_string_checked(vm, arg) else {
            return JsValue::Undefined;
        };
        s.push_str(&arg_s);
    }
    JsValue::String(s)
}

pub fn string_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let s = match this_string_value(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    JsValue::String(s)
}

/// Convert internal symbol representation to "Symbol(description)" display format.
pub(crate) fn symbol_display_name(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("__symbol__") {
        if let Some(idx) = rest.find('_') {
            let desc = &rest[idx + 1..];
            if desc.is_empty() {
                return String::from("Symbol()");
            }
            return alloc::format!("Symbol({})", desc);
        }
    }
    if let Some(rest) = s.strip_prefix("__symbol_global__") {
        return alloc::format!("Symbol({})", rest);
    }
    String::from("Symbol()")
}

// ═══════════════════════════════════════════════════════════
// String static methods
// ═══════════════════════════════════════════════════════════

/// `String.fromCharCode(...codes)` — create string from UTF-16 code units.
pub fn string_from_char_code(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut result = String::new();
    for arg in args {
        let code = arg.to_number() as u32;
        if let Some(ch) = char::from_u32(code) {
            result.push(ch);
        }
    }
    JsValue::String(result)
}

/// `String.fromCodePoint(...codePoints)` — create string from Unicode code points.
pub fn string_from_code_point(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut result = String::new();
    for arg in args {
        let cp = arg.to_number() as u32;
        if let Some(ch) = char::from_u32(cp) {
            result.push(ch);
        } else {
            let err = vm.make_type_error("Invalid code point");
            vm.throw_native(err);
            return JsValue::Undefined;
        }
    }
    JsValue::String(result)
}

/// `String.raw(template, ...substitutions)` — raw template literal tag.
pub fn string_raw(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let template = args.first().cloned().unwrap_or(JsValue::Undefined);
    let raw = match &template {
        JsValue::Object(obj) => obj.borrow().get("raw"),
        _ => JsValue::Undefined,
    };
    let raw_arr = match &raw {
        JsValue::Array(arr) => arr.borrow().to_dense_vec(),
        _ => return JsValue::String(String::new()),
    };

    let mut result = String::new();
    for (i, part) in raw_arr.iter().enumerate() {
        result.push_str(&part.to_js_string());
        if i + 1 < raw_arr.len() {
            if let Some(sub) = args.get(i + 1) {
                result.push_str(&sub.to_js_string());
            }
        }
    }
    JsValue::String(result)
}

/// `String.prototype.normalize([form])` — ES2023 §22.1.3.14.
///
/// We do not bundle the Unicode normalization tables, so for the four valid
/// `form` values we return the string unchanged (which is correct for any
/// input that contains no characters affected by normalization, i.e. pure
/// ASCII and most precomposed text). Invalid forms throw a `RangeError` per
/// the spec.
pub fn string_normalize(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let form = match args.first() {
        Some(JsValue::Undefined) | None => String::from("NFC"),
        Some(v) => match arg_to_string_checked(vm, v) {
            Some(f) => f,
            None => return JsValue::Undefined,
        },
    };
    if !matches!(form.as_str(), "NFC" | "NFD" | "NFKC" | "NFKD") {
        let err = vm.make_range_error("Invalid normalization form");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    JsValue::String(s)
}

/// `String.prototype.localeCompare(that [, locales [, options]])` — ES2023 §22.1.3.10.
///
/// Uses UCA Level-1 (primary-weight) comparison via [`super::uca`]. The
/// embedded Latin/Latin-1/Latin Extended-A table makes western text sort
/// correctly even without an installed Unicode data file; if
/// `/System/share/collation/uca.bin` is present at runtime the full BMP is
/// covered. The optional `locales` and `options` arguments are accepted but
/// not used (no locale tailorings).
pub fn string_locale_compare(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = match this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    let that = match args.first() {
        Some(v) => match arg_to_string_checked(vm, v) {
            Some(s) => s,
            None => return JsValue::Undefined,
        },
        None => String::from("undefined"),
    };
    let cmp = super::uca::compare_primary(&s, &that);
    JsValue::Number(match cmp {
        core::cmp::Ordering::Less => -1.0,
        core::cmp::Ordering::Equal => 0.0,
        core::cmp::Ordering::Greater => 1.0,
    })
}
