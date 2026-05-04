use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_ctor_fn, native_fn, Vm};

use super::buffer::buffer_to_bytes;
use super::util::object;

const ENCODING_KEY: &str = "__node_string_decoder_encoding__";

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("StringDecoder"),
        native_ctor_fn("StringDecoder", constructor),
    );
    object(module)
}

fn constructor(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let encoding = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_else(|| String::from("utf8"));
    vm.current_this
        .set_property(String::from(ENCODING_KEY), JsValue::String(encoding));
    vm.current_this
        .set_property(String::from("write"), native_fn("write", write));
    vm.current_this
        .set_property(String::from("end"), native_fn("end", end));
    JsValue::Undefined
}

fn write(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(decode(args.first().unwrap_or(&JsValue::Undefined)))
}

fn end(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(decode(args.first().unwrap_or(&JsValue::Undefined)))
}

fn decode(value: &JsValue) -> String {
    match value {
        JsValue::String(text) => text.clone(),
        value if super::buffer::is_buffer_like(value) => {
            String::from_utf8_lossy(&buffer_to_bytes(value)).into_owned()
        }
        JsValue::Array(array) => String::from_utf8_lossy(
            &array
                .borrow()
                .to_dense_vec()
                .into_iter()
                .map(|value| value.to_number().clamp(0.0, 255.0) as u8)
                .collect::<alloc::vec::Vec<u8>>(),
        )
        .into_owned(),
        JsValue::Undefined | JsValue::Null => String::new(),
        value => value.to_js_string(),
    }
}
