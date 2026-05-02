use alloc::format;
use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

const SOCKET_ID_KEY: &str = "__node_net_socket__";
const SOCKET_EVENTS_KEY: &str = "__node_net_events__";
const SERVER_LISTENER_KEY: &str = "__node_net_listener__";
const SERVER_HANDLER_KEY: &str = "__node_net_connection_handler__";
const SERVER_PORT_KEY: &str = "__node_net_port__";
const NET_SERVERS_KEY: &str = "__node_net_servers__";

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("createServer"),
        native_fn("createServer", create_server),
    );
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

pub fn poll_servers(vm: &mut Vm) -> usize {
    let mut handled = 0usize;
    let servers = net_servers(vm);
    for server in servers {
        let listener_id = server.get_property(SERVER_LISTENER_KEY).to_number() as u32;
        if listener_id == 0 || listener_id == u32::MAX {
            continue;
        }
        let mut listener = tcp_handle(listener_id, libuv::UvHandleKind::TcpServer);
        loop {
            let mut client = libuv::UvTcp::new();
            if libuv::tcp_accept_nowait(&mut listener, &mut client) != 0 {
                break;
            }
            let socket = make_socket_with_peer(client.socket_id, client.peer_ip, client.peer_port);
            let handler = server.get_property(SERVER_HANDLER_KEY);
            if matches!(handler, JsValue::Function(_)) {
                vm.call_value(&handler, &[socket], server.clone());
                handled += 1;
            } else {
                libuv::tcp_close(&mut client);
            }
        }
    }
    handled
}

pub fn has_active_servers(vm: &Vm) -> bool {
    net_servers(vm)
        .into_iter()
        .any(|server| server.get_property(SERVER_LISTENER_KEY).to_number() as u32 != 0)
}

fn create_server(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let handler = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
        .cloned()
        .unwrap_or(JsValue::Undefined);
    let mut server = JsObject::new();
    server.set(String::from("listen"), native_fn("listen", listen));
    server.set(String::from("close"), native_fn("close", close));
    server.set(String::from("address"), native_fn("address", address));
    server.set(String::from("on"), native_fn("on", on));
    server.set_hidden(String::from(SERVER_HANDLER_KEY), handler);
    server.set_hidden(String::from(SERVER_LISTENER_KEY), JsValue::Number(0.0));
    server.set_hidden(String::from(SERVER_PORT_KEY), JsValue::Number(0.0));
    object(server)
}

fn create_connection(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let (host, port) = parse_connect_args(args);
    let mut handle = libuv::UvTcp::new();
    let rc = libuv::tcp_connect_host(&mut handle, &host, port, 10_000);
    if rc == libuv::UV_ENOTFOUND {
        vm.pending_exception = Some(vm.make_type_error(&format!("ENOTFOUND: {}", host)));
        return JsValue::Undefined;
    }
    if rc != 0 {
        vm.pending_exception =
            Some(vm.make_type_error(&format!("ECONNREFUSED: {}:{}", host, port)));
        return JsValue::Undefined;
    }
    let socket = make_socket_with_peer(handle.socket_id, handle.peer_ip, handle.peer_port);
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], socket.clone());
    }
    emit_socket_event(vm, &socket, "connect", &[]);
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

fn make_socket_with_peer(socket_id: u32, peer_ip: [u8; 4], peer_port: u16) -> JsValue {
    let mut socket = JsObject::new();
    socket.set(String::from("write"), native_fn("write", write));
    socket.set(String::from("end"), native_fn("end", end));
    socket.set(String::from("destroy"), native_fn("destroy", destroy));
    socket.set(String::from("read"), native_fn("read", read));
    socket.set(String::from("on"), native_fn("on", socket_on));
    socket.set(
        String::from("addListener"),
        native_fn("addListener", socket_on),
    );
    socket.set(String::from("once"), native_fn("once", socket_once));
    socket.set(String::from("emit"), native_fn("emit", socket_emit));
    socket.set(
        String::from("remoteAddress"),
        JsValue::String(format!(
            "{}.{}.{}.{}",
            peer_ip[0], peer_ip[1], peer_ip[2], peer_ip[3]
        )),
    );
    socket.set(
        String::from("remotePort"),
        JsValue::Number(peer_port as f64),
    );
    socket.set_hidden(
        String::from(SOCKET_ID_KEY),
        JsValue::Number(socket_id as f64),
    );
    socket.set_hidden(String::from(SOCKET_EVENTS_KEY), JsValue::new_object());
    object(socket)
}

