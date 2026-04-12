//! Map and Set built-in objects.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::{native_fn, Vm};
use crate::value::*;

const MAP_TAG: &str = "Map";
const SET_TAG: &str = "Set";

fn make_iterator(vm: &Vm, items: Vec<JsValue>) -> JsValue {
    let items_arr = JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(items))));
    let mut iter_obj = JsObject::with_tag("__iterator__");
    iter_obj.prototype = Some(vm.iterator_proto.clone());
    iter_obj.set(String::from("__items__"), items_arr);
    iter_obj.set(String::from("__index__"), JsValue::Number(0.0));
    JsValue::Object(Rc::new(RefCell::new(iter_obj)))
}

// ═══════════════════════════════════════════════════════════
// Map constructor and prototype
// ═══════════════════════════════════════════════════════════

pub fn ctor_map(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Map requires construction via `new`.
    let obj_rc = match &vm.current_this {
        JsValue::Object(rc) => rc.clone(),
        _ => {
            let exc = vm.make_type_error("Constructor Map requires 'new'");
            vm.throw_native(exc);
            return JsValue::Undefined;
        }
    };

    // Tag `this` (created by `new_object` with the correct prototype chain).
    {
        let mut o = obj_rc.borrow_mut();
        o.internal_tag = Some(String::from(MAP_TAG));
        o.set(String::from("__keys"), JsValue::new_array(Vec::new()));
        o.set(String::from("__values"), JsValue::new_array(Vec::new()));
    }
    // Handle iterable argument: new Map(iterable)
    // Accepts any iterable of [key, value] pairs (Array, Map, Set, generator, etc.)
    if let Some(iterable) = args.first() {
        if !iterable.is_undefined() && !iterable.is_null() {
            // Collect entries from iterable using the iterator protocol.
            let entries = collect_iterable_entries(vm, iterable);
            for entry in &entries {
                let (key, val) = extract_pair(entry);
                {
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

/// Collect all values from an iterable (Array, Set, Map, generator, etc.)
/// using the iterator protocol (Symbol.iterator → .next()).
fn collect_iterable_entries(vm: &mut Vm, iterable: &JsValue) -> Vec<JsValue> {
    // Fast path: if it's a plain Array, just clone elements directly.
    if let JsValue::Array(arr) = iterable {
        return arr.borrow().to_dense_vec();
    }
    // Generic iterable: use the VM's iterator protocol.
    let iter = vm.create_iterator(iterable);
    if vm.pending_exception.is_some() {
        return Vec::new();
    }
    // Push iterator onto stack for iter_next_mut.
    vm.stack.push(iter);
    let mut items = Vec::new();
    loop {
        let (val, has_more) = vm.iter_next_mut();
        if !has_more {
            break;
        }
        items.push(val);
        if vm.pending_exception.is_some() {
            break;
        }
    }
    vm.stack.pop(); // Remove iterator from stack.
    items
}

/// Extract (key, value) from a Map constructor entry.
/// Accepts [key, value] arrays or falls back to (entry, undefined).
fn extract_pair(entry: &JsValue) -> (JsValue, JsValue) {
    if let JsValue::Array(pair) = entry {
        let p = pair.borrow();
        (p.get(0), p.get(1))
    } else {
        (entry.clone(), JsValue::Undefined)
    }
}

fn expect_map_this(vm: &mut Vm) -> Option<Rc<RefCell<JsObject>>> {
    let this = vm.current_this.clone();
    if let JsValue::Object(obj_rc) = this {
        let obj = obj_rc.borrow();
        let tag = obj.internal_tag.clone();
        let has_storage = matches!(obj.get("__keys"), JsValue::Array(_))
            && matches!(obj.get("__values"), JsValue::Array(_));
        drop(obj);
        if matches!(tag.as_deref(), Some(MAP_TAG) | Some("__map__")) || has_storage {
            return Some(obj_rc);
        }
    }
    let exc = vm.make_type_error("Incorrect Map invocation");
    vm.throw_native(exc);
    None
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
    obj_rc.borrow_mut().properties.remove("size");
}

pub fn map_size_get(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(obj_rc) = expect_map_this(vm) {
        let size = {
            let o = obj_rc.borrow();
            if let JsValue::Array(keys) = o.get("__keys") {
                keys.borrow().count() as f64
            } else {
                0.0
            }
        };
        return JsValue::Number(size);
    }
    JsValue::Undefined
}

pub fn map_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let this = vm.current_this.clone();

    if let Some(obj_rc) = expect_map_this(vm) {
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
    }
    this
}

pub fn map_get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(obj_rc) = expect_map_this(vm) {
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
    if let Some(obj_rc) = expect_map_this(vm) {
        let o = obj_rc.borrow();
        return JsValue::Bool(map_find_index(&o, &key).is_some());
    }
    JsValue::Bool(false)
}

pub fn map_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(obj_rc) = expect_map_this(vm) {
        let idx = {
            let o = obj_rc.borrow();
            map_find_index(&o, &key)
        };
        if let Some(idx) = idx {
            let o = obj_rc.borrow();
            if let (JsValue::Array(keys), JsValue::Array(vals)) =
                (o.get("__keys"), o.get("__values"))
            {
                keys.borrow_mut().remove_and_shift(idx);
                vals.borrow_mut().remove_and_shift(idx);
            }
            drop(o);
            update_size(&obj_rc);
            return JsValue::Bool(true);
        }
    }
    JsValue::Bool(false)
}

pub fn map_clear(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(obj_rc) = expect_map_this(vm) {
        let o = obj_rc.borrow();
        if let JsValue::Array(keys) = o.get("__keys") {
            keys.borrow_mut().clear();
        }
        if let JsValue::Array(vals) = o.get("__values") {
            vals.borrow_mut().clear();
        }
        drop(o);
        update_size(&obj_rc);
    }
    JsValue::Undefined
}

pub fn map_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(obj_rc) = expect_map_this(vm) {
        let o = obj_rc.borrow();
        if let JsValue::Array(keys) = o.get("__keys") {
            return make_iterator(vm, keys.borrow().values_vec());
        }
    }
    make_iterator(vm, Vec::new())
}

pub fn map_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(obj_rc) = expect_map_this(vm) {
        let o = obj_rc.borrow();
        if let JsValue::Array(vals) = o.get("__values") {
            return make_iterator(vm, vals.borrow().values_vec());
        }
    }
    make_iterator(vm, Vec::new())
}

pub fn map_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(obj_rc) = expect_map_this(vm) {
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
            return make_iterator(vm, entries);
        }
    }
    make_iterator(vm, Vec::new())
}

