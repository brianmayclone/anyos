//! Error constructor and Error.prototype.toString.

use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use super::Vm;
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// Error constructor
// ═══════════════════════════════════════════════════════════

/// `new Error(message)` or `Error(message)` — creates an error object.
///
/// When called as `super(msg)` from a derived class constructor, `vm.current_this`
/// is already the derived instance; we set `message`/`name` on it and return it.
/// For a plain `new Error(msg)`, we set up the pre-created `new_obj` (which is
/// `vm.current_this`) and return it so `new_object` uses it.
pub fn ctor_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let message = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    // ES2022: second argument can be { cause: value }
    let cause = args.get(1).and_then(|opts| {
        if let JsValue::Object(obj) = opts {
            let c = obj.borrow().get("cause");
            if c.is_undefined() {
                None
            } else {
                Some(c)
            }
        } else {
            None
        }
    });

    // Build a minimal stack trace string (V8-style).
    let stack_str = {
        let mut s = alloc::format!("Error: {}", message);
        for frame in vm.frames.iter().rev().take(8) {
            let fname = frame.chunk.name.as_deref().unwrap_or("<anonymous>");
            s.push_str("\n    at ");
            s.push_str(fname);
        }
        s
    };

    if let JsValue::Object(obj_rc) = &vm.current_this.clone() {
        let mut o = obj_rc.borrow_mut();
        o.set(String::from("message"), JsValue::String(message));
        o.set(String::from("name"), JsValue::String(String::from("Error")));
        o.set(String::from("stack"), JsValue::String(stack_str));
        if let Some(cause_val) = cause {
            o.set(String::from("cause"), cause_val);
        }
        if o.prototype.is_none() {
            o.prototype = Some(vm.error_proto.clone());
        }
        drop(o);
        return vm.current_this.clone();
    }
    let mut obj = JsObject::new();
    obj.prototype = Some(vm.error_proto.clone());
    obj.set(String::from("message"), JsValue::String(message));
    obj.set(String::from("name"), JsValue::String(String::from("Error")));
    obj.set(String::from("stack"), JsValue::String(stack_str));
    if let Some(cause_val) = cause {
        obj.set(String::from("cause"), cause_val);
    }
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

// ═══════════════════════════════════════════════════════════
// Error.prototype.toString
// ═══════════════════════════════════════════════════════════

// Helper: create a typed error
fn ctor_error_with_name(vm: &mut Vm, args: &[JsValue], type_name: &str) -> JsValue {
    let message = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let cause = args.get(1).and_then(|opts| {
        if let JsValue::Object(obj) = opts {
            let c = obj.borrow().get("cause");
            if c.is_undefined() {
                None
            } else {
                Some(c)
            }
        } else {
            None
        }
    });

    let stack_str = {
        let mut s = alloc::format!("{}: {}", type_name, message);
        for frame in vm.frames.iter().rev().take(8) {
            let fname = frame.chunk.name.as_deref().unwrap_or("<anonymous>");
            s.push_str("\n    at ");
            s.push_str(fname);
        }
        s
    };

    if let JsValue::Object(obj_rc) = &vm.current_this.clone() {
        let mut o = obj_rc.borrow_mut();
        o.set(String::from("message"), JsValue::String(message));
        o.set(
            String::from("name"),
            JsValue::String(String::from(type_name)),
        );
        o.set(String::from("stack"), JsValue::String(stack_str));
        if let Some(cause_val) = cause {
            o.set(String::from("cause"), cause_val);
        }
        if o.prototype.is_none() {
            o.prototype = Some(vm.error_proto.clone());
        }
        let ctor = vm.globals.get(type_name);
        if !matches!(ctor, JsValue::Undefined) {
            o.set(String::from("constructor"), ctor);
        }
        drop(o);
        return vm.current_this.clone();
    }
    let mut obj = JsObject::new();
    obj.prototype = Some(vm.error_proto.clone());
    obj.set(String::from("message"), JsValue::String(message));
    obj.set(
        String::from("name"),
        JsValue::String(String::from(type_name)),
    );
    obj.set(String::from("stack"), JsValue::String(stack_str));
    if let Some(cause_val) = cause {
        obj.set(String::from("cause"), cause_val);
    }
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

pub fn ctor_type_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ctor_error_with_name(vm, args, "TypeError")
}
pub fn ctor_range_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ctor_error_with_name(vm, args, "RangeError")
}
pub fn ctor_reference_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ctor_error_with_name(vm, args, "ReferenceError")
}
pub fn ctor_syntax_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ctor_error_with_name(vm, args, "SyntaxError")
}
pub fn ctor_uri_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ctor_error_with_name(vm, args, "URIError")
}
pub fn ctor_eval_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ctor_error_with_name(vm, args, "EvalError")
}

/// `new AggregateError(errors, message)` — creates an error with an `errors` array.
pub fn ctor_aggregate_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let errors = args.first().cloned().unwrap_or(JsValue::Undefined);
    let message = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();

    let errors_arr = match &errors {
        JsValue::Array(_) => errors.clone(),
        _ => JsValue::new_array(alloc::vec::Vec::new()),
    };

    if let JsValue::Object(obj_rc) = &vm.current_this.clone() {
        let mut o = obj_rc.borrow_mut();
        o.set(String::from("message"), JsValue::String(message));
        o.set(
            String::from("name"),
            JsValue::String(String::from("AggregateError")),
        );
        o.set(String::from("errors"), errors_arr);
        if o.prototype.is_none() {
            o.prototype = Some(vm.error_proto.clone());
        }
        drop(o);
        return vm.current_this.clone();
    }
    let mut obj = JsObject::new();
    obj.prototype = Some(vm.error_proto.clone());
    obj.set(String::from("message"), JsValue::String(message));
    obj.set(
        String::from("name"),
        JsValue::String(String::from("AggregateError")),
    );
    obj.set(String::from("errors"), errors_arr);
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

pub fn error_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match &vm.current_this {
        JsValue::Object(obj) => {
            let o = obj.borrow();
            let name = match o.properties.get("name") {
                Some(p) => p.value.to_js_string(),
                None => String::from("Error"),
            };
            let message = match o.properties.get("message") {
                Some(p) => p.value.to_js_string(),
                None => String::new(),
            };
            if message.is_empty() {
                JsValue::String(name)
            } else {
                let mut s = name;
                s.push_str(": ");
                s.push_str(&message);
                JsValue::String(s)
            }
        }
        _ => JsValue::String(String::from("Error")),
    }
}
