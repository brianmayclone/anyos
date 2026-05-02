use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_ctor_fn, native_fn, native_promise, Vm};

use super::util::object;

const HEADERS_TAG: &str = "__node_web_headers__";
const REQUEST_TAG: &str = "__node_web_request__";
const RESPONSE_TAG: &str = "__node_web_response__";
const ABORT_SIGNAL_TAG: &str = "__node_web_abort_signal__";
const ABORT_CONTROLLER_TAG: &str = "__node_web_abort_controller__";
const BLOB_TAG: &str = "__node_web_blob__";
const FORM_DATA_TAG: &str = "__node_web_form_data__";
const HEADER_PREFIX: &str = "__header_";
const BODY_KEY: &str = "__body";

pub fn install_globals(engine: &mut libjs::JsEngine) {
    engine.set_global("Headers", headers_constructor());
    engine.set_global("Request", request_constructor());
    engine.set_global("Response", response_constructor());
    engine.set_global("AbortController", abort_controller_constructor());
    engine.set_global("AbortSignal", abort_signal_constructor());
    engine.set_global("Blob", blob_constructor());
    engine.set_global("File", file_constructor());
    engine.set_global("FormData", form_data_constructor());
    engine.set_global("fetch", native_fn("fetch", fetch));
}

pub fn globals_module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("Headers"), headers_constructor());
    module.set(String::from("Request"), request_constructor());
    module.set(String::from("Response"), response_constructor());
    module.set(
        String::from("AbortController"),
        abort_controller_constructor(),
    );
    module.set(String::from("AbortSignal"), abort_signal_constructor());
    module.set(String::from("Blob"), blob_constructor());
    module.set(String::from("File"), file_constructor());
    module.set(String::from("FormData"), form_data_constructor());
    module.set(String::from("fetch"), native_fn("fetch", fetch));
    object(module)
}

fn headers_constructor() -> JsValue {
    let ctor = native_ctor_fn("Headers", ctor_headers);
    add_proto_method(&ctor, "append", headers_append);
    add_proto_method(&ctor, "delete", headers_delete);
    add_proto_method(&ctor, "get", headers_get);
    add_proto_method(&ctor, "has", headers_has);
    add_proto_method(&ctor, "set", headers_set);
    add_proto_method(&ctor, "toJSON", headers_to_json);
    ctor
}

fn request_constructor() -> JsValue {
    let ctor = native_ctor_fn("Request", ctor_request);
    add_proto_method(&ctor, "clone", request_clone);
    ctor
}

fn response_constructor() -> JsValue {
    let ctor = native_ctor_fn("Response", ctor_response);
    add_proto_method(&ctor, "clone", response_clone);
    add_proto_method(&ctor, "text", response_text);
    add_proto_method(&ctor, "json", response_json);
    ctor
}

fn abort_controller_constructor() -> JsValue {
    let ctor = native_ctor_fn("AbortController", ctor_abort_controller);
    add_proto_method(&ctor, "abort", abort_controller_abort);
    ctor
}

fn abort_signal_constructor() -> JsValue {
    let ctor = native_ctor_fn("AbortSignal", ctor_abort_signal);
    if let JsValue::Function(func) = &ctor {
        func.borrow_mut().own_props.insert(
            String::from("abort"),
            native_fn("abort", abort_signal_abort),
        );
        func.borrow_mut().own_props.insert(
            String::from("timeout"),
            native_fn("timeout", abort_signal_timeout),
        );
    }
    ctor
}

fn blob_constructor() -> JsValue {
    let ctor = native_ctor_fn("Blob", ctor_blob);
    add_proto_method(&ctor, "text", blob_text);
    ctor
}

fn file_constructor() -> JsValue {
    let ctor = native_ctor_fn("File", ctor_file);
    add_proto_method(&ctor, "text", blob_text);
    ctor
}

fn form_data_constructor() -> JsValue {
    let ctor = native_ctor_fn("FormData", ctor_form_data);
    add_proto_method(&ctor, "append", form_data_append);
    add_proto_method(&ctor, "get", form_data_get);
    add_proto_method(&ctor, "has", form_data_has);
    add_proto_method(&ctor, "set", form_data_set);
    ctor
}

