//! Host-mode networking — delegates to std::net.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;

// Global connection table: socket_id → TcpStream
static CONNECTIONS: Mutex<Vec<Option<TcpStream>>> = Mutex::new(Vec::new());

fn store_stream(stream: TcpStream) -> u32 {
    let mut conns = CONNECTIONS.lock().unwrap();
    // Find empty slot
    for (i, slot) in conns.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(stream);
            return i as u32;
        }
    }
    let id = conns.len() as u32;
    conns.push(Some(stream));
    id
}

fn with_stream<F, R>(socket_id: u32, f: F) -> R
where
    F: FnOnce(&mut TcpStream) -> R,
    R: Default,
{
    let mut conns = CONNECTIONS.lock().unwrap();
    if let Some(Some(stream)) = conns.get_mut(socket_id as usize) {
        f(stream)
    } else {
        R::default()
    }
}

pub fn get_config(_buf: &mut [u8; 24]) -> u32 { u32::MAX }
pub fn set_config(_buf: &[u8; 16]) -> u32 { u32::MAX }

pub fn dns(hostname: &str, result: &mut [u8; 4]) -> u32 {
    use std::net::ToSocketAddrs;
    let addr_str = alloc::format!("{}:80", hostname);
    if let Ok(mut addrs) = addr_str.as_str().to_socket_addrs() {
        if let Some(addr) = addrs.next() {
            if let std::net::IpAddr::V4(ipv4) = addr.ip() {
                *result = ipv4.octets();
                return 0;
            }
        }
    }
    u32::MAX
}

pub fn tcp_connect(ip: &[u8; 4], port: u16, timeout_ms: u32) -> u32 {
    let addr = std::net::SocketAddrV4::new(
        std::net::Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
        port,
    );
    let timeout = std::time::Duration::from_millis(if timeout_ms == 0 { 10000 } else { timeout_ms as u64 });
    match TcpStream::connect_timeout(&std::net::SocketAddr::V4(addr), timeout) {
        Ok(stream) => {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
            store_stream(stream)
        }
        Err(_) => u32::MAX,
    }
}

pub fn tcp_send(socket_id: u32, data: &[u8]) -> u32 {
    with_stream(socket_id, |stream| {
        match stream.write_all(data) {
            Ok(()) => data.len() as u32,
            Err(_) => u32::MAX,
        }
    })
}

pub fn tcp_recv(socket_id: u32, buf: &mut [u8]) -> u32 {
    with_stream(socket_id, |stream| {
        match stream.read(buf) {
            Ok(0) => 0,
            Ok(n) => n as u32,
            Err(_) => u32::MAX,
        }
    })
}

pub fn tcp_recv_available(_socket_id: u32) -> u32 {
    0 // Not easily implementable with std
}

pub fn tcp_close(socket_id: u32) -> u32 {
    let mut conns = CONNECTIONS.lock().unwrap();
    if let Some(slot) = conns.get_mut(socket_id as usize) {
        *slot = None;
    }
    0
}

pub fn tcp_status(socket_id: u32) -> u32 {
    let conns = CONNECTIONS.lock().unwrap();
    if let Some(Some(_)) = conns.get(socket_id as usize) {
        2 // Established
    } else {
        0 // Closed
    }
}

pub fn tcp_listen(_port: u16, _backlog: u16) -> u32 { u32::MAX }
pub fn tcp_accept(_listener_id: u32) -> (u32, [u8; 4], u16) { (u32::MAX, [0; 4], 0) }

pub fn ping(_ip: &[u8; 4], _seq: u32, _timeout: u32) -> u32 { u32::MAX }
pub fn dhcp(_buf: &mut [u8; 16]) -> u32 { u32::MAX }
