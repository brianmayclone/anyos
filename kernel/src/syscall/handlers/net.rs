//! Networking syscall handlers.
//!
//! Covers general network config, TCP, UDP, DNS, DHCP, ARP, and
//! network polling/statistics.

use super::helpers::read_user_str;
#[allow(unused_imports)]
use super::helpers::{
    copy_to_user_bytes, copy_user_bytes, is_user_range_accessible, is_valid_user_ptr,
};

// Bound the copy size per syscall and keep the scratch buffer on the kernel
// stack. Large downloads call recv() thousands of times; heap-allocating a
// temporary buffer here makes Activity Mon show permanently committed kernel
// heap pages even after the TCP socket has been closed.
const TCP_RECV_COPY_CHUNK: usize = 64 * 1024;

// =========================================================================
// Networking (SYS_NET_*)
// =========================================================================

/// sys_net_config - Get or set network configuration.
/// arg1=cmd (0=get, 1=set), arg2=buf_ptr (24 bytes: ip4+mask4+gw4+dns4+mac6+link1+pad1)
pub fn sys_net_config(cmd: u32, buf_ptr: u64) -> u32 {
    match cmd {
        0 => {
            if buf_ptr == 0 {
                return u32::MAX;
            }
            let cfg = crate::net::config();
            let link_up = crate::drivers::network::link_up();
            let mut out = [0u8; 24];
            out[0..4].copy_from_slice(&cfg.ip.0);
            out[4..8].copy_from_slice(&cfg.mask.0);
            out[8..12].copy_from_slice(&cfg.gateway.0);
            out[12..16].copy_from_slice(&cfg.dns.0);
            out[16..22].copy_from_slice(&cfg.mac.0);
            out[22] = if link_up { 1 } else { 0 };
            out[23] = 0;
            if !copy_to_user_bytes(buf_ptr, &out, 24) {
                return u32::MAX;
            }
            0
        }
        1 => {
            if buf_ptr == 0 {
                return u32::MAX;
            }
            let raw = match copy_user_bytes(buf_ptr, 16, 16) {
                Some(v) => v,
                None => return u32::MAX,
            };
            let ip = [raw[0], raw[1], raw[2], raw[3]];
            let mask = [raw[4], raw[5], raw[6], raw[7]];
            let gw = [raw[8], raw[9], raw[10], raw[11]];
            let dns = [raw[12], raw[13], raw[14], raw[15]];
            crate::net::set_config(
                crate::net::types::Ipv4Addr(ip),
                crate::net::types::Ipv4Addr(mask),
                crate::net::types::Ipv4Addr(gw),
                crate::net::types::Ipv4Addr(dns),
            );
            0
        }
        2 => {
            // Disable NIC
            crate::drivers::network::set_enabled(false);
            0
        }
        3 => {
            // Enable NIC
            crate::drivers::network::set_enabled(true);
            0
        }
        4 => {
            // Query enabled state
            if crate::drivers::network::is_enabled() {
                1
            } else {
                0
            }
        }
        5 => {
            // Query hardware availability
            if crate::drivers::network::is_available() {
                1
            } else {
                0
            }
        }
        6 => {
            // Reload hosts file from disk
            crate::net::dns::load_hosts();
            0
        }
        13 => {
            // Flush the in-kernel DNS cache
            crate::net::dns::flush_cache();
            0
        }
        7 => {
            // Get interface configs. buf_ptr = output buffer, must hold N*128 bytes.
            // Returns number of interfaces written.
            if buf_ptr == 0 {
                return u32::MAX;
            }
            let mut tmp = [0u8; 8 * 128];
            let n = crate::net::interfaces::serialize_configs(&mut tmp);
            if !copy_to_user_bytes(buf_ptr, &tmp, 8 * 128) {
                return u32::MAX;
            }
            n
        }
        8 => {
            // Set interface configs and save to disk.
            // buf_ptr points to: [count:u32, entries: count*128 bytes]
            if buf_ptr == 0 {
                return u32::MAX;
            }
            let header = match copy_user_bytes(buf_ptr, 4, 4) {
                Some(v) => v,
                None => return u32::MAX,
            };
            let count = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            if count == 0 || count > 8 {
                return u32::MAX;
            }
            let body = match copy_user_bytes(buf_ptr + 4, count as usize * 128, 8 * 128) {
                Some(v) => v,
                None => return u32::MAX,
            };
            crate::net::interfaces::apply_and_save(&body, count)
        }
        9 => {
            // Get NIC driver name. buf_ptr = output buffer (up to 64 bytes).
            // Returns name length, or 0 if no NIC.
            if buf_ptr == 0 {
                return 0;
            }
            {
                if let Some(name) = crate::drivers::network::driver_name() {
                    let bytes = name.as_bytes();
                    let len = bytes.len().min(64);
                    if !copy_to_user_bytes(buf_ptr, &bytes[..len], 64) {
                        return 0;
                    }
                    len as u32
                } else {
                    0
                }
            }
        }
        10 => {
            // Get IPv6 config. buf_ptr = output buffer (80 bytes):
            //   [0..16]:  link-local address
            //   [16..32]: global address
            //   [32]:     prefix length
            //   [33..35]: reserved
            //   [35..51]: gateway
            //   [51..67]: dns
            //   [67..80]: reserved
            if buf_ptr == 0 {
                return u32::MAX;
            }
            let cfg = crate::net::config();
            let mut out = [0u8; 67];
            out[0..16].copy_from_slice(&cfg.ipv6_link_local.0);
            out[16..32].copy_from_slice(&cfg.ipv6_addr.0);
            out[32] = cfg.ipv6_prefix_len;
            out[33] = 0;
            out[34] = 0;
            out[35..51].copy_from_slice(&cfg.ipv6_gateway.0);
            out[51..67].copy_from_slice(&cfg.ipv6_dns.0);
            if !copy_to_user_bytes(buf_ptr, &out, 67) {
                return u32::MAX;
            }
            0
        }
        11 => {
            // Set IPv6 config. buf_ptr = input buffer (49 bytes):
            //   [0..16]:  global address
            //   [16]:     prefix length
            //   [17..33]: gateway
            //   [33..49]: dns
            if buf_ptr == 0 {
                return u32::MAX;
            }
            let raw = match copy_user_bytes(buf_ptr, 49, 49) {
                Some(v) => v,
                None => return u32::MAX,
            };
            let mut addr = [0u8; 16];
            addr.copy_from_slice(&raw[0..16]);
            let prefix_len = raw[16];
            let mut gw = [0u8; 16];
            gw.copy_from_slice(&raw[17..33]);
            let mut dns = [0u8; 16];
            dns.copy_from_slice(&raw[33..49]);
            crate::net::set_config_v6(
                crate::net::types::Ipv6Addr(addr),
                prefix_len,
                crate::net::types::Ipv6Addr(gw),
                crate::net::types::Ipv6Addr(dns),
            );
            0
        }
        12 => {
            // Get NDP neighbor table. Same format as ARP (but 28 bytes per entry: ip6[16]+mac[6]+pad[6]).
            if buf_ptr == 0 {
                return u32::MAX;
            }
            let entries = crate::net::ndp::entries();
            let count = entries.len().min(32);
            if count == 0 {
                return 0;
            }
            let mut out = [0u8; 32 * 28];
            for (i, (ip, mac)) in entries.iter().enumerate().take(count) {
                let off = i * 28;
                out[off..off + 16].copy_from_slice(&ip.0);
                out[off + 16..off + 22].copy_from_slice(&mac.0);
                // padding bytes [off+22 .. off+28] already zero
            }
            if !copy_to_user_bytes(buf_ptr, &out[..count * 28], 32 * 28) {
                return u32::MAX;
            }
            count as u32
        }
        14 => {
            crate::net::trace::set_enabled(true);
            0
        }
        15 => {
            crate::net::trace::set_enabled(false);
            0
        }
        16 => {
            crate::net::trace::clear();
            0
        }
        17 => {
            if buf_ptr == 0 {
                return u32::MAX;
            }
            let mut tmp = [0u8; crate::net::trace::ENTRY_SIZE * 64];
            let n = crate::net::trace::read_and_clear(&mut tmp);
            if !copy_to_user_bytes(buf_ptr, &tmp, crate::net::trace::ENTRY_SIZE * 64) {
                return u32::MAX;
            }
            n
        }
        _ => u32::MAX,
    }
}

