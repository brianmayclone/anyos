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

use crate::value::*;
use super::{Vm, native_fn, CallFrame};

// ═══════════════════════════════════════════════════════════
// Promise constructor
// ═══════════════════════════════════════════════════════════

/// `new Promise(executor)` — creates a Promise and runs executor synchronously.
pub fn ctor_promise(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let executor = args.first().cloned().unwrap_or(JsValue::Undefined);

    let mut obj = JsObject::new();
    obj.internal_tag = Some(String::from("__promise__"));
    obj.set(String::from("__state"), JsValue::String(String::from("pending")));
    obj.set(String::from("__value"), JsValue::Undefined);
    obj.set(String::from("__then_cbs"), JsValue::new_array(Vec::new()));
    obj.set(String::from("__catch_cbs"), JsValue::new_array(Vec::new()));

    // Install .then, .catch, .finally methods
    obj.set(String::from("then"), native_fn("then", promise_then));
    obj.set(String::from("catch"), native_fn("catch", promise_catch));
    obj.set(String::from("finally"), native_fn("finally", promise_finally));

    let promise = JsValue::Object(Rc::new(RefCell::new(obj)));

    // Execute the executor(resolve, reject) synchronously
    if let JsValue::Function(func_rc) = &executor {
        let kind = func_rc.borrow().kind.clone();
        let promise_clone = promise.clone();

        // We can only pass native resolvers.  Create temporary globals.
        // Store promise reference for resolve/reject to find.
        vm.set_global("__promise_pending", promise_clone);

        vm.register_native("__promise_resolve_fn", promise_resolve_native);
        vm.register_native("__promise_reject_fn", promise_reject_native);

        let resolve_fn = vm.get_global("__promise_resolve_fn");
        let reject_fn = vm.get_global("__promise_reject_fn");

        match kind {
            FnKind::Native(f) => {
                f(vm, &[resolve_fn, reject_fn]);
            }
            FnKind::Bytecode(chunk) => {
                let local_count = chunk.local_count as usize;
                let mut locals = vec![JsValue::Undefined; local_count];
                if local_count > 0 { locals[0] = resolve_fn; }
                if local_count > 1 { locals[1] = reject_fn; }
                let frame = CallFrame {
                    chunk,
                    ip: 0,
                    stack_base: vm.stack.len(),
                    locals,
                    this_val: JsValue::Undefined,
                };
                vm.frames.push(frame);
                vm.run();
            }
        }
    }

    promise
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
            if current_state != "pending" { return; }
            o.set(String::from("__state"), JsValue::String(String::from(state)));
            o.set(String::from("__value"), value.clone());
        }

        // Run appropriate callbacks
        let cb_key = if state == "fulfilled" { "__then_cbs" } else { "__catch_cbs" };
        let cbs = {
            let o = obj.borrow();
            o.get(cb_key)
        };
        if let JsValue::Array(arr) = cbs {
            let callbacks = arr.borrow().elements.clone();
            for cb in &callbacks {
                call_callback(vm, cb, &[value.clone()]);
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
        let mut new_obj = JsObject::new();
        new_obj.internal_tag = Some(String::from("__promise__"));
        new_obj.set(String::from("__state"), JsValue::String(String::from("pending")));
        new_obj.set(String::from("__value"), JsValue::Undefined);
        new_obj.set(String::from("__then_cbs"), JsValue::new_array(Vec::new()));
        new_obj.set(String::from("__catch_cbs"), JsValue::new_array(Vec::new()));
        new_obj.set(String::from("then"), native_fn("then", promise_then));
        new_obj.set(String::from("catch"), native_fn("catch", promise_catch));
        new_obj.set(String::from("finally"), native_fn("finally", promise_finally));
        let new_promise = JsValue::Object(Rc::new(RefCell::new(new_obj)));

        if state == "fulfilled" {
            if on_fulfilled.is_function() {
                let result = call_callback(vm, &on_fulfilled, &[value]);
                settle_promise(vm, &new_promise, "fulfilled", &result);
            } else {
                settle_promise(vm, &new_promise, "fulfilled", &value);
            }
        } else if state == "rejected" {
            if on_rejected.is_function() {
                let result = call_callback(vm, &on_rejected, &[value]);
                settle_promise(vm, &new_promise, "fulfilled", &result);
            } else {
                settle_promise(vm, &new_promise, "rejected", &value);
            }
        } else {
            // Still pending — queue callbacks
            if on_fulfilled.is_function() {
                let o = obj.borrow();
                if let JsValue::Array(arr) = o.get("__then_cbs") {
                    arr.borrow_mut().elements.push(on_fulfilled);
                }
            }
            if on_rejected.is_function() {
                let o = obj.borrow();
                if let JsValue::Array(arr) = o.get("__catch_cbs") {
                    arr.borrow_mut().elements.push(on_rejected);
                }
            }
        }

        return new_promise;
    }

    JsValue::Undefined
}

