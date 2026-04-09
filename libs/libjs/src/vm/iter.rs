//! Iterator handling for for-of / for-in loops and array destructuring.
//!
//! Implements the ES2023 Iterator Protocol:
//! - `GetIterator(val)`: calls `val[Symbol.iterator]()` to get an iterator object
//! - `IteratorNext(iter)`: calls `iter.next()` to get `{ value, done }`

use alloc::rc::Rc;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeSet;
use core::cell::RefCell;

use super::native_symbol::WELL_KNOWN_ITERATOR;
use super::Vm;
use super::native_fn;
use crate::value::*;

impl Vm {
    fn collect_for_in_keys_from_object(
        &self,
        obj: &Rc<RefCell<JsObject>>,
        seen: &mut BTreeSet<String>,
        out: &mut Vec<JsValue>,
    ) {
        let borrowed = obj.borrow();
        for key in borrowed.keys() {
            if seen.insert(key.clone()) {
                out.push(JsValue::String(key));
            }
        }
        if let Some(ref proto) = borrowed.prototype {
            let proto = proto.clone();
            drop(borrowed);
            self.collect_for_in_keys_from_object(&proto, seen, out);
        }
    }

    fn collect_for_in_keys(&self, val: &JsValue) -> Vec<JsValue> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        match val {
            JsValue::Object(obj) => {
                self.collect_for_in_keys_from_object(obj, &mut seen, &mut out);
            }
            JsValue::Array(arr) => {
                let a = arr.borrow();
                for idx in a.elements.keys() {
                    let key = idx.to_string();
                    if seen.insert(key.clone()) {
                        out.push(JsValue::String(key));
                    }
                }
                for (key, prop) in &a.properties {
                    if prop.enumerable && seen.insert(key.clone()) {
                        out.push(JsValue::String(key.clone()));
                    }
                }
            }
            JsValue::Function(func) => {
                let f = func.borrow();
                for (key, val) in &f.own_props {
                    if key.starts_with("__get_") || key.starts_with("__set_") || key.starts_with("__desc_") {
                        continue;
                    }
                    let enumerable = match f.own_props.get(&alloc::format!("__desc_enumerable_{}", key)) {
                        Some(JsValue::Bool(v)) => *v,
                        _ => true,
                    };
                    if enumerable && seen.insert(key.clone()) {
                        out.push(JsValue::String(key.clone()));
                    }
                    let _ = val;
                }
            }
            JsValue::String(s) => {
                for (idx, _) in s.chars().enumerate() {
                    out.push(JsValue::String(idx.to_string()));
                }
            }
            _ => {}
        }
        out
    }

    pub fn create_for_in_iterator(&self, val: &JsValue) -> JsValue {
        self.make_internal_iterator(self.collect_for_in_keys(val))
    }

    /// ES2023 §7.4.1 GetIterator(obj).
    ///
    /// 1. Look up `obj[Symbol.iterator]`.
    /// 2. Call it with `this = obj` to get the iterator.
    /// 3. If no Symbol.iterator, fall back to internal iteration for Arrays/Strings.
    ///
    /// The result is an iterator object that has a `.next()` method.
    pub fn create_iterator(&mut self, val: &JsValue) -> JsValue {
        // Symbols are not iterable by default. Since symbols are internally
        // encoded as strings in libjs, guard them here before string-like
        // prototype lookup can accidentally expose String iteration semantics.
        if super::is_symbol_value(val) {
            self.pending_exception = Some(self.make_type_error("Symbol is not iterable"));
            return self.make_internal_iterator(Vec::new());
        }

        // 1. Try Symbol.iterator method
        let iter_fn = self.get_property_invoking_getter(val, WELL_KNOWN_ITERATOR);
        if matches!(iter_fn, JsValue::Empty) {
            return JsValue::Empty;
        }
        if self.pending_exception.is_some() {
            return self.make_internal_iterator(Vec::new());
        }
        if let Some(exc) = self.last_exception.take() {
            self.pending_exception = Some(exc);
            return self.make_internal_iterator(Vec::new());
        }
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
            // The iterator result must be an object with a callable `.next()`.
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
                    let exc =
                        self.make_type_error("Symbol.iterator returned an object without a callable next method");
                    self.pending_exception = Some(exc);
                    return self.make_internal_iterator(Vec::new());
                }
                JsValue::Array(_) | JsValue::Function(_) => {
                    let next = self.get_property_invoking_getter(&iterator, "next");
                    if matches!(next, JsValue::Empty) {
                        return JsValue::Empty;
                    }
                    if self.pending_exception.is_some() {
                        return self.make_internal_iterator(Vec::new());
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return self.make_internal_iterator(Vec::new());
                    }
                    if next.is_function() {
                        return iterator;
                    }
                    let exc =
                        self.make_type_error("Symbol.iterator returned an object without a callable next method");
                    self.pending_exception = Some(exc);
                    return self.make_internal_iterator(Vec::new());
                }
                _ => {
                    let exc = self.make_type_error("Symbol.iterator returned a non-object value");
                    self.pending_exception = Some(exc);
                    return self.make_internal_iterator(Vec::new());
                }
            }
        }

        // 2. ES2023 §7.4.1: without a callable @@iterator, the value is not iterable.
        let val_str = val.to_js_string();
        let msg = alloc::format!("{} is not iterable", val_str);
        let exc = self.make_type_error(&msg);
        self.pending_exception = Some(exc);
        self.make_internal_iterator(Vec::new())
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
        iter_obj.set(String::from("next"), native_fn("next", iterator_next));
        iter_obj.set(
            String::from(WELL_KNOWN_ITERATOR),
            native_fn("[Symbol.iterator]", iterator_self),
        );
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
                    let _ = obj;
                    let next_fn = self.get_property_invoking_getter(&iter, "next");
                    if matches!(next_fn, JsValue::Empty) {
                        return (JsValue::Empty, false);
                    }
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }
                    if !next_fn.is_function() {
                        self.pending_exception =
                            Some(self.make_type_error("Iterator protocol violation: missing next method"));
                        return (JsValue::Undefined, false);
                    }
                    // Call next() with this=iterator
                    let result = self.call_value(&next_fn, &[], iter.clone());
                    if matches!(result, JsValue::Empty) {
                        return (JsValue::Empty, false);
                    }
                    // Propagate exceptions from next() call
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }
                    if !result.is_object() && !result.is_array() && !result.is_function() {
                        self.pending_exception = Some(
                            self.make_type_error("Iterator protocol violation: next() returned a non-object value"),
                        );
                        return (JsValue::Undefined, false);
                    }

                    // Extract { value, done } from the result
                    let done_val = self.get_property_invoking_getter(&result, "done");
                    if matches!(done_val, JsValue::Empty) {
                        return (JsValue::Empty, false);
                    }
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }
                    let done = done_val.to_boolean();
                    if done {
                        return (JsValue::Undefined, false);
                    }
                    let value = self.get_property_invoking_getter(&result, "value");
                    if matches!(value, JsValue::Empty) {
                        return (JsValue::Empty, false);
                    }
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }
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
                    let next_fn = self.get_property_invoking_getter(iter, "next");
                    if matches!(next_fn, JsValue::Empty) {
                        return (JsValue::Empty, false);
                    }
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }
                    if !next_fn.is_function() {
                        self.pending_exception =
                            Some(self.make_type_error("Iterator protocol violation: missing next method"));
                        return (JsValue::Undefined, false);
                    }
                    let result = self.call_value(&next_fn, &[], iter.clone());
                    if matches!(result, JsValue::Empty) {
                        return (JsValue::Empty, false);
                    }
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }
                    if !result.is_object() && !result.is_array() && !result.is_function() {
                        self.pending_exception = Some(
                            self.make_type_error("Iterator protocol violation: next() returned a non-object value"),
                        );
                        return (JsValue::Undefined, false);
                    }
                    let done_val = self.get_property_invoking_getter(&result, "done");
                    if matches!(done_val, JsValue::Empty) {
                        return (JsValue::Empty, false);
                    }
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }
                    let done = done_val.to_boolean();
                    if done {
                        return (JsValue::Undefined, false);
                    }
                    let value = self.get_property_invoking_getter(&result, "value");
                    if matches!(value, JsValue::Empty) {
                        return (JsValue::Empty, false);
                    }
                    if self.pending_exception.is_some() {
                        return (JsValue::Undefined, false);
                    }
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                        return (JsValue::Undefined, false);
                    }
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
            vm.pending_exception =
                Some(vm.make_type_error("Iterator protocol violation: missing next method"));
            break;
        }
        let result = vm.call_value(&next_fn, &[], iter.clone());
        if vm.pending_exception.is_some() {
            break;
        }
        if !result.is_object() && !result.is_array() && !result.is_function() {
            vm.pending_exception = Some(
                vm.make_type_error("Iterator protocol violation: next() returned a non-object value"),
            );
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

fn iterator_next(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if let JsValue::Object(obj) = &this {
        let mut o = obj.borrow_mut();
        let index = o.get("__index__").to_number() as usize;
        let items = o.get("__items__");
        if let JsValue::Array(arr) = &items {
            let a = arr.borrow();
            if index < a.length {
                let val = a.get(index);
                o.properties.insert(
                    String::from("__index__"),
                    Property::data(JsValue::Number((index + 1) as f64)),
                );
                drop(o);
                let result = JsValue::new_object();
                result.set_property(String::from("value"), val);
                result.set_property(String::from("done"), JsValue::Bool(false));
                return result;
            }
        }
    }
    let result = JsValue::new_object();
    result.set_property(String::from("value"), JsValue::Undefined);
    result.set_property(String::from("done"), JsValue::Bool(true));
    result
}

fn iterator_self(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
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
