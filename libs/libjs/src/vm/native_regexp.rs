// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! RegExp built-in object and prototype methods.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::Vm;
use crate::regexp::Regex;
use crate::value::*;

/// Internal tag for RegExp objects.
pub const REGEXP_TAG: &str = "__regexp__";

/// Key used to store the compiled Regex in a RegExp object's internal slot.
/// We serialize into the internal_tag and store the Regex pointer as a hidden property.
const REGEX_KEY: &str = "__regex_ptr__";
const REGEX_SRC_KEY: &str = "source";
const REGEX_FLAGS_KEY: &str = "flags";
const LAST_INDEX_KEY: &str = "lastIndex";

/// Create a RegExp object from pattern and flags strings.
pub fn create_regexp_object(vm: &Vm, pattern: &str, flags: &str) -> JsValue {
    let regex = match Regex::new(pattern, flags) {
        Ok(r) => r,
        Err(_) => {
            // Return object with empty regex on parse error
            match Regex::new("(?:)", "") {
                Ok(r) => r,
                Err(_) => return JsValue::Undefined,
            }
        }
    };

    let mut obj = JsObject::with_tag(REGEXP_TAG);
    obj.prototype = Some(vm.regexp_proto.clone());
    obj.set(
        String::from(REGEX_SRC_KEY),
        JsValue::String(String::from(pattern)),
    );
    obj.set(
        String::from(REGEX_FLAGS_KEY),
        JsValue::String(String::from(flags)),
    );
    obj.set(String::from(LAST_INDEX_KEY), JsValue::Number(0.0));

    // Store flags as individual boolean properties (non-enumerable)
    obj.set_hidden(String::from("global"), JsValue::Bool(regex.flags.global));
    obj.set_hidden(
        String::from("ignoreCase"),
        JsValue::Bool(regex.flags.ignore_case),
    );
    obj.set_hidden(
        String::from("multiline"),
        JsValue::Bool(regex.flags.multiline),
    );
    obj.set_hidden(String::from("dotAll"), JsValue::Bool(regex.flags.dot_all));
    obj.set_hidden(String::from("unicode"), JsValue::Bool(regex.flags.unicode));
    obj.set_hidden(String::from("sticky"), JsValue::Bool(regex.flags.sticky));

    // Store the compiled regex as a serialized hidden property
    // We store the pattern+flags and recompile on use (simple but correct for no_std)
    obj.set_hidden(String::from(REGEX_KEY), JsValue::Bool(true));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// Extract compiled Regex from a RegExp JsValue. Recompiles from source+flags each time.
pub fn get_regex(val: &JsValue) -> Option<Regex> {
    if let JsValue::Object(obj) = val {
        let o = obj.borrow();
        if o.internal_tag.as_deref() != Some(REGEXP_TAG) {
            return None;
        }
        let source = match o.get(REGEX_SRC_KEY) {
            JsValue::String(s) => s,
            _ => return None,
        };
        let flags = match o.get(REGEX_FLAGS_KEY) {
            JsValue::String(s) => s,
            _ => String::new(),
        };
        Regex::new(&source, &flags).ok()
    } else {
        None
    }
}

fn get_last_index(val: &JsValue) -> usize {
    if let JsValue::Object(obj) = val {
        let o = obj.borrow();
        match o.get(LAST_INDEX_KEY) {
            JsValue::Number(n) if n >= 0.0 => n as usize,
            _ => 0,
        }
    } else {
        0
    }
}

fn set_last_index(val: &JsValue, idx: usize) {
    if let JsValue::Object(obj) = val {
        obj.borrow_mut()
            .set(String::from(LAST_INDEX_KEY), JsValue::Number(idx as f64));
    }
}

fn is_global_or_sticky(val: &JsValue) -> bool {
    if let JsValue::Object(obj) = val {
        let o = obj.borrow();
        let g = matches!(o.get("global"), JsValue::Bool(true));
        let y = matches!(o.get("sticky"), JsValue::Bool(true));
        g || y
    } else {
        false
    }
}

/// Build a match result array (like `exec` returns).
fn build_exec_result(_vm: &Vm, m: &crate::regexp::Match, input: &str) -> JsValue {
    let mut elems = Vec::new();
    // Index 0 = full match
    elems.push(JsValue::String(String::from(m.full_match(input))));
    // Groups 1..n
    for i in 1..m.groups.len() {
        if let Some(s) = m.group(i, input) {
            elems.push(JsValue::String(String::from(s)));
        } else {
            elems.push(JsValue::Undefined);
        }
    }
    let arr = JsArray::from_vec(elems);
    let arr_val = JsValue::Array(Rc::new(RefCell::new(arr)));
    arr_val.set_property(String::from("index"), JsValue::Number(m.start as f64));
    arr_val.set_property(String::from("input"), JsValue::String(String::from(input)));

    // ES2018: named groups as .groups property
    if !m.named_groups.is_empty() {
        let groups_obj = JsValue::new_object();
        for (name, idx) in &m.named_groups {
            let val = m
                .group(*idx as usize, input)
                .map(|s| JsValue::String(String::from(s)))
                .unwrap_or(JsValue::Undefined);
            groups_obj.set_property(name.clone(), val);
        }
        arr_val.set_property(String::from("groups"), groups_obj);
    } else {
        arr_val.set_property(String::from("groups"), JsValue::Undefined);
    }

    arr_val
}

// ── RegExp.prototype methods ──

/// `RegExp.prototype.test(string)`
pub fn regexp_test(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let input = args.first().map(|a| a.to_js_string()).unwrap_or_default();
    let regex = match get_regex(&this) {
        Some(r) => r,
        None => return JsValue::Bool(false),
    };

    let start = if is_global_or_sticky(&this) {
        get_last_index(&this)
    } else {
        0
    };
    let result = regex.test(&input, start);
    if is_global_or_sticky(&this) {
        if result {
            if let Some(m) = regex.exec(&input, start) {
                set_last_index(&this, m.end);
            }
        } else {
            set_last_index(&this, 0);
        }
    }
    JsValue::Bool(result)
}

/// `RegExp.prototype.exec(string)`
pub fn regexp_exec(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let input = args.first().map(|a| a.to_js_string()).unwrap_or_default();
    let regex = match get_regex(&this) {
        Some(r) => r,
        None => return JsValue::Null,
    };

    let start = if is_global_or_sticky(&this) {
        get_last_index(&this)
    } else {
        0
    };
    match regex.exec(&input, start) {
        Some(m) => {
            if is_global_or_sticky(&this) {
                set_last_index(&this, m.end);
            }
            build_exec_result(vm, &m, &input)
        }
        None => {
            if is_global_or_sticky(&this) {
                set_last_index(&this, 0);
            }
            JsValue::Null
        }
    }
}

/// `RegExp.prototype.toString()`
pub fn regexp_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if let JsValue::Object(obj) = &this {
        let o = obj.borrow();
        let source = match o.get(REGEX_SRC_KEY) {
            JsValue::String(s) => s,
            _ => String::from("(?:)"),
        };
        let flags = match o.get(REGEX_FLAGS_KEY) {
            JsValue::String(s) => s,
            _ => String::new(),
        };
        JsValue::String(format!("/{}/{}", source, flags))
    } else {
        JsValue::String(String::from("/(?:)/"))
    }
}