pub fn promise_catch(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let on_rejected = args.first().cloned().unwrap_or(JsValue::Undefined);
    // .catch(fn) is sugar for .then(undefined, fn)
    let saved_this = vm.current_this.clone();
    vm.current_this = saved_this;
    promise_then(vm, &[JsValue::Undefined, on_rejected])
}

pub fn promise_finally(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let on_finally = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promise = vm.current_this.clone();

    if let JsValue::Object(obj) = &promise {
        let state = obj.borrow().get("__state").to_js_string();
        if state != "pending" && on_finally.is_function() {
            call_callback(vm, &on_finally, &[]);
        }
    }
    promise
}

// ═══════════════════════════════════════════════════════════
// Promise static methods
// ═══════════════════════════════════════════════════════════

pub fn promise_resolve(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    // If already a promise, return it
    if let JsValue::Object(obj) = &value {
        if obj.borrow().internal_tag.as_deref() == Some("__promise__") {
            return value;
        }
    }
    let mut obj = JsObject::new();
    obj.internal_tag = Some(String::from("__promise__"));
    obj.set(String::from("__state"), JsValue::String(String::from("fulfilled")));
    obj.set(String::from("__value"), value);
    obj.set(String::from("__then_cbs"), JsValue::new_array(Vec::new()));
    obj.set(String::from("__catch_cbs"), JsValue::new_array(Vec::new()));
    obj.set(String::from("then"), native_fn("then", promise_then));
    obj.set(String::from("catch"), native_fn("catch", promise_catch));
    obj.set(String::from("finally"), native_fn("finally", promise_finally));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

pub fn promise_reject(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let mut obj = JsObject::new();
    obj.internal_tag = Some(String::from("__promise__"));
    obj.set(String::from("__state"), JsValue::String(String::from("rejected")));
    obj.set(String::from("__value"), value);
    obj.set(String::from("__then_cbs"), JsValue::new_array(Vec::new()));
    obj.set(String::from("__catch_cbs"), JsValue::new_array(Vec::new()));
    obj.set(String::from("then"), native_fn("then", promise_then));
    obj.set(String::from("catch"), native_fn("catch", promise_catch));
    obj.set(String::from("finally"), native_fn("finally", promise_finally));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

pub fn promise_all(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let iterable = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promises = match &iterable {
        JsValue::Array(arr) => arr.borrow().elements.clone(),
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
        JsValue::Array(arr) => arr.borrow().elements.clone(),
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
        entry.set_property(String::from("status"), JsValue::String(String::from("fulfilled")));
        entry.set_property(String::from("value"), p.clone());
        results.push(entry);
    }
    promise_resolve(vm, &[JsValue::new_array(results)])
}

pub fn promise_race(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let iterable = args.first().cloned().unwrap_or(JsValue::Undefined);
    let promises = match &iterable {
        JsValue::Array(arr) => arr.borrow().elements.clone(),
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

// ═══════════════════════════════════════════════════════════
// Helper
// ═══════════════════════════════════════════════════════════

fn call_callback(vm: &mut Vm, callback: &JsValue, args: &[JsValue]) -> JsValue {
    match callback {
        JsValue::Function(func_rc) => {
            let kind = func_rc.borrow().kind.clone();
            match kind {
                FnKind::Native(f) => f(vm, args),
                FnKind::Bytecode(chunk) => {
                    let local_count = chunk.local_count as usize;
                    let mut locals = vec![JsValue::Undefined; local_count];
                    for (i, arg) in args.iter().enumerate() {
                        if i < local_count { locals[i] = arg.clone(); }
                    }
                    let frame = CallFrame {
                        chunk,
                        ip: 0,
                        stack_base: vm.stack.len(),
                        locals,
                        this_val: JsValue::Undefined,
                    };
                    vm.frames.push(frame);
                    vm.run()
                }
            }
        }
        _ => JsValue::Undefined,
    }
}
