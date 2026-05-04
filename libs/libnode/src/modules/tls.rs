use alloc::string::String;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::dns;
use super::util::object;

const TLS_HANDLE_KEY: &str = "__node_tls_handle__";
const TLS_SOCKET_KEY: &str = "__node_tls_socket__";

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(String::from("connect"), native_fn("connect", connect));
    module.set(
        String::from("createSecureContext"),
        native_fn("createSecureContext", create_secure_context),
    );
    module.set(
        String::from("TLSSocket"),
        native_fn("TLSSocket", tls_socket_ctor),
    );
    module.set(
        String::from("DEFAULT_MIN_VERSION"),
        JsValue::String(String::from("TLSv1.2")),
    );
    module.set(
        String::from("DEFAULT_MAX_VERSION"),
        JsValue::String(String::from("TLSv1.3")),
    );
    object(module)
}

fn connect(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    ensure_transport();
    let (host, port, callback) = parse_connect_args(args);
    let Some(address) = dns::resolve_host(&host) else {
        vm.pending_exception = Some(vm.make_type_error("getaddrinfo ENOTFOUND"));
        return JsValue::Undefined;
    };
    let Some(ip) = parse_ipv4(&address) else {
        vm.pending_exception = Some(vm.make_type_error("TLS connect requires an IPv4 address"));
        return JsValue::Undefined;
    };
    let socket = anyos_std::net::tcp_connect(&ip, port, 30_000);
    if socket == u32::MAX {
        vm.pending_exception = Some(vm.make_type_error("ECONNREFUSED"));
        return JsValue::Undefined;
    }
    let handle = libtls::connect(socket, &host);
    if handle < 0 {
        anyos_std::net::tcp_close(socket);
        vm.pending_exception = Some(vm.make_type_error("TLS handshake failed"));
        return JsValue::Undefined;
    }
    let out = make_tls_socket(handle as u32, socket, host);
    if matches!(callback, JsValue::Function(_)) {
        vm.call_value(&callback, &[], out.clone());
    }
    out
}

fn create_secure_context(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::new_object()
}

fn tls_socket_ctor(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this
        .set_property(String::from("authorized"), JsValue::Bool(true));
    vm.current_this.clone()
}

fn make_tls_socket(handle: u32, socket: u32, servername: String) -> JsValue {
    let mut out = JsObject::new();
    out.set(String::from("authorized"), JsValue::Bool(true));
    out.set(String::from("encrypted"), JsValue::Bool(true));
    out.set(String::from("servername"), JsValue::String(servername));
    out.set(String::from("write"), native_fn("write", write));
    out.set(String::from("end"), native_fn("end", end));
    out.set(String::from("destroy"), native_fn("destroy", destroy));
    out.set(String::from("on"), native_fn("on", on));
    out.set(String::from("once"), native_fn("once", on));
    out.set(
        String::from("setEncoding"),
        native_fn("setEncoding", this_value),
    );
    out.set(
        String::from("getProtocol"),
        native_fn("getProtocol", get_protocol),
    );
    out.set_hidden(String::from(TLS_HANDLE_KEY), JsValue::Number(handle as f64));
    out.set_hidden(String::from(TLS_SOCKET_KEY), JsValue::Number(socket as f64));
    object(out)
}

fn write(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let handle = vm.current_this.get_property(TLS_HANDLE_KEY).to_number() as u32;
    let data = args
        .first()
        .map(|value| value.to_js_string().into_bytes())
        .unwrap_or_default();
    let sent = libtls::send(handle, &data);
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], vm.current_this.clone());
    }
    JsValue::Bool(sent >= 0)
}

fn end(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if !args.is_empty() && !matches!(args[0], JsValue::Function(_)) {
        let _ = write(vm, &args[..1]);
    }
    close_current(vm);
    if let Some(callback) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        vm.call_value(callback, &[], vm.current_this.clone());
    }
    vm.current_this.clone()
}

fn destroy(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    close_current(vm);
    vm.current_this.clone()
}

fn on(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = args
        .first()
        .map(|value| value.to_js_string())
        .unwrap_or_default();
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if !matches!(callback, JsValue::Function(_)) {
        return vm.current_this.clone();
    }
    if event == "secureConnect" || event == "connect" || event == "ready" {
        vm.call_value(&callback, &[], vm.current_this.clone());
    }
    vm.current_this.clone()
}

fn this_value(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    vm.current_this.clone()
}

fn get_protocol(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("TLSv1.3"))
}

fn close_current(vm: &mut Vm) {
    let handle = vm.current_this.get_property(TLS_HANDLE_KEY).to_number() as u32;
    let socket = vm.current_this.get_property(TLS_SOCKET_KEY).to_number() as u32;
    if handle != 0 {
        libtls::close(handle);
    }
    if socket != 0 {
        anyos_std::net::tcp_close(socket);
    }
}

fn parse_connect_args(args: &[JsValue]) -> (String, u16, JsValue) {
    let mut host = String::from("localhost");
    let mut port = 443u16;
    let mut callback = JsValue::Undefined;
    if let Some(first) = args.first() {
        if matches!(first, JsValue::Object(_)) {
            let h = first.get_property("host").to_js_string();
            let servername = first.get_property("servername").to_js_string();
            if !h.is_empty() {
                host = h;
            } else if !servername.is_empty() {
                host = servername;
            }
            let p = first.get_property("port").to_number();
            if p > 0.0 {
                port = p as u16;
            }
        } else if first.to_number() > 0.0 {
            port = first.to_number() as u16;
            if let Some(second) = args.get(1) {
                if !matches!(second, JsValue::Function(_)) {
                    host = second.to_js_string();
                }
            }
        } else {
            host = first.to_js_string();
        }
    }
    if let Some(found) = args
        .iter()
        .find(|value| matches!(value, JsValue::Function(_)))
    {
        callback = found.clone();
    }
    (host, port, callback)
}

fn parse_ipv4(address: &str) -> Option<[u8; 4]> {
    let parts = address
        .split('.')
        .map(|part| part.parse::<u8>().ok())
        .collect::<alloc::vec::Vec<Option<u8>>>();
    if parts.len() != 4 {
        return None;
    }
    Some([parts[0]?, parts[1]?, parts[2]?, parts[3]?])
}

fn ensure_transport() {
    use core::sync::atomic::{AtomicBool, Ordering};
    static INITIALIZED: AtomicBool = AtomicBool::new(false);
    if !INITIALIZED.swap(true, Ordering::SeqCst) {
        libtls::set_transport(tcp_send, tcp_recv, sleep, random);
    }
}

fn tcp_send(fd: u32, data: &[u8]) -> i32 {
    let n = anyos_std::net::tcp_send(fd, data);
    if n == u32::MAX {
        -1
    } else {
        n as i32
    }
}

fn tcp_recv(fd: u32, buf: &mut [u8]) -> i32 {
    let n = anyos_std::net::tcp_recv(fd, buf);
    if n == u32::MAX {
        -1
    } else {
        n as i32
    }
}

fn sleep(ms: u32) {
    anyos_std::process::sleep(ms);
}

fn random(buf: &mut [u8]) -> u32 {
    anyos_std::sys::random(buf)
}
