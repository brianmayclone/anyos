use alloc::format;
use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("readFileSync"),
        native_fn("readFileSync", read_file_sync),
    );
    module.set(
        String::from("writeFileSync"),
        native_fn("writeFileSync", write_file_sync),
    );
    module.set(
        String::from("existsSync"),
        native_fn("existsSync", exists_sync),
    );
    module.set(
        String::from("mkdirSync"),
        native_fn("mkdirSync", mkdir_sync),
    );
    object(module)
}

fn read_file_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.readFileSync requires a path"));
        return JsValue::Undefined;
    };
    match anyos_std::fs::read_to_string(&path) {
        Ok(data) => JsValue::String(data),
        Err(_) => {
            vm.pending_exception = Some(vm.make_type_error(&format!("ENOENT: {}", path)));
            JsValue::Undefined
        }
    }
}

fn write_file_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.writeFileSync requires a path"));
        return JsValue::Undefined;
    };
    let data = args
        .get(1)
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    if anyos_std::fs::write_bytes(&path, data.as_bytes()).is_err() {
        vm.pending_exception = Some(vm.make_type_error(&format!("EIO: {}", path)));
    }
    JsValue::Undefined
}

fn exists_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let path = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    JsValue::Bool(
        anyos_std::fs::read_to_vec(&path)
            .map(|_| true)
            .unwrap_or(false),
    )
}

fn mkdir_sync(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(path) = args.first().map(|value| value.to_js_string()) else {
        vm.pending_exception = Some(vm.make_type_error("fs.mkdirSync requires a path"));
        return JsValue::Undefined;
    };
    if anyos_std::fs::mkdir(&path) == u32::MAX {
        vm.pending_exception = Some(vm.make_type_error(&format!("EIO: {}", path)));
    }
    JsValue::Undefined
}