fn listen(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let port = args
        .first()
        .map(|value| value.to_number().max(0.0) as u16)
        .unwrap_or(0);
    let mut listener = libuv::UvTcp::new();
    if libuv::tcp_listen(&mut listener, port, 128) != 0 {
        vm.pending_exception = Some(vm.make_type_error(&format!("EADDRINUSE: {}", port)));
        return JsValue::Undefined;
    }
    vm.current_this.set_property(
        String::from(SERVER_LISTENER_KEY),
        JsValue::Number(listener.socket_id as f64),
    );
    vm.current_this
        .set_property(String::from(SERVER_PORT_KEY), JsValue::Number(port as f64));
    register_server(vm, vm.current_this.clone());
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], vm.current_this.clone());
    }
    vm.current_this.clone()
}

fn close(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let listener_id = vm
        .current_this
        .get_property(SERVER_LISTENER_KEY)
        .to_number() as u32;
    if listener_id != 0 && listener_id != u32::MAX {
        let mut listener = tcp_handle(listener_id, libuv::UvHandleKind::TcpServer);
        libuv::tcp_close(&mut listener);
    }
    vm.current_this
        .set_property(String::from(SERVER_LISTENER_KEY), JsValue::Number(0.0));
    JsValue::Undefined
}

fn address(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let out = JsValue::new_object();
    out.set_property(
        String::from("address"),
        JsValue::String(String::from("127.0.0.1")),
    );
    out.set_property(
        String::from("family"),
        JsValue::String(String::from("IPv4")),
    );
    out.set_property(
        String::from("port"),
        vm.current_this.get_property(SERVER_PORT_KEY),
    );
    out
}

fn on(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    if event == "connection" {
        if let Some(listener) = args.get(1).cloned() {
            vm.current_this
                .set_property(String::from(SERVER_HANDLER_KEY), listener);
        }
    }
    vm.current_this.clone()
}

fn write(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut socket = tcp_socket_from_this(vm);
    let data = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    JsValue::Bool(libuv::tcp_write(&mut socket, data.as_bytes()) >= 0)
}

fn end(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !args.is_empty() {
        let _ = write(vm, args);
    }
    let socket_obj = vm.current_this.clone();
    let mut socket = tcp_socket_from_this(vm);
    libuv::tcp_close(&mut socket);
    emit_socket_event(vm, &socket_obj, "end", &[]);
    emit_socket_event(vm, &socket_obj, "close", &[]);
    JsValue::Undefined
}

fn destroy(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let socket_obj = vm.current_this.clone();
    let mut socket = tcp_socket_from_this(vm);
    libuv::tcp_close(&mut socket);
    emit_socket_event(vm, &socket_obj, "close", &[]);
    JsValue::Undefined
}

fn read(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let mut socket = tcp_socket_from_this(vm);
    let mut data = alloc::vec![0u8; 16 * 1024];
    let n = libuv::tcp_read(&mut socket, &mut data);
    if n <= 0 {
        if n == libuv::UV_EOF {
            emit_socket_event(vm, &vm.current_this.clone(), "end", &[]);
        }
        return JsValue::Null;
    }
    data.truncate(n as usize);
    JsValue::String(String::from_utf8_lossy(&data).into_owned())
}

fn socket_on(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    register_socket_listener(vm, args, false)
}