// ── RegExp constructor ──

/// `RegExp(pattern, flags)` / `new RegExp(pattern, flags)`
pub fn regexp_constructor(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let pattern = match args.first() {
        Some(JsValue::String(s)) => s.clone(),
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            if o.internal_tag.as_deref() == Some(REGEXP_TAG) {
                match o.get(REGEX_SRC_KEY) {
                    JsValue::String(s) => s,
                    _ => String::new(),
                }
            } else {
                args.first().map(|a| a.to_js_string()).unwrap_or_default()
            }
        }
        Some(v) => v.to_js_string(),
        None => String::new(),
    };
    let flags = match args.get(1) {
        Some(JsValue::String(s)) => s.clone(),
        Some(JsValue::Undefined) | None => {
            // If first arg is a RegExp and no flags given, use its flags
            if let Some(JsValue::Object(obj)) = args.first() {
                let o = obj.borrow();
                if o.internal_tag.as_deref() == Some(REGEXP_TAG) {
                    match o.get(REGEX_FLAGS_KEY) {
                        JsValue::String(s) => s,
                        _ => String::new(),
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        }
        Some(v) => v.to_js_string(),
    };
    create_regexp_object(vm, &pattern, &flags)
}

// ── String methods that use RegExp ──

/// Per ECMA §22.1.3.{11,15,17,21}: the String.prototype.{match,replace,search,split,matchAll}
/// methods must first call GetMethod(arg, well-known symbol). If a callable
/// is returned, dispatch to it. Otherwise fall through to the regex-based path.
///
/// Returns Some(result) if dispatched (caller should return immediately).
/// Returns None if no custom method was found and the caller should continue.
fn dispatch_well_known(vm: &mut Vm, args: &[JsValue], sym_key: &str) -> Option<JsValue> {
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
    // ToString(this) — propagate exceptions from the receiver coercion.
    let this_str = match super::native_string::this_string_checked(vm) {
        Some(s) => JsValue::String(s),
        None => return Some(JsValue::Undefined),
    };
    let result = vm.call_value(&method, &[this_str], arg_clone);
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
        return Some(JsValue::Undefined);
    }
    if vm.pending_exception.is_some() {
        return Some(JsValue::Undefined);
    }
    Some(result)
}

/// Implements `String.prototype.match(regexp)`
pub fn string_match(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(r) = dispatch_well_known(vm, args, super::native_symbol::WELL_KNOWN_MATCH) {
        return r;
    }
    let this_str = vm.current_this.to_js_string();
    let arg = args.first().cloned().unwrap_or(JsValue::Undefined);

    let (regex, _regexp_val) = match &arg {
        JsValue::Object(obj) if obj.borrow().internal_tag.as_deref() == Some(REGEXP_TAG) => {
            match get_regex(&arg) {
                Some(r) => (r, arg.clone()),
                None => return JsValue::Null,
            }
        }
        JsValue::String(s) => match Regex::new(s, "") {
            Ok(r) => {
                let rv = create_regexp_object(vm, s, "");
                (r, rv)
            }
            Err(_) => return JsValue::Null,
        },
        _ => {
            let s = arg.to_js_string();
            match Regex::new(&s, "") {
                Ok(r) => {
                    let rv = create_regexp_object(vm, &s, "");
                    (r, rv)
                }
                Err(_) => return JsValue::Null,
            }
        }
    };

    if regex.flags.global {
        // Global: return array of all matches
        let matches = regex.exec_all(&this_str);
        if matches.is_empty() {
            return JsValue::Null;
        }
        let elements: Vec<JsValue> = matches
            .iter()
            .map(|m| JsValue::String(String::from(m.full_match(&this_str))))
            .collect();
        JsValue::new_array(elements)
    } else {
        // Non-global: return first exec result
        match regex.exec(&this_str, 0) {
            Some(m) => build_exec_result(vm, &m, &this_str),
            None => JsValue::Null,
        }
    }
}

/// Implements `String.prototype.search(regexp)`
pub fn string_search(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(r) = dispatch_well_known(vm, args, super::native_symbol::WELL_KNOWN_SEARCH) {
        return r;
    }
    let this_str = vm.current_this.to_js_string();
    let arg = args.first().cloned().unwrap_or(JsValue::Undefined);

    let regex = match &arg {
        JsValue::Object(obj) if obj.borrow().internal_tag.as_deref() == Some(REGEXP_TAG) => {
            get_regex(&arg)
        }
        _ => {
            let s = arg.to_js_string();
            Regex::new(&s, "").ok()
        }
    };

    match regex {
        Some(r) => match r.exec(&this_str, 0) {
            Some(m) => JsValue::Number(m.start as f64),
            None => JsValue::Number(-1.0),
        },
        None => JsValue::Number(-1.0),
    }
}

/// Implements `String.prototype.replace(searchValue, replaceValue)` with RegExp support.
pub fn string_replace_regexp(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_str = vm.current_this.to_js_string();
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    let replace_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);

    // Check if search is a RegExp
    let is_regexp = if let JsValue::Object(obj) = &search {
        obj.borrow().internal_tag.as_deref() == Some(REGEXP_TAG)
    } else {
        false
    };

    if !is_regexp {
        // Delegate to plain string replace (original behavior)
        return JsValue::Undefined; // Sentinel: caller handles non-regexp case
    }

    let regex = match get_regex(&search) {
        Some(r) => r,
        None => return JsValue::String(this_str),
    };

    let replace_str = replace_arg.to_js_string();

    if regex.flags.global {
        let matches = regex.exec_all(&this_str);
        if matches.is_empty() {
            return JsValue::String(this_str);
        }
        let mut result = String::new();
        let chars: Vec<char> = this_str.chars().collect();
        let mut last_end = 0usize;
        for m in &matches {
            // Append text before match
            for i in last_end..m.start {
                result.push(chars[i]);
            }
            // Append replacement (with $-substitutions)
            apply_replacement(&mut result, &replace_str, m, &this_str);
            last_end = m.end;
        }
        // Append remainder
        for i in last_end..chars.len() {
            result.push(chars[i]);
        }
        JsValue::String(result)
    } else {
        match regex.exec(&this_str, 0) {
            Some(m) => {
                let chars: Vec<char> = this_str.chars().collect();
                let mut result = String::new();
                for i in 0..m.start {
                    result.push(chars[i]);
                }
                apply_replacement(&mut result, &replace_str, &m, &this_str);
                for i in m.end..chars.len() {
                    result.push(chars[i]);
                }
                JsValue::String(result)
            }
            None => JsValue::String(this_str),
        }
    }
}

