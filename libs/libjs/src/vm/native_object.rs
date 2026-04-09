//! Object.prototype methods and Object static methods.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::Vm;
use crate::value::*;

fn string_exotic_own_property_descriptor(s: &str, key: &str) -> Option<Property> {
    if key == "length" {
        return Some(Property {
            value: JsValue::Number(s.chars().count() as f64),
            writable: false,
            enumerable: false,
            configurable: false,
            getter: None,
            setter: None,
        });
    }
    if let Some(idx) = super::try_parse_index(key) {
        if let Some(ch) = s.chars().nth(idx) {
            let mut buf = String::new();
            buf.push(ch);
            return Some(Property {
                value: JsValue::String(buf),
                writable: false,
                enumerable: true,
                configurable: false,
                getter: None,
                setter: None,
            });
        }
    }
    None
}

fn boxed_string_value(obj: &JsObject) -> Option<String> {
    match obj.primitive_value.as_deref() {
        Some(JsValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn function_desc_flag_key(key: &str, flag: &str) -> String {
    alloc::format!("__desc_{}_{}", flag, key)
}

fn function_deleted_builtin_key(key: &str) -> String {
    alloc::format!("__deleted_builtin_{}", key)
}

fn function_builtin_deleted(func: &JsFunction, key: &str) -> bool {
    matches!(
        func.own_props.get(&function_deleted_builtin_key(key)),
        Some(JsValue::Bool(true))
    )
}

fn is_function_hidden_prop_key(key: &str) -> bool {
    key.starts_with("__get_")
        || key.starts_with("__set_")
        || key.starts_with("__desc_")
        || key.starts_with("__deleted_builtin_")
        || key == "__constructable__"
}

fn function_public_own_prop_names(func: &JsFunction) -> Vec<String> {
    let mut keys: Vec<String> = func
        .own_props
        .keys()
        .filter(|k| !is_function_hidden_prop_key(k))
        .cloned()
        .collect();
    for raw in func.own_props.keys() {
        if let Some(name) = raw.strip_prefix("__get_").or_else(|| raw.strip_prefix("__set_")) {
            if !keys.iter().any(|k| k == name) {
                keys.push(String::from(name));
            }
        }
    }
    keys
}

fn function_descriptor_flag(func: &JsFunction, key: &str, flag: &str, default: bool) -> bool {
    match func.own_props.get(&function_desc_flag_key(key, flag)) {
        Some(JsValue::Bool(v)) => *v,
        _ => default,
    }
}

fn to_property_key(vm: &mut Vm, val: &JsValue) -> Option<String> {
    let key_val = match val {
        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => {
            vm.to_primitive_for_op(val.clone(), "string")
        }
        _ => val.clone(),
    };
    if vm.pending_exception.is_some() {
        return None;
    }
    Some(match key_val {
        JsValue::String(s) => s,
        other => other.to_js_string(),
    })
}

// ═══════════════════════════════════════════════════════════
// Object.prototype methods
// ═══════════════════════════════════════════════════════════

pub fn object_has_own_property(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = match args.first() {
        Some(v) => match to_property_key(vm, v) {
            Some(k) => k,
            None => return JsValue::Undefined,
        },
        None => String::new(),
    };
    if matches!(vm.current_this, JsValue::Null | JsValue::Undefined) {
        let err = vm.make_type_error("Cannot convert undefined or null to object");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    match &vm.current_this {
        JsValue::Object(obj) => {
            let o = obj.borrow();
            if o.has_own(&key) {
                JsValue::Bool(true)
            } else if let Some(s) = boxed_string_value(&o) {
                JsValue::Bool(string_exotic_own_property_descriptor(&s, &key).is_some())
            } else {
                JsValue::Bool(false)
            }
        }
        JsValue::Array(arr) => {
            let a = arr.borrow();
            if let Some(idx) = super::try_parse_index(&key) {
                JsValue::Bool(a.has(idx))
            } else {
                JsValue::Bool(key == "length" || a.properties.contains_key(&key))
            }
        }
        JsValue::String(s) => JsValue::Bool(string_exotic_own_property_descriptor(s, &key).is_some()),
        // Function values: check own_props AND built-in properties (name, length, prototype).
        JsValue::Function(f) => {
            let func = f.borrow();
            let constructable = match &func.kind {
                FnKind::Bytecode(chunk) => !chunk.is_arrow && !chunk.is_generator,
                FnKind::Native(_) => matches!(
                    func.own_props.get("__constructable__"),
                    Some(JsValue::Bool(true))
                ),
            };
            JsValue::Bool(
                function_public_own_prop_names(&func).iter().any(|k| k == &key)
                    || (key == "name" && !function_builtin_deleted(&func, "name"))
                    || (key == "length" && !function_builtin_deleted(&func, "length"))
                    || (key == "prototype"
                        && constructable
                        && !function_builtin_deleted(&func, "prototype")),
            )
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
        JsValue::Null | JsValue::Undefined => {
            let err = vm.make_type_error("Cannot convert undefined or null to object");
            vm.throw_native(err);
            return JsValue::Undefined;
        }
        _ => return JsValue::Bool(false),
    };

    // Walk the [[Prototype]] chain of `args[0]`.
    // Functions inherit from function_proto, so treat them specially.
    let mut maybe_proto: Option<Rc<RefCell<JsObject>>> = match args.first() {
        Some(JsValue::Object(obj)) => obj.borrow().prototype.clone(),
        Some(JsValue::Array(_)) => Some(vm.array_proto.clone()),
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
            let o = obj.borrow();
            let tag_value = o.get(super::native_symbol::WELL_KNOWN_TO_STRING_TAG);
            let kind = if let JsValue::String(tag) = tag_value {
                tag
            } else {
                match o.internal_tag.as_deref() {
                    Some("__boolean__") => String::from("Boolean"),
                    Some("__number__") => String::from("Number"),
                    Some("__string__") => String::from("String"),
                    Some("__regexp__") => String::from("RegExp"),
                    Some("__date__") => String::from("Date"),
                    Some("__math__") => String::from("Math"),
                    Some("__json__") => String::from("JSON"),
                    Some("__error__") => String::from("Error"),
                    _ => String::from("Object"),
                }
            };
            JsValue::String(alloc::format!("[object {}]", kind))
        }
        _ => JsValue::String(String::from("[object Object]")),
    }
}

pub fn object_value_of(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

/// `Object.prototype.propertyIsEnumerable(key)`
pub fn object_property_is_enumerable(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = match args.first() {
        Some(v) => match to_property_key(vm, v) {
            Some(k) => k,
            None => return JsValue::Undefined,
        },
        None => String::new(),
    };
    if matches!(vm.current_this, JsValue::Null | JsValue::Undefined) {
        let err = vm.make_type_error("Cannot convert undefined or null to object");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    match &vm.current_this {
        JsValue::Object(obj) => {
            let o = obj.borrow();
            if let Some(prop) = o.properties.get(&key) {
                JsValue::Bool(prop.enumerable)
            } else if let Some(s) = boxed_string_value(&o) {
                if let Some(prop) = string_exotic_own_property_descriptor(&s, &key) {
                    JsValue::Bool(prop.enumerable)
                } else {
                    JsValue::Bool(false)
                }
            } else {
                JsValue::Bool(false)
            }
        }
        JsValue::Array(arr) => {
            let a = arr.borrow();
            if let Ok(idx) = key.parse::<usize>() {
                JsValue::Bool(a.elements.contains_key(&idx))
            } else if key == "length" {
                JsValue::Bool(false) // length is not enumerable
            } else if let Some(prop) = a.properties.get(&key) {
                JsValue::Bool(prop.enumerable)
            } else {
                JsValue::Bool(false)
            }
        }
        JsValue::String(s) => {
            if let Some(prop) = string_exotic_own_property_descriptor(s, &key) {
                JsValue::Bool(prop.enumerable)
            } else {
                JsValue::Bool(false)
            }
        }
        _ => JsValue::Bool(false),
    }
}

pub fn object_to_locale_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if matches!(vm.current_this, JsValue::Null | JsValue::Undefined) {
        let err = vm.make_type_error("Cannot convert undefined or null to object");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let to_string = vm.get_property_with_proto(&vm.current_this.clone(), "toString");
    if !matches!(to_string, JsValue::Function(_)) {
        let err = vm.make_type_error("toString is not callable");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let result = vm.call_value(&to_string, &[], vm.current_this.clone());
    if let Some(exc) = vm.last_exception.take() {
        vm.pending_exception = Some(exc);
        return JsValue::Undefined;
    }
    result
}

pub fn object_keys_method(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match &vm.current_this {
        JsValue::Object(obj) => {
            let keys: Vec<JsValue> = obj
                .borrow()
                .keys()
                .into_iter()
                .map(JsValue::String)
                .collect();
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
            let keys: Vec<JsValue> = obj
                .borrow()
                .keys()
                .into_iter()
                .map(JsValue::String)
                .collect();
            JsValue::new_array(keys)
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            let keys: Vec<JsValue> = a
                .elements
                .keys()
                .map(|&i| JsValue::String(format_usize(i)))
                .collect();
            JsValue::new_array(keys)
        }
        Some(JsValue::Function(func)) => {
            let keys: Vec<JsValue> = func
                .borrow()
                .own_props
                .keys()
                .filter(|k| !k.starts_with("__get_") && !k.starts_with("__set_"))
                .cloned()
                .map(JsValue::String)
                .collect();
            JsValue::new_array(keys)
        }
        _ => JsValue::new_array(Vec::new()),
    }
}

pub fn object_values(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let keys = obj.borrow().keys();
            let vals: Vec<JsValue> = keys
                .into_iter()
                .map(|k| vm.get_property_invoking_getter(args.first().unwrap(), &k))
                .collect();
            JsValue::new_array(vals)
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            let mut vals = a.values_vec();
            let prop_keys: Vec<String> = a
                .properties
                .keys()
                .filter(|k| k.parse::<usize>().is_err())
                .cloned()
                .collect();
            drop(a);
            for key in prop_keys {
                vals.push(vm.get_property_invoking_getter(args.first().unwrap(), &key));
            }
            JsValue::new_array(vals)
        }
        Some(JsValue::Function(func)) => {
            let keys: Vec<String> = func
                .borrow()
                .own_props
                .keys()
                .filter(|k| !k.starts_with("__get_") && !k.starts_with("__set_"))
                .cloned()
                .collect();
            let vals: Vec<JsValue> = keys
                .into_iter()
                .map(|k| vm.get_property_invoking_getter(args.first().unwrap(), &k))
                .collect();
            JsValue::new_array(vals)
        }
        _ => JsValue::new_array(Vec::new()),
    }
}

pub fn object_entries(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let keys = obj.borrow().keys();
            let entries: Vec<JsValue> = keys
                .into_iter()
                .map(|k| {
                    let v = vm.get_property_invoking_getter(args.first().unwrap(), &k);
                    JsValue::new_array(alloc::vec![JsValue::String(k), v])
                })
                .collect();
            JsValue::new_array(entries)
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            let mut entries: Vec<JsValue> = a
                .elements
                .iter()
                .map(|(i, v)| {
                    JsValue::new_array(alloc::vec![JsValue::String(format_usize(*i)), v.clone()])
                })
                .collect();
            let prop_keys: Vec<String> = a
                .properties
                .keys()
                .filter(|k| k.parse::<usize>().is_err())
                .cloned()
                .collect();
            drop(a);
            for key in prop_keys {
                let v = vm.get_property_invoking_getter(args.first().unwrap(), &key);
                entries.push(JsValue::new_array(alloc::vec![JsValue::String(key), v]));
            }
            JsValue::new_array(entries)
        }
        Some(JsValue::Function(func)) => {
            let keys: Vec<String> = func
                .borrow()
                .own_props
                .keys()
                .filter(|k| !k.starts_with("__get_") && !k.starts_with("__set_"))
                .cloned()
                .collect();
            let entries: Vec<JsValue> = keys
                .into_iter()
                .map(|k| {
                    let v = vm.get_property_invoking_getter(args.first().unwrap(), &k);
                    JsValue::new_array(alloc::vec![JsValue::String(k), v])
                })
                .collect();
            JsValue::new_array(entries)
        }
        _ => JsValue::new_array(Vec::new()),
    }
}

pub fn object_assign(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    for source in args.iter().skip(1) {
        let keys: Vec<String> = match source {
            JsValue::Object(src) => src.borrow().keys(),
            JsValue::Array(arr) => {
                let a = arr.borrow();
                let mut keys: Vec<String> = a.elements.keys().map(|&i| format_usize(i)).collect();
                keys.extend(
                    a.properties
                        .keys()
                        .filter(|k| k.parse::<usize>().is_err())
                        .cloned(),
                );
                keys
            }
            JsValue::Function(func) => func
                .borrow()
                .own_props
                .keys()
                .filter(|k| !k.starts_with("__get_") && !k.starts_with("__set_"))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        for key in keys {
            let value = vm.get_property_invoking_getter(source, &key);
            target.set_property(key, value);
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
                if !prop.is_accessor() {
                    prop.writable = false;
                }
                prop.configurable = false;
            }
        }
        // Mark as non-extensible (frozen implies sealed implies non-extensible).
        o.properties.insert(
            String::from("__non_extensible__"),
            crate::value::Property {
                value: JsValue::Bool(true),
                writable: false,
                enumerable: false,
                configurable: false,
                getter: None,
                setter: None,
            },
        );
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

    if let JsValue::Object(desc_obj) = &descriptor {
        let desc = desc_obj.borrow();
        let has_get = desc.has_own("get");
        let has_set = desc.has_own("set");
        let prop = if has_get || has_set {
            let getter = {
                let v = desc.get("get");
                if v.is_function() {
                    Some(v)
                } else {
                    None
                }
            };
            let setter = {
                let v = desc.get("set");
                if v.is_function() {
                    Some(v)
                } else {
                    None
                }
            };
            let enumerable = if desc.has_own("enumerable") {
                desc.get("enumerable").to_boolean()
            } else {
                false
            };
            let configurable = if desc.has_own("configurable") {
                desc.get("configurable").to_boolean()
            } else {
                false
            };
            let mut p = Property::accessor(getter, setter);
            p.enumerable = enumerable;
            p.configurable = configurable;
            p
        } else {
            let value = desc.get("value");
            let writable = if desc.has_own("writable") {
                desc.get("writable").to_boolean()
            } else {
                false
            };
            let enumerable = if desc.has_own("enumerable") {
                desc.get("enumerable").to_boolean()
            } else {
                false
            };
            let configurable = if desc.has_own("configurable") {
                desc.get("configurable").to_boolean()
            } else {
                false
            };
            Property {
                value,
                writable,
                enumerable,
                configurable,
                getter: None,
                setter: None,
            }
        };
        drop(desc);

        match &target {
            JsValue::Object(target_obj) => {
                target_obj.borrow_mut().properties.insert(key, prop);
            }
            JsValue::Array(arr) => {
                // For arrays: accessor properties go in .properties,
                // data properties can go in .elements if numeric
                if prop.is_accessor() {
                    arr.borrow_mut().properties.insert(key, prop);
                } else if let Ok(idx) = key.parse::<usize>() {
                    // Store as data in elements, but also keep descriptor in properties
                    // so that getOwnPropertyDescriptor works correctly
                    let mut a = arr.borrow_mut();
                    a.elements.insert(idx, prop.value.clone());
                    if idx >= a.length {
                        a.length = idx + 1;
                    }
                    a.properties.insert(key, prop);
                } else if key == "length" {
                    // length is handled specially
                    if let JsValue::Number(n) = &prop.value {
                        arr.borrow_mut().set_length(*n as usize);
                    }
                } else {
                    arr.borrow_mut().properties.insert(key, prop);
                }
            }
            JsValue::Function(f) => {
                let deleted_builtin_key = function_deleted_builtin_key(&key);
                if prop.is_accessor() {
                    // Store getter/setter as __get_key / __set_key pattern
                    let mut func = f.borrow_mut();
                    func.own_props.remove(&deleted_builtin_key);
                    if let Some(ref g) = prop.getter {
                        func.own_props
                            .insert(alloc::format!("__get_{}", key), g.clone());
                    }
                    if let Some(ref s) = prop.setter {
                        func.own_props
                            .insert(alloc::format!("__set_{}", key), s.clone());
                    }
                    func.own_props.insert(
                        function_desc_flag_key(&key, "enumerable"),
                        JsValue::Bool(prop.enumerable),
                    );
                    func.own_props.insert(
                        function_desc_flag_key(&key, "configurable"),
                        JsValue::Bool(prop.configurable),
                    );
                } else {
                    let mut func = f.borrow_mut();
                    func.own_props.remove(&deleted_builtin_key);
                    func.own_props.insert(key.clone(), prop.value);
                    func.own_props.insert(
                        function_desc_flag_key(&key, "writable"),
                        JsValue::Bool(prop.writable),
                    );
                    func.own_props.insert(
                        function_desc_flag_key(&key, "enumerable"),
                        JsValue::Bool(prop.enumerable),
                    );
                    func.own_props.insert(
                        function_desc_flag_key(&key, "configurable"),
                        JsValue::Bool(prop.configurable),
                    );
                }
            }
            _ => {}
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
        Some(JsValue::Array(_)) => JsValue::Object(vm.array_proto.clone()),
        Some(JsValue::Function(_)) => {
            // Functions inherit from Function.prototype
            JsValue::Object(vm.function_proto.clone())
        }
        Some(JsValue::String(_)) => JsValue::Object(vm.string_proto.clone()),
        Some(JsValue::Number(_)) => JsValue::Object(vm.number_proto.clone()),
        Some(JsValue::Bool(_)) => JsValue::Object(vm.boolean_proto.clone()),
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
            if x.is_nan() && y.is_nan() {
                return true;
            }
            if *x == 0.0 && *y == 0.0 {
                return x.is_sign_positive() == y.is_sign_positive();
            }
            x.to_bits() == y.to_bits()
        }
        (JsValue::String(x), JsValue::String(y)) => x == y,
        (JsValue::Object(x), JsValue::Object(y)) => Rc::ptr_eq(x, y),
        (JsValue::Array(x), JsValue::Array(y)) => Rc::ptr_eq(x, y),
        (JsValue::Function(x), JsValue::Function(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

fn prototype_slots_equal(
    current: &Option<Rc<RefCell<JsObject>>>,
    new_proto: &Option<Rc<RefCell<JsObject>>>,
) -> bool {
    match (current, new_proto) {
        (None, None) => true,
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}

fn would_create_prototype_cycle(
    target: &Rc<RefCell<JsObject>>,
    new_proto: &Option<Rc<RefCell<JsObject>>>,
) -> bool {
    let mut current = new_proto.clone();
    let mut seen = Vec::new();
    while let Some(obj) = current {
        if Rc::ptr_eq(&obj, target) {
            return true;
        }
        let ptr = Rc::as_ptr(&obj) as usize;
        if seen.contains(&ptr) {
            return false;
        }
        seen.push(ptr);
        current = obj.borrow().prototype.clone();
    }
    false
}

pub(crate) fn set_prototype_of_internal(
    vm: &mut Vm,
    obj: &Rc<RefCell<JsObject>>,
    new_proto: Option<Rc<RefCell<JsObject>>>,
) -> bool {
    let current = obj.borrow().prototype.clone();
    if prototype_slots_equal(&current, &new_proto) {
        return true;
    }
    if Rc::ptr_eq(obj, &vm.object_proto) {
        return false;
    }
    if would_create_prototype_cycle(obj, &new_proto) {
        return false;
    }
    obj.borrow_mut().prototype = new_proto;
    true
}

/// `Object.setPrototypeOf(obj, proto)` — set __proto__.
pub fn object_set_prototype_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let obj = args.first().cloned().unwrap_or(JsValue::Undefined);
    let proto = args.get(1).cloned().unwrap_or(JsValue::Null);
    let new_proto = match &proto {
        JsValue::Object(p) => Some(p.clone()),
        JsValue::Null => None,
        _ => {
            let err = vm.make_type_error("Object prototype may only be an Object or null");
            vm.throw_native(err);
            return JsValue::Undefined;
        }
    };
    if let JsValue::Object(o) = &obj {
        if !set_prototype_of_internal(vm, o, new_proto) {
            let err = vm.make_type_error("Cannot set prototype");
            vm.throw_native(err);
            return JsValue::Undefined;
        }
    }
    obj
}

/// `Object.getOwnPropertyNames(obj)` — all own property names (including non-enumerable).
pub fn object_get_own_property_names(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            let mut keys: Vec<JsValue> = Vec::new();
            if let Some(s) = boxed_string_value(&o) {
                for i in 0..s.chars().count() {
                    keys.push(JsValue::String(format_usize(i)));
                }
                keys.push(JsValue::String(String::from("length")));
            }
            keys.extend(
                o.own_property_names()
                    .into_iter()
                    .filter(|k| !(k == "length" && boxed_string_value(&o).is_some()))
                    .map(JsValue::String),
            );
            JsValue::new_array(keys)
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            let mut keys: Vec<JsValue> = a
                .elements
                .keys()
                .map(|&i| JsValue::String(format_usize(i)))
                .collect();
            // Also include non-numeric property names
            for key in a.properties.keys() {
                if key.parse::<usize>().is_err() {
                    keys.push(JsValue::String(key.clone()));
                }
            }
            keys.push(JsValue::String(String::from("length")));
            JsValue::new_array(keys)
        }
        Some(JsValue::Function(f)) => {
            let func = f.borrow();
            let mut keys: Vec<JsValue> = Vec::new();
            if !function_builtin_deleted(&func, "length") {
                keys.push(JsValue::String(String::from("length")));
            }
            if !function_builtin_deleted(&func, "name") {
                keys.push(JsValue::String(String::from("name")));
            }
            if !func.kind.is_arrow() && !function_builtin_deleted(&func, "prototype") {
                keys.push(JsValue::String(String::from("prototype")));
            }
            for k in function_public_own_prop_names(&func) {
                if k != "length" && k != "name" && k != "prototype" {
                    keys.push(JsValue::String(k));
                }
            }
            JsValue::new_array(keys)
        }
        Some(JsValue::String(s)) => {
            let mut keys: Vec<JsValue> = (0..s.chars().count())
                .map(|i| JsValue::String(format_usize(i)))
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
            } else if let Some(s) = boxed_string_value(&o) {
                string_exotic_own_property_descriptor(&s, &key)
                    .map(|p| prop_to_descriptor(&p))
                    .unwrap_or(JsValue::Undefined)
            } else {
                JsValue::Undefined
            }
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            // Check properties first (for accessor descriptors set via defineProperty)
            if let Some(prop) = a.properties.get(&key) {
                return prop_to_descriptor(prop);
            }
            // Check numeric elements
            if let Ok(idx) = key.parse::<usize>() {
                if let Some(val) = a.elements.get(&idx) {
                    return prop_to_descriptor(&Property::data(val.clone()));
                }
            }
            // Built-in: length
            if key == "length" {
                return prop_to_descriptor(&Property {
                    value: JsValue::Number(a.length as f64),
                    writable: true,
                    enumerable: false,
                    configurable: false,
                    getter: None,
                    setter: None,
                });
            }
            JsValue::Undefined
        }
        Some(JsValue::Function(fn_rc)) => fn_get_own_property_descriptor(fn_rc, &key),
        Some(JsValue::String(s)) => string_exotic_own_property_descriptor(s, &key)
            .map(|p| prop_to_descriptor(&p))
            .unwrap_or(JsValue::Undefined),
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
        let mut prop = Property::accessor(getter, setter);
        prop.enumerable = function_descriptor_flag(&func, key, "enumerable", true);
        prop.configurable = function_descriptor_flag(&func, key, "configurable", true);
        return prop_to_descriptor(&prop);
    }
    // Check regular own_props first
    if let Some(val) = func.own_props.get(key) {
        let prop = Property {
            value: val.clone(),
            writable: function_descriptor_flag(&func, key, "writable", true),
            enumerable: function_descriptor_flag(&func, key, "enumerable", true),
            configurable: function_descriptor_flag(&func, key, "configurable", true),
            getter: None,
            setter: None,
        };
        return prop_to_descriptor(&prop);
    }
    // Built-in function properties (ES2023 §10.2.4, §20.2.4)
    match key {
        "name" => {
            if function_builtin_deleted(&func, "name") {
                return JsValue::Undefined;
            }
            let name_val = func
                .name
                .as_ref()
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
            if function_builtin_deleted(&func, "length") {
                return JsValue::Undefined;
            }
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
            if func.kind.is_arrow() || function_builtin_deleted(&func, "prototype") {
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
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            if let Some(s) = boxed_string_value(&o) {
                for i in 0..s.chars().count() {
                    if let Some(prop) =
                        string_exotic_own_property_descriptor(&s, &format_usize(i))
                    {
                        result.set_property(format_usize(i), prop_to_descriptor(&prop));
                    }
                }
                if let Some(prop) = string_exotic_own_property_descriptor(&s, "length") {
                    result.set_property(String::from("length"), prop_to_descriptor(&prop));
                }
            }
            for (key, prop) in &o.properties {
                let desc = prop_to_descriptor(prop);
                result.set_property(key.clone(), desc);
            }
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            for (idx, value) in &a.elements {
                let desc = prop_to_descriptor(&Property::data(value.clone()));
                result.set_property(format_usize(*idx), desc);
            }
            for (key, prop) in &a.properties {
                let desc = prop_to_descriptor(prop);
                result.set_property(key.clone(), desc);
            }
            result.set_property(
                String::from("length"),
                prop_to_descriptor(&Property {
                    value: JsValue::Number(a.length as f64),
                    writable: true,
                    enumerable: false,
                    configurable: false,
                    getter: None,
                    setter: None,
                }),
            );
        }
        Some(JsValue::Function(func)) => {
            let f = func.borrow();
            let length_desc = fn_get_own_property_descriptor(func, "length");
            if !matches!(length_desc, JsValue::Undefined) {
                result.set_property(String::from("length"), length_desc);
            }
            let name_desc = fn_get_own_property_descriptor(func, "name");
            if !matches!(name_desc, JsValue::Undefined) {
                result.set_property(String::from("name"), name_desc);
            }
            if !f.kind.is_arrow() {
                let proto_desc = fn_get_own_property_descriptor(func, "prototype");
                if !matches!(proto_desc, JsValue::Undefined) {
                    result.set_property(String::from("prototype"), proto_desc);
                }
            }
            for key in function_public_own_prop_names(&f) {
                let desc = fn_get_own_property_descriptor(func, &key);
                result.set_property(key, desc);
            }
        }
        Some(JsValue::String(s)) => {
            for i in 0..s.chars().count() {
                if let Some(prop) = string_exotic_own_property_descriptor(s, &format_usize(i)) {
                    result.set_property(format_usize(i), prop_to_descriptor(&prop));
                }
            }
            if let Some(prop) = string_exotic_own_property_descriptor(s, "length") {
                result.set_property(String::from("length"), prop_to_descriptor(&prop));
            }
        }
        _ => {}
    }
    result
}

fn prop_to_descriptor(prop: &Property) -> JsValue {
    let desc = JsValue::new_object();
    if prop.is_accessor() {
        if let Some(ref g) = prop.getter {
            desc.set_property(String::from("get"), g.clone());
        }
        if let Some(ref s) = prop.setter {
            desc.set_property(String::from("set"), s.clone());
        }
    } else {
        desc.set_property(String::from("value"), prop.value.clone());
        desc.set_property(String::from("writable"), JsValue::Bool(prop.writable));
    }
    desc.set_property(String::from("enumerable"), JsValue::Bool(prop.enumerable));
    desc.set_property(
        String::from("configurable"),
        JsValue::Bool(prop.configurable),
    );
    desc
}

/// `Object.preventExtensions(obj)`.
pub fn object_prevent_extensions(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(JsValue::Object(obj)) = args.first() {
        let mut o = obj.borrow_mut();
        // Use a hidden property to mark non-extensibility without overwriting internal_tag.
        o.properties.insert(
            String::from("__non_extensible__"),
            crate::value::Property {
                value: JsValue::Bool(true),
                writable: false,
                enumerable: false,
                configurable: false,
                getter: None,
                setter: None,
            },
        );
    }
    args.first().cloned().unwrap_or(JsValue::Undefined)
}

/// `Object.isExtensible(obj)`.
pub fn object_is_extensible(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            JsValue::Bool(!o.properties.contains_key("__non_extensible__"))
        }
        Some(JsValue::Function(_)) => JsValue::Bool(true),
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
        o.properties.insert(
            String::from("__non_extensible__"),
            crate::value::Property {
                value: JsValue::Bool(true),
                writable: false,
                enumerable: false,
                configurable: false,
                getter: None,
                setter: None,
            },
        );
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
            let all_frozen = o
                .properties
                .values()
                .all(|p| !p.writable && !p.configurable);
            JsValue::Bool(all_frozen)
        }
        _ => JsValue::Bool(true),
    }
}

/// `Object.hasOwn(obj, key)` — static version of hasOwnProperty.
pub fn object_has_own(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    match args.first() {
        Some(JsValue::Object(obj)) => {
            let o = obj.borrow();
            JsValue::Bool(
                o.has_own(&key)
                    || boxed_string_value(&o)
                        .map(|s| string_exotic_own_property_descriptor(&s, &key).is_some())
                        .unwrap_or(false),
            )
        }
        Some(JsValue::Array(arr)) => {
            let a = arr.borrow();
            if let Some(idx) = super::try_parse_index(&key) {
                JsValue::Bool(a.has(idx))
            } else {
                JsValue::Bool(key == "length" || a.properties.contains_key(&key))
            }
        }
        Some(JsValue::String(s)) => {
            JsValue::Bool(string_exotic_own_property_descriptor(s, &key).is_some())
        }
        Some(JsValue::Function(f)) => {
            let func = f.borrow();
            JsValue::Bool(
                func.own_props.contains_key(&key)
                    || key == "name"
                    || key == "length"
                    || (key == "prototype" && !func.kind.is_arrow()),
            )
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
            let syms: Vec<JsValue> = o
                .own_symbol_keys()
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
