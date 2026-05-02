use crate::error::{UV_EAGAIN, UV_ECONNREFUSED, UV_EINVAL, UV_ENOTFOUND, UV_EOF};
use crate::handle::UvHandleKind;
use crate::loop_::UvLoop;

#[repr(C)]
#[derive(Debug)]
pub struct UvTcp {
    pub socket_id: u32,
    pub kind: UvHandleKind,
    pub local_port: u16,
    pub peer_ip: [u8; 4],
    pub peer_port: u16,
    pub active: bool,
}

impl UvTcp {
    pub const fn new() -> Self {
        Self {
            socket_id: u32::MAX,
            kind: UvHandleKind::Unknown,
            local_port: 0,
            peer_ip: [0; 4],
            peer_port: 0,
            active: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active && self.socket_id != u32::MAX
    }
}

impl Default for UvTcp {
    fn default() -> Self {
        Self::new()
    }
}

pub fn tcp_connect_host(handle: &mut UvTcp, host: &str, port: u16, timeout_ms: u32) -> i32 {
    let mut ip = [0u8; 4];
    if anyos_std::net::dns(host, &mut ip) == u32::MAX {
        return UV_ENOTFOUND;
    }
    tcp_connect(handle, ip, port, timeout_ms)
}

pub fn tcp_connect(handle: &mut UvTcp, ip: [u8; 4], port: u16, timeout_ms: u32) -> i32 {
    let socket_id = anyos_std::net::tcp_connect(&ip, port, timeout_ms);
    if socket_id == u32::MAX {
        return UV_ECONNREFUSED;
    }
    handle.socket_id = socket_id;
    handle.kind = UvHandleKind::Tcp;
    handle.peer_ip = ip;
    handle.peer_port = port;
    handle.active = true;
    0
}

pub fn tcp_listen(handle: &mut UvTcp, port: u16, backlog: u16) -> i32 {
    let listener = anyos_std::net::tcp_listen(port, backlog);
    if listener == u32::MAX {
        return UV_EINVAL;
    }
    handle.socket_id = listener;
    handle.kind = UvHandleKind::TcpServer;
    handle.local_port = port;
    handle.active = true;
    0
}

pub fn tcp_accept_nowait(server: &mut UvTcp, client: &mut UvTcp) -> i32 {
    if !server.is_active() || server.kind != UvHandleKind::TcpServer {
        return UV_EINVAL;
    }
    let (socket_id, ip, port) = anyos_std::net::tcp_accept_nowait(server.socket_id);
    if socket_id == u32::MAX {
        return UV_EAGAIN;
    }
    client.socket_id = socket_id;
    client.kind = UvHandleKind::Tcp;
    client.peer_ip = ip;
    client.peer_port = port;
    client.active = true;
    0
}

pub fn tcp_read(handle: &mut UvTcp, buf: &mut [u8]) -> i32 {
    if !handle.is_active() {
        return UV_EINVAL;
    }
    let n = anyos_std::net::tcp_recv(handle.socket_id, buf);
    match n {
        u32::MAX => UV_EAGAIN,
        0 => UV_EOF,
        value => value as i32,
    }
}

pub fn tcp_write(handle: &mut UvTcp, data: &[u8]) -> i32 {
    if !handle.is_active() {
        return UV_EINVAL;
    }
    let n = anyos_std::net::tcp_send(handle.socket_id, data);
    if n == u32::MAX {
        UV_EINVAL
    } else {
        n as i32
    }
}

pub fn tcp_close(handle: &mut UvTcp) -> i32 {
    if handle.socket_id != u32::MAX {
        anyos_std::net::tcp_close(handle.socket_id);
    }
    handle.socket_id = u32::MAX;
    handle.kind = UvHandleKind::Unknown;
    handle.active = false;
    0
}

#[no_mangle]
pub extern "C" fn uv_tcp_init(loop_: *mut UvLoop, handle: *mut UvTcp) -> i32 {
    if loop_.is_null() || handle.is_null() {
        return UV_EINVAL;
    }
    unsafe {
        *handle = UvTcp::new();
    }
    0
}

#[no_mangle]
pub extern "C" fn uv_tcp_connect_ipv4(
    handle: *mut UvTcp,
    ip0: u8,
    ip1: u8,
    ip2: u8,
    ip3: u8,
    port: u16,
    timeout_ms: u32,
) -> i32 {
    if handle.is_null() {
        return UV_EINVAL;
    }
    unsafe { tcp_connect(&mut *handle, [ip0, ip1, ip2, ip3], port, timeout_ms) }
}

#[no_mangle]
pub extern "C" fn uv_tcp_bind_listen(handle: *mut UvTcp, port: u16, backlog: u16) -> i32 {
    if handle.is_null() {
        return UV_EINVAL;
    }
    unsafe { tcp_listen(&mut *handle, port, backlog) }
}

#[no_mangle]
pub extern "C" fn uv_tcp_accept_nowait(server: *mut UvTcp, client: *mut UvTcp) -> i32 {
    if server.is_null() || client.is_null() {
        return UV_EINVAL;
    }
    unsafe { tcp_accept_nowait(&mut *server, &mut *client) }
}

#[no_mangle]
pub extern "C" fn uv_tcp_read(handle: *mut UvTcp, buf: *mut u8, len: usize) -> i32 {
    if handle.is_null() || buf.is_null() {
        return UV_EINVAL;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    unsafe { tcp_read(&mut *handle, slice) }
}

#[no_mangle]
pub extern "C" fn uv_tcp_write(handle: *mut UvTcp, data: *const u8, len: usize) -> i32 {
    if handle.is_null() || data.is_null() {
        return UV_EINVAL;
    }
    let slice = unsafe { core::slice::from_raw_parts(data, len) };
    unsafe { tcp_write(&mut *handle, slice) }
}

#[no_mangle]
pub extern "C" fn uv_close(handle: *mut UvTcp) -> i32 {
    if handle.is_null() {
        return UV_EINVAL;
    }
    unsafe { tcp_close(&mut *handle) }
}