fn add_proto_method(ctor: &JsValue, name: &str, func: fn(&mut Vm, &[JsValue]) -> JsValue) {
    if let JsValue::Function(func_obj) = ctor {
        if let Some(proto) = func_obj.borrow().prototype.clone() {
            proto
                .borrow_mut()
                .set(String::from(name), native_fn(name, func));
        }
    }
}

fn ctor_headers(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = expect_constructed(vm, HEADERS_TAG, "Headers");
    if this.is_undefined() {
        return JsValue::Undefined;
    }
    if let Some(init) = args.first() {
        copy_headers_init(&this, init);
    }
    JsValue::Undefined
}

fn headers_append(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = header_key(args.first());
    let value = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    let previous = vm.current_this.get_property(&key).to_js_string();
    let joined = if previous == "undefined" || previous.is_empty() {
        value
    } else {
        format!("{}, {}", previous, value)
    };
    vm.current_this.set_property(key, JsValue::String(joined));
    JsValue::Undefined
}

fn headers_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = header_key(args.first());
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().properties.remove(&key);
    }
    JsValue::Undefined
}

fn headers_get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = vm.current_this.get_property(&header_key(args.first()));
    if value.is_undefined() {
        JsValue::Null
    } else {
        value
    }
}

fn headers_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(
        !vm.current_this
            .get_property(&header_key(args.first()))
            .is_undefined(),
    )
}

fn headers_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = header_key(args.first());
    let value = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    vm.current_this.set_property(key, JsValue::String(value));
    JsValue::Undefined
}

fn headers_to_json(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let out = JsValue::new_object();
    copy_header_properties(&vm.current_this, &out);
    out
}

fn ctor_request(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = expect_constructed(vm, REQUEST_TAG, "Request");
    if this.is_undefined() {
        return JsValue::Undefined;
    }
    let input = args
        .first()
        .cloned()
        .unwrap_or(JsValue::String(String::new()));
    let init = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let url = match &input {
        JsValue::Object(_) => input.get_property("url").to_js_string(),
        _ => input.to_js_string(),
    };
    let method = clean_option_string(init.get_property("method"), "GET").to_ascii_uppercase();
    let headers = init.get_property("headers");
    let body = init.get_property("body");
    this.set_property(String::from("url"), JsValue::String(url));
    this.set_property(String::from("method"), JsValue::String(method));
    this.set_property(String::from("headers"), make_headers_from(headers));
    this.set_property(String::from("bodyUsed"), JsValue::Bool(false));
    this.set_property(String::from(BODY_KEY), body);
    JsValue::Undefined
}

fn request_clone(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    make_request_object(
        vm.current_this.get_property("url"),
        vm.current_this.get_property("method"),
        vm.current_this.get_property("headers"),
        vm.current_this.get_property(BODY_KEY),
    )
}

fn ctor_response(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = expect_constructed(vm, RESPONSE_TAG, "Response");
    if this.is_undefined() {
        return JsValue::Undefined;
    }
    initialize_response(
        &this,
        args.first()
            .cloned()
            .unwrap_or(JsValue::String(String::new())),
        args.get(1).cloned().unwrap_or(JsValue::Undefined),
    );
    JsValue::Undefined
}

fn response_clone(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    make_response_object(
        vm.current_this.get_property(BODY_KEY),
        vm.current_this.get_property("status"),
        vm.current_this.get_property("headers"),
    )
}

fn response_text(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let body = vm.current_this.get_property(BODY_KEY);
    native_promise::promise_resolve(vm, &[JsValue::String(body.to_js_string())])
}

fn response_json(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let body = vm.current_this.get_property(BODY_KEY).to_js_string();
    let json = vm.get_global("JSON");
    let parse = json.get_property("parse");
    let parsed = vm.call_value(&parse, &[JsValue::String(body)], json);
    native_promise::promise_resolve(vm, &[parsed])
}

fn ctor_abort_controller(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = expect_constructed(vm, ABORT_CONTROLLER_TAG, "AbortController");
    if this.is_undefined() {
        return JsValue::Undefined;
    }
    this.set_property(String::from("signal"), make_abort_signal(false));
    JsValue::Undefined
}

