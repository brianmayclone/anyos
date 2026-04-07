//! Array.prototype methods and Array static methods.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

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

/// Snapshot all (index, value) pairs from the array — used by higher-order
/// methods that must not be affected by mutations during iteration.
fn snapshot_entries(a: &JsArray) -> Vec<(usize, JsValue)> {
    a.elements.iter().map(|(&k, v)| (k, v.clone())).collect()
}

/// ToNumber with VM — calls valueOf/toString on Objects per ES2023 §7.1.4.
/// Throws TypeError if ToPrimitive fails (both valueOf and toString return objects).
pub fn to_number_vm(vm: &mut Vm, val: &JsValue) -> f64 {
    match val {
        JsValue::Number(n) => *n,
        JsValue::String(s) => crate::value::parse_js_float(s),
        JsValue::Bool(true) => 1.0,
        JsValue::Bool(false) => 0.0,
        JsValue::Null => 0.0,
        JsValue::Undefined => f64::NAN,
        JsValue::Object(obj) => {
            let o = obj.borrow();
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
            // If both valueOf and toString were tried and returned objects → TypeError
            if tried_valueof || tried_tostring {
                let err = vm.make_type_error("Cannot convert object to primitive value");
                vm.throw_native(err);
            }
            f64::NAN
        }
        JsValue::Array(_) | JsValue::Function(_) => f64::NAN,
    }
}

/// Convert a JsValue to a length (ToLength), using to_number_vm.
fn to_length_vm(vm: &mut Vm, val: &JsValue) -> usize {
    let n = to_number_vm(vm, val);
    if n.is_nan() || n < 0.0 {
        0
    } else if !n.is_finite() {
        usize::MAX
    }
    // Infinity → huge value (callers check for RangeError)
    else {
        (n as u64).min(0x1F_FFFF_FFFF_FFFF) as usize
    } // 2^53 - 1
}

