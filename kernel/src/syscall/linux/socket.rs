use super::*;
use crate::fs::fd_table::FdKind;
use crate::net::types::{IpAddr, Ipv4Addr, Ipv6Addr};
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

const MAX_LINUX_SOCKETS: usize = 256;

const AF_UNIX: u64 = 1;
const AF_INET: u64 = 2;
const AF_INET6: u64 = 10;

const SOCK_TYPE_MASK: u64 = 0xF;
const SOCK_STREAM: u64 = 1;
const SOCK_DGRAM: u64 = 2;
const SOCK_NONBLOCK: u64 = 0x800;
const SOCK_CLOEXEC: u64 = 0x80000;

const IPPROTO_TCP: u64 = 6;
const IPPROTO_UDP: u64 = 17;

const SOL_SOCKET: u64 = 1;
const SO_ERROR: u64 = 4;
const SO_RCVTIMEO: u64 = 20;
const SO_SNDTIMEO: u64 = 21;

const EPHEMERAL_FIRST: u16 = 49152;
const EPHEMERAL_LAST: u16 = 60999;
static NEXT_EPHEMERAL: AtomicU32 = AtomicU32::new(EPHEMERAL_FIRST as u32);

#[derive(Clone, Copy, PartialEq, Eq)]
enum LinuxSocketKind {
    Empty,
    InetStream,
    InetDatagram,
    Inet6Stream,
    Inet6Datagram,
}

#[derive(Clone, Copy)]
enum LinuxSocketState {
    Empty,
    New,
    TcpBound {
        local_port: u16,
    },
    TcpListener {
        listener_id: u32,
        local_port: u16,
    },
    TcpConnecting {
        tcp_id: u32,
        remote_ip: [u8; 4],
        remote_port: u16,
    },
    TcpConnectingV6 {
        tcp_id: u32,
        remote_ip: [u8; 16],
        remote_port: u16,
    },
    TcpConnected {
        tcp_id: u32,
        remote_ip: [u8; 4],
        remote_port: u16,
    },
    TcpConnectedV6 {
        tcp_id: u32,
        remote_ip: [u8; 16],
        remote_port: u16,
    },
    TcpConnectFailed {
        errno: i32,
    },
    Udp {
        local_port: u16,
        remote_ip: [u8; 4],
        remote_port: u16,
        connected: bool,
    },
    UdpV6 {
        local_port: u16,
        remote_ip: [u8; 16],
        remote_port: u16,
        connected: bool,
    },
}

#[derive(Clone, Copy)]
struct LinuxSocketEntry {
    in_use: bool,
    refs: u16,
    kind: LinuxSocketKind,
    protocol: u16,
    state: LinuxSocketState,
}

impl LinuxSocketEntry {
    const EMPTY: Self = Self {
        in_use: false,
        refs: 0,
        kind: LinuxSocketKind::Empty,
        protocol: 0,
        state: LinuxSocketState::Empty,
    };
}

static LINUX_SOCKETS: Spinlock<[LinuxSocketEntry; MAX_LINUX_SOCKETS]> =
    Spinlock::new([LinuxSocketEntry::EMPTY; MAX_LINUX_SOCKETS]);

pub(super) fn linux_socket(domain: u64, type_: u64, protocol: u64) -> u64 {
    let sock_type = type_ & SOCK_TYPE_MASK;
    let kind = match (domain, sock_type) {
        (AF_INET, SOCK_STREAM) if protocol == 0 || protocol == IPPROTO_TCP => {
            LinuxSocketKind::InetStream
        }
        (AF_INET, SOCK_DGRAM) if protocol == 0 || protocol == IPPROTO_UDP => {
            LinuxSocketKind::InetDatagram
        }
        (AF_INET, _) => return linux_err(EPROTONOSUPPORT),
        (AF_INET6, SOCK_STREAM) if protocol == 0 || protocol == IPPROTO_TCP => {
            LinuxSocketKind::Inet6Stream
        }
        (AF_INET6, SOCK_DGRAM) if protocol == 0 || protocol == IPPROTO_UDP => {
            LinuxSocketKind::Inet6Datagram
        }
        (AF_INET6, _) => return linux_err(EPROTONOSUPPORT),
        (AF_UNIX, _) => return linux_err(EAFNOSUPPORT),
        _ => return linux_err(EAFNOSUPPORT),
    };

    let socket_id = match socket_alloc(kind, protocol as u16) {
        Some(id) => id,
        None => return linux_err(ENOMEM),
    };

    let fd = match crate::task::scheduler::current_fd_alloc(FdKind::LinuxSocket { socket_id }) {
        Some(fd) => fd,
        None => {
            socket_decref(socket_id);
            return linux_err(ENOMEM);
        }
    };
    if (type_ & SOCK_CLOEXEC) != 0 {
        crate::task::scheduler::current_fd_set_cloexec(fd, true);
    }
    if (type_ & SOCK_NONBLOCK) != 0 {
        crate::task::scheduler::current_fd_set_nonblock(fd, true);
    }
    fd as u64
}

