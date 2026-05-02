use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::buffer::is_buffer_like;

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("format"), native_fn("format", format));
    module.set(String::from("inspect"), native_fn("inspect", inspect));
    module.set(String::from("inherits"), native_fn("inherits", inherits));
    module.set(String::from("promisify"), native_fn("promisify", identity));
    module.set(
        String::from("callbackify"),
        native_fn("callbackify", identity),
    );
    module.set(String::from("isArray"), native_fn("isArray", is_array));
    module.set(String::from("isBuffer"), native_fn("isBuffer", is_buffer));
    module.set(String::from("types"), types_object());
    object(module)
}

pub fn object(module: JsObject) -> JsValue {
    JsValue::Object(Rc::new(RefCell::new(module)))
}

pub fn empty_object() -> JsValue {
    object(JsObject::new())
}

pub fn string_array(values: &[String]) -> JsValue {
    JsValue::new_array(
        values
            .iter()
            .map(|value| JsValue::String(value.clone()))
            .collect(),
    )
}

fn format(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(first) = args.first() else {
        return JsValue::String(String::new());
    };
    let fmt = first.to_js_string();
    if !fmt.contains('%') {
        return JsValue::String(
            args.iter()
                .map(inspect_value)
                .collect::<Vec<String>>()
                .join(" "),
        );
    }
    let mut out = String::new();
    let mut chars = fmt.chars();
    let mut arg_idx = 1usize;
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        let Some(kind) = chars.next() else {
            out.push('%');
            break;
        };
        if kind == '%' {
            out.push('%');
            continue;
        }
        let value = args.get(arg_idx).cloned().unwrap_or(JsValue::Undefined);
        arg_idx += 1;
        match kind {
            's' => out.push_str(&value.to_js_string()),
            'd' | 'i' => out.push_str(&(value.to_number() as i64).to_string()),
            'f' => out.push_str(&value.to_number().to_string()),
            'j' | 'o' | 'O' => out.push_str(&inspect_value(&value)),
            other => {
                out.push('%');
                out.push(other);
                arg_idx = arg_idx.saturating_sub(1);
            }
        }
    }
    for value in &args[arg_idx..] {
        out.push(' ');
        out.push_str(&inspect_value(value));
    }
    JsValue::String(out)
}

fn inspect(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(inspect_value(args.first().unwrap_or(&JsValue::Undefined)))
}

fn inherits(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(ctor) = args.first() else {
        return JsValue::Undefined;
    };
    let Some(super_ctor) = args.get(1) else {
        return JsValue::Undefined;
    };
    let super_proto = match super_ctor {
        JsValue::Function(func) => func.borrow().prototype.clone(),
        _ => None,
    };
    if let (JsValue::Function(ctor), Some(super_proto)) = (ctor, super_proto) {
        let proto = ctor.borrow().prototype.clone();
        if let Some(proto) = proto {
            proto.borrow_mut().prototype = Some(super_proto);
        }
        ctor.borrow_mut()
            .own_props
            .insert(String::from("super_"), super_ctor.clone());
    }
    JsValue::Undefined
}

fn identity(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    args.first().cloned().unwrap_or(JsValue::Undefined)
}

fn is_array(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(matches!(args.first(), Some(JsValue::Array(_))))
}

fn is_buffer(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(args.first().map(is_buffer_like).unwrap_or(false))
}

fn types_object() -> JsValue {
    let mut types = JsObject::new();
    types.set(
        String::from("isArrayBuffer"),
        native_fn("isArrayBuffer", false_fn),
    );
    types.set(
        String::from("isTypedArray"),
        native_fn("isTypedArray", false_fn),
    );
    types.set(String::from("isPromise"), native_fn("isPromise", false_fn));
    types.set(String::from("isProxy"), native_fn("isProxy", false_fn));
    types.set(String::from("isRegExp"), native_fn("isRegExp", regexp_fn));
    types.set(String::from("isDate"), native_fn("isDate", false_fn));
    object(types)
}

fn false_fn(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(false)
}

fn regexp_fn(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(
        args.first()
            .map(|value| value.to_js_string().starts_with('/'))
            .unwrap_or(false),
    )
}

fn inspect_value(value: &JsValue) -> String {
    match value {
        JsValue::Undefined => String::from("undefined"),
        JsValue::Null => String::from("null"),
        JsValue::Bool(value) => value.to_string(),
        JsValue::Number(value) => value.to_string(),
        JsValue::String(value) => value.clone(),
        JsValue::Array(array) => {
            let parts = array
                .borrow()
                .to_dense_vec()
                .iter()
                .map(inspect_value)
                .collect::<Vec<String>>();
            alloc::format!("[{}]", parts.join(", "))
        }
        JsValue::Object(obj) => {
            let keys = obj.borrow().keys();
            let mut parts = Vec::new();
            for key in keys {
                if key.starts_with("__") {
                    continue;
                }
                parts.push(alloc::format!(
                    "{}: {}",
                    key,
                    inspect_value(&value.get_property(&key))
                ));
            }
            alloc::format!("{{ {} }}", parts.join(", "))
        }
        JsValue::Function(func) => alloc::format!(
            "[Function{}{}]",
            if func.borrow().name.is_some() {
                ": "
            } else {
                ""
            },
            func.borrow().name.clone().unwrap_or_default()
        ),
        JsValue::BigInt(value) => alloc::format!("{}n", value.to_string_radix(10)),
        JsValue::Empty => String::new(),
    }
}
