//! Native classList host object.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::JsObject;
use libjs::vm::native_fn;

use super::{get_bridge, this_node_id, arg_string, DomMutation};

/// Create a classList object bound to the given node/class.
pub fn make_class_list(node_id: i64, initial_class: &str) -> JsValue {
    let mut obj = JsObject::new();
    obj.set(String::from("__nodeId"), JsValue::Number(node_id as f64));
    obj.set(String::from("__value"), JsValue::String(String::from(initial_class)));

    obj.set(String::from("add"), native_fn("add", cl_add));
    obj.set(String::from("remove"), native_fn("remove", cl_remove));
    obj.set(String::from("toggle"), native_fn("toggle", cl_toggle));
    obj.set(String::from("contains"), native_fn("contains", cl_contains));
    obj.set(String::from("item"), native_fn("item", cl_item));
    obj.set(String::from("toString"), native_fn("toString", cl_to_string));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// Read the current class string from the classList's __value.
fn get_class_value(vm: &Vm) -> String {
    if let JsValue::Object(obj) = &vm.current_this {
        if let Some(p) = obj.borrow().properties.get("__value") {
            return p.value.to_js_string();
        }
    }
    String::new()
}

/// Write class string back and record mutation.
fn set_class_value(vm: &mut Vm, new_val: &str) {
    let nid = this_node_id(vm);
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().set(String::from("__value"), JsValue::String(String::from(new_val)));
    }
    if let Some(bridge) = get_bridge(vm) {
        if nid >= 0 {
            bridge.mutations.push(DomMutation::SetAttribute {
                node_id: nid as usize,
                name: String::from("class"),
                value: String::from(new_val),
            });
        }
    }
}

fn cl_add(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let class = arg_string(args, 0);
    let current = get_class_value(vm);
    let has = current.split_whitespace().any(|c| c == class);
    if !has {
        let new_val = if current.is_empty() {
            class
        } else {
            alloc::format!("{} {}", current, class)
        };
        set_class_value(vm, &new_val);
    }
    JsValue::Undefined
}

fn cl_remove(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let class = arg_string(args, 0);
    let current = get_class_value(vm);
    let parts: Vec<&str> = current.split_whitespace().filter(|c| *c != class).collect();
    let new_val = parts.join(" ");
    set_class_value(vm, &new_val);
    JsValue::Undefined
}

fn cl_toggle(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let class = arg_string(args, 0);
    let force = args.get(1).cloned();
    let current = get_class_value(vm);
    let has = current.split_whitespace().any(|c| c == class);

    let should_add = match force {
        Some(JsValue::Bool(f)) => f,
        Some(JsValue::Undefined) | None => !has,
        _ => !has,
    };

    if should_add && !has {
        let new_val = if current.is_empty() {
            class
        } else {
            alloc::format!("{} {}", current, class)
        };
        set_class_value(vm, &new_val);
        JsValue::Bool(true)
    } else if !should_add && has {
        let parts: Vec<&str> = current.split_whitespace().filter(|c| *c != class).collect();
        set_class_value(vm, &parts.join(" "));
        JsValue::Bool(false)
    } else {
        JsValue::Bool(has)
    }
}

fn cl_contains(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let class = arg_string(args, 0);
    let current = get_class_value(vm);
    JsValue::Bool(current.split_whitespace().any(|c| c == class))
}

fn cl_item(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let idx = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let current = get_class_value(vm);
    let parts: Vec<&str> = current.split_whitespace().collect();
    match parts.get(idx) {
        Some(s) => JsValue::String(String::from(*s)),
        None => JsValue::Null,
    }
}

fn cl_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(get_class_value(vm))
}
