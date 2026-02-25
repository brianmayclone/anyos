//! Native localStorage / sessionStorage host objects.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::JsObject;
use libjs::vm::native_fn;

use super::arg_string;

/// Create a storage object (localStorage or sessionStorage).
/// Data is stored in a `__data` sub-object on the storage itself.
pub fn make_storage() -> JsValue {
    let mut obj = JsObject::new();
    obj.set(String::from("__data"), JsValue::Object(Rc::new(RefCell::new(JsObject::new()))));

    obj.set(String::from("getItem"), native_fn("getItem", storage_get_item));
    obj.set(String::from("setItem"), native_fn("setItem", storage_set_item));
    obj.set(String::from("removeItem"), native_fn("removeItem", storage_remove_item));
    obj.set(String::from("clear"), native_fn("clear", storage_clear));
    obj.set(String::from("key"), native_fn("key", storage_key));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn get_data(vm: &Vm) -> Option<JsValue> {
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(p) = o.properties.get("__data") {
            return Some(p.value.clone());
        }
    }
    None
}

fn storage_get_item(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = arg_string(args, 0);
    if let Some(data) = get_data(vm) {
        let val = data.get_property(&key);
        if matches!(val, JsValue::Undefined) { return JsValue::Null; }
        return val;
    }
    JsValue::Null
}

fn storage_set_item(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = arg_string(args, 0);
    let val = arg_string(args, 1);
    if let Some(data) = get_data(vm) {
        data.set_property(key, JsValue::String(val));
    }
    JsValue::Undefined
}

fn storage_remove_item(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = arg_string(args, 0);
    if let Some(data) = get_data(vm) {
        if let JsValue::Object(obj) = &data {
            obj.borrow_mut().properties.remove(&key);
        }
    }
    JsValue::Undefined
}

fn storage_clear(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().set(String::from("__data"), JsValue::Object(Rc::new(RefCell::new(JsObject::new()))));
    }
    JsValue::Undefined
}

fn storage_key(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let idx = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    if let Some(data) = get_data(vm) {
        if let JsValue::Object(obj) = &data {
            let o = obj.borrow();
            let keys: Vec<&String> = o.properties.keys().collect();
            if let Some(k) = keys.get(idx) {
                return JsValue::String((*k).clone());
            }
        }
    }
    JsValue::Null
}
