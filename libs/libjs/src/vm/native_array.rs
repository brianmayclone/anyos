//! Array.prototype methods and Array static methods.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::value::*;
use super::Vm;

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
        if r < 0.0 { 0 } else { r as usize }
    } else {
        (idx as usize).min(len)
    }
}

/// Snapshot all (index, value) pairs from the array — used by higher-order
/// methods that must not be affected by mutations during iteration.
fn snapshot_entries(a: &JsArray) -> Vec<(usize, JsValue)> {
    a.elements.iter().map(|(&k, v)| (k, v.clone())).collect()
}

// ═══════════════════════════════════════════════════════════
// Mutating methods
// ═══════════════════════════════════════════════════════════

pub fn array_push(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let current_len = arr.borrow().len();
        if args.is_empty() {
            return JsValue::Number(current_len as f64);
        }
        let new_len = current_len + args.len();
        // Max array length is 2^32 − 1 (ES2023 §23.1.3.20 step 5).
        if new_len > 0xFFFF_FFFF {
            let exc = vm.make_range_error("Invalid array length");
            if !vm.handle_exception(exc) { return JsValue::Undefined; }
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
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        a.pop()
    } else {
        JsValue::Undefined
    }
}

pub fn array_shift(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
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
            let entries_after: Vec<(usize, JsValue)> = a.elements.range(after_start..)
                .map(|(&k, v)| (k, v.clone())).collect();
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
    if let Some(arr) = this_array(vm) {
        arr.borrow_mut().reverse();
        JsValue::Array(arr)
    } else {
        JsValue::Undefined
    }
}

pub fn array_sort(vm: &mut Vm, args: &[JsValue]) -> JsValue {
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
                        let result = call_callback(vm, &cmp, &[values[j].clone(), values[j + 1].clone()]);
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
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        let len = a.length;
        let value = args.first().cloned().unwrap_or(JsValue::Undefined);
        let start = resolve_index(args.get(1).map(|v| v.to_number()).unwrap_or(0.0), len);
        let end = resolve_index(args.get(2).map(|v| v.to_number()).unwrap_or(len as f64), len);
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
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        let len = a.length;
        let target = resolve_index(args.first().map(|v| v.to_number()).unwrap_or(0.0), len);
        let start = resolve_index(args.get(1).map(|v| v.to_number()).unwrap_or(0.0), len);
        let end = resolve_index(args.get(2).map(|v| v.to_number()).unwrap_or(len as f64), len);
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
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let search = args.first().cloned().unwrap_or(JsValue::Undefined);
        let len = a.len();
        if len == 0 { return JsValue::Number(-1.0); }
        let from_raw = args.get(1).map(|v| v.to_number()).unwrap_or(0.0);
        if from_raw.is_infinite() && from_raw > 0.0 {
            return JsValue::Number(-1.0);
        }
        let from = if from_raw < 0.0 {
            let r = len as f64 + from_raw;
            if r < 0.0 { 0usize } else { r as usize }
        } else {
            (from_raw as usize).min(len)
        };
        // Only iterate over actually-set entries >= from (sparse-safe).
        for (&idx, val) in a.elements.range(from..) {
            if val.strict_eq(&search) {
                return JsValue::Number(idx as f64);
            }
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_last_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let search = args.first().cloned().unwrap_or(JsValue::Undefined);
        let len = a.len();
        if len == 0 { return JsValue::Number(-1.0); }
        let from_raw = args.get(1).map(|v| v.to_number()).unwrap_or(len as f64 - 1.0);
        let from = if from_raw < 0.0 {
            let r = len as f64 + from_raw;
            if r < 0.0 { return JsValue::Number(-1.0); }
            r as usize
        } else if from_raw.is_infinite() || from_raw >= len as f64 {
            len - 1
        } else {
            from_raw as usize
        };
        // Iterate set entries in reverse, up to `from`.
        for (&idx, val) in a.elements.range(..=from).rev() {
            if val.strict_eq(&search) {
                return JsValue::Number(idx as f64);
            }
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_includes(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let search = args.first().cloned().unwrap_or(JsValue::Undefined);
        let len = a.len();
        let from = args.get(1).map(|v| resolve_index(v.to_number(), len)).unwrap_or(0);
        let check = |val: &JsValue| -> bool {
            if val.strict_eq(&search) { return true; }
            if let (JsValue::Number(a_n), JsValue::Number(s_n)) = (val, &search) {
                if a_n.is_nan() && s_n.is_nan() { return true; }
            }
            false
        };
        for (_, val) in a.elements.range(from..) {
            if check(val) { return JsValue::Bool(true); }
        }
    }
    JsValue::Bool(false)
}

pub fn array_join(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let sep = match args.first() {
            Some(JsValue::Undefined) | None => String::from(","),
            Some(v) => v.to_js_string(),
        };
        let len = a.length;
        let mut out = String::new();
        for i in 0..len {
            if i > 0 { out.push_str(&sep); }
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
        let end = resolve_index(args.get(1).map(|v| v.to_number()).unwrap_or(len as f64), len);
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
        let a = arr.borrow();
        let idx = args.first().map(|v| v.to_number() as i64).unwrap_or(0);
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
fn call_callback(vm: &mut Vm, callback: &JsValue, args: &[JsValue]) -> JsValue {
    match callback {
        JsValue::Function(_) => vm.call_value(callback, args, JsValue::Undefined),
        _ => JsValue::Undefined,
    }
}

pub fn array_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        let len = arr.borrow().length;
        let mut result = JsArray::new();
        result.length = len;
        for (idx, el) in entries {
            let val = call_callback(vm, &callback, &[el, JsValue::Number(idx as f64)]);
            result.elements.insert(idx, val);
        }
        JsValue::Array(Rc::new(RefCell::new(result)))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_filter(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        let mut result = Vec::new();
        for (idx, el) in entries {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
            if val.to_boolean() {
                result.push(el);
            }
        }
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(result))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        for (idx, el) in entries {
            call_callback(vm, &callback, &[el, JsValue::Number(idx as f64)]);
        }
    }
    JsValue::Undefined
}

pub fn array_reduce(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        let has_initial = args.len() > 1;
        if entries.is_empty() && !has_initial { return JsValue::Undefined; }

        let (start, mut acc) = if has_initial {
            (0, args[1].clone())
        } else {
            if entries.is_empty() { return JsValue::Undefined; }
            (1, entries[0].1.clone())
        };

        for &(idx, ref el) in &entries[start..] {
            acc = call_callback(vm, &callback, &[acc, el.clone(), JsValue::Number(idx as f64)]);
        }
        acc
    } else {
        JsValue::Undefined
    }
}

pub fn array_reduce_right(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        let has_initial = args.len() > 1;
        if entries.is_empty() && !has_initial { return JsValue::Undefined; }

        let (skip_last, mut acc) = if has_initial {
            (false, args[1].clone())
        } else {
            if entries.is_empty() { return JsValue::Undefined; }
            (true, entries.last().unwrap().1.clone())
        };

        let iter = if skip_last {
            &entries[..entries.len() - 1]
        } else {
            &entries[..]
        };
        for &(idx, ref el) in iter.iter().rev() {
            acc = call_callback(vm, &callback, &[acc, el.clone(), JsValue::Number(idx as f64)]);
        }
        acc
    } else {
        JsValue::Undefined
    }
}