pub(super) fn linux_connect(fd: u32, addr_ptr: u64, addr_len: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };

    let entry = match socket_entry(socket_id) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    match entry.kind {
        LinuxSocketKind::InetStream => {
            let (remote_ip, remote_port) = match read_sockaddr_in(addr_ptr, addr_len) {
                Ok(addr) => addr,
                Err(errno) => return linux_err(errno),
            };
            match entry.state {
                LinuxSocketState::TcpConnected { .. } => return linux_err(EISCONN),
                LinuxSocketState::TcpConnecting { .. } => return linux_err(EALREADY),
                LinuxSocketState::TcpConnectFailed { errno } => {
                    let _ = socket_set_state(socket_id, LinuxSocketState::New);
                    return linux_err(errno);
                }
                _ => {}
            }

            if fd_nonblock(fd) {
                let tcp_id = crate::net::tcp::connect_start(remote_ip, remote_port);
                if tcp_id == u32::MAX {
                    return linux_err(ECONNREFUSED);
                }
                if !socket_set_state(
                    socket_id,
                    LinuxSocketState::TcpConnecting {
                        tcp_id,
                        remote_ip: remote_ip.0,
                        remote_port,
                    },
                ) {
                    crate::net::tcp::close(tcp_id);
                    return linux_err(EBADF);
                }
                return linux_err(EINPROGRESS);
            } else {
                let timeout_ticks = 10 * crate::arch::hal::timer_frequency_hz() as u32;
                let tcp_id = crate::net::tcp::connect(remote_ip, remote_port, timeout_ticks);
                if tcp_id == u32::MAX {
                    return linux_err(ECONNREFUSED);
                }
                if !socket_set_state(
                    socket_id,
                    LinuxSocketState::TcpConnected {
                        tcp_id,
                        remote_ip: remote_ip.0,
                        remote_port,
                    },
                ) {
                    crate::net::tcp::close(tcp_id);
                    return linux_err(EBADF);
                }
                0
            }
        }
        LinuxSocketKind::Inet6Stream => {
            let (remote_ip, remote_port) = match read_sockaddr_in6(addr_ptr, addr_len) {
                Ok(addr) => addr,
                Err(errno) => return linux_err(errno),
            };
            match entry.state {
                LinuxSocketState::TcpConnectedV6 { .. } => return linux_err(EISCONN),
                LinuxSocketState::TcpConnectingV6 { .. } => return linux_err(EALREADY),
                LinuxSocketState::TcpConnectFailed { errno } => {
                    let _ = socket_set_state(socket_id, LinuxSocketState::New);
                    return linux_err(errno);
                }
                _ => {}
            }

            if fd_nonblock(fd) {
                let tcp_id = crate::net::tcp::connect_start_v6(remote_ip, remote_port);
                if tcp_id == u32::MAX {
                    return linux_err(ECONNREFUSED);
                }
                if !socket_set_state(
                    socket_id,
                    LinuxSocketState::TcpConnectingV6 {
                        tcp_id,
                        remote_ip: remote_ip.0,
                        remote_port,
                    },
                ) {
                    crate::net::tcp::close(tcp_id);
                    return linux_err(EBADF);
                }
                return linux_err(EINPROGRESS);
            }

            let timeout_ticks = 10 * crate::arch::hal::timer_frequency_hz() as u32;
            let tcp_id = crate::net::tcp::connect_v6(remote_ip, remote_port, timeout_ticks);
            if tcp_id == u32::MAX {
                return linux_err(ECONNREFUSED);
            }
            if !socket_set_state(
                socket_id,
                LinuxSocketState::TcpConnectedV6 {
                    tcp_id,
                    remote_ip: remote_ip.0,
                    remote_port,
                },
            ) {
                crate::net::tcp::close(tcp_id);
                return linux_err(EBADF);
            }
            0
        }
        LinuxSocketKind::InetDatagram => {
            let (remote_ip, remote_port) = match read_sockaddr_in(addr_ptr, addr_len) {
                Ok(addr) => addr,
                Err(errno) => return linux_err(errno),
            };
            if ensure_udp_bound(socket_id).is_err() {
                return linux_err(ENOMEM);
            }
            if !socket_update_udp_remote(socket_id, remote_ip, remote_port) {
                return linux_err(EBADF);
            }
            0
        }
        LinuxSocketKind::Inet6Datagram => {
            let (remote_ip, remote_port) = match read_sockaddr_in6(addr_ptr, addr_len) {
                Ok(addr) => addr,
                Err(errno) => return linux_err(errno),
            };
            if ensure_udp_bound(socket_id).is_err() {
                return linux_err(ENOMEM);
            }
            if !socket_update_udp_remote_v6(socket_id, remote_ip, remote_port) {
                return linux_err(EBADF);
            }
            0
        }
        LinuxSocketKind::Empty => linux_err(EBADF),
    }
}

pub(super) fn linux_bind(fd: u32, addr_ptr: u64, addr_len: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    let entry = match socket_entry(socket_id) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    match entry.kind {
        LinuxSocketKind::InetStream => {
            let (_ip, port) = match read_sockaddr_in(addr_ptr, addr_len) {
                Ok(addr) => addr,
                Err(errno) => return linux_err(errno),
            };
            if !socket_set_state(socket_id, LinuxSocketState::TcpBound { local_port: port }) {
                return linux_err(EBADF);
            }
            0
        }
        LinuxSocketKind::Inet6Stream => {
            let (_ip, port) = match read_sockaddr_in6(addr_ptr, addr_len) {
                Ok(addr) => addr,
                Err(errno) => return linux_err(errno),
            };
            if !socket_set_state(socket_id, LinuxSocketState::TcpBound { local_port: port }) {
                return linux_err(EBADF);
            }
            0
        }
        LinuxSocketKind::InetDatagram => {
            let (_ip, port) = match read_sockaddr_in(addr_ptr, addr_len) {
                Ok(addr) => addr,
                Err(errno) => return linux_err(errno),
            };
            if !crate::net::udp::bind(port) {
                return linux_err(EAGAIN);
            }
            if !socket_set_state(
                socket_id,
                LinuxSocketState::Udp {
                    local_port: port,
                    remote_ip: [0; 4],
                    remote_port: 0,
                    connected: false,
                },
            ) {
                crate::net::udp::unbind(port);
                return linux_err(EBADF);
            }
            0
        }
        LinuxSocketKind::Inet6Datagram => {
            let (_ip, port) = match read_sockaddr_in6(addr_ptr, addr_len) {
                Ok(addr) => addr,
                Err(errno) => return linux_err(errno),
            };
            if !crate::net::udp::bind(port) {
                return linux_err(EAGAIN);
            }
            if !socket_set_state(
                socket_id,
                LinuxSocketState::UdpV6 {
                    local_port: port,
                    remote_ip: [0; 16],
                    remote_port: 0,
                    connected: false,
                },
            ) {
                crate::net::udp::unbind(port);
                return linux_err(EBADF);
            }
            0
        }
        LinuxSocketKind::Empty => linux_err(EBADF),
    }
}

pub(super) fn linux_listen(fd: u32, backlog: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    let entry = match socket_entry(socket_id) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    let local_port = match entry.state {
        LinuxSocketState::TcpBound { local_port } => local_port,
        LinuxSocketState::TcpListener { .. } => return 0,
        _ => return linux_err(EINVAL),
    };
    let listener_id = crate::net::tcp::listen(local_port, backlog.min(16) as u16);
    if listener_id == u32::MAX {
        return linux_err(EAGAIN);
    }
    if socket_set_state(
        socket_id,
        LinuxSocketState::TcpListener {
            listener_id,
            local_port,
        },
    ) {
        0
    } else {
        crate::net::tcp::close_listener(listener_id);
        linux_err(EBADF)
    }
}

