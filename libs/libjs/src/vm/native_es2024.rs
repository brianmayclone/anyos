// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! ES2022-2024 additions to built-in prototypes.
//!
//! - Array: findLast, findLastIndex, toReversed, toSorted, toSpliced, with, at (already done)
//! - Object: hasOwn, groupBy
//! - String: isWellFormed, toWellFormed
//! - Error: cause support
//! - structuredClone

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::Vm;
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// Array.prototype additions (ES2023+)
// ═══════════════════════════════════════════════════════════

/// Length of an array-like `this` per ES `LengthOfArrayLike`. Reads the
/// `length` property (invoking accessors), coerces it via ToInteger, and
/// propagates any exceptions through `pending_exception`.
fn array_like_length(vm: &mut Vm, this: &JsValue) -> Option<usize> {
    let len_val = vm.get_property_invoking_getter(this, "length");
    if vm.pending_exception.is_some() {
        return None;
    }
    let n = super::native_array::to_number_vm(vm, &len_val);
    if vm.pending_exception.is_some() {
        return None;
    }
    if n.is_nan() || n <= 0.0 {
        Some(0)
    } else if !n.is_finite() {
        Some(usize::MAX)
    } else {
        Some(n as usize)
    }
}

/// Generic Get(O, k) for findLast/findLastIndex paths. Falls back to
/// direct array indexing for `JsValue::Array`, otherwise invokes any
/// accessor on the object.
fn array_like_get(vm: &mut Vm, this: &JsValue, idx: usize) -> Option<JsValue> {
    let key = alloc::format!("{}", idx);
    let val = match this {
        JsValue::Array(arr) => {
            let a = arr.borrow();
            if let Some(v) = a.elements.get(&idx) {
                v.clone()
            } else {
                drop(a);
                vm.get_property_invoking_getter(this, &key)
            }
        }
        _ => vm.get_property_invoking_getter(this, &key),
    };
    if vm.pending_exception.is_some() {
        return None;
    }
    Some(val)
}