/// sys_net_ping - ICMP ping. arg1=ip_ptr(4 bytes), arg2=seq, arg3=timeout_ticks
/// Returns RTT in ticks, or u32::MAX on timeout.
pub fn sys_net_ping(ip_ptr: u64, seq: u32, timeout: u32) -> u32 {
    if ip_ptr == 0 {
        return u32::MAX;
    }
    let raw = match copy_user_bytes(ip_ptr, 4, 4) {
        Some(v) => v,
        None => return u32::MAX,
    };
    let ip = crate::net::types::Ipv4Addr([raw[0], raw[1], raw[2], raw[3]]);
    match crate::net::icmp::ping(ip, seq as u16, timeout) {
        Some((rtt, _ttl)) => rtt,
        None => u32::MAX,
    }
}

/// sys_net_dhcp - DHCP discovery. arg1=buf_ptr (16 bytes: ip+mask+gw+dns)
/// Returns 0 on success, applies config automatically.
/// Error codes:
///   1 = no NIC hardware available
///   2 = NIC disabled
///   3 = DISCOVER timed out (no server responded)
///   4 = REQUEST timed out (no ACK after OFFER)
///   u32::MAX = other / unknown error
pub fn sys_net_dhcp(buf_ptr: u64) -> u32 {
    if !crate::drivers::network::is_available() {
        return 1;
    }
    if !crate::drivers::network::is_enabled() {
        return 2;
    }
    match crate::net::dhcp::discover() {
        Ok(result) => {
            crate::net::set_config(result.ip, result.mask, result.gateway, result.dns);
            if buf_ptr != 0 {
                let mut out = [0u8; 16];
                out[0..4].copy_from_slice(&result.ip.0);
                out[4..8].copy_from_slice(&result.mask.0);
                out[8..12].copy_from_slice(&result.gateway.0);
                out[12..16].copy_from_slice(&result.dns.0);
                // DHCP config is already applied above; ignore copy failure and
                // still report success.
                copy_to_user_bytes(buf_ptr, &out, 16);
            }
            0
        }
        Err(msg) => {
            if msg.contains("OFFER") {
                3
            } else if msg.contains("ACK") {
                4
            } else {
                u32::MAX
            }
        }
    }
}

/// sys_net_dns - DNS resolve. arg1=hostname_ptr, arg2=result_ptr(4 bytes)
pub fn sys_net_dns(hostname_ptr: u64, result_ptr: u64) -> u32 {
    let hostname = unsafe { read_user_str(hostname_ptr) };
    match crate::net::dns::resolve(hostname) {
        Ok(ip) => {
            if result_ptr != 0 && !copy_to_user_bytes(result_ptr, &ip.0, 4) {
                return u32::MAX;
            }
            0
        }
        Err(_) => u32::MAX,
    }
}

// =========================================================================
// TCP Networking (SYS_TCP_*)
// =========================================================================