pub(super) fn linux_accept(fd: u32, addr_ptr: u64, addrlen_ptr: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    let entry = match socket_entry(socket_id) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    let listener_id = match entry.state {
        LinuxSocketState::TcpListener { listener_id, .. } => listener_id,
        _ => return linux_err(EINVAL),
    };
    let timeout_ticks = if fd_nonblock(fd) {
        0
    } else {
        30 * crate::arch::hal::timer_frequency_hz() as u32
    };
    if entry.kind == LinuxSocketKind::Inet6Stream {
        let (tcp_id, remote_ip, remote_port) =
            crate::net::tcp::accept_v6(listener_id, timeout_ticks);
        if tcp_id == u32::MAX {
            return linux_err(EAGAIN);
        }
        let accepted_id = match socket_alloc_connected_tcp_v6(tcp_id, remote_ip, remote_port) {
            Some(id) => id,
            None => {
                crate::net::tcp::close(tcp_id);
                return linux_err(ENOMEM);
            }
        };
        let accepted_fd = match crate::task::scheduler::current_fd_alloc(FdKind::LinuxSocket {
            socket_id: accepted_id,
        }) {
            Some(fd) => fd,
            None => {
                socket_decref(accepted_id);
                return linux_err(ENOMEM);
            }
        };
        let _ = write_sockaddr_in6(addr_ptr, addrlen_ptr, remote_ip, remote_port);
        return accepted_fd as u64;
    }

    let (tcp_id, remote_ip, remote_port) = crate::net::tcp::accept(listener_id, timeout_ticks);
    if tcp_id == u32::MAX {
        return linux_err(EAGAIN);
    }
    let accepted_id = match socket_alloc_connected_tcp(tcp_id, remote_ip, remote_port) {
        Some(id) => id,
        None => {
            crate::net::tcp::close(tcp_id);
            return linux_err(ENOMEM);
        }
    };
    let accepted_fd = match crate::task::scheduler::current_fd_alloc(FdKind::LinuxSocket {
        socket_id: accepted_id,
    }) {
        Some(fd) => fd,
        None => {
            socket_decref(accepted_id);
            return linux_err(ENOMEM);
        }
    };
    let _ = write_sockaddr_in(addr_ptr, addrlen_ptr, remote_ip, remote_port);
    accepted_fd as u64
}

pub(super) fn linux_sendto(
    fd: u32,
    buf_ptr: u64,
    len: u64,
    _flags: u64,
    addr_ptr: u64,
    addr_len: u64,
) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    let entry = match socket_entry(socket_id) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    match entry.state {
        LinuxSocketState::TcpConnected { .. } | LinuxSocketState::TcpConnectedV6 { .. } => {
            socket_write(fd, buf_ptr, len)
        }
        LinuxSocketState::Udp { .. } | LinuxSocketState::New => {
            let (remote_ip, remote_port) = if addr_ptr != 0 {
                match read_sockaddr_in(addr_ptr, addr_len) {
                    Ok(addr) => addr,
                    Err(errno) => return linux_err(errno),
                }
            } else {
                match entry.state {
                    LinuxSocketState::Udp {
                        remote_ip,
                        remote_port,
                        connected: true,
                        ..
                    } => (Ipv4Addr(remote_ip), remote_port),
                    _ => return linux_err(ENOTCONN),
                }
            };
            let local_port = match ensure_udp_bound(socket_id) {
                Ok(port) => port,
                Err(errno) => return linux_err(errno),
            };
            let copy_len = (len as usize).min(1472);
            let data = match handlers::helpers::copy_user_bytes(buf_ptr, copy_len, 1472) {
                Some(data) => data,
                None => return linux_err(EFAULT),
            };
            if crate::net::udp::send(remote_ip, local_port, remote_port, &data) {
                crate::task::scheduler::record_net_tx(data.len() as u64);
                data.len() as u64
            } else {
                linux_err(EAGAIN)
            }
        }
        LinuxSocketState::UdpV6 { .. } => {
            let (remote_ip, remote_port) = if addr_ptr != 0 {
                match read_sockaddr_in6(addr_ptr, addr_len) {
                    Ok(addr) => addr,
                    Err(errno) => return linux_err(errno),
                }
            } else {
                match entry.state {
                    LinuxSocketState::UdpV6 {
                        remote_ip,
                        remote_port,
                        connected: true,
                        ..
                    } => (Ipv6Addr(remote_ip), remote_port),
                    _ => return linux_err(ENOTCONN),
                }
            };
            let local_port = match ensure_udp_bound(socket_id) {
                Ok(port) => port,
                Err(errno) => return linux_err(errno),
            };
            let copy_len = (len as usize).min(1452);
            let data = match handlers::helpers::copy_user_bytes(buf_ptr, copy_len, 1452) {
                Some(data) => data,
                None => return linux_err(EFAULT),
            };
            if crate::net::udp::send_v6(remote_ip, local_port, remote_port, &data) {
                crate::task::scheduler::record_net_tx(data.len() as u64);
                data.len() as u64
            } else {
                linux_err(EAGAIN)
            }
        }
        _ => linux_err(ENOTCONN),
    }
}

pub(super) fn linux_recvfrom(
    fd: u32,
    buf_ptr: u64,
    len: u64,
    _flags: u64,
    addr_ptr: u64,
    addrlen_ptr: u64,
) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    match refresh_tcp_connect(socket_id) {
        Ok(true) => {}
        Ok(false) => return linux_err(EAGAIN),
        Err(errno) => return linux_err(errno),
    }
    let entry = match socket_entry(socket_id) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    match entry.state {
        LinuxSocketState::TcpConnected { .. } | LinuxSocketState::TcpConnectedV6 { .. } => {
            socket_read(fd, buf_ptr, len)
        }
        LinuxSocketState::Udp { local_port, .. } if local_port != 0 => {
            if buf_ptr == 0 || len > u32::MAX as u64 {
                return linux_err(EFAULT);
            }
            let dgram = if fd_nonblock(fd) {
                crate::net::poll();
                crate::net::udp::recv(local_port)
            } else {
                let timeout = 3 * crate::arch::hal::timer_frequency_hz() as u32;
                crate::net::udp::recv_timeout(local_port, timeout)
            };
            let Some(dgram) = dgram else {
                return linux_err(EAGAIN);
            };
            let copy_len = dgram.data.len().min(len as usize);
            if copy_len != 0
                && !handlers::helpers::copy_to_user_bytes(
                    buf_ptr,
                    &dgram.data[..copy_len],
                    copy_len,
                )
            {
                return linux_err(EFAULT);
            }
            let src_ip = match dgram.src_ip {
                IpAddr::V4(ip) => ip,
                IpAddr::V6(_) => Ipv4Addr::ZERO,
            };
            let _ = write_sockaddr_in(addr_ptr, addrlen_ptr, src_ip, dgram.src_port);
            crate::task::scheduler::record_net_rx(copy_len as u64);
            copy_len as u64
        }
        LinuxSocketState::UdpV6 { local_port, .. } if local_port != 0 => {
            if buf_ptr == 0 || len > u32::MAX as u64 {
                return linux_err(EFAULT);
            }
            let dgram = if fd_nonblock(fd) {
                crate::net::poll();
                crate::net::udp::recv(local_port)
            } else {
                let timeout = 3 * crate::arch::hal::timer_frequency_hz() as u32;
                crate::net::udp::recv_timeout(local_port, timeout)
            };
            let Some(dgram) = dgram else {
                return linux_err(EAGAIN);
            };
            let copy_len = dgram.data.len().min(len as usize);
            if copy_len != 0
                && !handlers::helpers::copy_to_user_bytes(
                    buf_ptr,
                    &dgram.data[..copy_len],
                    copy_len,
                )
            {
                return linux_err(EFAULT);
            }
            let src_ip = match dgram.src_ip {
                IpAddr::V6(ip) => ip,
                IpAddr::V4(ip) => ipv4_mapped_v6(ip),
            };
            let _ = write_sockaddr_in6(addr_ptr, addrlen_ptr, src_ip, dgram.src_port);
            crate::task::scheduler::record_net_rx(copy_len as u64);
            copy_len as u64
        }
        LinuxSocketState::Udp { .. } | LinuxSocketState::UdpV6 { .. } | LinuxSocketState::New => {
            linux_err(EAGAIN)
        }
        _ => linux_err(ENOTCONN),
    }
}

