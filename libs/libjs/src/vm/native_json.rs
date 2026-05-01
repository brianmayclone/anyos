//! JSON.parse and JSON.stringify.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::Vm;
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// JSON.stringify
// ═══════════════════════════════════════════════════════════

pub fn json_stringify(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let val = args.first().cloned().unwrap_or(JsValue::Undefined);
    let replacer = args.get(1).cloned().unwrap_or(JsValue::Undefined);

    // §25.5.2 step 4: process the replacer argument up front so abrupt
    // completions from accessor reads on a Proxy/array replacer surface here
    // before any value serialization runs.
    let mut replacer_fn: Option<JsValue> = None;
    let mut property_list: Option<alloc::vec::Vec<String>> = None;
    if matches!(
        replacer,
        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
    ) {
        if matches!(replacer, JsValue::Function(_)) {
            replacer_fn = Some(replacer.clone());
        } else {
            // Treat as an array-like list of property keys.
            let len_val = vm.get_property_invoking_getter(&replacer, "length");
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            let len_n = super::native_array::to_number_vm(vm, &len_val);
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            let len = if len_n.is_nan() || len_n < 0.0 {
                0
            } else if !len_n.is_finite() {
                usize::MAX
            } else {
                len_n as usize
            };
            let mut list: alloc::vec::Vec<String> = alloc::vec::Vec::new();
            for i in 0..len {
                let key = alloc::format!("{}", i);
                let v = vm.get_property_invoking_getter(&replacer, &key);
                if vm.pending_exception.is_some() {
                    return JsValue::Undefined;
                }
                let item = match v {
                    JsValue::String(s) => Some(s),
                    JsValue::Number(n) => Some(alloc::format!("{}", n)),
                    JsValue::Object(_) | JsValue::Array(_) => {
                        // ToString via to_primitive (string hint).
                        let p = vm.to_primitive_for_op(v.clone(), "string");
                        if vm.pending_exception.is_some() {
                            return JsValue::Undefined;
                        }
                        Some(p.to_js_string())
                    }
                    _ => None,
                };
                if let Some(s) = item {
                    if !list.contains(&s) {
                        list.push(s);
                    }
                }
            }
            property_list = Some(list);
        }
    }

    let indent = args
        .get(2)
        .map(|v| match v {
            JsValue::Number(n) => {
                let n = *n as usize;
                let mut s = String::new();
                for _ in 0..n.min(10) {
                    s.push(' ');
                }
                s
            }
            JsValue::String(s) => {
                let mut out = String::new();
                for (i, c) in s.chars().enumerate() {
                    if i >= 10 {
                        break;
                    }
                    out.push(c);
                }
                out
            }
            _ => String::new(),
        })
        .unwrap_or_default();

    // §25.5.2 step 11–12: wrap the value in a holder and call SerializeJSONProperty.
    let wrapper = JsValue::new_object();
    wrapper.set_property(String::new(), val);
    let mut state = StringifyState {
        replacer_fn,
        property_list,
        indent,
    };
    match serialize_json_property(vm, &mut state, "", &wrapper, 0) {
        Some(Some(s)) => JsValue::String(s),
        Some(None) => JsValue::Undefined,
        None => JsValue::Undefined,
    }
}

struct StringifyState {
    replacer_fn: Option<JsValue>,
    property_list: Option<alloc::vec::Vec<String>>,
    indent: String,
}

