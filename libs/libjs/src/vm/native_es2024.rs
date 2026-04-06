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

/// `Array.prototype.findLast(callback)`
pub fn array_find_last(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Array(arr) = &this {
        let entries: Vec<(usize, JsValue)> = arr
            .borrow()
            .elements
            .iter()
            .map(|(&k, v)| (k, v.clone()))
            .collect();
        for &(i, ref el) in entries.iter().rev() {
            let result = vm.invoke_function(
                &callback,
                &[el.clone(), JsValue::Number(i as f64), this.clone()],
                JsValue::Undefined,
            );
            let val = vm.stack.pop().unwrap_or(JsValue::Undefined);
            if val.to_boolean() {
                return el.clone();
            }
        }
    }
    JsValue::Undefined
}

/// `Array.prototype.findLastIndex(callback)`
pub fn array_find_last_index(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Array(arr) = &this {
        let entries: Vec<(usize, JsValue)> = arr
            .borrow()
            .elements
            .iter()
            .map(|(&k, v)| (k, v.clone()))
            .collect();
        for &(i, ref el) in entries.iter().rev() {
            vm.invoke_function(
                &callback,
                &[el.clone(), JsValue::Number(i as f64), this.clone()],
                JsValue::Undefined,
            );
            let val = vm.stack.pop().unwrap_or(JsValue::Undefined);
            if val.to_boolean() {
                return JsValue::Number(i as f64);
            }
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

/// `Array.prototype.toSorted(compareFn?)` — non-mutating sort
pub fn array_to_sorted(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if let JsValue::Array(arr) = &this {
        let mut dense = arr.borrow().to_dense_vec();
        dense.sort_by(|a, b| {
            let sa = a.to_js_string();
            let sb = b.to_js_string();
            sa.cmp(&sb)
        });
        JsValue::new_array(dense)
    } else {
        JsValue::new_array(Vec::new())
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
    let s = vm.current_this.to_js_string();
    // In Rust, all Strings are valid UTF-8, so always well-formed
    JsValue::Bool(true)
}

/// `String.prototype.toWellFormed()` — ES2024
pub fn string_to_well_formed(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let s = vm.current_this.to_js_string();
    // Already well-formed in Rust
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