/// sys_tcp_connect - Connect to a remote host.
/// arg1=params_ptr: [ip:4, port:u16, pad:u16, timeout:u32] = 12 bytes
/// Returns socket_id or u32::MAX on error.
pub fn sys_tcp_connect(params_ptr: u64) -> u32 {
    if params_ptr == 0 {
        return u32::MAX;
    }
    let params = match copy_user_bytes(params_ptr, 12, 12) {
        Some(v) => v,
        None => return u32::MAX,
    };
    let ip = crate::net::types::Ipv4Addr([params[0], params[1], params[2], params[3]]);
    let port = u16::from_le_bytes([params[4], params[5]]);
    let timeout = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    let pit_hz = crate::arch::hal::timer_frequency_hz() as u32;
    let timeout_ticks = if timeout == 0 {
        pit_hz
    } else {
        timeout * pit_hz / 1000
    };
    crate::net::tcp::connect(ip, port, timeout_ticks)
}

/// sys_tcp_send - Send data on TCP connection.
/// arg1=socket_id, arg2=buf_ptr, arg3=len
/// Returns bytes sent or u32::MAX on error.
pub fn sys_tcp_send(socket_id: u32, buf_ptr: u64, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 {
        return 0;
    }
    // Validate the whole user range is mapped, then hand it straight to the TCP
    // stack (which buffers/segments it internally, up to MAX_SEND_BUF = 2 MiB).
    // This preserves the original "send up to `len` bytes in one call" behavior
    // — a single stream write may be far larger than any fixed chunk — while
    // ensuring an unmapped page returns an error instead of faulting the kernel.
    if !is_user_range_accessible(buf_ptr, len as u64) {
        return 0;
    }
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as usize as *const u8, len as usize) };
    let result = crate::net::tcp::send(socket_id, buf, 1000); // 10s timeout
    if result != u32::MAX && result > 0 {
        crate::task::scheduler::record_net_tx(result as u64);
    }
    result
}

/// sys_tcp_recv - Receive data from TCP connection.
/// arg1=socket_id, arg2=buf_ptr, arg3=len
/// Returns bytes received, 0=EOF, u32::MAX=error.
pub fn sys_tcp_recv(socket_id: u32, buf_ptr: u64, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 {
        return u32::MAX;
    }
    let recv_len = (len as usize).min(TCP_RECV_COPY_CHUNK);
    if !is_valid_user_ptr(buf_ptr, recv_len as u64) {
        return u32::MAX;
    }

    let mut buf = [0u8; TCP_RECV_COPY_CHUNK];
    let result = crate::net::tcp::recv(socket_id, &mut buf[..recv_len], 3000); // 3s timeout (3000 ticks @ 1 kHz)
    if result != u32::MAX && result > 0 {
        let n = result as usize;
        if !copy_to_user_bytes(buf_ptr, &buf[..n], n) {
            return u32::MAX;
        }
    }
    if result != u32::MAX && result > 0 {
        crate::task::scheduler::record_net_rx(result as u64);
    }
    result
}

/// sys_tcp_close - Close TCP connection. arg1=socket_id.
pub fn sys_tcp_close(socket_id: u32) -> u32 {
    crate::net::tcp::close(socket_id)
}

/// sys_tcp_status - Get TCP connection state. arg1=socket_id.
/// Returns state enum as u32, or u32::MAX if not found.
pub fn sys_tcp_status(socket_id: u32) -> u32 {
    crate::net::tcp::status(socket_id)
}

/// sys_tcp_recv_available - Check bytes available to read.
/// Returns: >0 = bytes available, 0 = no data, u32::MAX-1 = EOF, u32::MAX = error.
pub fn sys_tcp_recv_available(socket_id: u32) -> u32 {
    crate::net::tcp::recv_available(socket_id)
}

/// sys_tcp_shutdown_wr - Half-close (send FIN, don't block).
/// arg1=socket_id. Returns 0 on success.
pub fn sys_tcp_shutdown_wr(socket_id: u32) -> u32 {
    crate::net::tcp::shutdown_write(socket_id)
}

/// sys_tcp_listen - Listen on a TCP port for incoming connections.
/// arg1=port, arg2=backlog. Returns listener socket_id or u32::MAX on error.
pub fn sys_tcp_listen(port: u32, backlog: u32) -> u32 {
    if port == 0 || port > 65535 {
        return u32::MAX;
    }
    crate::net::tcp::listen(port as u16, backlog.min(16) as u16)
}

/// sys_tcp_accept - Accept a connection from a listening socket.
/// arg1=listener_id, arg2=result_ptr (12 bytes: [socket_id:u32, ip:[u8;4], port:u16, pad:u16])
/// Returns 0 on success, u32::MAX on timeout/error.
pub fn sys_tcp_accept(listener_id: u32, result_ptr: u64) -> u32 {
    if result_ptr == 0 {
        return u32::MAX;
    }
    let pit_hz = crate::arch::hal::timer_frequency_hz() as u32;
    let timeout_ticks = 30 * pit_hz; // 30 second timeout
    let (sock_id, remote_ip, remote_port) = crate::net::tcp::accept(listener_id, timeout_ticks);
    if sock_id == u32::MAX {
        return u32::MAX - 1; // timeout / no pending connection
    }
    // Write result to user buffer
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&sock_id.to_le_bytes());
    out[4..8].copy_from_slice(remote_ip.as_bytes());
    out[8..10].copy_from_slice(&remote_port.to_le_bytes());
    // out[10], out[11] already 0
    if !copy_to_user_bytes(result_ptr, &out, 12) {
        return u32::MAX;
    }
    0
}