pub fn array_find(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        for (idx, el) in entries {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
            if val.to_boolean() {
                return el;
            }
        }
    }
    JsValue::Undefined
}

pub fn array_find_index(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        for (idx, el) in entries {
            let val = call_callback(vm, &callback, &[el, JsValue::Number(idx as f64)]);
            if val.to_boolean() {
                return JsValue::Number(idx as f64);
            }
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_some(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        for (idx, el) in entries {
            let val = call_callback(vm, &callback, &[el, JsValue::Number(idx as f64)]);
            if val.to_boolean() {
                return JsValue::Bool(true);
            }
        }
    }
    JsValue::Bool(false)
}

pub fn array_every(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        for (idx, el) in entries {
            let val = call_callback(vm, &callback, &[el, JsValue::Number(idx as f64)]);
            if !val.to_boolean() {
                return JsValue::Bool(false);
            }
        }
    }
    JsValue::Bool(true)
}

pub fn array_flat_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let entries = { snapshot_entries(&arr.borrow()) };
        let mut result = Vec::new();
        for (idx, el) in entries {
            let val = call_callback(vm, &callback, &[el, JsValue::Number(idx as f64)]);
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
    } else {
        JsValue::new_array(Vec::new())
    }
}

// ═══════════════════════════════════════════════════════════
// Iterator-returning methods
// ═══════════════════════════════════════════════════════════

pub fn array_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let pairs: Vec<JsValue> = a.elements.iter().map(|(&idx, v)| {
            JsValue::new_array(vec![JsValue::Number(idx as f64), v.clone()])
        }).collect();
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(pairs))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let keys: Vec<JsValue> = a.elements.keys()
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
        JsValue::String(s) => s.chars().map(|c| {
            let mut buf = String::new();
            buf.push(c);
            JsValue::String(buf)
        }).collect(),
        JsValue::Object(obj) => {
            let tag = obj.borrow().internal_tag.clone();
            match tag.as_deref() {
                Some("__set__") => {
                    if let JsValue::Array(items) = obj.borrow().get("__items") {
                        items.borrow().values_vec()
                    } else { Vec::new() }
                }
                Some("__map__") => {
                    if let (JsValue::Array(keys), JsValue::Array(vals)) =
                        (obj.borrow().get("__keys"), obj.borrow().get("__values"))
                    {
                        let ks = keys.borrow();
                        let vs = vals.borrow();
                        let kv: Vec<_> = ks.elements.values().cloned().collect();
                        let vv: Vec<_> = vs.elements.values().cloned().collect();
                        kv.into_iter().zip(vv.into_iter()).map(|(k, v)| {
                            JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(vec![k, v]))))
                        }).collect()
                    } else { Vec::new() }
                }
                _ => {
                    let len = obj.borrow().get("length").to_number();
                    if len > 0.0 && len.is_finite() {
                        let n = len as usize;
                        (0..n.min(10_000)).map(|i| obj.borrow().get(&alloc::format!("{}", i))).collect()
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
