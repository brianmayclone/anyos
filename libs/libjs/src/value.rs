//! JavaScript runtime value types.
//!
//! Uses Rc<RefCell<>> for Object/Array/Function to provide proper
//! reference semantics — mutations are visible to all holders.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use core::cell::RefCell;
use core::fmt;

use crate::bytecode::Chunk;

/// A JavaScript value.
///
/// Objects, Arrays, and Functions use Rc for reference semantics:
/// cloning a JsValue only bumps the reference count, so mutations
/// through one handle are visible through all others.
#[derive(Clone)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(Rc<RefCell<JsObject>>),
    Array(Rc<RefCell<JsArray>>),
    Function(Rc<RefCell<JsFunction>>),
}

impl fmt::Debug for JsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsValue::Undefined => write!(f, "undefined"),
            JsValue::Null => write!(f, "null"),
            JsValue::Bool(b) => write!(f, "{}", b),
            JsValue::Number(n) => write!(f, "{}", format_number(*n)),
            JsValue::String(s) => write!(f, "\"{}\"", s),
            JsValue::Object(_) => write!(f, "[object Object]"),
            JsValue::Array(a) => {
                let arr = a.borrow();
                write!(f, "[Array({})]", arr.elements.len())
            }
            JsValue::Function(func) => {
                let fun = func.borrow();
                if let Some(ref name) = fun.name {
                    write!(f, "function {}()", name)
                } else {
                    write!(f, "function()")
                }
            }
        }
    }
}

/// A JavaScript object (property map).
#[derive(Clone, Debug)]
pub struct JsObject {
    pub properties: BTreeMap<String, Property>,
    pub prototype: Option<Rc<RefCell<JsObject>>>,
    pub internal_tag: Option<String>,
    /// Optional hook called when a property is set. Args: (userdata, key, value).
    pub set_hook: Option<fn(*mut u8, &str, &JsValue)>,
    pub set_hook_data: *mut u8,
}

/// A property descriptor (simplified).
#[derive(Clone, Debug)]
pub struct Property {
    pub value: JsValue,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}

impl Property {
    pub fn data(value: JsValue) -> Self {
        Property {
            value,
            writable: true,
            enumerable: true,
            configurable: true,
        }
    }

    pub fn readonly(value: JsValue) -> Self {
        Property {
            value,
            writable: false,
            enumerable: true,
            configurable: false,
        }
    }

    pub fn hidden(value: JsValue) -> Self {
        Property {
            value,
            writable: true,
            enumerable: false,
            configurable: true,
        }
    }
}

impl JsObject {
    pub fn new() -> Self {
        JsObject {
            properties: BTreeMap::new(),
            prototype: None,
            internal_tag: None,
            set_hook: None,
            set_hook_data: core::ptr::null_mut(),
        }
    }

    pub fn with_tag(tag: &str) -> Self {
        JsObject {
            properties: BTreeMap::new(),
            prototype: None,
            internal_tag: Some(String::from(tag)),
            set_hook: None,
            set_hook_data: core::ptr::null_mut(),
        }
    }

    pub fn get(&self, key: &str) -> JsValue {
        if let Some(prop) = self.properties.get(key) {
            return prop.value.clone();
        }
        if let Some(ref proto) = self.prototype {
            return proto.borrow().get(key);
        }
        JsValue::Undefined
    }

    pub fn set(&mut self, key: String, value: JsValue) {
        if let Some(hook) = self.set_hook {
            hook(self.set_hook_data, &key, &value);
        }
        self.properties.insert(key, Property::data(value));
    }

    pub fn set_hidden(&mut self, key: String, value: JsValue) {
        self.properties.insert(key, Property::hidden(value));
    }

    pub fn has(&self, key: &str) -> bool {
        if self.properties.contains_key(key) {
            return true;
        }
        if let Some(ref proto) = self.prototype {
            return proto.borrow().has(key);
        }
        false
    }

