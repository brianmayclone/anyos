//! Iterator handling for for-of / for-in loops and array destructuring.
//!
//! Implements the ES2023 Iterator Protocol:
//! - `GetIterator(val)`: calls `val[Symbol.iterator]()` to get an iterator object
//! - `IteratorNext(iter)`: calls `iter.next()` to get `{ value, done }`

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::native_symbol::WELL_KNOWN_ITERATOR;
use super::Vm;
use super::native_fn;
use crate::value::*;

impl Vm {
    /// ES2023 §7.4.1 GetIterator(obj).
    ///
    /// 1. Look up `obj[Symbol.iterator]`.
    /// 2. Call it with `this = obj` to get the iterator.
    /// 3. If no Symbol.iterator, fall back to internal iteration for Arrays/Strings.
    ///
    /// The result is an iterator object that has a `.next()` method.
    pub fn create_iterator(&mut self, val: &JsValue) -> JsValue {
        // 1. Try Symbol.iterator method
        let iter_fn = self.get_property_with_proto(val, WELL_KNOWN_ITERATOR);
        if iter_fn.is_function() {
            // Call val[Symbol.iterator]() with this=val
            let iterator = self.call_value(&iter_fn, &[], val.clone());
            // Propagate exceptions from Symbol.iterator() call
            if let Some(exc) = self.last_exception.take() {
                self.pending_exception = Some(exc);
            }
            if self.pending_exception.is_some() {
                return self.make_internal_iterator(Vec::new());
            }
            // If the result is a proper iterator (has .next method), use it directly.
            // Otherwise (e.g. Array returned from Map.entries), wrap in internal iterator.
            match &iterator {
                JsValue::Object(obj) => {
                    let has_next = {
                        let o = obj.borrow();
                        matches!(o.get("next"), JsValue::Function(_))
                    };
                    let is_internal = obj.borrow().internal_tag.as_deref() == Some("__iterator__");
                    if has_next || is_internal {
                        return iterator;
                    }
                    // Object without .next — not a spec-compliant iterator, wrap it
                    let items = obj
                        .borrow()
                        .keys()
                        .into_iter()
                        .map(JsValue::String)
                        .collect();
                    return self.make_internal_iterator(items);
                }
                JsValue::Array(arr) => {
                    // Array returned — convert to internal iterator over elements
                    let items = arr.borrow().to_dense_vec();
                    return self.make_internal_iterator(items);
                }
                _ => return iterator,
            }
        }

        // 2. Fallback: create internal iterator for built-in types
        let items: Vec<JsValue> = match val {
            JsValue::Array(arr) => arr.borrow().to_dense_vec(),
            JsValue::String(s) => s
                .chars()
                .map(|c| {
                    let mut cs = String::new();
                    cs.push(c);
                    JsValue::String(cs)
                })
                .collect(),
            JsValue::Object(obj) => {
                // For-in semantics: iterate over keys
                obj.borrow()
                    .keys()
                    .into_iter()
                    .map(JsValue::String)
                    .collect()
            }
            _ => {
                // ES2023 §7.4.1: non-iterable values throw TypeError
                let type_str = val.type_of();
                let val_str = val.to_js_string();
                let msg = alloc::format!("{} is not iterable", val_str);
                let exc = self.make_type_error(&msg);
                self.pending_exception = Some(exc);
                return self.make_internal_iterator(Vec::new());
            }
        };

        self.make_internal_iterator(items)
    }

