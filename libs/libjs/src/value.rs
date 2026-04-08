//! JavaScript runtime value types.
//!
//! Uses Rc<RefCell<>> for Object/Array/Function to provide proper
//! reference semantics — mutations are visible to all holders.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

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
    BigInt(JsBigInt),
}

/// Arbitrary-precision integer (ES2020 BigInt).
///
/// Uses a sign + magnitude representation with base-2^32 digits.
/// Sufficient for all practical BigInt use cases (crypto, IDs, bitfields).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsBigInt {
    /// True if the value is negative.
    pub negative: bool,
    /// Magnitude in base 2^32 (little-endian: digits[0] is least significant).
    /// Empty vec represents zero.
    pub digits: Vec<u32>,
}

impl JsBigInt {
    pub fn zero() -> Self {
        JsBigInt {
            negative: false,
            digits: Vec::new(),
        }
    }

    pub fn from_i64(val: i64) -> Self {
        if val == 0 {
            return Self::zero();
        }
        let negative = val < 0;
        let abs = if val == i64::MIN {
            val as u64
        } else if negative {
            (-val) as u64
        } else {
            val as u64
        };
        let lo = abs as u32;
        let hi = (abs >> 32) as u32;
        let mut digits = vec![lo];
        if hi != 0 {
            digits.push(hi);
        }
        JsBigInt { negative, digits }
    }

    pub fn is_zero(&self) -> bool {
        self.digits.is_empty() || self.digits.iter().all(|&d| d == 0)
    }

    /// Convert to string in given radix (2-36).
    pub fn to_string_radix(&self, radix: u32) -> String {
        if self.is_zero() {
            return String::from("0");
        }
        // Simple conversion: repeated division by radix.
        let mut result = Vec::new();
        let mut tmp = self.digits.clone();
        while !tmp.is_empty() && !tmp.iter().all(|&d| d == 0) {
            let rem = div_by_u32(&mut tmp, radix);
            result.push(digit_char(rem));
            // Remove leading zeros.
            while tmp.last() == Some(&0) {
                tmp.pop();
            }
        }
        if result.is_empty() {
            return String::from("0");
        }
        result.reverse();
        let mut s = String::new();
        if self.negative {
            s.push('-');
        }
        for c in result {
            s.push(c);
        }
        s
    }

