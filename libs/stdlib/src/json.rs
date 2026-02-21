//! JSON parser and serializer for anyOS.
//!
//! Supports the full JSON specification (RFC 8259):
//!   - Objects, arrays, strings, numbers (integer + float), booleans, null
//!   - Unicode escape sequences (\uXXXX) in strings
//!   - Nested structures of arbitrary depth
//!   - Pretty-printing with configurable indentation
//!
//! # Parsing
//! ```ignore
//! use anyos_std::json::Value;
//!
//! let val = Value::parse(r#"{"name": "anyOS", "version": 1}"#).unwrap();
//! let name = val["name"].as_str().unwrap(); // "anyOS"
//! let ver = val["version"].as_i64().unwrap(); // 1
//! ```
//!
//! # Serialization
//! ```ignore
//! use anyos_std::json::Value;
//!
//! let mut obj = Value::new_object();
//! obj.set("name", Value::String("anyOS".into()));
//! obj.set("version", Value::Number(Number::Int(1)));
//!
//! let json = obj.to_string();         // compact
//! let pretty = obj.to_string_pretty(); // indented
//! ```

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::hashmap::HashMap;

// ── Value ────────────────────────────────────────────────────────────────

/// A JSON value.
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Object),
}

/// A JSON number (integer or floating-point).
#[derive(Debug, Clone, Copy)]
pub enum Number {
    Int(i64),
    Float(f64),
}

/// A JSON object (ordered key-value pairs).
/// Preserves insertion order for serialization.
#[derive(Debug, Clone)]
pub struct Object {
    /// Ordered entries (preserves insertion order).
    entries: Vec<(String, Value)>,
    /// Hash index for O(1) key lookup. Value = index into `entries`.
    index: HashMap<String, usize>,
}

impl Object {
    pub fn new() -> Self {
        Object {
            entries: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// Insert or update a key-value pair. Returns the previous value if any.
    pub fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        if let Some(&idx) = self.index.get(&key) {
            let old = core::mem::replace(&mut self.entries[idx].1, value);
            Some(old)
        } else {
            let idx = self.entries.len();
            self.index.insert(key.clone(), idx);
            self.entries.push((key, value));
            None
        }
    }

    /// Get a value by key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.index.get(&String::from(key)).map(|&idx| &self.entries[idx].1)
    }

    /// Get a mutable value by key.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        let k = String::from(key);
        if let Some(&idx) = self.index.get(&k) {
            Some(&mut self.entries[idx].1)
        } else {
            None
        }
    }

    /// Remove a key-value pair. Returns the value if it existed.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let k = String::from(key);
        if let Some(&idx) = self.index.get(&k) {
            self.index.remove(&k);
            let (_, val) = self.entries.remove(idx);
            self.rebuild_index();
            Some(val)
        } else {
            None
        }
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for (i, (k, _)) in self.entries.iter().enumerate() {
            self.index.insert(k.clone(), i);
        }
    }

    /// Check if a key exists.
    pub fn contains_key(&self, key: &str) -> bool {
        self.index.contains_key(&String::from(key))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the object is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over key-value pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate over keys in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    /// Iterate over values in insertion order.
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.entries.iter().map(|(_, v)| v)
    }
}

impl Default for Object {
    fn default() -> Self {
        Self::new()
    }
}

// ── Value constructors & accessors ───────────────────────────────────────

impl Value {
    /// Create a new empty object.
    pub fn new_object() -> Self {
        Value::Object(Object::new())
    }

    /// Create a new empty array.
    pub fn new_array() -> Self {
        Value::Array(Vec::new())
    }

    // ── Type checks ──

    pub fn is_null(&self) -> bool { matches!(self, Value::Null) }
    pub fn is_bool(&self) -> bool { matches!(self, Value::Bool(_)) }
    pub fn is_number(&self) -> bool { matches!(self, Value::Number(_)) }
    pub fn is_string(&self) -> bool { matches!(self, Value::String(_)) }
    pub fn is_array(&self) -> bool { matches!(self, Value::Array(_)) }
    pub fn is_object(&self) -> bool { matches!(self, Value::Object(_)) }