    /// Create an internal iterator object from a Vec of values.
    /// Used as fallback when Symbol.iterator is not available.
    pub fn make_internal_iterator(&self, items: Vec<JsValue>) -> JsValue {
        let mut iter_obj = JsObject::with_tag("__iterator__");
        iter_obj.set(
            String::from("__items__"),
            JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(items)))),
        );
        iter_obj.set(String::from("__index__"), JsValue::Number(0.0));
        // ES2025 Iterator Helper methods
        iter_obj.set(String::from("toArray"), native_fn("toArray", iterator_to_array));
        iter_obj.set(String::from("forEach"), native_fn("forEach", iterator_for_each));
        iter_obj.set(String::from("map"), native_fn("map", iterator_map));
        iter_obj.set(String::from("filter"), native_fn("filter", iterator_filter));
        iter_obj.set(String::from("take"), native_fn("take", iterator_take));
        iter_obj.set(String::from("drop"), native_fn("drop", iterator_drop));
        iter_obj.set(String::from("some"), native_fn("some", iterator_some));
        iter_obj.set(String::from("every"), native_fn("every", iterator_every));
        iter_obj.set(String::from("find"), native_fn("find", iterator_find));
        iter_obj.set(String::from("reduce"), native_fn("reduce", iterator_reduce));
        iter_obj.set(String::from("flatMap"), native_fn("flatMap", iterator_flat_map));
        JsValue::Object(Rc::new(RefCell::new(iter_obj)))
    }

    /// ES2023 §7.4.2 IteratorNext(iterator).
    ///
    /// Calls `iterator.next()` and returns `(value, done)`.
    /// For internal iterators (tagged `__iterator__`), uses direct array indexing.
    /// For spec-compliant iterators, calls the `.next()` method.
    pub fn iter_next_mut(&mut self) -> (JsValue, bool) {
        let iter = match self.stack.last() {
            Some(v) => v.clone(),
            None => return (JsValue::Undefined, false),
        };

        match &iter {
            JsValue::Object(obj) => {
                let is_internal = obj.borrow().internal_tag.as_deref() == Some("__iterator__");

                if is_internal {
                    // Fast path: internal iterator with __items__ + __index__
                    let mut o = obj.borrow_mut();
                    let index = match o.properties.get("__index__") {
                        Some(p) => p.value.to_number() as usize,
                        None => return (JsValue::Undefined, false),
                    };
                    let items_val = match o.properties.get("__items__") {
                        Some(p) => p.value.clone(),
                        None => return (JsValue::Undefined, false),
                    };
                    match &items_val {
                        JsValue::Array(arr) => {
                            let a = arr.borrow();
                            if index < a.length {
                                let val = a.get(index);
                                o.properties.insert(
                                    String::from("__index__"),
                                    Property::data(JsValue::Number((index + 1) as f64)),
                                );
                                (val, true) // has_more = true (value is valid)
                            } else {
                                (JsValue::Undefined, false)
                            }
                        }
                        _ => (JsValue::Undefined, false),
                    }
                } else {
                    // Spec path: call iterator.next()
                    drop(obj);
                    let next_fn = self.get_property_with_proto(&iter, "next");
                    if !next_fn.is_function() {
                        return (JsValue::Undefined, false);
                    }
                    // Call next() with this=iterator
                    let result = self.call_value(&next_fn, &[], iter.clone());
                    // Propagate exceptions from next() call
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }

                    // Extract { value, done } from the result
                    let done = self.get_property_with_proto(&result, "done").to_boolean();
                    if done {
                        return (JsValue::Undefined, false);
                    }
                    let value = self.get_property_with_proto(&result, "value");
                    (value, true)
                }
            }
            _ => (JsValue::Undefined, false),
        }
    }

    /// Like `iter_next_mut` but takes the iterator explicitly (not from stack).
    pub fn iter_next_for(&mut self, iter: &JsValue) -> (JsValue, bool) {
        match iter {
            JsValue::Object(obj) => {
                let is_internal = obj.borrow().internal_tag.as_deref() == Some("__iterator__");
                if is_internal {
                    let mut o = obj.borrow_mut();
                    let index = match o.properties.get("__index__") {
                        Some(p) => p.value.to_number() as usize,
                        None => return (JsValue::Undefined, false),
                    };
                    let items_val = match o.properties.get("__items__") {
                        Some(p) => p.value.clone(),
                        None => return (JsValue::Undefined, false),
                    };
                    match &items_val {
                        JsValue::Array(arr) => {
                            let a = arr.borrow();
                            if index < a.length {
                                let val = a.get(index);
                                o.properties.insert(
                                    String::from("__index__"),
                                    Property::data(JsValue::Number((index + 1) as f64)),
                                );
                                (val, true)
                            } else {
                                (JsValue::Undefined, false)
                            }
                        }
                        _ => (JsValue::Undefined, false),
                    }
                } else {
                    let next_fn = self.get_property_with_proto(iter, "next");
                    if !next_fn.is_function() {
                        return (JsValue::Undefined, false);
                    }
                    let result = self.call_value(&next_fn, &[], iter.clone());
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }
                    let done = self.get_property_with_proto(&result, "done").to_boolean();
                    if done {
                        return (JsValue::Undefined, false);
                    }
                    let value = self.get_property_with_proto(&result, "value");
                    (value, true)
                }
            }
            _ => (JsValue::Undefined, false),
        }
    }
}

// ═══════════════════════════════════════════════════════════
// ES2025 Iterator Helpers — Iterator.prototype methods
// ═══════════════════════════════════════════════════════════

