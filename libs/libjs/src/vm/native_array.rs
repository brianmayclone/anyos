//! Array.prototype methods and Array static methods.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::native_proxy;
use super::native_symbol;
use super::Vm;
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// Helper: extract array elements from `this`
// ═══════════════════════════════════════════════════════════

fn this_array(vm: &Vm) -> Option<Rc<RefCell<JsArray>>> {
    match &vm.current_this {
        JsValue::Array(a) => Some(a.clone()),
        _ => None,
    }
}

/// Resolve a possibly-negative index against a length.
fn resolve_index(idx: f64, len: usize) -> usize {
    let int = if idx.is_nan() {
        0.0
    } else if idx.is_infinite() {
        idx
    } else {
        (idx as i64) as f64
    };
    if int < 0.0 {
        let r = len as f64 + int;
        if r < 0.0 {
            0
        } else {
            r as usize
        }
    } else {
        (int as usize).min(len)
    }
}

const MAX_SAFE_INTEGER_LEN: usize = 9_007_199_254_740_991;
const SPARSE_INDEX_SCAN_THRESHOLD: usize = 4096;

/// ToNumber with VM — calls valueOf/toString on Objects per ES2023 §7.1.4.
/// Throws TypeError if ToPrimitive fails (both valueOf and toString return objects).
pub fn to_number_vm(vm: &mut Vm, val: &JsValue) -> f64 {
    match val {
        JsValue::Empty => f64::NAN,
        JsValue::Number(n) => *n,
        JsValue::String(s) => {
            if s.starts_with("__symbol_") {
                let err = vm.make_type_error("Cannot convert a Symbol value to a number");
                vm.throw_native(err);
                return f64::NAN;
            }
            crate::value::parse_js_float(s)
        }
        JsValue::Bool(true) => 1.0,
        JsValue::Bool(false) => 0.0,
        JsValue::Null => 0.0,
        JsValue::Undefined => f64::NAN,
        JsValue::Object(obj) => {
            let o = obj.borrow();
            if o.internal_tag.as_deref() == Some("__symbol__") {
                drop(o);
                let err = vm.make_type_error("Cannot convert a Symbol value to a number");
                vm.throw_native(err);
                return f64::NAN;
            }
            if let Some(prim) = &o.primitive_value {
                let p = prim.clone();
                drop(o);
                return to_number_vm(vm, &p);
            }
            // ToPrimitive: try valueOf first, then toString
            let value_of = o.get("valueOf");
            let to_string_fn = o.get("toString");
            drop(o);
            let mut tried_valueof = false;
            let mut tried_tostring = false;
            if let JsValue::Function(_) = &value_of {
                tried_valueof = true;
                let result = vm.call_value(&value_of, &[], val.clone());
                if let Some(exc) = vm.last_exception.take() {
                    vm.pending_exception = Some(exc);
                    return f64::NAN;
                }
                if vm.pending_exception.is_some() {
                    return f64::NAN;
                }
                // If result is a primitive, convert to number
                match &result {
                    JsValue::Number(n) => return *n,
                    JsValue::String(s) => return crate::value::parse_js_float(s),
                    JsValue::Bool(true) => return 1.0,
                    JsValue::Bool(false) => return 0.0,
                    JsValue::Null => return 0.0,
                    JsValue::Undefined => return f64::NAN,
                    _ => {} // Non-primitive: fall through to toString
                }
            }
            if let JsValue::Function(_) = &to_string_fn {
                tried_tostring = true;
                let result = vm.call_value(&to_string_fn, &[], val.clone());
                if let Some(exc) = vm.last_exception.take() {
                    vm.pending_exception = Some(exc);
                    return f64::NAN;
                }
                if vm.pending_exception.is_some() {
                    return f64::NAN;
                }
                match &result {
                    JsValue::String(s) => return crate::value::parse_js_float(s),
                    JsValue::Number(n) => return *n,
                    JsValue::Bool(true) => return 1.0,
                    JsValue::Bool(false) => return 0.0,
                    JsValue::Null => return 0.0,
                    JsValue::Undefined => return f64::NAN,
                    _ => {} // Non-primitive: TypeError
                }
            }
            // OrdinaryToPrimitive failed to produce a primitive.
            let err = vm.make_type_error("Cannot convert object to primitive value");
            vm.throw_native(err);
            f64::NAN
        }
        JsValue::Array(_) | JsValue::Function(_) => f64::NAN,
        JsValue::BigInt(bi) => bi.to_f64(),
    }
}

/// Convert a JsValue to a length (ToLength), using to_number_vm.
fn to_length_vm(vm: &mut Vm, val: &JsValue) -> usize {
    let n = to_number_vm(vm, val);
    if n.is_nan() || n < 0.0 {
        0
    } else if !n.is_finite() {
        MAX_SAFE_INTEGER_LEN
    }
    else {
        (n as u64).min(MAX_SAFE_INTEGER_LEN as u64) as usize
    }
}

fn coerce_array_like_this(vm: &mut Vm) -> Option<JsValue> {
    match &vm.current_this {
        JsValue::Null | JsValue::Undefined => {
            let err = vm.make_type_error("Cannot convert undefined or null to object");
            vm.throw_native(err);
            None
        }
        JsValue::Bool(_) | JsValue::Number(_) | JsValue::String(_) | JsValue::BigInt(_) => {
            Some(wrap_primitive_for_concat(vm, &vm.current_this.clone()))
        }
        _ => Some(vm.current_this.clone()),
    }
}

fn array_like_length(vm: &mut Vm, this_obj: &JsValue) -> Option<usize> {
    let len_val = vm.get_property_invoking_getter(this_obj, "length");
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
    }
    if vm.pending_exception.is_some() {
        return None;
    }
    Some(to_length_vm(vm, &len_val))
}

fn array_like_has_index(vm: &mut Vm, this_obj: &JsValue, idx: usize) -> Option<bool> {
    has_concat_property(vm, this_obj, &alloc::format!("{}", idx))
}

pub(crate) fn array_effective_proto_value(vm: &Vm, arr: &JsArray) -> JsValue {
    match arr.properties.get("__proto__") {
        Some(prop) => prop.value.clone(),
        None => JsValue::Object(vm.array_proto.clone()),
    }
}


/// Coerce `this` to an object and read its `length` once.
fn this_array_like_len(vm: &mut Vm) -> Option<(JsValue, usize)> {
    let this_obj = coerce_array_like_this(vm)?;
    let len = array_like_length(vm, &this_obj)?;
    Some((this_obj, len))
}