pub(super) fn linux_sendmsg(fd: u32, msg_ptr: u64, flags: u64) -> u64 {
    let (name, namelen, iov, iovlen) = match read_msghdr(msg_ptr) {
        Ok(v) => v,
        Err(errno) => return linux_err(errno),
    };
    socket_send_iov(fd, name, namelen, iov, iovlen, flags)
}

pub(super) fn linux_sendmmsg(fd: u32, msgvec_ptr: u64, vlen: u64, flags: u64) -> u64 {
    const MMSGHDR_SIZE: u64 = 64;
    const MMSGHDR_LEN_OFFSET: u64 = 56;

    if vlen == 0 {
        return 0;
    }
    if vlen > 1024 {
        return linux_err(EINVAL);
    }
    let Some(msgvec_bytes) = vlen.checked_mul(MMSGHDR_SIZE) else {
        return linux_err(EINVAL);
    };
    if msgvec_ptr == 0 || !handlers::helpers::is_user_range_accessible(msgvec_ptr, msgvec_bytes) {
        return linux_err(EFAULT);
    }

    let mut sent = 0u64;
    for idx in 0..vlen {
        let msg_ptr = msgvec_ptr + idx * MMSGHDR_SIZE;
        let ret = linux_sendmsg(fd, msg_ptr, flags);
        if (ret as i64) < 0 {
            return if sent == 0 { ret } else { sent };
        }
        unsafe {
            write_u32(msg_ptr, MMSGHDR_LEN_OFFSET, ret.min(u32::MAX as u64) as u32);
        }
        sent += 1;
    }
    sent
}

pub(super) fn linux_recvmsg(fd: u32, msg_ptr: u64, flags: u64) -> u64 {
    let (name, _namelen, iov, iovlen) = match read_msghdr(msg_ptr) {
        Ok(v) => v,
        Err(errno) => return linux_err(errno),
    };
    if iovlen == 0 {
        return 0;
    }
    if iov == 0 || !handlers::helpers::is_user_range_accessible(iov, 16) {
        return linux_err(EFAULT);
    }
    let base = unsafe { read_u64(iov, 0) };
    let len = unsafe { read_u64(iov, 8) };
    let ret = linux_recvfrom(fd, base, len, flags, name, msg_ptr + 8);
    if (ret as i64) >= 0 {
        unsafe {
            write_u32(msg_ptr, 48, 0);
        }
    }
    ret
}

pub(super) fn linux_shutdown(fd: u32, _how: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    match refresh_tcp_connect(socket_id) {
        Ok(_) => {}
        Err(errno) => return linux_err(errno),
    }
    match socket_entry(socket_id).map(|entry| entry.state) {
        Some(LinuxSocketState::TcpConnected { tcp_id, .. })
        | Some(LinuxSocketState::TcpConnectedV6 { tcp_id, .. }) => {
            let _ = crate::net::tcp::shutdown_write(tcp_id);
            0
        }
        Some(_) => 0,
        None => linux_err(EBADF),
    }
}

pub(super) fn linux_getsockname(fd: u32, addr_ptr: u64, addrlen_ptr: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    let entry = match socket_entry(socket_id) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    let port = match entry.state {
        LinuxSocketState::TcpBound { local_port }
        | LinuxSocketState::TcpListener { local_port, .. }
        | LinuxSocketState::Udp { local_port, .. }
        | LinuxSocketState::UdpV6 { local_port, .. } => local_port,
        _ => 0,
    };
    if entry.kind == LinuxSocketKind::Inet6Stream || entry.kind == LinuxSocketKind::Inet6Datagram {
        let cfg = crate::net::config();
        let local_ip = if !cfg.ipv6_addr.is_unspecified() {
            cfg.ipv6_addr
        } else {
            cfg.ipv6_link_local
        };
        write_sockaddr_in6(addr_ptr, addrlen_ptr, local_ip, port)
    } else {
        write_sockaddr_in(addr_ptr, addrlen_ptr, crate::net::config().ip, port)
    }
}

pub(super) fn linux_getpeername(fd: u32, addr_ptr: u64, addrlen_ptr: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    match refresh_tcp_connect(socket_id) {
        Ok(true) => {}
        Ok(false) => return linux_err(ENOTCONN),
        Err(errno) => return linux_err(errno),
    }
    match socket_entry(socket_id).map(|entry| entry.state) {
        Some(LinuxSocketState::TcpConnected {
            remote_ip,
            remote_port,
            ..
        })
        | Some(LinuxSocketState::Udp {
            remote_ip,
            remote_port,
            connected: true,
            ..
        }) => write_sockaddr_in(addr_ptr, addrlen_ptr, Ipv4Addr(remote_ip), remote_port),
        Some(LinuxSocketState::TcpConnectedV6 {
            remote_ip,
            remote_port,
            ..
        })
        | Some(LinuxSocketState::UdpV6 {
            remote_ip,
            remote_port,
            connected: true,
            ..
        }) => write_sockaddr_in6(addr_ptr, addrlen_ptr, Ipv6Addr(remote_ip), remote_port),
        Some(_) => linux_err(ENOTCONN),
        None => linux_err(EBADF),
    }
}

pub(super) fn linux_socketpair(_domain: u64, _type_: u64, _protocol: u64, _sv: u64) -> u64 {
    linux_err(EAFNOSUPPORT)
}

pub(super) fn linux_setsockopt(
    fd: u32,
    _level: u64,
    optname: u64,
    optval: u64,
    _optlen: u64,
) -> u64 {
    if fd_socket_id(fd).is_err() {
        return linux_err(EBADF);
    }
    if optval == 0 {
        return 0;
    }
    if optname == SO_RCVTIMEO || optname == SO_SNDTIMEO {
        return 0;
    }
    0
}