pub fn map_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(obj_rc) = expect_map_this(vm) {
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
    }
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════════
// Set constructor and prototype
// ═══════════════════════════════════════════════════════════

pub fn ctor_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let obj_rc = match &vm.current_this {
        JsValue::Object(rc) => rc.clone(),
        _ => {
            let exc = vm.make_type_error("Constructor Set requires 'new'");
            vm.throw_native(exc);
            return JsValue::Undefined;
        }
    };

    {
        let mut o = obj_rc.borrow_mut();
        o.internal_tag = Some(String::from(SET_TAG));
        o.set(String::from("__items"), JsValue::new_array(Vec::new()));
    }
    // Pre-populate from iterable argument if provided.
    if let Some(iterable) = args.first() {
        if !iterable.is_undefined() && !iterable.is_null() {
            let elements = collect_iterable_entries(vm, iterable);
            for v in &elements {
                {
                    if let JsValue::Array(items) = obj_rc.borrow().get("__items") {
                        let mut items_mut = items.borrow_mut();
                        let has = items_mut.elements.values().any(|s| s.strict_eq(v));
                        if !has {
                            items_mut.push(v.clone());
                        }
                    }
                    update_set_size(&obj_rc);
                }
            }
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

fn expect_set_this(vm: &mut Vm) -> Option<Rc<RefCell<JsObject>>> {
    let this = vm.current_this.clone();
    if let JsValue::Object(obj_rc) = this {
        let obj = obj_rc.borrow();
        let tag = obj.internal_tag.clone();
        let has_storage = matches!(obj.get("__items"), JsValue::Array(_));
        drop(obj);
        if matches!(tag.as_deref(), Some(SET_TAG) | Some("__set__")) || has_storage {
            return Some(obj_rc);
        }
    }
    let exc = vm.make_type_error("Incorrect Set invocation");
    vm.throw_native(exc);
    None
}

fn update_set_size(obj_rc: &Rc<RefCell<JsObject>>) {
    obj_rc.borrow_mut().properties.remove("size");
}

pub fn set_size_get(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(obj_rc) = expect_set_this(vm) {
        let size = {
            let o = obj_rc.borrow();
            if let JsValue::Array(items) = o.get("__items") {
                items.borrow().count() as f64
            } else {
                0.0
            }
        };
        return JsValue::Number(size);
    }
    JsValue::Undefined
}

pub fn set_add(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let this = vm.current_this.clone();
    if let Some(obj_rc) = expect_set_this(vm) {
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
    }
    this
}

pub fn set_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(obj_rc) = expect_set_this(vm) {
        let o = obj_rc.borrow();
        return JsValue::Bool(set_find_index(&o, &value).is_some());
    }
    JsValue::Bool(false)
}

pub fn set_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(obj_rc) = expect_set_this(vm) {
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
    }
    JsValue::Bool(false)
}

pub fn set_clear(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(obj_rc) = expect_set_this(vm) {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            items.borrow_mut().clear();
        }
        drop(o);
        update_set_size(&obj_rc);
    }
    JsValue::Undefined
}

pub fn set_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(obj_rc) = expect_set_this(vm) {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            return make_iterator(vm, items.borrow().values_vec());
        }
    }
    make_iterator(vm, Vec::new())
}

pub fn set_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(obj_rc) = expect_set_this(vm) {
        let o = obj_rc.borrow();
        if let JsValue::Array(items) = o.get("__items") {
            let entries: Vec<JsValue> = items
                .borrow()
                .elements
                .values()
                .map(|v| JsValue::new_array(alloc::vec![v.clone(), v.clone()]))
                .collect();
            return make_iterator(vm, entries);
        }
    }
    make_iterator(vm, Vec::new())
}

