use alloc::string::String;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("lookup"), native_fn("lookup", lookup));
    module.set(String::from("resolve"), native_fn("resolve", resolve));
    module.set(String::from("resolve4"), native_fn("resolve4", resolve4));
    module.set(String::from("promises"), promises_object());
    module.set(String::from("ADDRCONFIG"), JsValue::Number(32.0));
    module.set(String::from("V4MAPPED"), JsValue::Number(8.0));
    module.set(String::from("ALL"), JsValue::Number(16.0));
    object(module)
}

fn lookup(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let host = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let callback = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)));
    let ip = resolve_host(&host);
    if let Some(callback) = callback {
        match ip {
            Some(address) => {
                vm.call_value(
                    callback,
                    &[
                        JsValue::Null,
                        JsValue::String(address),
                        JsValue::Number(4.0),
                    ],
                    JsValue::Undefined,
                );
            }
            None => {
                vm.call_value(
                    callback,
                    &[dns_error(&host), JsValue::Undefined, JsValue::Undefined],
                    JsValue::Undefined,
                );
            }
        }
        return JsValue::Undefined;
    }
    ip.map(JsValue::String).unwrap_or(JsValue::Undefined)
}

fn resolve(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    resolve_common(vm, args)
}

fn resolve4(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    resolve_common(vm, args)
}

fn resolve_common(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let host = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let callback = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)));
    let result = resolve_host(&host)
        .map(|address| JsValue::new_array(Vec::from([JsValue::String(address)])))
        .unwrap_or_else(|| JsValue::new_array(Vec::new()));
    if let Some(callback) = callback {
        let err = if array_len(&result) == 0 {
            dns_error(&host)
        } else {
            JsValue::Null
        };
        vm.call_value(callback, &[err, result.clone()], JsValue::Undefined);
        return JsValue::Undefined;
    }
    result
}

fn promises_object() -> JsValue {
    let mut promises = JsObject::new();
    promises.set(String::from("lookup"), native_fn("lookup", lookup));
    promises.set(String::from("resolve"), native_fn("resolve", resolve));
    promises.set(String::from("resolve4"), native_fn("resolve4", resolve4));
    object(promises)
}

fn resolve_host(host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    if is_ipv4_literal(host) {
        return Some(String::from(host));
    }
    if host == "localhost" {
        return Some(String::from("127.0.0.1"));
    }
    let mut ip = [0u8; 4];
    if anyos_std::net::dns(host, &mut ip) == 0 {
        Some(alloc::format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]))
    } else {
        None
    }
}

fn dns_error(host: &str) -> JsValue {
    let error = JsValue::new_object();
    error.set_property(
        String::from("code"),
        JsValue::String(String::from("ENOTFOUND")),
    );
    error.set_property(
        String::from("message"),
        JsValue::String(alloc::format!("getaddrinfo ENOTFOUND {}", host)),
    );
    error
}

fn is_ipv4_literal(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<&str>>();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
}

fn array_len(value: &JsValue) -> usize {
    value.get_property("length").to_number().max(0.0) as usize
}