/// Snapshot present array-like entries from `this`.
fn this_array_like_entries(vm: &mut Vm) -> Option<(JsValue, usize, Vec<(usize, JsValue)>)> {
    let (this_obj, len) = this_array_like_len(vm)?;
    let mut entries = Vec::new();
    if len > SPARSE_INDEX_SCAN_THRESHOLD {
        let mut indices = Vec::new();
        collect_numeric_keys_from_value(vm, &this_obj, len, &mut indices);
        indices.sort_unstable();
        indices.dedup();
        for idx in indices {
            let val = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
            if let Some(exc) = vm.last_exception.take() {
                vm.pending_exception = Some(exc);
            }
            if vm.pending_exception.is_some() {
                return None;
            }
            entries.push((idx, val));
        }
        return Some((this_obj, len, entries));
    }
    for idx in 0..len {
        if !array_like_has_index(vm, &this_obj, idx)? {
            continue;
        }
        let val = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return None;
        }
        entries.push((idx, val));
    }
    Some((this_obj, len, entries))
}

/// Validate that a callback is callable. Returns false and throws TypeError if not.
fn require_callable(vm: &mut Vm, callback: &JsValue) -> bool {
    if matches!(callback, JsValue::Function(_)) {
        true
    } else {
        let msg = alloc::format!("{} is not a function", callback.to_js_string());
        let err = vm.make_type_error(&msg);
        vm.throw_native(err);
        false
    }
}

/// Check if `this` is null/undefined and throw TypeError if so.
/// Returns true if the check passed (this is valid), false if TypeError was thrown.
fn require_object_coercible(vm: &mut Vm) -> bool {
    if matches!(&vm.current_this, JsValue::Null | JsValue::Undefined) {
        let err = vm.make_type_error("Cannot convert undefined or null to object");
        vm.throw_native(err);
        false
    } else {
        true
    }
}

fn is_string_receiver(value: &JsValue) -> bool {
    match value {
        JsValue::String(_) => true,
        JsValue::Object(obj) => matches!(
            obj.borrow().primitive_value.as_deref(),
            Some(JsValue::String(_))
        ),
        _ => false,
    }
}

fn array_like_max_length_for(value: &JsValue) -> usize {
    if matches!(value, JsValue::Array(_)) {
        0xFFFF_FFFF
    } else {
        MAX_SAFE_INTEGER_LEN
    }
}

fn set_array_like_length_or_throw(vm: &mut Vm, this_obj: &JsValue, new_len: usize) -> bool {
    if !vm.set_property_or_throw(this_obj, "length", JsValue::Number(new_len as f64)) {
        return false;
    }
    match this_obj {
        JsValue::Array(arr) => {
            arr.borrow_mut().length = new_len;
        }
        JsValue::Object(obj) => {
            let mut o = obj.borrow_mut();
            if let Some(prop) = o.properties.get_mut("length") {
                prop.value = JsValue::Number(new_len as f64);
            }
        }
        _ => {}
    }
    true
}

// ═══════════════════════════════════════════════════════════
// Mutating methods
// ═══════════════════════════════════════════════════════════

pub fn array_push(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    let Some(this_obj) = coerce_array_like_this(vm) else {
        return JsValue::Undefined;
    };
    let Some(current_len) = array_like_length(vm, &this_obj) else {
        return JsValue::Undefined;
    };
    if !matches!(this_obj, JsValue::Array(_)) {
        let max_len = array_like_max_length_for(&this_obj);
        if current_len > max_len.saturating_sub(args.len()) {
            let err = vm.make_type_error("Invalid array length");
            vm.throw_native(err);
            return JsValue::Undefined;
        }
    }
    let mut new_len = current_len;
    for arg in args {
        if !vm.set_property_or_throw(&this_obj, &alloc::format!("{}", new_len), arg.clone()) {
            return JsValue::Undefined;
        }
        new_len += 1;
    }
    if matches!(this_obj, JsValue::Array(_)) && new_len > 0xFFFF_FFFF {
        let err = vm.make_range_error("Invalid array length");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    if !set_array_like_length_or_throw(vm, &this_obj, new_len) {
        return JsValue::Undefined;
    }
    JsValue::Number(new_len as f64)
}

pub fn array_pop(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    let Some(this_obj) = coerce_array_like_this(vm) else {
        return JsValue::Undefined;
    };
    let Some(len) = array_like_length(vm, &this_obj) else {
        return JsValue::Undefined;
    };
    if len == 0 {
        if !set_array_like_length_or_throw(vm, &this_obj, 0) {
            return JsValue::Undefined;
        }
        return JsValue::Undefined;
    }
    let index = len - 1;
    let elem = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", index));
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
    }
    if vm.pending_exception.is_some() {
        return JsValue::Undefined;
    }
    if !vm.delete_property_or_throw(&this_obj, &alloc::format!("{}", index)) {
        return JsValue::Undefined;
    }
    if !set_array_like_length_or_throw(vm, &this_obj, index) {
        return JsValue::Undefined;
    }
    elem
}

pub fn array_shift(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    let Some(this_obj) = coerce_array_like_this(vm) else {
        return JsValue::Undefined;
    };
    let Some(len) = array_like_length(vm, &this_obj) else {
        return JsValue::Undefined;
    };
    if len == 0 {
        if !set_array_like_length_or_throw(vm, &this_obj, 0) {
            return JsValue::Undefined;
        }
        return JsValue::Undefined;
    }
    let first = vm.get_property_invoking_getter(&this_obj, "0");
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
    }
    if vm.pending_exception.is_some() {
        return JsValue::Undefined;
    }
    for k in 1..len {
        let from_key = alloc::format!("{}", k);
        let to_key = alloc::format!("{}", k - 1);
        let from_present = match array_like_has_index(vm, &this_obj, k) {
            Some(v) => v,
            None => return JsValue::Undefined,
        };
        if from_present {
            let from_val = vm.get_property_invoking_getter(&this_obj, &from_key);
            if let Some(exc) = vm.last_exception.take() {
                vm.pending_exception = Some(exc);
            }
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            if !vm.set_property_or_throw(&this_obj, &to_key, from_val) {
                return JsValue::Undefined;
            }
        } else if !vm.delete_property_or_throw(&this_obj, &to_key) {
            return JsValue::Undefined;
        }
    }
    if !vm.delete_property_or_throw(&this_obj, &alloc::format!("{}", len - 1)) {
        return JsValue::Undefined;
    }
    if !set_array_like_length_or_throw(vm, &this_obj, len - 1) {
        return JsValue::Undefined;
    }
    first
}

