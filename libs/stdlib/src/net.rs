//! Networking — config, ping, DHCP, DNS, ARP.

use crate::raw::*;

/// Get network config. Writes 24 bytes: [ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1]
pub fn get_config(buf: &mut [u8; 24]) -> u32 {
    syscall2(SYS_NET_CONFIG, 0, buf.as_mut_ptr() as u64)
}

/// Set network config. Takes 16 bytes: [ip:4, mask:4, gw:4, dns:4]
pub fn set_config(buf: &[u8; 16]) -> u32 {
    syscall2(SYS_NET_CONFIG, 1, buf.as_ptr() as u64)
}

/// ICMP ping. ip=4 bytes, returns RTT ticks or u32::MAX.
pub fn ping(ip: &[u8; 4], seq: u32, timeout: u32) -> u32 {
    syscall3(SYS_NET_PING, ip.as_ptr() as u64, seq as u64, timeout as u64)
}

/// DHCP discover. Writes result [ip:4, mask:4, gw:4, dns:4] to buf.
/// Returns 0 on success.
pub fn dhcp(buf: &mut [u8; 16]) -> u32 {
    syscall1(SYS_NET_DHCP, buf.as_mut_ptr() as u64)
}

/// DNS resolve. Writes resolved IP (4 bytes) to result.
/// Returns 0 on success.
pub fn dns(hostname: &str, result: &mut [u8; 4]) -> u32 {
    let mut host_buf = [0u8; 257];
    let len = hostname.len().min(256);
    host_buf[..len].copy_from_slice(&hostname.as_bytes()[..len]);
    host_buf[len] = 0;
    syscall2(SYS_NET_DNS, host_buf.as_ptr() as u64, result.as_mut_ptr() as u64)
}

/// Disable the NIC.
pub fn disable_nic() -> u32 {
    syscall2(SYS_NET_CONFIG, 2, 0)
}

/// Enable the NIC.
pub fn enable_nic() -> u32 {
    syscall2(SYS_NET_CONFIG, 3, 0)
}

/// Check if the NIC is enabled. Returns true if enabled.
pub fn is_nic_enabled() -> bool {
    syscall2(SYS_NET_CONFIG, 4, 0) == 1
}

/// Check if NIC hardware was detected. Returns true if available.
pub fn is_nic_available() -> bool {
    syscall2(SYS_NET_CONFIG, 5, 0) == 1
}

/// Get ARP table. Each entry 12 bytes: [ip:4, mac:6, pad:2]. Returns count.
pub fn arp(buf: &mut [u8]) -> u32 {
    syscall2(SYS_NET_ARP, buf.as_mut_ptr() as u64, buf.len() as u64)
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
    syscall1(SYS_TCP_CONNECT, &params as *const _ as u64)
}

/// Send data on a TCP connection. Returns bytes sent or u32::MAX on error.
pub fn tcp_send(socket_id: u32, data: &[u8]) -> u32 {
    syscall3(SYS_TCP_SEND, socket_id as u64, data.as_ptr() as u64, data.len() as u64)
}