/// §25.5.2.1 SerializeJSONProperty(state, key, holder).
/// Returns:
/// - `Some(Some(s))` — value serialized to `s`
/// - `Some(None)` — value should be omitted (e.g. function, undefined)
/// - `None` — exception is pending; abort
fn serialize_json_property(
    vm: &mut Vm,
    state: &mut StringifyState,
    key: &str,
    holder: &JsValue,
    depth: usize,
) -> Option<Option<String>> {
    if depth > MAX_JSON_DEPTH {
        return Some(None);
    }
    // Step 1: Get(holder, key) — must invoke accessors.
    let mut value = vm.get_property_invoking_getter(holder, key);
    if vm.pending_exception.is_some() {
        return None;
    }
    // Step 2: if Object/Array, look up `toJSON` (with accessor support).
    if matches!(value, JsValue::Object(_) | JsValue::Array(_)) {
        let to_json = vm.get_property_invoking_getter(&value, "toJSON");
        if vm.pending_exception.is_some() {
            return None;
        }
        if matches!(to_json, JsValue::Function(_)) {
            let result = vm.call_value(
                &to_json,
                &[JsValue::String(String::from(key))],
                value.clone(),
            );
            if let Some(exc) = vm.last_exception.take() {
                vm.pending_exception = Some(exc);
                return None;
            }
            if vm.pending_exception.is_some() {
                return None;
            }
            value = result;
        }
    }
    // Step 3: if a replacer function exists, call it.
    if let Some(replacer) = state.replacer_fn.clone() {
        let result = vm.call_value(
            &replacer,
            &[JsValue::String(String::from(key)), value.clone()],
            holder.clone(),
        );
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
            return None;
        }
        if vm.pending_exception.is_some() {
            return None;
        }
        value = result;
    }
    // Step 4: unwrap boxed primitives (Number/String/Boolean wrappers).
    if let JsValue::Object(obj) = &value {
        let prim = obj.borrow().primitive_value.as_deref().cloned();
        if let Some(prim) = prim {
            match prim {
                JsValue::Number(_) | JsValue::String(_) | JsValue::Bool(_) => {
                    value = prim;
                }
                _ => {}
            }
        }
    }
    // Step 5–10: serialize.
    match value {
        JsValue::Null => Some(Some(String::from("null"))),
        JsValue::Bool(true) => Some(Some(String::from("true"))),
        JsValue::Bool(false) => Some(Some(String::from("false"))),
        JsValue::String(s) => Some(Some(stringify_string(&s))),
        JsValue::Number(n) => {
            if n.is_nan() || n.is_infinite() {
                Some(Some(String::from("null")))
            } else {
                Some(Some(format_number(n)))
            }
        }
        JsValue::Array(_) | JsValue::Object(_) => {
            // Per spec: dispatch via IsArray so Proxy([]) is handled as an
            // array (the trap may throw on the length read).
            let is_arr = match super::native_array::is_array_value(vm, &value) {
                Some(b) => b,
                None => return None,
            };
            if is_arr {
                serialize_json_array(vm, state, &value, depth + 1)
            } else {
                serialize_json_object(vm, state, &value, depth + 1)
            }
        }
        // Functions, Undefined, BigInt → omit (caller decides).
        _ => Some(None),
    }
}

fn serialize_json_array(
    vm: &mut Vm,
    state: &mut StringifyState,
    value: &JsValue,
    depth: usize,
) -> Option<Option<String>> {
    let len_val = vm.get_property_invoking_getter(value, "length");
    if vm.pending_exception.is_some() {
        return None;
    }
    let len_n = super::native_array::to_number_vm(vm, &len_val);
    if vm.pending_exception.is_some() {
        return None;
    }
    let len = if len_n.is_nan() || len_n < 0.0 {
        0
    } else if !len_n.is_finite() {
        usize::MAX
    } else {
        len_n as usize
    };
    if len == 0 {
        return Some(Some(String::from("[]")));
    }
    let has_indent = !state.indent.is_empty();
    let indent = state.indent.clone();
    let mut out = String::from("[");
    for i in 0..len {
        if i > 0 {
            out.push(',');
        }
        if has_indent {
            out.push('\n');
            push_indent(&mut out, &indent, depth);
        }
        let key = alloc::format!("{}", i);
        match serialize_json_property(vm, state, &key, value, depth)? {
            Some(s) => out.push_str(&s),
            None => out.push_str("null"),
        }
    }
    if has_indent {
        out.push('\n');
        push_indent(&mut out, &indent, depth - 1);
    }
    out.push(']');
    Some(Some(out))
}

