// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Unified TCP/TLS stream for email protocols.
//!
//! Wraps `anyos_std::net::tcp_*` and the BearSSL TLS layer to provide
//! a single interface for plain and encrypted connections.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::net;

// ---------------------------------------------------------------------------
// TLS FFI (same pattern as surf/tls.rs)
// ---------------------------------------------------------------------------

mod tls {
    use anyos_std::net;

    pub type TlsHandle = u32;

    #[no_mangle]
    extern "C" fn anyos_tcp_send(fd: i32, data: *const u8, len: i32) -> i32 {
        if data.is_null() || len <= 0 {
            return -1;
        }
        let buf = unsafe { core::slice::from_raw_parts(data, len as usize) };
        let n = net::tcp_send(fd as u32, buf);
        if n == u32::MAX {
            -1
        } else {
            n as i32
        }
    }

    #[no_mangle]
    extern "C" fn anyos_tcp_recv(fd: i32, data: *mut u8, len: i32) -> i32 {
        if data.is_null() || len <= 0 {
            return -1;
        }
        let buf = unsafe { core::slice::from_raw_parts_mut(data, len as usize) };
        let n = net::tcp_recv(fd as u32, buf);
        if n == u32::MAX {
            -1
        } else {
            n as i32
        }
    }

    #[no_mangle]
    extern "C" fn anyos_sleep(ms: i32) {
        anyos_std::process::sleep(ms as u32);
    }

    #[no_mangle]
    extern "C" fn anyos_random(buf: *mut u8, len: i32) -> i32 {
        if buf.is_null() || len <= 0 {
            return -1;
        }
        let slice = unsafe { core::slice::from_raw_parts_mut(buf, len as usize) };
        anyos_std::sys::random(slice) as i32
    }

    extern "C" {
        fn tls_connect(fd: i32, host: *const u8) -> i32;
        fn tls_send(handle: i32, data: *const u8, len: i32) -> i32;
        fn tls_recv(handle: i32, data: *mut u8, len: i32) -> i32;
        fn tls_close(handle: i32);
    }

    pub fn connect(fd: u32, host: &str) -> Result<TlsHandle, i32> {
        let mut host_buf = [0u8; 256];
        let len = host.len().min(host_buf.len() - 1);
        host_buf[..len].copy_from_slice(&host.as_bytes()[..len]);
        host_buf[len] = 0;
        let handle = unsafe { tls_connect(fd as i32, host_buf.as_ptr()) };
        if handle > 0 {
            Ok(handle as TlsHandle)
        } else {
            Err(handle)
        }
    }

    pub fn send(handle: TlsHandle, data: &[u8]) -> i32 {
        unsafe { tls_send(handle as i32, data.as_ptr(), data.len() as i32) }
    }

    pub fn recv(handle: TlsHandle, buf: &mut [u8]) -> i32 {
        unsafe { tls_recv(handle as i32, buf.as_mut_ptr(), buf.len() as i32) }
    }

    pub fn close(handle: TlsHandle) {
        unsafe {
            tls_close(handle as i32);
        }
    }
}

// ---------------------------------------------------------------------------
// Connection Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ConnError {
    DnsFailure,
    ConnectFailed,
    TlsHandshakeFailed,
    SendFailed,
    RecvFailed,
    Timeout,
    Closed,
}

// ---------------------------------------------------------------------------
// TcpStream
// ---------------------------------------------------------------------------

pub struct TcpStream {
    socket_id: u32,
    use_tls: bool,
    tls_handle: Option<tls::TlsHandle>,
    host: String,
    /// Internal line-read buffer.
    read_buf: Vec<u8>,
    read_pos: usize,
    read_len: usize,
}

impl TcpStream {
    const BUF_SIZE: usize = 8192;

    /// Connect to host:port, optionally with TLS.
    pub fn connect(host: &str, port: u16, use_tls: bool) -> Result<Self, ConnError> {
        // DNS resolve
        let mut ip = [0u8; 4];
        if net::dns(host, &mut ip) != 0 {
            return Err(ConnError::DnsFailure);
        }

        // TCP connect (10 second timeout)
        let sock = net::tcp_connect(&ip, port, 10000);
        if sock == u32::MAX {
            return Err(ConnError::ConnectFailed);
        }

        // TLS handshake if needed
        let tls_handle = if use_tls {
            match tls::connect(sock, host) {
                Ok(handle) => Some(handle),
                Err(_) => {
                    net::tcp_close(sock);
                    return Err(ConnError::TlsHandshakeFailed);
                }
            }
        } else {
            None
        };

        let mut read_buf = Vec::new();
        read_buf.resize(Self::BUF_SIZE, 0);

        Ok(Self {
            socket_id: sock,
            use_tls,
            tls_handle,
            host: String::from(host),
            read_buf,
            read_pos: 0,
            read_len: 0,
        })
    }

    /// Upgrade an existing plaintext connection to TLS (for STARTTLS).
    pub fn upgrade_to_tls(&mut self) -> Result<(), ConnError> {
        let handle =
            tls::connect(self.socket_id, &self.host).map_err(|_| ConnError::TlsHandshakeFailed)?;
        self.use_tls = true;
        self.tls_handle = Some(handle);
        // Clear read buffer after upgrade
        self.read_pos = 0;
        self.read_len = 0;
        Ok(())
    }

    /// Send raw bytes.
    pub fn send(&self, data: &[u8]) -> Result<usize, ConnError> {
        if self.use_tls {
            let handle = self.tls_handle.ok_or(ConnError::TlsHandshakeFailed)?;
            let n = tls::send(handle, data);
            if n < 0 {
                return Err(ConnError::SendFailed);
            }
            Ok(n as usize)
        } else {
            let n = net::tcp_send(self.socket_id, data);
            if n == u32::MAX {
                return Err(ConnError::SendFailed);
            }
            Ok(n as usize)
        }
    }

