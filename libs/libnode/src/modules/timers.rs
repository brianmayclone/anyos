use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("setTimeout"),
        native_fn("setTimeout", set_timeout),
    );
    module.set(
        String::from("clearTimeout"),
        native_fn("clearTimeout", clear_timeout),
    );
    module.set(
        String::from("setInterval"),
        native_fn("setInterval", set_interval),
    );
    module.set(
        String::from("clearInterval"),
        native_fn("clearInterval", clear_interval),
    );
    module.set(
        String::from("setImmediate"),
        native_fn("setImmediate", set_immediate),
    );
    module.set(
        String::from("clearImmediate"),
        native_fn("clearImmediate", clear_immediate),
    );
    object(module)
}

fn set_timeout(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    call_global(vm, "setTimeout", args)
}

fn clear_timeout(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    call_global(vm, "clearTimeout", args)
}

fn set_interval(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    call_global(vm, "setInterval", args)
}

fn clear_interval(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    call_global(vm, "clearInterval", args)
}

fn set_immediate(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let mut timer_args = alloc::vec![callback, JsValue::Number(0.0)];
    timer_args.extend(args.iter().skip(1).cloned());
    call_global(vm, "setTimeout", &timer_args)
}

fn clear_immediate(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    call_global(vm, "clearTimeout", args)
}

fn call_global(vm: &mut Vm, name: &str, args: &[JsValue]) -> JsValue {
    let callee = vm.get_global(name);
    if !matches!(callee, JsValue::Function(_)) {
        vm.pending_exception = Some(vm.make_type_error("timer function is not available"));
        return JsValue::Undefined;
    }
    vm.call_value(&callee, args, JsValue::Undefined)
}