pub(super) fn linux_getsockopt(fd: u32, level: u64, optname: u64, optval: u64, optlen: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    if optval == 0 || optlen == 0 || !handlers::helpers::is_user_range_accessible(optlen, 4) {
        return linux_err(EFAULT);
    }
    if level == SOL_SOCKET && optname == SO_ERROR {
        let mut so_error = 0i32;
        match refresh_tcp_connect(socket_id) {
            Ok(_) => {}
            Err(errno) => {
                so_error = errno;
                let _ = socket_set_state(socket_id, LinuxSocketState::New);
            }
        }
        if let Some(LinuxSocketEntry {
            state: LinuxSocketState::TcpConnectFailed { errno },
            ..
        }) = socket_entry(socket_id)
        {
            so_error = errno;
            let _ = socket_set_state(socket_id, LinuxSocketState::New);
        }
        unsafe {
            write_u32(optval, 0, so_error as u32);
            write_u32(optlen, 0, 4);
        }
        return 0;
    }
    unsafe {
        write_u32(optval, 0, 0);
        write_u32(optlen, 0, 4);
    }
    0
}

pub(super) fn socket_read(fd: u32, buf_ptr: u64, len: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    match refresh_tcp_connect(socket_id) {
        Ok(true) => {}
        Ok(false) => return linux_err(EAGAIN),
        Err(errno) => return linux_err(errno),
    }
    let entry = match socket_entry(socket_id) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    match entry.state {
        LinuxSocketState::TcpConnected { tcp_id, .. }
        | LinuxSocketState::TcpConnectedV6 { tcp_id, .. } => {
            if buf_ptr == 0 || len > u32::MAX as u64 {
                return linux_err(EFAULT);
            }
            let nonblock = fd_nonblock(fd);
            if nonblock {
                match crate::net::tcp::recv_available(tcp_id) {
                    u32::MAX => return linux_err(EAGAIN),
                    n if n == u32::MAX - 1 => return 0,
                    0 => return linux_err(EAGAIN),
                    _ => {}
                }
            }
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
            let timeout = if nonblock {
                0
            } else {
                3 * crate::arch::hal::timer_frequency_hz() as u32
            };
            match crate::net::tcp::recv(tcp_id, buf, timeout) {
                u32::MAX => linux_err(EAGAIN),
                n => {
                    crate::task::scheduler::record_net_rx(n as u64);
                    n as u64
                }
            }
        }
        LinuxSocketState::Udp { .. } | LinuxSocketState::UdpV6 { .. } => {
            linux_recvfrom(fd, buf_ptr, len, 0, 0, 0)
        }
        _ => linux_err(ENOTCONN),
    }
}

pub(super) fn socket_write(fd: u32, buf_ptr: u64, len: u64) -> u64 {
    let socket_id = match fd_socket_id(fd) {
        Ok(id) => id,
        Err(errno) => return linux_err(errno),
    };
    match refresh_tcp_connect(socket_id) {
        Ok(true) => {}
        Ok(false) => return linux_err(EAGAIN),
        Err(errno) => return linux_err(errno),
    }
    let entry = match socket_entry(socket_id) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    match entry.state {
        LinuxSocketState::TcpConnected { tcp_id, .. }
        | LinuxSocketState::TcpConnectedV6 { tcp_id, .. } => {
            let mut total = 0usize;
            while total < len as usize {
                let chunk_len = ((len as usize) - total).min(64 * 1024);
                let data = match handlers::helpers::copy_user_bytes(
                    buf_ptr.wrapping_add(total as u64),
                    chunk_len,
                    64 * 1024,
                ) {
                    Some(data) => data,
                    None => {
                        return if total > 0 {
                            total as u64
                        } else {
                            linux_err(EFAULT)
                        }
                    }
                };
                let sent = crate::net::tcp::send(tcp_id, &data, 10_000);
                if sent == u32::MAX {
                    return if total > 0 {
                        total as u64
                    } else {
                        linux_err(EAGAIN)
                    };
                }
                crate::task::scheduler::record_net_tx(sent as u64);
                total += sent as usize;
                if sent as usize != chunk_len {
                    break;
                }
            }
            total as u64
        }
        LinuxSocketState::Udp {
            connected: true, ..
        }
        | LinuxSocketState::UdpV6 {
            connected: true, ..
        } => linux_sendto(fd, buf_ptr, len, 0, 0, 0),
        _ => linux_err(ENOTCONN),
    }
}

pub(crate) fn socket_incref(socket_id: u32) {
    let Some(idx) = socket_index(socket_id) else {
        return;
    };
    let mut table = LINUX_SOCKETS.lock();
    if table[idx].in_use {
        table[idx].refs = table[idx].refs.saturating_add(1);
    }
}

pub(crate) fn socket_decref(socket_id: u32) {
    let Some(idx) = socket_index(socket_id) else {
        return;
    };
    let close_state = {
        let mut table = LINUX_SOCKETS.lock();
        if !table[idx].in_use {
            return;
        }
        if table[idx].refs > 1 {
            table[idx].refs -= 1;
            return;
        }
        let close_state = table[idx].state;
        table[idx] = LinuxSocketEntry::EMPTY;
        close_state
    };
    close_socket_state(close_state);
}

pub(super) fn linux_socket_poll_revents(socket_id: u32, events: i16) -> i16 {
    const POLLIN: i16 = 0x0001;
    const POLLOUT: i16 = 0x0004;
    const POLLERR: i16 = 0x0008;
    const POLLHUP: i16 = 0x0010;

    let state = match socket_entry(socket_id) {
        Some(entry) => entry.state,
        None => return POLLERR,
    };

    match state {
        LinuxSocketState::TcpConnecting { .. } | LinuxSocketState::TcpConnectingV6 { .. } => {
            match refresh_tcp_connect(socket_id) {
                Ok(true) => events & POLLOUT,
                Ok(false) => 0,
                Err(_) => (events & POLLOUT) | POLLERR,
            }
        }
        LinuxSocketState::TcpConnectFailed { .. } => (events & POLLOUT) | POLLERR,
        LinuxSocketState::TcpConnected { tcp_id, .. }
        | LinuxSocketState::TcpConnectedV6 { tcp_id, .. } => {
            let mut revents = 0;
            if events & POLLOUT != 0 {
                revents |= POLLOUT;
            }
            if events & POLLIN != 0 {
                match crate::net::tcp::recv_available(tcp_id) {
                    n if n == u32::MAX - 1 => revents |= POLLIN | POLLHUP,
                    n if n > 0 && n != u32::MAX => revents |= POLLIN,
                    _ => {}
                }
            }
            revents
        }
        LinuxSocketState::Udp { .. } | LinuxSocketState::UdpV6 { .. } => {
            events & (POLLIN | POLLOUT)
        }
        LinuxSocketState::TcpListener { .. } => events & POLLIN,
        _ => 0,
    }
}