fn serialize_json_object(
    vm: &mut Vm,
    state: &mut StringifyState,
    value: &JsValue,
    depth: usize,
) -> Option<Option<String>> {
    let keys: alloc::vec::Vec<String> = if let Some(list) = &state.property_list {
        list.clone()
    } else if let JsValue::Object(o) = value {
        o.borrow().keys()
    } else {
        alloc::vec::Vec::new()
    };
    if keys.is_empty() {
        return Some(Some(String::from("{}")));
    }
    let has_indent = !state.indent.is_empty();
    let indent = state.indent.clone();
    let mut out = String::from("{");
    let mut first = true;
    for key in &keys {
        let serialized = serialize_json_property(vm, state, key, value, depth)?;
        if let Some(s) = serialized {
            if !first {
                out.push(',');
            }
            first = false;
            if has_indent {
                out.push('\n');
                push_indent(&mut out, &indent, depth);
            }
            out.push_str(&stringify_string(key));
            out.push(':');
            if has_indent {
                out.push(' ');
            }
            out.push_str(&s);
        }
    }
    if first {
        // No serializable properties → still emit "{}"
        return Some(Some(String::from("{}")));
    }
    if has_indent {
        out.push('\n');
        push_indent(&mut out, &indent, depth - 1);
    }
    out.push('}');
    Some(Some(out))
}

/// Maximum nesting depth for JSON.stringify to prevent stack overflow.
const MAX_JSON_DEPTH: usize = 128;

