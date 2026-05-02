use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

pub fn module(loop_: &libuv::UvLoop) -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("now"), JsValue::Number(loop_.now_ms as f64));
    module.set(String::from("run"), native_fn("run", run));
    object(module)
}

fn run(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(0.0)
}
