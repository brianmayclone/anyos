//! Error constructor and Error.prototype.toString.

use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use crate::value::*;
use super::Vm;

// ═══════════════════════════════════════════════════════════
// Error constructor
// ═══════════════════════════════════════════════════════════

/// `new Error(message)` or `Error(message)` — creates an error object.
pub fn ctor_error(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let message = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let mut obj = JsObject::new();
    obj.prototype = Some(vm.error_proto.clone());
    obj.set(String::from("message"), JsValue::String(message));
    obj.set(String::from("name"), JsValue::String(String::from("Error")));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

// ═══════════════════════════════════════════════════════════
// Error.prototype.toString
// ═══════════════════════════════════════════════════════════

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
