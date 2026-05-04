use alloc::string::String;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_ctor_fn, native_fn, native_promise, Vm};

use super::util::object;

const EVENTS_KEY: &str = "__node_stream_events__";
const BUFFER_KEY: &str = "__node_stream_buffer__";
const PIPES_KEY: &str = "__node_stream_pipes__";

pub fn module() -> JsValue {
    let stream = stream_constructor("Stream");
    let readable = stream_constructor("Readable");
    let writable = stream_constructor("Writable");
    let duplex = stream_constructor("Duplex");
    let transform = stream_constructor("Transform");
    let pass_through = stream_constructor("PassThrough");

    let mut module = JsObject::new();
    module.set(String::from("Stream"), stream.clone());
    module.set(String::from("Readable"), readable);
    module.set(String::from("Writable"), writable);
    module.set(String::from("Duplex"), duplex);
    module.set(String::from("Transform"), transform);
    module.set(String::from("PassThrough"), pass_through);
    module.set(String::from("pipeline"), native_fn("pipeline", pipeline));
    module.set(String::from("finished"), native_fn("finished", finished));

    if let JsValue::Function(func) = &stream {
        for (key, value) in func.borrow().own_props.iter() {
            module.set(key.clone(), value.clone());
        }
    }

    object(module)
}

pub fn promises_module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("pipeline"),
        native_fn("pipeline", pipeline_promise),
    );
    module.set(
        String::from("finished"),
        native_fn("finished", finished_promise),
    );
    object(module)
}

pub fn consumers_module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("text"), native_fn("text", consumer_text));
    module.set(String::from("buffer"), native_fn("buffer", consumer_buffer));
    module.set(
        String::from("arrayBuffer"),
        native_fn("arrayBuffer", consumer_buffer),
    );
    module.set(String::from("json"), native_fn("json", consumer_json));
    object(module)
}

pub fn web_module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("ReadableStream"),
        stream_constructor("ReadableStream"),
    );
    module.set(
        String::from("WritableStream"),
        stream_constructor("WritableStream"),
    );
    module.set(
        String::from("TransformStream"),
        stream_constructor("TransformStream"),
    );
    object(module)
}

pub fn make_passthrough_stream() -> JsValue {
    let mut stream = JsObject::new();
    stream.set(String::from("readable"), JsValue::Bool(true));
    stream.set(String::from("writable"), JsValue::Bool(true));
    install_stream_methods(&mut stream);
    let stream = object(stream);
    ensure_stream_state(&stream);
    stream
}

fn stream_constructor(name: &str) -> JsValue {
    let ctor = native_ctor_fn(name, stream_init);
    if let JsValue::Function(func) = &ctor {
        let proto = func.borrow().prototype.clone();
        if let Some(proto) = proto {
            install_stream_methods(&mut proto.borrow_mut());
        }
    }
    ctor
}

fn install_stream_methods(proto: &mut JsObject) {
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
    proto.set(String::from("push"), native_fn("push", push));
    proto.set(String::from("read"), native_fn("read", read));
    proto.set(String::from("write"), native_fn("write", write));
    proto.set(String::from("end"), native_fn("end", end));
    proto.set(String::from("pipe"), native_fn("pipe", pipe));
    proto.set(String::from("destroy"), native_fn("destroy", destroy));
    proto.set(
        String::from("setEncoding"),
        native_fn("setEncoding", this_value),
    );
    proto.set(String::from("resume"), native_fn("resume", this_value));
    proto.set(String::from("pause"), native_fn("pause", this_value));
}

fn stream_init(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    ensure_stream_state(&vm.current_this);
    vm.current_this
        .set_property(String::from("readable"), JsValue::Bool(true));
    vm.current_this
        .set_property(String::from("writable"), JsValue::Bool(true));
    JsValue::Undefined
}

fn on(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    add_listener_native(vm, args, false)
}

fn once(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    add_listener_native(vm, args, true)
}

fn add_listener_native(vm: &mut Vm, args: &[JsValue], once: bool) -> JsValue {
    ensure_stream_state(&vm.current_this);
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
    add_listener(&vm.current_this, &event, listener, once);
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
    JsValue::Bool(emit_event(vm, &vm.current_this.clone(), &event, call_args))
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
    let listeners = JsValue::new_array(
        list.borrow()
            .to_dense_vec()
            .into_iter()
            .map(|entry| entry.get_property("listener"))
            .collect(),
    );
    listeners
}

fn listener_count(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(listeners(vm, args).get_property("length").to_number())
}

fn push(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ensure_stream_state(&vm.current_this);
    let chunk = args.first().cloned().unwrap_or(JsValue::Null);
    if matches!(chunk, JsValue::Null) {
        emit_event(vm, &vm.current_this.clone(), "end", &[]);
        return JsValue::Bool(false);
    }
    append_buffered(&vm.current_this, chunk.clone());
    emit_event(vm, &vm.current_this.clone(), "data", &[chunk.clone()]);
    forward_to_pipes(vm, &vm.current_this.clone(), chunk);
    JsValue::Bool(true)
}

fn read(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    ensure_stream_state(&vm.current_this);
    let buffered = vm.current_this.get_property(BUFFER_KEY);
    let JsValue::Array(array) = buffered else {
        return JsValue::Null;
    };
    let mut data = array.borrow().to_dense_vec();
    if data.is_empty() {
        return JsValue::Null;
    }
    let first = data.remove(0);
    vm.current_this
        .set_property(String::from(BUFFER_KEY), JsValue::new_array(data));
    first
}