pub(super) fn linux_socket_fionread(socket_id: u32, arg: u64) -> u64 {
    if arg == 0 || !handlers::helpers::is_user_range_accessible(arg, 4) {
        return linux_err(EFAULT);
    }
    let mut available = 0u32;
    match refresh_tcp_connect(socket_id) {
        Ok(_) => {}
        Err(errno) => return linux_err(errno),
    }
    if let Some(LinuxSocketEntry {
        state:
            LinuxSocketState::TcpConnected { tcp_id, .. }
            | LinuxSocketState::TcpConnectedV6 { tcp_id, .. },
        ..
    }) = socket_entry(socket_id)
    {
        available = match crate::net::tcp::recv_available(tcp_id) {
            n if n == u32::MAX || n == u32::MAX - 1 => 0,
            n => n,
        };
    }
    unsafe {
        write_u32(arg, 0, available);
    }
    0
}

fn socket_alloc(kind: LinuxSocketKind, protocol: u16) -> Option<u32> {
    let mut table = LINUX_SOCKETS.lock();
    for (idx, entry) in table.iter_mut().enumerate() {
        if !entry.in_use {
            *entry = LinuxSocketEntry {
                in_use: true,
                refs: 1,
                kind,
                protocol,
                state: match kind {
                    LinuxSocketKind::InetDatagram => LinuxSocketState::Udp {
                        local_port: 0,
                        remote_ip: [0; 4],
                        remote_port: 0,
                        connected: false,
                    },
                    LinuxSocketKind::Inet6Datagram => LinuxSocketState::UdpV6 {
                        local_port: 0,
                        remote_ip: [0; 16],
                        remote_port: 0,
                        connected: false,
                    },
                    LinuxSocketKind::InetStream | LinuxSocketKind::Inet6Stream => {
                        LinuxSocketState::New
                    }
                    LinuxSocketKind::Empty => LinuxSocketState::Empty,
                },
            };
            return Some((idx + 1) as u32);
        }
    }
    None
}

fn socket_alloc_connected_tcp(tcp_id: u32, remote_ip: Ipv4Addr, remote_port: u16) -> Option<u32> {
    let socket_id = socket_alloc(LinuxSocketKind::InetStream, IPPROTO_TCP as u16)?;
    if socket_set_state(
        socket_id,
        LinuxSocketState::TcpConnected {
            tcp_id,
            remote_ip: remote_ip.0,
            remote_port,
        },
    ) {
        Some(socket_id)
    } else {
        socket_decref(socket_id);
        None
    }
}

fn socket_alloc_connected_tcp_v6(
    tcp_id: u32,
    remote_ip: Ipv6Addr,
    remote_port: u16,
) -> Option<u32> {
    let socket_id = socket_alloc(LinuxSocketKind::Inet6Stream, IPPROTO_TCP as u16)?;
    if socket_set_state(
        socket_id,
        LinuxSocketState::TcpConnectedV6 {
            tcp_id,
            remote_ip: remote_ip.0,
            remote_port,
        },
    ) {
        Some(socket_id)
    } else {
        socket_decref(socket_id);
        None
    }
}

fn socket_index(socket_id: u32) -> Option<usize> {
    let idx = socket_id.checked_sub(1)? as usize;
    if idx < MAX_LINUX_SOCKETS {
        Some(idx)
    } else {
        None
    }
}

fn socket_entry(socket_id: u32) -> Option<LinuxSocketEntry> {
    let idx = socket_index(socket_id)?;
    let table = LINUX_SOCKETS.lock();
    if table[idx].in_use {
        Some(table[idx])
    } else {
        None
    }
}

fn socket_set_state(socket_id: u32, state: LinuxSocketState) -> bool {
    let Some(idx) = socket_index(socket_id) else {
        return false;
    };
    let mut table = LINUX_SOCKETS.lock();
    if !table[idx].in_use {
        return false;
    }
    table[idx].state = state;
    true
}

fn refresh_tcp_connect(socket_id: u32) -> Result<bool, i32> {
    let state = match socket_entry(socket_id) {
        Some(entry) => entry.state,
        None => return Err(EBADF),
    };
    let LinuxSocketState::TcpConnecting {
        tcp_id,
        remote_ip,
        remote_port,
    } = state
    else {
        if let LinuxSocketState::TcpConnectingV6 {
            tcp_id,
            remote_ip,
            remote_port,
        } = state
        {
            crate::net::poll();
            return match crate::net::tcp::connect_status(tcp_id) {
                crate::net::tcp::ConnectStatus::Established => {
                    if socket_set_state(
                        socket_id,
                        LinuxSocketState::TcpConnectedV6 {
                            tcp_id,
                            remote_ip,
                            remote_port,
                        },
                    ) {
                        Ok(true)
                    } else {
                        crate::net::tcp::close(tcp_id);
                        Err(EBADF)
                    }
                }
                crate::net::tcp::ConnectStatus::InProgress => Ok(false),
                crate::net::tcp::ConnectStatus::Failed => {
                    let _ = socket_set_state(
                        socket_id,
                        LinuxSocketState::TcpConnectFailed {
                            errno: ECONNREFUSED,
                        },
                    );
                    Err(ECONNREFUSED)
                }
            };
        }
        return Ok(!matches!(state, LinuxSocketState::TcpConnectFailed { .. }));
    };

    crate::net::poll();
    match crate::net::tcp::connect_status(tcp_id) {
        crate::net::tcp::ConnectStatus::Established => {
            if socket_set_state(
                socket_id,
                LinuxSocketState::TcpConnected {
                    tcp_id,
                    remote_ip,
                    remote_port,
                },
            ) {
                Ok(true)
            } else {
                crate::net::tcp::close(tcp_id);
                Err(EBADF)
            }
        }
        crate::net::tcp::ConnectStatus::InProgress => Ok(false),
        crate::net::tcp::ConnectStatus::Failed => {
            let _ = socket_set_state(
                socket_id,
                LinuxSocketState::TcpConnectFailed {
                    errno: ECONNREFUSED,
                },
            );
            Err(ECONNREFUSED)
        }
    }
}

fn socket_update_udp_remote(socket_id: u32, remote_ip: Ipv4Addr, remote_port: u16) -> bool {
    let Some(idx) = socket_index(socket_id) else {
        return false;
    };
    let mut table = LINUX_SOCKETS.lock();
    if !table[idx].in_use {
        return false;
    }
    let local_port = match table[idx].state {
        LinuxSocketState::Udp { local_port, .. } => local_port,
        _ => 0,
    };
    table[idx].state = LinuxSocketState::Udp {
        local_port,
        remote_ip: remote_ip.0,
        remote_port,
        connected: true,
    };
    true
}

