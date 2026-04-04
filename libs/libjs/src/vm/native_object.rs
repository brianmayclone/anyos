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
                JsValue::Bool(a.has(idx))
            } else {
                JsValue::Bool(key == "length" || a.properties.contains_key(&key))
            }
        }
        // Function values: check own_props AND built-in properties (name, length, prototype).
        JsValue::Function(f) => {
            let func = f.borrow();
            JsValue::Bool(func.own_props.contains_key(&key) || key == "name" || key == "length" || (key == "prototype" && !func.kind.is_arrow()))
        }
        _ => JsValue::Bool(false),
    }
}

/// `Object.prototype.isPrototypeOf(V)` — returns true if `this` appears anywhere
/// in the prototype chain of `V`.
pub fn object_is_prototype_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // `this` is the candidate prototype; arg[0] is the object whose chain we walk.
    let self_rc = match &vm.current_this {
        JsValue::Object(rc) => rc.clone(),
        _ => return JsValue::Bool(false),
    };

    // Walk the [[Prototype]] chain of `args[0]`.
    // Functions inherit from function_proto, so treat them specially.
    let mut maybe_proto: Option<Rc<RefCell<JsObject>>> = match args.first() {
        Some(JsValue::Object(obj)) => obj.borrow().prototype.clone(),
        // Functions' implicit [[Prototype]] is Function.prototype.
        Some(JsValue::Function(_)) => Some(vm.function_proto.clone()),
        _ => return JsValue::Bool(false),
    };

    while let Some(proto_rc) = maybe_proto {
        if Rc::ptr_eq(&proto_rc, &self_rc) {
            return JsValue::Bool(true);
        }
        maybe_proto = proto_rc.borrow().prototype.clone();
    }
    JsValue::Bool(false)
}

pub fn object_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match &vm.current_this {
        JsValue::Array(_) => JsValue::String(String::from("[object Array]")),
        JsValue::Function(_) => JsValue::String(String::from("[object Function]")),
        JsValue::Null => JsValue::String(String::from("[object Null]")),
        JsValue::Undefined => JsValue::String(String::from("[object Undefined]")),
        JsValue::Object(obj) => {
            // Return the appropriate [object X] tag based on the object's internal tag.
            let tag = obj.borrow().internal_tag.clone();
            let kind = match tag.as_deref() {
                Some("__boolean__") => "Boolean",
                Some("__number__")  => "Number",
                Some("__string__")  => "String",
                Some("__regexp__")  => "RegExp",
                Some("__date__")    => "Date",
                _                   => "Object",
            };
            JsValue::String(alloc::format!("[object {}]", kind))
        }
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
            let keys: Vec<JsValue> = a.elements.keys()
                .map(|&i| JsValue::String(format_usize(i)))
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
            JsValue::new_array(a.values_vec())
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
        primitive_value: None,
        set_hook: None,
        set_hook_data: core::ptr::null_mut(),
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
            // Check if it's an accessor descriptor (has get or set)
            let has_get = desc.has_own("get");
            let has_set = desc.has_own("set");
            if has_get || has_set {
                let getter = {
                    let v = desc.get("get");
                    if v.is_function() { Some(v) } else { None }
                };
                let setter = {
                    let v = desc.get("set");
                    if v.is_function() { Some(v) } else { None }
                };
                let enumerable = if desc.has_own("enumerable") { desc.get("enumerable").to_boolean() } else { false };
                let configurable = if desc.has_own("configurable") { desc.get("configurable").to_boolean() } else { false };
                let mut prop = Property::accessor(getter, setter);
                prop.enumerable = enumerable;
                prop.configurable = configurable;
                target_obj.borrow_mut().properties.insert(key, prop);
            } else {
                let value = desc.get("value");
                let writable = if desc.has_own("writable") { desc.get("writable").to_boolean() } else { false };
                let enumerable = if desc.has_own("enumerable") { desc.get("enumerable").to_boolean() } else { false };
                let configurable = if desc.has_own("configurable") { desc.get("configurable").to_boolean() } else { false };
                let prop = Property {
                    value,
                    writable,
                    enumerable,
                    configurable,
                    getter: None,
                    setter: None,
                };
                target_obj.borrow_mut().properties.insert(key, prop);
            }
        }
    }
    target
}

pub fn object_get_prototype_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            match &o.prototype {
                Some(proto) => JsValue::Object(proto.clone()),
                None => JsValue::Null,
            }
        }
        Some(JsValue::Function(_)) => {
            // Functions inherit from Function.prototype
            JsValue::Object(vm.function_proto.clone())
        }
        _ => JsValue::Null,
    }
}

// ═══════════════════════════════════════════════════════════
// Additional Object static methods
// ═══════════════════════════════════════════════════════════

