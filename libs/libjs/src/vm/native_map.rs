//! Map and Set built-in objects.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::{native_fn, Vm};
use crate::value::*;

const MAP_TAG: &str = "__map__";
const SET_TAG: &str = "__set__";

fn require_receiver(
    vm: &mut Vm,
    expected_tag: &str,
    expected_name: &str,
    invocation_msg: &str,
    slot_keys: &[&str],
) -> Option<Rc<RefCell<JsObject>>> {
    match &vm.current_this {
        JsValue::Object(obj_rc) => {
            let (tagged, slot_backed) = {
                let obj = obj_rc.borrow();
                (
                    obj.internal_tag.as_deref() == Some(expected_tag),
                    slot_keys.iter().all(|key| obj.has(key)),
                )
            };
            if tagged || slot_backed {
                Some(obj_rc.clone())
            } else {
                let err = vm.make_type_error(invocation_msg);
                vm.throw_native(err);
                vm.log_engine(&alloc::format!(
                    "[libjs] invalid {} receiver for {}",
                    expected_name,
                    invocation_msg
                ));
                None
            }
        }
        _ => {
            let err = vm.make_type_error(invocation_msg);
            vm.throw_native(err);
            vm.log_engine(&alloc::format!(
                "[libjs] invalid {} receiver for {}",
                expected_name,
                invocation_msg
            ));
            None
        }
    }
}

fn require_construct_receiver(vm: &mut Vm, expected_name: &str) -> Option<Rc<RefCell<JsObject>>> {
    match &vm.current_this {
        JsValue::Object(obj_rc) => Some(obj_rc.clone()),
        _ => {
            let err = vm.make_type_error(&alloc::format!(
                "Constructor {} requires 'new'",
                expected_name
            ));
            vm.throw_native(err);
            None
        }
    }
}

fn require_map_receiver(vm: &mut Vm) -> Option<Rc<RefCell<JsObject>>> {
    require_receiver(
        vm,
        MAP_TAG,
        "Map",
        "Incorrect Map invocation",
        &["__keys", "__values"],
    )
}

fn require_set_receiver(vm: &mut Vm) -> Option<Rc<RefCell<JsObject>>> {
    require_receiver(vm, SET_TAG, "Set", "Incorrect Set invocation", &["__items"])
}

fn make_iterator(items: Vec<JsValue>) -> JsValue {
    let items_arr = JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(items))));
    let iter_obj = JsValue::new_object();
    iter_obj.set_property(String::from("__items__"), items_arr);
    iter_obj.set_property(String::from("__index__"), JsValue::Number(0.0));
    iter_obj.set_property(String::from("next"), native_fn("next", iterator_next));
    iter_obj.set_property(
        String::from(super::native_symbol::WELL_KNOWN_ITERATOR),
        native_fn("[Symbol.iterator]", iterator_self),
    );
    iter_obj
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

// ═══════════════════════════════════════════════════════════
// Map constructor and prototype
// ═══════════════════════════════════════════════════════════

pub fn ctor_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Tag `this` (created by `new_object` with the correct prototype chain).
    let Some(obj_rc) = require_construct_receiver(vm, "Map") else {
        return JsValue::Undefined;
    };
    {
        let mut o = obj_rc.borrow_mut();
        o.internal_tag = Some(String::from(MAP_TAG));
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
                    let (keys_arr, vals_arr) = {
                        let ob = obj_rc.borrow();
                        (ob.get("__keys"), ob.get("__values"))
                    };
                    if let (JsValue::Array(keys), JsValue::Array(vals)) = (keys_arr, vals_arr) {
                        keys.borrow_mut().push(key);
                        vals.borrow_mut().push(val);
                    }
                    update_size(&obj_rc);
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
    obj_rc
        .borrow_mut()
        .set(String::from("size"), JsValue::Number(size));
}

pub fn map_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let this = vm.current_this.clone();

    let Some(obj_rc) = require_map_receiver(vm) else {
        return JsValue::Undefined;
    };
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
    update_size(&obj_rc);
    this
}

pub fn map_get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(obj_rc) = require_map_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    if let Some(idx) = map_find_index(&o, &key) {
        if let JsValue::Array(vals) = o.get("__values") {
            return vals.borrow().get(idx);
        }
    }
    JsValue::Undefined
}

pub fn map_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(obj_rc) = require_map_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    JsValue::Bool(map_find_index(&o, &key).is_some())
}

pub fn map_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(obj_rc) = require_map_receiver(vm) else {
        return JsValue::Undefined;
    };
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
        update_size(&obj_rc);
        return JsValue::Bool(true);
    }
    JsValue::Bool(false)
}

pub fn map_clear(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let Some(obj_rc) = require_map_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    if let JsValue::Array(keys) = o.get("__keys") {
        keys.borrow_mut().clear();
    }
    if let JsValue::Array(vals) = o.get("__values") {
        vals.borrow_mut().clear();
    }
    drop(o);
    update_size(&obj_rc);
    JsValue::Undefined
}

pub fn map_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let Some(obj_rc) = require_map_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    if let JsValue::Array(keys) = o.get("__keys") {
        return make_iterator(keys.borrow().values_vec());
    }
    make_iterator(Vec::new())
}

pub fn map_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let Some(obj_rc) = require_map_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    if let JsValue::Array(vals) = o.get("__values") {
        return make_iterator(vals.borrow().values_vec());
    }
    make_iterator(Vec::new())
}