fn abort_controller_abort(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let signal = vm.current_this.get_property("signal");
    signal.set_property(String::from("aborted"), JsValue::Bool(true));
    signal.set_property(
        String::from("reason"),
        args.first()
            .cloned()
            .unwrap_or(JsValue::String(String::from("AbortError"))),
    );
    JsValue::Undefined
}

fn ctor_abort_signal(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = expect_constructed(vm, ABORT_SIGNAL_TAG, "AbortSignal");
    if this.is_undefined() {
        return JsValue::Undefined;
    }
    this.set_property(String::from("aborted"), JsValue::Bool(false));
    this.set_property(String::from("reason"), JsValue::Undefined);
    JsValue::Undefined
}

fn abort_signal_abort(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let signal = make_abort_signal(true);
    signal.set_property(
        String::from("reason"),
        args.first()
            .cloned()
            .unwrap_or(JsValue::String(String::from("AbortError"))),
    );
    signal
}

fn abort_signal_timeout(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    make_abort_signal(false)
}

fn ctor_blob(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = expect_constructed(vm, BLOB_TAG, "Blob");
    if this.is_undefined() {
        return JsValue::Undefined;
    }
    let body = collect_blob_parts(args.first());
    let type_ = args
        .get(1)
        .map(|opts| opts.get_property("type").to_js_string())
        .filter(|value| value != "undefined")
        .unwrap_or_default();
    initialize_blob(&this, body, type_);
    JsValue::Undefined
}

fn ctor_file(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = expect_constructed(vm, BLOB_TAG, "File");
    if this.is_undefined() {
        return JsValue::Undefined;
    }
    let body = collect_blob_parts(args.first());
    let name = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    initialize_blob(&this, body, String::new());
    this.set_property(String::from("name"), JsValue::String(name));
    this.set_property(String::from("lastModified"), JsValue::Number(0.0));
    JsValue::Undefined
}

fn blob_text(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    native_promise::promise_resolve(vm, &[vm.current_this.get_property(BODY_KEY)])
}

fn ctor_form_data(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let this = expect_constructed(vm, FORM_DATA_TAG, "FormData");
    if this.is_undefined() {
        return JsValue::Undefined;
    }
    JsValue::Undefined
}

fn form_data_append(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = form_key(args.first());
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let previous = vm.current_this.get_property(&key);
    if previous.is_undefined() {
        vm.current_this.set_property(key, value);
    } else {
        let joined = JsValue::new_array(alloc::vec![previous, value]);
        vm.current_this.set_property(key, joined);
    }
    JsValue::Undefined
}

fn form_data_get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = vm.current_this.get_property(&form_key(args.first()));
    if value.is_undefined() {
        JsValue::Null
    } else {
        value
    }
}

fn form_data_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(
        !vm.current_this
            .get_property(&form_key(args.first()))
            .is_undefined(),
    )
}

fn form_data_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = form_key(args.first());
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    vm.current_this.set_property(key, value);
    JsValue::Undefined
}

fn fetch(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let request = if let Some(JsValue::Object(_)) = args.first() {
        args.first().cloned().unwrap_or(JsValue::Undefined)
    } else {
        make_request_object(
            args.first()
                .cloned()
                .unwrap_or(JsValue::String(String::new())),
            args.get(1)
                .map(|init| init.get_property("method"))
                .unwrap_or(JsValue::Undefined),
            args.get(1)
                .map(|init| init.get_property("headers"))
                .unwrap_or(JsValue::Undefined),
            args.get(1)
                .map(|init| init.get_property("body"))
                .unwrap_or(JsValue::Undefined),
        )
    };
    let url = request.get_property("url").to_js_string();
    let method = request.get_property("method").to_js_string();
    let headers = request.get_property("headers");
    let body = request.get_property(BODY_KEY);
    let body_string = body_to_string(&body);
    let extra_headers = headers_to_http_block(&headers);
    let content_type = header_value(&headers, "content-type");
    let content_type = if content_type.is_empty() {
        "application/octet-stream"
    } else {
        content_type.as_str()
    };

    let _ = libhttp_client::init();
    let Some(data) = libhttp_client::request_with_headers(
        &url,
        &method,
        body_string.as_bytes(),
        content_type,
        &extra_headers,
    ) else {
        let err = vm.make_type_error("fetch failed");
        return native_promise::promise_reject(vm, &[err]);
    };

    let status = libhttp_client::last_status();
    let response_headers = headers_from_http_header_block(&libhttp_client::last_headers());
    let response = make_response_object(
        JsValue::String(String::from_utf8_lossy(&data).into_owned()),
        JsValue::Number(status as f64),
        response_headers,
    );
    response.set_property(String::from("url"), JsValue::String(url));
    native_promise::promise_resolve(vm, &[response])
}

