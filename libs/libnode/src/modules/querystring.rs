use alloc::string::String;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("parse"), native_fn("parse", parse));
    module.set(String::from("decode"), native_fn("decode", parse));
    module.set(String::from("stringify"), native_fn("stringify", stringify));
    module.set(String::from("encode"), native_fn("encode", stringify));
    module.set(String::from("escape"), native_fn("escape", escape));
    module.set(String::from("unescape"), native_fn("unescape", unescape));
    object(module)
}

fn parse(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let input = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let sep = args
        .get(1)
        .map(|value| value.to_js_string())
        .filter(|value| !value.is_empty() && value != "undefined")
        .unwrap_or_else(|| String::from("&"));
    let eq = args
        .get(2)
        .map(|value| value.to_js_string())
        .filter(|value| !value.is_empty() && value != "undefined")
        .unwrap_or_else(|| String::from("="));
    let out = JsValue::new_object();
    for pair in input.split(&sep) {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair
            .split_once(&eq)
            .map(|(key, value)| (key, value))
            .unwrap_or((pair, ""));
        out.set_property(
            percent_decode(key.replace('+', " ").as_str()),
            JsValue::String(percent_decode(value.replace('+', " ").as_str())),
        );
    }
    out
}

fn stringify(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(input) = args.first() else {
        return JsValue::String(String::new());
    };
    let sep = args
        .get(1)
        .map(|value| value.to_js_string())
        .filter(|value| !value.is_empty() && value != "undefined")
        .unwrap_or_else(|| String::from("&"));
    let eq = args
        .get(2)
        .map(|value| value.to_js_string())
        .filter(|value| !value.is_empty() && value != "undefined")
        .unwrap_or_else(|| String::from("="));
    let mut pairs = Vec::new();
    if let JsValue::Object(obj) = input {
        for key in obj.borrow().keys() {
            let value = input.get_property(&key);
            if matches!(value, JsValue::Undefined) {
                continue;
            }
            pairs.push(format_pair(&key, &value.to_js_string(), &eq));
        }
    }
    JsValue::String(pairs.join(&sep))
}

fn escape(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(percent_encode(
        &args
            .first()
            .map(|value| value.to_js_string())
            .unwrap_or_default(),
    ))
}

fn unescape(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(percent_decode(
        &args
            .first()
            .map(|value| value.to_js_string())
            .unwrap_or_default(),
    ))
}

fn format_pair(key: &str, value: &str, eq: &str) -> String {
    let mut out = percent_encode(key);
    out.push_str(eq);
    out.push_str(&percent_encode(value));
    out
}

fn percent_encode(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else if byte == b' ' {
            out.push_str("%20");
        } else {
            out.push('%');
            out.push(nibble(byte >> 4));
            out.push(nibble(byte & 0x0f));
        }
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%' && idx + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[idx + 1]), hex(bytes[idx + 2])) {
                out.push((hi << 4) | lo);
                idx += 3;
                continue;
            }
        }
        out.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn nibble(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'A' + value - 10) as char,
    }
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
