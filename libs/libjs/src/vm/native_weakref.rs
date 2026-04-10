// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! WeakMap, WeakSet, WeakRef implementations.
//!
//! In our `no_std` environment without GC, these are simplified:
//! - WeakMap/WeakSet use strong references internally (no weak GC integration)
//!   but provide the correct API surface.
//! - WeakRef.deref() always returns the held value (never collected).
//! - FinalizationRegistry tracks register/unregister entries but never
//!   invokes its cleanup callback (no GC hook).

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::{native_fn, Vm};
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// WeakMap
// ═══════════════════════════════════════════════════════════

pub const WEAKMAP_TAG: &str = "__weakmap__";

/// `new WeakMap()`
pub fn ctor_weakmap(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    // Tag `this` as a WeakMap so that set/get/has/delete can identify it.
    // The `new_object` mechanism already set the correct prototype chain
    // (WeakMap.prototype with set/get/has/delete methods).
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().internal_tag = Some(String::from(WEAKMAP_TAG));
    }
    // Return undefined so `new_object` uses `this` (which has the right prototype).
    JsValue::Undefined
}

fn obj_ptr(val: &JsValue) -> Option<usize> {
    match val {
        JsValue::Object(rc) => Some(Rc::as_ptr(rc) as usize),
        JsValue::Array(rc) => Some(Rc::as_ptr(rc) as usize),
        JsValue::Function(rc) => Some(Rc::as_ptr(rc) as usize),
        _ => None,
    }
}

fn wm_key(ptr: usize) -> String {
    use alloc::format;
    format!("__wm_{}", ptr)
}

/// `WeakMap.prototype.set(key, value)`
pub fn weakmap_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);

    if let Some(ptr) = obj_ptr(&key) {
        this.set_property(wm_key(ptr), value);
    }
    this
}

/// `WeakMap.prototype.get(key)`
pub fn weakmap_get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(ptr) = obj_ptr(&key) {
        let val = this.get_property(&wm_key(ptr));
        if val.is_undefined() {
            JsValue::Undefined
        } else {
            val
        }
    } else {
        JsValue::Undefined
    }
}

/// `WeakMap.prototype.has(key)`
pub fn weakmap_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(ptr) = obj_ptr(&key) {
        let val = this.get_property(&wm_key(ptr));
        JsValue::Bool(!val.is_undefined())
    } else {
        JsValue::Bool(false)
    }
}

/// `WeakMap.prototype.delete(key)`
pub fn weakmap_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let key = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(ptr) = obj_ptr(&key) {
        JsValue::Bool(this.delete_property(&wm_key(ptr)))
    } else {
        JsValue::Bool(false)
    }
}

// ═══════════════════════════════════════════════════════════
// WeakSet
// ═══════════════════════════════════════════════════════════

pub const WEAKSET_TAG: &str = "__weakset__";

/// `new WeakSet()`
pub fn ctor_weakset(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().internal_tag = Some(String::from(WEAKSET_TAG));
    }
    JsValue::Undefined
}

/// `WeakSet.prototype.add(value)`
pub fn weakset_add(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(ptr) = obj_ptr(&value) {
        this.set_property(wm_key(ptr), JsValue::Bool(true));
    }
    this
}

/// `WeakSet.prototype.has(value)`
pub fn weakset_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(ptr) = obj_ptr(&value) {
        let val = this.get_property(&wm_key(ptr));
        JsValue::Bool(!val.is_undefined())
    } else {
        JsValue::Bool(false)
    }
}

/// `WeakSet.prototype.delete(value)`
pub fn weakset_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(ptr) = obj_ptr(&value) {
        JsValue::Bool(this.delete_property(&wm_key(ptr)))
    } else {
        JsValue::Bool(false)
    }
}

// ═══════════════════════════════════════════════════════════
// WeakRef
// ═══════════════════════════════════════════════════════════

pub const WEAKREF_TAG: &str = "__weakref__";

/// `new WeakRef(target)`
pub fn ctor_weakref(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(obj) = &vm.current_this {
        let mut o = obj.borrow_mut();
        o.internal_tag = Some(String::from(WEAKREF_TAG));
        o.set(String::from("__target"), target);
    }
    JsValue::Undefined
}

