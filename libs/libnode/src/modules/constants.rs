use alloc::string::String;
use libjs::value::{JsObject, JsValue};

use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("O_RDONLY"), JsValue::Number(0.0));
    module.set(String::from("O_WRONLY"), JsValue::Number(1.0));
    module.set(String::from("O_RDWR"), JsValue::Number(2.0));
    module.set(String::from("O_CREAT"), JsValue::Number(64.0));
    module.set(String::from("O_EXCL"), JsValue::Number(128.0));
    module.set(String::from("O_TRUNC"), JsValue::Number(512.0));
    module.set(String::from("O_APPEND"), JsValue::Number(1024.0));
    module.set(String::from("S_IFREG"), JsValue::Number(0o100000 as f64));
    module.set(String::from("S_IFDIR"), JsValue::Number(0o040000 as f64));
    module.set(String::from("UV_UDP_REUSEADDR"), JsValue::Number(4.0));
    object(module)
}
