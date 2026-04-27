//! Promise — simplified synchronous implementation.
//!
//! Since our VM is single-threaded and has no event loop, Promises
//! resolve synchronously during construction.  This is sufficient for
//! the vast majority of web-page JS that uses `new Promise(...)`,
//! `.then()`, `.catch()`, `Promise.resolve()`, `Promise.reject()`,
//! and `Promise.all()`.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::{native_fn, Vm};
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// Promise constructor
// ═══════════════════════════════════════════════════════════

/// `new Promise(executor)` — creates a Promise and runs executor synchronously.
pub fn ctor_promise(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let executor = args.first().cloned().unwrap_or(JsValue::Undefined);

    // Tag `this` (created by `new_object` with the correct prototype chain).
    if let JsValue::Object(obj_rc) = &vm.current_this {
        let mut o = obj_rc.borrow_mut();
        o.internal_tag = Some(String::from("__promise__"));
        o.set(
            String::from("__state"),
            JsValue::String(String::from("pending")),
        );
        o.set(String::from("__value"), JsValue::Undefined);
        o.set(String::from("__then_cbs"), JsValue::new_array(Vec::new()));
        o.set(String::from("__catch_cbs"), JsValue::new_array(Vec::new()));
    }

    let promise = vm.current_this.clone();

    // Execute the executor(resolve, reject) synchronously
    if executor.is_function() {
        let promise_clone = promise.clone();

        // Store promise reference for resolve/reject to find via globals.
        vm.set_global("__promise_pending", promise_clone);
        vm.register_native("__promise_resolve_fn", promise_resolve_native);
        vm.register_native("__promise_reject_fn", promise_reject_native);

        let resolve_fn = vm.get_global("__promise_resolve_fn");
        let reject_fn = vm.get_global("__promise_reject_fn");

        vm.call_value(&executor, &[resolve_fn, reject_fn], JsValue::Undefined);
    }

    JsValue::Undefined // Return undefined → new_object uses this
}

fn promise_proto(vm: &Vm) -> Option<Rc<RefCell<JsObject>>> {
    match vm.globals.borrow().get("Promise") {
        JsValue::Function(func) => func.borrow().prototype.clone(),
        _ => None,
    }
}

fn attach_promise_shape(
    obj: &mut JsObject,
    proto: Option<Rc<RefCell<JsObject>>>,
    state: &str,
    value: JsValue,
) {
    obj.internal_tag = Some(String::from("__promise__"));
    obj.prototype = proto;
    obj.set(String::from("__state"), JsValue::String(String::from(state)));
    obj.set(String::from("__value"), value);
    obj.set(String::from("__then_cbs"), JsValue::new_array(Vec::new()));
    obj.set(String::from("__catch_cbs"), JsValue::new_array(Vec::new()));
    obj.set(String::from("then"), native_fn("then", promise_then));
    obj.set(String::from("catch"), native_fn("catch", promise_catch));
    obj.set(String::from("finally"), native_fn("finally", promise_finally));
}

