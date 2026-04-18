//! std::net compatible networking types.
//!
//! Wraps anyos_std::net TCP/UDP with std-style types.

use crate::io::{self, Read, Write};
use core::fmt;

// ── IP addresses ────────────────────────────────────────────────────────────

/// IPv4 address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv4Addr {
    octets: [u8; 4],
}

impl Ipv4Addr {
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Ipv4Addr { octets: [a, b, c, d] }
    }

    pub const LOCALHOST: Self = Self::new(127, 0, 0, 1);
    pub const UNSPECIFIED: Self = Self::new(0, 0, 0, 0);
    pub const BROADCAST: Self = Self::new(255, 255, 255, 255);

    pub const fn octets(&self) -> [u8; 4] {
        self.octets
    }

    pub fn is_loopback(&self) -> bool {
        self.octets[0] == 127
    }

    pub fn is_unspecified(&self) -> bool {
        self.octets == [0, 0, 0, 0]
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}.{}", self.octets[0], self.octets[1], self.octets[2], self.octets[3])
    }
}

/// IPv6 address (stub — anyOS is IPv4 only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv6Addr {
    segments: [u16; 8],
}

impl Ipv6Addr {
    pub const fn new(a: u16, b: u16, c: u16, d: u16, e: u16, f: u16, g: u16, h: u16) -> Self {
        Ipv6Addr { segments: [a, b, c, d, e, f, g, h] }
    }

    pub const LOCALHOST: Self = Self::new(0, 0, 0, 0, 0, 0, 0, 1);
    pub const UNSPECIFIED: Self = Self::new(0, 0, 0, 0, 0, 0, 0, 0);

    pub const fn segments(&self) -> [u16; 8] {
        self.segments
    }
}

impl fmt::Display for Ipv6Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "::1") // Simplified
    }
}

/// IP address enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IpAddr {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

impl fmt::Display for IpAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpAddr::V4(a) => a.fmt(f),
            IpAddr::V6(a) => a.fmt(f),
        }
    }
}

// ── Socket addresses ────────────────────────────────────────────────────────

/// Socket address (IPv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketAddrV4 {
    ip: Ipv4Addr,
    port: u16,
}

impl SocketAddrV4 {
    pub fn new(ip: Ipv4Addr, port: u16) -> Self {
        SocketAddrV4 { ip, port }
    }
    pub fn ip(&self) -> &Ipv4Addr { &self.ip }
    pub fn port(&self) -> u16 { self.port }
}

impl fmt::Display for SocketAddrV4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ip, self.port)
    }
}

/// Socket address (IPv6, stub).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketAddrV6 {
    ip: Ipv6Addr,
    port: u16,
}

impl SocketAddrV6 {
    pub fn new(ip: Ipv6Addr, port: u16, _flowinfo: u32, _scope_id: u32) -> Self {
        SocketAddrV6 { ip, port }
    }
    pub fn ip(&self) -> &Ipv6Addr { &self.ip }
    pub fn port(&self) -> u16 { self.port }
}

/// Socket address enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SocketAddr {
    V4(SocketAddrV4),
    V6(SocketAddrV6),
}

impl SocketAddr {
    pub fn new(ip: IpAddr, port: u16) -> Self {
        match ip {
            IpAddr::V4(v4) => SocketAddr::V4(SocketAddrV4::new(v4, port)),
            IpAddr::V6(v6) => SocketAddr::V6(SocketAddrV6::new(v6, port, 0, 0)),
        }
    }

    pub fn ip(&self) -> IpAddr {
        match self {
            SocketAddr::V4(a) => IpAddr::V4(*a.ip()),
            SocketAddr::V6(a) => IpAddr::V6(*a.ip()),
        }
    }

    pub fn port(&self) -> u16 {
        match self {
            SocketAddr::V4(a) => a.port(),
            SocketAddr::V6(a) => a.port(),
        }
    }
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SocketAddr::V4(a) => a.fmt(f),
            SocketAddr::V6(a) => write!(f, "[{}]:{}", a.ip, a.port),
        }
    }
}

/// Trait for converting to socket addresses.
pub trait ToSocketAddrs {
    type Iter: Iterator<Item = SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter>;
}

impl ToSocketAddrs for SocketAddr {
    type Iter = core::iter::Once<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<core::iter::Once<SocketAddr>> {
        Ok(core::iter::once(*self))
    }
}

impl ToSocketAddrs for SocketAddrV4 {
    type Iter = core::iter::Once<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<core::iter::Once<SocketAddr>> {
        Ok(core::iter::once(SocketAddr::V4(*self)))
    }
}

impl ToSocketAddrs for (&str, u16) {
    type Iter = alloc::vec::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<alloc::vec::IntoIter<SocketAddr>> {
        let (host, port) = *self;
        // Try parsing as IP first
        if let Some(ip) = parse_ipv4(host) {
            return Ok(alloc::vec![SocketAddr::V4(SocketAddrV4::new(ip, port))].into_iter());
        }
        // DNS resolve
        let mut result = [0u8; 4];
        let ret = anyos_std::net::dns(host, &mut result);
        if ret != 0 {
            return Err(io::Error::new(io::ErrorKind::Other, "DNS resolution failed"));
        }
        let ip = Ipv4Addr::new(result[0], result[1], result[2], result[3]);
        Ok(alloc::vec![SocketAddr::V4(SocketAddrV4::new(ip, port))].into_iter())
    }
}

fn parse_ipv4(s: &str) -> Option<Ipv4Addr> {
    let mut octets = [0u8; 4];
    let mut parts = s.split('.');
    for octet in &mut octets {
        let part = parts.next()?;
        *octet = part.parse::<u8>().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]))
}

