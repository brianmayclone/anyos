use alloc::format;
use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

const SOCKET_ID_KEY: &str = "__node_net_socket__";

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("createConnection"),
        native_fn("createConnection", create_connection),
    );
    module.set(
        String::from("connect"),
        native_fn("connect", create_connection),
    );
    module.set(String::from("isIP"), native_fn("isIP", is_ip));
    object(module)
}

fn create_connection(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let (host, port) = parse_connect_args(args);
    let mut ip = [0u8; 4];
    if anyos_std::net::dns(&host, &mut ip) == u32::MAX {
        vm.pending_exception = Some(vm.make_type_error(&format!("ENOTFOUND: {}", host)));
        return JsValue::Undefined;
    }
    let socket_id = anyos_std::net::tcp_connect(&ip, port, 10_000);
    if socket_id == u32::MAX {
        vm.pending_exception =
            Some(vm.make_type_error(&format!("ECONNREFUSED: {}:{}", host, port)));
        return JsValue::Undefined;
    }
    let socket = make_socket(socket_id);
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], socket.clone());
    }
    socket
}

fn parse_connect_args(args: &[JsValue]) -> (String, u16) {
    if let Some(options) = args.first() {
        if matches!(options, JsValue::Object(_)) {
            let host = options.get_property("host").to_js_string();
            let hostname = if host.is_empty() || host == "undefined" {
                String::from("127.0.0.1")
            } else {
                host
            };
            let port = options.get_property("port").to_number().max(0.0) as u16;
            return (hostname, port);
        }
    }
    let port = args
        .first()
        .map(|value| value.to_number().max(0.0) as u16)
        .unwrap_or(0);
    let host = args
        .get(1)
        .map(|value| value.to_js_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| String::from("127.0.0.1"));
    (host, port)
}

fn make_socket(socket_id: u32) -> JsValue {
    let mut socket = JsObject::new();
    socket.set(String::from("write"), native_fn("write", write));
    socket.set(String::from("end"), native_fn("end", end));
    socket.set(String::from("destroy"), native_fn("destroy", destroy));
    socket.set(String::from("read"), native_fn("read", read));
    socket.set_hidden(
        String::from(SOCKET_ID_KEY),
        JsValue::Number(socket_id as f64),
    );
    object(socket)
}

fn write(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let socket = vm.current_this.get_property(SOCKET_ID_KEY).to_number() as u32;
    let data = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    JsValue::Bool(anyos_std::net::tcp_send(socket, data.as_bytes()) != u32::MAX)
}

fn end(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !args.is_empty() {
        let _ = write(vm, args);
    }
    let socket = vm.current_this.get_property(SOCKET_ID_KEY).to_number() as u32;
    anyos_std::net::tcp_close(socket);
    JsValue::Undefined
}

fn destroy(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let socket = vm.current_this.get_property(SOCKET_ID_KEY).to_number() as u32;
    anyos_std::net::tcp_close(socket);
    JsValue::Undefined
}

fn read(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let socket = vm.current_this.get_property(SOCKET_ID_KEY).to_number() as u32;
    let mut data = alloc::vec![0u8; 16 * 1024];
    let n = anyos_std::net::tcp_recv(socket, &mut data);
    if n == u32::MAX || n == 0 {
        return JsValue::Null;
    }
    data.truncate(n as usize);
    JsValue::String(String::from_utf8_lossy(&data).into_owned())
}

fn is_ip(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let parts: alloc::vec::Vec<&str> = value.split('.').collect();
    JsValue::Number(
        if parts.len() == 4 && parts.iter().all(|part| part.parse::<u8>().is_ok()) {
            4.0
        } else {
            0.0
        },
    )
}
