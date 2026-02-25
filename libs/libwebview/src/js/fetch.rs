//! Native fetch() API + Headers constructor.

use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::JsObject;
use libjs::vm::native_fn;

use super::arg_string;
use super::http;

/// `fetch(url, options)` — performs HTTP request, returns a Promise-like result.
///
/// Since our Promise implementation is synchronous, the request is made
/// immediately via `__http_request` and wrapped in Promise.resolve/reject.
pub fn native_fetch(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let url = arg_string(args, 0);
    let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);

    let method = if let JsValue::Object(_) = &options {
        let m = options.get_property("method").to_js_string();
        if m.is_empty() || m == "undefined" { String::from("GET") } else { m }
    } else {
        String::from("GET")
    };

    let body = if let JsValue::Object(_) = &options {
        let b = options.get_property("body").to_js_string();
        if b == "undefined" { String::new() } else { b }
    } else {
        String::new()
    };

    let headers_str = String::from("{}");

    // Perform the request.
    let result = http::http_request(vm, &[
        JsValue::String(method),
        JsValue::String(url.clone()),
        JsValue::String(headers_str),
        JsValue::String(body),
    ]);

    let status = result.get_property("status").to_number();
    let status_text = result.get_property("statusText").to_js_string();
    let resp_body = result.get_property("body").to_js_string();

    if status > 0.0 {
        // Build Response object.
        let response = make_response(status, &status_text, &url, &resp_body);

        // Wrap in Promise.resolve(response).
        // Call the global Promise.resolve if available.
        let promise_ctor = vm.get_global("Promise");
        if let JsValue::Function(_) = &promise_ctor {
            let resolve_fn = promise_ctor.get_property("resolve");
            if let JsValue::Function(f) = resolve_fn {
                let kind = f.borrow().kind.clone();
                if let libjs::value::FnKind::Native(native) = kind {
                    return native(vm, &[response]);
                }
            }
        }
        // Fallback: return response directly.
        response
    } else {
        // Wrap in Promise.reject(error).
        let promise_ctor = vm.get_global("Promise");
        if let JsValue::Function(_) = &promise_ctor {
            let reject_fn = promise_ctor.get_property("reject");
            if let JsValue::Function(f) = reject_fn {
                let kind = f.borrow().kind.clone();
                if let libjs::value::FnKind::Native(native) = kind {
                    let err = JsValue::String(String::from("Network request failed"));
                    return native(vm, &[err]);
                }
            }
        }
        JsValue::Undefined
    }
}

fn make_response(status: f64, status_text: &str, url: &str, body: &str) -> JsValue {
    let mut obj = JsObject::new();
    obj.set(String::from("ok"), JsValue::Bool(status >= 200.0 && status < 300.0));
    obj.set(String::from("status"), JsValue::Number(status));
    obj.set(String::from("statusText"), JsValue::String(String::from(status_text)));
    obj.set(String::from("url"), JsValue::String(String::from(url)));
    obj.set(String::from("redirected"), JsValue::Bool(false));
    obj.set(String::from("type"), JsValue::String(String::from("basic")));
    obj.set(String::from("bodyUsed"), JsValue::Bool(false));
    obj.set(String::from("__body"), JsValue::String(String::from(body)));

    // Headers sub-object.
    let headers = JsValue::new_object();
    headers.set_property(String::from("get"), native_fn("get", |_,_| JsValue::Null));
    headers.set_property(String::from("has"), native_fn("has", |_,_| JsValue::Bool(false)));
    headers.set_property(String::from("forEach"), native_fn("forEach", |_,_| JsValue::Undefined));
    obj.set(String::from("headers"), headers);

    // Body methods — return the stored __body wrapped in Promise.resolve.
    obj.set(String::from("text"), native_fn("text", resp_text));
    obj.set(String::from("json"), native_fn("json", resp_json));
    obj.set(String::from("blob"), native_fn("blob", resp_text));
    obj.set(String::from("arrayBuffer"), native_fn("arrayBuffer", resp_text));
    obj.set(String::from("clone"), native_fn("clone", resp_clone));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn resp_text(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let body = if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow().get("__body")
    } else {
        JsValue::String(String::new())
    };
    // Wrap in Promise.resolve.
    wrap_promise_resolve(vm, body)
}

fn resp_json(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let body_str = if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow().get("__body").to_js_string()
    } else {
        String::new()
    };
    // Parse JSON using the global JSON.parse.
    let json_obj = vm.get_global("JSON");
    let parse_fn = json_obj.get_property("parse");
    let parsed = if let JsValue::Function(f) = parse_fn {
        let kind = f.borrow().kind.clone();
        if let libjs::value::FnKind::Native(native) = kind {
            native(vm, &[JsValue::String(body_str)])
        } else {
            JsValue::Undefined
        }
    } else {
        JsValue::Undefined
    };
    wrap_promise_resolve(vm, parsed)
}

fn resp_clone(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

fn wrap_promise_resolve(vm: &mut Vm, val: JsValue) -> JsValue {
    let promise_ctor = vm.get_global("Promise");
    if let JsValue::Function(_) = &promise_ctor {
        let resolve_fn = promise_ctor.get_property("resolve");
        if let JsValue::Function(f) = resolve_fn {
            let kind = f.borrow().kind.clone();
            if let libjs::value::FnKind::Native(native) = kind {
                return native(vm, &[val]);
            }
        }
    }
    val
}

/// `new Headers(init)` constructor.
pub fn native_headers_ctor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut obj = JsObject::new();
    obj.set(String::from("_headers"), JsValue::new_object());

    // Copy init if provided.
    if let Some(JsValue::Object(init)) = args.first() {
        let init_obj = init.borrow();
        for (k, prop) in &init_obj.properties {
            let lower = k.to_ascii_lowercase();
            if let JsValue::Object(h) = &JsValue::Object(Rc::new(RefCell::new(JsObject::new()))) {
                // This is simplified — just store the headers.
                let _ = h; // placeholder
            }
            obj.set(String::from("_headers"), JsValue::new_object());
            // Re-set with the lowercased key.
            if let Some(p) = obj.properties.get("_headers") {
                p.value.set_property(lower, prop.value.clone());
            }
        }
    }

    obj.set(String::from("get"), native_fn("get", headers_get));
    obj.set(String::from("set"), native_fn("set", headers_set));
    obj.set(String::from("has"), native_fn("has", headers_has));
    obj.set(String::from("append"), native_fn("append", headers_set));
    obj.set(String::from("delete"), native_fn("delete", headers_delete));
    obj.set(String::from("forEach"), native_fn("forEach", |_,_| JsValue::Undefined));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn get_headers_data(vm: &Vm) -> JsValue {
    if let JsValue::Object(obj) = &vm.current_this {
        return obj.borrow().get("_headers");
    }
    JsValue::new_object()
}

fn headers_get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0).to_ascii_lowercase();
    let data = get_headers_data(vm);
    let val = data.get_property(&name);
    if matches!(val, JsValue::Undefined) { JsValue::Null } else { val }
}

fn headers_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0).to_ascii_lowercase();
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let data = get_headers_data(vm);
    data.set_property(name, value);
    JsValue::Undefined
}

fn headers_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0).to_ascii_lowercase();
    let data = get_headers_data(vm);
    let val = data.get_property(&name);
    JsValue::Bool(!matches!(val, JsValue::Undefined))
}

fn headers_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0).to_ascii_lowercase();
    let data = get_headers_data(vm);
    if let JsValue::Object(obj) = &data {
        obj.borrow_mut().properties.remove(&name);
    }
    JsValue::Undefined
}