// ── TcpStream ───────────────────────────────────────────────────────────────

/// A TCP stream, mirroring std::net::TcpStream.
pub struct TcpStream {
    socket_id: u32,
    peer_addr: SocketAddr,
}

impl TcpStream {
    /// Connect to a remote address.
    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<TcpStream> {
        Self::connect_timeout_inner(addr, 10_000)
    }

    /// Connect with a timeout.
    pub fn connect_timeout(addr: &SocketAddr, timeout: core::time::Duration) -> io::Result<TcpStream> {
        let ms = timeout.as_millis() as u32;
        let addrs: alloc::vec::Vec<_> = core::iter::once(*addr).collect();
        Self::connect_timeout_inner_vec(addrs, ms)
    }

    fn connect_timeout_inner<A: ToSocketAddrs>(addr: A, timeout_ms: u32) -> io::Result<TcpStream> {
        let addrs: alloc::vec::Vec<_> = addr.to_socket_addrs()?.collect();
        Self::connect_timeout_inner_vec(addrs, timeout_ms)
    }

    fn connect_timeout_inner_vec(addrs: alloc::vec::Vec<SocketAddr>, timeout_ms: u32) -> io::Result<TcpStream> {
        for addr in &addrs {
            match addr {
                SocketAddr::V4(v4) => {
                    let ip = v4.ip().octets();
                    let sock = anyos_std::net::tcp_connect(&ip, v4.port(), timeout_ms);
                    if sock != u32::MAX {
                        return Ok(TcpStream { socket_id: sock, peer_addr: *addr });
                    }
                }
                SocketAddr::V6(_) => continue, // IPv6 not supported
            }
        }
        Err(io::Error::from_kind(io::ErrorKind::ConnectionRefused))
    }

    /// Get the peer address.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.peer_addr)
    }

    /// Get the local address (not available on anyOS, returns placeholder).
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)))
    }

    /// Shutdown the connection.
    pub fn shutdown(&self, _how: Shutdown) -> io::Result<()> {
        anyos_std::net::tcp_close(self.socket_id);
        Ok(())
    }

    /// Try to clone this stream (not supported).
    pub fn try_clone(&self) -> io::Result<TcpStream> {
        Err(io::Error::new(io::ErrorKind::Other, "try_clone not supported"))
    }

    /// Set read timeout.
    pub fn set_read_timeout(&self, _dur: Option<core::time::Duration>) -> io::Result<()> {
        Ok(()) // Timeout not configurable per-socket on anyOS
    }

    /// Set write timeout.
    pub fn set_write_timeout(&self, _dur: Option<core::time::Duration>) -> io::Result<()> {
        Ok(())
    }

    /// Set non-blocking mode.
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        anyos_std::fs::set_fd_nonblock(self.socket_id, nonblocking);
        Ok(())
    }

    /// Set TCP nodelay (no-op on anyOS).
    pub fn set_nodelay(&self, _nodelay: bool) -> io::Result<()> {
        Ok(())
    }

    pub fn nodelay(&self) -> io::Result<bool> {
        Ok(false)
    }
}

impl Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let ret = anyos_std::net::tcp_recv(self.socket_id, buf);
        if ret == u32::MAX {
            Err(io::Error::from_kind(io::ErrorKind::ConnectionRefused))
        } else {
            Ok(ret as usize)
        }
    }
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let ret = anyos_std::net::tcp_send(self.socket_id, buf);
        if ret == u32::MAX {
            Err(io::Error::from_kind(io::ErrorKind::BrokenPipe))
        } else {
            Ok(ret as usize)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        anyos_std::net::tcp_close(self.socket_id);
    }
}

// ── TcpListener ─────────────────────────────────────────────────────────────

/// A TCP listener, mirroring std::net::TcpListener.
pub struct TcpListener {
    socket_id: u32,
    local_addr: SocketAddr,
}

impl TcpListener {
    /// Bind to a local address and start listening.
    pub fn bind<A: ToSocketAddrs>(addr: A) -> io::Result<TcpListener> {
        let addr = addr.to_socket_addrs()?.next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no addresses"))?;
        let port = addr.port();
        let sock = anyos_std::net::tcp_listen(port, 16);
        if sock == u32::MAX {
            return Err(io::Error::from_kind(io::ErrorKind::Other));
        }
        Ok(TcpListener { socket_id: sock, local_addr: addr })
    }

    /// Accept a new connection.
    pub fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        let (sock_id, ip, port) = anyos_std::net::tcp_accept(self.socket_id);
        if sock_id == u32::MAX {
            return Err(io::Error::from_kind(io::ErrorKind::TimedOut));
        }
        let peer = SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
            port,
        ));
        Ok((TcpStream { socket_id: sock_id, peer_addr: peer }, peer))
    }

    /// Get the local address.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.local_addr)
    }

    /// Iterator over incoming connections.
    pub fn incoming(&self) -> Incoming<'_> {
        Incoming { listener: self }
    }

    /// Set non-blocking mode.
    pub fn set_nonblocking(&self, _nonblocking: bool) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        anyos_std::net::tcp_close(self.socket_id);
    }
}

/// Iterator over incoming TCP connections.
pub struct Incoming<'a> {
    listener: &'a TcpListener,
}

impl<'a> Iterator for Incoming<'a> {
    type Item = io::Result<TcpStream>;
    fn next(&mut self) -> Option<io::Result<TcpStream>> {
        Some(self.listener.accept().map(|(stream, _)| stream))
    }
}

/// Shutdown direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    Read,
    Write,
    Both,
}