    // ── Accessors ──

    pub fn as_bool(&self) -> Option<bool> {
        match self { Value::Bool(b) => Some(*b), _ => None }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(Number::Int(n)) => Some(*n),
            Value::Number(Number::Float(f)) => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(Number::Int(n)) if *n >= 0 => Some(*n as u64),
            Value::Number(Number::Float(f)) if *f >= 0.0 => Some(*f as u64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(Number::Float(f)) => Some(*f),
            Value::Number(Number::Int(n)) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self { Value::String(s) => Some(s.as_str()), _ => None }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self { Value::Array(a) => Some(a), _ => None }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self { Value::Array(a) => Some(a), _ => None }
    }

    pub fn as_object(&self) -> Option<&Object> {
        match self { Value::Object(o) => Some(o), _ => None }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Object> {
        match self { Value::Object(o) => Some(o), _ => None }
    }

    // ── Object convenience ──

    /// Set a key on an object value. No-op if not an object.
    pub fn set(&mut self, key: &str, value: Value) {
        if let Value::Object(ref mut obj) = self {
            obj.insert(String::from(key), value);
        }
    }

    /// Push a value into an array. No-op if not an array.
    pub fn push(&mut self, value: Value) {
        if let Value::Array(ref mut arr) = self {
            arr.push(value);
        }
    }

    // ── Parsing ──

    /// Parse a JSON string into a Value.
    pub fn parse(input: &str) -> Result<Value, ParseError> {
        let mut parser = Parser::new(input);
        let value = parser.parse_value()?;
        parser.skip_whitespace();
        if parser.pos < parser.input.len() {
            return Err(ParseError::TrailingData(parser.pos));
        }
        Ok(value)
    }

    // ── Serialization ──

    /// Serialize to compact JSON string.
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        serialize_value(self, &mut out, None, 0);
        out
    }

    /// Serialize to pretty-printed JSON string (2-space indent).
    pub fn to_json_string_pretty(&self) -> String {
        let mut out = String::new();
        serialize_value(self, &mut out, Some(2), 0);
        out
    }

    /// Serialize to pretty-printed JSON with custom indent.
    pub fn to_json_string_indent(&self, indent: usize) -> String {
        let mut out = String::new();
        serialize_value(self, &mut out, Some(indent), 0);
        out
    }
}

// ── Index operator (obj["key"] and arr[0]) ───────────────────────────────

static NULL_VALUE: Value = Value::Null;

impl core::ops::Index<&str> for Value {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        match self {
            Value::Object(obj) => obj.get(key).unwrap_or(&NULL_VALUE),
            _ => &NULL_VALUE,
        }
    }
}

impl core::ops::Index<usize> for Value {
    type Output = Value;
    fn index(&self, idx: usize) -> &Value {
        match self {
            Value::Array(arr) => arr.get(idx).unwrap_or(&NULL_VALUE),
            _ => &NULL_VALUE,
        }
    }
}

// ── Display impl ─────────────────────────────────────────────────────────

impl core::fmt::Display for Value {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.to_json_string();
        f.write_str(&s)
    }
}

impl core::fmt::Display for Number {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Number::Int(n) => write!(f, "{}", n),
            Number::Float(v) => {
                // Format float: if it has no fractional part, add .0
                if *v == (*v as i64) as f64 && v.is_finite() {
                    write!(f, "{}.0", *v as i64)
                } else {
                    write!(f, "{}", v)
                }
            }
        }
    }
}

// ── From impls for convenient construction ───────────────────────────────

impl From<bool> for Value {
    fn from(b: bool) -> Self { Value::Bool(b) }
}
impl From<i32> for Value {
    fn from(n: i32) -> Self { Value::Number(Number::Int(n as i64)) }
}
impl From<i64> for Value {
    fn from(n: i64) -> Self { Value::Number(Number::Int(n)) }
}
impl From<u32> for Value {
    fn from(n: u32) -> Self { Value::Number(Number::Int(n as i64)) }
}
impl From<u64> for Value {
    fn from(n: u64) -> Self { Value::Number(Number::Int(n as i64)) }
}
impl From<f64> for Value {
    fn from(f: f64) -> Self { Value::Number(Number::Float(f)) }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self { Value::String(String::from(s)) }
}
impl From<String> for Value {
    fn from(s: String) -> Self { Value::String(s) }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::Array(v.into_iter().map(|x| x.into()).collect())
    }
}