/// `WeakRef.prototype.deref()`
pub fn weakref_deref(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    if let JsValue::Object(obj) = &this {
        let o = obj.borrow();
        let target = o.get("__target");
        if target.is_undefined() || target.is_null() {
            JsValue::Undefined
        } else {
            target
        }
    } else {
        JsValue::Undefined
    }
}

// ═══════════════════════════════════════════════════════════
// FinalizationRegistry
// ═══════════════════════════════════════════════════════════
//
// Without GC integration the cleanup callback can never be invoked, but
// register/unregister still maintain the registered cell list so that
// programs which round-trip values through the registry observe consistent
// behaviour. Each registry stores a `__entries` array of `[target, held,
// token]` triples on a hidden slot.

/// `new FinalizationRegistry(callback)`
///
/// ES2023 §26.2.1.1: throws TypeError if `callback` is not callable.
pub fn ctor_finalization_registry(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if !matches!(callback, JsValue::Function(_)) {
        let err = vm.make_type_error("FinalizationRegistry callback must be callable");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let mut obj = JsObject::new();
    obj.prototype = Some(vm.object_proto.clone());
    obj.internal_tag = Some(String::from("__finalization_registry__"));
    obj.set_hidden(String::from("__callback"), callback);
    obj.set_hidden(
        String::from("__entries"),
        JsValue::Array(Rc::new(RefCell::new(JsArray::new()))),
    );
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn fr_entries(this: &JsValue) -> Option<Rc<RefCell<JsArray>>> {
    if let JsValue::Object(obj) = this {
        if let JsValue::Array(arr) = obj.borrow().get("__entries") {
            return Some(arr);
        }
    }
    None
}

/// `FinalizationRegistry.prototype.register(target, heldValue [, unregisterToken])`
///
/// ES2023 §26.2.3.2. Stores the entry; the cleanup callback is never invoked
/// in this implementation because there is no GC hook, but `unregister` can
/// still locate and remove the entry by token.
pub fn fr_register(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    if !matches!(
        target,
        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
    ) {
        let err = vm.make_type_error("FinalizationRegistry.register target must be an object");
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let held = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let token = args.get(2).cloned().unwrap_or(JsValue::Undefined);
    if let Some(entries) = fr_entries(&this) {
        let entry = JsArray::new();
        let entry_rc = Rc::new(RefCell::new(entry));
        {
            let mut e = entry_rc.borrow_mut();
            e.push(target);
            e.push(held);
            e.push(token);
        }
        entries.borrow_mut().push(JsValue::Array(entry_rc));
    }
    JsValue::Undefined
}

/// `FinalizationRegistry.prototype.unregister(token)`
///
/// ES2023 §26.2.3.3. Removes every entry with a matching unregister token
/// using SameValue comparison. Returns `true` if at least one entry was
/// removed.
pub fn fr_unregister(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let token = args.first().cloned().unwrap_or(JsValue::Undefined);
    if !matches!(
        token,
        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
    ) {
        let err = vm.make_type_error(
            "FinalizationRegistry.unregister token must be an object",
        );
        vm.throw_native(err);
        return JsValue::Undefined;
    }
    let Some(entries) = fr_entries(&this) else {
        return JsValue::Bool(false);
    };
    let mut removed = false;
    let mut arr = entries.borrow_mut();
    let mut keep: alloc::vec::Vec<JsValue> = alloc::vec::Vec::new();
    let len = arr.length;
    for i in 0..len {
        let entry = arr.get(i);
        let entry_token = if let JsValue::Array(inner) = &entry {
            inner.borrow().get(2)
        } else {
            JsValue::Undefined
        };
        if same_value_object(&entry_token, &token) {
            removed = true;
        } else {
            keep.push(entry);
        }
    }
    arr.elements.clear();
    arr.length = 0;
    for v in keep {
        arr.push(v);
    }
    JsValue::Bool(removed)
}

/// SameValue comparison restricted to object identity (Rc pointer equality).
/// Sufficient for unregister tokens, which must be objects.
fn same_value_object(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Object(x), JsValue::Object(y)) => Rc::ptr_eq(x, y),
        (JsValue::Array(x), JsValue::Array(y)) => Rc::ptr_eq(x, y),
        (JsValue::Function(x), JsValue::Function(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}
