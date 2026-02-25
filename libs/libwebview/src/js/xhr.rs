//! Native XMLHttpRequest host object.
//!
//! Implements the full XHR lifecycle as native methods.
//! send() calls `__http_request` synchronously, then fires callbacks.

use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::{JsObject, FnKind};
use libjs::vm::native_fn;

use super::arg_string;
use super::http;

/// Create the XMLHttpRequest constructor function.
pub fn make_xhr_constructor() -> JsValue {
    native_fn("XMLHttpRequest", xhr_ctor)
}

fn xhr_ctor(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let mut obj = JsObject::new();

    // State.
    obj.set(String::from("readyState"), JsValue::Number(0.0));
    obj.set(String::from("status"), JsValue::Number(0.0));
    obj.set(String::from("statusText"), JsValue::String(String::new()));
    obj.set(String::from("responseText"), JsValue::String(String::new()));
    obj.set(String::from("responseXML"), JsValue::Null);
    obj.set(String::from("responseType"), JsValue::String(String::new()));
    obj.set(String::from("response"), JsValue::String(String::new()));
    obj.set(String::from("responseURL"), JsValue::String(String::new()));
    obj.set(String::from("timeout"), JsValue::Number(0.0));
    obj.set(String::from("withCredentials"), JsValue::Bool(false));
    obj.set(String::from("upload"), JsValue::new_object());

    // Internal state.
    obj.set(String::from("_method"), JsValue::String(String::from("GET")));
    obj.set(String::from("_url"), JsValue::String(String::new()));
    obj.set(String::from("_async"), JsValue::Bool(true));
    obj.set(String::from("_headers"), JsValue::new_object());
    obj.set(String::from("_sent"), JsValue::Bool(false));

    // Callbacks.
    obj.set(String::from("onreadystatechange"), JsValue::Null);
    obj.set(String::from("onload"), JsValue::Null);
    obj.set(String::from("onerror"), JsValue::Null);
    obj.set(String::from("onabort"), JsValue::Null);
    obj.set(String::from("onprogress"), JsValue::Null);
    obj.set(String::from("ontimeout"), JsValue::Null);
    obj.set(String::from("onloadstart"), JsValue::Null);
    obj.set(String::from("onloadend"), JsValue::Null);

    // Constants.
    obj.set(String::from("UNSENT"), JsValue::Number(0.0));
    obj.set(String::from("OPENED"), JsValue::Number(1.0));
    obj.set(String::from("HEADERS_RECEIVED"), JsValue::Number(2.0));
    obj.set(String::from("LOADING"), JsValue::Number(3.0));
    obj.set(String::from("DONE"), JsValue::Number(4.0));

    // Methods.
    obj.set(String::from("open"), native_fn("open", xhr_open));
    obj.set(String::from("setRequestHeader"), native_fn("setRequestHeader", xhr_set_request_header));
    obj.set(String::from("send"), native_fn("send", xhr_send));
    obj.set(String::from("abort"), native_fn("abort", xhr_abort));
    obj.set(String::from("getResponseHeader"), native_fn("getResponseHeader", xhr_get_response_header));
    obj.set(String::from("getAllResponseHeaders"), native_fn("getAllResponseHeaders", xhr_get_all_response_headers));
    obj.set(String::from("overrideMimeType"), native_fn("overrideMimeType", xhr_noop));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

// ═══════════════════════════════════════════════════════════
// XHR methods
// ═══════════════════════════════════════════════════════════

fn xhr_open(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let method = arg_string(args, 0);
    let url = arg_string(args, 1);
    let is_async = args.get(2).map(|v| v.to_boolean()).unwrap_or(true);

    set_this_prop(vm, "_method", JsValue::String(if method.is_empty() { String::from("GET") } else { method }));
    set_this_prop(vm, "_url", JsValue::String(url));
    set_this_prop(vm, "_async", JsValue::Bool(is_async));
    set_this_prop(vm, "_headers", JsValue::new_object());
    set_this_prop(vm, "_sent", JsValue::Bool(false));
    set_this_prop(vm, "readyState", JsValue::Number(1.0));
    set_this_prop(vm, "status", JsValue::Number(0.0));
    set_this_prop(vm, "statusText", JsValue::String(String::new()));
    set_this_prop(vm, "responseText", JsValue::String(String::new()));
    set_this_prop(vm, "response", JsValue::String(String::new()));

    fire_callback(vm, "onreadystatechange");
    JsValue::Undefined
}

fn xhr_set_request_header(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0);
    let value = arg_string(args, 1);
    let headers = get_this_prop(vm, "_headers");
    headers.set_property(name, JsValue::String(value));
    JsValue::Undefined
}