// ── Parse Error ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ParseError {
    UnexpectedEnd,
    UnexpectedChar(usize, char),
    InvalidNumber(usize),
    InvalidEscape(usize),
    InvalidUnicode(usize),
    TrailingData(usize),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::UnexpectedEnd => write!(f, "unexpected end of input"),
            ParseError::UnexpectedChar(pos, ch) => write!(f, "unexpected '{}' at position {}", ch, pos),
            ParseError::InvalidNumber(pos) => write!(f, "invalid number at position {}", pos),
            ParseError::InvalidEscape(pos) => write!(f, "invalid escape at position {}", pos),
            ParseError::InvalidUnicode(pos) => write!(f, "invalid unicode escape at position {}", pos),
            ParseError::TrailingData(pos) => write!(f, "trailing data at position {}", pos),
        }
    }
}

// ── Parser ───────────────────────────────────────────────────────────────

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Parser { input: input.as_bytes(), pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.input.get(self.pos).copied();
        if b.is_some() { self.pos += 1; }
        b
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), ParseError> {
        match self.advance() {
            Some(b) if b == expected => Ok(()),
            Some(b) => Err(ParseError::UnexpectedChar(self.pos - 1, b as char)),
            None => Err(ParseError::UnexpectedEnd),
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'"') => self.parse_string().map(Value::String),
            Some(b'{') => self.parse_object().map(Value::Object),
            Some(b'[') => self.parse_array().map(Value::Array),
            Some(b't') => self.parse_literal(b"true", Value::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", Value::Bool(false)),
            Some(b'n') => self.parse_literal(b"null", Value::Null),
            Some(b) if b == b'-' || b.is_ascii_digit() => self.parse_number(),
            Some(b) => Err(ParseError::UnexpectedChar(self.pos, b as char)),
            None => Err(ParseError::UnexpectedEnd),
        }
    }

    fn parse_literal(&mut self, expected: &[u8], value: Value) -> Result<Value, ParseError> {
        for &b in expected {
            match self.advance() {
                Some(actual) if actual == b => {}
                _ => return Err(ParseError::UnexpectedChar(self.pos, '?')),
            }
        }
        Ok(value)
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut s = String::new();

        loop {
            match self.advance() {
                Some(b'"') => return Ok(s),
                Some(b'\\') => {
                    match self.advance() {
                        Some(b'"') => s.push('"'),
                        Some(b'\\') => s.push('\\'),
                        Some(b'/') => s.push('/'),
                        Some(b'b') => s.push('\x08'),
                        Some(b'f') => s.push('\x0C'),
                        Some(b'n') => s.push('\n'),
                        Some(b'r') => s.push('\r'),
                        Some(b't') => s.push('\t'),
                        Some(b'u') => {
                            let cp = self.parse_unicode_escape()?;
                            // Handle surrogate pairs
                            if (0xD800..=0xDBFF).contains(&cp) {
                                // High surrogate — expect \uXXXX low surrogate
                                if self.advance() != Some(b'\\') || self.advance() != Some(b'u') {
                                    return Err(ParseError::InvalidUnicode(self.pos));
                                }
                                let low = self.parse_unicode_escape()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(ParseError::InvalidUnicode(self.pos));
                                }
                                let combined = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                                match char::from_u32(combined) {
                                    Some(c) => s.push(c),
                                    None => return Err(ParseError::InvalidUnicode(self.pos)),
                                }
                            } else {
                                match char::from_u32(cp) {
                                    Some(c) => s.push(c),
                                    None => return Err(ParseError::InvalidUnicode(self.pos)),
                                }
                            }
                        }
                        _ => return Err(ParseError::InvalidEscape(self.pos)),
                    }
                }
                Some(b) => {
                    // UTF-8 byte — pass through
                    s.push(b as char);
                }
                None => return Err(ParseError::UnexpectedEnd),
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<u32, ParseError> {
        let mut cp = 0u32;
        for _ in 0..4 {
            let b = self.advance().ok_or(ParseError::UnexpectedEnd)?;
            let digit = match b {
                b'0'..=b'9' => (b - b'0') as u32,
                b'a'..=b'f' => (b - b'a' + 10) as u32,
                b'A'..=b'F' => (b - b'A' + 10) as u32,
                _ => return Err(ParseError::InvalidUnicode(self.pos)),
            };
            cp = (cp << 4) | digit;
        }
        Ok(cp)
    }

    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        let mut is_float = false;

        // Optional negative sign
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }

        // Integer part
        match self.peek() {
            Some(b'0') => { self.pos += 1; }
            Some(b) if b >= b'1' && b <= b'9' => {
                self.pos += 1;
                while let Some(b) = self.peek() {
                    if b.is_ascii_digit() { self.pos += 1; } else { break; }
                }
            }
            _ => return Err(ParseError::InvalidNumber(start)),
        }

        // Fractional part
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            let digit_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() { self.pos += 1; } else { break; }
            }
            if self.pos == digit_start {
                return Err(ParseError::InvalidNumber(start));
            }
        }

        // Exponent part
        if let Some(b'e' | b'E') = self.peek() {
            is_float = true;
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            let digit_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() { self.pos += 1; } else { break; }
            }
            if self.pos == digit_start {
                return Err(ParseError::InvalidNumber(start));
            }
        }

        let num_str = core::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| ParseError::InvalidNumber(start))?;

        if is_float {
            let f = parse_f64(num_str).ok_or(ParseError::InvalidNumber(start))?;
            Ok(Value::Number(Number::Float(f)))
        } else {
            match parse_i64(num_str) {
                Some(n) => Ok(Value::Number(Number::Int(n))),
                None => {
                    let f = parse_f64(num_str).ok_or(ParseError::InvalidNumber(start))?;
                    Ok(Value::Number(Number::Float(f)))
                }
            }
        }
    }

    fn parse_object(&mut self) -> Result<Object, ParseError> {
        self.expect(b'{')?;
        let mut obj = Object::new();

        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(obj);
        }

        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.parse_value()?;
            obj.insert(key, value);

            self.skip_whitespace();
            match self.peek() {
                Some(b',') => { self.pos += 1; }
                Some(b'}') => { self.pos += 1; return Ok(obj); }
                Some(b) => return Err(ParseError::UnexpectedChar(self.pos, b as char)),
                None => return Err(ParseError::UnexpectedEnd),
            }
        }
    }

    fn parse_array(&mut self) -> Result<Vec<Value>, ParseError> {
        self.expect(b'[')?;
        let mut arr = Vec::new();

        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(arr);
        }

        loop {
            let value = self.parse_value()?;
            arr.push(value);

            self.skip_whitespace();
            match self.peek() {
                Some(b',') => { self.pos += 1; }
                Some(b']') => { self.pos += 1; return Ok(arr); }
                Some(b) => return Err(ParseError::UnexpectedChar(self.pos, b as char)),
                None => return Err(ParseError::UnexpectedEnd),
            }
        }
    }
}

