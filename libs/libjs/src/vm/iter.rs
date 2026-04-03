//! Iterator handling for for-of / for-in loops and array destructuring.
//!
//! Implements the ES2023 Iterator Protocol:
//! - `GetIterator(val)`: calls `val[Symbol.iterator]()` to get an iterator object
//! - `IteratorNext(iter)`: calls `iter.next()` to get `{ value, done }`

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::value::*;
use super::Vm;
use super::native_symbol::WELL_KNOWN_ITERATOR;

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
                    let items = obj.borrow().keys().into_iter().map(JsValue::String).collect();
                    return self.make_internal_iterator(items);
                }
                JsValue::Array(arr) => {
                    // Array returned — convert to internal iterator over elements
                    let items = arr.borrow().elements.clone();
                    return self.make_internal_iterator(items);
                }
                _ => return iterator,
            }
        }

        // 2. Fallback: create internal iterator for built-in types
        let items: Vec<JsValue> = match val {
            JsValue::Array(arr) => {
                arr.borrow().elements.clone()
            }
            JsValue::String(s) => {
                s.chars().map(|c| {
                    let mut cs = String::new();
                    cs.push(c);
                    JsValue::String(cs)
                }).collect()
            }
            JsValue::Object(obj) => {
                // For-in semantics: iterate over keys
                obj.borrow().keys().into_iter().map(JsValue::String).collect()
            }
            _ => Vec::new(),
        };

        self.make_internal_iterator(items)
    }

    /// Create an internal iterator object from a Vec of values.
    /// Used as fallback when Symbol.iterator is not available.
    fn make_internal_iterator(&self, items: Vec<JsValue>) -> JsValue {
        let mut iter_obj = JsObject::with_tag("__iterator__");
        iter_obj.set(
            String::from("__items__"),
            JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(items)))),
        );
        iter_obj.set(String::from("__index__"), JsValue::Number(0.0));
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
                            if index < a.elements.len() {
                                let val = a.elements[index].clone();
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
}