/// `Object.fromEntries(iterable)` — create object from [key, value] pairs.
pub fn object_from_entries(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let obj = JsValue::new_object();
    if let Some(JsValue::Array(arr)) = args.first() {
        let entries = arr.borrow();
        for (_, entry) in entries.elements.iter() {
            if let JsValue::Array(pair) = entry {
                let p = pair.borrow();
                let key = p.get(0).to_js_string();
                let val = p.get(1);
                obj.set_property(key, val);
            }
        }
    }
    obj
}

/// `Object.is(value1, value2)` — SameValue comparison.
pub fn object_is(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let a = args.first().cloned().unwrap_or(JsValue::Undefined);
    let b = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    JsValue::Bool(same_value(&a, &b))
}

fn same_value(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Undefined, JsValue::Undefined) => true,
        (JsValue::Null, JsValue::Null) => true,
        (JsValue::Bool(x), JsValue::Bool(y)) => x == y,
        (JsValue::Number(x), JsValue::Number(y)) => {
            if x.is_nan() && y.is_nan() { return true; }
            if *x == 0.0 && *y == 0.0 { return x.is_sign_positive() == y.is_sign_positive(); }
            x.to_bits() == y.to_bits()
        }
        (JsValue::String(x), JsValue::String(y)) => x == y,
        (JsValue::Object(x), JsValue::Object(y)) => Rc::ptr_eq(x, y),
        (JsValue::Array(x), JsValue::Array(y)) => Rc::ptr_eq(x, y),
        (JsValue::Function(x), JsValue::Function(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// `Object.setPrototypeOf(obj, proto)` — set __proto__.
pub fn object_set_prototype_of(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let obj = args.first().cloned().unwrap_or(JsValue::Undefined);
    let proto = args.get(1).cloned().unwrap_or(JsValue::Null);
    if let JsValue::Object(o) = &obj {
        let new_proto = match &proto {
            JsValue::Object(p) => Some(p.clone()),
            JsValue::Null => None,
            _ => return obj,
        };
        o.borrow_mut().prototype = new_proto;
    }
    obj
}

/// `Object.getOwnPropertyNames(obj)` — all own property names (including non-enumerable).
pub fn object_get_own_property_names(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            let keys: Vec<JsValue> = o.own_property_names()
                .into_iter()
                .map(JsValue::String)
                .collect();
            JsValue::new_array(keys)
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            let mut keys: Vec<JsValue> = a.elements.keys()
                .map(|&i| JsValue::String(format_usize(i)))
                .collect();
            keys.push(JsValue::String(String::from("length")));
            JsValue::new_array(keys)
        }
        _ => JsValue::new_array(Vec::new()),
    }
}

/// `Object.getOwnPropertyDescriptor(obj, key)`.
pub fn object_get_own_property_descriptor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            if let Some(prop) = o.properties.get(&key) {
                prop_to_descriptor(prop)
            } else {
                JsValue::Undefined
            }
        }
        Some(JsValue::Function(fn_rc)) => {
            fn_get_own_property_descriptor(fn_rc, &key)
        }
        _ => JsValue::Undefined,
    }
}

/// Get own property descriptor for Function objects.
/// Handles built-in properties (`name`, `length`, `prototype`) and own_props
/// including static accessor workaround keys (`__get_`/`__set_`).
fn fn_get_own_property_descriptor(fn_rc: &Rc<RefCell<JsFunction>>, key: &str) -> JsValue {
    let func = fn_rc.borrow();
    // Check for static accessor (stored as __get_<name> / __set_<name>)
    let get_key = alloc::format!("__get_{}", key);
    let set_key = alloc::format!("__set_{}", key);
    let has_getter = func.own_props.contains_key(&get_key);
    let has_setter = func.own_props.contains_key(&set_key);
    if has_getter || has_setter {
        let getter = func.own_props.get(&get_key).cloned();
        let setter = func.own_props.get(&set_key).cloned();
        return prop_to_descriptor(&Property::accessor(getter, setter));
    }
    // Check regular own_props first
    if let Some(val) = func.own_props.get(key) {
        return prop_to_descriptor(&Property::data(val.clone()));
    }
    // Built-in function properties (ES2023 §10.2.4, §20.2.4)
    match key {
        "name" => {
            let name_val = func.name.as_ref()
                .map(|n| JsValue::String(n.clone()))
                .unwrap_or(JsValue::String(String::new()));
            prop_to_descriptor(&Property {
                value: name_val,
                writable: false,
                enumerable: false,
                configurable: true,
                getter: None,
                setter: None,
            })
        }
        "length" => {
            let len = func.arity.unwrap_or(func.params.len());
            prop_to_descriptor(&Property {
                value: JsValue::Number(len as f64),
                writable: false,
                enumerable: false,
                configurable: true,
                getter: None,
                setter: None,
            })
        }
        "prototype" => {
            if func.kind.is_arrow() {
                return JsValue::Undefined; // Arrow functions have no .prototype
            }
            let proto_val = if let Some(ref proto) = func.prototype {
                JsValue::Object(proto.clone())
            } else {
                // Don't auto-create prototype here; just report what exists.
                return JsValue::Undefined;
            };
            prop_to_descriptor(&Property {
                value: proto_val,
                writable: true,
                enumerable: false,
                configurable: false,
                getter: None,
                setter: None,
            })
        }
        _ => JsValue::Undefined,
    }
}

