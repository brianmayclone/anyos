use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
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
    process.set(String::from("binding"), native_fn("binding", binding));
    process.set(String::from("nextTick"), native_fn("nextTick", next_tick));
    process.set(String::from("env"), env_object());
    process.set(String::from("argv"), string_array(&options.argv));
    JsValue::Object(Rc::new(RefCell::new(process)))
}

fn cwd(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(current_dir())
}

fn binding(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    match name.as_str() {
        "buffer" => {
            let out = JsValue::new_object();
            out.set_property(
                String::from("kStringMaxLength"),
                JsValue::Number((usize::MAX / 2) as f64),
            );
            out
        }
        "tty_wrap" => {
            let out = JsValue::new_object();
            out.set_property(
                String::from("guessHandleType"),
                native_fn("guessHandleType", guess_handle_type),
            );
            out
        }
        _ => {
            vm.pending_exception = Some(vm.make_type_error(&alloc::format!(
                "No such module: {}",
                name
            )));
            JsValue::Undefined
        }
    }
}

fn guess_handle_type(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first().map(|value| value.to_number() as i32) {
        Some(0) => JsValue::String(String::from("TTY")),
        Some(1) | Some(2) => JsValue::String(String::from("TTY")),
        _ => JsValue::String(String::from("FILE")),
    }
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

fn env_object() -> JsValue {
    let env = JsValue::new_object();
    env.set_property(
        String::from("NODE_ENV"),
        JsValue::String(String::from("development")),
    );
    env
}

fn next_tick(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(callback) = args.first().cloned() else {
        vm.pending_exception = Some(vm.make_type_error("process.nextTick requires a callback"));
        return JsValue::Undefined;
    };
    if !matches!(callback, JsValue::Function(_)) {
        vm.pending_exception = Some(vm.make_type_error("process.nextTick requires a callback"));
        return JsValue::Undefined;
    }
    let tick_args = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        Vec::new()
    };
    vm.enqueue_microtask(callback, tick_args);
    JsValue::Undefined
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
