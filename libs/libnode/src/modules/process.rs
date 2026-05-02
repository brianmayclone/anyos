use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use crate::options::NodeOptions;
use crate::VERSION;

use super::util::string_array;

pub fn module(options: &NodeOptions) -> JsValue {
    let mut process = JsObject::new();
    process.set(String::from("versions"), versions_object());
    process.set(
        String::from("platform"),
        JsValue::String(String::from("anyos")),
    );
    process.set(String::from("cwd"), native_fn("cwd", cwd));
    process.set(String::from("argv"), string_array(&options.argv));
    JsValue::Object(Rc::new(RefCell::new(process)))
}

fn cwd(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(current_dir())
}

fn versions_object() -> JsValue {
    let mut versions = JsObject::new();
    versions.set(String::from("node"), JsValue::String(String::from(VERSION)));
    versions.set(
        String::from("libnode"),
        JsValue::String(String::from(VERSION)),
    );
    versions.set(
        String::from("libjs"),
        JsValue::String(String::from("anyos")),
    );
    JsValue::Object(Rc::new(RefCell::new(versions)))
}

pub fn current_dir() -> String {
    let mut buf = [0u8; 512];
    let len = anyos_std::fs::getcwd(&mut buf);
    if len == u32::MAX {
        return String::from(".");
    }
    let len = (len as usize).min(buf.len());
    String::from(core::str::from_utf8(&buf[..len]).unwrap_or("."))
}