/// `Object.getOwnPropertyDescriptors(obj)`.
pub fn object_get_own_property_descriptors(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let result = JsValue::new_object();
    if let Some(JsValue::Object(obj)) = args.first() {
        let o = obj.borrow();
        for (key, prop) in &o.properties {
            let desc = prop_to_descriptor(prop);
            result.set_property(key.clone(), desc);
        }
    }
    result
}

fn prop_to_descriptor(prop: &Property) -> JsValue {
    let desc = JsValue::new_object();
    if prop.is_accessor() {
        if let Some(ref g) = prop.getter { desc.set_property(String::from("get"), g.clone()); }
        if let Some(ref s) = prop.setter { desc.set_property(String::from("set"), s.clone()); }
    } else {
        desc.set_property(String::from("value"), prop.value.clone());
        desc.set_property(String::from("writable"), JsValue::Bool(prop.writable));
    }
    desc.set_property(String::from("enumerable"), JsValue::Bool(prop.enumerable));
    desc.set_property(String::from("configurable"), JsValue::Bool(prop.configurable));
    desc
}

/// `Object.preventExtensions(obj)`.
pub fn object_prevent_extensions(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Mark object as non-extensible (simplified: we set an internal tag)
    if let Some(JsValue::Object(obj)) = args.first() {
        let mut o = obj.borrow_mut();
        if o.internal_tag.is_none() {
            o.internal_tag = Some(String::from("__sealed__"));
        }
    }
    args.first().cloned().unwrap_or(JsValue::Undefined)
}

/// `Object.isExtensible(obj)`.
pub fn object_is_extensible(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            let sealed = o.internal_tag.as_deref() == Some("__sealed__") ||
                         o.internal_tag.as_deref() == Some("__frozen__");
            JsValue::Bool(!sealed)
        }
        Some(JsValue::Function(_)) => {
            // Functions are extensible by default (ES2023 §10.2.4)
            JsValue::Bool(true)
        }
        _ => JsValue::Bool(false),
    }
}

/// `Object.seal(obj)` — make all properties non-configurable.
pub fn object_seal(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(JsValue::Object(obj)) = args.first() {
        let mut o = obj.borrow_mut();
        let keys: Vec<String> = o.properties.keys().cloned().collect();
        for key in keys {
            if let Some(prop) = o.properties.get_mut(&key) {
                prop.configurable = false;
            }
        }
        o.internal_tag = Some(String::from("__sealed__"));
    }
    args.first().cloned().unwrap_or(JsValue::Undefined)
}

/// `Object.isSealed(obj)`.
pub fn object_is_sealed(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            let all_non_configurable = o.properties.values().all(|p| !p.configurable);
            JsValue::Bool(all_non_configurable)
        }
        _ => JsValue::Bool(true),
    }
}

/// `Object.isFrozen(obj)`.
pub fn object_is_frozen(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            let all_frozen = o.properties.values().all(|p| !p.writable && !p.configurable);
            JsValue::Bool(all_frozen)
        }
        _ => JsValue::Bool(true),
    }
}

/// `Object.hasOwn(obj, key)` — static version of hasOwnProperty.
pub fn object_has_own(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    match args.first() {
        Some(JsValue::Object(obj)) => JsValue::Bool(obj.borrow().has_own(&key)),
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            if let Some(idx) = super::try_parse_index(&key) {
                JsValue::Bool(a.has(idx))
            } else {
                JsValue::Bool(key == "length" || a.properties.contains_key(&key))
            }
        }
        Some(JsValue::Function(f)) => {
            let func = f.borrow();
            JsValue::Bool(func.own_props.contains_key(&key) || key == "name" || key == "length" || (key == "prototype" && !func.kind.is_arrow()))
        }
        _ => JsValue::Bool(false),
    }
}

/// `Object.defineProperties(obj, descriptors)`.
pub fn object_define_properties(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(JsValue::Object(descs)) = args.get(1) {
        let d = descs.borrow();
        let keys: Vec<String> = d.keys();
        drop(d);
        for key in keys {
            let desc = descs.borrow().get(&key);
            object_define_property(vm, &[target.clone(), JsValue::String(key), desc]);
        }
    }
    target
}

/// `Object.getOwnPropertySymbols(obj)` — returns symbol-keyed property names.
pub fn object_get_own_property_symbols(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            let syms: Vec<JsValue> = o.own_symbol_keys()
                .into_iter()
                .map(JsValue::String)
                .collect();
            JsValue::new_array(syms)
        }
        _ => JsValue::new_array(Vec::new()),
    }
}

// ── Helpers ──

fn format_usize(n: usize) -> String {
    use alloc::format;
    format!("{}", n)
}