/// Implements `String.prototype.split(separator)` with RegExp support.
pub fn string_split_regexp(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_str = vm.current_this.to_js_string();
    let sep = args.first().cloned().unwrap_or(JsValue::Undefined);
    let limit = args
        .get(1)
        .map(|v| v.to_number() as usize)
        .unwrap_or(usize::MAX);

    let is_regexp = if let JsValue::Object(obj) = &sep {
        obj.borrow().internal_tag.as_deref() == Some(REGEXP_TAG)
    } else {
        false
    };

    if !is_regexp {
        return JsValue::Undefined; // Sentinel: caller handles non-regexp case
    }

    let regex = match get_regex(&sep) {
        Some(r) => r,
        None => return JsValue::new_array(vec![JsValue::String(this_str)]),
    };

    let chars: Vec<char> = this_str.chars().collect();
    let len = chars.len();
    let mut result = Vec::new();
    let mut last_end = 0usize;
    let mut pos = 0usize;

    while pos <= len && result.len() < limit {
        match regex.exec(&this_str, pos) {
            Some(m) if m.end != last_end || (m.start != pos && m.end > m.start) => {
                // Collect substring before match
                let part: String = chars[last_end..m.start].iter().collect();
                result.push(JsValue::String(part));
                if result.len() >= limit {
                    break;
                }

                // Collect capturing groups
                for i in 1..m.groups.len() {
                    if result.len() >= limit {
                        break;
                    }
                    match m.group(i, &this_str) {
                        Some(s) => result.push(JsValue::String(String::from(s))),
                        None => result.push(JsValue::Undefined),
                    }
                }

                last_end = m.end;
                pos = if m.end == m.start { m.end + 1 } else { m.end };
            }
            _ => {
                pos += 1;
            }
        }
    }

    if result.len() < limit {
        let remainder: String = chars[last_end..].iter().collect();
        result.push(JsValue::String(remainder));
    }

    JsValue::new_array(result)
}