/// sys_tcp_accept_v6 - Accept an IPv6 connection from a listening socket.
/// arg1=listener_id, arg2=result_ptr (24 bytes: [socket_id:u32, ip:[u8;16], port:u16, pad:u16])
pub fn sys_tcp_accept_v6(listener_id: u32, result_ptr: u64) -> u32 {
    if result_ptr == 0 {
        return u32::MAX;
    }
    let pit_hz = crate::arch::hal::timer_frequency_hz() as u32;
    let timeout_ticks = 30 * pit_hz;
    let (sock_id, remote_ip, remote_port) = crate::net::tcp::accept_v6(listener_id, timeout_ticks);
    if sock_id == u32::MAX {
        return u32::MAX - 1;
    }
    let mut out = [0u8; 24];
    out[0..4].copy_from_slice(&sock_id.to_le_bytes());
    out[4..20].copy_from_slice(remote_ip.as_bytes());
    out[20..22].copy_from_slice(&remote_port.to_le_bytes());
    // out[22], out[23] already 0
    if !copy_to_user_bytes(result_ptr, &out, 24) {
        return u32::MAX;
    }
    0
}

/// sys_tcp_accept_nowait - Non-blocking accept: returns immediately.
/// arg1=listener_id, arg2=result_ptr (12 bytes, same as sys_tcp_accept)
/// Returns 0 if a connection was accepted, u32::MAX if none pending.
pub fn sys_tcp_accept_nowait(listener_id: u32, result_ptr: u64) -> u32 {
    if result_ptr == 0 {
        return u32::MAX;
    }
    // Use a 0-tick timeout so the accept loop returns immediately if nothing is ready.
    let (sock_id, remote_ip, remote_port) = crate::net::tcp::accept(listener_id, 0);
    if sock_id == u32::MAX {
        return u32::MAX - 1; // no pending connection
    }
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&sock_id.to_le_bytes());
    out[4..8].copy_from_slice(remote_ip.as_bytes());
    out[8..10].copy_from_slice(&remote_port.to_le_bytes());
    // out[10], out[11] already 0
    if !copy_to_user_bytes(result_ptr, &out, 12) {
        return u32::MAX;
    }
    0
}

/// sys_tcp_list - List all TCP connections.
/// arg1=buf_ptr, arg2=max_entries. Each entry is 16 bytes:
///   [local_ip:4, local_port:u16, remote_ip:4, remote_port:u16, state:u8, owner_tid_lo:u8, recv_buf_len:u16]
/// Returns number of entries written.
pub fn sys_tcp_list(buf_ptr: u64, max_entries: u32) -> u32 {
    if buf_ptr == 0 || max_entries == 0 {
        return 0;
    }
    let conns = crate::net::tcp::list_connections();
    let count = conns.len().min(max_entries as usize);
    if count == 0 {
        return 0;
    }
    let mut buf = alloc::vec![0u8; count * 16];

    for (i, info) in conns.iter().take(count).enumerate() {
        let off = i * 16;
        buf[off..off + 4].copy_from_slice(info.local_ip.as_bytes());
        let lp = info.local_port.to_be_bytes();
        buf[off + 4] = lp[0];
        buf[off + 5] = lp[1];
        buf[off + 6..off + 10].copy_from_slice(info.remote_ip.as_bytes());
        let rp = info.remote_port.to_be_bytes();
        buf[off + 10] = rp[0];
        buf[off + 11] = rp[1];
        buf[off + 12] = info.state as u8;
        buf[off + 13] = (info.owner_tid & 0xFF) as u8;
        let recv_len = (info.recv_buf_len.min(u16::MAX as usize) as u16).to_le_bytes();
        buf[off + 14] = recv_len[0];
        buf[off + 15] = recv_len[1];
    }

    if !copy_to_user_bytes(buf_ptr, &buf, count * 16) {
        return 0;
    }
    count as u32
}

/// sys_net_poll - Process pending network packets.
/// Triggers E1000 RX ring processing and TCP packet dispatch.
pub fn sys_net_poll() -> u32 {
    crate::net::poll();
    0
}

// =========================================================================
// UDP Networking (SYS_UDP_*)
// =========================================================================

/// sys_udp_bind - Bind to a UDP port (creates receive queue).
/// arg1=port. Returns 0 on success, u32::MAX if already bound or invalid.
pub fn sys_udp_bind(port: u32) -> u32 {
    if port == 0 || port > 65535 {
        return u32::MAX;
    }
    if crate::net::udp::bind(port as u16) {
        0
    } else {
        u32::MAX
    }
}

/// sys_udp_unbind - Unbind a UDP port.
/// arg1=port. Returns 0.
pub fn sys_udp_unbind(port: u32) -> u32 {
    if port > 65535 {
        return u32::MAX;
    }
    crate::net::udp::unbind(port as u16);
    0
}

/// sys_udp_sendto - Send a UDP datagram.
/// arg1=params_ptr: [dst_ip:4, dst_port:u16, src_port:u16, data_ptr:u32, data_len:u32, flags:u32] = 20 bytes
/// flags: bit 0 = force broadcast (bypass SO_BROADCAST check).
/// Returns bytes sent or u32::MAX on error.
pub fn sys_udp_sendto(params_ptr: u64) -> u32 {
    if params_ptr == 0 {
        return u32::MAX;
    }
    let params = match copy_user_bytes(params_ptr, 20, 20) {
        Some(v) => v,
        None => return u32::MAX,
    };

    let dst_ip = crate::net::types::Ipv4Addr([params[0], params[1], params[2], params[3]]);
    let dst_port = u16::from_le_bytes([params[4], params[5]]);
    let src_port = u16::from_le_bytes([params[6], params[7]]);
    let data_ptr = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    let data_len = u32::from_le_bytes([params[12], params[13], params[14], params[15]]);
    let flags = u32::from_le_bytes([params[16], params[17], params[18], params[19]]);

    if data_ptr == 0 || data_len == 0 {
        return 0;
    }
    if data_len > 1472 {
        return u32::MAX;
    } // Max UDP payload (1500 - 20 IP - 8 UDP)

    let data = match copy_user_bytes(data_ptr as u64, data_len as usize, 1472) {
        Some(v) => v,
        None => return u32::MAX,
    };

    let ok = if flags & 1 != 0 {
        // Force broadcast flag — skip SO_BROADCAST check
        crate::net::udp::send_unchecked(dst_ip, src_port, dst_port, &data)
    } else {
        crate::net::udp::send(dst_ip, src_port, dst_port, &data)
    };

    if ok {
        crate::task::scheduler::record_net_tx(data_len as u64);
        data_len
    } else {
        u32::MAX
    }
}

