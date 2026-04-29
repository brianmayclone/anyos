#![cfg_attr(not(feature = "host"), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

const PIPE_NAME: &str = "dnsd";

#[cfg(feature = "host")]
pub fn resolve_ipv4(host: &str) -> Option<[u8; 4]> {
    if let Some(ip) = parse_ipv4(host) {
        return Some(ip);
    }
    use std::net::ToSocketAddrs;
    let addr_str = alloc::format!("{}:80", host);
    let mut addrs = addr_str.as_str().to_socket_addrs().ok()?;
    let addr = addrs.next()?;
    match addr.ip() {
        std::net::IpAddr::V4(ip) => Some(ip.octets()),
        _ => None,
    }
}

#[cfg(feature = "host")]
pub fn flush_cache() -> bool {
    true
}

#[cfg(feature = "host")]
pub fn reload() -> bool {
    true
}

#[cfg(feature = "host")]
pub fn stats() -> Option<String> {
    None
}

#[cfg(not(feature = "host"))]
pub fn resolve_ipv4(host: &str) -> Option<[u8; 4]> {
    if let Some(ip) = parse_ipv4(host) {
        return Some(ip);
    }

    if let Some(response) = request(&alloc::format!("RESOLVE {}", host)) {
        if let Some(ip) = parse_resolve_response(&response) {
            return Some(ip);
        }
    }

    // Keep kernel DNS as fallback until dnsd is guaranteed to be present.
    let mut resolved = [0u8; 4];
    if libsyscall::dns_resolve(host, &mut resolved) == 0 {
        Some(resolved)
    } else {
        None
    }
}

#[cfg(not(feature = "host"))]
pub fn flush_cache() -> bool {
    if let Some(response) = request("FLUSH") {
        if response.starts_with("OK\t") {
            return true;
        }
    }
    flush_kernel_cache() == 0
}

#[cfg(not(feature = "host"))]
pub fn reload() -> bool {
    request("RELOAD").map(|response| response.starts_with("OK\t")).unwrap_or(false)
}

#[cfg(not(feature = "host"))]
pub fn stats() -> Option<String> {
    request("STATS")
}

#[cfg(not(feature = "host"))]
fn request(command: &str) -> Option<String> {
    let tid = libsyscall::get_tid();
    let mut tid_buf = [0u8; 16];
    let tid_len = fmt_u32(tid, &mut tid_buf);
    let tid_str = core::str::from_utf8(&tid_buf[..tid_len]).ok()?;

    let mut resp_name = String::from("dnsd-");
    resp_name.push_str(tid_str);

    let main_pipe = pipe_open(PIPE_NAME);
    if main_pipe == 0 {
        return None;
    }

    let resp_pipe = pipe_create(&resp_name);
    if resp_pipe == 0 {
        pipe_close(main_pipe);
        return None;
    }

    let mut req = String::from(tid_str);
    req.push('\t');
    req.push_str(command);
    req.push('\n');
    pipe_write(main_pipe, req.as_bytes());

    let mut data = Vec::new();
    let mut chunk = [0u8; 512];
    for _ in 0..40 {
        let n = pipe_read(resp_pipe, &mut chunk);
        if n > 0 && n != u32::MAX {
            data.extend_from_slice(&chunk[..n as usize]);
            if data.len() >= 2 && &data[data.len() - 2..] == b"\n\n" {
                break;
            }
        } else {
            libsyscall::sleep(10);
        }
    }

    pipe_close(main_pipe);
    pipe_close(resp_pipe);
    String::from_utf8(data).ok()
}

#[cfg(not(feature = "host"))]
fn pipe_create(name: &str) -> u32 {
    let mut buf = [0u8; 257];
    let len = name.len().min(256);
    buf[..len].copy_from_slice(&name.as_bytes()[..len]);
    buf[len] = 0;
    libsyscall::syscall1(libsyscall::SYS_PIPE_CREATE, buf.as_ptr() as u64) as u32
}

#[cfg(not(feature = "host"))]
fn pipe_open(name: &str) -> u32 {
    let mut buf = [0u8; 257];
    let len = name.len().min(256);
    buf[..len].copy_from_slice(&name.as_bytes()[..len]);
    buf[len] = 0;
    libsyscall::syscall1(libsyscall::SYS_PIPE_OPEN, buf.as_ptr() as u64) as u32
}

#[cfg(not(feature = "host"))]
fn pipe_read(pipe_id: u32, buf: &mut [u8]) -> u32 {
    libsyscall::syscall3(
        libsyscall::SYS_PIPE_READ,
        pipe_id as u64,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
    ) as u32
}

#[cfg(not(feature = "host"))]
fn pipe_write(pipe_id: u32, data: &[u8]) -> u32 {
    libsyscall::syscall3(
        libsyscall::SYS_PIPE_WRITE,
        pipe_id as u64,
        data.as_ptr() as u64,
        data.len() as u64,
    ) as u32
}

#[cfg(not(feature = "host"))]
fn pipe_close(pipe_id: u32) -> u32 {
    libsyscall::syscall1(libsyscall::SYS_PIPE_CLOSE, pipe_id as u64) as u32
}

#[cfg(not(feature = "host"))]
fn flush_kernel_cache() -> u32 {
    libsyscall::syscall2(libsyscall::SYS_NET_CONFIG, 13, 0) as u32
}

fn parse_resolve_response(response: &str) -> Option<[u8; 4]> {
    let line = response.lines().next()?;
    let mut parts = line.split('\t');
    if parts.next()? != "OK" {
        return None;
    }
    if parts.next()? != "A" {
        return None;
    }
    parse_ipv4(parts.next()?)
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for octet in &mut out {
        *octet = parse_u8(parts.next()?)?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

fn parse_u8(s: &str) -> Option<u8> {
    let mut val = 0u32;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
        if val > 255 {
            return None;
        }
    }
    Some(val as u8)
}

fn fmt_u32(mut val: u32, buf: &mut [u8]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut i = 0usize;
    while val > 0 {
        tmp[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    i
}
