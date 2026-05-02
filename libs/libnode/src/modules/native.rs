use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use crate::options::NativeModulePolicy;

use super::util::{object, string_array};

pub fn ffi_module(policy: &NativeModulePolicy) -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("enabled"), JsValue::Bool(policy.allow_ffi));
    module.set(String::from("open"), native_fn("open", ffi_open));
    module.set(String::from("call"), native_fn("call", ffi_call));
    module.set(
        String::from("allowedLibraries"),
        string_array(&policy.allowed_libraries),
    );
    object(module)
}

pub fn anyui_module(_policy: &NativeModulePolicy) -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("library"),
        JsValue::String(String::from("libanyui.so")),
    );
    module.set(
        String::from("createApp"),
        native_fn("createApp", native_pending),
    );
    module.set(String::from("run"), native_fn("run", native_pending));
    object(module)
}

pub fn image_module(_policy: &NativeModulePolicy) -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("library"),
        JsValue::String(String::from("libimage.so")),
    );
    module.set(String::from("load"), native_fn("load", native_pending));
    object(module)
}

fn ffi_open(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.pending_exception = Some(vm.make_type_error("Native FFI is disabled by policy"));
    JsValue::Undefined
}

fn ffi_call(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.pending_exception = Some(vm.make_type_error("Native FFI calls are not available yet"));
    JsValue::Undefined
}

fn native_pending(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.pending_exception = Some(vm.make_type_error("Native module binding is not linked yet"));
    JsValue::Undefined
}
