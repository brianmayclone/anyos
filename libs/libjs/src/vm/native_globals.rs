//! Global functions and type constructors:
//! parseInt, parseFloat, isNaN, isFinite, encodeURIComponent,
//! decodeURIComponent, Object, Array, String, Number, Boolean.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::value::*;
use super::Vm;

// ═══════════════════════════════════════════════════════════
// Global functions
// ═══════════════════════════════════════════════════════════

pub fn global_parse_int(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let radix = args.get(1).map(|v| v.to_number() as u32).unwrap_or(0);
    let s = s.trim();

    if s.is_empty() { return JsValue::Number(f64::NAN); }

    let (negative, s) = if s.starts_with('-') {
        (true, &s[1..])
    } else if s.starts_with('+') {
        (false, &s[1..])
    } else {
        (false, s)
    };

    let actual_radix = if radix == 0 {
        if s.starts_with("0x") || s.starts_with("0X") { 16 } else { 10 }
    } else {
        radix
    };

    let digits = if actual_radix == 16 && (s.starts_with("0x") || s.starts_with("0X")) {
        &s[2..]
    } else {
        s
    };

    if actual_radix < 2 || actual_radix > 36 {
        return JsValue::Number(f64::NAN);
    }

    let mut result: f64 = 0.0;
    let mut found = false;
    for b in digits.bytes() {
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'z' => (b - b'a' + 10) as u32,
            b'A'..=b'Z' => (b - b'A' + 10) as u32,
            _ => break,
        };
        if digit >= actual_radix { break; }
        result = result * actual_radix as f64 + digit as f64;
        found = true;
    }

    if !found { return JsValue::Number(f64::NAN); }
    JsValue::Number(if negative { -result } else { result })
}

pub fn global_parse_float(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    JsValue::Number(parse_js_float(&s))
}

pub fn global_is_nan(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Bool(n.is_nan())
}

pub fn global_is_finite(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Bool(n.is_finite())
}

pub fn global_encode_uri_component(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let mut result = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                result.push(b as char);
            }
            _ => {
                result.push('%');
                result.push(hex_digit(b >> 4));
                result.push(hex_digit(b & 0x0F));
            }
        }
    }
    JsValue::String(result)
}

pub fn global_decode_uri_component(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let bytes = s.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                result.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    JsValue::String(String::from_utf8(result).unwrap_or_default())
}

// ═══════════════════════════════════════════════════════════
// Type constructors
// ═══════════════════════════════════════════════════════════

/// `Object()` / `new Object()` — returns an empty object or wraps a value.
pub fn ctor_object(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(val @ JsValue::Object(_)) => val.clone(),
        Some(val @ JsValue::Array(_)) => val.clone(),
        None | Some(JsValue::Undefined) | Some(JsValue::Null) => JsValue::new_object(),
        _ => JsValue::new_object(),
    }
}

/// `Array(len)` / `Array(...items)` / `new Array(...)`.
pub fn ctor_array(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.len() == 1 {
        if let JsValue::Number(n) = &args[0] {
            let len = *n as usize;
            let elements = alloc::vec![JsValue::Undefined; len];
            return JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(elements))));
        }
    }
    JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(args.to_vec()))))
}

/// `String(value)` — converts to string.
pub fn ctor_string(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    JsValue::String(s)
}

/// `Number(value)` — converts to number.
pub fn ctor_number(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(0.0);
    JsValue::Number(n)
}

/// `Function([...bodyArgs])` — stub constructor.
///
/// A full implementation would compile the body source string.  This stub
/// creates a no-op function, which is enough to make `new Function()` return
/// a truthy callable value and `Function.prototype.isPrototypeOf(Boolean)` work.
pub fn ctor_function(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    // Return undefined so that `new_object()` uses the pre-allocated new_obj
    // (which has Function.prototype in its chain).  The new_obj is an Object,
    // not a real function; a proper implementation would return a JsValue::Function.
    JsValue::Undefined
}

/// `Boolean(value)` — converts to boolean, or creates a wrapper object when called as `new`.
///
/// When called as `new Boolean(x)`, `vm.current_this` is the freshly allocated
/// object (set by `new_object()`).  We tag it with `__bool_data__` and return it
/// so the caller receives the wrapper.  When called as a plain function,
/// `current_this` is `undefined` and we return the primitive bool.
pub fn ctor_boolean(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let b = args.first().map(|v| v.to_boolean()).unwrap_or(false);
    // Detect `new Boolean(x)` by checking whether `current_this` is an Object.
    if let JsValue::Object(obj) = vm.current_this.clone() {
        let mut o = obj.borrow_mut();
        o.internal_tag = Some(String::from("__boolean__"));
        // Store the bool both as [[PrimitiveValue]] (for abstract equality) and
        // as a named property (for backward compatibility with extract_bool_this).
        o.primitive_value = Some(Box::new(JsValue::Bool(b)));
        o.set(String::from("__bool_data__"), JsValue::Bool(b));
        drop(o);
        return vm.current_this.clone();
    }
    JsValue::Bool(b)
}

// ═══════════════════════════════════════════════════════════
// Boolean.prototype methods
// ═══════════════════════════════════════════════════════════

/// `Boolean.prototype.valueOf()` — returns the boolean primitive value.
/// Throws TypeError when called on a non-Boolean `this`.
pub fn boolean_value_of(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match extract_bool_this(vm) {
        Some(v) => v,
        None => {
            let err = vm.make_type_error("Boolean.prototype.valueOf called on non-Boolean");
            vm.throw_native(err);
            JsValue::Undefined
        }
    }
}

/// `Boolean.prototype.toString()` — returns "true" or "false".
/// Throws TypeError when called on a non-Boolean `this`.
pub fn boolean_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match extract_bool_this(vm) {
        Some(JsValue::Bool(true))  => JsValue::String(String::from("true")),
        Some(JsValue::Bool(false)) => JsValue::String(String::from("false")),
        Some(_) => JsValue::String(String::from("false")),
        None => {
            let err = vm.make_type_error("Boolean.prototype.toString called on non-Boolean");
            vm.throw_native(err);
            JsValue::Undefined
        }
    }
}

/// Try to extract the boolean value from `this`.
/// Returns `Some(Bool)` for Boolean primitives and `Boolean` wrapper objects.
/// Returns `None` for any other type (caller should throw TypeError).
fn extract_bool_this(vm: &Vm) -> Option<JsValue> {
    match &vm.current_this {
        JsValue::Bool(_) => Some(vm.current_this.clone()),
        JsValue::Object(obj) => {
            let o = obj.borrow();
            if o.internal_tag.as_deref() == Some("__boolean__") {
                if let Some(prop) = o.properties.get("__bool_data__") {
                    Some(prop.value.clone())
                } else {
                    Some(JsValue::Bool(false))
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════
// Number static methods
// ═══════════════════════════════════════════════════════════

/// `Number.isNaN(value)` — strict NaN check (no coercion).
pub fn number_is_nan(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Number(n)) => JsValue::Bool(n.is_nan()),
        _ => JsValue::Bool(false),
    }
}

/// `Number.isFinite(value)` — strict finite check (no coercion).
pub fn number_is_finite(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Number(n)) => JsValue::Bool(n.is_finite()),
        _ => JsValue::Bool(false),
    }
}

/// `Number.isInteger(value)` — true if value is a finite integer.
pub fn number_is_integer(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Number(n)) => JsValue::Bool(n.is_finite() && *n % 1.0 == 0.0),
        _ => JsValue::Bool(false),
    }
}

// ═══════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + n - 10) as char,
        _ => '0',
    }
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