fn stringify_value(val: &JsValue, indent: &str, depth: usize) -> Option<String> {
    // Depth limit to prevent stack overflow from circular references
    if depth > MAX_JSON_DEPTH {
        return None; // treated as undefined → omitted or "null"
    }
    match val {
        JsValue::Empty => None,
        JsValue::Undefined | JsValue::Function(_) | JsValue::BigInt(_) => None,
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
            if a.length == 0 {
                return Some(String::from("[]"));
            }
            let has_indent = !indent.is_empty();
            let mut out = String::from("[");
            let new_depth = depth + 1;
            for i in 0..a.length {
                if i > 0 {
                    out.push(',');
                }
                if has_indent {
                    out.push('\n');
                    push_indent(&mut out, indent, new_depth);
                }
                let el = a.get(i);
                match stringify_value(&el, indent, new_depth) {
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
                        if !first {
                            out.push(',');
                        }
                        first = false;
                        if has_indent {
                            out.push('\n');
                            push_indent(&mut out, indent, new_depth);
                        }
                        out.push_str(&stringify_string(key));
                        out.push(':');
                        if has_indent {
                            out.push(' ');
                        }
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
    let val = match result {
        Some(val) => {
            // After parsing, only whitespace should remain
            skip_ws(bytes, &mut pos);
            if pos < bytes.len() {
                let err = vm.make_syntax_error("Unexpected token in JSON");
                vm.throw_native(err);
                return JsValue::Undefined;
            }
            val
        }
        None => {
            let err = vm.make_syntax_error("Unexpected end of JSON input");
            vm.throw_native(err);
            return JsValue::Undefined;
        }
    };

    // §25.5.1.1 step 7: if reviver is callable, wrap the result in a holder
    // object and walk it via InternalizeJSONProperty.
    let reviver = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if matches!(reviver, JsValue::Function(_)) {
        let root = JsValue::new_object();
        root.set_property(String::new(), val);
        return internalize_json_property(vm, &root, "", &reviver).unwrap_or(JsValue::Undefined);
    }
    val
}

/// §25.5.1.1.1 InternalizeJSONProperty(holder, name, reviver). Returns
/// `None` if an exception is pending so the caller can short-circuit.
fn internalize_json_property(
    vm: &mut Vm,
    holder: &JsValue,
    name: &str,
    reviver: &JsValue,
) -> Option<JsValue> {
    let val = vm.get_property_invoking_getter(holder, name);
    if vm.pending_exception.is_some() {
        return None;
    }
    if matches!(val, JsValue::Object(_) | JsValue::Array(_)) {
        // §25.5.1.1.1 step 2.a: IsArray(val). Proxies must dispatch to their
        // target so a Proxy([]) is treated as an array.
        let is_arr = super::native_array::is_array_value(vm, &val)?;
        if is_arr {
            let len_val = vm.get_property_invoking_getter(&val, "length");
            if vm.pending_exception.is_some() {
                return None;
            }
            let len_n = super::native_array::to_number_vm(vm, &len_val);
            if vm.pending_exception.is_some() {
                return None;
            }
            let len = if len_n.is_nan() || len_n < 0.0 {
                0
            } else if !len_n.is_finite() {
                usize::MAX
            } else {
                len_n as usize
            };
            for i in 0..len {
                let key = alloc::format!("{}", i);
                let new_element = internalize_json_property(vm, &val, &key, reviver)?;
                if matches!(new_element, JsValue::Undefined) {
                    if !vm.delete_property_or_throw(&val, &key) {
                        return None;
                    }
                    if vm.pending_exception.is_some() {
                        return None;
                    }
                } else {
                    if !vm.create_data_property_or_throw(&val, &key, new_element) {
                        return None;
                    }
                    if vm.pending_exception.is_some() {
                        return None;
                    }
                }
            }
        } else {
            let keys = vm.own_property_keys(&val);
            if vm.pending_exception.is_some() {
                return None;
            }
            for key in keys {
                let new_element = internalize_json_property(vm, &val, &key, reviver)?;
                if matches!(new_element, JsValue::Undefined) {
                    if !vm.delete_property_or_throw(&val, &key) {
                        return None;
                    }
                    if vm.pending_exception.is_some() {
                        return None;
                    }
                } else {
                    if !vm.create_data_property_or_throw(&val, &key, new_element) {
                        return None;
                    }
                    if vm.pending_exception.is_some() {
                        return None;
                    }
                }
            }
        }
    }
    let result = vm.call_value(
        reviver,
        &[JsValue::String(String::from(name)), val],
        holder.clone(),
    );
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
        return None;
    }
    if vm.pending_exception.is_some() {
        return None;
    }
    Some(result)
}

fn skip_ws(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }
}

fn parse_value(vm: &mut Vm, bytes: &[u8], pos: &mut usize) -> Option<JsValue> {
    skip_ws(bytes, pos);
    if *pos >= bytes.len() {
        return None;
    }

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
    if *pos >= bytes.len() || bytes[*pos] != b'"' {
        return None;
    }
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
            if *pos >= bytes.len() {
                return None;
            }
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
                        if *pos >= bytes.len() {
                            return None;
                        }
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
                let width = if b & 0xE0 == 0xC0 {
                    2
                } else if b & 0xF0 == 0xE0 {
                    3
                } else if b & 0xF8 == 0xF0 {
                    4
                } else {
                    1
                };
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
    if *pos < bytes.len() && bytes[*pos] == b'-' {
        *pos += 1;
    }

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
        if *pos < bytes.len() && (bytes[*pos] == b'+' || bytes[*pos] == b'-') {
            *pos += 1;
        }
        while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' {
            *pos += 1;
        }
    }

    if !has_digits {
        return None;
    }

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
        if *pos >= bytes.len() || bytes[*pos] != b':' {
            return None;
        }
        *pos += 1;
        let value = parse_value(vm, bytes, pos)?;
        obj.set(key, value);

        skip_ws(bytes, pos);
        if *pos >= bytes.len() {
            return None;
        }
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
        return Some(JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(
            elements,
        )))));
    }

    loop {
        let value = parse_value(vm, bytes, pos)?;
        elements.push(value);

        skip_ws(bytes, pos);
        if *pos >= bytes.len() {
            return None;
        }
        if bytes[*pos] == b']' {
            *pos += 1;
            return Some(JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(
                elements,
            )))));
        }
        if bytes[*pos] == b',' {
            *pos += 1;
        } else {
            return None;
        }
    }
}