/// sys_udp_recvfrom - Receive a UDP datagram on a bound port.
/// arg1=port, arg2=buf_ptr, arg3=buf_len.
/// Writes header [src_ip:4, src_port:u16, payload_len:u16] (8 bytes) then payload.
/// Returns total bytes written (8 + payload), 0 = no data/timeout, u32::MAX = error.
pub fn sys_udp_recvfrom(port: u32, buf_ptr: u64, buf_len: u32) -> u32 {
    if port == 0 || port > 65535 || buf_ptr == 0 || buf_len < 8 {
        return u32::MAX;
    }

    let port16 = port as u16;
    let timeout_ms = crate::net::udp::get_timeout_ms(port16);

    let dgram = if timeout_ms == 0 {
        // Non-blocking: poll once then try
        crate::net::poll();
        crate::net::udp::recv(port16)
    } else {
        let timeout_ticks = timeout_ms * crate::arch::hal::timer_frequency_hz() as u32 / 1000;
        crate::net::udp::recv_timeout(port16, if timeout_ticks == 0 { 1 } else { timeout_ticks })
    };

    match dgram {
        Some(d) => {
            let payload_len = d.data.len().min((buf_len as usize).saturating_sub(8));
            let total = 8 + payload_len;
            let mut out = alloc::vec![0u8; total];

            // Header: src_ip (4 bytes). IPv6 datagrams are available via
            // SYS_UDP_RECVFROM_V6; keep the legacy ABI IPv4-shaped.
            match d.src_ip {
                crate::net::types::IpAddr::V4(ip) => out[0..4].copy_from_slice(&ip.0),
                crate::net::types::IpAddr::V6(_) => out[0..4].copy_from_slice(&[0; 4]),
            }
            // Header: src_port (u16 LE)
            out[4..6].copy_from_slice(&d.src_port.to_le_bytes());
            // Header: payload_len (u16 LE)
            out[6..8].copy_from_slice(&(payload_len as u16).to_le_bytes());
            // Payload
            out[8..8 + payload_len].copy_from_slice(&d.data[..payload_len]);

            if !copy_to_user_bytes(buf_ptr, &out, total) {
                return u32::MAX;
            }

            crate::task::scheduler::record_net_rx(payload_len as u64);
            total as u32
        }
        None => 0,
    }
}

/// sys_udp_sendto_v6 - Send a UDP datagram over IPv6.
/// arg1=params_ptr: [dst_ip:16, dst_port:u16, src_port:u16, data_ptr:u32, data_len:u32] = 28 bytes
/// Returns bytes sent or u32::MAX on error.
pub fn sys_udp_sendto_v6(params_ptr: u64) -> u32 {
    if params_ptr == 0 {
        return u32::MAX;
    }
    let params = match copy_user_bytes(params_ptr, 28, 28) {
        Some(v) => v,
        None => return u32::MAX,
    };

    let mut dst = [0u8; 16];
    dst.copy_from_slice(&params[0..16]);
    let dst_port = u16::from_le_bytes([params[16], params[17]]);
    let src_port = u16::from_le_bytes([params[18], params[19]]);
    let data_ptr = u32::from_le_bytes([params[20], params[21], params[22], params[23]]);
    let data_len = u32::from_le_bytes([params[24], params[25], params[26], params[27]]);

    if data_ptr == 0 || data_len == 0 {
        return 0;
    }
    if data_len > 1452 {
        return u32::MAX;
    }

    let data = match copy_user_bytes(data_ptr as u64, data_len as usize, 1452) {
        Some(v) => v,
        None => return u32::MAX,
    };
    if crate::net::udp::send_v6(crate::net::types::Ipv6Addr(dst), src_port, dst_port, &data) {
        crate::task::scheduler::record_net_tx(data_len as u64);
        data_len
    } else {
        u32::MAX
    }
}

/// sys_udp_recvfrom_v6 - Receive a UDP datagram on a bound port.
/// Writes [src_ip:16, src_port:u16, payload_len:u16] then payload.
pub fn sys_udp_recvfrom_v6(port: u32, buf_ptr: u64, buf_len: u32) -> u32 {
    if port == 0 || port > 65535 || buf_ptr == 0 || buf_len < 20 {
        return u32::MAX;
    }

    let port16 = port as u16;
    let timeout_ms = crate::net::udp::get_timeout_ms(port16);
    let dgram = if timeout_ms == 0 {
        crate::net::poll();
        crate::net::udp::recv(port16)
    } else {
        let timeout_ticks = timeout_ms * crate::arch::hal::timer_frequency_hz() as u32 / 1000;
        crate::net::udp::recv_timeout(port16, if timeout_ticks == 0 { 1 } else { timeout_ticks })
    };

    match dgram {
        Some(d) => {
            let payload_len = d.data.len().min((buf_len as usize).saturating_sub(20));
            let total = 20 + payload_len;
            let mut out = alloc::vec![0u8; total];

            match d.src_ip {
                crate::net::types::IpAddr::V6(ip) => out[0..16].copy_from_slice(&ip.0),
                crate::net::types::IpAddr::V4(ip) => {
                    out[0..10].fill(0);
                    out[10] = 0xff;
                    out[11] = 0xff;
                    out[12..16].copy_from_slice(&ip.0);
                }
            }
            out[16..18].copy_from_slice(&d.src_port.to_le_bytes());
            out[18..20].copy_from_slice(&(payload_len as u16).to_le_bytes());
            out[20..20 + payload_len].copy_from_slice(&d.data[..payload_len]);

            if !copy_to_user_bytes(buf_ptr, &out, total) {
                return u32::MAX;
            }

            crate::task::scheduler::record_net_rx(payload_len as u64);
            total as u32
        }
        None => 0,
    }
}

