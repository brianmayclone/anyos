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

use libjs::value::{FnKind, JsObject};
use libjs::JsValue;
use libjs::Vm;

use super::{arg_string, get_bridge, PendingHttpRequest};

static mut NEXT_HTTP_REQ_ID: u64 = 1;

fn has_url_scheme(url: &str) -> bool {
    let Some(colon_pos) = url.find(':') else {
        return false;
    };
    if colon_pos == 0 {
        return false;
    }
    url[..colon_pos]
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
}

fn split_base_url(url: &str) -> (String, String) {
    let path_start = url.find("://").map(|pos| {
        let after_scheme = pos + 3;
        url[after_scheme..]
            .find('/')
            .map(|inner| after_scheme + inner)
            .unwrap_or(url.len())
    });
    let path_start = path_start.unwrap_or_else(|| url.find('/').unwrap_or(url.len()));
    let origin = String::from(&url[..path_start]);
    let path = if path_start < url.len() {
        &url[path_start..]
    } else {
        "/"
    };
    let path_end = path.find(['?', '#']).unwrap_or(path.len());
    let path_only = &path[..path_end];
    let base_dir = if let Some(last_slash) = path_only.rfind('/') {
        &path_only[..last_slash + 1]
    } else {
        "/"
    };
    (origin, String::from(base_dir))
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let mut normalized = String::from("/");
    normalized.push_str(&parts.join("/"));
    if path.ends_with('/') && !normalized.ends_with('/') {
        normalized.push('/');
    }
    normalized
}

fn resolve_request_url(vm: &mut Vm, url: &str) -> String {
    if url.is_empty() || has_url_scheme(url) {
        return String::from(url);
    }

    let location = vm.get_global("location");
    let base_href = location.get_property("href").to_js_string();
    if base_href.is_empty() || base_href == "undefined" {
        return String::from(url);
    }

    let (origin, base_dir) = split_base_url(&base_href);
    if url.starts_with("//") {
        let protocol = location.get_property("protocol").to_js_string();
        return alloc::format!("{}{}", protocol, url);
    }
    if url.starts_with('/') {
        return alloc::format!("{}{}", origin, url);
    }
    if url.starts_with('?') {
        let base_without_hash = base_href.split('#').next().unwrap_or(&base_href);
        let base_without_query = base_without_hash
            .split('?')
            .next()
            .unwrap_or(base_without_hash);
        return alloc::format!("{}{}", base_without_query, url);
    }
    if url.starts_with('#') {
        let base_without_hash = base_href.split('#').next().unwrap_or(&base_href);
        return alloc::format!("{}{}", base_without_hash, url);
    }

    let joined_path = normalize_path(&alloc::format!("{}{}", base_dir, url));
    alloc::format!("{}{}", origin, joined_path)
}

pub fn http_request(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let method = arg_string(args, 0);
    let url = resolve_request_url(vm, &arg_string(args, 1));
    let headers_json = arg_string(args, 2);
    let body = arg_string(args, 3);
    let resolved_args = [
        JsValue::String(method.clone()),
        JsValue::String(url.clone()),
        JsValue::String(headers_json),
        JsValue::String(body.clone()),
    ];

    // Check if host provided a synchronous handler.
    let handler = vm.get_global("__http_handler");
    if let JsValue::Function(f) = handler {
        let kind = f.borrow().kind.clone();
        if let FnKind::Native(native) = kind {
            return native(vm, &resolved_args);
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