/// Get array-like entries from `this`. Supports Array and Object with `length`.
/// Returns (this_obj, length, entries) or None if `this` is null/undefined (throws TypeError).
/// `this_obj` is the original `this` value for passing as 3rd callback argument.
fn this_array_like(vm: &mut Vm) -> Option<(JsValue, usize, Vec<(usize, JsValue)>)> {
    let this_val = vm.current_this.clone();
    match &this_val {
        JsValue::Null | JsValue::Undefined => {
            let err = vm.make_type_error("Cannot convert undefined or null to object");
            vm.throw_native(err);
            None
        }
        JsValue::Array(a) => {
            let length;
            let mut entries;
            let mut accessor_keys = Vec::new();
            {
                let a = a.borrow();
                length = a.length;
                entries = snapshot_entries(&a);
                // Collect numeric accessor properties (set via Object.defineProperty)
                for (key, prop) in a.properties.iter() {
                    if prop.is_accessor() {
                        if let Ok(idx) = key.parse::<usize>() {
                            if idx < length {
                                accessor_keys.push((idx, prop.getter.clone()));
                            }
                        }
                    }
                }
                // Also check prototype chain for inherited accessor properties
                // (e.g. Object.defineProperty(Array.prototype, "0", {get: ...}))
                let mut proto_opt: Option<Rc<RefCell<JsObject>>> = Some(vm.array_proto.clone());
                let mut depth = 0;
                while let Some(proto) = proto_opt {
                    depth += 1;
                    if depth > 20 {
                        break;
                    }
                    let p = proto.borrow();
                    for (key, prop) in p.properties.iter() {
                        if let Ok(idx) = key.parse::<usize>() {
                            if idx < length
                                && !entries.iter().any(|&(i, _)| i == idx)
                                && !accessor_keys.iter().any(|&(i, _)| i == idx)
                            {
                                if prop.is_accessor() {
                                    if let Some(ref g) = prop.getter {
                                        accessor_keys.push((idx, Some(g.clone())));
                                    }
                                } else {
                                    entries.push((idx, prop.value.clone()));
                                }
                            }
                        }
                    }
                    proto_opt = p.prototype.clone();
                }
            }
            // Invoke getters via VM (outside borrow)
            for (idx, getter) in accessor_keys {
                if let Some(getter_fn) = getter {
                    let val = vm.call_value(&getter_fn, &[], this_val.clone());
                    if let Some(exc) = vm.last_exception.take() {
                        vm.pending_exception = Some(exc);
                    }
                    if vm.pending_exception.is_some() {
                        break;
                    }
                    // Replace or insert the entry
                    if let Some(existing) = entries.iter_mut().find(|(i, _)| *i == idx) {
                        existing.1 = val;
                    } else {
                        entries.push((idx, val));
                    }
                }
            }
            entries.sort_by_key(|&(idx, _)| idx);
            Some((this_val.clone(), length, entries))
        }
        JsValue::Object(obj) => {
            // Get length value — invoke getter if it's an accessor property
            let len_val = vm.get_property_invoking_getter(&this_val, "length");
            let len = to_length_vm(vm, &len_val);
            // If ToPrimitive threw TypeError, propagate it
            if vm.pending_exception.is_some() {
                return None;
            }
            // Collect data properties and accessor getters separately
            let mut entries = Vec::new();
            let mut accessor_getters: Vec<(usize, JsValue)> = Vec::new();
            {
                let o = obj.borrow();
                for (key, prop) in o.properties.iter() {
                    if let Ok(idx) = key.parse::<usize>() {
                        if idx < len {
                            if prop.is_accessor() {
                                if let Some(ref g) = prop.getter {
                                    accessor_getters.push((idx, g.clone()));
                                }
                            } else {
                                entries.push((idx, prop.value.clone()));
                            }
                        }
                    }
                }
                // Walk prototype chain for inherited numeric properties
                let mut proto_opt = o.prototype.clone();
                let mut depth = 0;
                while let Some(proto) = proto_opt {
                    depth += 1;
                    if depth > 50 {
                        break;
                    }
                    let p = proto.borrow();
                    for (key, prop) in p.properties.iter() {
                        if let Ok(idx) = key.parse::<usize>() {
                            if idx < len
                                && !entries.iter().any(|&(i, _)| i == idx)
                                && !accessor_getters.iter().any(|&(i, _)| i == idx)
                            {
                                if prop.is_accessor() {
                                    if let Some(ref g) = prop.getter {
                                        accessor_getters.push((idx, g.clone()));
                                    }
                                } else {
                                    entries.push((idx, prop.value.clone()));
                                }
                            }
                        }
                    }
                    proto_opt = p.prototype.clone();
                }
            }
            // Invoke accessor getters via VM (outside borrow)
            for (idx, getter_fn) in accessor_getters {
                let val = vm.call_value(&getter_fn, &[], this_val.clone());
                if let Some(exc) = vm.last_exception.take() {
                    vm.pending_exception = Some(exc);
                }
                if vm.pending_exception.is_some() {
                    break;
                }
                entries.push((idx, val));
            }
            entries.sort_by_key(|&(idx, _)| idx);
            Some((this_val.clone(), len, entries))
        }
        JsValue::String(s) => {
            let chars: Vec<char> = s.chars().collect();
            let entries: Vec<(usize, JsValue)> = chars
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let mut s = String::new();
                    s.push(*c);
                    (i, JsValue::String(s))
                })
                .collect();
            Some((this_val.clone(), chars.len(), entries))
        }
        // Primitives (bool, number): ToObject wraps them — read prototype for length + numeric properties
        JsValue::Bool(_) | JsValue::Number(_) => {
            let proto_rc = match &this_val {
                JsValue::Bool(_) => vm.boolean_proto.clone(),
                _ => vm.number_proto.clone(),
            };
            // Read all data from proto before calling to_length_vm (avoids borrow conflicts)
            let len_val;
            let mut entries = Vec::new();
            {
                let p = proto_rc.borrow();
                len_val = p.get("length");
                for (key, prop) in p.properties.iter() {
                    if !prop.is_accessor() {
                        if let Ok(idx) = key.parse::<usize>() {
                            entries.push((idx, prop.value.clone()));
                        }
                    }
                }
            }
            let len = to_length_vm(vm, &len_val);
            entries.retain(|&(idx, _)| idx < len);
            entries.sort_by_key(|&(idx, _)| idx);
            Some((this_val.clone(), len, entries))
        }
        _ => Some((this_val.clone(), 0, Vec::new())),
    }
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

