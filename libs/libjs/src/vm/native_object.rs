//! Object.prototype methods and Object static methods.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::value::*;
use super::Vm;

// ═══════════════════════════════════════════════════════════
// Object.prototype methods
// ═══════════════════════════════════════════════════════════

pub fn object_has_own_property(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    match &vm.current_this {
        JsValue::Object(obj) => JsValue::Bool(obj.borrow().has_own(&key)),
        JsValue::Array(arr) => {
            let a = arr.borrow();
            if let Some(idx) = super::try_parse_index(&key) {
                JsValue::Bool(idx < a.elements.len())
            } else {
                JsValue::Bool(a.properties.contains_key(&key))
            }
        }
        _ => JsValue::Bool(false),
    }
}

pub fn object_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match &vm.current_this {
        JsValue::Array(_) => JsValue::String(String::from("[object Array]")),
        JsValue::Function(_) => JsValue::String(String::from("[object Function]")),
        JsValue::Null => JsValue::String(String::from("[object Null]")),
        JsValue::Undefined => JsValue::String(String::from("[object Undefined]")),
        _ => JsValue::String(String::from("[object Object]")),
    }
}

pub fn object_value_of(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

pub fn object_keys_method(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match &vm.current_this {
        JsValue::Object(obj) => {
            let keys: Vec<JsValue> = obj.borrow().keys().into_iter()
                .map(JsValue::String).collect();
            JsValue::new_array(keys)
        }
        _ => JsValue::new_array(Vec::new()),
    }
}

// ═══════════════════════════════════════════════════════════
// Object static methods (Object.keys, Object.values, etc.)
// ═══════════════════════════════════════════════════════════

pub fn object_keys(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let keys: Vec<JsValue> = obj.borrow().keys().into_iter()
                .map(JsValue::String).collect();
            JsValue::new_array(keys)
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            let keys: Vec<JsValue> = (0..a.elements.len())
                .map(|i| JsValue::String(format_usize(i)))
                .collect();
            JsValue::new_array(keys)
        }
        _ => JsValue::new_array(Vec::new()),
    }
}

pub fn object_values(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            let vals: Vec<JsValue> = o.keys().into_iter()
                .map(|k| o.properties.get(&k).map(|p| p.value.clone()).unwrap_or(JsValue::Undefined))
                .collect();
            JsValue::new_array(vals)
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            JsValue::new_array(a.elements.clone())
        }
        _ => JsValue::new_array(Vec::new()),
    }
}

pub fn object_entries(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            let entries: Vec<JsValue> = o.keys().into_iter()
                .map(|k| {
                    let v = o.properties.get(&k).map(|p| p.value.clone()).unwrap_or(JsValue::Undefined);
                    JsValue::new_array(alloc::vec![JsValue::String(k), v])
                })
                .collect();
            JsValue::new_array(entries)
        }
        _ => JsValue::new_array(Vec::new()),
    }
}

pub fn object_assign(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(target_obj) = &target {
        for source in args.iter().skip(1) {
            if let JsValue::Object(src) = source {
                let s = src.borrow();
                for key in s.keys() {
                    if let Some(prop) = s.properties.get(&key) {
                        target_obj.borrow_mut().set(key, prop.value.clone());
                    }
                }
            }
        }
    }
    target
}

pub fn object_freeze(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(JsValue::Object(obj)) = args.first() {
        let mut o = obj.borrow_mut();
        let keys: Vec<String> = o.properties.keys().cloned().collect();
        for key in keys {
            if let Some(prop) = o.properties.get_mut(&key) {
                prop.writable = false;
                prop.configurable = false;
            }
        }
    }
    args.first().cloned().unwrap_or(JsValue::Undefined)
}

pub fn object_create(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let proto = match args.first() {
        Some(JsValue::Object(obj)) => Some(obj.clone()),
        Some(JsValue::Null) => None,
        _ => None,
    };
    let obj = JsObject {
        properties: alloc::collections::BTreeMap::new(),
        prototype: proto,
        internal_tag: None,
    };
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

pub fn object_define_property(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let key = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    let descriptor = args.get(2).cloned().unwrap_or(JsValue::Undefined);

    if let JsValue::Object(target_obj) = &target {
        if let JsValue::Object(desc_obj) = &descriptor {
            let desc = desc_obj.borrow();
            let value = desc.get("value");
            let writable = desc.get("writable").to_boolean();
            let enumerable = desc.get("enumerable").to_boolean();
            let configurable = desc.get("configurable").to_boolean();
            let prop = Property {
                value,
                writable,
                enumerable,
                configurable,
            };
            target_obj.borrow_mut().properties.insert(key, prop);
        }
    }
    target
}

pub fn object_get_prototype_of(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            match &o.prototype {
                Some(proto) => JsValue::Object(proto.clone()),
                None => JsValue::Null,
            }
        }
        _ => JsValue::Null,
    }
}

// ── Helpers ──

fn format_usize(n: usize) -> String {
    use alloc::format;
    format!("{}", n)
}
