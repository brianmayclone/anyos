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
        a.elements.pop().unwrap_or(JsValue::Undefined)
    } else {
        JsValue::Undefined
    }
}

pub fn array_shift(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        if a.elements.is_empty() {
            JsValue::Undefined
        } else {
            a.elements.remove(0)
        }
    } else {
        JsValue::Undefined
    }
}

pub fn array_unshift(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        for (i, arg) in args.iter().enumerate() {
            a.elements.insert(i, arg.clone());
        }
        JsValue::Number(a.elements.len() as f64)
    } else {
        JsValue::Undefined
    }
}

pub fn array_splice(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        let len = a.elements.len();
        let start_raw = args.first().map(|v| v.to_number()).unwrap_or(0.0);
        let start = resolve_index(start_raw, len);
        let delete_count = if args.len() > 1 {
            let dc = args[1].to_number() as usize;
            dc.min(len - start)
        } else {
            len - start
        };
        let removed: Vec<JsValue> = a.elements.drain(start..start + delete_count).collect();
        // Insert new elements
        if args.len() > 2 {
            for (i, item) in args[2..].iter().enumerate() {
                a.elements.insert(start + i, item.clone());
            }
        }
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(removed))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_reverse(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        a.elements.reverse();
        drop(a);
        JsValue::Array(arr)
    } else {
        JsValue::Undefined
    }
}

pub fn array_sort(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let comparefn = args.first().cloned();
    if let Some(arr) = this_array(vm) {
        // Extract elements, sort, put back
        let mut elements = {
            let a = arr.borrow();
            a.elements.clone()
        };

        if let Some(cmp) = &comparefn {
            if matches!(cmp, JsValue::Function(_)) {
                let cmp = cmp.clone();
                // Bubble sort using call_callback so bytecode comparators work correctly.
                let len = elements.len();
                for i in 0..len {
                    for j in 0..len.saturating_sub(1 + i) {
                        let result = call_callback(vm, &cmp, &[elements[j].clone(), elements[j + 1].clone()]);
                        if result.to_number() > 0.0 {
                            elements.swap(j, j + 1);
                        }
                    }
                }
            }
        } else {
            // Default: lexicographic sort
            elements.sort_by(|a, b| {
                let sa = a.to_js_string();
                let sb = b.to_js_string();
                sa.cmp(&sb)
            });
        }

        {
            let mut a = arr.borrow_mut();
            a.elements = elements;
        }
        JsValue::Array(arr)
    } else {
        JsValue::Undefined
    }
}