    pub fn has_own(&self, key: &str) -> bool {
        self.properties.contains_key(key)
    }

    pub fn delete(&mut self, key: &str) -> bool {
        if let Some(prop) = self.properties.get(key) {
            if !prop.configurable {
                return false;
            }
        }
        self.properties.remove(key).is_some()
    }

    pub fn keys(&self) -> Vec<String> {
        self.properties
            .iter()
            .filter(|(_, p)| p.enumerable)
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// A JavaScript array.
#[derive(Clone, Debug)]
pub struct JsArray {
    pub elements: Vec<JsValue>,
    pub properties: BTreeMap<String, Property>,
}

impl JsArray {
    pub fn new() -> Self {
        JsArray {
            elements: Vec::new(),
            properties: BTreeMap::new(),
        }
    }

    pub fn from_vec(elements: Vec<JsValue>) -> Self {
        JsArray {
            elements,
            properties: BTreeMap::new(),
        }
    }

    pub fn get(&self, index: usize) -> JsValue {
        self.elements.get(index).cloned().unwrap_or(JsValue::Undefined)
    }

    pub fn set(&mut self, index: usize, value: JsValue) {
        while self.elements.len() <= index {
            self.elements.push(JsValue::Undefined);
        }
        self.elements[index] = value;
    }

    pub fn push(&mut self, value: JsValue) {
        self.elements.push(value);
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }
}

/// Compiled or native JavaScript function.
#[derive(Clone)]
pub struct JsFunction {
    pub name: Option<String>,
    pub params: Vec<String>,
    pub kind: FnKind,
    pub this_binding: Option<JsValue>,
    /// Captured upvalue cells — shared `Rc<RefCell<JsValue>>` for each closed-over variable.
    pub upvalues: Vec<Rc<RefCell<JsValue>>>,
    /// The function's `.prototype` object (instance methods for classes, shared across `new` calls).
    pub prototype: Option<Rc<RefCell<JsObject>>>,
    /// Own properties stored directly on the function (e.g. static class methods).
    pub own_props: BTreeMap<String, JsValue>,
}

impl fmt::Debug for JsFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JsFunction({:?})", self.name)
    }
}

/// Function implementation: either bytecode or a native Rust function.
#[derive(Clone)]
pub enum FnKind {
    Bytecode(Chunk),
    Native(fn(&mut crate::vm::Vm, &[JsValue]) -> JsValue),
}

// ── Constructors ──

impl JsValue {
    /// Create a new empty JS object wrapped in Rc<RefCell>.
    pub fn new_object() -> JsValue {
        JsValue::Object(Rc::new(RefCell::new(JsObject::new())))
    }

    /// Create a new JS array from elements.
    pub fn new_array(elements: Vec<JsValue>) -> JsValue {
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(elements))))
    }

    /// Create a new JS function.
    pub fn new_function(func: JsFunction) -> JsValue {
        JsValue::Function(Rc::new(RefCell::new(func)))
    }
}

// ── Type checks ──