/// `Array.prototype.findLast(callback)`
pub fn array_find_last(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    // §1: Let O be ? ToObject(this value). For null/undefined this fails.
    if matches!(this, JsValue::Null | JsValue::Undefined) {
        let err = vm.make_type_error("Array.prototype.findLast called on null or undefined");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let len = match array_like_length(vm, &this) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    if len == 0 {
        return JsValue::Undefined;
    }
    let mut k = len;
    while k > 0 {
        k -= 1;
        let el = match array_like_get(vm, &this, k) {
            Some(v) => v,
            None => return JsValue::Undefined,
        };
        let val = vm.call_value(
            &callback,
            &[el.clone(), JsValue::Number(k as f64), this.clone()],
            JsValue::Undefined,
        );
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
            return JsValue::Undefined;
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        if val.to_boolean() {
            return el;
        }
    }
    JsValue::Undefined
}

/// `Array.prototype.findLastIndex(callback)`
pub fn array_find_last_index(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if matches!(this, JsValue::Null | JsValue::Undefined) {
        let err = vm.make_type_error("Array.prototype.findLastIndex called on null or undefined");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let len = match array_like_length(vm, &this) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    if len == 0 {
        return JsValue::Number(-1.0);
    }
    let mut k = len;
    while k > 0 {
        k -= 1;
        let el = match array_like_get(vm, &this, k) {
            Some(v) => v,
            None => return JsValue::Undefined,
        };
        let val = vm.call_value(
            &callback,
            &[el, JsValue::Number(k as f64), this.clone()],
            JsValue::Undefined,
        );
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
            return JsValue::Undefined;
        }
        if vm.pending_exception.is_some() {
            return JsValue::Undefined;
        }
        if val.to_boolean() {
            return JsValue::Number(k as f64);
        }
    }
    JsValue::Number(-1.0)
}

/// `Array.prototype.toReversed()` — non-mutating reverse
pub fn array_to_reversed(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if let JsValue::Array(arr) = &this {
        let mut dense = arr.borrow().to_dense_vec();
        dense.reverse();
        JsValue::new_array(dense)
    } else {
        JsValue::new_array(Vec::new())
    }
}

/// `Array.prototype.toSorted(compareFn?)` — non-mutating sort. Per
/// ES §23.1.3.34: read all elements first, then sort using SortCompare.
/// Exceptions from accessor reads or from the user-supplied comparator
/// must abort the sort and propagate.
pub fn array_to_sorted(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if matches!(this, JsValue::Null | JsValue::Undefined) {
        let err = vm.make_type_error("Array.prototype.toSorted called on null or undefined");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let compare_fn = args.first().cloned().unwrap_or(JsValue::Undefined);
    if !matches!(compare_fn, JsValue::Undefined | JsValue::Function(_)) {
        let err = vm.make_type_error("toSorted: compareFn must be a function or undefined");
        vm.throw_native(err);
        return JsValue::Undefined;
    }

    let len = match array_like_length(vm, &this) {
        Some(n) => n,
        None => return JsValue::Undefined,
    };
    // Guard against pathological lengths from array-likes that report e.g.
    // 2^53 — we can't allocate that and tests using such lengths usually
    // expect a TypeError or different code path.
    if len > 16 * 1024 * 1024 {
        let err = vm.make_range_error("Invalid array length");
        vm.throw_native(err);
        return JsValue::Undefined;
    }

    // Step 5: read all len entries (this can throw via accessors).
    let mut items: Vec<JsValue> = Vec::with_capacity(len);
    for k in 0..len {
        let v = match array_like_get(vm, &this, k) {
            Some(v) => v,
            None => return JsValue::Undefined,
        };
        items.push(v);
    }

    // Step 6: SortIndexedProperties with the SortCompare abstract operation.
    // We use insertion sort so we can stop on the first abrupt completion
    // without invoking sort_by's panic-on-throw behaviour.
    let n = items.len();
    for i in 1..n {
        let mut j = i;
        while j > 0 {
            let cmp = sort_compare(vm, &compare_fn, &items[j - 1], &items[j]);
            if vm.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            if cmp <= 0 {
                break;
            }
            items.swap(j - 1, j);
            j -= 1;
        }
    }

    JsValue::new_array(items)
}

/// SortCompare(x, y) — returns negative, zero, or positive `i32`.
/// Mirrors ECMA §23.1.3.30.2 with `READ-THROUGH-HOLES` behaviour
/// (undefined is sorted to the end when no comparator is supplied).
fn sort_compare(vm: &mut Vm, compare_fn: &JsValue, x: &JsValue, y: &JsValue) -> i32 {
    let x_undef = matches!(x, JsValue::Undefined);
    let y_undef = matches!(y, JsValue::Undefined);
    if x_undef && y_undef {
        return 0;
    }
    if x_undef {
        return 1;
    }
    if y_undef {
        return -1;
    }
    if let JsValue::Function(_) = compare_fn {
        let result = vm.call_value(compare_fn, &[x.clone(), y.clone()], JsValue::Undefined);
        if let Some(exc) = vm.last_exception.take() {
            vm.pending_exception = Some(exc);
            return 0;
        }
        if vm.pending_exception.is_some() {
            return 0;
        }
        let n = super::native_array::to_number_vm(vm, &result);
        if vm.pending_exception.is_some() {
            return 0;
        }
        if n.is_nan() {
            return 0;
        }
        if n < 0.0 {
            return -1;
        }
        if n > 0.0 {
            return 1;
        }
        return 0;
    }
    // No comparator: compare as strings.
    let sx = x.to_js_string();
    let sy = y.to_js_string();
    if sx < sy {
        -1
    } else if sx > sy {
        1
    } else {
        0
    }
}

/// `Array.prototype.toSpliced(start, deleteCount, ...items)` — non-mutating splice
pub fn array_to_spliced(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if let JsValue::Array(arr) = &this {
        let dense = arr.borrow().to_dense_vec();
        let len = dense.len();
        let start_raw = args.first().map(|v| v.to_number() as i64).unwrap_or(0);
        let start = if start_raw < 0 {
            (len as i64 + start_raw).max(0) as usize
        } else {
            (start_raw as usize).min(len)
        };
        let delete_count = args
            .get(1)
            .map(|v| (v.to_number() as usize).min(len - start))
            .unwrap_or(len - start);
        let items: Vec<JsValue> = args.iter().skip(2).cloned().collect();

        let mut result = Vec::with_capacity(len - delete_count + items.len());
        result.extend_from_slice(&dense[..start]);
        result.extend(items);
        if start + delete_count < len {
            result.extend_from_slice(&dense[start + delete_count..]);
        }
        JsValue::new_array(result)
    } else {
        JsValue::new_array(Vec::new())
    }
}

/// `Array.prototype.with(index, value)` — non-mutating index replacement
pub fn array_with(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if let JsValue::Array(arr) = &this {
        let mut dense = arr.borrow().to_dense_vec();
        let len = dense.len();
        let idx_raw = args.first().map(|v| v.to_number() as i64).unwrap_or(0);
        let idx = if idx_raw < 0 {
            (len as i64 + idx_raw) as usize
        } else {
            idx_raw as usize
        };
        let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        if idx < len {
            dense[idx] = value;
        }
        JsValue::new_array(dense)
    } else {
        JsValue::new_array(Vec::new())
    }
}

// ═══════════════════════════════════════════════════════════
// Object additions (ES2022+)
// ═══════════════════════════════════════════════════════════

/// `Object.hasOwn(obj, prop)` — ES2022
pub fn object_has_own(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let key = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    match &target {
        JsValue::Object(obj) => JsValue::Bool(obj.borrow().has_own(&key)),
        _ => JsValue::Bool(false),
    }
}

/// `Object.groupBy(items, callback)` — ES2024
pub fn object_group_by(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let items = args.first().cloned().unwrap_or(JsValue::Undefined);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);

    let result = JsValue::new_object();
    if let JsValue::Array(arr) = &items {
        let entries: Vec<(usize, JsValue)> = arr
            .borrow()
            .elements
            .iter()
            .map(|(&k, v)| (k, v.clone()))
            .collect();
        for (i, item) in entries.iter().map(|(k, v)| (*k, v)) {
            vm.invoke_function(
                &callback,
                &[item.clone(), JsValue::Number(i as f64)],
                JsValue::Undefined,
            );
            let key = vm.stack.pop().unwrap_or(JsValue::Undefined).to_js_string();
            let group = result.get_property(&key);
            if let JsValue::Array(grp) = &group {
                grp.borrow_mut().push(item.clone());
            } else {
                let new_arr = JsValue::new_array(alloc::vec![item.clone()]);
                result.set_property(key, new_arr);
            }
        }
    }
    result
}

// ═══════════════════════════════════════════════════════════
// String additions
// ═══════════════════════════════════════════════════════════

/// `String.prototype.isWellFormed()` — ES2024
pub fn string_is_well_formed(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    // ToString must invoke Symbol.toPrimitive / toString and propagate exceptions.
    let _s = match super::native_string::this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    // In Rust, all Strings are valid UTF-8, so always well-formed.
    JsValue::Bool(true)
}

/// `String.prototype.toWellFormed()` — ES2024
pub fn string_to_well_formed(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    // 1. Let O be ? RequireObjectCoercible(this value).
    // 2. Let S be ? ToString(O).
    let s = match super::native_string::this_string_checked(vm) {
        Some(s) => s,
        None => return JsValue::Undefined,
    };
    // Already well-formed in Rust.
    JsValue::String(s)
}

// ═══════════════════════════════════════════════════════════
// structuredClone
// ═══════════════════════════════════════════════════════════

/// `structuredClone(value)` — deep clone
pub fn structured_clone(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    deep_clone(&value)
}

fn deep_clone(val: &JsValue) -> JsValue {
    match val {
        JsValue::Object(obj) => {
            let o = obj.borrow();
            let mut new_obj = JsObject::new();
            new_obj.internal_tag = o.internal_tag.clone();
            for (k, prop) in &o.properties {
                let cloned_val = deep_clone(&prop.value);
                new_obj.properties.insert(
                    k.clone(),
                    Property {
                        value: cloned_val,
                        writable: prop.writable,
                        enumerable: prop.enumerable,
                        configurable: prop.configurable,
                        getter: prop.getter.clone(),
                        setter: prop.setter.clone(),
                    },
                );
            }
            if let Some(ref proto) = o.prototype {
                new_obj.prototype = Some(proto.clone()); // share prototype
            }
            JsValue::Object(Rc::new(RefCell::new(new_obj)))
        }
        JsValue::Array(arr) => {
            let a = arr.borrow();
            let mut new_arr = JsArray::new();
            new_arr.length = a.length;
            for (&k, v) in a.elements.iter() {
                new_arr.elements.insert(k, deep_clone(v));
            }
            JsValue::Array(Rc::new(RefCell::new(new_arr)))
        }
        // Primitives are copied by value
        other => other.clone(),
    }
}