pub fn array_fill(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let mut a = arr.borrow_mut();
        let len = a.elements.len();
        let value = args.first().cloned().unwrap_or(JsValue::Undefined);
        let start = resolve_index(args.get(1).map(|v| v.to_number()).unwrap_or(0.0), len);
        let end = resolve_index(args.get(2).map(|v| v.to_number()).unwrap_or(len as f64), len);
        for i in start..end {
            a.elements[i] = value.clone();
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
        let len = a.elements.len();
        let target = resolve_index(args.first().map(|v| v.to_number()).unwrap_or(0.0), len);
        let start = resolve_index(args.get(1).map(|v| v.to_number()).unwrap_or(0.0), len);
        let end = resolve_index(args.get(2).map(|v| v.to_number()).unwrap_or(len as f64), len);
        let count = end.saturating_sub(start).min(len.saturating_sub(target));
        let copy: Vec<JsValue> = a.elements[start..start + count].to_vec();
        for (i, v) in copy.into_iter().enumerate() {
            a.elements[target + i] = v;
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
        // Search dense part.
        for i in from..a.elements.len() {
            if a.elements[i].strict_eq(&search) {
                return JsValue::Number(i as f64);
            }
        }
        // Search sparse part.
        let start = from.max(a.elements.len());
        for (&idx, val) in a.sparse.range(start..) {
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
        // Search sparse part in reverse first (indices >= elements.len()).
        for (&idx, val) in a.sparse.range(..=from).rev() {
            if val.strict_eq(&search) {
                return JsValue::Number(idx as f64);
            }
        }
        // Search dense part in reverse.
        let dense_end = from.min(a.elements.len().saturating_sub(1));
        if !a.elements.is_empty() {
            for i in (0..=dense_end).rev() {
                if a.elements[i].strict_eq(&search) {
                    return JsValue::Number(i as f64);
                }
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
        for i in from..a.elements.len() {
            if check(&a.elements[i]) { return JsValue::Bool(true); }
        }
        let sparse_start = from.max(a.elements.len());
        for (_, val) in a.sparse.range(sparse_start..) {
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
        let mut out = String::new();
        for (i, el) in a.elements.iter().enumerate() {
            if i > 0 { out.push_str(&sep); }
            match el {
                JsValue::Undefined | JsValue::Null => {}
                _ => out.push_str(&el.to_js_string()),
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
        let len = a.elements.len();
        let start = resolve_index(args.first().map(|v| v.to_number()).unwrap_or(0.0), len);
        let end = resolve_index(args.get(1).map(|v| v.to_number()).unwrap_or(len as f64), len);
        let result: Vec<JsValue> = if start < end {
            a.elements[start..end].to_vec()
        } else {
            Vec::new()
        };
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(result))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_concat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut result = Vec::new();
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        result.extend(a.elements.iter().cloned());
    }
    for arg in args {
        match arg {
            JsValue::Array(a) => {
                let arr = a.borrow();
                result.extend(arr.elements.iter().cloned());
            }
            _ => result.push(arg.clone()),
        }
    }
    JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(result))))
}

pub fn array_flat(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let depth = args.first().map(|v| v.to_number() as usize).unwrap_or(1);
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let result = flatten_elements(&a.elements, depth);
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(result))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

fn flatten_elements(elements: &[JsValue], depth: usize) -> Vec<JsValue> {
    let mut result = Vec::new();
    for el in elements {
        if depth > 0 {
            if let JsValue::Array(a) = el {
                let inner = a.borrow();
                result.extend(flatten_elements(&inner.elements, depth - 1));
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
        let len = a.elements.len() as i64;
        let actual = if idx < 0 { len + idx } else { idx };
        if actual >= 0 && actual < len {
            a.elements[actual as usize].clone()
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
    // Use call_value which correctly saves/restores run_target_depth so that
    // vm.run() stops after the callback returns without continuing into the
    // caller's frame (which is suspended inside the native array method).
    match callback {
        JsValue::Function(_) => vm.call_value(callback, args, JsValue::Undefined),
        _ => JsValue::Undefined,
    }
}

pub fn array_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        let mut result = Vec::with_capacity(elements.len());
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            result.push(val);
        }
        // Handle sparse entries.
        let mut result_arr = JsArray::from_vec(result);
        for (&idx, el) in sparse.iter() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
            result_arr.set(idx, val);
        }
        JsValue::Array(Rc::new(RefCell::new(result_arr)))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_filter(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        let mut result = Vec::new();
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            if val.to_boolean() {
                result.push(el.clone());
            }
        }
        for (&idx, el) in sparse.iter() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
            if val.to_boolean() {
                result.push(el.clone());
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
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        for (i, el) in elements.iter().enumerate() {
            call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
        }
        for (&idx, el) in sparse.iter() {
            call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
        }
    }
    JsValue::Undefined
}

pub fn array_reduce(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        let has_initial = args.len() > 1;
        let total = elements.len() + sparse.len();
        if total == 0 && !has_initial { return JsValue::Undefined; }
        let mut start_idx = 0;
        let mut acc = if has_initial {
            args[1].clone()
        } else {
            if elements.is_empty() && sparse.is_empty() { return JsValue::Undefined; }
            if !elements.is_empty() {
                start_idx = 1;
                elements[0].clone()
            } else {
                // First sparse entry is the initial value.
                let (&first_idx, first_val) = sparse.iter().next().unwrap();
                // We'll skip this entry in the sparse loop below.
                start_idx = first_idx + 1;
                first_val.clone()
            }
        };
        for i in start_idx..elements.len() {
            acc = call_callback(vm, &callback, &[acc, elements[i].clone(), JsValue::Number(i as f64)]);
        }
        let sparse_start = if start_idx > elements.len() { start_idx } else { elements.len() };
        for (&idx, el) in sparse.range(sparse_start..) {
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
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        let has_initial = args.len() > 1;
        let total = elements.len() + sparse.len();
        if total == 0 && !has_initial { return JsValue::Undefined; }
        let mut acc = if has_initial {
            args[1].clone()
        } else if !sparse.is_empty() {
            let (_, last_val) = sparse.iter().next_back().unwrap();
            last_val.clone()
        } else {
            elements[elements.len() - 1].clone()
        };
        // Iterate sparse in reverse first.
        let skip_last_sparse = !has_initial && !sparse.is_empty();
        let sparse_iter: Vec<_> = if skip_last_sparse {
            sparse.iter().rev().skip(1).collect()
        } else {
            sparse.iter().rev().collect()
        };
        for (&idx, el) in sparse_iter {
            acc = call_callback(vm, &callback, &[acc, el.clone(), JsValue::Number(idx as f64)]);
        }
        // Then iterate dense in reverse.
        let end = if !has_initial && sparse.is_empty() { elements.len() - 1 } else { elements.len() };
        for i in (0..end).rev() {
            acc = call_callback(vm, &callback, &[acc, elements[i].clone(), JsValue::Number(i as f64)]);
        }
        acc
    } else {
        JsValue::Undefined
    }
}

pub fn array_find(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            if val.to_boolean() {
                return el.clone();
            }
        }
        for (&idx, el) in sparse.iter() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
            if val.to_boolean() {
                return el.clone();
            }
        }
    }
    JsValue::Undefined
}

pub fn array_find_index(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            if val.to_boolean() {
                return JsValue::Number(i as f64);
            }
        }
        for (&idx, el) in sparse.iter() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
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
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            if val.to_boolean() {
                return JsValue::Bool(true);
            }
        }
        for (&idx, el) in sparse.iter() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
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
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            if !val.to_boolean() {
                return JsValue::Bool(false);
            }
        }
        for (&idx, el) in sparse.iter() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
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
        let (elements, sparse) = {
            let a = arr.borrow();
            (a.elements.clone(), a.sparse.clone())
        };
        let mut result = Vec::new();
        let do_flat = |result: &mut Vec<JsValue>, val: JsValue| {
            match val {
                JsValue::Array(a) => {
                    let inner = a.borrow();
                    result.extend(inner.elements.iter().cloned());
                }
                _ => result.push(val),
            }
        };
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            do_flat(&mut result, val);
        }
        for (&idx, el) in sparse.iter() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(idx as f64)]);
            do_flat(&mut result, val);
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
        let mut pairs: Vec<JsValue> = a.elements.iter().enumerate().map(|(i, v)| {
            JsValue::new_array(vec![JsValue::Number(i as f64), v.clone()])
        }).collect();
        for (&idx, v) in a.sparse.iter() {
            pairs.push(JsValue::new_array(vec![JsValue::Number(idx as f64), v.clone()]));
        }
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(pairs))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let mut keys: Vec<JsValue> = (0..a.elements.len())
            .map(|i| JsValue::Number(i as f64))
            .collect();
        for &idx in a.sparse.keys() {
            keys.push(JsValue::Number(idx as f64));
        }
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(keys))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let mut vals = a.elements.clone();
        for v in a.sparse.values() {
            vals.push(v.clone());
        }
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
        JsValue::Array(a) => a.borrow().elements.clone(),
        JsValue::String(s) => s.chars().map(|c| {
            let mut buf = String::new();
            buf.push(c);
            JsValue::String(buf)
        }).collect(),
        JsValue::Object(obj) => {
            let tag = obj.borrow().internal_tag.clone();
            match tag.as_deref() {
                // Set: internal __items array
                Some("__set__") => {
                    if let JsValue::Array(items) = obj.borrow().get("__items") {
                        items.borrow().elements.clone()
                    } else { Vec::new() }
                }
                // Map: pairs of [key, value]
                Some("__map__") => {
                    if let (JsValue::Array(keys), JsValue::Array(vals)) =
                        (obj.borrow().get("__keys"), obj.borrow().get("__values"))
                    {
                        let ks = keys.borrow();
                        let vs = vals.borrow();
                        ks.elements.iter().zip(vs.elements.iter()).map(|(k, v)| {
                            JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(vec![k.clone(), v.clone()]))))
                        }).collect()
                    } else { Vec::new() }
                }
                _ => {
                    // Array-like object with .length (NodeList, arguments, etc.)
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
