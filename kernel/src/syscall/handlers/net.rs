//! Networking syscall handlers.
//!
//! Covers general network config, TCP, UDP, DNS, DHCP, ARP, and
//! network polling/statistics.

use super::helpers::read_user_str;
#[allow(unused_imports)]
use super::helpers::is_valid_user_ptr;

// =========================================================================
// Networking (SYS_NET_*)
// =========================================================================

/// sys_net_config - Get or set network configuration.
/// arg1=cmd (0=get, 1=set), arg2=buf_ptr (24 bytes: ip4+mask4+gw4+dns4+mac6+link1+pad1)
pub fn sys_net_config(cmd: u32, buf_ptr: u32) -> u32 {
    match cmd {
        0 => {
            if buf_ptr == 0 { return u32::MAX; }
            let cfg = crate::net::config();
            #[cfg(target_arch = "x86_64")]
            let link_up = crate::drivers::network::e1000::is_link_up();
            #[cfg(target_arch = "aarch64")]
            let link_up = false;
            unsafe {
                let buf = buf_ptr as *mut u8;
                core::ptr::copy_nonoverlapping(cfg.ip.0.as_ptr(), buf, 4);
                core::ptr::copy_nonoverlapping(cfg.mask.0.as_ptr(), buf.add(4), 4);
                core::ptr::copy_nonoverlapping(cfg.gateway.0.as_ptr(), buf.add(8), 4);
                core::ptr::copy_nonoverlapping(cfg.dns.0.as_ptr(), buf.add(12), 4);
                core::ptr::copy_nonoverlapping(cfg.mac.0.as_ptr(), buf.add(16), 6);
                *buf.add(22) = if link_up { 1 } else { 0 };
                *buf.add(23) = 0;
            }
            0
        }
        1 => {
            if buf_ptr == 0 { return u32::MAX; }
            unsafe {
                let buf = buf_ptr as *const u8;
                let mut ip = [0u8; 4]; let mut mask = [0u8; 4];
                let mut gw = [0u8; 4]; let mut dns = [0u8; 4];
                core::ptr::copy_nonoverlapping(buf, ip.as_mut_ptr(), 4);
                core::ptr::copy_nonoverlapping(buf.add(4), mask.as_mut_ptr(), 4);
                core::ptr::copy_nonoverlapping(buf.add(8), gw.as_mut_ptr(), 4);
                core::ptr::copy_nonoverlapping(buf.add(12), dns.as_mut_ptr(), 4);
                crate::net::set_config(
                    crate::net::types::Ipv4Addr(ip), crate::net::types::Ipv4Addr(mask),
                    crate::net::types::Ipv4Addr(gw), crate::net::types::Ipv4Addr(dns),
                );
            }
            0
        }
        2 => {
            // Disable NIC
            #[cfg(target_arch = "x86_64")]
            crate::drivers::network::e1000::set_enabled(false);
            0
        }
        3 => {
            // Enable NIC
            #[cfg(target_arch = "x86_64")]
            crate::drivers::network::e1000::set_enabled(true);
            0
        }
        4 => {
            // Query enabled state
            #[cfg(target_arch = "x86_64")]
            { if crate::drivers::network::e1000::is_enabled() { 1 } else { 0 } }
            #[cfg(target_arch = "aarch64")]
            { 0 }
        }
        5 => {
            // Query hardware availability
            #[cfg(target_arch = "x86_64")]
            { if crate::drivers::network::e1000::is_available() { 1 } else { 0 } }
            #[cfg(target_arch = "aarch64")]
            { 0 }
        }
        6 => {
            // Reload hosts file from disk
            crate::net::dns::load_hosts();
            0
        }
        7 => {
            // Get interface configs. buf_ptr = output buffer, must hold N*64 bytes.
            // Returns number of interfaces written.
            if buf_ptr == 0 { return u32::MAX; }
            // Assume caller provides buffer for up to 8 interfaces (512 bytes)
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, 8 * 64) };
            crate::net::interfaces::serialize_configs(buf)
        }
        8 => {
            // Set interface configs and save to disk.
            // buf_ptr points to: [count:u32, entries: count*64 bytes]
            if buf_ptr == 0 { return u32::MAX; }
            let count = unsafe { *(buf_ptr as *const u32) };
            if count == 0 || count > 8 { return u32::MAX; }
            let data_ptr = (buf_ptr + 4) as *const u8;
            let data = unsafe { core::slice::from_raw_parts(data_ptr, count as usize * 64) };
            crate::net::interfaces::apply_and_save(data, count)
        }
        9 => {
            // Get NIC driver name. buf_ptr = output buffer (up to 64 bytes).
            // Returns name length, or 0 if no NIC.
            if buf_ptr == 0 { return 0; }
            #[cfg(target_arch = "x86_64")]
            {
                if let Some(name) = crate::drivers::network::with_net(|d| {
                    let n = d.name();
                    let bytes = n.as_bytes();
                    let len = bytes.len().min(64);
                    unsafe {
                        core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf_ptr as *mut u8, len);
                    }
                    len as u32
                }) {
                    name
                } else {
                    0
                }
            }
            #[cfg(target_arch = "aarch64")]
            { 0 }
        }
        _ => u32::MAX,
    }
}