// ═══════════════════════════════════════════════════════════
// Mutating methods
// ═══════════════════════════════════════════════════════════

pub fn array_push(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    if let Some(arr) = this_array(vm) {
        let current_len = arr.borrow().len();
        if args.is_empty() {
            return JsValue::Number(current_len as f64);
        }
        let new_len = current_len + args.len();
        // Max array length is 2^32 − 1 (ES2023 §23.1.3.20 step 5).
        if new_len > 0xFFFF_FFFF {
            let exc = vm.make_range_error("Invalid array length");
            if !vm.handle_exception(exc) {
                return JsValue::Undefined;
            }
            return JsValue::Undefined;
        }
        let mut a = arr.borrow_mut();
        for (i, arg) in args.iter().enumerate() {
            a.set(current_len + i, arg.clone());
        }
        JsValue::Number(a.len() as f64)
    } else {
        JsValue::Undefined
    }
}

pub fn array_pop(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        a.pop()
    } else {
        JsValue::Undefined
    }
}

pub fn array_shift(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        if a.length == 0 {
            JsValue::Undefined
        } else {
            a.remove_and_shift(0)
        }
    } else {
        JsValue::Undefined
    }
}

pub fn array_unshift(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        // Insert in reverse order so they end up in the right position.
        for (i, arg) in args.iter().enumerate() {
            a.insert_and_shift(i, arg.clone());
        }
        JsValue::Number(a.length as f64)
    } else {
        JsValue::Undefined
    }
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
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        let len = a.length;
        let value = args.first().cloned().unwrap_or(JsValue::Undefined);
        let start = resolve_index(args.get(1).map(|v| v.to_number()).unwrap_or(0.0), len);
        let end = resolve_index(
            args.get(2).map(|v| v.to_number()).unwrap_or(len as f64),
            len,
        );
        for i in start..end {
            a.elements.insert(i, value.clone());
        }
        drop(a);
        JsValue::Array(arr)
    } else {
        JsValue::Undefined
    }
}

pub fn array_copy_within(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !require_object_coercible(vm) {
        return JsValue::Undefined;
    }
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        let len = a.length;
        let target = resolve_index(args.first().map(|v| v.to_number()).unwrap_or(0.0), len);
        let start = resolve_index(args.get(1).map(|v| v.to_number()).unwrap_or(0.0), len);
        let end = resolve_index(
            args.get(2).map(|v| v.to_number()).unwrap_or(len as f64),
            len,
        );
        let count = end.saturating_sub(start).min(len.saturating_sub(target));
        // Read source values first.
        let copy: Vec<JsValue> = (0..count).map(|i| a.get(start + i)).collect();
        for (i, v) in copy.into_iter().enumerate() {
            a.elements.insert(target + i, v);
        }
        drop(a);
        JsValue::Array(arr)
    } else {
        JsValue::Undefined
    }
}

// ═══════════════════════════════════════════════════════════
// Non-mutating / accessor methods
// ═══════════════════════════════════════════════════════════

