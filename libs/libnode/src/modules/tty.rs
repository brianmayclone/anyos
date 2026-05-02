use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("isatty"), native_fn("isatty", isatty));
    module.set(String::from("ReadStream"), native_fn("ReadStream", stream));
    module.set(
        String::from("WriteStream"),
        native_fn("WriteStream", stream),
    );
    object(module)
}

fn isatty(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let fd = args
        .first()
        .map(|value| value.to_number() as i32)
        .unwrap_or(-1);
    JsValue::Bool(matches!(fd, 0..=2))
}

fn stream(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let fd = args.first().map(|value| value.to_number()).unwrap_or(0.0);
    let stream = JsValue::new_object();
    stream.set_property(String::from("fd"), JsValue::Number(fd));
    stream.set_property(
        String::from("isTTY"),
        JsValue::Bool(matches!(fd as i32, 0..=2)),
    );
    stream.set_property(String::from("write"), native_fn("write", write));
    stream
}

fn write(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(value) = args.first() {
        anyos_std::print!("{}", value.to_js_string());
    }
    JsValue::Bool(true)
}