    /// Receive raw bytes.
    pub fn recv(&self, buf: &mut [u8]) -> Result<usize, ConnError> {
        if self.use_tls {
            let handle = self.tls_handle.ok_or(ConnError::TlsHandshakeFailed)?;
            let n = tls::recv(handle, buf);
            if n < 0 {
                return Err(ConnError::RecvFailed);
            }
            if n == 0 {
                return Err(ConnError::Closed);
            }
            Ok(n as usize)
        } else {
            let n = net::tcp_recv(self.socket_id, buf);
            if n == u32::MAX {
                return Err(ConnError::RecvFailed);
            }
            if n == 0 {
                return Err(ConnError::Closed);
            }
            Ok(n as usize)
        }
    }

    /// Send a line (appends \r\n).
    pub fn send_line(&self, line: &str) -> Result<(), ConnError> {
        let mut data = Vec::with_capacity(line.len() + 2);
        data.extend_from_slice(line.as_bytes());
        data.push(b'\r');
        data.push(b'\n');
        self.send_all(&data)
    }

    /// Send all bytes (retry until complete).
    pub fn send_all(&self, data: &[u8]) -> Result<(), ConnError> {
        let mut sent = 0;
        while sent < data.len() {
            let n = self.send(&data[sent..])?;
            if n == 0 {
                return Err(ConnError::SendFailed);
            }
            sent += n;
        }
        Ok(())
    }

    /// Read a single line (terminated by \r\n or \n).
    /// Returns the line without the trailing newline.
    pub fn recv_line(&mut self) -> Result<String, ConnError> {
        let mut line = Vec::with_capacity(512);

        loop {
            // Check buffered data for newline
            while self.read_pos < self.read_len {
                let b = self.read_buf[self.read_pos];
                self.read_pos += 1;

                if b == b'\n' {
                    // Strip trailing \r
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    // Try UTF-8 first, fall back to Latin-1
                    return Ok(match core::str::from_utf8(&line) {
                        Ok(s) => String::from(s),
                        Err(_) => {
                            // Latin-1 fallback: decode each byte as Unicode code point
                            let mut s = String::with_capacity(line.len());
                            for &b in &line {
                                s.push(b as char);
                            }
                            s
                        }
                    });
                }

                line.push(b);

                // Safety limit
                if line.len() > 65536 {
                    return Ok(match core::str::from_utf8(&line) {
                        Ok(s) => String::from(s),
                        Err(_) => {
                            // Latin-1 fallback
                            let mut s = String::with_capacity(line.len());
                            for &b in &line {
                                s.push(b as char);
                            }
                            s
                        }
                    });
                }
            }

            // Need more data
            self.read_pos = 0;
            self.read_len = 0;

            let n = if self.use_tls {
                let handle = self.tls_handle.ok_or(ConnError::TlsHandshakeFailed)?;
                let r = tls::recv(handle, &mut self.read_buf);
                if r < 0 {
                    return Err(ConnError::RecvFailed);
                }
                r as usize
            } else {
                // Retry with timeout for non-blocking recv
                let mut retries = 0;
                let mut n;
                loop {
                    n = net::tcp_recv(self.socket_id, &mut self.read_buf);
                    if n != 0 {
                        break;
                    }
                    anyos_std::process::sleep(1);
                    retries += 1;
                    if retries > 10000 {
                        return Err(ConnError::Timeout);
                    }
                }
                if n == u32::MAX {
                    return Err(ConnError::RecvFailed);
                }
                n as usize
            };

            if n == 0 {
                // Connection closed - return what we have
                if !line.is_empty() {
                    return Ok(String::from(core::str::from_utf8(&line).unwrap_or("")));
                }
                return Err(ConnError::Closed);
            }

            self.read_len = n;
        }
    }

    /// Read all available data until connection closes or a pattern is found.
    /// Useful for reading multi-line responses terminated by a marker.
    pub fn recv_until_marker(&mut self, marker: &str) -> Result<String, ConnError> {
        let mut response = String::new();

        loop {
            let line = self.recv_line()?;
            response.push_str(&line);
            response.push('\n');

            if line == marker || line.starts_with(marker) {
                break;
            }
        }

        Ok(response)
    }

    /// Read exactly `n` bytes from the stream.
    pub fn recv_exact(&mut self, count: usize) -> Result<Vec<u8>, ConnError> {
        let mut data = Vec::with_capacity(count);
        let mut remaining = count;

        // First, drain buffered data
        while remaining > 0 && self.read_pos < self.read_len {
            data.push(self.read_buf[self.read_pos]);
            self.read_pos += 1;
            remaining -= 1;
        }

        // Read more from socket
        while remaining > 0 {
            let mut buf = [0u8; 4096];
            let to_read = remaining.min(buf.len());
            let n = if self.use_tls {
                let handle = self.tls_handle.ok_or(ConnError::TlsHandshakeFailed)?;
                let r = tls::recv(handle, &mut buf[..to_read]);
                if r < 0 {
                    return Err(ConnError::RecvFailed);
                }
                r as usize
            } else {
                let r = net::tcp_recv(self.socket_id, &mut buf[..to_read]);
                if r == u32::MAX {
                    return Err(ConnError::RecvFailed);
                }
                r as usize
            };

            if n == 0 {
                break;
            }

            data.extend_from_slice(&buf[..n]);
            remaining -= n;
        }

        Ok(data)
    }

    /// Close the connection.
    pub fn close(&mut self) {
        if let Some(handle) = self.tls_handle.take() {
            tls::close(handle);
        }
        self.use_tls = false;
        net::tcp_close(self.socket_id);
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        self.close();
    }
}