    /// Parse a BigInt from a decimal string (no trailing 'n').
    pub fn from_str(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }
        let (negative, digits_str) = if s.starts_with('-') {
            (true, &s[1..])
        } else {
            (false, s)
        };
        if digits_str.is_empty() || !digits_str.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        // Build magnitude by multiplying by 10 and adding each digit.
        let mut mag: Vec<u32> = Vec::new();
        for ch in digits_str.chars() {
            let d = ch as u32 - '0' as u32;
            mul_by_u32(&mut mag, 10);
            add_u32(&mut mag, d);
        }
        // Remove leading zeros.
        while mag.last() == Some(&0) {
            mag.pop();
        }
        Some(JsBigInt {
            negative: negative && !mag.is_empty(),
            digits: mag,
        })
    }

    pub fn add(&self, other: &JsBigInt) -> JsBigInt {
        if self.negative == other.negative {
            // Same sign: add magnitudes.
            JsBigInt {
                negative: self.negative,
                digits: add_mag(&self.digits, &other.digits),
            }
        } else {
            // Different signs: subtract smaller from larger.
            match cmp_mag(&self.digits, &other.digits) {
                core::cmp::Ordering::Equal => JsBigInt::zero(),
                core::cmp::Ordering::Greater => JsBigInt {
                    negative: self.negative,
                    digits: sub_mag(&self.digits, &other.digits),
                },
                core::cmp::Ordering::Less => JsBigInt {
                    negative: other.negative,
                    digits: sub_mag(&other.digits, &self.digits),
                },
            }
        }
    }

    pub fn sub(&self, other: &JsBigInt) -> JsBigInt {
        let neg_other = JsBigInt {
            negative: !other.negative && !other.is_zero(),
            digits: other.digits.clone(),
        };
        self.add(&neg_other)
    }

    pub fn mul(&self, other: &JsBigInt) -> JsBigInt {
        if self.is_zero() || other.is_zero() {
            return JsBigInt::zero();
        }
        let mut result = vec![0u32; self.digits.len() + other.digits.len()];
        for (i, &a) in self.digits.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in other.digits.iter().enumerate() {
                let prod = a as u64 * b as u64 + result[i + j] as u64 + carry;
                result[i + j] = prod as u32;
                carry = prod >> 32;
            }
            if carry > 0 {
                result[i + other.digits.len()] += carry as u32;
            }
        }
        while result.last() == Some(&0) {
            result.pop();
        }
        JsBigInt {
            negative: self.negative != other.negative && !result.is_empty(),
            digits: result,
        }
    }

    pub fn div(&self, other: &JsBigInt) -> JsBigInt {
        if other.is_zero() {
            return JsBigInt::zero(); // Division by zero → handled by caller
        }
        let (q, _) = div_rem(self, other);
        q
    }

    pub fn rem(&self, other: &JsBigInt) -> JsBigInt {
        if other.is_zero() {
            return JsBigInt::zero();
        }
        let (_, r) = div_rem(self, other);
        r
    }

    pub fn neg(&self) -> JsBigInt {
        JsBigInt {
            negative: !self.negative && !self.is_zero(),
            digits: self.digits.clone(),
        }
    }

    pub fn cmp_val(&self, other: &JsBigInt) -> core::cmp::Ordering {
        use core::cmp::Ordering::*;
        if self.negative != other.negative {
            return if self.negative { Less } else { Greater };
        }
        let mag = cmp_mag(&self.digits, &other.digits);
        if self.negative {
            mag.reverse()
        } else {
            mag
        }
    }

    pub fn to_f64(&self) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let mut val = 0.0f64;
        let base = (1u64 << 32) as f64;
        for &d in self.digits.iter().rev() {
            val = val * base + d as f64;
        }
        if self.negative {
            -val
        } else {
            val
        }
    }
}

// ── BigInt arithmetic helpers ──

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let len = a.len().max(b.len());
    let mut result = Vec::with_capacity(len + 1);
    let mut carry = 0u64;
    for i in 0..len {
        let av = *a.get(i).unwrap_or(&0) as u64;
        let bv = *b.get(i).unwrap_or(&0) as u64;
        let sum = av + bv + carry;
        result.push(sum as u32);
        carry = sum >> 32;
    }
    if carry > 0 {
        result.push(carry as u32);
    }
    result
}

fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    // Assumes a >= b.
    let mut result = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for i in 0..a.len() {
        let av = *a.get(i).unwrap_or(&0) as i64;
        let bv = *b.get(i).unwrap_or(&0) as i64;
        let diff = av - bv - borrow;
        if diff < 0 {
            result.push((diff + (1i64 << 32)) as u32);
            borrow = 1;
        } else {
            result.push(diff as u32);
            borrow = 0;
        }
    }
    while result.last() == Some(&0) {
        result.pop();
    }
    result
}

fn cmp_mag(a: &[u32], b: &[u32]) -> core::cmp::Ordering {
    let al = a.len();
    let bl = b.len();
    if al != bl {
        return al.cmp(&bl);
    }
    for i in (0..al).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    core::cmp::Ordering::Equal
}

fn mul_by_u32(mag: &mut Vec<u32>, factor: u32) {
    let mut carry = 0u64;
    for d in mag.iter_mut() {
        let prod = *d as u64 * factor as u64 + carry;
        *d = prod as u32;
        carry = prod >> 32;
    }
    if carry > 0 {
        mag.push(carry as u32);
    }
}

