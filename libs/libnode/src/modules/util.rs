use alloc::rc::Rc;
use alloc::string::{String, ToString};
use core::cell::RefCell;
use libjs::value::{JsObject, JsValue};

pub fn object(module: JsObject) -> JsValue {
    JsValue::Object(Rc::new(RefCell::new(module)))
}

pub fn empty_object() -> JsValue {
    object(JsObject::new())
}

pub fn string_array(values: &[String]) -> JsValue {
    let mut object = JsObject::new();
    for (idx, value) in values.iter().enumerate() {
        object.set(idx.to_string(), JsValue::String(value.clone()));
    }
    object.set(String::from("length"), JsValue::Number(values.len() as f64));
    self::object(object)
}