/// Receive data from a TCP connection.
/// Returns bytes received, 0=EOF (remote closed), u32::MAX=error/timeout.
pub fn tcp_recv(socket_id: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_TCP_RECV, socket_id as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Close a TCP connection. Returns 0.
pub fn tcp_close(socket_id: u32) -> u32 {
    syscall1(SYS_TCP_CLOSE, socket_id as u64)
}

/// Get TCP connection state. Returns state enum as u32.
/// 0=Closed, 1=SynSent, 2=Established, etc. u32::MAX=not found.
pub fn tcp_status(socket_id: u32) -> u32 {
    syscall1(SYS_TCP_STATUS, socket_id as u64)
}

/// Listen on a TCP port. Returns listener socket_id or u32::MAX.
pub fn tcp_listen(port: u16, backlog: u16) -> u32 {
    syscall2(SYS_TCP_LISTEN, port as u64, backlog as u64)
}

/// Accept a connection from a listening socket. Blocks until a connection
/// arrives or timeout (30s). Returns (socket_id, remote_ip, remote_port)
/// or (u32::MAX, [0;4], 0) on error/timeout.
pub fn tcp_accept(listener_id: u32) -> (u32, [u8; 4], u16) {
    let mut result = [0u8; 12];
    let rc = syscall2(SYS_TCP_ACCEPT, listener_id as u64, result.as_mut_ptr() as u64);
    if rc == u32::MAX {
        return (u32::MAX, [0; 4], 0);
    }
    let sock_id = u32::from_le_bytes([result[0], result[1], result[2], result[3]]);
    let ip = [result[4], result[5], result[6], result[7]];
    let port = u16::from_le_bytes([result[8], result[9]]);
    (sock_id, ip, port)
}

/// TCP connection info returned by `tcp_list()`.
pub struct TcpConnInfo {
    pub local_ip: [u8; 4],
    pub local_port: u16,
    pub remote_ip: [u8; 4],
    pub remote_port: u16,
    pub state: u8,
    pub owner_tid: u8,
    pub recv_buf_len: u16,
}

/// List all active TCP connections/listeners. Returns a Vec of connection info.
pub fn tcp_list() -> alloc::vec::Vec<TcpConnInfo> {
    let mut buf = [0u8; 64 * 16]; // max 64 entries * 16 bytes each
    let count = syscall2(SYS_TCP_LIST, buf.as_mut_ptr() as u64, 64);
    let mut result = alloc::vec::Vec::new();
    for i in 0..count as usize {
        let off = i * 16;
        result.push(TcpConnInfo {
            local_ip: [buf[off], buf[off+1], buf[off+2], buf[off+3]],
            local_port: u16::from_be_bytes([buf[off+4], buf[off+5]]),
            remote_ip: [buf[off+6], buf[off+7], buf[off+8], buf[off+9]],
            remote_port: u16::from_be_bytes([buf[off+10], buf[off+11]]),
            state: buf[off+12],
            owner_tid: buf[off+13],
            recv_buf_len: u16::from_le_bytes([buf[off+14], buf[off+15]]),
        });
    }
    result
}

// =========================================================================
// UDP
// =========================================================================

/// Bind to a UDP port (creates receive queue). Returns 0 on success.
pub fn udp_bind(port: u16) -> u32 {
    syscall1(SYS_UDP_BIND, port as u64)
}

/// Unbind a UDP port. Returns 0.
pub fn udp_unbind(port: u16) -> u32 {
    syscall1(SYS_UDP_UNBIND, port as u64)
}

/// Send a UDP datagram. Returns bytes sent or u32::MAX on error.
/// flags: bit 0 = force broadcast (bypass SO_BROADCAST check).
pub fn udp_sendto(dst_ip: &[u8; 4], dst_port: u16, src_port: u16, data: &[u8], flags: u32) -> u32 {
    #[repr(C)]
    struct UdpSendParams {
        dst_ip: [u8; 4],
        dst_port: u16,
        src_port: u16,
        data_ptr: u32,
        data_len: u32,
        flags: u32,
    }
    let params = UdpSendParams {
        dst_ip: *dst_ip,
        dst_port,
        src_port,
        data_ptr: data.as_ptr() as u32,
        data_len: data.len() as u32,
        flags,
    };
    syscall1(SYS_UDP_SENDTO, &params as *const _ as u64)
}

/// Receive a UDP datagram on a bound port.
/// Buffer receives: [src_ip:4, src_port:u16, payload_len:u16, payload...].
/// Returns total bytes written (8 + payload), 0=no data/timeout, u32::MAX=error.
pub fn udp_recvfrom(port: u16, buf: &mut [u8]) -> u32 {
    syscall3(SYS_UDP_RECVFROM, port as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Set UDP socket option on a bound port.
/// opt=1: SO_BROADCAST (val=1 enable, 0 disable).
/// opt=2: SO_RCVTIMEO (val=timeout in ms, 0=non-blocking).
/// Returns 0 on success, u32::MAX on error.
pub fn udp_set_opt(port: u16, opt: u32, val: u32) -> u32 {
    syscall3(SYS_UDP_SET_OPT, port as u64, opt as u64, val as u64)
}