fn add_u32(mag: &mut Vec<u32>, val: u32) {
    if mag.is_empty() {
        if val != 0 {
            mag.push(val);
        }
        return;
    }
    let mut carry = val as u64;
    for d in mag.iter_mut() {
        let sum = *d as u64 + carry;
        *d = sum as u32;
        carry = sum >> 32;
        if carry == 0 {
            return;
        }
    }
    if carry > 0 {
        mag.push(carry as u32);
    }
}

fn div_by_u32(mag: &mut Vec<u32>, divisor: u32) -> u32 {
    let mut rem = 0u64;
    for d in mag.iter_mut().rev() {
        let val = (rem << 32) | (*d as u64);
        *d = (val / divisor as u64) as u32;
        rem = val % divisor as u64;
    }
    rem as u32
}

fn div_rem(a: &JsBigInt, b: &JsBigInt) -> (JsBigInt, JsBigInt) {
    // Simple long division via repeated subtraction (for now).
    if a.is_zero() {
        return (JsBigInt::zero(), JsBigInt::zero());
    }
    if cmp_mag(&a.digits, &b.digits) == core::cmp::Ordering::Less {
        return (JsBigInt::zero(), a.clone());
    }
    // Use digit-by-digit division.
    let mut remainder = JsBigInt::zero();
    let mut quotient_digits = vec![0u32; a.digits.len()];
    let abs_b = JsBigInt {
        negative: false,
        digits: b.digits.clone(),
    };
    for i in (0..a.digits.len()).rev() {
        // Shift remainder left by one digit and add a.digits[i].
        remainder.digits.insert(0, a.digits[i]);
        while remainder.digits.last() == Some(&0) {
            remainder.digits.pop();
        }
        // Find how many times b fits into remainder.
        let mut count = 0u32;
        while cmp_mag(&remainder.digits, &abs_b.digits) != core::cmp::Ordering::Less {
            remainder = remainder.sub(&abs_b);
            count += 1;
            if count > u32::MAX - 1 {
                break;
            }
        }
        quotient_digits[i] = count;
    }
    while quotient_digits.last() == Some(&0) {
        quotient_digits.pop();
    }
    let q_neg = a.negative != b.negative && !quotient_digits.is_empty();
    let r_neg = a.negative && !remainder.is_zero();
    (
        JsBigInt {
            negative: q_neg,
            digits: quotient_digits,
        },
        JsBigInt {
            negative: r_neg,
            digits: remainder.digits,
        },
    )
}

fn digit_char(d: u32) -> char {
    if d < 10 {
        (b'0' + d as u8) as char
    } else {
        (b'a' + (d - 10) as u8) as char
    }
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
                write!(f, "[Array({})]", arr.length)
            }
            JsValue::BigInt(bi) => write!(f, "{}n", bi.to_string_radix(10)),
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
    /// The `[[PrimitiveValue]]` for wrapper objects (Boolean, Number, String).
    /// Used in abstract equality (`==`) and ToPrimitive coercion so that e.g.
    /// `new Boolean(false) == false` evaluates to `true`.
    pub primitive_value: Option<Box<JsValue>>,
    /// Optional hook called when a property is set. Args: (userdata, key, value).
    pub set_hook: Option<fn(*mut u8, &str, &JsValue)>,
    pub set_hook_data: *mut u8,
}

/// A property descriptor.
///
/// Can be either a *data* descriptor (has `value` + `writable`) or an
/// *accessor* descriptor (has `getter` and/or `setter`).  When `getter`
/// or `setter` is `Some`, the property is an accessor — `value` and
/// `writable` are ignored.
#[derive(Clone, Debug)]
pub struct Property {
    pub value: JsValue,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
    /// Getter function (accessor descriptor).
    pub getter: Option<JsValue>,
    /// Setter function (accessor descriptor).
    pub setter: Option<JsValue>,
}