pub fn array_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let (_this_obj, len, entries) = match this_array_like(vm) {
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
    let (_this_obj, len, entries) = match this_array_like(vm) {
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
    let (_this_obj, len, entries) = match this_array_like(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if len == 0 {
        return JsValue::Bool(false);
    }
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    let from = args
        .get(1)
        .map(|v| resolve_index(v.to_number(), len))
        .unwrap_or(0);
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
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let len = a.length;
        let start = resolve_index(args.first().map(|v| v.to_number()).unwrap_or(0.0), len);
        let end = resolve_index(
            args.get(1).map(|v| v.to_number()).unwrap_or(len as f64),
            len,
        );
        let mut result = JsArray::new();
        if start < end {
            let new_len = end - start;
            result.length = new_len;
            for (&idx, val) in a.elements.range(start..end) {
                result.elements.insert(idx - start, val.clone());
            }
        }
        JsValue::Array(Rc::new(RefCell::new(result)))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_concat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut result = JsArray::new();
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        for (&idx, val) in a.elements.iter() {
            result.elements.insert(idx, val.clone());
        }
        result.length = a.length;
    }
    for arg in args {
        match arg {
            JsValue::Array(a) => {
                let arr = a.borrow();
                let offset = result.length;
                for (&idx, val) in arr.elements.iter() {
                    result.elements.insert(offset + idx, val.clone());
                }
                result.length += arr.length;
            }
            _ => {
                result.push(arg.clone());
            }
        }
    }
    JsValue::Array(Rc::new(RefCell::new(result)))
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
    if let Some(arr) = this_array(vm) {
        let idx_val = args.first().cloned().unwrap_or(JsValue::Undefined);
        let idx = to_number_vm(vm, &idx_val) as i64;
        let a = arr.borrow();
        let len = a.length as i64;
        let actual = if idx < 0 { len + idx } else { idx };
        if actual >= 0 && actual < len {
            a.get(actual as usize)
        } else {
            JsValue::Undefined
        }
    } else {
        JsValue::Undefined
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
    let (this_obj, len, entries) = match this_array_like(vm) {
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
    let mut result = JsArray::new();
    result.length = len;
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
        result.elements.insert(idx, val);
    }
    JsValue::Array(Rc::new(RefCell::new(result)))
}

pub fn array_filter(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, _len, entries) = match this_array_like(vm) {
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
            &[el.clone(), JsValue::Number(idx as f64), this_obj.clone()],
        );
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        if val.to_boolean() {
            result.push(el);
        }
    }
    JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(result))))
}

pub fn array_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, _len, entries) = match this_array_like(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    for (idx, el) in entries {
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
    let (this_obj, _len, entries) = match this_array_like(vm) {
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
    let (this_obj, _len, entries) = match this_array_like(vm) {
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
    let (this_obj, _len, entries) = match this_array_like(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
    for (idx, el) in entries {
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
    let (this_obj, _len, entries) = match this_array_like(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
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
        if val.to_boolean() {
            return JsValue::Number(idx as f64);
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_some(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, _len, entries) = match this_array_like(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
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
        if val.to_boolean() {
            return JsValue::Bool(true);
        }
    }
    JsValue::Bool(false)
}

pub fn array_every(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, _len, entries) = match this_array_like(vm) {
        Some(v) => v,
        None => return JsValue::Undefined,
    };
    if !require_callable(vm, &callback) {
        return JsValue::Undefined;
    }
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
        if !val.to_boolean() {
            return JsValue::Bool(false);
        }
    }
    JsValue::Bool(true)
}

pub fn array_flat_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (this_obj, _len, entries) = match this_array_like(vm) {
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
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let pairs: Vec<JsValue> = a
            .elements
            .iter()
            .map(|(&idx, v)| JsValue::new_array(vec![JsValue::Number(idx as f64), v.clone()]))
            .collect();
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(pairs))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let keys: Vec<JsValue> = a
            .elements
            .keys()
            .map(|&idx| JsValue::Number(idx as f64))
            .collect();
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(keys))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let vals: Vec<JsValue> = a.elements.values().cloned().collect();
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(vals))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

// ═══════════════════════════════════════════════════════════
// Array static methods
// ═══════════════════════════════════════════════════════════

pub fn array_is_array(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(matches!(args.first(), Some(JsValue::Array(_))))
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