fn expect_constructed(vm: &mut Vm, tag: &str, name: &str) -> JsValue {
    match &vm.current_this {
        JsValue::Object(obj) => {
            obj.borrow_mut().internal_tag = Some(String::from(tag));
            vm.current_this.clone()
        }
        _ => {
            let err = vm.make_type_error(&format!("Constructor {} requires 'new'", name));
            vm.throw_native(err);
            JsValue::Undefined
        }
    }
}

fn make_headers_from(value: JsValue) -> JsValue {
    let headers = JsValue::Object(Rc::new(RefCell::new(JsObject::with_tag(HEADERS_TAG))));
    copy_headers_init(&headers, &value);
    headers.set_property(String::from("append"), native_fn("append", headers_append));
    headers.set_property(String::from("delete"), native_fn("delete", headers_delete));
    headers.set_property(String::from("get"), native_fn("get", headers_get));
    headers.set_property(String::from("has"), native_fn("has", headers_has));
    headers.set_property(String::from("set"), native_fn("set", headers_set));
    headers
}

fn copy_headers_init(target: &JsValue, init: &JsValue) {
    if init.is_undefined() || init.is_null() {
        return;
    }
    if let JsValue::Object(obj) = init {
        let props = obj.borrow().properties.clone();
        for (key, prop) in props {
            if !prop.enumerable {
                continue;
            }
            if key.starts_with(HEADER_PREFIX) {
                target.set_property(key, JsValue::String(prop.value.to_js_string()));
            } else if !key.starts_with("__") {
                target.set_property(
                    header_key_from_name(&key),
                    JsValue::String(prop.value.to_js_string()),
                );
            }
        }
    }
}

fn copy_header_properties(from: &JsValue, to: &JsValue) {
    if let JsValue::Object(obj) = from {
        let props = obj.borrow().properties.clone();
        for (key, prop) in props {
            if let Some(name) = key.strip_prefix(HEADER_PREFIX) {
                to.set_property(String::from(name), prop.value);
            }
        }
    }
}

fn headers_to_http_block(headers: &JsValue) -> String {
    let mut out = String::new();
    if let JsValue::Object(obj) = headers {
        let props = obj.borrow().properties.clone();
        for (key, prop) in props {
            let Some(name) = key.strip_prefix(HEADER_PREFIX) else {
                continue;
            };
            if name == "host" || name == "content-length" || name == "connection" {
                continue;
            }
            out.push_str(name);
            out.push_str(": ");
            out.push_str(&prop.value.to_js_string());
            out.push_str("\r\n");
        }
    }
    out
}

fn headers_from_http_header_block(block: &str) -> JsValue {
    let headers = JsValue::Object(Rc::new(RefCell::new(JsObject::with_tag(HEADERS_TAG))));
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        let Some(idx) = line.find(':') else {
            continue;
        };
        let name = line[..idx].trim();
        let value = line[idx + 1..].trim();
        if !name.is_empty() {
            headers.set_property(
                header_key_from_name(name),
                JsValue::String(String::from(value)),
            );
        }
    }
    headers.set_property(String::from("append"), native_fn("append", headers_append));
    headers.set_property(String::from("delete"), native_fn("delete", headers_delete));
    headers.set_property(String::from("get"), native_fn("get", headers_get));
    headers.set_property(String::from("has"), native_fn("has", headers_has));
    headers.set_property(String::from("set"), native_fn("set", headers_set));
    headers
}

fn header_value(headers: &JsValue, name: &str) -> String {
    let value = headers.get_property(&header_key_from_name(name));
    if value.is_undefined() || value.is_null() {
        String::new()
    } else {
        value.to_js_string()
    }
}