// ── Number parsing (no_std) ──────────────────────────────────────────────

fn parse_i64(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.is_empty() { return None; }

    let (negative, start) = if bytes[0] == b'-' { (true, 1) } else { (false, 0) };
    if start >= bytes.len() { return None; }

    let mut n: i64 = 0;
    for &b in &bytes[start..] {
        if !b.is_ascii_digit() { return None; }
        n = n.checked_mul(10)?.checked_add((b - b'0') as i64)?;
    }
    if negative { Some(-n) } else { Some(n) }
}

fn parse_f64(s: &str) -> Option<f64> {
    // Simple float parser for JSON numbers.
    // Handles: [-]digits[.digits][e[+-]digits]
    let bytes = s.as_bytes();
    if bytes.is_empty() { return None; }

    let (negative, mut pos) = if bytes[0] == b'-' { (true, 1) } else { (false, 0) };

    // Integer part
    let mut int_part: f64 = 0.0;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        int_part = int_part * 10.0 + (bytes[pos] - b'0') as f64;
        pos += 1;
    }

    // Fractional part
    let mut frac_part: f64 = 0.0;
    if pos < bytes.len() && bytes[pos] == b'.' {
        pos += 1;
        let mut scale: f64 = 0.1;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            frac_part += (bytes[pos] - b'0') as f64 * scale;
            scale *= 0.1;
            pos += 1;
        }
    }

    let mut result = int_part + frac_part;

    // Exponent
    if pos < bytes.len() && (bytes[pos] == b'e' || bytes[pos] == b'E') {
        pos += 1;
        let (exp_neg, exp_start) = if pos < bytes.len() && bytes[pos] == b'-' {
            (true, pos + 1)
        } else if pos < bytes.len() && bytes[pos] == b'+' {
            (false, pos + 1)
        } else {
            (false, pos)
        };
        pos = exp_start;
        let mut exp: i32 = 0;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            exp = exp * 10 + (bytes[pos] - b'0') as i32;
            pos += 1;
        }
        if exp_neg { exp = -exp; }
        // Apply exponent via repeated multiply (no libm pow)
        if exp > 0 {
            for _ in 0..exp.min(308) {
                result *= 10.0;
            }
        } else if exp < 0 {
            for _ in 0..(-exp).min(308) {
                result /= 10.0;
            }
        }
    }

    if negative { result = -result; }
    Some(result)
}

