use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::util::object;

const SERVER_LISTENER_KEY: &str = "__node_http_listener__";
const SERVER_HANDLER_KEY: &str = "__node_http_handler__";
const SERVER_PORT_KEY: &str = "__node_http_port__";
const RESPONSE_SOCKET_KEY: &str = "__node_http_socket__";
const RESPONSE_HEADERS_KEY: &str = "__node_http_headers__";
const HTTP_SERVERS_KEY: &str = "__node_http_servers__";

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("IncomingMessage"),
        constructor_with_prototype("IncomingMessage", incoming_message_prototype()),
    );
    module.set(
        String::from("ServerResponse"),
        constructor_with_prototype("ServerResponse", server_response_prototype()),
    );
    module.set(
        String::from("createServer"),
        native_fn("createServer", create_server),
    );
    module.set(String::from("get"), native_fn("get", get));
    module.set(String::from("request"), native_fn("request", request));
    object(module)
}

fn constructor_with_prototype(name: &str, prototype: JsValue) -> JsValue {
    let ctor = native_fn(name, noop_constructor);
    ctor.set_property(String::from("prototype"), prototype);
    ctor
}

fn noop_constructor(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

fn incoming_message_prototype() -> JsValue {
    let mut proto = JsObject::new();
    install_stream_methods(&mut proto);
    object(proto)
}

fn server_response_prototype() -> JsValue {
    let mut proto = JsObject::new();
    install_stream_methods(&mut proto);
    proto.set(
        String::from("setHeader"),
        native_fn("setHeader", set_header),
    );
    proto.set(
        String::from("getHeader"),
        native_fn("getHeader", get_header),
    );
    proto.set(
        String::from("removeHeader"),
        native_fn("removeHeader", remove_header),
    );
    proto.set(
        String::from("writeHead"),
        native_fn("writeHead", write_head),
    );
    proto.set(String::from("write"), native_fn("write", write));
    proto.set(String::from("end"), native_fn("end", end));
    object(proto)
}

fn install_stream_methods(proto: &mut JsObject) {
    proto.set(String::from("on"), native_fn("on", stream_on));
    proto.set(String::from("once"), native_fn("once", stream_on));
    proto.set(String::from("emit"), native_fn("emit", stream_emit));
    proto.set(String::from("listeners"), native_fn("listeners", stream_listeners));
    proto.set(
        String::from("removeListener"),
        native_fn("removeListener", stream_remove_listener),
    );
    proto.set(String::from("unpipe"), native_fn("unpipe", stream_unpipe));
}

fn stream_on(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

fn stream_emit(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(false)
}

fn stream_listeners(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::new_array(Vec::new())
}

fn stream_remove_listener(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

fn stream_unpipe(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

pub fn poll_servers(vm: &mut Vm) -> usize {
    let mut handled = 0usize;
    let servers = http_servers(vm);
    for server in servers {
        let listener = server.get_property(SERVER_LISTENER_KEY).to_number() as u32;
        if listener == 0 || listener == u32::MAX {
            continue;
        }
        let mut listener_handle = tcp_handle(listener, libuv::UvHandleKind::TcpServer);
        loop {
            let mut client = libuv::UvTcp::new();
            if libuv::tcp_accept_nowait(&mut listener_handle, &mut client) != 0 {
                break;
            }
            if handle_socket(vm, server.clone(), client.socket_id) {
                handled += 1;
            } else {
                libuv::tcp_close(&mut client);
            }
        }
    }
    handled
}

pub fn has_active_servers(vm: &Vm) -> bool {
    http_servers(vm)
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

fn get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    http_client_request(vm, args, true)
}

fn request(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    http_client_request(vm, args, false)
}

fn http_client_request(vm: &mut Vm, args: &[JsValue], auto_end: bool) -> JsValue {
    let url = request_url(args.first());
    let callback = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
        .cloned();
    let request = make_client_request(&url, callback);
    if auto_end {
        perform_client_request(vm, &request);
    }
    request
}

fn request_url(input: Option<&JsValue>) -> String {
    let Some(input) = input else {
        return String::new();
    };
    if matches!(input, JsValue::Object(_)) {
        let protocol = input.get_property("protocol").to_js_string();
        let host_prop = input.get_property("hostname");
        let host = if matches!(host_prop, JsValue::Undefined) {
            input.get_property("host").to_js_string()
        } else {
            host_prop.to_js_string()
        };
        let path = input.get_property("path").to_js_string();
        let scheme = if protocol.is_empty() || protocol == "undefined" {
            "http:"
        } else {
            protocol.as_str()
        };
        let path = if path.is_empty() || path == "undefined" {
            "/"
        } else {
            path.as_str()
        };
        return format!("//{}{}", host, path).replacen("//", &format!("{}//", scheme), 1);
    }
    input.to_js_string()
}

fn make_client_request(url: &str, callback: Option<JsValue>) -> JsValue {
    let mut request = JsObject::new();
    request.set(String::from("end"), native_fn("end", client_end));
    request.set(String::from("write"), native_fn("write", client_write));
    request.set_hidden(
        String::from("__node_http_client_url__"),
        JsValue::String(String::from(url)),
    );
    request.set_hidden(
        String::from("__node_http_client_callback__"),
        callback.unwrap_or(JsValue::Undefined),
    );
    request.set_hidden(
        String::from("__node_http_client_body__"),
        JsValue::String(String::new()),
    );
    object(request)
}

fn client_write(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut body = vm
        .current_this
        .get_property("__node_http_client_body__")
        .to_js_string();
    if let Some(chunk) = args.first() {
        body.push_str(&chunk.to_js_string());
    }
    vm.current_this.set_property(
        String::from("__node_http_client_body__"),
        JsValue::String(body),
    );
    JsValue::Bool(true)
}

fn client_end(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !args.is_empty() {
        let _ = client_write(vm, args);
    }
    let request = vm.current_this.clone();
    perform_client_request(vm, &request);
    JsValue::Undefined
}

fn perform_client_request(vm: &mut Vm, request: &JsValue) {
    let url = request
        .get_property("__node_http_client_url__")
        .to_js_string();
    let body = request
        .get_property("__node_http_client_body__")
        .to_js_string();
    let data = if body.is_empty() {
        libhttp_client::get(&url)
    } else {
        libhttp_client::post(&url, body.as_bytes(), "application/octet-stream")
    };
    let response = make_client_response(data.unwrap_or_default());
    let callback = request.get_property("__node_http_client_callback__");
    if matches!(callback, JsValue::Function(_)) {
        vm.call_value(&callback, &[response], request.clone());
    }
}

fn make_client_response(body: Vec<u8>) -> JsValue {
    let mut response = JsObject::new();
    response.set(String::from("statusCode"), JsValue::Number(200.0));
    response.set(String::from("headers"), JsValue::new_object());
    response.set(
        String::from("setEncoding"),
        native_fn("setEncoding", noop_this),
    );
    response.set(String::from("on"), native_fn("on", client_response_on));
    response.set_hidden(
        String::from("__node_http_client_body__"),
        JsValue::String(String::from_utf8_lossy(&body).into_owned()),
    );
    object(response)
}

fn noop_this(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

fn client_response_on(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let Some(callback) = args.get(1) else {
        return vm.current_this.clone();
    };
    if !matches!(callback, JsValue::Function(_)) {
        return vm.current_this.clone();
    }
    if event == "data" {
        let body = vm.current_this.get_property("__node_http_client_body__");
        if !body.to_js_string().is_empty() {
            vm.call_value(callback, &[body], vm.current_this.clone());
        }
    } else if event == "end" {
        vm.call_value(callback, &[], vm.current_this.clone());
    }
    vm.current_this.clone()
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
    let listener = vm
        .current_this
        .get_property(SERVER_LISTENER_KEY)
        .to_number() as u32;
    if listener != 0 && listener != u32::MAX {
        let mut listener_handle = tcp_handle(listener, libuv::UvHandleKind::TcpServer);
        libuv::tcp_close(&mut listener_handle);
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
    if event == "request" {
        if let Some(listener) = args.get(1).cloned() {
            vm.current_this
                .set_property(String::from(SERVER_HANDLER_KEY), listener);
        }
    }
    vm.current_this.clone()
}

fn handle_socket(vm: &mut Vm, server: JsValue, socket: u32) -> bool {
    let Some(request) = read_http_request(socket) else {
        return false;
    };
    let req = make_request(&request);
    let res = make_response(socket);
    let handler = server.get_property(SERVER_HANDLER_KEY);
    if !matches!(handler, JsValue::Function(_)) {
        send_simple_response(socket, 404, "Not Found", b"");
        return true;
    }
    vm.call_value(&handler, &[req, res.clone()], server);
    drain_request_tasks(vm, &res);
    if vm.last_exception.is_some() {
        #[cfg(feature = "host")]
        if std::env::var_os("LIBNODE_DEBUG_HTTP").is_some() {
            if let Some(exc) = vm.last_exception.as_ref() {
                std::eprintln!(
                    "[libnode-http] handler exception: {} message={} stack={}",
                    exc.to_js_string(),
                    exc.get_property("message").to_js_string(),
                    exc.get_property("stack").to_js_string()
                );
            }
        }
        send_simple_response(
            socket,
            500,
            "Internal Server Error",
            b"Internal Server Error",
        );
        return true;
    }
    if !res.get_property("__node_http_sent__").to_boolean() {
        response_end_with_value(res, JsValue::Undefined);
    }
    true
}

fn drain_request_tasks(vm: &mut Vm, res: &JsValue) {
    for _ in 0..100 {
        vm.drain_microtasks();
        if res.get_property("__node_http_sent__").to_boolean() {
            break;
        }
        if !vm.event_loop.has_pending_timers() {
            break;
        }
        vm.tick(1);
        if vm.last_exception.is_some() {
            break;
        }
    }
}

fn read_http_request(socket: u32) -> Option<HttpRequest> {
    let mut socket = tcp_handle(socket, libuv::UvHandleKind::Tcp);
    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    for _ in 0..128 {
        let n = libuv::tcp_read(&mut socket, &mut buf);
        if n == libuv::UV_EAGAIN {
            anyos_std::process::sleep(1);
            continue;
        }
        if n <= 0 {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
        if find_header_end(&data).is_some() {
            break;
        }
    }
    let header_end = find_header_end(&data)?;
    let header_text = core::str::from_utf8(&data[..header_end]).ok()?;
    let mut lines = header_text.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let url = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((lower_ascii(name.trim()), value.trim().to_string()));
        }
    }
    Some(HttpRequest {
        method,
        url,
        headers,
    })
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 2)
}

fn make_request(request: &HttpRequest) -> JsValue {
    let req = JsValue::new_object();
    req.set_property(
        String::from("method"),
        JsValue::String(request.method.clone()),
    );
    req.set_property(String::from("url"), JsValue::String(request.url.clone()));
    req.set_property(
        String::from("originalUrl"),
        JsValue::String(request.url.clone()),
    );
    req.set_property(String::from("headers"), headers_object(&request.headers));
    req
}

fn headers_object(headers: &[(String, String)]) -> JsValue {
    let out = JsValue::new_object();
    for (name, value) in headers {
        out.set_property(name.clone(), JsValue::String(value.clone()));
    }
    out
}

fn make_response(socket: u32) -> JsValue {
    let mut res = JsObject::new();
    res.set(String::from("statusCode"), JsValue::Number(200.0));
    res.set(
        String::from("setHeader"),
        native_fn("setHeader", set_header),
    );
    res.set(
        String::from("getHeader"),
        native_fn("getHeader", get_header),
    );
    res.set(
        String::from("writeHead"),
        native_fn("writeHead", write_head),
    );
    res.set(String::from("write"), native_fn("write", write));
    res.set(String::from("end"), native_fn("end", end));
    res.set_hidden(
        String::from(RESPONSE_SOCKET_KEY),
        JsValue::Number(socket as f64),
    );
    res.set_hidden(String::from(RESPONSE_HEADERS_KEY), JsValue::new_object());
    object(res)
}

fn set_header(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args
        .first()
        .map(|value| lower_ascii(&value.to_js_string()))
        .unwrap_or_default();
    let value = args
        .get(1)
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let headers = vm.current_this.get_property(RESPONSE_HEADERS_KEY);
    headers.set_property(name, JsValue::String(value));
    vm.current_this.clone()
}

fn get_header(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args
        .first()
        .map(|value| lower_ascii(&value.to_js_string()))
        .unwrap_or_default();
    vm.current_this
        .get_property(RESPONSE_HEADERS_KEY)
        .get_property(&name)
}

fn remove_header(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args
        .first()
        .map(|value| lower_ascii(&value.to_js_string()))
        .unwrap_or_default();
    let headers = vm.current_this.get_property(RESPONSE_HEADERS_KEY);
    headers.delete_property(&name);
    vm.current_this.clone()
}

fn write_head(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(status) = args.first() {
        vm.current_this.set_property(
            String::from("statusCode"),
            JsValue::Number(status.to_number()),
        );
    }
    if let Some(headers) = args.get(1) {
        copy_headers(headers, &vm.current_this.get_property(RESPONSE_HEADERS_KEY));
    }
    vm.current_this.clone()
}

fn write(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let chunk = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let body = vm.current_this.get_property("__node_http_body__");
    let mut text = body.to_js_string();
    if matches!(body, JsValue::Undefined) {
        text.clear();
    }
    text.push_str(&chunk);
    vm.current_this
        .set_property(String::from("__node_http_body__"), JsValue::String(text));
    JsValue::Bool(true)
}

fn end(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let body = args.first().cloned().unwrap_or(JsValue::Undefined);
    response_end_with_value(vm.current_this.clone(), body);
    JsValue::Undefined
}

fn response_end_with_value(res: JsValue, body_value: JsValue) {
    if res.get_property("__node_http_sent__").to_boolean() {
        return;
    }
    let mut body = res.get_property("__node_http_body__").to_js_string();
    if matches!(res.get_property("__node_http_body__"), JsValue::Undefined) {
        body.clear();
    }
    if !matches!(body_value, JsValue::Undefined) {
        body.push_str(&body_value.to_js_string());
    }
    let status = res.get_property("statusCode").to_number() as u16;
    let socket = res.get_property(RESPONSE_SOCKET_KEY).to_number() as u32;
    let status_text = status_text(status);
    let headers = res.get_property(RESPONSE_HEADERS_KEY);
    if matches!(headers.get_property("content-length"), JsValue::Undefined) {
        headers.set_property(
            String::from("content-length"),
            JsValue::String(body.as_bytes().len().to_string()),
        );
    }
    if matches!(headers.get_property("connection"), JsValue::Undefined) {
        headers.set_property(
            String::from("connection"),
            JsValue::String(String::from("close")),
        );
    }
    let response = format!(
        "HTTP/1.1 {} {}\r\n{}\r\n{}",
        status,
        status_text,
        render_headers(&headers),
        body
    );
    let mut socket = tcp_handle(socket, libuv::UvHandleKind::Tcp);
    libuv::tcp_write(&mut socket, response.as_bytes());
    libuv::tcp_close(&mut socket);
    res.set_property(String::from("__node_http_sent__"), JsValue::Bool(true));
}

fn copy_headers(from: &JsValue, to: &JsValue) {
    for key in ["content-type", "content-length", "location", "connection"] {
        let value = from.get_property(key);
        if !matches!(value, JsValue::Undefined) {
            to.set_property(String::from(key), value);
        }
    }
}

fn render_headers(headers: &JsValue) -> String {
    let mut out = String::new();
    for key in ["content-type", "content-length", "location", "connection"] {
        let value = headers.get_property(key);
        if !matches!(value, JsValue::Undefined) {
            out.push_str(key);
            out.push_str(": ");
            out.push_str(&value.to_js_string());
            out.push_str("\r\n");
        }
    }
    out
}

fn send_simple_response(socket: u32, status: u16, text: &str, body: &[u8]) {
    let response = format!(
        "HTTP/1.1 {} {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        status,
        text,
        body.len(),
        core::str::from_utf8(body).unwrap_or("")
    );
    let mut socket = tcp_handle(socket, libuv::UvHandleKind::Tcp);
    libuv::tcp_write(&mut socket, response.as_bytes());
    libuv::tcp_close(&mut socket);
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn lower_ascii(input: &str) -> String {
    input
        .bytes()
        .map(|byte| {
            if byte.is_ascii_uppercase() {
                (byte + 32) as char
            } else {
                byte as char
            }
        })
        .collect()
}

fn register_server(vm: &mut Vm, server: JsValue) {
    let mut servers = http_servers(vm);
    if !servers.iter().any(|candidate| candidate.strict_eq(&server)) {
        servers.push(server);
    }
    vm.globals
        .borrow_mut()
        .set_hidden(String::from(HTTP_SERVERS_KEY), JsValue::new_array(servers));
}

fn http_servers(vm: &Vm) -> Vec<JsValue> {
    match vm.globals.borrow().get(HTTP_SERVERS_KEY) {
        JsValue::Array(array) => array.borrow().to_dense_vec(),
        _ => Vec::new(),
    }
}

fn tcp_handle(socket_id: u32, kind: libuv::UvHandleKind) -> libuv::UvTcp {
    let mut handle = libuv::UvTcp::new();
    handle.socket_id = socket_id;
    handle.kind = kind;
    handle.active = socket_id != u32::MAX;
    handle
}

struct HttpRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
}