fn body_to_string(body: &JsValue) -> String {
    if body.is_undefined() || body.is_null() {
        String::new()
    } else {
        let nested = body.get_property(BODY_KEY);
        if !nested.is_undefined() {
            nested.to_js_string()
        } else {
            body.to_js_string()
        }
    }
}

fn make_request_object(url: JsValue, method: JsValue, headers: JsValue, body: JsValue) -> JsValue {
    let obj = JsValue::Object(Rc::new(RefCell::new(JsObject::with_tag(REQUEST_TAG))));
    obj.set_property(String::from("url"), JsValue::String(url.to_js_string()));
    let method = clean_option_string(method, "GET").to_ascii_uppercase();
    obj.set_property(String::from("method"), JsValue::String(method));
    obj.set_property(String::from("headers"), make_headers_from(headers));
    obj.set_property(String::from("bodyUsed"), JsValue::Bool(false));
    obj.set_property(String::from(BODY_KEY), body);
    obj.set_property(String::from("clone"), native_fn("clone", request_clone));
    obj
}

fn make_response_object(body: JsValue, status: JsValue, headers: JsValue) -> JsValue {
    let obj = JsValue::Object(Rc::new(RefCell::new(JsObject::with_tag(RESPONSE_TAG))));
    let init = JsValue::new_object();
    init.set_property(String::from("status"), status);
    init.set_property(String::from("headers"), headers);
    initialize_response(&obj, body, init);
    obj.set_property(String::from("clone"), native_fn("clone", response_clone));
    obj.set_property(String::from("text"), native_fn("text", response_text));
    obj.set_property(String::from("json"), native_fn("json", response_json));
    obj
}

fn initialize_response(this: &JsValue, body: JsValue, init: JsValue) {
    let status = init.get_property("status").to_number();
    let status = if status.is_nan() || status == 0.0 {
        200.0
    } else {
        status
    };
    this.set_property(String::from("status"), JsValue::Number(status));
    this.set_property(
        String::from("ok"),
        JsValue::Bool((200.0..300.0).contains(&status)),
    );
    this.set_property(
        String::from("statusText"),
        JsValue::String(clean_option_string(init.get_property("statusText"), "")),
    );
    this.set_property(
        String::from("headers"),
        make_headers_from(init.get_property("headers")),
    );
    this.set_property(String::from("bodyUsed"), JsValue::Bool(false));
    this.set_property(String::from(BODY_KEY), body);
}

fn make_abort_signal(aborted: bool) -> JsValue {
    let signal = JsValue::Object(Rc::new(RefCell::new(JsObject::with_tag(ABORT_SIGNAL_TAG))));
    signal.set_property(String::from("aborted"), JsValue::Bool(aborted));
    signal.set_property(String::from("reason"), JsValue::Undefined);
    signal
}

fn initialize_blob(this: &JsValue, body: String, type_: String) {
    this.set_property(String::from("size"), JsValue::Number(body.len() as f64));
    this.set_property(String::from("type"), JsValue::String(type_));
    this.set_property(String::from(BODY_KEY), JsValue::String(body));
}

fn collect_blob_parts(value: Option<&JsValue>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    match value {
        JsValue::Array(array) => array
            .borrow()
            .to_dense_vec()
            .iter()
            .map(|part| part.to_js_string())
            .collect::<alloc::vec::Vec<String>>()
            .join(""),
        other => other.to_js_string(),
    }
}

fn header_key(value: Option<&JsValue>) -> String {
    header_key_from_name(&value.map(|v| v.to_js_string()).unwrap_or_default())
}

fn header_key_from_name(name: &str) -> String {
    format!("{}{}", HEADER_PREFIX, name.to_ascii_lowercase())
}

fn form_key(value: Option<&JsValue>) -> String {
    format!(
        "__form_{}",
        value.map(|v| v.to_js_string()).unwrap_or_default()
    )
}

fn clean_option_string(value: JsValue, default: &str) -> String {
    if value.is_undefined() || value.is_null() {
        String::from(default)
    } else {
        value.to_js_string()
    }
}
