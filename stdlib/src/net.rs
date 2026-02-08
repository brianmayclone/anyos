//! Networking — config, ping, DHCP, DNS, ARP.

use crate::raw::*;

/// Get network config. Writes 24 bytes: [ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1]
pub fn get_config(buf: &mut [u8; 24]) -> u32 {
    syscall2(SYS_NET_CONFIG, 0, buf.as_mut_ptr() as u32)
}

/// Set network config. Takes 16 bytes: [ip:4, mask:4, gw:4, dns:4]
pub fn set_config(buf: &[u8; 16]) -> u32 {
    syscall2(SYS_NET_CONFIG, 1, buf.as_ptr() as u32)
}

/// ICMP ping. ip=4 bytes, returns RTT ticks or u32::MAX.
pub fn ping(ip: &[u8; 4], seq: u32, timeout: u32) -> u32 {
    syscall3(SYS_NET_PING, ip.as_ptr() as u32, seq, timeout)
}

/// DHCP discover. Writes result [ip:4, mask:4, gw:4, dns:4] to buf.
/// Returns 0 on success.
pub fn dhcp(buf: &mut [u8; 16]) -> u32 {
    syscall1(SYS_NET_DHCP, buf.as_mut_ptr() as u32)
}

/// DNS resolve. Writes resolved IP (4 bytes) to result.
/// Returns 0 on success.
pub fn dns(hostname: &str, result: &mut [u8; 4]) -> u32 {
    let mut host_buf = [0u8; 257];
    let len = hostname.len().min(256);
    host_buf[..len].copy_from_slice(&hostname.as_bytes()[..len]);
    host_buf[len] = 0;
    syscall2(SYS_NET_DNS, host_buf.as_ptr() as u32, result.as_mut_ptr() as u32)
}

/// Get ARP table. Each entry 12 bytes: [ip:4, mac:6, pad:2]. Returns count.
pub fn arp(buf: &mut [u8]) -> u32 {
    syscall2(SYS_NET_ARP, buf.as_mut_ptr() as u32, buf.len() as u32)
}

// =========================================================================
// TCP
// =========================================================================

/// TCP connect to remote host. Returns socket_id or u32::MAX on error.
/// timeout is in milliseconds (0 = default 10s).
pub fn tcp_connect(ip: &[u8; 4], port: u16, timeout_ms: u32) -> u32 {
    #[repr(C)]
    struct TcpConnectParams {
        ip: [u8; 4],
        port: u16,
        _pad: u16,
        timeout: u32,
    }
    let params = TcpConnectParams {
        ip: *ip,
        port,
        _pad: 0,
        timeout: timeout_ms,
    };
    syscall1(SYS_TCP_CONNECT, &params as *const _ as u32)
}

/// Send data on a TCP connection. Returns bytes sent or u32::MAX on error.
pub fn tcp_send(socket_id: u32, data: &[u8]) -> u32 {
    syscall3(SYS_TCP_SEND, socket_id, data.as_ptr() as u32, data.len() as u32)
}

/// Receive data from a TCP connection.
/// Returns bytes received, 0=EOF (remote closed), u32::MAX=error/timeout.
pub fn tcp_recv(socket_id: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_TCP_RECV, socket_id, buf.as_mut_ptr() as u32, buf.len() as u32)
}

/// Close a TCP connection. Returns 0.
pub fn tcp_close(socket_id: u32) -> u32 {
    syscall1(SYS_TCP_CLOSE, socket_id)
}

/// Get TCP connection state. Returns state enum as u32.
/// 0=Closed, 1=SynSent, 2=Established, etc. u32::MAX=not found.
pub fn tcp_status(socket_id: u32) -> u32 {
    syscall1(SYS_TCP_STATUS, socket_id)
}