/// sys_net_ping - ICMP ping. arg1=ip_ptr(4 bytes), arg2=seq, arg3=timeout_ticks
/// Returns RTT in ticks, or u32::MAX on timeout.
pub fn sys_net_ping(ip_ptr: u32, seq: u32, timeout: u32) -> u32 {
    if ip_ptr == 0 { return u32::MAX; }
    let mut ip_bytes = [0u8; 4];
    unsafe { core::ptr::copy_nonoverlapping(ip_ptr as *const u8, ip_bytes.as_mut_ptr(), 4); }
    let ip = crate::net::types::Ipv4Addr(ip_bytes);
    match crate::net::icmp::ping(ip, seq as u16, timeout) {
        Some((rtt, _ttl)) => rtt,
        None => u32::MAX,
    }
}

/// sys_net_dhcp - DHCP discovery. arg1=buf_ptr (16 bytes: ip+mask+gw+dns)
/// Returns 0 on success, applies config automatically.
pub fn sys_net_dhcp(buf_ptr: u32) -> u32 {
    match crate::net::dhcp::discover() {
        Ok(result) => {
            crate::net::set_config(result.ip, result.mask, result.gateway, result.dns);
            if buf_ptr != 0 {
                unsafe {
                    let buf = buf_ptr as *mut u8;
                    core::ptr::copy_nonoverlapping(result.ip.0.as_ptr(), buf, 4);
                    core::ptr::copy_nonoverlapping(result.mask.0.as_ptr(), buf.add(4), 4);
                    core::ptr::copy_nonoverlapping(result.gateway.0.as_ptr(), buf.add(8), 4);
                    core::ptr::copy_nonoverlapping(result.dns.0.as_ptr(), buf.add(12), 4);
                }
            }
            0
        }
        Err(_) => u32::MAX,
    }
}

