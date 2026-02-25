//! Map and Set built-in objects.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::value::*;
use super::{Vm, native_fn};

// ═══════════════════════════════════════════════════════════
// Map constructor and prototype
// ═══════════════════════════════════════════════════════════

pub fn ctor_map(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let mut obj = JsObject::new();
    obj.internal_tag = Some(String::from("__map__"));
    obj.set(String::from("__keys"), JsValue::new_array(Vec::new()));
    obj.set(String::from("__values"), JsValue::new_array(Vec::new()));
    obj.set(String::from("size"), JsValue::Number(0.0));
    // Install methods
    obj.set(String::from("set"), native_fn("set", map_set));
    obj.set(String::from("get"), native_fn("get", map_get));
    obj.set(String::from("has"), native_fn("has", map_has));
    obj.set(String::from("delete"), native_fn("delete", map_delete));
    obj.set(String::from("clear"), native_fn("clear", map_clear));
    obj.set(String::from("keys"), native_fn("keys", map_keys));
    obj.set(String::from("values"), native_fn("values", map_values));
    obj.set(String::from("entries"), native_fn("entries", map_entries));
    obj.set(String::from("forEach"), native_fn("forEach", map_for_each));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn map_find_index(obj: &JsObject, key: &JsValue) -> Option<usize> {
    if let JsValue::Array(keys) = obj.get("__keys") {
        let k = keys.borrow();
        for (i, k_val) in k.elements.iter().enumerate() {
            if k_val.strict_eq(key) {
                return Some(i);
            }
        }
    }
    None
}

fn update_size(obj_rc: &Rc<RefCell<JsObject>>) {
    let size = {
        let o = obj_rc.borrow();
        if let JsValue::Array(keys) = o.get("__keys") {
            keys.borrow().elements.len() as f64
        } else {
            0.0
        }
    };
    obj_rc.borrow_mut().set(String::from("size"), JsValue::Number(size));
}

pub fn map_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let this = vm.current_this.clone();

    if let JsValue::Object(obj_rc) = &this {
        let (keys_arr, vals_arr, existing_idx) = {
            let o = obj_rc.borrow();
            let idx = map_find_index(&o, &key);
            (o.get("__keys"), o.get("__values"), idx)
        };
        if let (JsValue::Array(keys), JsValue::Array(vals)) = (keys_arr, vals_arr) {
            if let Some(idx) = existing_idx {
                vals.borrow_mut().elements[idx] = value;
            } else {
                keys.borrow_mut().elements.push(key);
                vals.borrow_mut().elements.push(value);
            }
        }
        update_size(obj_rc);
    }
    this
}

pub fn map_get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let Some(idx) = map_find_index(&o, &key) {
            if let JsValue::Array(vals) = o.get("__values") {
                let v = vals.borrow();
                return v.elements.get(idx).cloned().unwrap_or(JsValue::Undefined);
            }
        }
    }
    JsValue::Undefined
}

pub fn map_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        return JsValue::Bool(map_find_index(&o, &key).is_some());
    }
    JsValue::Bool(false)
}

pub fn map_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let idx = {
            let o = obj_rc.borrow();
            map_find_index(&o, &key)
        };
        if let Some(idx) = idx {
            let o = obj_rc.borrow();
            if let (JsValue::Array(keys), JsValue::Array(vals)) = (o.get("__keys"), o.get("__values")) {
                keys.borrow_mut().elements.remove(idx);
                vals.borrow_mut().elements.remove(idx);
            }
            drop(o);
            update_size(obj_rc);
            return JsValue::Bool(true);
        }
    }
    JsValue::Bool(false)
}

pub fn map_clear(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(keys) = o.get("__keys") { keys.borrow_mut().elements.clear(); }
        if let JsValue::Array(vals) = o.get("__values") { vals.borrow_mut().elements.clear(); }
        drop(o);
        update_size(obj_rc);
    }
    JsValue::Undefined
}

pub fn map_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(keys) = o.get("__keys") {
            return JsValue::new_array(keys.borrow().elements.clone());
        }
    }
    JsValue::new_array(Vec::new())
}

pub fn map_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(vals) = o.get("__values") {
            return JsValue::new_array(vals.borrow().elements.clone());
        }
    }
    JsValue::new_array(Vec::new())
}