fn socket_update_udp_remote_v6(socket_id: u32, remote_ip: Ipv6Addr, remote_port: u16) -> bool {
    let Some(idx) = socket_index(socket_id) else {
        return false;
    };
    let mut table = LINUX_SOCKETS.lock();
    if !table[idx].in_use {
        return false;
    }
    let local_port = match table[idx].state {
        LinuxSocketState::UdpV6 { local_port, .. } => local_port,
        _ => 0,
    };
    table[idx].state = LinuxSocketState::UdpV6 {
        local_port,
        remote_ip: remote_ip.0,
        remote_port,
        connected: true,
    };
    true
}

fn close_socket_state(state: LinuxSocketState) {
    match state {
        LinuxSocketState::TcpConnecting { tcp_id, .. } => {
            let _ = crate::net::tcp::close(tcp_id);
        }
        LinuxSocketState::TcpConnectingV6 { tcp_id, .. } => {
            let _ = crate::net::tcp::close(tcp_id);
        }
        LinuxSocketState::TcpConnected { tcp_id, .. } => {
            let _ = crate::net::tcp::close(tcp_id);
        }
        LinuxSocketState::TcpConnectedV6 { tcp_id, .. } => {
            let _ = crate::net::tcp::close(tcp_id);
        }
        LinuxSocketState::TcpListener { listener_id, .. } => {
            let _ = crate::net::tcp::close_listener(listener_id);
        }
        LinuxSocketState::Udp { local_port, .. } if local_port != 0 => {
            crate::net::udp::unbind(local_port);
        }
        LinuxSocketState::UdpV6 { local_port, .. } if local_port != 0 => {
            crate::net::udp::unbind(local_port);
        }
        _ => {}
    }
}

fn fd_socket_id(fd: u32) -> Result<u32, i32> {
    match crate::task::scheduler::current_fd_get(fd).map(|entry| entry.kind) {
        Some(FdKind::LinuxSocket { socket_id }) => Ok(socket_id),
        Some(_) => Err(ENOTCONN),
        None => Err(EBADF),
    }
}

fn fd_nonblock(fd: u32) -> bool {
    crate::task::scheduler::current_fd_get(fd)
        .map(|entry| entry.flags.nonblock)
        .unwrap_or(false)
}

fn ensure_udp_bound(socket_id: u32) -> Result<u16, i32> {
    if let Some(LinuxSocketEntry {
        state: LinuxSocketState::Udp { local_port, .. },
        ..
    }) = socket_entry(socket_id)
    {
        if local_port != 0 {
            return Ok(local_port);
        }
    }
    if let Some(LinuxSocketEntry {
        state: LinuxSocketState::UdpV6 { local_port, .. },
        ..
    }) = socket_entry(socket_id)
    {
        if local_port != 0 {
            return Ok(local_port);
        }
    }
    let port = bind_ephemeral_udp().ok_or(ENOMEM)?;
    let Some(idx) = socket_index(socket_id) else {
        crate::net::udp::unbind(port);
        return Err(EBADF);
    };
    let mut table = LINUX_SOCKETS.lock();
    if !table[idx].in_use {
        crate::net::udp::unbind(port);
        return Err(EBADF);
    }
    match table[idx].state {
        LinuxSocketState::Udp {
            remote_ip,
            remote_port,
            connected,
            ..
        } => {
            table[idx].state = LinuxSocketState::Udp {
                local_port: port,
                remote_ip,
                remote_port,
                connected,
            };
            Ok(port)
        }
        LinuxSocketState::UdpV6 {
            remote_ip,
            remote_port,
            connected,
            ..
        } => {
            table[idx].state = LinuxSocketState::UdpV6 {
                local_port: port,
                remote_ip,
                remote_port,
                connected,
            };
            Ok(port)
        }
        _ => {
            crate::net::udp::unbind(port);
            Err(EINVAL)
        }
    }
}

fn bind_ephemeral_udp() -> Option<u16> {
    let span = (EPHEMERAL_LAST - EPHEMERAL_FIRST + 1) as u32;
    for _ in 0..span {
        let raw = NEXT_EPHEMERAL.fetch_add(1, Ordering::Relaxed);
        let port = EPHEMERAL_FIRST + (raw % span) as u16;
        if crate::net::udp::bind(port) {
            return Some(port);
        }
    }
    None
}

fn read_sockaddr_in(addr_ptr: u64, addr_len: u64) -> Result<(Ipv4Addr, u16), i32> {
    if addr_ptr == 0 || addr_len < 16 || !handlers::helpers::is_user_range_accessible(addr_ptr, 16)
    {
        return Err(EFAULT);
    }
    let family =
        unsafe { u16::from_le_bytes([*(addr_ptr as *const u8), *((addr_ptr + 1) as *const u8)]) };
    if family as u64 != AF_INET {
        return Err(EAFNOSUPPORT);
    }
    let port = unsafe {
        u16::from_be_bytes([
            *((addr_ptr + 2) as *const u8),
            *((addr_ptr + 3) as *const u8),
        ])
    };
    let ip = unsafe {
        Ipv4Addr([
            *((addr_ptr + 4) as *const u8),
            *((addr_ptr + 5) as *const u8),
            *((addr_ptr + 6) as *const u8),
            *((addr_ptr + 7) as *const u8),
        ])
    };
    Ok((ip, port))
}

fn write_sockaddr_in(addr_ptr: u64, addrlen_ptr: u64, ip: Ipv4Addr, port: u16) -> u64 {
    if addr_ptr == 0 || addrlen_ptr == 0 {
        return 0;
    }
    if !handlers::helpers::is_user_range_accessible(addrlen_ptr, 4) {
        return linux_err(EFAULT);
    }
    let len = unsafe { *((addrlen_ptr) as *const u32) };
    if len < 16 {
        unsafe {
            write_u32(addrlen_ptr, 0, 16);
        }
        return linux_err(EINVAL);
    }
    if !handlers::helpers::is_user_range_accessible(addr_ptr, 16) {
        return linux_err(EFAULT);
    }
    unsafe {
        write_u16(addr_ptr, 0, AF_INET as u16);
        let port_be = port.to_be_bytes();
        *((addr_ptr + 2) as *mut u8) = port_be[0];
        *((addr_ptr + 3) as *mut u8) = port_be[1];
        core::ptr::copy_nonoverlapping(ip.0.as_ptr(), (addr_ptr + 4) as *mut u8, 4);
        core::ptr::write_bytes((addr_ptr + 8) as *mut u8, 0, 8);
        write_u32(addrlen_ptr, 0, 16);
    }
    0
}