// ── Serialization ────────────────────────────────────────────────────────

fn serialize_value(value: &Value, out: &mut String, indent: Option<usize>, depth: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => {
            let s = format!("{}", n);
            out.push_str(&s);
        }
        Value::String(s) => serialize_string(s, out),
        Value::Array(arr) => serialize_array(arr, out, indent, depth),
        Value::Object(obj) => serialize_object(obj, out, indent, depth),
    }
}

fn serialize_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                // Control character — use \uXXXX
                let hex = format!("\\u{:04x}", c as u32);
                out.push_str(&hex);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn serialize_array(arr: &[Value], out: &mut String, indent: Option<usize>, depth: usize) {
    if arr.is_empty() {
        out.push_str("[]");
        return;
    }

    out.push('[');

    if let Some(indent_size) = indent {
        out.push('\n');
        for i in 0..arr.len() {
            push_indent(out, indent_size, depth + 1);
            serialize_value(&arr[i], out, Some(indent_size), depth + 1);
            if i + 1 < arr.len() {
                out.push(',');
            }
            out.push('\n');
        }
        push_indent(out, indent_size, depth);
    } else {
        for i in 0..arr.len() {
            serialize_value(&arr[i], out, None, depth + 1);
            if i + 1 < arr.len() {
                out.push(',');
            }
        }
    }

    out.push(']');
}

fn serialize_object(obj: &Object, out: &mut String, indent: Option<usize>, depth: usize) {
    if obj.is_empty() {
        out.push_str("{}");
        return;
    }

    out.push('{');

    if let Some(indent_size) = indent {
        out.push('\n');
        let count = obj.entries.len();
        for (i, (key, value)) in obj.entries.iter().enumerate() {
            push_indent(out, indent_size, depth + 1);
            serialize_string(key, out);
            out.push_str(": ");
            serialize_value(value, out, Some(indent_size), depth + 1);
            if i + 1 < count {
                out.push(',');
            }
            out.push('\n');
        }
        push_indent(out, indent_size, depth);
    } else {
        let count = obj.entries.len();
        for (i, (key, value)) in obj.entries.iter().enumerate() {
            serialize_string(key, out);
            out.push(':');
            serialize_value(value, out, None, depth + 1);
            if i + 1 < count {
                out.push(',');
            }
        }
    }

    out.push('}');
}

fn push_indent(out: &mut String, indent_size: usize, depth: usize) {
    for _ in 0..(indent_size * depth) {
        out.push(' ');
    }
}