/// sys_net_dns - DNS resolve. arg1=hostname_ptr, arg2=result_ptr(4 bytes)
pub fn sys_net_dns(hostname_ptr: u32, result_ptr: u32) -> u32 {
    let hostname = unsafe { read_user_str(hostname_ptr) };
    match crate::net::dns::resolve(hostname) {
        Ok(ip) => {
            if result_ptr != 0 {
                unsafe { core::ptr::copy_nonoverlapping(ip.0.as_ptr(), result_ptr as *mut u8, 4); }
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
pub fn sys_tcp_connect(params_ptr: u32) -> u32 {
    if params_ptr == 0 { return u32::MAX; }
    let params = unsafe { core::slice::from_raw_parts(params_ptr as *const u8, 12) };
    let ip = crate::net::types::Ipv4Addr([params[0], params[1], params[2], params[3]]);
    let port = u16::from_le_bytes([params[4], params[5]]);
    let timeout = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    let pit_hz = crate::arch::hal::timer_frequency_hz() as u32;
    let timeout_ticks = if timeout == 0 { pit_hz } else { timeout * pit_hz / 1000 };
    crate::net::tcp::connect(ip, port, timeout_ticks)
}

/// sys_tcp_send - Send data on TCP connection.
/// arg1=socket_id, arg2=buf_ptr, arg3=len
/// Returns bytes sent or u32::MAX on error.
pub fn sys_tcp_send(socket_id: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 { return 0; }
    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
    crate::net::tcp::send(socket_id, buf, 1000) // 10s timeout
}

/// sys_tcp_recv - Receive data from TCP connection.
/// arg1=socket_id, arg2=buf_ptr, arg3=len
/// Returns bytes received, 0=EOF, u32::MAX=error.
pub fn sys_tcp_recv(socket_id: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 { return u32::MAX; }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
    crate::net::tcp::recv(socket_id, buf, 3000) // 30s timeout
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
    if port == 0 || port > 65535 { return u32::MAX; }
    crate::net::tcp::listen(port as u16, backlog.min(16) as u16)
}

/// sys_tcp_accept - Accept a connection from a listening socket.
/// arg1=listener_id, arg2=result_ptr (12 bytes: [socket_id:u32, ip:[u8;4], port:u16, pad:u16])
/// Returns 0 on success, u32::MAX on timeout/error.
pub fn sys_tcp_accept(listener_id: u32, result_ptr: u32) -> u32 {
    if result_ptr == 0 { return u32::MAX; }
    let pit_hz = crate::arch::hal::timer_frequency_hz() as u32;
    let timeout_ticks = 30 * pit_hz; // 30 second timeout
    let (sock_id, remote_ip, remote_port) = crate::net::tcp::accept(listener_id, timeout_ticks);
    if sock_id == u32::MAX {
        return u32::MAX;
    }
    // Write result to user buffer
    let result = unsafe { core::slice::from_raw_parts_mut(result_ptr as *mut u8, 12) };
    let sid_bytes = sock_id.to_le_bytes();
    result[0..4].copy_from_slice(&sid_bytes);
    result[4..8].copy_from_slice(remote_ip.as_bytes());
    let port_bytes = remote_port.to_le_bytes();
    result[8..10].copy_from_slice(&port_bytes);
    result[10] = 0;
    result[11] = 0;
    0
}

/// sys_tcp_list - List all TCP connections.
/// arg1=buf_ptr, arg2=max_entries. Each entry is 16 bytes:
///   [local_ip:4, local_port:u16, remote_ip:4, remote_port:u16, state:u8, owner_tid_lo:u8, recv_buf_hi:u16]
/// Returns number of entries written.
pub fn sys_tcp_list(buf_ptr: u32, max_entries: u32) -> u32 {
    if buf_ptr == 0 || max_entries == 0 { return 0; }
    let conns = crate::net::tcp::list_connections();
    let count = conns.len().min(max_entries as usize);
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, count * 16) };

    for (i, info) in conns.iter().take(count).enumerate() {
        let off = i * 16;
        buf[off..off+4].copy_from_slice(info.local_ip.as_bytes());
        let lp = info.local_port.to_be_bytes();
        buf[off+4] = lp[0];
        buf[off+5] = lp[1];
        buf[off+6..off+10].copy_from_slice(info.remote_ip.as_bytes());
        let rp = info.remote_port.to_be_bytes();
        buf[off+10] = rp[0];
        buf[off+11] = rp[1];
        buf[off+12] = info.state as u8;
        buf[off+13] = (info.owner_tid & 0xFF) as u8;
        let recv_len = (info.recv_buf_len as u16).to_le_bytes();
        buf[off+14] = recv_len[0];
        buf[off+15] = recv_len[1];
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
    if port == 0 || port > 65535 { return u32::MAX; }
    if crate::net::udp::bind(port as u16) { 0 } else { u32::MAX }
}

/// sys_udp_unbind - Unbind a UDP port.
/// arg1=port. Returns 0.
pub fn sys_udp_unbind(port: u32) -> u32 {
    if port > 65535 { return u32::MAX; }
    crate::net::udp::unbind(port as u16);
    0
}

/// sys_udp_sendto - Send a UDP datagram.
/// arg1=params_ptr: [dst_ip:4, dst_port:u16, src_port:u16, data_ptr:u32, data_len:u32, flags:u32] = 20 bytes
/// flags: bit 0 = force broadcast (bypass SO_BROADCAST check).
/// Returns bytes sent or u32::MAX on error.
pub fn sys_udp_sendto(params_ptr: u32) -> u32 {
    if params_ptr == 0 { return u32::MAX; }
    let params = unsafe { core::slice::from_raw_parts(params_ptr as *const u8, 20) };

    let dst_ip = crate::net::types::Ipv4Addr([params[0], params[1], params[2], params[3]]);
    let dst_port = u16::from_le_bytes([params[4], params[5]]);
    let src_port = u16::from_le_bytes([params[6], params[7]]);
    let data_ptr = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    let data_len = u32::from_le_bytes([params[12], params[13], params[14], params[15]]);
    let flags = u32::from_le_bytes([params[16], params[17], params[18], params[19]]);

    if data_ptr == 0 || data_len == 0 { return 0; }
    if data_len > 1472 { return u32::MAX; } // Max UDP payload (1500 - 20 IP - 8 UDP)

    let data = unsafe { core::slice::from_raw_parts(data_ptr as *const u8, data_len as usize) };

    let ok = if flags & 1 != 0 {
        // Force broadcast flag — skip SO_BROADCAST check
        crate::net::udp::send_unchecked(dst_ip, src_port, dst_port, data)
    } else {
        crate::net::udp::send(dst_ip, src_port, dst_port, data)
    };

    if ok { data_len } else { u32::MAX }
}

/// sys_udp_recvfrom - Receive a UDP datagram on a bound port.
/// arg1=port, arg2=buf_ptr, arg3=buf_len.
/// Writes header [src_ip:4, src_port:u16, payload_len:u16] (8 bytes) then payload.
/// Returns total bytes written (8 + payload), 0 = no data/timeout, u32::MAX = error.
pub fn sys_udp_recvfrom(port: u32, buf_ptr: u32, buf_len: u32) -> u32 {
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
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize) };

            // Header: src_ip (4 bytes)
            buf[0..4].copy_from_slice(&d.src_ip.0);
            // Header: src_port (u16 LE)
            buf[4..6].copy_from_slice(&d.src_port.to_le_bytes());
            // Header: payload_len (u16 LE)
            buf[6..8].copy_from_slice(&(payload_len as u16).to_le_bytes());
            // Payload
            buf[8..8 + payload_len].copy_from_slice(&d.data[..payload_len]);

            total as u32
        }
        None => 0,
    }
}