impl Property {
    pub fn data(value: JsValue) -> Self {
        Property {
            value,
            writable: true,
            enumerable: true,
            configurable: true,
            getter: None,
            setter: None,
        }
    }

    pub fn readonly(value: JsValue) -> Self {
        Property {
            value,
            writable: false,
            enumerable: true,
            configurable: false,
            getter: None,
            setter: None,
        }
    }

    pub fn hidden(value: JsValue) -> Self {
        Property {
            value,
            writable: true,
            enumerable: false,
            configurable: true,
            getter: None,
            setter: None,
        }
    }

    /// Create an accessor property with a getter and/or setter.
    pub fn accessor(getter: Option<JsValue>, setter: Option<JsValue>) -> Self {
        Property {
            value: JsValue::Undefined,
            writable: false,
            enumerable: true,
            configurable: true,
            getter,
            setter,
        }
    }

    /// True if this is an accessor descriptor (has getter or setter).
    pub fn is_accessor(&self) -> bool {
        self.getter.is_some() || self.setter.is_some()
    }
}

impl JsObject {
    pub fn new() -> Self {
        JsObject {
            properties: BTreeMap::new(),
            prototype: None,
            internal_tag: None,
            primitive_value: None,
            set_hook: None,
            set_hook_data: core::ptr::null_mut(),
        }
    }

    pub fn with_tag(tag: &str) -> Self {
        JsObject {
            properties: BTreeMap::new(),
            prototype: None,
            internal_tag: Some(String::from(tag)),
            primitive_value: None,
            set_hook: None,
            set_hook_data: core::ptr::null_mut(),
        }
    }

    pub fn get(&self, key: &str) -> JsValue {
        if let Some(prop) = self.properties.get(key) {
            // Accessor descriptors: getter must be invoked by the VM (not here)
            // because calling a JsFunction requires the VM. We return a sentinel.
            // The VM's get_property_with_proto handles getter invocation.
            if prop.is_accessor() {
                // Return Undefined for accessor without getter
                if prop.getter.is_none() {
                    return JsValue::Undefined;
                }
            }
            return prop.value.clone();
        }
        if let Some(ref proto) = self.prototype {
            return proto.borrow().get(key);
        }
        JsValue::Undefined
    }

    /// Get the raw property descriptor (for getter/setter detection by the VM).
    pub fn get_property_descriptor(&self, key: &str) -> Option<&Property> {
        if let Some(prop) = self.properties.get(key) {
            return Some(prop);
        }
        None
    }

    /// Walk the prototype chain looking for a property descriptor.
    pub fn find_property_descriptor(&self, key: &str) -> Option<Property> {
        if let Some(prop) = self.properties.get(key) {
            return Some(prop.clone());
        }
        if let Some(ref proto) = self.prototype {
            return proto.borrow().find_property_descriptor(key);
        }
        None
    }

