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
        let mut a = arr.borrow_mut();
        for arg in args {
            a.elements.push(arg.clone());
        }
        JsValue::Number(a.elements.len() as f64)
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
        let count = (end - start).min(len - target);
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
        let from = args.get(1).map(|v| v.to_number() as usize).unwrap_or(0);
        for i in from..a.elements.len() {
            if a.elements[i].strict_eq(&search) {
                return JsValue::Number(i as f64);
            }
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_last_index_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let search = args.first().cloned().unwrap_or(JsValue::Undefined);
        let len = a.elements.len();
        let from = args.get(1).map(|v| {
            let n = v.to_number() as i64;
            if n < 0 { (len as i64 + n).max(0) as usize } else { (n as usize).min(len - 1) }
        }).unwrap_or(if len > 0 { len - 1 } else { 0 });
        if len == 0 { return JsValue::Number(-1.0); }
        for i in (0..=from).rev() {
            if a.elements[i].strict_eq(&search) {
                return JsValue::Number(i as f64);
            }
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_includes(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let search = args.first().cloned().unwrap_or(JsValue::Undefined);
        let from = args.get(1).map(|v| resolve_index(v.to_number(), a.elements.len())).unwrap_or(0);
        for i in from..a.elements.len() {
            if a.elements[i].strict_eq(&search) {
                return JsValue::Bool(true);
            }
            // NaN check: NaN !== NaN but includes should find NaN
            if let (JsValue::Number(a_n), JsValue::Number(s_n)) = (&a.elements[i], &search) {
                if a_n.is_nan() && s_n.is_nan() {
                    return JsValue::Bool(true);
                }
            }
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
        let elements = arr.borrow().elements.clone();
        let mut result = Vec::with_capacity(elements.len());
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            result.push(val);
        }
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(result))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_filter(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let elements = arr.borrow().elements.clone();
        let mut result = Vec::new();
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
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
        let elements = arr.borrow().elements.clone();
        for (i, el) in elements.iter().enumerate() {
            call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
        }
    }
    JsValue::Undefined
}

pub fn array_reduce(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let elements = arr.borrow().elements.clone();
        let mut start_idx = 0;
        let mut acc = if args.len() > 1 {
            args[1].clone()
        } else {
            if elements.is_empty() { return JsValue::Undefined; }
            start_idx = 1;
            elements[0].clone()
        };
        for i in start_idx..elements.len() {
            acc = call_callback(vm, &callback, &[acc, elements[i].clone(), JsValue::Number(i as f64)]);
        }
        acc
    } else {
        JsValue::Undefined
    }
}

pub fn array_reduce_right(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let elements = arr.borrow().elements.clone();
        let len = elements.len();
        if len == 0 && args.len() <= 1 { return JsValue::Undefined; }
        let mut acc = if args.len() > 1 {
            args[1].clone()
        } else {
            elements[len - 1].clone()
        };
        let end = if args.len() > 1 { len } else { len - 1 };
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
        let elements = arr.borrow().elements.clone();
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
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
        let elements = arr.borrow().elements.clone();
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            if val.to_boolean() {
                return JsValue::Number(i as f64);
            }
        }
    }
    JsValue::Number(-1.0)
}

pub fn array_some(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(arr) = this_array(vm) {
        let elements = arr.borrow().elements.clone();
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
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
        let elements = arr.borrow().elements.clone();
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
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
        let elements = arr.borrow().elements.clone();
        let mut result = Vec::new();
        for (i, el) in elements.iter().enumerate() {
            let val = call_callback(vm, &callback, &[el.clone(), JsValue::Number(i as f64)]);
            match val {
                JsValue::Array(a) => {
                    let inner = a.borrow();
                    result.extend(inner.elements.iter().cloned());
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
        let pairs: Vec<JsValue> = a.elements.iter().enumerate().map(|(i, v)| {
            JsValue::new_array(vec![JsValue::Number(i as f64), v.clone()])
        }).collect();
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(pairs))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        let keys: Vec<JsValue> = (0..a.elements.len())
            .map(|i| JsValue::Number(i as f64))
            .collect();
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(keys))))
    } else {
        JsValue::new_array(Vec::new())
    }
}

pub fn array_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(arr) = this_array(vm) {
        let a = arr.borrow();
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(a.elements.clone()))))
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
