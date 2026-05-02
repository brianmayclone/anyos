use alloc::format;
use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::Vm;

use super::util::{empty_object, object};

pub fn require(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let specifier = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    vm.module_registry
        .get(&specifier)
        .cloned()
        .unwrap_or_else(|| {
            vm.pending_exception =
                Some(vm.make_type_error(&format!("Cannot find module '{}'", specifier)));
            JsValue::Undefined
        })
}

pub fn resolve(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let specifier = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let map = vm.get_global("__node_resolved__");
    let resolved = map.get_property(&specifier);
    if !matches!(resolved, JsValue::Undefined) {
        return resolved;
    }
    if vm.module_registry.contains_key(&specifier) {
        return JsValue::String(specifier);
    }
    vm.pending_exception = Some(vm.make_type_error(&format!("Cannot find module '{}'", specifier)));
    JsValue::Undefined
}

pub fn module_object(filename: &str, dirname: &str) -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("id"), JsValue::String(String::from(filename)));
    module.set(
        String::from("filename"),
        JsValue::String(String::from(filename)),
    );
    module.set(String::from("path"), JsValue::String(String::from(dirname)));
    module.set(String::from("loaded"), JsValue::Bool(false));
    module.set(String::from("exports"), empty_object());
    object(module)
}