impl JsValue {
    pub fn is_undefined(&self) -> bool {
        matches!(self, JsValue::Undefined)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, JsValue::Null)
    }

    pub fn is_nullish(&self) -> bool {
        matches!(self, JsValue::Undefined | JsValue::Null)
    }

    pub fn is_number(&self) -> bool {
        matches!(self, JsValue::Number(_))
    }

    pub fn is_string(&self) -> bool {
        matches!(self, JsValue::String(_))
    }

    pub fn is_bool(&self) -> bool {
        matches!(self, JsValue::Bool(_))
    }

    pub fn is_object(&self) -> bool {
        matches!(self, JsValue::Object(_))
    }

    pub fn is_array(&self) -> bool {
        matches!(self, JsValue::Array(_))
    }

    pub fn is_function(&self) -> bool {
        matches!(self, JsValue::Function(_))
    }

    // ── Type conversions (ECMAScript abstract operations) ──

    /// ToBoolean
    pub fn to_boolean(&self) -> bool {
        match self {
            JsValue::Undefined | JsValue::Null => false,
            JsValue::Bool(b) => *b,
            JsValue::Number(n) => *n != 0.0 && !n.is_nan(),
            JsValue::String(s) => !s.is_empty(),
            JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => true,
        }
    }

    /// ToNumber
    pub fn to_number(&self) -> f64 {
        match self {
            JsValue::Undefined => f64::NAN,
            JsValue::Null => 0.0,
            JsValue::Bool(true) => 1.0,
            JsValue::Bool(false) => 0.0,
            JsValue::Number(n) => *n,
            JsValue::String(s) => parse_js_float(s),
            JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => f64::NAN,
        }
    }

    /// ToString
    pub fn to_js_string(&self) -> String {
        match self {
            JsValue::Undefined => String::from("undefined"),
            JsValue::Null => String::from("null"),
            JsValue::Bool(true) => String::from("true"),
            JsValue::Bool(false) => String::from("false"),
            JsValue::Number(n) => format_number(*n),
            JsValue::String(s) => s.clone(),
            JsValue::Object(_) => String::from("[object Object]"),
            JsValue::Array(a) => {
                let arr = a.borrow();
                let parts: Vec<String> = arr.elements.iter().map(|v| v.to_js_string()).collect();
                let mut out = String::new();
                for (i, p) in parts.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(p);
                }
                out
            }
            JsValue::Function(f) => {
                let fun = f.borrow();
                if let Some(ref name) = fun.name {
                    format!("function {}() {{ [native code] }}", name)
                } else {
                    String::from("function() { [native code] }")
                }
            }
        }
    }

    /// typeof operator result
    pub fn type_of(&self) -> &'static str {
        match self {
            JsValue::Undefined => "undefined",
            JsValue::Null => "object", // historical JS quirk
            JsValue::Bool(_) => "boolean",
            JsValue::Number(_) => "number",
            JsValue::String(_) => "string",
            JsValue::Object(_) | JsValue::Array(_) => "object",
            JsValue::Function(_) => "function",
        }
    }

    /// Abstract equality (==)
    pub fn abstract_eq(&self, other: &JsValue) -> bool {
        match (self, other) {
            (JsValue::Undefined, JsValue::Undefined) => true,
            (JsValue::Null, JsValue::Null) => true,
            (JsValue::Undefined, JsValue::Null) | (JsValue::Null, JsValue::Undefined) => true,
            (JsValue::Number(a), JsValue::Number(b)) => *a == *b,
            (JsValue::String(a), JsValue::String(b)) => *a == *b,
            (JsValue::Bool(a), JsValue::Bool(b)) => *a == *b,
            (JsValue::Number(_), JsValue::String(_)) => {
                self.to_number() == other.to_number()
            }
            (JsValue::String(_), JsValue::Number(_)) => {
                self.to_number() == other.to_number()
            }
            (JsValue::Bool(_), _) => JsValue::Number(self.to_number()).abstract_eq(other),
            (_, JsValue::Bool(_)) => self.abstract_eq(&JsValue::Number(other.to_number())),
            // Object identity via Rc pointer equality
            (JsValue::Object(a), JsValue::Object(b)) => Rc::ptr_eq(a, b),
            (JsValue::Array(a), JsValue::Array(b)) => Rc::ptr_eq(a, b),
            (JsValue::Function(a), JsValue::Function(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Strict equality (===)
    pub fn strict_eq(&self, other: &JsValue) -> bool {
        match (self, other) {
            (JsValue::Undefined, JsValue::Undefined) => true,
            (JsValue::Null, JsValue::Null) => true,
            (JsValue::Number(a), JsValue::Number(b)) => *a == *b,
            (JsValue::String(a), JsValue::String(b)) => *a == *b,
            (JsValue::Bool(a), JsValue::Bool(b)) => *a == *b,
            (JsValue::Object(a), JsValue::Object(b)) => Rc::ptr_eq(a, b),
            (JsValue::Array(a), JsValue::Array(b)) => Rc::ptr_eq(a, b),
            (JsValue::Function(a), JsValue::Function(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Get a property (works on objects, arrays, strings).
    pub fn get_property(&self, key: &str) -> JsValue {
        match self {
            JsValue::Object(obj) => obj.borrow().get(key),
            JsValue::Array(arr) => {
                let a = arr.borrow();
                if key == "length" {
                    return JsValue::Number(a.elements.len() as f64);
                }
                if let Some(idx) = parse_index(key) {
                    return a.get(idx);
                }
                if let Some(prop) = a.properties.get(key) {
                    return prop.value.clone();
                }
                JsValue::Undefined
            }
            JsValue::String(s) => {
                if key == "length" {
                    return JsValue::Number(s.chars().count() as f64);
                }
                if let Some(idx) = parse_index(key) {
                    if let Some(ch) = s.chars().nth(idx) {
                        let mut buf = String::new();
                        buf.push(ch);
                        return JsValue::String(buf);
                    }
                }
                JsValue::Undefined
            }
            _ => JsValue::Undefined,
        }
    }

    /// Set a property.
    pub fn set_property(&self, key: String, value: JsValue) {
        match self {
            JsValue::Object(obj) => {
                obj.borrow_mut().set(key, value);
            }
            JsValue::Array(arr) => {
                let mut a = arr.borrow_mut();
                if let Some(idx) = parse_index(&key) {
                    a.set(idx, value);
                } else if key == "length" {
                    if let JsValue::Number(n) = &value {
                        let new_len = *n as usize;
                        a.elements.truncate(new_len);
                        while a.elements.len() < new_len {
                            a.elements.push(JsValue::Undefined);
                        }
                    }
                } else {
                    a.properties.insert(key, Property::data(value));
                }
            }
            JsValue::Function(f) => {
                f.borrow_mut().own_props.insert(key, value);
            }
            _ => {} // silently ignore
        }
    }

    /// Delete a property.
    pub fn delete_property(&self, key: &str) -> bool {
        match self {
            JsValue::Object(obj) => obj.borrow_mut().delete(key),
            _ => true,
        }
    }
}

fn parse_index(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}

/// Parse a string to f64 (no_std compatible).
pub fn parse_js_float(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return 0.0;
    }
    if s == "Infinity" || s == "+Infinity" {
        return f64::INFINITY;
    }
    if s == "-Infinity" {
        return f64::NEG_INFINITY;
    }
    if s == "NaN" {
        return f64::NAN;
    }

    // Hex
    if s.starts_with("0x") || s.starts_with("0X") {
        return parse_hex_float(&s[2..]);
    }

    let bytes = s.as_bytes();
    let mut i = 0;
    let negative = if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
        true
    } else {
        if i < bytes.len() && bytes[i] == b'+' {
            i += 1;
        }
        false
    };

    let mut integer: f64 = 0.0;
    let mut has_digits = false;
    while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
        integer = integer * 10.0 + (bytes[i] - b'0') as f64;
        i += 1;
        has_digits = true;
    }

    let mut frac: f64 = 0.0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let mut divisor: f64 = 10.0;
        while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
            frac += (bytes[i] - b'0') as f64 / divisor;
            divisor *= 10.0;
            i += 1;
            has_digits = true;
        }
    }

    if !has_digits {
        return f64::NAN;
    }

    let mut result = integer + frac;

    // Exponent
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        let exp_neg = if i < bytes.len() && bytes[i] == b'-' {
            i += 1;
            true
        } else {
            if i < bytes.len() && bytes[i] == b'+' {
                i += 1;
            }
            false
        };
        let mut exp: i32 = 0;
        while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
            exp = exp * 10 + (bytes[i] - b'0') as i32;
            i += 1;
        }
        if exp_neg {
            exp = -exp;
        }
        result *= pow10(exp);
    }

    if i < bytes.len() {
        return f64::NAN;
    }

    if negative {
        -result
    } else {
        result
    }
}