/// sys_udp_set_opt - Set a per-port socket option.
/// arg1=port, arg2=opt (1=SO_BROADCAST, 2=SO_RCVTIMEO), arg3=val.
/// Returns 0 on success, u32::MAX on error.
pub fn sys_udp_set_opt(port: u32, opt: u32, val: u32) -> u32 {
    if port == 0 || port > 65535 {
        return u32::MAX;
    }
    if crate::net::udp::set_opt(port as u16, opt, val) {
        0
    } else {
        u32::MAX
    }
}

/// sys_udp_list - List all bound UDP ports.
/// arg1=buf_ptr, arg2=max_entries. Each entry is 8 bytes:
///   [port:u16, owner_tid:u16, recv_queue_len:u16, pad:u16]
/// Returns number of entries written.
pub fn sys_udp_list(buf_ptr: u64, max_entries: u32) -> u32 {
    if buf_ptr == 0 || max_entries == 0 {
        return 0;
    }
    let bindings = crate::net::udp::list_bindings();
    let count = bindings.len().min(max_entries as usize);
    if count == 0 {
        return 0;
    }
    let mut buf = alloc::vec![0u8; count * 8];

    for (i, info) in bindings.iter().take(count).enumerate() {
        let off = i * 8;
        let port_bytes = info.port.to_le_bytes();
        buf[off] = port_bytes[0];
        buf[off + 1] = port_bytes[1];
        let tid_bytes = (info.owner_tid as u16).to_le_bytes();
        buf[off + 2] = tid_bytes[0];
        buf[off + 3] = tid_bytes[1];
        let qlen_bytes = info.recv_queue_len.to_le_bytes();
        buf[off + 4] = qlen_bytes[0];
        buf[off + 5] = qlen_bytes[1];
        buf[off + 6] = 0;
        buf[off + 7] = 0;
    }

    if !copy_to_user_bytes(buf_ptr, &buf, count * 8) {
        return 0;
    }
    count as u32
}

/// sys_net_stats - Get network protocol statistics.
/// arg1=buf_ptr, arg2=buf_size (must be >= 104).
/// Buffer layout (all little-endian):
///   [0..8]   rx_packets (u64)     — NIC
///   [8..16]  tx_packets (u64)
///   [16..24] rx_bytes (u64)
///   [24..32] tx_bytes (u64)
///   [32..40] rx_errors (u64)
///   [40..48] tx_errors (u64)
///   [48..56] tcp_active_opens (u64)
///   [56..64] tcp_passive_opens (u64)
///   [64..72] tcp_segments_sent (u64)
///   [72..80] tcp_segments_recv (u64)
///   [80..88] tcp_retransmits (u64)
///   [88..96] tcp_resets_sent (u64)
///   [96..100] tcp_curr_established (u32)
///   [100..104] tcp_conn_errors_lo (u32)
/// Returns 0 on success.
pub fn sys_net_stats(buf_ptr: u64, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size < 104 {
        return u32::MAX;
    }
    let mut buf = [0u8; 104];

    // NIC stats
    #[cfg(target_arch = "x86_64")]
    let (mut rxp, mut txp, mut rxb, mut txb, rxe, txe) = crate::drivers::network::get_stats();
    #[cfg(target_arch = "aarch64")]
    let (mut rxp, mut txp, mut rxb, mut txb, rxe, txe): (u64, u64, u64, u64, u64, u64) =
        (0, 0, 0, 0, 0, 0);
    let io = crate::net::io_stats();
    if rxb == 0 && io.rx_bytes > 0 {
        rxp = io.rx_packets;
        rxb = io.rx_bytes;
    }
    if txb == 0 && io.tx_bytes > 0 {
        txp = io.tx_packets;
        txb = io.tx_bytes;
    }
    buf[0..8].copy_from_slice(&rxp.to_le_bytes());
    buf[8..16].copy_from_slice(&txp.to_le_bytes());
    buf[16..24].copy_from_slice(&rxb.to_le_bytes());
    buf[24..32].copy_from_slice(&txb.to_le_bytes());
    buf[32..40].copy_from_slice(&rxe.to_le_bytes());
    buf[40..48].copy_from_slice(&txe.to_le_bytes());

    // TCP stats
    let ts = crate::net::tcp::get_stats();
    buf[48..56].copy_from_slice(&ts.active_opens.to_le_bytes());
    buf[56..64].copy_from_slice(&ts.passive_opens.to_le_bytes());
    buf[64..72].copy_from_slice(&ts.segments_sent.to_le_bytes());
    buf[72..80].copy_from_slice(&ts.segments_recv.to_le_bytes());
    buf[80..88].copy_from_slice(&ts.retransmits.to_le_bytes());
    buf[88..96].copy_from_slice(&ts.resets_sent.to_le_bytes());
    buf[96..100].copy_from_slice(&ts.curr_established.to_le_bytes());
    buf[100..104].copy_from_slice(&(ts.conn_errors as u32).to_le_bytes());

    if !copy_to_user_bytes(buf_ptr, &buf, 104) {
        return u32::MAX;
    }
    0
}

