// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Host-mode networking — delegates to std::net.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;

// Global connection table: socket_id → TcpStream
static CONNECTIONS: Mutex<Vec<Option<TcpStream>>> = Mutex::new(Vec::new());
static LISTENERS: Mutex<Vec<Option<TcpListener>>> = Mutex::new(Vec::new());
const LISTENER_ID_BASE: u32 = 1_000_000;

fn store_stream(stream: TcpStream) -> u32 {
    let mut conns = CONNECTIONS.lock().unwrap();
    // Find empty slot
    for (i, slot) in conns.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(stream);
            return LISTENER_ID_BASE + i as u32;
        }
    }
    let id = conns.len() as u32;
    conns.push(Some(stream));
    LISTENER_ID_BASE + id
}

fn store_listener(listener: TcpListener) -> u32 {
    let mut listeners = LISTENERS.lock().unwrap();
    for (i, slot) in listeners.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(listener);
            return i as u32;
        }
    }
    let id = listeners.len() as u32;
    listeners.push(Some(listener));
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

pub fn get_config(_buf: &mut [u8; 24]) -> u32 {
    u32::MAX
}
pub fn set_config(_buf: &[u8; 16]) -> u32 {
    u32::MAX
}

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
    let addr =
        std::net::SocketAddrV4::new(std::net::Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]), port);
    let timeout = std::time::Duration::from_millis(if timeout_ms == 0 {
        10000
    } else {
        timeout_ms as u64
    });
    match TcpStream::connect_timeout(&std::net::SocketAddr::V4(addr), timeout) {
        Ok(stream) => {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
            store_stream(stream)
        }
        Err(_) => u32::MAX,
    }
}

pub fn tcp_send(socket_id: u32, data: &[u8]) -> u32 {
    with_stream(socket_id, |stream| match stream.write_all(data) {
        Ok(()) => data.len() as u32,
        Err(_) => u32::MAX,
    })
}

pub fn tcp_recv(socket_id: u32, buf: &mut [u8]) -> u32 {
    with_stream(socket_id, |stream| match stream.read(buf) {
        Ok(0) => 0,
        Ok(n) => n as u32,
        Err(_) => u32::MAX,
    })
}

pub fn tcp_recv_available(_socket_id: u32) -> u32 {
    0 // Not easily implementable with std
}

pub fn tcp_close(socket_id: u32) -> u32 {
    if socket_id >= LISTENER_ID_BASE {
        let mut listeners = LISTENERS.lock().unwrap();
        let index = (socket_id - LISTENER_ID_BASE) as usize;
        if let Some(slot) = listeners.get_mut(index) {
            *slot = None;
        }
        return 0;
    }
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

pub fn tcp_listen(port: u16, _backlog: u16) -> u32 {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => {
            let _ = listener.set_nonblocking(true);
            store_listener(listener)
        }
        Err(_) => u32::MAX,
    }
}

pub fn tcp_accept(listener_id: u32) -> (u32, [u8; 4], u16) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        let accepted = tcp_accept_nowait(listener_id);
        if accepted.0 != u32::MAX {
            return accepted;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    (u32::MAX, [0; 4], 0)
}

pub fn tcp_accept_nowait(listener_id: u32) -> (u32, [u8; 4], u16) {
    if listener_id < LISTENER_ID_BASE {
        return (u32::MAX, [0; 4], 0);
    }
    let mut listeners = LISTENERS.lock().unwrap();
    let index = (listener_id - LISTENER_ID_BASE) as usize;
    let Some(Some(listener)) = listeners.get_mut(index) else {
        return (u32::MAX, [0; 4], 0);
    };
    match listener.accept() {
        Ok((stream, addr)) => {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));
            let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
            let socket = store_stream(stream);
            let ip = match addr.ip() {
                std::net::IpAddr::V4(ip) => ip.octets(),
                std::net::IpAddr::V6(_) => [127, 0, 0, 1],
            };
            (socket, ip, addr.port())
        }
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => (u32::MAX, [0; 4], 0),
        Err(_) => (u32::MAX, [0; 4], 0),
    }
}

pub fn ping(_ip: &[u8; 4], _seq: u32, _timeout: u32) -> u32 {
    u32::MAX
}
pub fn dhcp(_buf: &mut [u8; 16]) -> u32 {
    u32::MAX
}