fn make_promise(vm: &Vm, state: &str, value: JsValue) -> JsValue {
    let mut obj = JsObject::new();
    attach_promise_shape(&mut obj, promise_proto(vm), state, value);
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn is_internal_promise(value: &JsValue) -> bool {
    matches!(
        value,
        JsValue::Object(obj) if obj.borrow().internal_tag.as_deref() == Some("__promise__")
    )
}

fn make_bound_native(
    name: &str,
    native: fn(&mut Vm, &[JsValue]) -> JsValue,
    bound_args: Vec<JsValue>,
) -> JsValue {
    JsValue::Function(Rc::new(RefCell::new(JsFunction {
        name: Some(String::from(name)),
        params: Vec::new(),
        kind: FnKind::Native(native),
        object_proto: None,
        this_binding: None,
        bound_args,
        upvalues: Vec::new(),
        with_scopes: Vec::new(),
        prototype: None,
        own_props: alloc::collections::BTreeMap::new(),
        arity: None,
        super_class: None,
    })))
}

fn chain_internal_promise(vm: &mut Vm, source: &JsValue, target: &JsValue) {
    if let JsValue::Object(source_obj) = source {
        let state = source_obj.borrow().get("__state").to_js_string();
        if state == "fulfilled" || state == "rejected" {
            let value = source_obj.borrow().get("__value");
            settle_promise(vm, target, &state, &value);
            return;
        }

        let source_ref = source_obj.borrow();
        if let JsValue::Array(arr) = source_ref.get("__then_cbs") {
            let mut entry = JsObject::new();
            entry.set(String::from("cb"), JsValue::Undefined);
            entry.set(String::from("promise"), target.clone());
            arr.borrow_mut()
                .push(JsValue::Object(Rc::new(RefCell::new(entry))));
        }
        if let JsValue::Array(arr) = source_ref.get("__catch_cbs") {
            let mut entry = JsObject::new();
            entry.set(String::from("cb"), JsValue::Undefined);
            entry.set(String::from("promise"), target.clone());
            arr.borrow_mut()
                .push(JsValue::Object(Rc::new(RefCell::new(entry))));
        }
    }
}

fn thenable_resolve_runner(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    settle_promise(vm, &target, "fulfilled", &value);
    JsValue::Undefined
}

fn thenable_reject_runner(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    settle_promise(vm, &target, "rejected", &value);
    JsValue::Undefined
}

fn adopt_thenable(vm: &mut Vm, target: &JsValue, thenable: &JsValue) -> bool {
    if is_internal_promise(thenable) {
        chain_internal_promise(vm, thenable, target);
        return true;
    }

    let then_fn = thenable.get_property("then");
    if !then_fn.is_function() {
        return false;
    }

    let resolve = make_bound_native(
        "__promise_thenable_resolve__",
        thenable_resolve_runner,
        vec![target.clone()],
    );
    let reject = make_bound_native(
        "__promise_thenable_reject__",
        thenable_reject_runner,
        vec![target.clone()],
    );

    vm.call_value(&then_fn, &[resolve, reject], thenable.clone());
    if let Some(exc) = vm.last_exception.take() {
        settle_promise(vm, target, "rejected", &exc);
    }
    true
}

fn settle_chained_result(vm: &mut Vm, target: &JsValue, result: JsValue) {
    if adopt_thenable(vm, target, &result) {
        return;
    }
    settle_promise(vm, target, "fulfilled", &result);
}

fn promise_resolve_native(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promise = vm.get_global("__promise_pending");
    settle_promise(vm, &promise, "fulfilled", &value);
    JsValue::Undefined
}

fn promise_reject_native(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promise = vm.get_global("__promise_pending");
    settle_promise(vm, &promise, "rejected", &value);
    JsValue::Undefined
}

fn settle_promise(vm: &mut Vm, promise: &JsValue, state: &str, value: &JsValue) {
    if let JsValue::Object(obj) = promise {
        {
            let mut o = obj.borrow_mut();
            // Only settle if still pending
            let current_state = o.get("__state").to_js_string();
            if current_state != "pending" {
                return;
            }
            o.set(
                String::from("__state"),
                JsValue::String(String::from(state)),
            );
            o.set(String::from("__value"), value.clone());
        }

        // Run appropriate callbacks as microtasks
        let cb_key = if state == "fulfilled" {
            "__then_cbs"
        } else {
            "__catch_cbs"
        };
        let cbs = {
            let o = obj.borrow();
            o.get(cb_key)
        };
        if let JsValue::Array(arr) = cbs {
            let entries = arr.borrow().to_dense_vec();
            for entry in &entries {
                // New format: each entry is { cb, promise } object
                if let JsValue::Object(entry_obj) = entry {
                    let eo = entry_obj.borrow();
                    let cb = eo.get("cb");
                    let chained = eo.get("promise");
                    drop(eo);
                    if cb.is_function() {
                        enqueue_promise_reaction(
                            vm,
                            cb,
                            value.clone(),
                            chained,
                            state == "fulfilled",
                        );
                    } else if !matches!(chained, JsValue::Undefined) {
                        // Pass value through to chained promise
                        settle_promise(vm, &chained, state, value);
                    }
                } else if entry.is_function() {
                    // Legacy format: bare callback function
                    call_callback(vm, entry, &[value.clone()]);
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Promise.prototype methods
// ═══════════════════════════════════════════════════════════

pub fn promise_then(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let on_fulfilled = args.first().cloned().unwrap_or(JsValue::Undefined);
    let on_rejected = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let promise = vm.current_this.clone();

    if let JsValue::Object(obj) = &promise {
        let (state, value) = {
            let o = obj.borrow();
            (o.get("__state").to_js_string(), o.get("__value"))
        };

        // Create a new promise for chaining
        let new_promise = make_pending_promise(vm);

        if state == "fulfilled" {
            if on_fulfilled.is_function() {
                enqueue_promise_reaction(vm, on_fulfilled, value, new_promise.clone(), true);
            } else {
                settle_promise(vm, &new_promise, "fulfilled", &value);
            }
        } else if state == "rejected" {
            if on_rejected.is_function() {
                enqueue_promise_reaction(vm, on_rejected, value, new_promise.clone(), false);
            } else {
                settle_promise(vm, &new_promise, "rejected", &value);
            }
        } else {
            // Still pending — queue callbacks for later settlement
            let o = obj.borrow();
            if let JsValue::Array(arr) = o.get("__then_cbs") {
                // Store (callback, new_promise) pairs
                let mut entry = JsObject::new();
                entry.set(String::from("cb"), on_fulfilled);
                entry.set(String::from("promise"), new_promise.clone());
                arr.borrow_mut()
                    .push(JsValue::Object(Rc::new(RefCell::new(entry))));
            }
            if let JsValue::Array(arr) = o.get("__catch_cbs") {
                let mut entry = JsObject::new();
                entry.set(String::from("cb"), on_rejected);
                entry.set(String::from("promise"), new_promise.clone());
                arr.borrow_mut()
                    .push(JsValue::Object(Rc::new(RefCell::new(entry))));
            }
        }

        return new_promise;
    }

    JsValue::Undefined
}

/// Create a pending promise with all required methods.
fn make_pending_promise(vm: &Vm) -> JsValue {
    let mut obj = JsObject::new();
    attach_promise_shape(&mut obj, promise_proto(vm), "pending", JsValue::Undefined);
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// Enqueue a promise reaction as a microtask.
fn enqueue_promise_reaction(
    vm: &mut Vm,
    callback: JsValue,
    value: JsValue,
    new_promise: JsValue,
    is_fulfill: bool,
) {
    // Create a native wrapper that calls the callback and settles the chained promise
    let wrapper_fn = JsFunction {
        name: Some(String::from("__promise_reaction__")),
        params: Vec::new(),
        kind: FnKind::Native(promise_reaction_runner),
        object_proto: None,
        this_binding: None,
        bound_args: Vec::new(),
        upvalues: Vec::new(),
        with_scopes: Vec::new(),
        prototype: None,
        own_props: alloc::collections::BTreeMap::new(),
        arity: None,
        super_class: None,
    };
    let wrapper = JsValue::Function(Rc::new(RefCell::new(wrapper_fn)));
    // Pass callback, value, new_promise, is_fulfill as args to the microtask
    let is_fulfill_val = JsValue::Bool(is_fulfill);
    vm.enqueue_microtask(wrapper, vec![callback, value, new_promise, is_fulfill_val]);
}

/// Microtask runner for promise reactions.
fn promise_reaction_runner(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let new_promise = args.get(2).cloned().unwrap_or(JsValue::Undefined);
    let is_fulfill = args
        .get(3)
        .map(|v| matches!(v, JsValue::Bool(true)))
        .unwrap_or(true);

    if callback.is_function() {
        let result = call_callback(vm, &callback, &[value]);
        if let Some(exc) = vm.last_exception.take() {
            settle_promise(vm, &new_promise, "rejected", &exc);
        } else {
            settle_chained_result(vm, &new_promise, result);
        }
    } else {
        let state = if is_fulfill { "fulfilled" } else { "rejected" };
        settle_promise(vm, &new_promise, state, &value);
    }
    JsValue::Undefined
}

fn promise_finally_fulfill_runner(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let on_finally = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if on_finally.is_function() {
        let _ = call_callback(vm, &on_finally, &[]);
        if vm.last_exception.is_some() {
            return JsValue::Undefined;
        }
    }
    value
}

fn promise_finally_reject_runner(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let on_finally = args.first().cloned().unwrap_or(JsValue::Undefined);
    let reason = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if on_finally.is_function() {
        let _ = call_callback(vm, &on_finally, &[]);
        if vm.last_exception.is_some() {
            return JsValue::Undefined;
        }
    }
    make_rejected_promise(reason)
}

pub fn promise_catch(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let on_rejected = args.first().cloned().unwrap_or(JsValue::Undefined);
    // .catch(fn) is sugar for .then(undefined, fn)
    promise_then(vm, &[JsValue::Undefined, on_rejected])
}

pub fn promise_finally(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let on_finally = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promise = vm.current_this.clone();

    if let JsValue::Object(obj) = &promise {
        let (state, value, on_fulfilled, on_rejected) = {
            let o = obj.borrow();
            (
                o.get("__state").to_js_string(),
                o.get("__value"),
                make_bound_native(
                    "__promise_finally_fulfill__",
                    promise_finally_fulfill_runner,
                    vec![on_finally.clone()],
                ),
                make_bound_native(
                    "__promise_finally_reject__",
                    promise_finally_reject_runner,
                    vec![on_finally.clone()],
                ),
            )
        };
        if state == "fulfilled" {
            let result = vm.call_value(&on_fulfilled, &[value], JsValue::Undefined);
            if let Some(exc) = vm.last_exception.take() {
                return make_rejected_promise(exc);
            }
            return if is_internal_promise(&result) {
                result
            } else {
                make_resolved_promise(result)
            };
        }
        if state == "rejected" {
            let result = vm.call_value(&on_rejected, &[value], JsValue::Undefined);
            if let Some(exc) = vm.last_exception.take() {
                return make_rejected_promise(exc);
            }
            return if is_internal_promise(&result) {
                result
            } else {
                make_resolved_promise(result)
            };
        }
        return promise_then(vm, &[on_fulfilled, on_rejected]);
    }
    promise
}

// ═══════════════════════════════════════════════════════════
// Promise static methods
// ═══════════════════════════════════════════════════════════

/// Create a resolved Promise wrapping the given value. No VM needed.
pub fn make_resolved_promise(value: JsValue) -> JsValue {
    if is_internal_promise(&value) {
        return value;
    }
    let mut obj = JsObject::new();
    attach_promise_shape(&mut obj, None, "fulfilled", value);
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// Create a rejected Promise with the given reason.
pub fn make_rejected_promise(reason: JsValue) -> JsValue {
    let mut obj = JsObject::new();
    attach_promise_shape(&mut obj, None, "rejected", reason);
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

pub fn promise_resolve(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if is_internal_promise(&value) {
        return value;
    }
    if value.is_object() || value.is_function() {
        let adopted = make_pending_promise(vm);
        if adopt_thenable(vm, &adopted, &value) {
            return adopted;
        }
    }
    make_promise(vm, "fulfilled", value)
}

pub fn promise_reject(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    make_promise(vm, "rejected", value)
}

pub fn promise_all(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let iterable = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promises = match &iterable {
        JsValue::Array(arr) => arr.borrow().to_dense_vec(),
        _ => Vec::new(),
    };

    let mut results = Vec::with_capacity(promises.len());
    for p in &promises {
        if let JsValue::Object(obj) = p {
            let o = obj.borrow();
            if o.internal_tag.as_deref() == Some("__promise__") {
                let state = o.get("__state").to_js_string();
                if state == "rejected" {
                    let value = o.get("__value");
                    drop(o);
                    return promise_reject(vm, &[value]);
                }
                results.push(o.get("__value"));
                continue;
            }
        }
        results.push(p.clone());
    }
    promise_resolve(vm, &[JsValue::new_array(results)])
}

pub fn promise_all_settled(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let iterable = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promises = match &iterable {
        JsValue::Array(arr) => arr.borrow().to_dense_vec(),
        _ => Vec::new(),
    };

    let mut results = Vec::with_capacity(promises.len());
    for p in &promises {
        let entry = JsValue::new_object();
        if let JsValue::Object(obj) = p {
            let o = obj.borrow();
            if o.internal_tag.as_deref() == Some("__promise__") {
                let state = o.get("__state").to_js_string();
                entry.set_property(String::from("status"), JsValue::String(state.clone()));
                if state == "fulfilled" {
                    entry.set_property(String::from("value"), o.get("__value"));
                } else {
                    entry.set_property(String::from("reason"), o.get("__value"));
                }
                results.push(entry);
                continue;
            }
        }
        entry.set_property(
            String::from("status"),
            JsValue::String(String::from("fulfilled")),
        );
        entry.set_property(String::from("value"), p.clone());
        results.push(entry);
    }
    promise_resolve(vm, &[JsValue::new_array(results)])
}

pub fn promise_race(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let iterable = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promises = match &iterable {
        JsValue::Array(arr) => arr.borrow().to_dense_vec(),
        _ => Vec::new(),
    };
    // Return the first settled promise
    for p in &promises {
        if let JsValue::Object(obj) = p {
            let o = obj.borrow();
            if o.internal_tag.as_deref() == Some("__promise__") {
                let state = o.get("__state").to_js_string();
                if state == "fulfilled" {
                    let val = o.get("__value");
                    drop(o);
                    return promise_resolve(vm, &[val]);
                } else if state == "rejected" {
                    let val = o.get("__value");
                    drop(o);
                    return promise_reject(vm, &[val]);
                }
            }
        }
    }
    // All pending — return a pending promise
    promise_resolve(vm, &[JsValue::Undefined])
}

/// `Promise.any(iterable)` — resolves with the first fulfilled value.
///
/// If all promises are rejected, rejects with an `AggregateError` containing
/// all rejection reasons (ES2021 §27.2.4.1).
pub fn promise_any(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let iterable = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promises = match &iterable {
        JsValue::Array(arr) => arr.borrow().to_dense_vec(),
        _ => Vec::new(),
    };
    let mut errors: Vec<JsValue> = Vec::new();
    for p in &promises {
        if let JsValue::Object(obj) = p {
            let o = obj.borrow();
            if o.internal_tag.as_deref() == Some("__promise__") {
                let state = o.get("__state").to_js_string();
                if state == "fulfilled" {
                    let val = o.get("__value");
                    drop(o);
                    return promise_resolve(vm, &[val]);
                } else if state == "rejected" {
                    let val = o.get("__value");
                    errors.push(val);
                }
                continue;
            }
        }
        // Non-promise values: treat as fulfilled.
        return promise_resolve(vm, &[p.clone()]);
    }
    // All rejected — create AggregateError.
    let err = JsValue::new_object();
    err.set_property(
        alloc::string::String::from("name"),
        JsValue::String(alloc::string::String::from("AggregateError")),
    );
    err.set_property(
        alloc::string::String::from("message"),
        JsValue::String(alloc::string::String::from("All promises were rejected")),
    );
    err.set_property(
        alloc::string::String::from("errors"),
        JsValue::new_array(errors),
    );
    promise_reject(vm, &[err])
}

// ═══════════════════════════════════════════════════════════
// Helper
// ═══════════════════════════════════════════════════════════

fn call_callback(vm: &mut Vm, callback: &JsValue, args: &[JsValue]) -> JsValue {
    if callback.is_function() {
        vm.call_value(callback, args, JsValue::Undefined)
    } else {
        JsValue::Undefined
    }
}