/// sys_pipe_bytes_available — Non-blocking poll of a pipe read-end FD.
///
/// `fd` must be a `FdKind::PipeRead` in the calling thread's FD table.
///
/// Return values (mirror `SYS_TCP_RECV_AVAILABLE` convention for libc parity):
/// - `> 0`        — that many bytes are ready to read from the pipe
/// - `0`          — pipe is open but currently empty (no data yet)
/// - `u32::MAX-1` — EOF: pipe is empty **and** all write ends are closed
/// - `u32::MAX`   — FD is not a pipe read-end (regular file, Tty, or invalid)
///                  libc `poll()` treats this as "always readable" for files.
pub fn sys_pipe_bytes_available(fd: u32) -> u32 {
    use crate::fs::fd_table::FdKind;
    let entry = match crate::task::scheduler::current_fd_get(fd) {
        Some(e) => e,
        None => return u32::MAX, // FD not open
    };
    match entry.kind {
        FdKind::PipeRead { pipe_id } => {
            let avail = crate::ipc::anon_pipe::bytes_available(pipe_id);
            if avail > 0 {
                avail
            } else if crate::ipc::anon_pipe::is_write_closed(pipe_id) {
                u32::MAX - 1 // EOF sentinel
            } else {
                0 // pipe open but empty
            }
        }
        // Regular files, Tty, write-end pipes — poll() treats these as always ready
        _ => u32::MAX,
    }
}

/// sys_net_arp - Get ARP table. arg1=buf_ptr, arg2=buf_size
/// Each entry: [ip:4, mac:6, pad:2] = 12 bytes. Returns entry count.
pub fn sys_net_arp(buf_ptr: u64, buf_size: u32) -> u32 {
    let entries = crate::net::arp::entries();
    if buf_ptr != 0 && buf_size > 0 {
        let max = (buf_size / 12) as usize;
        let n = entries.len().min(max);
        if n > 0 {
            let mut out = alloc::vec![0u8; n * 12];
            for (i, (ip, mac)) in entries.iter().enumerate().take(n) {
                let off = i * 12;
                out[off..off + 4].copy_from_slice(&ip.0);
                out[off + 4..off + 10].copy_from_slice(&mac.0);
                // out[off+10], out[off+11] already 0
            }
            if !copy_to_user_bytes(buf_ptr, &out, n * 12) {
                return 0;
            }
        }
    }
    entries.len() as u32
}

// =========================================================================
// WiFi (SYS_WIFI)
// =========================================================================
//
// arg1 = cmd, arg2 = buf_ptr, arg3 = buf_len
//
// Commands:
//   0  WIFI_AVAILABLE    — returns 1 if a WiFi driver is registered, else 0
//   1  WIFI_STATE        — returns current state code:
//                          0=Disconnected, 1=Scanning, 2=Associating,
//                          3=Authenticating, 4=Connected
//   2  WIFI_SCAN         — start a new scan; returns 0
//   3  WIFI_SCAN_RESULTS — read scan results into buf; each entry 48 bytes:
//                          [bssid:6, ssid:32, ssid_len:1, channel:1, rssi:1(i8), security:1, pad:6]
//                          Returns count of entries written.
//   4  WIFI_CONNECT      — connect; buf = [ssid_len:1, ssid:32, pw_len:1, pw:64]
//                          Returns 0 on success (connection is async).
//   5  WIFI_DISCONNECT   — disconnect; returns 0
//   6  WIFI_STATUS       — 48-byte status buf:
//                          [state:1, connected:1, channel:1, rssi:1, pad:2,
//                           bssid:6, ssid:32, ssid_len:1, pad:3]