fn socket_once(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    register_socket_listener(vm, args, true)
}

fn register_socket_listener(vm: &mut Vm, args: &[JsValue], once: bool) -> JsValue {
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
    add_socket_listener(&vm.current_this, &event, listener, once);
    if event == "data" {
        emit_pending_socket_data(vm, vm.current_this.clone());
    } else if event == "connect" {
        emit_socket_event(vm, &vm.current_this.clone(), "connect", &[]);
    }
    vm.current_this.clone()
}

fn socket_emit(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let call_args = if args.len() > 1 { &args[1..] } else { &[] };
    JsValue::Bool(emit_socket_event(
        vm,
        &vm.current_this.clone(),
        &event,
        call_args,
    ))
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

fn tcp_socket_from_this(vm: &Vm) -> libuv::UvTcp {
    let socket_id = vm.current_this.get_property(SOCKET_ID_KEY).to_number() as u32;
    tcp_handle(socket_id, libuv::UvHandleKind::Tcp)
}

fn tcp_handle(socket_id: u32, kind: libuv::UvHandleKind) -> libuv::UvTcp {
    let mut handle = libuv::UvTcp::new();
    handle.socket_id = socket_id;
    handle.kind = kind;
    handle.active = socket_id != u32::MAX;
    handle
}

fn register_server(vm: &mut Vm, server: JsValue) {
    let mut servers = net_servers(vm);
    if !servers.iter().any(|candidate| candidate.strict_eq(&server)) {
        servers.push(server);
    }
    vm.globals
        .borrow_mut()
        .set_hidden(String::from(NET_SERVERS_KEY), JsValue::new_array(servers));
}

fn net_servers(vm: &Vm) -> alloc::vec::Vec<JsValue> {
    match vm.globals.borrow().get(NET_SERVERS_KEY) {
        JsValue::Array(array) => array.borrow().to_dense_vec(),
        _ => alloc::vec::Vec::new(),
    }
}

fn emit_pending_socket_data(vm: &mut Vm, socket: JsValue) {
    let socket_id = socket.get_property(SOCKET_ID_KEY).to_number() as u32;
    let mut handle = tcp_handle(socket_id, libuv::UvHandleKind::Tcp);
    let mut data = alloc::vec![0u8; 16 * 1024];
    let n = libuv::tcp_read(&mut handle, &mut data);
    if n > 0 {
        data.truncate(n as usize);
        let chunk = JsValue::String(String::from_utf8_lossy(&data).into_owned());
        emit_socket_event(vm, &socket, "data", &[chunk]);
    } else if n == libuv::UV_EOF {
        emit_socket_event(vm, &socket, "end", &[]);
    }
}

fn add_socket_listener(socket: &JsValue, event: &str, listener: JsValue, once: bool) {
    let events = socket.get_property(SOCKET_EVENTS_KEY);
    let mut current = match events.get_property(event) {
        JsValue::Array(array) => array.borrow().to_dense_vec(),
        _ => alloc::vec::Vec::new(),
    };
    let entry = JsValue::new_object();
    entry.set_property(String::from("listener"), listener);
    entry.set_property(String::from("once"), JsValue::Bool(once));
    current.push(entry);
    events.set_property(String::from(event), JsValue::new_array(current));
}

fn emit_socket_event(vm: &mut Vm, socket: &JsValue, event: &str, args: &[JsValue]) -> bool {
    let events = socket.get_property(SOCKET_EVENTS_KEY);
    let list = events.get_property(event);
    let JsValue::Array(list) = list else {
        return false;
    };
    let entries = list.borrow().to_dense_vec();
    if entries.is_empty() {
        return false;
    }
    let mut kept = alloc::vec::Vec::new();
    for entry in entries {
        let listener = entry.get_property("listener");
        let once = matches!(entry.get_property("once"), JsValue::Bool(true));
        if matches!(listener, JsValue::Function(_)) {
            vm.call_value(&listener, args, socket.clone());
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