fn xhr_send(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Check if already sent.
    let sent = get_this_prop(vm, "_sent");
    if sent.to_boolean() { return JsValue::Undefined; }
    set_this_prop(vm, "_sent", JsValue::Bool(true));

    let method = get_this_prop(vm, "_method").to_js_string();
    let url = get_this_prop(vm, "_url").to_js_string();
    let body = arg_string(args, 0);

    // Serialize headers (simplified).
    let headers_str = String::from("{}");

    // readyState = 2 (HEADERS_RECEIVED).
    set_this_prop(vm, "readyState", JsValue::Number(2.0));
    fire_callback(vm, "onreadystatechange");
    fire_callback(vm, "onloadstart");

    // Perform the HTTP request via the bridge.
    let result = http::http_request(vm, &[
        JsValue::String(method),
        JsValue::String(url),
        JsValue::String(headers_str),
        JsValue::String(body),
    ]);

    // readyState = 3 (LOADING).
    set_this_prop(vm, "readyState", JsValue::Number(3.0));
    fire_callback(vm, "onreadystatechange");
    fire_callback(vm, "onprogress");

    // Process result.
    let status = result.get_property("status").to_number();
    let status_text = result.get_property("statusText").to_js_string();
    let resp_body = result.get_property("body").to_js_string();

    set_this_prop(vm, "status", JsValue::Number(status));
    set_this_prop(vm, "statusText", JsValue::String(status_text));
    set_this_prop(vm, "responseText", JsValue::String(resp_body.clone()));
    set_this_prop(vm, "response", JsValue::String(resp_body));

    // readyState = 4 (DONE).
    set_this_prop(vm, "readyState", JsValue::Number(4.0));
    fire_callback(vm, "onreadystatechange");

    if status >= 200.0 && status < 400.0 {
        fire_callback(vm, "onload");
    } else {
        fire_callback(vm, "onerror");
    }
    fire_callback(vm, "onloadend");

    JsValue::Undefined
}

fn xhr_abort(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    set_this_prop(vm, "readyState", JsValue::Number(0.0));
    set_this_prop(vm, "_sent", JsValue::Bool(false));
    fire_callback(vm, "onabort");
    JsValue::Undefined
}

fn xhr_get_response_header(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Null
}

fn xhr_get_all_response_headers(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn xhr_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue { JsValue::Undefined }

// ═══════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════

fn get_this_prop(vm: &Vm, name: &str) -> JsValue {
    if let JsValue::Object(obj) = &vm.current_this {
        return obj.borrow().get(name);
    }
    JsValue::Undefined
}

fn set_this_prop(vm: &Vm, name: &str, val: JsValue) {
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().set(String::from(name), val);
    }
}

/// Fire a callback stored as a property on `this`.
fn fire_callback(vm: &mut Vm, name: &str) {
    let cb = get_this_prop(vm, name);
    if let JsValue::Function(f) = cb {
        let kind = f.borrow().kind.clone();
        if let FnKind::Native(native) = kind {
            native(vm, &[]);
        }
        // For bytecode functions we'd need to call through the VM.
        // Simplified: only native callbacks work for now.
    }
}
