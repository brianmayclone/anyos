//! Iterator handling for for-of / for-in loops.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::value::*;
use super::Vm;

impl Vm {
    /// Create an iterator object from a value.
    /// Stores __items__ (array) and __index__ (number) on an internal object.
    pub fn create_iterator(&self, val: &JsValue) -> JsValue {
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
                obj.borrow().keys().into_iter().map(JsValue::String).collect()
            }
            _ => Vec::new(),
        };

        let mut iter_obj = JsObject::with_tag("__iterator__");
        iter_obj.set(
            String::from("__items__"),
            JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(items)))),
        );
        iter_obj.set(String::from("__index__"), JsValue::Number(0.0));
        JsValue::Object(Rc::new(RefCell::new(iter_obj)))
    }

    /// Advance the iterator that sits on top of the stack (mutates in-place).
    /// Returns (value, has_more).
    pub fn iter_next_mut(&mut self) -> (JsValue, bool) {
        let iter = match self.stack.last() {
            Some(v) => v.clone(),
            None => return (JsValue::Undefined, false),
        };

        match &iter {
            JsValue::Object(obj) => {
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
                            // Advance index
                            o.set(String::from("__index__"), JsValue::Number((index + 1) as f64));
                            (val, true) // has_more = true
                        } else {
                            (JsValue::Undefined, false)
                        }
                    }
                    _ => (JsValue::Undefined, false),
                }
            }
            _ => (JsValue::Undefined, false),
        }
    }
}