    pub fn set(&mut self, key: String, value: JsValue) {
        if let Some(hook) = self.set_hook {
            hook(self.set_hook_data, &key, &value);
        }
        // If the property already exists and is non-writable, reject the write
        // (ES2023 §10.1.2.1 OrdinarySet step 3).  Skip for accessor properties.
        if let Some(existing) = self.properties.get(&key) {
            if !existing.writable && existing.getter.is_none() {
                return;
            }
            // Preserve the existing descriptor flags, only update the value.
            let mut updated = existing.clone();
            updated.value = value;
            self.properties.insert(key, updated);
            return;
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
        self.properties.remove(key);
        true
    }

    /// Enumerable string-keyed property names (excludes symbol-like keys).
    /// Used for `for...in`, `Object.keys()`, `Object.values()`, `Object.entries()`.
    pub fn keys(&self) -> Vec<String> {
        self.properties
            .iter()
            .filter(|(k, p)| p.enumerable && !k.starts_with("__symbol_"))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// All own string-keyed property names (excludes symbol-like keys).
    /// Used for `Object.getOwnPropertyNames()`.
    pub fn own_property_names(&self) -> Vec<String> {
        self.properties
            .keys()
            .filter(|k| !k.starts_with("__symbol_"))
            .cloned()
            .collect()
    }

    /// Own symbol-keyed property names only.
    /// Used for `Object.getOwnPropertySymbols()`.
    pub fn own_symbol_keys(&self) -> Vec<String> {
        self.properties
            .keys()
            .filter(|k| k.starts_with("__symbol_"))
            .cloned()
            .collect()
    }
}

/// A JavaScript array — sparse storage like V8.
///
/// Only indices that have actually been written consume memory.
/// `length` is a separate logical value; unset indices return `undefined`
/// without ever being materialised.
#[derive(Clone, Debug)]
pub struct JsArray {
    /// Sparse element storage — only indices that were explicitly set.
    pub elements: BTreeMap<usize, JsValue>,
    /// Logical array length (ES2023 §10.4.2).
    pub length: usize,
    pub properties: BTreeMap<String, Property>,
}

/// Maximum valid ES array index (2^32 − 2).  Index 2^32 − 1 is a regular
/// property, not an array element (ES2023 §6.1.7).
pub const MAX_ARRAY_INDEX: usize = 0xFFFF_FFFE; // 4294967294

impl JsArray {
    pub fn new() -> Self {
        JsArray {
            elements: BTreeMap::new(),
            length: 0,
            properties: BTreeMap::new(),
        }
    }

    /// Create from a dense Vec (e.g. array literal `[1, 2, 3]`).
    pub fn from_vec(v: Vec<JsValue>) -> Self {
        let len = v.len();
        let mut map = BTreeMap::new();
        for (i, val) in v.into_iter().enumerate() {
            map.insert(i, val);
        }
        JsArray {
            elements: map,
            length: len,
            properties: BTreeMap::new(),
        }
    }

    /// Get element at `index`.  Returns `Undefined` for unset indices.
    pub fn get(&self, index: usize) -> JsValue {
        self.elements
            .get(&index)
            .cloned()
            .unwrap_or(JsValue::Undefined)
    }

    /// Set element at `index`.  Updates `length` if needed.
    /// No memory is allocated for intervening unset indices.
    pub fn set(&mut self, index: usize, value: JsValue) {
        self.elements.insert(index, value);
        if index >= self.length {
            self.length = index + 1;
        }
    }

    /// Returns true if `index` has been explicitly set (distinguishes
    /// a hole from an explicit `undefined` value).
    pub fn has(&self, index: usize) -> bool {
        self.elements.contains_key(&index)
    }

    /// Delete element at `index` (creates a hole).  Does NOT change `length`.
    pub fn delete(&mut self, index: usize) -> bool {
        self.elements.remove(&index);
        true
    }

    /// Append a value at the end (at the current `length`).
    pub fn push(&mut self, value: JsValue) {
        let idx = self.length;
        self.elements.insert(idx, value);
        self.length = idx + 1;
    }

    /// Remove and return the last element.
    pub fn pop(&mut self) -> JsValue {
        if self.length == 0 {
            return JsValue::Undefined;
        }
        self.length -= 1;
        self.elements
            .remove(&self.length)
            .unwrap_or(JsValue::Undefined)
    }

    /// Logical length.
    pub fn len(&self) -> usize {
        self.length
    }

    /// Set the logical length.  Removes entries >= `new_len`.
    pub fn set_length(&mut self, new_len: usize) {
        if new_len < self.length {
            // Truncate: remove all entries with index >= new_len.
            let to_remove: Vec<usize> = self.elements.range(new_len..).map(|(&k, _)| k).collect();
            for k in to_remove {
                self.elements.remove(&k);
            }
        }
        self.length = new_len;
    }

    /// Number of actually stored elements (not the logical length).
    pub fn count(&self) -> usize {
        self.elements.len()
    }

    /// Iterate over all (index, &value) pairs that actually exist,
    /// in ascending index order.  Holes are skipped.
    pub fn iter_entries(&self) -> impl Iterator<Item = (usize, &JsValue)> {
        self.elements.iter().map(|(&k, v)| (k, v))
    }

    /// Collect all values in index order as a dense Vec (for interop with
    /// code that needs a contiguous slice, e.g. function call args).
    /// Holes are filled with Undefined.
    pub fn to_dense_vec(&self) -> Vec<JsValue> {
        if self.length == 0 {
            return Vec::new();
        }
        let mut v = Vec::with_capacity(self.length.min(4096));
        for i in 0..self.length {
            v.push(self.elements.get(&i).cloned().unwrap_or(JsValue::Undefined));
        }
        v
    }

    /// Collect only the actually-set values in index order (no holes).
    pub fn values_vec(&self) -> Vec<JsValue> {
        self.elements.values().cloned().collect()
    }

    /// Remove element at index and shift higher indices down by 1
    /// (used by `shift`, `splice`).
    pub fn remove_and_shift(&mut self, index: usize) -> JsValue {
        let removed = self.elements.remove(&index).unwrap_or(JsValue::Undefined);
        // Collect keys > index and shift them down.
        let to_shift: Vec<(usize, JsValue)> = self
            .elements
            .range((index + 1)..)
            .map(|(&k, v)| (k, v.clone()))
            .collect();
        for (k, v) in to_shift {
            self.elements.remove(&k);
            self.elements.insert(k - 1, v);
        }
        if self.length > 0 {
            self.length -= 1;
        }
        removed
    }

    /// Insert value at index and shift higher indices up by 1
    /// (used by `unshift`, `splice`).
    pub fn insert_and_shift(&mut self, index: usize, value: JsValue) {
        // Shift existing entries at index..length up by 1.
        let to_shift: Vec<(usize, JsValue)> = self
            .elements
            .range(index..)
            .map(|(&k, v)| (k, v.clone()))
            .rev()
            .collect();
        for (k, v) in to_shift {
            self.elements.remove(&k);
            self.elements.insert(k + 1, v);
        }
        self.elements.insert(index, value);
        self.length += 1;
    }

    /// Clear all elements and set length to 0.
    pub fn clear(&mut self) {
        self.elements.clear();
        self.length = 0;
    }

    /// Reverse elements in place.
    pub fn reverse(&mut self) {
        let entries: Vec<(usize, JsValue)> =
            self.elements.iter().map(|(&k, v)| (k, v.clone())).collect();
        self.elements.clear();
        for (k, v) in entries {
            self.elements.insert(self.length - 1 - k, v);
        }
    }
}

/// Compiled or native JavaScript function.
#[derive(Clone)]
pub struct JsFunction {
    pub name: Option<String>,
    pub params: Vec<String>,
    pub kind: FnKind,
    pub this_binding: Option<JsValue>,
    /// Arguments pre-bound via `Function.prototype.bind()` — prepended to
    /// the actual call arguments (ES2023 §10.4.1.1 [[Call]]).
    pub bound_args: Vec<JsValue>,
    /// Captured upvalue cells — shared `Rc<RefCell<JsValue>>` for each closed-over variable.
    pub upvalues: Vec<Rc<RefCell<JsValue>>>,
    /// The function's `.prototype` object (instance methods for classes, shared across `new` calls).
    pub prototype: Option<Rc<RefCell<JsObject>>>,
    /// Own properties stored directly on the function (e.g. static class methods).
    pub own_props: BTreeMap<String, JsValue>,
    /// Explicit arity for native functions (overrides params.len() for Function.length).
    pub arity: Option<usize>,
    /// For class constructors: the super class constructor (used by Op::SuperCall).
    pub super_class: Option<JsValue>,
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

impl FnKind {
    /// Returns true if this is an arrow function (no own `prototype`).
    pub fn is_arrow(&self) -> bool {
        match self {
            FnKind::Bytecode(ch) => ch.is_arrow,
            FnKind::Native(_) => false,
        }
    }
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
            JsValue::BigInt(bi) => !bi.is_zero(),
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
            JsValue::Object(obj) => {
                // Wrapper objects (new Number/String/Boolean) — unwrap primitive value.
                if let Some(prim) = &obj.borrow().primitive_value {
                    return prim.to_number();
                }
                f64::NAN
            }
            JsValue::Array(_) | JsValue::Function(_) => f64::NAN,
            JsValue::BigInt(bi) => bi.to_f64(),
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
                let mut out = String::new();
                for i in 0..arr.length {
                    if i > 0 {
                        out.push(',');
                    }
                    if let Some(v) = arr.elements.get(&i) {
                        match v {
                            JsValue::Undefined | JsValue::Null => {}
                            _ => out.push_str(&v.to_js_string()),
                        }
                    }
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
            JsValue::BigInt(bi) => {
                let mut s = bi.to_string_radix(10);
                s.push('n');
                s
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
            JsValue::BigInt(_) => "bigint",
            JsValue::String(s) => {
                // Symbols are represented as strings with "__symbol__" or
                // "__symbol_global__" prefix (for Symbol.for() values).
                if s.starts_with("__symbol_") {
                    "symbol"
                } else {
                    "string"
                }
            }
            JsValue::Object(obj) => {
                let o = obj.borrow();
                match o.internal_tag.as_deref() {
                    Some("__symbol__") => "symbol",
                    _ => "object",
                }
            }
            JsValue::Array(_) => "object",
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
            (JsValue::Number(_), JsValue::String(_)) => self.to_number() == other.to_number(),
            (JsValue::String(_), JsValue::Number(_)) => self.to_number() == other.to_number(),
            (JsValue::Bool(_), _) => JsValue::Number(self.to_number()).abstract_eq(other),
            (_, JsValue::Bool(_)) => self.abstract_eq(&JsValue::Number(other.to_number())),
            // Object identity via Rc pointer equality (same-type)
            (JsValue::Object(a), JsValue::Object(b)) => Rc::ptr_eq(a, b),
            (JsValue::Array(a), JsValue::Array(b)) => Rc::ptr_eq(a, b),
            (JsValue::Function(a), JsValue::Function(b)) => Rc::ptr_eq(a, b),
            // Object vs primitive: apply ToPrimitive via [[PrimitiveValue]] if present.
            (JsValue::Object(obj), _) => {
                if let Some(prim) = &obj.borrow().primitive_value {
                    let prim_clone = (**prim).clone();
                    prim_clone.abstract_eq(other)
                } else {
                    false
                }
            }
            (_, JsValue::Object(obj)) => {
                if let Some(prim) = &obj.borrow().primitive_value {
                    let prim_clone = (**prim).clone();
                    self.abstract_eq(&prim_clone)
                } else {
                    false
                }
            }
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
            (JsValue::BigInt(a), JsValue::BigInt(b)) => a == b,
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
                    return JsValue::Number(a.len() as f64);
                }
                if let Some(idx) = parse_index(key) {
                    if idx <= MAX_ARRAY_INDEX {
                        return a.get(idx);
                    }
                    // Indices > MAX_ARRAY_INDEX are regular properties.
                    if let Some(prop) = a.properties.get(key) {
                        return prop.value.clone();
                    }
                    return JsValue::Undefined;
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
            JsValue::Bool(_) => {
                if key == "toString" {
                    return JsValue::Function(Rc::new(RefCell::new(JsFunction {
                        name: Some(String::from("toString")),
                        params: Vec::new(),
                        kind: FnKind::Native(crate::vm::native_globals::boolean_to_string),
                        this_binding: Some(self.clone()),
                        bound_args: Vec::new(),
                        upvalues: Vec::new(),
                        prototype: None,
                        own_props: BTreeMap::new(),
                        arity: None,
                        super_class: None,
                    })));
                }
                if key == "valueOf" {
                    return JsValue::Function(Rc::new(RefCell::new(JsFunction {
                        name: Some(String::from("valueOf")),
                        params: Vec::new(),
                        kind: FnKind::Native(crate::vm::native_globals::boolean_value_of),
                        this_binding: Some(self.clone()),
                        bound_args: Vec::new(),
                        upvalues: Vec::new(),
                        prototype: None,
                        own_props: BTreeMap::new(),
                        arity: None,
                        super_class: None,
                    })));
                }
                JsValue::Undefined
            }
            JsValue::BigInt(_) => {
                // BigInt primitive auto-boxing: provide toString/valueOf.
                if key == "toString" {
                    return JsValue::Function(Rc::new(RefCell::new(JsFunction {
                        name: Some(String::from("toString")),
                        params: Vec::new(),
                        kind: FnKind::Native(bigint_to_string_native),
                        this_binding: Some(self.clone()),
                        bound_args: Vec::new(),
                        upvalues: Vec::new(),
                        prototype: None,
                        own_props: BTreeMap::new(),
                        arity: None,
                        super_class: None,
                    })));
                }
                if key == "valueOf" {
                    return JsValue::Function(Rc::new(RefCell::new(JsFunction {
                        name: Some(String::from("valueOf")),
                        params: Vec::new(),
                        kind: FnKind::Native(bigint_value_of_native),
                        this_binding: Some(self.clone()),
                        bound_args: Vec::new(),
                        upvalues: Vec::new(),
                        prototype: None,
                        own_props: BTreeMap::new(),
                        arity: None,
                        super_class: None,
                    })));
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
                    if idx <= MAX_ARRAY_INDEX {
                        a.set(idx, value);
                    } else {
                        a.properties.insert(key, Property::data(value));
                    }
                } else if key == "length" {
                    if let JsValue::Number(n) = &value {
                        let new_len = *n as usize;
                        a.set_length(new_len);
                    }
                } else {
                    a.properties.insert(key, Property::data(value));
                }
            }
            JsValue::Function(f) => {
                // ES2023 §10.2.4: name and length are non-writable, configurable.
                // Reject writes to these built-in properties (silent in sloppy mode).
                if key == "name" || key == "length" {
                    // Only allow if explicitly set via own_props already (defineProperty path)
                    if !f.borrow().own_props.contains_key(&key) {
                        return;
                    }
                }
                f.borrow_mut().own_props.insert(key, value);
            }
            _ => {} // silently ignore
        }
    }

    /// Delete a property.
    pub fn delete_property(&self, key: &str) -> bool {
        match self {
            JsValue::Object(obj) => obj.borrow_mut().delete(key),
            JsValue::Array(arr) => {
                if let Some(idx) = parse_index(key) {
                    if idx <= MAX_ARRAY_INDEX {
                        return arr.borrow_mut().delete(idx);
                    }
                }
                arr.borrow_mut().properties.remove(key);
                true
            }
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
    if negative {
        out.push('-');
    }
    out.push_str(&coeff_str);
    out.push('e');
    if exp >= 0 {
        out.push('+');
    }
    out.push_str(&format_i64(exp as i64));
    out
}

// ── BigInt primitive method natives ──

fn bigint_to_string_native(vm: &mut crate::vm::Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let radix = args.first().map(|v| v.to_number() as u32).unwrap_or(10);
    if let JsValue::BigInt(bi) = &this {
        JsValue::String(bi.to_string_radix(if radix >= 2 && radix <= 36 { radix } else { 10 }))
    } else {
        JsValue::String(String::from("0"))
    }
}

fn bigint_value_of_native(vm: &mut crate::vm::Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}