pub fn array_unshift(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    let Some(this_obj) = coerce_array_like_this(vm) else {
        return JsValue::Undefined;
    };
    let Some(len) = array_like_length(vm, &this_obj) else {
        return JsValue::Undefined;
    };
    let max_len = array_like_max_length_for(&this_obj);
    if len > max_len.saturating_sub(args.len()) {
        let err = if matches!(this_obj, JsValue::Array(_)) {
            vm.make_range_error("Invalid array length")
        } else {
            vm.make_type_error("Invalid array length")
        };
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let arg_count = args.len();
    if arg_count > 0 {
        for k in (0..len).rev() {
            let from_key = alloc::format!("{}", k);
            let to_key = alloc::format!("{}", k + arg_count);
            let from_present = match array_like_has_index(vm, &this_obj, k) {
                Some(v) => v,
                None => return JsValue::Undefined,
            };
            if from_present {
                let from_val = vm.get_property_invoking_getter(&this_obj, &from_key);
                if let Some(exc) = vm.last_exception.take() {
                    vm.pending_exception = Some(exc);
                }
                if vm.pending_exception.is_some() {
                    return JsValue::Undefined;
                }
                if !vm.set_property_or_throw(&this_obj, &to_key, from_val) {
                    return JsValue::Undefined;
                }
            } else if !vm.delete_property_or_throw(&this_obj, &to_key) {
                return JsValue::Undefined;
            }
        }
        for (j, item) in args.iter().enumerate() {
            if !vm.set_property_or_throw(&this_obj, &alloc::format!("{}", j), item.clone()) {
                return JsValue::Undefined;
            }
        }
    }
    let new_len = len + arg_count;
    if !set_array_like_length_or_throw(vm, &this_obj, new_len) {
        return JsValue::Undefined;
    }
    JsValue::Number(new_len as f64)
}

pub fn array_splice(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        let len = a.length;
        let start_raw = args.first().map(|v| v.to_number()).unwrap_or(0.0);
        let start = resolve_index(start_raw, len);
        let delete_count = if args.len() > 1 {
            let dc = args[1].to_number() as usize;
            dc.min(len.saturating_sub(start))
        } else {
            len.saturating_sub(start)
        };

        // Collect removed elements.
        let mut removed = Vec::new();
        for i in start..start + delete_count {
            removed.push(a.elements.remove(&i).unwrap_or(JsValue::Undefined));
        }

        let insert_items = if args.len() > 2 { &args[2..] } else { &[] };
        let diff = insert_items.len() as isize - delete_count as isize;

        if diff != 0 {
            // Shift elements after the deleted range.
            let after_start = start + delete_count;
            let entries_after: Vec<(usize, JsValue)> = a
                .elements
                .range(after_start..)
                .map(|(&k, v)| (k, v.clone()))
                .collect();
            for (k, _) in &entries_after {
                a.elements.remove(k);
            }
            for (k, v) in entries_after {
                let new_k = (k as isize + diff) as usize;
                a.elements.insert(new_k, v);
            }
            a.length = (len as isize + diff) as usize;
        }

        // Insert new elements.
        for (i, item) in insert_items.iter().enumerate() {
            a.elements.insert(start + i, item.clone());
        }

        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(removed))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_reverse(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    if let Some(arr) = this_array(vm) {
        arr.borrow_mut().reverse();
        JsValue::Array(arr)
    } else {
        JsValue::Undefined
    }
}

pub fn array_sort(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    let comparefn = args.first().cloned();
    if let Some(arr) = this_array(vm) {
        // Extract set elements as (index, value), sort values, re-assign to
        // the same index positions (preserving sparseness structure).
        let mut values: Vec<JsValue> = {
            let a = arr.borrow();
            a.elements.values().cloned().collect()
        };

        if let Some(cmp) = &comparefn {
            if matches!(cmp, JsValue::Function(_)) {
                let cmp = cmp.clone();
                let len = values.len();
                for i in 0..len {
                    for j in 0..len.saturating_sub(1 + i) {
                        let result =
                            call_callback(vm, &cmp, &[values[j].clone(), values[j + 1].clone()]);
                        if result.to_number() > 0.0 {
                            values.swap(j, j + 1);
                        }
                    }
                }
            }
        } else {
            values.sort_by(|a, b| {
                let sa = a.to_js_string();
                let sb = b.to_js_string();
                sa.cmp(&sb)
            });
        }

        // Put sorted values back into the original index positions.
        {
            let mut a = arr.borrow_mut();
            let keys: Vec<usize> = a.elements.keys().cloned().collect();
            a.elements.clear();
            for (i, k) in keys.into_iter().enumerate() {
                if i < values.len() {
                    a.elements.insert(k, values[i].clone());
                }
            }
        }
        JsValue::Array(arr)
    } else {
        JsValue::Undefined
    }
}

pub fn array_fill(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    let this_obj = match coerce_array_like_this(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let len = match array_like_length(vm, &this_obj) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let start_num = match args.get(1) {
        Some(v) => to_number_vm(vm, v),
        None => 0.0,
    };
    if vm.pending_exception.is_some() {
        JsValue::Undefined
    } else {
        let end_num = match args.get(2) {
            None | Some(JsValue::Undefined) => len as f64,
            Some(v) => to_number_vm(vm, v),
        };
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        let start = resolve_index(start_num, len);
        let end = resolve_index(end_num, len);
        for i in start..end {
            // §22.1.3.6 step 11.b: Set(O, Pk, value, true) — invokes setters and
            // must propagate any exception they throw.
            if !vm.set_property_or_throw(&this_obj, &alloc::format!("{}", i), value.clone()) {
                return JsValue::Undefined;
            }
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
        }
        this_obj
    }
}

pub fn array_copy_within(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    let this_obj = match coerce_array_like_this(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let len = match array_like_length(vm, &this_obj) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let target_num = match args.first() {
        Some(v) => to_number_vm(vm, v),
        None => 0.0,
    };
    let start_num = match args.get(1) {
        Some(v) => to_number_vm(vm, v),
        None => 0.0,
    };
    if vm.pending_exception.is_some() {
        JsValue::Undefined
    } else {
        let end_num = match args.get(2) {
            None | Some(JsValue::Undefined) => len as f64,
            Some(v) => to_number_vm(vm, v),
        };
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        let target = resolve_index(target_num, len);
        let start = resolve_index(start_num, len);
        let end = resolve_index(end_num, len);
        let count = end.saturating_sub(start).min(len.saturating_sub(target));
        let descending = start < target && target < start + count;
        if descending {
            for offset in (0..count).rev() {
                let from_idx = start + offset;
                let to_idx = target + offset;
                if array_like_has_index(vm, &this_obj, from_idx).unwrap_or(false) {
                    let value =
                        vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", from_idx));
                    if let Some(exc) = vm.last_exception.take() {
                        vm.pending_exception = Some(exc);
                    }
                    if vm.pending_exception.is_some() {
                        return JsValue::Undefined;
                    }
                    if !vm.set_property_or_throw(&this_obj, &alloc::format!("{}", to_idx), value) {
                        return JsValue::Undefined;
                    }
                } else {
                    if !vm.delete_property_or_throw(&this_obj, &alloc::format!("{}", to_idx)) {
                        return JsValue::Undefined;
                    }
                }
            }
        } else {
            for offset in 0..count {
                let from_idx = start + offset;
                let to_idx = target + offset;
                if array_like_has_index(vm, &this_obj, from_idx).unwrap_or(false) {
                    let value =
                        vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", from_idx));
                    if let Some(exc) = vm.last_exception.take() {
                        vm.pending_exception = Some(exc);
                    }
                    if vm.pending_exception.is_some() {
                        return JsValue::Undefined;
                    }
                    if !vm.set_property_or_throw(&this_obj, &alloc::format!("{}", to_idx), value) {
                        return JsValue::Undefined;
                    }
                } else {
                    if !vm.delete_property_or_throw(&this_obj, &alloc::format!("{}", to_idx)) {
                        return JsValue::Undefined;
                    }
                }
            }
        }
        this_obj
    }
}

// ═══════════════════════════════════════════════════════════
// Non-mutating / accessor methods
// ═══════════════════════════════════════════════════════════

pub fn array_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let (_this_obj, len, entries) = match this_array_like_entries(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if len == 0 {
        return JsValue::Number(-1.0);
    }
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    let from_raw = args.get(1).map(|v| v.to_number()).unwrap_or(0.0);
    if from_raw.is_infinite() && from_raw > 0.0 {
        return JsValue::Number(-1.0);
    }
    let from = if from_raw < 0.0 {
        let r = len as f64 + from_raw;
        if r < 0.0 {
            0usize
        } else {
            r as usize
        }
    } else {
        (from_raw as usize).min(len)
    };
    for (idx, val) in &entries {
        if *idx >= from && val.strict_eq(&search) {
            return JsValue::Number(*idx as f64);
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_last_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let (_this_obj, len, entries) = match this_array_like_entries(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if len == 0 {
        return JsValue::Number(-1.0);
    }
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    let from_raw = args
        .get(1)
        .map(|v| v.to_number())
        .unwrap_or(len as f64 - 1.0);
    let from = if from_raw < 0.0 {
        let r = len as f64 + from_raw;
        if r < 0.0 {
            return JsValue::Number(-1.0);
        }
        r as usize
    } else if from_raw.is_infinite() || from_raw >= len as f64 {
        len - 1
    } else {
        from_raw as usize
    };
    for (idx, val) in entries.iter().rev() {
        if *idx <= from && val.strict_eq(&search) {
            return JsValue::Number(*idx as f64);
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_includes(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let (_this_obj, len, entries) = match this_array_like_entries(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if len == 0 {
        return JsValue::Bool(false);
    }
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    // §22.1.3.13 step 4: ToInteger(fromIndex). Must invoke valueOf/Symbol.toPrimitive
    // and propagate exceptions.
    let from = match args.get(1) {
        None | Some(JsValue::Undefined) => 0,
        Some(v) => {
            let n = to_number_vm(vm, v);
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            resolve_index(n, len)
        }
    };
    for (idx, val) in &entries {
        if *idx < from {
            continue;
        }
        if val.strict_eq(&search) {
            return JsValue::Bool(true);
        }
        if let (JsValue::Number(a_n), JsValue::Number(s_n)) = (val, &search) {
            if a_n.is_nan() && s_n.is_nan() {
                return JsValue::Bool(true);
            }
        }
    }
    JsValue::Bool(false)
}

pub fn array_join(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let sep = match args.first() {
            Some(JsValue::Undefined) | None => String::from(","),
            Some(v) => v.to_js_string(),
        };
        let len = a.length;
        let mut out = String::new();
        for i in 0..len {
            if i > 0 {
                out.push_str(&sep);
            }
            if let Some(el) = a.elements.get(&i) {
                match el {
                    JsValue::Undefined | JsValue::Null => {}
                    _ => out.push_str(&el.to_js_string()),
                }
            }
        }
        JsValue::String(out)
    } else {
        JsValue::String(String::new())
    }
}

pub fn array_slice(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let (this_obj, len, entries) = match this_array_like_entries(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let start_num = match args.first() {
        Some(v) => to_number_vm(vm, v),
        None => 0.0,
    };
    if vm.pending_exception.is_some() {
        return JsValue::Undefined;
    }
    let end_num = match args.get(1) {
        None | Some(JsValue::Undefined) => len as f64,
        Some(v) => to_number_vm(vm, v),
    };
    if vm.pending_exception.is_some() {
        return JsValue::Undefined;
    }
    let start = resolve_index(start_num, len);
    let end = resolve_index(end_num, len);
    let result = match array_species_create(vm, &this_obj, end.saturating_sub(start)) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if start < end {
        for (idx, val) in entries {
            if idx >= start && idx < end {
                if !concat_define_result_index(vm, &result, idx - start, val) {
                    return JsValue::Undefined;
                }
            }
        }
    }
    concat_set_result_length(&result, end.saturating_sub(start));
    result
}

fn observe_array_species_create(vm: &mut Vm, original: &JsValue) -> bool {
    let ctor = vm.get_property_invoking_getter(original, "constructor");
    if vm.pending_exception.is_some() {
        return false;
    }
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
        return false;
    }
    if matches!(ctor, JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)) {
        let _species = vm.get_property_invoking_getter(&ctor, native_symbol::WELL_KNOWN_SPECIES);
        if vm.pending_exception.is_some() {
            return false;
        }
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
            return false;
        }
    }
    true
}

fn is_array_value(vm: &mut Vm, value: &JsValue) -> Option<bool> {
    match value {
        JsValue::Array(_) => Some(true),
        JsValue::Object(obj) => {
            if obj.borrow().internal_tag.as_deref() == Some(native_proxy::PROXY_TAG) {
                return match native_proxy::proxy_target(value) {
                    Some(target) => is_array_value(vm, &target),
                    None => {
                        let err = vm.make_type_error("Cannot perform 'IsArray' on a revoked Proxy");
                        vm.throw_native(err);
                        None
                    }
                };
            }
            Some(false)
        }
        _ => Some(false),
    }
}

fn array_species_create(vm: &mut Vm, original: &JsValue, length: usize) -> Option<JsValue> {
    let is_array = is_array_value(vm, original)?;
    if !is_array {
        let mut arr = JsArray::new();
        arr.length = length;
        return Some(JsValue::Array(Rc::new(RefCell::new(arr))));
    }

    let ctor = vm.get_property_invoking_getter(original, "constructor");
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
    }
    if vm.pending_exception.is_some() {
        return None;
    }

    let species = match ctor {
        JsValue::Undefined => JsValue::Undefined,
        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => {
            let species = vm.get_property_invoking_getter(&ctor, native_symbol::WELL_KNOWN_SPECIES);
            if let Some(exc) = vm.last_exception.take() {
                vm.pending_exception = Some(exc);
            }
            if vm.pending_exception.is_some() {
                return None;
            }
            species
        }
        _ => {
            let err = vm.make_type_error("Array constructor is not an object");
            vm.throw_native(err);
            return None;
        }
    };

    match species {
        JsValue::Undefined | JsValue::Null => {
            let mut arr = JsArray::new();
            arr.length = length;
            Some(JsValue::Array(Rc::new(RefCell::new(arr))))
        }
        _ => {
            let Some(result) =
                vm.construct_value(&species, &[JsValue::Number(length as f64)], &species)
            else {
                let err = vm.make_type_error("Array species is not a constructor");
                vm.throw_native(err);
                return None;
            };
            Some(result)
        }
    }
}

fn wrap_primitive_for_concat(vm: &Vm, value: &JsValue) -> JsValue {
    let mut obj = JsObject::new();
    match value {
        JsValue::Bool(b) => {
            obj.prototype = Some(vm.boolean_proto.clone());
            obj.internal_tag = Some(String::from("__boolean__"));
            obj.primitive_value = Some(Box::new(JsValue::Bool(*b)));
            obj.set(String::from("__bool_data__"), JsValue::Bool(*b));
        }
        JsValue::Number(n) => {
            obj.prototype = Some(vm.number_proto.clone());
            obj.internal_tag = Some(String::from("__number__"));
            obj.primitive_value = Some(Box::new(JsValue::Number(*n)));
        }
        JsValue::String(s) => {
            obj.prototype = Some(vm.string_proto.clone());
            obj.internal_tag = Some(String::from("__string__"));
            obj.primitive_value = Some(Box::new(JsValue::String(s.clone())));
        }
        JsValue::BigInt(bi) => {
            obj.prototype = Some(vm.object_proto.clone());
            obj.primitive_value = Some(Box::new(JsValue::BigInt(bi.clone())));
        }
        _ => return value.clone(),
    }
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn has_concat_property(vm: &mut Vm, value: &JsValue, key: &str) -> Option<bool> {
    match value {
        JsValue::Array(arr) => {
            if let Some(idx) = super::try_parse_index(key) {
                let arr = arr.borrow();
                if arr.has(idx) || arr.properties.contains_key(key) {
                    return Some(true);
                }
                let proto = array_effective_proto_value(vm, &arr);
                return Some(!proto.is_null() && !vm.get_property_with_proto(&proto, key).is_undefined());
            }
            Some(
                key == "length"
                || arr.borrow().properties.contains_key(key)
                || vm.array_proto.borrow().has(key),
            )
        }
        JsValue::Object(obj) if obj.borrow().primitive_value.is_some() => {
            let o = obj.borrow();
            if let Some(prim) = &o.primitive_value {
                if let JsValue::String(s) = prim.as_ref() {
                    return Some(
                        key == "length"
                        || super::try_parse_index(key).is_some_and(|idx| idx < s.chars().count())
                        || o.has(key),
                    );
                }
            }
            Some(o.has(key))
        }
        JsValue::Object(obj) => {
            if let Some(idx) = super::try_parse_index(key) {
                let borrowed = obj.borrow();
                if borrowed.internal_tag.as_deref()
                    == Some(crate::vm::native_typed_array::TYPED_ARRAY_TAG)
                {
                    return Some(
                        crate::vm::native_typed_array::typed_array_get_index(&borrowed, idx)
                            .is_some(),
                    );
                }
            }
            if obj.borrow().internal_tag.as_deref() == Some(native_proxy::PROXY_TAG) {
                return vm.has_property(value, key);
            }
            Some(obj.borrow().has(key))
        }
        JsValue::Function(f) => {
            let func = f.borrow();
            Some(func.own_props.contains_key(key) || vm.function_proto.borrow().has(key))
        }
        _ => Some(false),
    }
}

fn is_concat_spreadable(vm: &mut Vm, value: &JsValue) -> Option<bool> {
    match value {
        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => {
            let spreadable =
                vm.get_property_invoking_getter(value, native_symbol::WELL_KNOWN_IS_CONCAT_SPREADABLE);
            if vm.pending_exception.is_some() {
                return None;
            }
            if let Some(exc) = vm.last_exception.take() {
                vm.pending_exception = Some(exc);
                return None;
            }
            if !matches!(spreadable, JsValue::Undefined) {
                return Some(spreadable.to_boolean());
            }
            if matches!(value, JsValue::Object(obj) if obj.borrow().internal_tag.as_deref() == Some(native_proxy::PROXY_TAG))
            {
                if let Some(target) = native_proxy::proxy_target(value) {
                    return is_array_value(vm, &target);
                }
                let err = vm.make_type_error("Cannot perform 'IsArray' on a revoked Proxy");
                vm.throw_native(err);
                return None;
            }
            Some(matches!(value, JsValue::Array(_)))
        }
        _ => Some(false),
    }
}

fn array_species_create_concat(vm: &mut Vm, original: &JsValue) -> Option<JsValue> {
    if !matches!(original, JsValue::Array(_)) {
        return Some(JsValue::Array(Rc::new(RefCell::new(JsArray::new()))));
    }
    let ctor = vm.get_property_invoking_getter(original, "constructor");
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
    }
    if vm.pending_exception.is_some() {
        return None;
    }
    let species = match ctor {
        JsValue::Undefined => JsValue::Undefined,
        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => {
            let species = vm.get_property_invoking_getter(&ctor, native_symbol::WELL_KNOWN_SPECIES);
            if let Some(exc) = vm.last_exception.take() {
                vm.pending_exception = Some(exc);
            }
            if vm.pending_exception.is_some() {
                return None;
            }
            species
        }
        _ => {
            let err = vm.make_type_error("Array constructor is not an object");
            vm.throw_native(err);
            return None;
        }
    };
    match species {
        JsValue::Undefined | JsValue::Null => {
            Some(JsValue::Array(Rc::new(RefCell::new(JsArray::new()))))
        }
        _ => {
            if !vm.construct_with_new_target(&species, &[JsValue::Number(0.0)], &species) {
                let err = vm.make_type_error("Array species is not a constructor");
                vm.throw_native(err);
                return None;
            }
            Some(vm.stack.pop().unwrap_or(JsValue::Undefined))
        }
    }
}

fn concat_define_result_index(vm: &mut Vm, target: &JsValue, index: usize, value: JsValue) -> bool {
    let key = alloc::format!("{}", index);
    match target {
        JsValue::Array(arr) => {
            let mut arr = arr.borrow_mut();
            let non_extensible = arr.properties.contains_key("__non_extensible__");
            if non_extensible && !arr.has(index) {
                let err = vm.make_type_error("Cannot define property on non-extensible array");
                vm.throw_native(err);
                return false;
            }
            arr.set(index, value);
            true
        }
        JsValue::Object(obj) => {
            let mut obj = obj.borrow_mut();
            let non_extensible = obj.properties.contains_key("__non_extensible__");
            if let Some(existing) = obj.properties.get(&key) {
                if !existing.configurable {
                    let err = vm.make_type_error("Cannot define property on target object");
                    vm.throw_native(err);
                    return false;
                }
            } else if non_extensible {
                let err = vm.make_type_error("Cannot define property on non-extensible object");
                vm.throw_native(err);
                return false;
            }
            obj.properties.insert(key, Property::data(value));
            true
        }
        JsValue::Function(f) => {
            let mut func = f.borrow_mut();
            if !func.own_props.contains_key(&key)
                && matches!(func.own_props.get("__non_extensible__"), Some(JsValue::Bool(true)))
            {
                let err = vm.make_type_error("Cannot define property on non-extensible object");
                vm.throw_native(err);
                return false;
            }
            func.own_props.insert(key, value);
            true
        }
        _ => {
            let err = vm.make_type_error("Concat result is not an object");
            vm.throw_native(err);
            false
        }
    }
}

fn concat_set_result_length(target: &JsValue, length: usize) {
    match target {
        JsValue::Array(arr) => arr.borrow_mut().length = length,
        JsValue::Object(obj) => {
            obj.borrow_mut()
                .properties
                .insert(String::from("length"), Property::data(JsValue::Number(length as f64)));
        }
        JsValue::Function(f) => {
            f.borrow_mut()
                .own_props
                .insert(String::from("length"), JsValue::Number(length as f64));
        }
        _ => {}
    }
}

fn collect_numeric_keys_from_object(
    obj: &Rc<RefCell<JsObject>>,
    len: usize,
    out: &mut Vec<usize>,
) {
    let o = obj.borrow();
    for key in o.properties.keys() {
        if let Some(idx) = super::try_parse_index(key) {
            if idx < len {
                out.push(idx);
            }
        }
    }
    let proto = o.prototype.clone();
    drop(o);
    if let Some(proto) = proto {
        collect_numeric_keys_from_object(&proto, len, out);
    }
}

fn collect_numeric_keys_from_value(vm: &mut Vm, value: &JsValue, len: usize, out: &mut Vec<usize>) {
    match value {
        JsValue::Array(arr) => {
            let a = arr.borrow();
            for idx in a.elements.keys() {
                if *idx < len {
                    out.push(*idx);
                }
            }
            for key in a.properties.keys() {
                if let Some(idx) = super::try_parse_index(key) {
                    if idx < len {
                        out.push(idx);
                    }
                }
            }
            drop(a);
            collect_numeric_keys_from_object(&vm.array_proto, len, out);
        }
        JsValue::Object(obj) => {
            if obj.borrow().internal_tag.as_deref()
                == Some(crate::vm::native_typed_array::TYPED_ARRAY_TAG)
            {
                let borrowed = obj.borrow();
                for idx in 0..len {
                    if crate::vm::native_typed_array::typed_array_get_index(&borrowed, idx)
                        .is_some()
                    {
                        out.push(idx);
                    }
                }
                return;
            }
            if obj.borrow().internal_tag.as_deref() == Some(native_proxy::PROXY_TAG) {
                if let Some(keys) = native_proxy::proxy_own_keys(vm, value) {
                    for key in keys {
                        if let Some(idx) = super::try_parse_index(&key) {
                            if idx < len {
                                out.push(idx);
                            }
                        }
                    }
                }
            } else {
                collect_numeric_keys_from_object(obj, len, out);
            }
        }
        JsValue::Function(f) => {
            let func = f.borrow();
            for key in func.own_props.keys() {
                if let Some(idx) = super::try_parse_index(key) {
                    if idx < len {
                        out.push(idx);
                    }
                }
            }
            drop(func);
            collect_numeric_keys_from_object(&vm.function_proto, len, out);
        }
        _ => {}
    }
}

fn concat_sparse_indices(vm: &mut Vm, value: &JsValue, len: usize) -> Option<Vec<usize>> {
    if len <= SPARSE_INDEX_SCAN_THRESHOLD {
        return None;
    }
    let mut indices = Vec::new();
    collect_numeric_keys_from_value(vm, value, len, &mut indices);
    indices.sort_unstable();
    indices.dedup();
    Some(indices)
}

fn concat_append_spread(vm: &mut Vm, result: &JsValue, next_index: &mut usize, value: &JsValue) -> bool {
    let len_val = vm.get_property_invoking_getter(value, "length");
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
    }
    if vm.pending_exception.is_some() {
        return false;
    }
    let len = to_length_vm(vm, &len_val);
    if vm.pending_exception.is_some() {
        return false;
    }
    if *next_index > MAX_SAFE_INTEGER_LEN.saturating_sub(len) {
        let err = vm.make_type_error("Invalid array length");
        vm.throw_native(err);
        return false;
    }
    if let Some(indices) = concat_sparse_indices(vm, value, len) {
        for idx in indices {
            let key = alloc::format!("{}", idx);
            let entry = vm.get_property_invoking_getter(value, &key);
            if let Some(exc) = vm.last_exception.take() {
                vm.pending_exception = Some(exc);
            }
            if vm.pending_exception.is_some() {
                return false;
            }
            if !concat_define_result_index(vm, result, *next_index + idx, entry) {
                return false;
            }
        }
        *next_index += len;
        return true;
    }
    for idx in 0..len {
        let key = alloc::format!("{}", idx);
        if !has_concat_property(vm, value, &key).unwrap_or(false) {
            *next_index += 1;
            continue;
        }
        let entry = vm.get_property_invoking_getter(value, &key);
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return false;
        }
        if !concat_define_result_index(vm, result, *next_index, entry) {
            return false;
        }
        *next_index += 1;
    }
    true
}

pub fn array_concat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }

    let this_val = match &vm.current_this {
        JsValue::Bool(_) | JsValue::Number(_) | JsValue::String(_) | JsValue::BigInt(_) => {
            wrap_primitive_for_concat(vm, &vm.current_this.clone())
        }
        _ => vm.current_this.clone(),
    };
    let result = match array_species_create(vm, &this_val, 0) {
        Some(result) => result,
        None => return JsValue::Undefined,
    };
    let this_spreadable = is_concat_spreadable(vm, &this_val);
    if vm.pending_exception.is_some() {
        return JsValue::Undefined;
    }
    let mut next_index = 0usize;
    match this_spreadable {
        Some(true) => {
            if !concat_append_spread(vm, &result, &mut next_index, &this_val) {
                return JsValue::Undefined;
            }
        }
        Some(false) => {
            if !concat_define_result_index(vm, &result, next_index, this_val) {
                return JsValue::Undefined;
            }
            next_index += 1;
        }
        None => return JsValue::Undefined,
    }

    for arg in args {
        match is_concat_spreadable(vm, arg) {
            Some(true) => {
                if !concat_append_spread(vm, &result, &mut next_index, arg) {
                    return JsValue::Undefined;
                }
            }
            Some(false) => {
                if !concat_define_result_index(vm, &result, next_index, arg.clone()) {
                    return JsValue::Undefined;
                }
                next_index += 1;
            }
            None => return JsValue::Undefined,
        }
    }
    concat_set_result_length(&result, next_index);
    result
}

pub fn array_flat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let depth = args.first().map(|v| v.to_number() as usize).unwrap_or(1);
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let dense = a.to_dense_vec();
        let result = flatten_vec(&dense, depth);
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(result))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

fn flatten_vec(elements: &[JsValue], depth: usize) -> Vec<JsValue> {
    let mut result = Vec::new();
    for el in elements {
        if depth > 0 {
            if let JsValue::Array(a) = el {
                let inner = a.borrow();
                let dense = inner.to_dense_vec();
                result.extend(flatten_vec(&dense, depth - 1));
                continue;
            }
        }
        result.push(el.clone());
    }
    result
}

pub fn array_at(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_obj = match coerce_array_like_this(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let len = match array_like_length(vm, &this_obj) {
        Some(v) => v as i64,
        None => return JsValue::Undefined,
    };
    let idx_val = args.first().cloned().unwrap_or(JsValue::Undefined);
    let idx = to_number_vm(vm, &idx_val) as i64;
    if vm.pending_exception.is_some() {
        JsValue::Undefined
    } else {
        let actual = if idx < 0 { len + idx } else { idx };
        if actual >= 0 && actual < len {
            vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", actual))
        } else {
            JsValue::Undefined
        }
    }
}

pub fn array_to_string(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    array_join(vm, args)
}

// ═══════════════════════════════════════════════════════════
// Higher-order methods (map, filter, reduce, etc.)
// ═══════════════════════════════════════════════════════════

/// Public wrapper for call_callback — used by other native modules.
pub fn call_callback_pub(vm: &mut Vm, callback: &JsValue, args: &[JsValue]) -> JsValue {
    call_callback(vm, callback, args)
}

/// Helper: call a callback function with given args.
/// Propagates exceptions from last_exception to pending_exception.
fn call_callback(vm: &mut Vm, callback: &JsValue, args: &[JsValue]) -> JsValue {
    match callback {
        JsValue::Function(_) => {
            let result = vm.call_value(callback, args, JsValue::Undefined);
            if let Some(exc) = vm.last_exception.take() {
                vm.pending_exception = Some(exc);
            }
            result
        }
        _ => JsValue::Undefined,
    }
}

/// Helper: call a callback function with an explicit `this` binding.
fn call_callback_with_this(
    vm: &mut Vm,
    callback: &JsValue,
    this_arg: &JsValue,
    args: &[JsValue],
) -> JsValue {
    match callback {
        JsValue::Function(_) => {
            let result = vm.call_value(callback, args, this_arg.clone());
            if let Some(exc) = vm.last_exception.take() {
                vm.pending_exception = Some(exc);
            }
            result
        }
        _ => JsValue::Undefined,
    }
}

pub fn array_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, len) = match this_array_like_len(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    // ArraySpeciesCreate: RangeError if length > 2^32 - 1
    if len > 0xFFFF_FFFF {
        let err = vm.make_range_error("Invalid array length");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let result = match array_species_create(vm, &this_obj, len) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    for idx in 0..len {
        if !array_like_has_index(vm, &this_obj, idx).unwrap_or(false) {
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            continue;
        }
        let el = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        let val = call_callback_with_this(
            vm,
            &callback,
            &this_arg,
            &[el, JsValue::Number(idx as f64), this_obj.clone()],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        if !concat_define_result_index(vm, &result, idx, val) {
            return JsValue::Undefined;
        }
    }
    concat_set_result_length(&result, len);
    result
}

pub fn array_filter(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, len) = match this_array_like_len(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    let result = match array_species_create(vm, &this_obj, 0) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let mut to = 0usize;
    for idx in 0..len {
        if !array_like_has_index(vm, &this_obj, idx).unwrap_or(false) {
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            continue;
        }
        let el = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        let val = call_callback_with_this(
            vm,
            &callback,
            &this_arg,
            &[el.clone(), JsValue::Number(idx as f64), this_obj.clone()],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        if val.to_boolean() {
            if !concat_define_result_index(vm, &result, to, el) {
                return JsValue::Undefined;
            }
            to += 1;
        }
    }
    concat_set_result_length(&result, to);
    result
}

pub fn array_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, len) = match this_array_like_len(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    for idx in 0..len {
        if !array_like_has_index(vm, &this_obj, idx).unwrap_or(false) {
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            continue;
        }
        let el = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        call_callback_with_this(
            vm,
            &callback,
            &this_arg,
            &[el, JsValue::Number(idx as f64), this_obj.clone()],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
    }
    JsValue::Undefined
}

pub fn array_reduce(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, _len, entries) = match this_array_like_entries(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    let has_initial = args.len() > 1;
    if entries.is_empty() && !has_initial {
        let err = vm.make_type_error("Reduce of empty array with no initial value");
        vm.throw_native(err);
        return JsValue::Undefined;
    }

    let (start, mut acc) = if has_initial {
        (0, args[1].clone())
    } else {
        (1, entries[0].1.clone())
    };

    for &(idx, ref el) in &entries[start..] {
        acc = call_callback(
            vm,
            &callback,
            &[
                acc,
                el.clone(),
                JsValue::Number(idx as f64),
                this_obj.clone(),
            ],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
    }
    acc
}

pub fn array_reduce_right(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, _len, entries) = match this_array_like_entries(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    let has_initial = args.len() > 1;
    if entries.is_empty() && !has_initial {
        let err = vm.make_type_error("Reduce of empty array with no initial value");
        vm.throw_native(err);
        return JsValue::Undefined;
    }

    let (skip_last, mut acc) = if has_initial {
        (false, args[1].clone())
    } else {
        (true, entries.last().unwrap().1.clone())
    };

    let iter = if skip_last {
        &entries[..entries.len() - 1]
    } else {
        &entries[..]
    };
    for &(idx, ref el) in iter.iter().rev() {
        acc = call_callback(
            vm,
            &callback,
            &[
                acc,
                el.clone(),
                JsValue::Number(idx as f64),
                this_obj.clone(),
            ],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
    }
    acc
}

pub fn array_find(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, len) = match this_array_like_len(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    for idx in 0..len {
        if !array_like_has_index(vm, &this_obj, idx).unwrap_or(false) {
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            continue;
        }
        let el = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        let val = call_callback_with_this(
            vm,
            &callback,
            &this_arg,
            &[el.clone(), JsValue::Number(idx as f64), this_obj.clone()],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        if val.to_boolean() {
            return el;
        }
    }
    JsValue::Undefined
}

pub fn array_find_index(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, len) = match this_array_like_len(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    for idx in 0..len {
        if !array_like_has_index(vm, &this_obj, idx).unwrap_or(false) {
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            continue;
        }
        let el = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        let val = call_callback_with_this(
            vm,
            &callback,
            &this_arg,
            &[el, JsValue::Number(idx as f64), this_obj.clone()],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        if val.to_boolean() {
            return JsValue::Number(idx as f64);
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_some(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, len) = match this_array_like_len(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    for idx in 0..len {
        if !array_like_has_index(vm, &this_obj, idx).unwrap_or(false) {
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            continue;
        }
        let el = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        let val = call_callback_with_this(
            vm,
            &callback,
            &this_arg,
            &[el, JsValue::Number(idx as f64), this_obj.clone()],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        if val.to_boolean() {
            return JsValue::Bool(true);
        }
    }
    JsValue::Bool(false)
}

pub fn array_every(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, len) = match this_array_like_len(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    for idx in 0..len {
        if !array_like_has_index(vm, &this_obj, idx).unwrap_or(false) {
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            continue;
        }
        let el = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        let val = call_callback_with_this(
            vm,
            &callback,
            &this_arg,
            &[el, JsValue::Number(idx as f64), this_obj.clone()],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        if !val.to_boolean() {
            return JsValue::Bool(false);
        }
    }
    JsValue::Bool(true)
}

pub fn array_flat_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, _len, entries) = match this_array_like_entries(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    let mut result = Vec::new();
    for (idx, el) in entries {
        let val = call_callback_with_this(
            vm,
            &callback,
            &this_arg,
            &[el, JsValue::Number(idx as f64), this_obj.clone()],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        match val {
            JsValue::Array(a) => {
                let inner = a.borrow();
                for (_, v) in inner.iter_entries() {
                    result.push(v.clone());
                }
            }
            _ => result.push(val),
        }
    }
    JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(result))))
}

// ═══════════════════════════════════════════════════════════
// Iterator-returning methods
// ═══════════════════════════════════════════════════════════

pub fn array_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this_obj = match coerce_array_like_this(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let len = match array_like_length(vm, &this_obj) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let mut pairs = Vec::with_capacity(len);
    for idx in 0..len {
        let value = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        pairs.push(JsValue::new_array(vec![JsValue::Number(idx as f64), value]));
    }
    vm.make_internal_iterator(pairs)
}

pub fn array_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this_obj = match coerce_array_like_this(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let len = match array_like_length(vm, &this_obj) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let keys: Vec<JsValue> = (0..len).map(|idx| JsValue::Number(idx as f64)).collect();
    vm.make_internal_iterator(keys)
}

pub fn array_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this_obj = match coerce_array_like_this(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let len = match array_like_length(vm, &this_obj) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    let mut vals = Vec::with_capacity(len);
    for idx in 0..len {
        let value = vm.get_property_invoking_getter(&this_obj, &alloc::format!("{}", idx));
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        vals.push(value);
    }
    vm.make_internal_iterator(vals)
}

// ═══════════════════════════════════════════════════════════
// Array static methods
// ═══════════════════════════════════════════════════════════

pub fn array_is_array(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(value) => match is_array_value(vm, value) {
            Some(is_array) => JsValue::Bool(is_array),
            None => JsValue::Undefined,
        },
        None => JsValue::Bool(false),
    }
}

pub fn array_from(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let source = args.first().cloned().unwrap_or(JsValue::Undefined);
    let map_fn = args.get(1).cloned();
    let elements: Vec<JsValue> = match &source {
        JsValue::Array(a) => a.borrow().to_dense_vec(),
        JsValue::String(s) => s
            .chars()
            .map(|c| {
                let mut buf = String::new();
                buf.push(c);
                JsValue::String(buf)
            })
            .collect(),
        JsValue::Object(obj) => {
            let tag = obj.borrow().internal_tag.clone();
            match tag.as_deref() {
                Some("__set__") | Some("Set") => {
                    if let JsValue::Array(items) = obj.borrow().get("__items") {
                        items.borrow().values_vec()
                    } else {
                        Vec::new()
                    }
                }
                Some("__map__") | Some("Map") => {
                    if let (JsValue::Array(keys), JsValue::Array(vals)) =
                        (obj.borrow().get("__keys"), obj.borrow().get("__values"))
                    {
                        let ks = keys.borrow();
                        let vs = vals.borrow();
                        let kv: Vec<_> = ks.elements.values().cloned().collect();
                        let vv: Vec<_> = vs.elements.values().cloned().collect();
                        kv.into_iter()
                            .zip(vv.into_iter())
                            .map(|(k, v)| {
                                JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(vec![k, v]))))
                            })
                            .collect()
                    } else {
                        Vec::new()
                    }
                }
                _ => {
                    let len = obj.borrow().get("length").to_number();
                    if len > 0.0 && len.is_finite() {
                        let n = len as usize;
                        (0..n.min(10_000))
                            .map(|i| obj.borrow().get(&alloc::format!("{}", i)))
                            .collect()
                    } else {
                        Vec::new()
                    }
                }
            }
        }
        _ => Vec::new(),
    };
    if let Some(callback) = map_fn {
        let mut result = Vec::with_capacity(elements.len());
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            result.push(val);
        }
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(result))))
    } else {
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(elements))))
    }
}

pub fn array_of(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(args.to_vec()))))
}