/// Apply $-substitution patterns in replacement string.
fn apply_replacement(out: &mut String, replace: &str, m: &crate::regexp::Match, input: &str) {
    let chars: Vec<char> = replace.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '$' && i + 1 < len {
            match chars[i + 1] {
                '$' => {
                    out.push('$');
                    i += 2;
                }
                '&' => {
                    out.push_str(m.full_match(input));
                    i += 2;
                }
                '`' => {
                    // Text before match
                    let before = &input[..crate::regexp::char_to_byte(input, m.start)];
                    out.push_str(before);
                    i += 2;
                }
                '\'' => {
                    // Text after match
                    let after = &input[crate::regexp::char_to_byte(input, m.end)..];
                    out.push_str(after);
                    i += 2;
                }
                d if d >= '1' && d <= '9' => {
                    // $1..$9 or $10..$99
                    let mut n = (d as u32 - '0' as u32) as usize;
                    i += 2;
                    if i < len && chars[i] >= '0' && chars[i] <= '9' {
                        let n2 = n * 10 + (chars[i] as u32 - '0' as u32) as usize;
                        if n2 <= m.groups.len().saturating_sub(1) {
                            n = n2;
                            i += 1;
                        }
                    }
                    if let Some(s) = m.group(n, input) {
                        out.push_str(s);
                    }
                }
                _ => {
                    out.push('$');
                    i += 1;
                }
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
}

/// `String.prototype.matchAll(regexp)` — returns an array of all exec results.
pub fn string_match_all(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(r) = dispatch_well_known(vm, args, super::native_symbol::WELL_KNOWN_MATCH_ALL) {
        return r;
    }
    let this_str = vm.current_this.to_js_string();
    let arg = args.first().cloned().unwrap_or(JsValue::Undefined);

    let regex = match &arg {
        JsValue::Object(obj) if obj.borrow().internal_tag.as_deref() == Some(REGEXP_TAG) => {
            get_regex(&arg)
        }
        _ => {
            let s = arg.to_js_string();
            Regex::new(&s, "g").ok()
        }
    };

    let regex = match regex {
        Some(r) => r,
        None => return JsValue::new_array(Vec::new()),
    };

    let matches = regex.exec_all(&this_str);
    let elements: Vec<JsValue> = matches
        .iter()
        .map(|m| build_exec_result(vm, m, &this_str))
        .collect();
    JsValue::new_array(elements)
}