fn write(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ensure_stream_state(&vm.current_this);
    let chunk = args.first().cloned().unwrap_or(JsValue::Undefined);
    emit_event(vm, &vm.current_this.clone(), "data", &[chunk.clone()]);
    forward_to_pipes(vm, &vm.current_this.clone(), chunk);
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], vm.current_this.clone());
    }
    JsValue::Bool(true)
}

fn end(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !args.is_empty() && !matches!(args[0], JsValue::Function(_)) {
        let _ = write(vm, &args[..1]);
    }
    emit_event(vm, &vm.current_this.clone(), "finish", &[]);
    emit_event(vm, &vm.current_this.clone(), "end", &[]);
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], vm.current_this.clone());
    }
    vm.current_this.clone()
}

fn pipe(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ensure_stream_state(&vm.current_this);
    let Some(destination) = args.first().cloned() else {
        return JsValue::Undefined;
    };
    let mut pipes = match vm.current_this.get_property(PIPES_KEY) {
        JsValue::Array(array) => array.borrow().to_dense_vec(),
        _ => Vec::new(),
    };
    pipes.push(destination.clone());
    vm.current_this
        .set_property(String::from(PIPES_KEY), JsValue::new_array(pipes));
    destination
}

fn destroy(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(error) = args.first() {
        if !matches!(error, JsValue::Undefined | JsValue::Null) {
            emit_event(vm, &vm.current_this.clone(), "error", &[error.clone()]);
        }
    }
    emit_event(vm, &vm.current_this.clone(), "close", &[]);
    vm.current_this.clone()
}

fn pipeline(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.len() < 2 {
        return JsValue::Undefined;
    }
    for idx in 0..args.len().saturating_sub(1) {
        let source = args[idx].clone();
        let destination = args[idx + 1].clone();
        let pipe = source.get_property("pipe");
        if matches!(pipe, JsValue::Function(_)) {
            vm.call_value(&pipe, &[destination], source);
        }
    }
    if let Some(callback) = args
        .iter()
        .rev()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], JsValue::Undefined);
    }
    args.last().cloned().unwrap_or(JsValue::Undefined)
}

fn pipeline_promise(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let result = pipeline(vm, args);
    if let Some(err) = vm
        .pending_exception
        .take()
        .or_else(|| vm.last_exception.take())
    {
        native_promise::promise_reject(vm, &[err])
    } else {
        native_promise::promise_resolve(vm, &[result])
    }
}

fn finished(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(
            callback,
            &[],
            args.first().cloned().unwrap_or(JsValue::Undefined),
        );
    }
    JsValue::Undefined
}

fn finished_promise(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let result = finished(vm, args);
    native_promise::promise_resolve(vm, &[result])
}

fn consumer_text(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let data = stream_data(args.first().unwrap_or(&JsValue::Undefined));
    native_promise::promise_resolve(vm, &[JsValue::String(data)])
}

fn consumer_buffer(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let data = stream_data(args.first().unwrap_or(&JsValue::Undefined));
    native_promise::promise_resolve(vm, &[super::buffer::buffer_from_bytes(data.into_bytes())])
}

fn consumer_json(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let data = stream_data(args.first().unwrap_or(&JsValue::Undefined));
    native_promise::promise_resolve(vm, &[JsValue::String(data)])
}

fn this_value(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

fn ensure_stream_state(stream: &JsValue) {
    ensure_events_object(stream);
    if !matches!(stream.get_property(BUFFER_KEY), JsValue::Array(_)) {
        stream.set_property(String::from(BUFFER_KEY), JsValue::new_array(Vec::new()));
    }
    if !matches!(stream.get_property(PIPES_KEY), JsValue::Array(_)) {
        stream.set_property(String::from(PIPES_KEY), JsValue::new_array(Vec::new()));
    }
}

fn append_buffered(stream: &JsValue, value: JsValue) {
    let mut data = match stream.get_property(BUFFER_KEY) {
        JsValue::Array(array) => array.borrow().to_dense_vec(),
        _ => Vec::new(),
    };
    data.push(value);
    stream.set_property(String::from(BUFFER_KEY), JsValue::new_array(data));
}

fn stream_data(stream: &JsValue) -> String {
    match stream.get_property(BUFFER_KEY) {
        JsValue::Array(array) => array
            .borrow()
            .to_dense_vec()
            .into_iter()
            .map(|value| value.to_js_string())
            .collect::<Vec<String>>()
            .join(""),
        _ => stream.to_js_string(),
    }
}

fn forward_to_pipes(vm: &mut Vm, stream: &JsValue, chunk: JsValue) {
    let pipes = match stream.get_property(PIPES_KEY) {
        JsValue::Array(array) => array.borrow().to_dense_vec(),
        _ => Vec::new(),
    };
    for destination in pipes {
        let write = destination.get_property("write");
        if matches!(write, JsValue::Function(_)) {
            vm.call_value(&write, &[chunk.clone()], destination);
        }
    }
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

fn emit_event(vm: &mut Vm, this: &JsValue, event: &str, args: &[JsValue]) -> bool {
    let events = ensure_events_object(this);
    let list = events.get_property(event);
    let JsValue::Array(list) = list else {
        return false;
    };
    let entries = list.borrow().to_dense_vec();
    if entries.is_empty() {
        return false;
    }
    let mut kept = Vec::new();
    for entry in entries {
        let listener = entry.get_property("listener");
        let once = matches!(entry.get_property("once"), JsValue::Bool(true));
        if matches!(listener, JsValue::Function(_)) {
            vm.call_value(&listener, args, this.clone());
            if vm.last_exception.is_some() {
                return true;
            }
        }
        if !once {
            kept.push(entry);
        }
    }
    events.set_property(String::from(event), JsValue::new_array(kept));
    true
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