pub fn map_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let Some(obj_rc) = require_map_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    if let (JsValue::Array(keys), JsValue::Array(vals)) = (o.get("__keys"), o.get("__values")) {
        let k = keys.borrow();
        let v = vals.borrow();
        let kv: Vec<_> = k.elements.values().cloned().collect();
        let vv: Vec<_> = v.elements.values().cloned().collect();
        let entries: Vec<JsValue> = kv
            .into_iter()
            .zip(vv.into_iter())
            .map(|(key, val)| JsValue::new_array(alloc::vec![key, val]))
            .collect();
        return make_iterator(entries);
    }
    make_iterator(Vec::new())
}

pub fn map_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(obj_rc) = require_map_receiver(vm) else {
        return JsValue::Undefined;
    };
    let (keys, vals) = {
        let o = obj_rc.borrow();
        let k = if let JsValue::Array(arr) = o.get("__keys") {
            arr.borrow().values_vec()
        } else {
            Vec::new()
        };
        let v = if let JsValue::Array(arr) = o.get("__values") {
            arr.borrow().values_vec()
        } else {
            Vec::new()
        };
        (k, v)
    };
    for (i, (k, v)) in keys.iter().zip(vals.iter()).enumerate() {
        let _ = i;
        super::native_array::call_callback_pub(vm, &callback, &[v.clone(), k.clone()]);
    }
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════════
// Set constructor and prototype
// ═══════════════════════════════════════════════════════════

pub fn ctor_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(obj_rc) = require_construct_receiver(vm, "Set") else {
        return JsValue::Undefined;
    };
    {
        let mut o = obj_rc.borrow_mut();
        o.internal_tag = Some(String::from(SET_TAG));
        o.set(String::from("__items"), JsValue::new_array(Vec::new()));
        o.set(String::from("size"), JsValue::Number(0.0));
    }
    // Pre-populate from iterable argument (array) if provided.
    if let Some(JsValue::Array(arr)) = args.first() {
        let elements = arr.borrow().to_dense_vec();
        for v in &elements {
            if let JsValue::Array(items) = obj_rc.borrow().get("__items") {
                let mut items_mut = items.borrow_mut();
                let has = items_mut.elements.values().any(|s| s.strict_eq(v));
                if !has {
                    items_mut.push(v.clone());
                }
            }
            let size = if let JsValue::Array(items) = obj_rc.borrow().get("__items") {
                items.borrow().count() as f64
            } else {
                0.0
            };
            obj_rc
                .borrow_mut()
                .set(String::from("size"), JsValue::Number(size));
        }
    }
    JsValue::Undefined
}

fn set_find_index(obj: &JsObject, value: &JsValue) -> Option<usize> {
    if let JsValue::Array(items) = obj.get("__items") {
        let arr = items.borrow();
        for (&i, v) in arr.elements.iter() {
            if v.strict_eq(value) {
                return Some(i);
            }
        }
    }
    None
}

fn update_set_size(obj_rc: &Rc<RefCell<JsObject>>) {
    let size = {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            items.borrow().count() as f64
        } else {
            0.0
        }
    };
    obj_rc
        .borrow_mut()
        .set(String::from("size"), JsValue::Number(size));
}

pub fn set_add(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this = vm.current_this.clone();
    let Some(obj_rc) = require_set_receiver(vm) else {
        return JsValue::Undefined;
    };
    let already = { obj_rc.borrow() }.get("__items");
    if let JsValue::Array(items) = already {
        let has = items
            .borrow()
            .elements
            .values()
            .any(|v| v.strict_eq(&value));
        if !has {
            items.borrow_mut().push(value);
        }
    }
    update_set_size(&obj_rc);
    this
}

pub fn set_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(obj_rc) = require_set_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    JsValue::Bool(set_find_index(&o, &value).is_some())
}

pub fn set_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(obj_rc) = require_set_receiver(vm) else {
        return JsValue::Undefined;
    };
    let idx = { set_find_index(&obj_rc.borrow(), &value) };
    if let Some(idx) = idx {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            items.borrow_mut().remove_and_shift(idx);
        }
        drop(o);
        update_set_size(&obj_rc);
        return JsValue::Bool(true);
    }
    JsValue::Bool(false)
}

pub fn set_clear(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let Some(obj_rc) = require_set_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    if let JsValue::Array(items) = o.get("__items") {
        items.borrow_mut().clear();
    }
    drop(o);
    update_set_size(&obj_rc);
    JsValue::Undefined
}

pub fn set_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let Some(obj_rc) = require_set_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    if let JsValue::Array(items) = o.get("__items") {
        return make_iterator(items.borrow().values_vec());
    }
    make_iterator(Vec::new())
}

pub fn set_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let Some(obj_rc) = require_set_receiver(vm) else {
        return JsValue::Undefined;
    };
    let o = obj_rc.borrow();
    if let JsValue::Array(items) = o.get("__items") {
        let entries: Vec<JsValue> = items
            .borrow()
            .elements
            .values()
            .map(|v| JsValue::new_array(alloc::vec![v.clone(), v.clone()]))
            .collect();
        return make_iterator(entries);
    }
    make_iterator(Vec::new())
}

pub fn set_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let Some(obj_rc) = require_set_receiver(vm) else {
        return JsValue::Undefined;
    };
    let items = {
        let o = obj_rc.borrow();
        if let JsValue::Array(arr) = o.get("__items") {
            arr.borrow().values_vec()
        } else {
            Vec::new()
        }
    };
    for v in &items {
        super::native_array::call_callback_pub(vm, &callback, &[v.clone(), v.clone()]);
    }
    JsValue::Undefined
}
