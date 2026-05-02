use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::process::current_dir;
use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("platform"), native_fn("platform", platform));
    module.set(String::from("type"), native_fn("type", os_type));
    module.set(String::from("release"), native_fn("release", release));
    module.set(String::from("arch"), native_fn("arch", arch));
    module.set(String::from("homedir"), native_fn("homedir", homedir));
    module.set(String::from("tmpdir"), native_fn("tmpdir", tmpdir));
    module.set(String::from("cwd"), native_fn("cwd", cwd));
    module.set(String::from("EOL"), JsValue::String(String::from("\n")));
    object(module)
}

fn platform(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("anyos"))
}

fn os_type(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("AnyOS"))
}

fn release(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("0.1.0"))
}

fn arch(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("x64"))
}

fn homedir(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("/Users"))
}

fn tmpdir(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("/tmp"))
}

fn cwd(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(current_dir())
}
