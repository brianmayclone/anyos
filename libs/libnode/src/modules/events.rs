use alloc::string::String;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_ctor_fn, native_fn, Vm};

use super::util::object;

const EVENTS_KEY: &str = "__node_events__";

pub fn module() -> JsValue {
    let event_emitter = event_emitter_constructor();
    let mut module = JsObject::new();
    module.set(String::from("EventEmitter"), event_emitter);
    object(module)
}

fn event_emitter_constructor() -> JsValue {
    let ctor = native_ctor_fn("EventEmitter", event_emitter_init);
    if let JsValue::Function(func) = &ctor {
        let proto = func.borrow().prototype.clone();
        if let Some(proto) = proto {
            let mut proto = proto.borrow_mut();
            proto.set(String::from("on"), native_fn("on", on));
            proto.set(String::from("addListener"), native_fn("addListener", on));
            proto.set(String::from("once"), native_fn("once", once));
            proto.set(String::from("off"), native_fn("off", off));
            proto.set(
                String::from("removeListener"),
                native_fn("removeListener", off),
            );
            proto.set(String::from("emit"), native_fn("emit", emit));
            proto.set(String::from("listeners"), native_fn("listeners", listeners));
            proto.set(
                String::from("listenerCount"),
                native_fn("listenerCount", listener_count),
            );
        }
    }
    ctor
}

fn event_emitter_init(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    ensure_events_object(&vm.current_this);
    JsValue::Undefined
}

fn on(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let Some(listener) = args.get(1).cloned() else {
        vm.pending_exception = Some(vm.make_type_error("listener must be a function"));
        return JsValue::Undefined;
    };
    if !matches!(listener, JsValue::Function(_)) {
        vm.pending_exception = Some(vm.make_type_error("listener must be a function"));
        return JsValue::Undefined;
    }
    add_listener(&vm.current_this, &event, listener, false);
    vm.current_this.clone()
}

fn once(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let Some(listener) = args.get(1).cloned() else {
        vm.pending_exception = Some(vm.make_type_error("listener must be a function"));
        return JsValue::Undefined;
    };
    if !matches!(listener, JsValue::Function(_)) {
        vm.pending_exception = Some(vm.make_type_error("listener must be a function"));
        return JsValue::Undefined;
    }
    add_listener(&vm.current_this, &event, listener, true);
    vm.current_this.clone()
}

fn off(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let Some(listener) = args.get(1) else {
        return vm.current_this.clone();
    };
    let events = ensure_events_object(&vm.current_this);
    let list = events.get_property(&event);
    let JsValue::Array(list) = list else {
        return vm.current_this.clone();
    };
    let kept: Vec<JsValue> = list
        .borrow()
        .to_dense_vec()
        .into_iter()
        .filter(|entry| !same_listener(&entry.get_property("listener"), listener))
        .collect();
    events.set_property(event, JsValue::new_array(kept));
    vm.current_this.clone()
}

fn emit(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let call_args = if args.len() > 1 { &args[1..] } else { &[] };
    let events = ensure_events_object(&vm.current_this);
    let list = events.get_property(&event);
    let JsValue::Array(list) = list else {
        return JsValue::Bool(false);
    };
    let entries = list.borrow().to_dense_vec();
    if entries.is_empty() {
        return JsValue::Bool(false);
    }
    let mut kept = Vec::new();
    for entry in entries {
        let listener = entry.get_property("listener");
        let once = matches!(entry.get_property("once"), JsValue::Bool(true));
        if matches!(listener, JsValue::Function(_)) {
            vm.call_value(&listener, call_args, vm.current_this.clone());
            if vm.last_exception.is_some() {
                return JsValue::Bool(true);
            }
        }
        if !once {
            kept.push(entry);
        }
    }
    events.set_property(event, JsValue::new_array(kept));
    JsValue::Bool(true)
}

fn listeners(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let events = ensure_events_object(&vm.current_this);
    let list = events.get_property(&event);
    let JsValue::Array(list) = list else {
        return JsValue::new_array(Vec::new());
    };
    let values = list
        .borrow()
        .to_dense_vec()
        .into_iter()
        .map(|entry| entry.get_property("listener"))
        .collect();
    JsValue::new_array(values)
}

fn listener_count(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let listeners = listeners(vm, args);
    JsValue::Number(listeners.get_property("length").to_number())
}

fn add_listener(this: &JsValue, event: &str, listener: JsValue, once: bool) {
    let events = ensure_events_object(this);
    let mut current = match events.get_property(event) {
        JsValue::Array(array) => array.borrow().to_dense_vec(),
        _ => Vec::new(),
    };
    let entry = JsValue::new_object();
    entry.set_property(String::from("listener"), listener);
    entry.set_property(String::from("once"), JsValue::Bool(once));
    current.push(entry);
    events.set_property(String::from(event), JsValue::new_array(current));
}

fn ensure_events_object(this: &JsValue) -> JsValue {
    let existing = this.get_property(EVENTS_KEY);
    if matches!(existing, JsValue::Object(_)) {
        return existing;
    }
    let events = JsValue::new_object();
    this.set_property(String::from(EVENTS_KEY), events.clone());
    events
}

fn same_listener(left: &JsValue, right: &JsValue) -> bool {
    match (left, right) {
        (JsValue::Function(a), JsValue::Function(b)) => alloc::rc::Rc::ptr_eq(a, b),
        _ => false,
    }
}
