//! Native HTTP request bridge.
//!
//! `__http_request(method, url, headersJson, body)` → `{status, statusText, body}`
//!
//! If the host registered `__http_handler` as a native global, it is
//! called synchronously.  Otherwise the request is queued as pending.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::{JsObject, FnKind};

use super::{get_bridge, arg_string, PendingHttpRequest};

static mut NEXT_HTTP_REQ_ID: u64 = 1;

pub fn http_request(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let method = arg_string(args, 0);
    let url = arg_string(args, 1);
    let _headers_json = arg_string(args, 2);
    let body = arg_string(args, 3);

    // Check if host provided a synchronous handler.
    let handler = vm.get_global("__http_handler");
    if let JsValue::Function(f) = handler {
        let kind = f.borrow().kind.clone();
        if let FnKind::Native(native) = kind {
            return native(vm, args);
        }
    }

    // Record as pending request.
    if let Some(bridge) = get_bridge(vm) {
        let id = unsafe {
            let id = NEXT_HTTP_REQ_ID;
            NEXT_HTTP_REQ_ID += 1;
            id
        };
        bridge.pending_http_requests.push(PendingHttpRequest {
            id,
            method,
            url,
            headers: Vec::new(),
            body: if body.is_empty() { None } else { Some(body) },
        });
    }

    // Return empty response.
    let mut obj = JsObject::new();
    obj.set(String::from("status"), JsValue::Number(0.0));
    obj.set(String::from("statusText"), JsValue::String(String::new()));
    obj.set(String::from("body"), JsValue::String(String::new()));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}