fn read_sockaddr_in6(addr_ptr: u64, addr_len: u64) -> Result<(Ipv6Addr, u16), i32> {
    if addr_ptr == 0 || addr_len < 28 || !handlers::helpers::is_user_range_accessible(addr_ptr, 28)
    {
        return Err(EFAULT);
    }
    let family =
        unsafe { u16::from_le_bytes([*(addr_ptr as *const u8), *((addr_ptr + 1) as *const u8)]) };
    if family as u64 != AF_INET6 {
        return Err(EAFNOSUPPORT);
    }
    let port = unsafe {
        u16::from_be_bytes([
            *((addr_ptr + 2) as *const u8),
            *((addr_ptr + 3) as *const u8),
        ])
    };
    let mut ip = [0u8; 16];
    unsafe {
        core::ptr::copy_nonoverlapping((addr_ptr + 8) as *const u8, ip.as_mut_ptr(), 16);
    }
    Ok((Ipv6Addr(ip), port))
}

fn write_sockaddr_in6(addr_ptr: u64, addrlen_ptr: u64, ip: Ipv6Addr, port: u16) -> u64 {
    if addr_ptr == 0 || addrlen_ptr == 0 {
        return 0;
    }
    if !handlers::helpers::is_user_range_accessible(addrlen_ptr, 4) {
        return linux_err(EFAULT);
    }
    let len = unsafe { *((addrlen_ptr) as *const u32) };
    if len < 28 {
        unsafe {
            write_u32(addrlen_ptr, 0, 28);
        }
        return linux_err(EINVAL);
    }
    if !handlers::helpers::is_user_range_accessible(addr_ptr, 28) {
        return linux_err(EFAULT);
    }
    unsafe {
        write_u16(addr_ptr, 0, AF_INET6 as u16);
        let port_be = port.to_be_bytes();
        *((addr_ptr + 2) as *mut u8) = port_be[0];
        *((addr_ptr + 3) as *mut u8) = port_be[1];
        write_u32(addr_ptr, 4, 0);
        core::ptr::copy_nonoverlapping(ip.0.as_ptr(), (addr_ptr + 8) as *mut u8, 16);
        write_u32(addr_ptr, 24, 0);
        write_u32(addrlen_ptr, 0, 28);
    }
    0
}

fn ipv4_mapped_v6(ip: Ipv4Addr) -> Ipv6Addr {
    let mut out = [0u8; 16];
    out[10] = 0xff;
    out[11] = 0xff;
    out[12..16].copy_from_slice(&ip.0);
    Ipv6Addr(out)
}

fn read_msghdr(msg_ptr: u64) -> Result<(u64, u64, u64, u64), i32> {
    if msg_ptr == 0 || !handlers::helpers::is_user_range_accessible(msg_ptr, 56) {
        return Err(EFAULT);
    }
    let name = unsafe { read_u64(msg_ptr, 0) };
    let namelen = unsafe { *((msg_ptr + 8) as *const u32) } as u64;
    let iov = unsafe { read_u64(msg_ptr, 16) };
    let iovlen = unsafe { read_u64(msg_ptr, 24) };
    if iovlen > 1024 {
        return Err(EINVAL);
    }
    Ok((name, namelen, iov, iovlen))
}

fn socket_send_iov(fd: u32, name: u64, namelen: u64, iov: u64, iovlen: u64, flags: u64) -> u64 {
    if iovlen == 0 {
        return 0;
    }
    let Some(iov_bytes) = iovlen.checked_mul(16) else {
        return linux_err(EINVAL);
    };
    if iov == 0 || !handlers::helpers::is_user_range_accessible(iov, iov_bytes) {
        return linux_err(EFAULT);
    }
    let mut total = 0u64;
    let mut datagram = Vec::new();
    let socket_kind = fd_socket_id(fd)
        .ok()
        .and_then(socket_entry)
        .map(|entry| entry.kind)
        .unwrap_or(LinuxSocketKind::Empty);
    let is_udp = socket_kind == LinuxSocketKind::InetDatagram
        || socket_kind == LinuxSocketKind::Inet6Datagram;
    for idx in 0..iovlen {
        let base = unsafe { read_u64(iov, idx * 16) };
        let len = unsafe { read_u64(iov, idx * 16 + 8) };
        if len == 0 {
            continue;
        }
        if is_udp {
            if datagram.len().saturating_add(len as usize) > 1472 {
                return linux_err(EINVAL);
            }
            let chunk = match handlers::helpers::copy_user_bytes(base, len as usize, 1472) {
                Some(chunk) => chunk,
                None => return linux_err(EFAULT),
            };
            datagram.extend_from_slice(&chunk);
            total += len;
        } else {
            let ret = linux_sendto(fd, base, len, flags, 0, 0);
            if (ret as i64) < 0 {
                return if total > 0 { total } else { ret };
            }
            total += ret;
            if ret != len {
                break;
            }
        }
    }
    if is_udp {
        let socket_id = match fd_socket_id(fd) {
            Ok(id) => id,
            Err(errno) => return linux_err(errno),
        };
        let entry = match socket_entry(socket_id) {
            Some(entry) => entry,
            None => return linux_err(EBADF),
        };
        let local_port = match ensure_udp_bound(socket_id) {
            Ok(port) => port,
            Err(errno) => return linux_err(errno),
        };
        let sent = if socket_kind == LinuxSocketKind::Inet6Datagram {
            let (remote_ip, remote_port) = if name != 0 {
                match read_sockaddr_in6(name, namelen) {
                    Ok(addr) => addr,
                    Err(errno) => return linux_err(errno),
                }
            } else {
                match entry.state {
                    LinuxSocketState::UdpV6 {
                        remote_ip,
                        remote_port,
                        connected: true,
                        ..
                    } => (Ipv6Addr(remote_ip), remote_port),
                    _ => return linux_err(ENOTCONN),
                }
            };
            crate::net::udp::send_v6(remote_ip, local_port, remote_port, &datagram)
        } else {
            let (remote_ip, remote_port) = if name != 0 {
                match read_sockaddr_in(name, namelen) {
                    Ok(addr) => addr,
                    Err(errno) => return linux_err(errno),
                }
            } else {
                match entry.state {
                    LinuxSocketState::Udp {
                        remote_ip,
                        remote_port,
                        connected: true,
                        ..
                    } => (Ipv4Addr(remote_ip), remote_port),
                    _ => return linux_err(ENOTCONN),
                }
            };
            crate::net::udp::send(remote_ip, local_port, remote_port, &datagram)
        };
        if sent {
            crate::task::scheduler::record_net_tx(datagram.len() as u64);
            datagram.len() as u64
        } else {
            linux_err(EAGAIN)
        }
    } else {
        total
    }
}