fn parse_hex_float(s: &str) -> f64 {
    let mut result: f64 = 0.0;
    for b in s.bytes() {
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as f64,
            b'a'..=b'f' => (b - b'a' + 10) as f64,
            b'A'..=b'F' => (b - b'A' + 10) as f64,
            _ => return f64::NAN,
        };
        result = result * 16.0 + digit;
    }
    result
}

fn pow10(exp: i32) -> f64 {
    if exp >= 0 {
        let mut result = 1.0f64;
        for _ in 0..exp.min(308) {
            result *= 10.0;
        }
        result
    } else {
        let mut result = 1.0f64;
        for _ in 0..(-exp).min(308) {
            result /= 10.0;
        }
        result
    }
}

/// Format a number for JavaScript string output.
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return String::from("NaN");
    }
    if n.is_infinite() {
        return if n > 0.0 {
            String::from("Infinity")
        } else {
            String::from("-Infinity")
        };
    }
    if n == 0.0 {
        return String::from("0");
    }

    // Integer check
    if n == (n as i64) as f64 && n.abs() < 1e15 {
        return format_i64(n as i64);
    }

    // Float formatting
    format_float(n)
}

fn format_i64(mut n: i64) -> String {
    if n == 0 {
        return String::from("0");
    }
    let negative = n < 0;
    if negative {
        n = -n;
    }
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    if negative {
        buf.push(b'-');
    }
    buf.reverse();
    // SAFETY: buf contains only ASCII digits and '-'
    unsafe { String::from_utf8_unchecked(buf) }
}

