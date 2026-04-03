//! WeakMap, WeakSet, WeakRef implementations.
//!
//! In our `no_std` environment without GC, these are simplified:
//! - WeakMap/WeakSet use strong references internally (no weak GC integration)
//!   but provide the correct API surface.
//! - WeakRef.deref() always returns the held value (never collected).
//! - FinalizationRegistry is a stub.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::rc::Rc;
use core::cell::RefCell;
use alloc::collections::BTreeMap;

use crate::value::*;
use super::{Vm, native_fn};

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
        if val.is_undefined() { JsValue::Undefined } else { val }
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
// FinalizationRegistry (stub)
// ═══════════════════════════════════════════════════════════

/// `new FinalizationRegistry(callback)` — stub
pub fn ctor_finalization_registry(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut obj = JsObject::new();
    obj.prototype = Some(vm.object_proto.clone());
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    obj.set_hidden(String::from("__callback"), callback);
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// `FinalizationRegistry.prototype.register(target, heldValue)` — no-op
pub fn fr_register(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

/// `FinalizationRegistry.prototype.unregister(token)` — no-op
pub fn fr_unregister(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(false)
}
