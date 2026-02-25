//! Proxy — ES6+ meta-programming.
//!
//! Simplified implementation: a Proxy wraps a target object and a
//! handler object.  When properties are accessed/set on the proxy,
//! we check the handler for `get`, `set`, `has`, `deleteProperty`
//! traps and invoke them if present.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::value::*;
use super::{Vm, native_fn};

// ═══════════════════════════════════════════════════════════
// Proxy constructor
// ═══════════════════════════════════════════════════════════

/// `new Proxy(target, handler)` — creates a proxy object.
pub fn ctor_proxy(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let handler = args.get(1).cloned().unwrap_or(JsValue::Undefined);

    let mut obj = JsObject::new();
    obj.internal_tag = Some(String::from("__proxy__"));
    obj.set(String::from("__target"), target);
    obj.set(String::from("__handler"), handler);
    // Proxy-specific methods for the host to trigger traps
    obj.set(String::from("__get"), native_fn("__get", proxy_get_trap));
    obj.set(String::from("__set"), native_fn("__set", proxy_set_trap));
    obj.set(String::from("__has"), native_fn("__has", proxy_has_trap));
    obj.set(String::from("__deleteProperty"), native_fn("__deleteProperty", proxy_delete_trap));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// `Proxy.revocable(target, handler)` — returns { proxy, revoke }.
pub fn proxy_revocable(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let proxy = ctor_proxy(vm, args);
    let result = JsValue::new_object();
    result.set_property(String::from("proxy"), proxy);
    result.set_property(String::from("revoke"), native_fn("revoke", proxy_revoke));
    result
}

fn proxy_revoke(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    // Simplified: set target and handler to null
    if let JsValue::Object(obj) = &vm.current_this {
        if let JsValue::Object(proxy_obj) = obj.borrow().get("proxy") {
            let mut p = proxy_obj.borrow_mut();
            p.set(String::from("__target"), JsValue::Null);
            p.set(String::from("__handler"), JsValue::Null);
        }
    }
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════════
// Trap implementations
// ═══════════════════════════════════════════════════════════

/// Internal: invoke the handler's `get` trap or fall through to target.
fn proxy_get_trap(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let prop = args.first().map(|v| v.to_js_string()).unwrap_or_default();

    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        let handler = o.get("__handler");
        let target = o.get("__target");
        drop(o);

        // Check for get trap in handler
        if let JsValue::Object(h) = &handler {
            let get_fn = h.borrow().get("get");
            if let JsValue::Function(f) = get_fn {
                let kind = f.borrow().kind.clone();
                if let FnKind::Native(native) = kind {
                    return native(vm, &[target, JsValue::String(prop)]);
                }
            }
        }

        // No trap — fall through to target
        target.get_property(&prop)
    } else {
        JsValue::Undefined
    }
}

/// Internal: invoke the handler's `set` trap or fall through to target.
fn proxy_set_trap(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let prop = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);

    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        let handler = o.get("__handler");
        let target = o.get("__target");
        drop(o);

        if let JsValue::Object(h) = &handler {
            let set_fn = h.borrow().get("set");
            if let JsValue::Function(f) = set_fn {
                let kind = f.borrow().kind.clone();
                if let FnKind::Native(native) = kind {
                    return native(vm, &[target, JsValue::String(prop), value]);
                }
            }
        }

        target.set_property(prop, value);
    }
    JsValue::Bool(true)
}

/// Internal: invoke the handler's `has` trap.
fn proxy_has_trap(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let prop = args.first().map(|v| v.to_js_string()).unwrap_or_default();

    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        let handler = o.get("__handler");
        let target = o.get("__target");
        drop(o);

        if let JsValue::Object(h) = &handler {
            let has_fn = h.borrow().get("has");
            if let JsValue::Function(f) = has_fn {
                let kind = f.borrow().kind.clone();
                if let FnKind::Native(native) = kind {
                    return native(vm, &[target, JsValue::String(prop)]);
                }
            }
        }

        match &target {
            JsValue::Object(t) => JsValue::Bool(t.borrow().has(&prop)),
            _ => JsValue::Bool(false),
        }
    } else {
        JsValue::Bool(false)
    }
}

/// Internal: invoke the handler's `deleteProperty` trap.
fn proxy_delete_trap(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let prop = args.first().map(|v| v.to_js_string()).unwrap_or_default();

    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        let handler = o.get("__handler");
        let target = o.get("__target");
        drop(o);

        if let JsValue::Object(h) = &handler {
            let del_fn = h.borrow().get("deleteProperty");
            if let JsValue::Function(f) = del_fn {
                let kind = f.borrow().kind.clone();
                if let FnKind::Native(native) = kind {
                    return native(vm, &[target, JsValue::String(prop)]);
                }
            }
        }

        JsValue::Bool(target.delete_property(&prop))
    } else {
        JsValue::Bool(false)
    }
}