/// Compute floor(log10(x)) without using f64::log10 (unavailable in no_std).
/// Precondition: x > 0 and finite.
fn floor_log10(x: f64) -> i32 {
    let mut exp = 0i32;
    let mut v = x;
    if v >= 10.0 {
        while v >= 10.0 {
            v /= 10.0;
            exp += 1;
        }
    } else if v < 1.0 {
        while v < 1.0 {
            v *= 10.0;
            exp -= 1;
        }
    }
    exp
}

fn format_float(n: f64) -> String {
    // Use Rust's built-in float formatter which implements the Grisu3 algorithm
    // for the shortest decimal representation that round-trips back to the same
    // f64 bit pattern (e.g. 3.14 not 3.1400000000000001).
    //
    // Rust's `{}` formatting matches JavaScript's Number-to-String for the
    // common range.  For values >= 1e21 or with exponent < -6 we apply
    // JavaScript's exponential-notation rules manually.
    let abs_val = if n < 0.0 { -n } else { n };

    // Determine the base-10 exponent without using log10 (unavailable in no_std)
    let exp = floor_log10(abs_val);

    if exp >= 21 || exp < -6 {
        return format_float_exponential(n, exp);
    }

    // For the normal range, Rust's {} gives the shortest round-trip representation
    alloc::format!("{}", n)
}

fn format_float_exponential(n: f64, exp: i32) -> String {
    // Build JS-style exponential: coefficient × 10^exponent with "e+" or "e-"
    let negative = n < 0.0;
    let abs_val = if negative { -n } else { n };
    let coeff = abs_val / pow10(exp);

    let coeff_str = alloc::format!("{}", coeff);
    // Strip trailing ".0" if it's an integer coefficient
    let coeff_str = if coeff_str.ends_with(".0") {
        String::from(&coeff_str[..coeff_str.len() - 2])
    } else {
        coeff_str
    };

    let mut out = String::new();
    if negative { out.push('-'); }
    out.push_str(&coeff_str);
    out.push('e');
    if exp >= 0 { out.push('+'); }
    out.push_str(&format_i64(exp as i64));
    out
}