pub fn map_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let (JsValue::Array(keys), JsValue::Array(vals)) = (o.get("__keys"), o.get("__values")) {
            let k = keys.borrow();
            let v = vals.borrow();
            let entries: Vec<JsValue> = k.elements.iter().zip(v.elements.iter())
                .map(|(key, val)| JsValue::new_array(alloc::vec![key.clone(), val.clone()]))
                .collect();
            return JsValue::new_array(entries);
        }
    }
    JsValue::new_array(Vec::new())
}

pub fn map_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let (keys, vals) = {
            let o = obj_rc.borrow();
            let k = if let JsValue::Array(arr) = o.get("__keys") { arr.borrow().elements.clone() } else { Vec::new() };
            let v = if let JsValue::Array(arr) = o.get("__values") { arr.borrow().elements.clone() } else { Vec::new() };
            (k, v)
        };
        for (i, (k, v)) in keys.iter().zip(vals.iter()).enumerate() {
            let _ = i;
            super::native_array::call_callback_pub(vm, &callback, &[v.clone(), k.clone()]);
        }
    }
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════════
// Set constructor and prototype
// ═══════════════════════════════════════════════════════════

pub fn ctor_set(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let mut obj = JsObject::new();
    obj.internal_tag = Some(String::from("__set__"));
    obj.set(String::from("__items"), JsValue::new_array(Vec::new()));
    obj.set(String::from("size"), JsValue::Number(0.0));
    obj.set(String::from("add"), native_fn("add", set_add));
    obj.set(String::from("has"), native_fn("has", set_has));
    obj.set(String::from("delete"), native_fn("delete", set_delete));
    obj.set(String::from("clear"), native_fn("clear", set_clear));
    obj.set(String::from("keys"), native_fn("keys", set_values));
    obj.set(String::from("values"), native_fn("values", set_values));
    obj.set(String::from("entries"), native_fn("entries", set_entries));
    obj.set(String::from("forEach"), native_fn("forEach", set_for_each));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn set_find_index(obj: &JsObject, value: &JsValue) -> Option<usize> {
    if let JsValue::Array(items) = obj.get("__items") {
        let arr = items.borrow();
        for (i, v) in arr.elements.iter().enumerate() {
            if v.strict_eq(value) { return Some(i); }
        }
    }
    None
}

fn update_set_size(obj_rc: &Rc<RefCell<JsObject>>) {
    let size = {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            items.borrow().elements.len() as f64
        } else { 0.0 }
    };
    obj_rc.borrow_mut().set(String::from("size"), JsValue::Number(size));
}

pub fn set_add(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this = vm.current_this.clone();
    if let JsValue::Object(obj_rc) = &this {
        let already = { obj_rc.borrow() }.get("__items");
        if let JsValue::Array(items) = already {
            let has = items.borrow().elements.iter().any(|v| v.strict_eq(&value));
            if !has {
                items.borrow_mut().elements.push(value);
            }
        }
        update_set_size(obj_rc);
    }
    this
}

pub fn set_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        return JsValue::Bool(set_find_index(&o, &value).is_some());
    }
    JsValue::Bool(false)
}

pub fn set_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let idx = { set_find_index(&obj_rc.borrow(), &value) };
        if let Some(idx) = idx {
            let o = obj_rc.borrow();
            if let JsValue::Array(items) = o.get("__items") {
                items.borrow_mut().elements.remove(idx);
            }
            drop(o);
            update_set_size(obj_rc);
            return JsValue::Bool(true);
        }
    }
    JsValue::Bool(false)
}

pub fn set_clear(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") { items.borrow_mut().elements.clear(); }
        drop(o);
        update_set_size(obj_rc);
    }
    JsValue::Undefined
}

pub fn set_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            return JsValue::new_array(items.borrow().elements.clone());
        }
    }
    JsValue::new_array(Vec::new())
}

pub fn set_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            let entries: Vec<JsValue> = items.borrow().elements.iter()
                .map(|v| JsValue::new_array(alloc::vec![v.clone(), v.clone()]))
                .collect();
            return JsValue::new_array(entries);
        }
    }
    JsValue::new_array(Vec::new())
}

pub fn set_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let items = {
            let o = obj_rc.borrow();
            if let JsValue::Array(arr) = o.get("__items") { arr.borrow().elements.clone() } else { Vec::new() }
        };
        for v in &items {
            super::native_array::call_callback_pub(vm, &callback, &[v.clone(), v.clone()]);
        }
    }
    JsValue::Undefined
}
