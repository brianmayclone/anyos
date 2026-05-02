use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("createRequire"),
        native_fn("createRequire", create_require),
    );
    object(module)
}

fn create_require(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.get_global("require")
}