/// sys_wifi — WiFi management syscall.
/// arg1=cmd, arg2=buf_ptr, arg3=buf_len
pub fn sys_wifi(cmd: u32, buf_ptr: u64, _buf_len: u32) -> u32 {
    match cmd {
        // 0 — is WiFi hardware available?
        0 => {
            if crate::drivers::network::wifi_available() {
                1
            } else {
                0
            }
        }

        // 1 — get WiFi state code
        1 => {
            use crate::net::wifi::WifiState;
            match crate::net::wifi::get_state() {
                WifiState::Disconnected => 0,
                WifiState::Scanning => 1,
                WifiState::Associating { .. } => 2,
                WifiState::Authenticating { .. } => 3,
                WifiState::Connected { .. } => 4,
            }
        }

        // 2 — start scan
        2 => {
            // Sets WiFi state to Scanning; the RTL8188EU poll thread will
            // detect the Scanning state and perform an active channel sweep.
            crate::net::wifi::start_scan();
            0
        }

        // 3 — read scan results
        3 => {
            if buf_ptr == 0 {
                return 0;
            }
            let results = crate::net::wifi::get_scan_results();
            let max = if _buf_len > 0 {
                (_buf_len as usize / 48).min(results.len())
            } else {
                results.len()
            };
            let count = max;
            if count == 0 {
                return 0;
            }
            let mut buf = alloc::vec![0u8; count * 48];
            for (i, bss) in results.iter().take(count).enumerate() {
                let off = i * 48;
                buf[off..off + 6].copy_from_slice(&bss.bssid);
                buf[off + 6..off + 38].copy_from_slice(&bss.ssid);
                buf[off + 38] = bss.ssid_len as u8;
                buf[off + 39] = bss.channel;
                buf[off + 40] = bss.rssi as u8;
                buf[off + 41] = match bss.security {
                    crate::net::wifi::WifiSecurity::Open => 0,
                    crate::net::wifi::WifiSecurity::Wpa2Personal => 1,
                };
                // pad [42..48] already zero
            }
            if !copy_to_user_bytes(buf_ptr, &buf, count * 48) {
                return 0;
            }
            count as u32
        }

        // 4 — connect to a network
        4 => {
            if buf_ptr == 0 {
                return u32::MAX;
            }
            // buf layout: [ssid_len:1, ssid:32, pw_len:1, pw:64] = 98 bytes
            let raw = match copy_user_bytes(buf_ptr, 98, 98) {
                Some(v) => v,
                None => return u32::MAX,
            };
            let ssid_len = raw[0] as usize;
            if ssid_len > 32 {
                return u32::MAX;
            }
            let ssid = &raw[1..1 + ssid_len];
            let pw_len = raw[33] as usize;
            if pw_len > 64 {
                return u32::MAX;
            }
            let pw = &raw[34..34 + pw_len];
            crate::net::wifi::connect(ssid, pw);
            0
        }

        // 5 — disconnect
        5 => {
            crate::net::wifi::disconnect();
            0
        }

        // 6 — get connection status (48-byte struct)
        6 => {
            if buf_ptr == 0 {
                return u32::MAX;
            }
            use crate::net::wifi::WifiState;
            let state = crate::net::wifi::get_state();
            let mut buf = [0u8; 48];
            match &state {
                WifiState::Disconnected => {
                    buf[0] = 0;
                    buf[1] = 0;
                }
                WifiState::Scanning => {
                    buf[0] = 1;
                    buf[1] = 0;
                }
                WifiState::Associating {
                    bssid,
                    ssid,
                    ssid_len,
                    channel,
                } => {
                    buf[0] = 2;
                    buf[1] = 0;
                    buf[2] = *channel;
                    buf[4..10].copy_from_slice(bssid);
                    buf[10..42].copy_from_slice(ssid);
                    buf[42] = *ssid_len as u8;
                }
                WifiState::Authenticating {
                    bssid,
                    ssid,
                    ssid_len,
                } => {
                    buf[0] = 3;
                    buf[1] = 0;
                    buf[4..10].copy_from_slice(bssid);
                    buf[10..42].copy_from_slice(ssid);
                    buf[42] = *ssid_len as u8;
                }
                WifiState::Connected {
                    bssid,
                    ssid,
                    ssid_len,
                    channel,
                } => {
                    buf[0] = 4;
                    buf[1] = 1;
                    buf[2] = *channel;
                    buf[4..10].copy_from_slice(bssid);
                    buf[10..42].copy_from_slice(ssid);
                    buf[42] = *ssid_len as u8;
                }
            }
            if !copy_to_user_bytes(buf_ptr, &buf, 48) {
                return u32::MAX;
            }
            0
        }

        _ => u32::MAX,
    }
}

// =========================================================================
// IPv6 Networking
// =========================================================================

/// sys_net_ping6 - ICMPv6 ping. arg1=ip6_ptr(16 bytes), arg2=seq, arg3=timeout_ticks
/// Returns RTT in ticks, or u32::MAX on timeout.
pub fn sys_net_ping6(ip_ptr: u64, seq: u32, timeout: u32) -> u32 {
    if ip_ptr == 0 {
        return u32::MAX;
    }
    let raw = match copy_user_bytes(ip_ptr, 16, 16) {
        Some(v) => v,
        None => return u32::MAX,
    };
    let mut ip_bytes = [0u8; 16];
    ip_bytes.copy_from_slice(&raw);
    let ip = crate::net::types::Ipv6Addr(ip_bytes);
    match crate::net::icmpv6::ping6(ip, seq as u16, timeout) {
        Some((rtt, _hop_limit)) => rtt,
        None => u32::MAX,
    }
}

/// sys_net_dns6 - DNS AAAA record resolution.
/// arg1=hostname_ptr, arg2=result_ptr (16 bytes for IPv6 address)
/// Returns 0 on success, u32::MAX on error.
pub fn sys_net_dns6(hostname_ptr: u64, result_ptr: u64) -> u32 {
    if hostname_ptr == 0 || result_ptr == 0 {
        return u32::MAX;
    }
    let hostname = unsafe { read_user_str(hostname_ptr) };
    if hostname.is_empty() {
        return u32::MAX;
    }
    match crate::net::dns::resolve_v6(hostname) {
        Ok(addr) => {
            if !copy_to_user_bytes(result_ptr, &addr.0, 16) {
                return u32::MAX;
            }
            0
        }
        Err(_) => u32::MAX,
    }
}

/// sys_tcp_connect_v6 - TCP connect over IPv6.
/// arg1=params_ptr: [ip6:16, port:u16, pad:u16, timeout:u32] = 24 bytes
/// Returns socket ID or u32::MAX on error.
pub fn sys_tcp_connect_v6(params_ptr: u64) -> u32 {
    if params_ptr == 0 {
        return u32::MAX;
    }
    let params = match copy_user_bytes(params_ptr, 24, 24) {
        Some(v) => v,
        None => return u32::MAX,
    };
    let mut ip6 = [0u8; 16];
    ip6.copy_from_slice(&params[0..16]);
    let port = u16::from_le_bytes([params[16], params[17]]);
    let timeout = u32::from_le_bytes([params[20], params[21], params[22], params[23]]);
    let timeout_ticks = if timeout > 0 {
        timeout * crate::arch::hal::timer_frequency_hz() as u32 / 1000
    } else {
        1000 // Default 10 seconds at 100Hz
    };
    crate::net::tcp::connect_v6(crate::net::types::Ipv6Addr(ip6), port, timeout_ticks)
}