pub fn set_for_each(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(obj_rc) = expect_set_this(vm) {
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
    }
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════════
// ES2025 Set methods
// ═══════════════════════════════════════════════════════════

fn get_set_items(obj_rc: &Rc<RefCell<JsObject>>) -> Vec<JsValue> {
    let o = obj_rc.borrow();
    if let JsValue::Array(arr) = o.get("__items") {
        arr.borrow().values_vec()
    } else {
        Vec::new()
    }
}

fn get_other_set_items(vm: &mut Vm, args: &[JsValue]) -> Vec<JsValue> {
    let other = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(obj) = &other {
        if obj.borrow().internal_tag.as_deref() == Some("Set") {
            return get_set_items(obj);
        }
    }
    // Try iterating over the argument as an iterable.
    if let JsValue::Array(arr) = &other {
        return arr.borrow().values_vec();
    }
    Vec::new()
}

fn make_new_set(vm: &Vm, items: Vec<JsValue>) -> JsValue {
    // Deduplicate.
    let mut unique = Vec::new();
    for v in items {
        if !unique.iter().any(|u: &JsValue| u.strict_eq(&v)) {
            unique.push(v);
        }
    }
    let mut obj = JsObject::with_tag("Set");
    obj.set(
        String::from("__items"),
        JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(unique)))),
    );
    // Set prototype methods (reuse the Set constructor's proto).
    if let JsValue::Function(ctor) = vm.globals.borrow().get("Set") {
        if let Some(proto) = &ctor.borrow().prototype {
            obj.prototype = Some(proto.clone());
        }
    }
    let obj_rc = Rc::new(RefCell::new(obj));
    update_set_size(&obj_rc);
    JsValue::Object(obj_rc)
}

/// `Set.prototype.union(other)` — ES2025
pub fn set_union(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_items = expect_set_this(vm)
        .map(|rc| get_set_items(&rc))
        .unwrap_or_default();
    let other_items = get_other_set_items(vm, args);
    let mut combined = this_items;
    for v in other_items {
        if !combined.iter().any(|u| u.strict_eq(&v)) {
            combined.push(v);
        }
    }
    make_new_set(vm, combined)
}

/// `Set.prototype.intersection(other)` — ES2025
pub fn set_intersection(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_items = expect_set_this(vm)
        .map(|rc| get_set_items(&rc))
        .unwrap_or_default();
    let other_items = get_other_set_items(vm, args);
    let result: Vec<JsValue> = this_items
        .into_iter()
        .filter(|v| other_items.iter().any(|u| u.strict_eq(v)))
        .collect();
    make_new_set(vm, result)
}

/// `Set.prototype.difference(other)` — ES2025
pub fn set_difference(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_items = expect_set_this(vm)
        .map(|rc| get_set_items(&rc))
        .unwrap_or_default();
    let other_items = get_other_set_items(vm, args);
    let result: Vec<JsValue> = this_items
        .into_iter()
        .filter(|v| !other_items.iter().any(|u| u.strict_eq(v)))
        .collect();
    make_new_set(vm, result)
}

/// `Set.prototype.symmetricDifference(other)` — ES2025
pub fn set_symmetric_difference(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_items = expect_set_this(vm)
        .map(|rc| get_set_items(&rc))
        .unwrap_or_default();
    let other_items = get_other_set_items(vm, args);
    let mut result = Vec::new();
    // Items in this but not other.
    for v in &this_items {
        if !other_items.iter().any(|u| u.strict_eq(v)) {
            result.push(v.clone());
        }
    }
    // Items in other but not this.
    for v in &other_items {
        if !this_items.iter().any(|u| u.strict_eq(v)) {
            result.push(v.clone());
        }
    }
    make_new_set(vm, result)
}

/// `Set.prototype.isSubsetOf(other)` — ES2025
pub fn set_is_subset_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_items = expect_set_this(vm)
        .map(|rc| get_set_items(&rc))
        .unwrap_or_default();
    let other_items = get_other_set_items(vm, args);
    JsValue::Bool(
        this_items
            .iter()
            .all(|v| other_items.iter().any(|u| u.strict_eq(v))),
    )
}

/// `Set.prototype.isSupersetOf(other)` — ES2025
pub fn set_is_superset_of(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_items = expect_set_this(vm)
        .map(|rc| get_set_items(&rc))
        .unwrap_or_default();
    let other_items = get_other_set_items(vm, args);
    JsValue::Bool(
        other_items
            .iter()
            .all(|v| this_items.iter().any(|u| u.strict_eq(v))),
    )
}

/// `Set.prototype.isDisjointFrom(other)` — ES2025
pub fn set_is_disjoint_from(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this_items = expect_set_this(vm)
        .map(|rc| get_set_items(&rc))
        .unwrap_or_default();
    let other_items = get_other_set_items(vm, args);
    JsValue::Bool(
        !this_items
            .iter()
            .any(|v| other_items.iter().any(|u| u.strict_eq(v))),
    )
}
