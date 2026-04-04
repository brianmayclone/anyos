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

pub fn ctor_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Tag `this` (created by `new_object` with the correct prototype chain).
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let mut o = obj_rc.borrow_mut();
        o.internal_tag = Some(String::from("__map__"));
        o.set(String::from("__keys"), JsValue::new_array(Vec::new()));
        o.set(String::from("__values"), JsValue::new_array(Vec::new()));
        o.set(String::from("size"), JsValue::Number(0.0));
    }
    // Handle iterable argument: new Map([[k1,v1],[k2,v2]])
    if let Some(iterable) = args.first() {
        if let JsValue::Array(arr) = iterable {
            let entries = arr.borrow().to_dense_vec();
            for entry in &entries {
                if let JsValue::Array(pair) = entry {
                    let p = pair.borrow();
                    let key = p.get(0);
                    let val = p.get(1);
                    if let JsValue::Object(obj_rc) = &vm.current_this {
                        let (keys_arr, vals_arr) = {
                            let ob = obj_rc.borrow();
                            (ob.get("__keys"), ob.get("__values"))
                        };
                        if let (JsValue::Array(keys), JsValue::Array(vals)) = (keys_arr, vals_arr) {
                            keys.borrow_mut().push(key);
                            vals.borrow_mut().push(val);
                        }
                        update_size(obj_rc);
                    }
                }
            }
        }
    }
    JsValue::Undefined // Return undefined → new_object uses this
}

fn map_find_index(obj: &JsObject, key: &JsValue) -> Option<usize> {
    if let JsValue::Array(keys) = obj.get("__keys") {
        let k = keys.borrow();
        for (&i, k_val) in k.elements.iter() {
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
            keys.borrow().count() as f64
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
                vals.borrow_mut().set(idx, value);
            } else {
                keys.borrow_mut().push(key);
                vals.borrow_mut().push(value);
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
                return vals.borrow().get(idx);
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
                keys.borrow_mut().remove_and_shift(idx);
                vals.borrow_mut().remove_and_shift(idx);
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
        if let JsValue::Array(keys) = o.get("__keys") { keys.borrow_mut().clear(); }
        if let JsValue::Array(vals) = o.get("__values") { vals.borrow_mut().clear(); }
        drop(o);
        update_size(obj_rc);
    }
    JsValue::Undefined
}

pub fn map_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(keys) = o.get("__keys") {
            return JsValue::new_array(keys.borrow().values_vec());
        }
    }
    JsValue::new_array(Vec::new())
}

pub fn map_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(vals) = o.get("__values") {
            return JsValue::new_array(vals.borrow().values_vec());
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
            let kv: Vec<_> = k.elements.values().cloned().collect();
            let vv: Vec<_> = v.elements.values().cloned().collect();
            let entries: Vec<JsValue> = kv.into_iter().zip(vv.into_iter())
                .map(|(key, val)| JsValue::new_array(alloc::vec![key, val]))
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
            let k = if let JsValue::Array(arr) = o.get("__keys") { arr.borrow().values_vec() } else { Vec::new() };
            let v = if let JsValue::Array(arr) = o.get("__values") { arr.borrow().values_vec() } else { Vec::new() };
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

pub fn ctor_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let mut o = obj_rc.borrow_mut();
        o.internal_tag = Some(String::from("__set__"));
        o.set(String::from("__items"), JsValue::new_array(Vec::new()));
        o.set(String::from("size"), JsValue::Number(0.0));
    }
    // Pre-populate from iterable argument (array) if provided.
    if let Some(JsValue::Array(arr)) = args.first() {
        let elements = arr.borrow().to_dense_vec();
        for v in &elements {
            if let JsValue::Object(obj_rc) = &vm.current_this {
                if let JsValue::Array(items) = obj_rc.borrow().get("__items") {
                    let mut items_mut = items.borrow_mut();
                    let has = items_mut.elements.values().any(|s| s.strict_eq(v));
                    if !has {
                        items_mut.push(v.clone());
                    }
                }
                let size = if let JsValue::Array(items) = obj_rc.borrow().get("__items") {
                    items.borrow().count() as f64
                } else { 0.0 };
                obj_rc.borrow_mut().set(String::from("size"), JsValue::Number(size));
            }
        }
    }
    JsValue::Undefined
}

fn set_find_index(obj: &JsObject, value: &JsValue) -> Option<usize> {
    if let JsValue::Array(items) = obj.get("__items") {
        let arr = items.borrow();
        for (&i, v) in arr.elements.iter() {
            if v.strict_eq(value) { return Some(i); }
        }
    }
    None
}

fn update_set_size(obj_rc: &Rc<RefCell<JsObject>>) {
    let size = {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            items.borrow().count() as f64
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
            let has = items.borrow().elements.values().any(|v| v.strict_eq(&value));
            if !has {
                items.borrow_mut().push(value);
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
                items.borrow_mut().remove_and_shift(idx);
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
        if let JsValue::Array(items) = o.get("__items") { items.borrow_mut().clear(); }
        drop(o);
        update_set_size(obj_rc);
    }
    JsValue::Undefined
}

pub fn set_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            return JsValue::new_array(items.borrow().values_vec());
        }
    }
    JsValue::new_array(Vec::new())
}

pub fn set_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            let entries: Vec<JsValue> = items.borrow().elements.values()
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
            if let JsValue::Array(arr) = o.get("__items") { arr.borrow().values_vec() } else { Vec::new() }
        };
        for v in &items {
            super::native_array::call_callback_pub(vm, &callback, &[v.clone(), v.clone()]);
        }
    }
    JsValue::Undefined
}