/// sys_udp_set_opt - Set a per-port socket option.
/// arg1=port, arg2=opt (1=SO_BROADCAST, 2=SO_RCVTIMEO), arg3=val.
/// Returns 0 on success, u32::MAX on error.
pub fn sys_udp_set_opt(port: u32, opt: u32, val: u32) -> u32 {
    if port == 0 || port > 65535 { return u32::MAX; }
    if crate::net::udp::set_opt(port as u16, opt, val) { 0 } else { u32::MAX }
}

/// sys_udp_list - List all bound UDP ports.
/// arg1=buf_ptr, arg2=max_entries. Each entry is 8 bytes:
///   [port:u16, owner_tid:u16, recv_queue_len:u16, pad:u16]
/// Returns number of entries written.
pub fn sys_udp_list(buf_ptr: u32, max_entries: u32) -> u32 {
    if buf_ptr == 0 || max_entries == 0 { return 0; }
    let bindings = crate::net::udp::list_bindings();
    let count = bindings.len().min(max_entries as usize);
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, count * 8) };

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
pub fn sys_net_stats(buf_ptr: u32, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size < 104 { return u32::MAX; }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, 104) };

    // NIC stats
    #[cfg(target_arch = "x86_64")]
    let (rxp, txp, rxb, txb, rxe, txe) = crate::drivers::network::e1000::get_stats();
    #[cfg(target_arch = "aarch64")]
    let (rxp, txp, rxb, txb, rxe, txe): (u64, u64, u64, u64, u64, u64) = (0, 0, 0, 0, 0, 0);
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
pub fn sys_net_arp(buf_ptr: u32, buf_size: u32) -> u32 {
    let entries = crate::net::arp::entries();
    if buf_ptr != 0 && buf_size > 0 {
        let max = (buf_size / 12) as usize;
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        for (i, (ip, mac)) in entries.iter().enumerate().take(max) {
            let off = i * 12;
            buf[off..off + 4].copy_from_slice(&ip.0);
            buf[off + 4..off + 10].copy_from_slice(&mac.0);
            buf[off + 10] = 0;
            buf[off + 11] = 0;
        }
    }
    entries.len() as u32
}
