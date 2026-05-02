use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_ctor_fn, native_fn, Vm};

use super::util::object;

const BUFFER_TAG: &str = "__node_buffer__";
const DATA_KEY: &str = "__node_buffer_data__";

pub fn module() -> JsValue {
    let buffer = buffer_constructor();
    let mut module = JsObject::new();
    module.set(String::from("Buffer"), buffer);
    object(module)
}

pub fn buffer_global() -> JsValue {
    buffer_constructor()
}

fn buffer_constructor() -> JsValue {
    let ctor = native_ctor_fn("Buffer", buffer_new);
    if let JsValue::Function(func) = &ctor {
        func.borrow_mut()
            .own_props
            .insert(String::from("from"), native_fn("from", buffer_from));
        func.borrow_mut()
            .own_props
            .insert(String::from("alloc"), native_fn("alloc", buffer_alloc));
        func.borrow_mut()
            .own_props
            .insert(String::from("isBuffer"), native_fn("isBuffer", is_buffer));
    }
    ctor
}

fn buffer_new(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    make_buffer(buffer_bytes_from(args.first()))
}

fn buffer_from(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    make_buffer(buffer_bytes_from(args.first()))
}

fn buffer_alloc(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let len = args
        .first()
        .map(|value| value.to_number().max(0.0) as usize)
        .unwrap_or(0);
    make_buffer(vec![0; len])
}

fn is_buffer(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(is_buffer_value(args.first().unwrap_or(&JsValue::Undefined)))
}

fn to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let bytes = read_buffer_bytes(&vm.current_this);
    JsValue::String(String::from_utf8_lossy(&bytes).into_owned())
}

fn to_json(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let bytes = read_buffer_bytes(&vm.current_this);
    let data = JsValue::new_array(
        bytes
            .into_iter()
            .map(|byte| JsValue::Number(byte as f64))
            .collect(),
    );
    let out = JsValue::new_object();
    out.set_property(
        String::from("type"),
        JsValue::String(String::from("Buffer")),
    );
    out.set_property(String::from("data"), data);
    out
}

fn make_buffer(bytes: Vec<u8>) -> JsValue {
    let mut obj = JsObject::with_tag(BUFFER_TAG);
    obj.set(String::from("length"), JsValue::Number(bytes.len() as f64));
    obj.set(String::from("toString"), native_fn("toString", to_string));
    obj.set(String::from("toJSON"), native_fn("toJSON", to_json));
    obj.set_hidden(String::from(DATA_KEY), bytes_to_array(bytes));
    object(obj)
}

pub fn buffer_from_bytes(bytes: Vec<u8>) -> JsValue {
    make_buffer(bytes)
}

fn bytes_to_array(bytes: Vec<u8>) -> JsValue {
    JsValue::new_array(
        bytes
            .into_iter()
            .map(|byte| JsValue::Number(byte as f64))
            .collect(),
    )
}

fn read_buffer_bytes(value: &JsValue) -> Vec<u8> {
    match value.get_property(DATA_KEY) {
        JsValue::Array(array) => array
            .borrow()
            .to_dense_vec()
            .into_iter()
            .map(|value| value.to_number().clamp(0.0, 255.0) as u8)
            .collect(),
        _ => Vec::new(),
    }
}

pub fn buffer_to_bytes(value: &JsValue) -> Vec<u8> {
    read_buffer_bytes(value)
}

fn buffer_bytes_from(value: Option<&JsValue>) -> Vec<u8> {
    match value {
        Some(JsValue::String(text)) => text.as_bytes().to_vec(),
        Some(JsValue::Array(array)) => array
            .borrow()
            .to_dense_vec()
            .into_iter()
            .map(|value| value.to_number().clamp(0.0, 255.0) as u8)
            .collect(),
        Some(value) if is_buffer_value(value) => read_buffer_bytes(value),
        Some(value) => value.to_js_string().into_bytes(),
        None => Vec::new(),
    }
}

fn is_buffer_value(value: &JsValue) -> bool {
    matches!(value, JsValue::Object(obj) if obj.borrow().internal_tag.as_deref() == Some(BUFFER_TAG))
}

pub fn is_buffer_like(value: &JsValue) -> bool {
    is_buffer_value(value)
}