/// Collect all remaining values from an iterator into a Vec.
fn drain_iterator(vm: &mut Vm, iter: &JsValue) -> Vec<JsValue> {
    // Fast path: internal iterator (tagged "__iterator__") — read __items__ directly.
    if let JsValue::Object(obj) = iter {
        let is_internal = obj.borrow().internal_tag.as_deref() == Some("__iterator__");
        if is_internal {
            let o = obj.borrow();
            let index = o
                .properties
                .get("__index__")
                .map(|p| p.value.to_number() as usize)
                .unwrap_or(0);
            if let Some(p) = o.properties.get("__items__") {
                if let JsValue::Array(arr) = &p.value {
                    let a = arr.borrow();
                    let len = a.length;
                    let remaining: Vec<JsValue> = (index..len)
                        .map(|i| a.get(i))
                        .collect();
                    drop(a);
                    drop(o);
                    // Advance index to end.
                    obj.borrow_mut().properties.insert(
                        String::from("__index__"),
                        Property::data(JsValue::Number(len as f64)),
                    );
                    return remaining;
                }
            }
            return Vec::new();
        }
    }

    // Slow path: spec-compliant iterator with .next() method.
    let mut items = Vec::new();
    let mut safety = 0u32;
    loop {
        let next_fn = vm.get_property_with_proto(iter, "next");
        if !next_fn.is_function() {
            break;
        }
        let result = vm.call_value(&next_fn, &[], iter.clone());
        if vm.pending_exception.is_some() {
            break;
        }
        let done = vm.get_property_with_proto(&result, "done").to_boolean();
        if done {
            break;
        }
        items.push(vm.get_property_with_proto(&result, "value"));
        safety += 1;
        if safety > 1_000_000 {
            break;
        }
    }
    items
}

/// `Iterator.prototype.toArray()`
pub fn iterator_to_array(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    JsValue::new_array(items)
}

/// `Iterator.prototype.forEach(fn)`
pub fn iterator_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    for (i, v) in items.iter().enumerate() {
        vm.call_value(&callback, &[v.clone(), JsValue::Number(i as f64)], JsValue::Undefined);
    }
    JsValue::Undefined
}

/// `Iterator.prototype.map(fn)`
pub fn iterator_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    let mapped: Vec<JsValue> = items
        .iter()
        .enumerate()
        .map(|(i, v)| vm.call_value(&callback, &[v.clone(), JsValue::Number(i as f64)], JsValue::Undefined))
        .collect();
    vm.make_internal_iterator(mapped)
}

/// `Iterator.prototype.filter(fn)`
pub fn iterator_filter(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    let filtered: Vec<JsValue> = items
        .into_iter()
        .enumerate()
        .filter(|(i, v)| {
            vm.call_value(&callback, &[v.clone(), JsValue::Number(*i as f64)], JsValue::Undefined)
                .to_boolean()
        })
        .map(|(_, v)| v)
        .collect();
    vm.make_internal_iterator(filtered)
}

/// `Iterator.prototype.take(limit)`
pub fn iterator_take(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let limit = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    let taken: Vec<JsValue> = items.into_iter().take(limit).collect();
    vm.make_internal_iterator(taken)
}

/// `Iterator.prototype.drop(count)`
pub fn iterator_drop(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let count = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    let dropped: Vec<JsValue> = items.into_iter().skip(count).collect();
    vm.make_internal_iterator(dropped)
}

/// `Iterator.prototype.some(fn)`
pub fn iterator_some(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    for (i, v) in items.iter().enumerate() {
        if vm.call_value(&callback, &[v.clone(), JsValue::Number(i as f64)], JsValue::Undefined).to_boolean() {
            return JsValue::Bool(true);
        }
    }
    JsValue::Bool(false)
}

/// `Iterator.prototype.every(fn)`
pub fn iterator_every(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    for (i, v) in items.iter().enumerate() {
        if !vm.call_value(&callback, &[v.clone(), JsValue::Number(i as f64)], JsValue::Undefined).to_boolean() {
            return JsValue::Bool(false);
        }
    }
    JsValue::Bool(true)
}

/// `Iterator.prototype.find(fn)`
pub fn iterator_find(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    for (i, v) in items.iter().enumerate() {
        if vm.call_value(&callback, &[v.clone(), JsValue::Number(i as f64)], JsValue::Undefined).to_boolean() {
            return v.clone();
        }
    }
    JsValue::Undefined
}

/// `Iterator.prototype.reduce(fn, initial)`
pub fn iterator_reduce(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    let mut acc = args.get(1).cloned().unwrap_or_else(|| {
        if items.is_empty() {
            JsValue::Undefined
        } else {
            items[0].clone()
        }
    });
    let start = if args.len() >= 2 { 0 } else { 1 };
    for (i, v) in items.iter().enumerate().skip(start) {
        acc = vm.call_value(&callback, &[acc, v.clone(), JsValue::Number(i as f64)], JsValue::Undefined);
    }
    acc
}

/// `Iterator.prototype.flatMap(fn)`
pub fn iterator_flat_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let iter = vm.current_this.clone();
    let items = drain_iterator(vm, &iter);
    let mut result = Vec::new();
    for (i, v) in items.iter().enumerate() {
        let mapped = vm.call_value(&callback, &[v.clone(), JsValue::Number(i as f64)], JsValue::Undefined);
        match &mapped {
            JsValue::Array(arr) => {
                result.extend(arr.borrow().to_dense_vec());
            }
            _ => result.push(mapped),
        }
    }
    vm.make_internal_iterator(result)
}
