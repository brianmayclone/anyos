use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

pub fn module() -> JsValue {
    let assert = native_fn("assert", ok);
    if let JsValue::Function(func) = &assert {
        let mut func = func.borrow_mut();
        func.own_props
            .insert(String::from("ok"), native_fn("ok", ok));
        func.own_props
            .insert(String::from("equal"), native_fn("equal", equal));
        func.own_props.insert(
            String::from("strictEqual"),
            native_fn("strictEqual", strict_equal),
        );
        func.own_props.insert(
            String::from("notStrictEqual"),
            native_fn("notStrictEqual", not_strict_equal),
        );
    }
    assert
}

pub fn strict_module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("ok"), native_fn("ok", ok));
    module.set(String::from("equal"), native_fn("equal", strict_equal));
    module.set(
        String::from("strictEqual"),
        native_fn("strictEqual", strict_equal),
    );
    module.set(
        String::from("notStrictEqual"),
        native_fn("notStrictEqual", not_strict_equal),
    );
    object(module)
}

fn ok(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.first().map(is_truthy).unwrap_or(false) {
        return JsValue::Undefined;
    }
    throw_assertion(vm, args.get(1), "Expected value to be truthy")
}

fn equal(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let left = args.first().unwrap_or(&JsValue::Undefined).to_js_string();
    let right = args.get(1).unwrap_or(&JsValue::Undefined).to_js_string();
    if left == right {
        JsValue::Undefined
    } else {
        throw_assertion(vm, args.get(2), "Expected values to be equal")
    }
}

fn strict_equal(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if same_value(args.first(), args.get(1)) {
        JsValue::Undefined
    } else {
        throw_assertion(vm, args.get(2), "Expected values to be strictly equal")
    }
}

fn not_strict_equal(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !same_value(args.first(), args.get(1)) {
        JsValue::Undefined
    } else {
        throw_assertion(vm, args.get(2), "Expected values to be different")
    }
}

fn throw_assertion(vm: &mut Vm, message: Option<&JsValue>, fallback: &str) -> JsValue {
    let message = message
        .map(|value| value.to_js_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| String::from(fallback));
    vm.pending_exception = Some(vm.make_error(&message));
    JsValue::Undefined
}

fn same_value(left: Option<&JsValue>, right: Option<&JsValue>) -> bool {
    match (
        left.unwrap_or(&JsValue::Undefined),
        right.unwrap_or(&JsValue::Undefined),
    ) {
        (JsValue::Undefined, JsValue::Undefined) | (JsValue::Null, JsValue::Null) => true,
        (JsValue::Bool(a), JsValue::Bool(b)) => a == b,
        (JsValue::Number(a), JsValue::Number(b)) => a == b,
        (JsValue::String(a), JsValue::String(b)) => a == b,
        (JsValue::Object(a), JsValue::Object(b)) => alloc::rc::Rc::ptr_eq(a, b),
        (JsValue::Array(a), JsValue::Array(b)) => alloc::rc::Rc::ptr_eq(a, b),
        (JsValue::Function(a), JsValue::Function(b)) => alloc::rc::Rc::ptr_eq(a, b),
        _ => false,
    }
}

fn is_truthy(value: &JsValue) -> bool {
    match value {
        JsValue::Undefined | JsValue::Null => false,
        JsValue::Bool(value) => *value,
        JsValue::Number(value) => *value != 0.0 && !value.is_nan(),
        JsValue::String(value) => !value.is_empty(),
        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) | JsValue::BigInt(_) => true,
        JsValue::Empty => false,
    }
}
