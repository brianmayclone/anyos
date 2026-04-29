//! Streaming I/O for git network operations.
//!
//! Provides a `StreamReader` that reads from a TLS connection with
//! HTTP chunked transfer-encoding decoding built in.
//! Used by the pack parser to process objects on-the-fly without
//! buffering the entire response in memory.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
#[cfg(feature = "host")]
use std::fs::{remove_file, File};
#[cfg(feature = "host")]
use std::io::{Read, Write};
#[cfg(feature = "host")]
use std::path::PathBuf;
#[cfg(feature = "host")]
use std::process::{Command, Stdio};
#[cfg(feature = "host")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "host")]
static HOST_SPOOL_ID: AtomicU64 = AtomicU64::new(0);

/// A streaming reader over an HTTPS connection with chunked encoding support.
pub struct HttpStream {
    tls_handle: u32,
    tcp_socket: u32,
    /// Buffered data not yet consumed by the caller.
    pub(crate) buf: Vec<u8>,
    pub(crate) buf_pos: usize,
    /// Decoded response body bytes pushed back by pack parsing.
    pushback: Vec<u8>,
    pushback_pos: usize,
    /// Logical decoded response-body position. Bytes moved into pushback are
    /// rewound and counted again when consumed.
    decoded_pos: usize,
    /// Chunked encoding state.
    chunked: bool,
    chunk_remaining: usize,
    /// Set when the final chunk (size 0) has been read.
    eof: bool,
    /// Decode git side-band pkt-lines and deliver only channel 1 pack bytes.
    sideband: bool,
    sideband_buf: Vec<u8>,
    sideband_pos: usize,
    sideband_progress_line_start: bool,
    /// Total bytes delivered to caller.
    pub total_read: usize,
    #[cfg(feature = "host")]
    host_file: Option<File>,
    #[cfg(feature = "host")]
    host_path: Option<PathBuf>,
}

impl HttpStream {
    /// Open an HTTPS connection, send a POST request, read response headers,
    /// and return a stream positioned at the start of the response body.
    pub fn post(
        host: &str,
        path: &str,
        body: &[u8],
        content_type: &str,
        extra_headers: &str,
    ) -> Result<Self, String> {
        #[cfg(feature = "host")]
        {
            return Self::post_host(host, path, body, content_type, extra_headers);
        }

        #[cfg(not(feature = "host"))]
        {
            // DNS resolve
            let mut ip = [0u8; 4];
            anyos_std::net::dns(host, &mut ip);
            if ip == [0, 0, 0, 0] {
                return Err(format!("DNS failed for {}", host));
            }

            // TCP connect
            let sock = anyos_std::net::tcp_connect(&ip, 443, 15000);
            if sock == u32::MAX {
                return Err(String::from("TCP connect failed"));
            }

            // TLS handshake
            libtls::set_transport(tls_send_cb, tls_recv_cb, tls_sleep_cb, tls_random_cb);
            let handle = libtls::connect(sock, host);
            if handle < 0 {
                anyos_std::net::tcp_close(sock);
                return Err(format!("TLS handshake failed ({})", handle));
            }
            let handle = handle as u32;

            // Build and send HTTP request (headers + body in one send)
            let request = format!(
                "POST {} HTTP/1.1\r\n\
             Host: {}\r\n\
             Content-Type: {}\r\n\
             {}\
             Content-Length: {}\r\n\
             User-Agent: git/anyos\r\n\
             Connection: close\r\n\
             \r\n",
                path,
                host,
                content_type,
                extra_headers,
                body.len()
            );

            let mut send_buf = Vec::with_capacity(request.len() + body.len());
            send_buf.extend_from_slice(request.as_bytes());
            send_buf.extend_from_slice(body);

            if libtls::send(handle, &send_buf) <= 0 {
                libtls::close(handle);
                anyos_std::net::tcp_close(sock);
                return Err(String::from("TLS send failed"));
            }

            // Read response headers
            let mut hdr_buf = Vec::with_capacity(4096);
            let mut recv_buf = [0u8; 4096];
            let header_end;
            loop {
                let n = tls_recv_patient(handle, sock, &mut recv_buf);
                if n <= 0 {
                    libtls::close(handle);
                    anyos_std::net::tcp_close(sock);
                    return Err(String::from("no response"));
                }
                hdr_buf.extend_from_slice(&recv_buf[..n as usize]);
                if let Some(pos) = hdr_buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = pos + 4;
                    break;
                }
                if hdr_buf.len() > 16384 {
                    libtls::close(handle);
                    anyos_std::net::tcp_close(sock);
                    return Err(String::from("headers too large"));
                }
            }

            let header_str = core::str::from_utf8(&hdr_buf[..header_end]).unwrap_or("");

            let leftover_len = hdr_buf.len() - header_end;
            if crate::pack::verbose() {
                anyos_std::println!("[stream] HTTP response headers:");
                for line in header_str.lines().take(10) {
                    anyos_std::println!("  {}", line);
                }
                anyos_std::println!("[stream] leftover after headers: {} bytes", leftover_len);
                if leftover_len > 0 && leftover_len <= 32 {
                    anyos_std::print!("[stream] leftover hex: ");
                    for b in &hdr_buf[header_end..] {
                        anyos_std::print!("{:02x} ", b);
                    }
                    anyos_std::println!();
                }
            }

            // Check HTTP status
            let status = parse_http_status(header_str);
            if status != 200 {
                libtls::close(handle);
                anyos_std::net::tcp_close(sock);
                return Err(format!("HTTP {}", status));
            }

            let chunked = header_str
                .to_lowercase()
                .contains("transfer-encoding: chunked");

            // Any data after headers is already buffered body data
            let leftover = hdr_buf[header_end..].to_vec();

            let mut stream = HttpStream {
                tls_handle: handle,
                tcp_socket: sock,
                buf: leftover,
                buf_pos: 0,
                pushback: Vec::new(),
                pushback_pos: 0,
                decoded_pos: 0,
                chunked,
                chunk_remaining: 0,
                eof: false,
                sideband: false,
                sideband_buf: Vec::new(),
                sideband_pos: 0,
                sideband_progress_line_start: true,
                total_read: 0,
                #[cfg(feature = "host")]
                host_file: None,
                #[cfg(feature = "host")]
                host_path: None,
            };

            // If chunked, read the first chunk header
            if chunked {
                stream.read_next_chunk_header();
            }

            Ok(stream)
        }
    }

    #[cfg(feature = "host")]
    fn post_host(
        host: &str,
        path: &str,
        body: &[u8],
        content_type: &str,
        extra_headers: &str,
    ) -> Result<Self, String> {
        let url = format!("https://{}{}", host, path);
        let spool_path = host_spool_path();
        let mut cmd = Command::new("curl");
        cmd.arg("-fsSL")
            .arg("--proto")
            .arg("=http,https")
            .arg("--max-time")
            .arg("300")
            .arg("--user-agent")
            .arg("git/anyos")
            .arg("-H")
            .arg(format!("Content-Type: {}", content_type));

        for header in extra_headers.lines() {
            let trimmed = header.trim();
            if !trimmed.is_empty() {
                cmd.arg("-H").arg(trimmed);
            }
        }

        let mut child = cmd
            .arg("-o")
            .arg(&spool_path)
            .arg("--data-binary")
            .arg("@-")
            .arg(url)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("curl spawn failed: {}", e))?;

        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| String::from("curl stdin unavailable"))?;
            stdin
                .write_all(body)
                .map_err(|e| format!("curl stdin write failed: {}", e))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("curl wait failed: {}", e))?;
        if !output.status.success() {
            let _ = remove_file(&spool_path);
            let stderr = core::str::from_utf8(&output.stderr).unwrap_or("curl failed");
            return Err(format!("curl POST failed: {}", stderr.trim()));
        }

        let file = File::open(&spool_path)
            .map_err(|e| format!("curl output open failed {}: {}", spool_path.display(), e))?;

        Ok(HttpStream {
            tls_handle: u32::MAX,
            tcp_socket: u32::MAX,
            buf: Vec::new(),
            buf_pos: 0,
            pushback: Vec::new(),
            pushback_pos: 0,
            decoded_pos: 0,
            chunked: false,
            chunk_remaining: 0,
            eof: false,
            sideband: false,
            sideband_buf: Vec::new(),
            sideband_pos: 0,
            sideband_progress_line_start: true,
            total_read: 0,
            host_file: Some(file),
            host_path: Some(spool_path),
        })
    }

    /// Read exactly `n` bytes from the stream. Blocks until all bytes are available.
    pub fn read_exact(&mut self, out: &mut [u8]) -> bool {
        let mut filled = 0;
        while filled < out.len() {
            let n = self.read(&mut out[filled..]);
            if n == 0 {
                return false;
            }
            filled += n;
        }
        true
    }

    /// Number of decoded HTTP response-body bytes consumed by callers.
    pub fn decoded_pos(&self) -> usize {
        self.decoded_pos
    }

    /// Enable git side-band demuxing for upload-pack responses.
    pub fn enable_sideband(&mut self) {
        self.sideband = true;
    }

    /// Read up to `out.len()` bytes. Returns number of bytes read (0 = EOF).
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        if self.pushback_pos < self.pushback.len() {
            let available = self.pushback.len() - self.pushback_pos;
            let n = available.min(out.len());
            out[..n].copy_from_slice(&self.pushback[self.pushback_pos..self.pushback_pos + n]);
            self.pushback_pos += n;
            self.decoded_pos += n;
            if self.pushback_pos == self.pushback.len() {
                self.pushback.clear();
                self.pushback_pos = 0;
            }
            return n;
        }

        if self.sideband {
            return self.read_sideband(out);
        }

        self.read_body(out, true)
    }

    fn read_body_exact(&mut self, out: &mut [u8]) -> bool {
        let mut filled = 0;
        while filled < out.len() {
            let n = self.read_body(&mut out[filled..], false);
            if n == 0 {
                return false;
            }
            filled += n;
        }
        true
    }

    fn read_sideband(&mut self, out: &mut [u8]) -> usize {
        loop {
            if self.sideband_pos < self.sideband_buf.len() {
                let available = self.sideband_buf.len() - self.sideband_pos;
                let n = available.min(out.len());
                out[..n]
                    .copy_from_slice(&self.sideband_buf[self.sideband_pos..self.sideband_pos + n]);
                self.sideband_pos += n;
                self.total_read += n;
                self.decoded_pos += n;
                if self.sideband_pos == self.sideband_buf.len() {
                    self.sideband_buf.clear();
                    self.sideband_pos = 0;
                }
                return n;
            }

            if self.eof {
                return 0;
            }

            if !self.read_next_sideband_packet() {
                return 0;
            }
        }
    }

    fn read_next_sideband_packet(&mut self) -> bool {
        loop {
            let mut len_hex = [0u8; 4];
            if !self.read_body_exact(&mut len_hex) {
                self.eof = true;
                return false;
            }

            let Ok(len_str) = core::str::from_utf8(&len_hex) else {
                self.eof = true;
                return false;
            };
            let Some(pkt_len) = usize::from_str_radix(len_str, 16).ok() else {
                self.eof = true;
                return false;
            };

            if pkt_len == 0 {
                continue;
            }
            if pkt_len <= 4 {
                continue;
            }

            let payload_len = pkt_len - 4;
            let mut payload = Vec::with_capacity(payload_len);
            payload.resize(payload_len, 0);
            if !self.read_body_exact(&mut payload) {
                self.eof = true;
                return false;
            }
            if payload.is_empty() {
                continue;
            }

            match payload[0] {
                1 => {
                    self.sideband_buf.extend_from_slice(&payload[1..]);
                    if !self.sideband_buf.is_empty() {
                        return true;
                    }
                }
                2 => self.print_sideband_progress(&payload[1..]),
                3 => {
                    anyos_std::print!("remote: error: ");
                    self.print_sideband_progress(&payload[1..]);
                    self.eof = true;
                    return false;
                }
                _ => {
                    if crate::pack::verbose() {
                        if let Ok(text) = core::str::from_utf8(&payload) {
                            anyos_std::println!("[stream] upload-pack control: {}", text.trim());
                        }
                    }
                }
            }
        }
    }

    fn print_sideband_progress(&mut self, payload: &[u8]) {
        for &b in payload {
            if self.sideband_progress_line_start {
                anyos_std::print!("remote: ");
                self.sideband_progress_line_start = false;
            }
            anyos_std::print!("{}", b as char);
            if b == b'\n' || b == b'\r' {
                self.sideband_progress_line_start = true;
            }
        }
    }

    fn read_body(&mut self, out: &mut [u8], count_decoded: bool) -> usize {
        if self.eof {
            return 0;
        }

        // Serve from internal buffer first
        if self.buf_pos < self.buf.len() {
            let available = self.buf.len() - self.buf_pos;
            let n = available.min(out.len());

            if self.chunked {
                // Don't read more than the current chunk
                let n = n.min(self.chunk_remaining);
                if n == 0 {
                    // Current chunk exhausted, read next chunk header
                    self.skip_chunk_crlf();
                    self.read_next_chunk_header();
                    if self.eof {
                        return 0;
                    }
                    return self.read(out); // Retry with new chunk
                }
                out[..n].copy_from_slice(&self.buf[self.buf_pos..self.buf_pos + n]);
                self.buf_pos += n;
                self.chunk_remaining -= n;
                if count_decoded {
                    self.total_read += n;
                    self.decoded_pos += n;
                }
                return n;
            }

            out[..n].copy_from_slice(&self.buf[self.buf_pos..self.buf_pos + n]);
            self.buf_pos += n;
            if count_decoded {
                self.total_read += n;
                self.decoded_pos += n;
            }
            return n;
        }

        // Buffer exhausted — read more from TLS
        #[cfg(feature = "host")]
        {
            if let Some(file) = self.host_file.as_mut() {
                match file.read(out) {
                    Ok(0) => {
                        self.eof = true;
                        0
                    }
                    Ok(n) => {
                        if count_decoded {
                            self.total_read += n;
                            self.decoded_pos += n;
                        }
                        n
                    }
                    Err(_) => {
                        self.eof = true;
                        0
                    }
                }
            } else {
                self.eof = true;
                0
            }
        }

        #[cfg(not(feature = "host"))]
        {
            self.buf.clear();
            self.buf_pos = 0;

            let mut recv_buf = [0u8; 16384];
            let n = tls_recv_patient(self.tls_handle, self.tcp_socket, &mut recv_buf);
            if n <= 0 {
                self.eof = true;
                return 0;
            }
            self.buf.extend_from_slice(&recv_buf[..n as usize]);

            // Recurse to serve from the newly filled buffer
            self.read_body(out, count_decoded)
        }
    }

    /// Put decoded response body bytes back in front of the reader.
    pub(crate) fn unread_decoded(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.decoded_pos = self.decoded_pos.saturating_sub(data.len());
        let mut next =
            Vec::with_capacity(data.len() + self.pushback.len().saturating_sub(self.pushback_pos));
        next.extend_from_slice(data);
        if self.pushback_pos < self.pushback.len() {
            next.extend_from_slice(&self.pushback[self.pushback_pos..]);
        }
        self.pushback = next;
        self.pushback_pos = 0;
    }

    /// Read the next HTTP chunk header (hex size + CRLF).
    fn read_next_chunk_header(&mut self) {
        // Ensure we have data containing a CRLF (chunk size line)
        loop {
            let remaining = &self.buf[self.buf_pos..];
            if let Some(crlf) = remaining.windows(2).position(|w| w == b"\r\n") {
                let size_str = core::str::from_utf8(&remaining[..crlf]).unwrap_or("0");
                let size_str = match size_str.find(';') {
                    Some(i) => &size_str[..i],
                    None => size_str,
                };
                let trimmed = size_str.trim();
                let Some(size) = usize::from_str_radix(trimmed, 16).ok() else {
                    if crate::pack::verbose() {
                        anyos_std::println!("[stream] invalid chunk size '{}'", trimmed);
                    }
                    self.eof = true;
                    return;
                };
                self.chunk_remaining = size;
                if crate::pack::verbose() {
                    anyos_std::println!(
                        "[stream] chunk: size_str='{}' -> {} bytes",
                        trimmed,
                        self.chunk_remaining
                    );
                }
                self.buf_pos += crlf + 2;
                if self.chunk_remaining == 0 {
                    self.eof = true;
                }
                return;
            }
            // Need more data
            let mut recv_buf = [0u8; 4096];
            let n = tls_recv_patient(self.tls_handle, self.tcp_socket, &mut recv_buf);
            if n <= 0 {
                self.eof = true;
                return;
            }
            // Compact buffer before extending
            if self.buf_pos > 0 {
                self.buf = self.buf[self.buf_pos..].to_vec();
                self.buf_pos = 0;
            }
            self.buf.extend_from_slice(&recv_buf[..n as usize]);
        }
    }

    /// Skip the trailing CRLF after a chunk body.
    fn skip_chunk_crlf(&mut self) {
        // Need 2 bytes for \r\n
        while self.buf.len() - self.buf_pos < 2 {
            let mut recv_buf = [0u8; 4096];
            let n = tls_recv_patient(self.tls_handle, self.tcp_socket, &mut recv_buf);
            if n <= 0 {
                return;
            }
            if self.buf_pos > 0 {
                self.buf = self.buf[self.buf_pos..].to_vec();
                self.buf_pos = 0;
            }
            self.buf.extend_from_slice(&recv_buf[..n as usize]);
        }
        if self.buf_pos + 2 <= self.buf.len()
            && self.buf[self.buf_pos] == b'\r'
            && self.buf[self.buf_pos + 1] == b'\n'
        {
            self.buf_pos += 2;
        } else {
            if crate::pack::verbose() {
                anyos_std::println!("[stream] missing chunk terminator");
            }
            self.eof = true;
        }
    }

    /// Close the connection.
    pub fn close(self) {
        #[cfg(not(feature = "host"))]
        {
            libtls::close(self.tls_handle);
            anyos_std::net::tcp_close(self.tcp_socket);
        }
    }
}

impl Drop for HttpStream {
    fn drop(&mut self) {
        #[cfg(feature = "host")]
        {
            self.host_file.take();
            if let Some(path) = self.host_path.take() {
                let _ = remove_file(path);
            }
        }

        #[cfg(not(feature = "host"))]
        {
            libtls::close(self.tls_handle);
            anyos_std::net::tcp_close(self.tcp_socket);
        }
    }
}

#[cfg(feature = "host")]
fn host_spool_path() -> PathBuf {
    let seq = HOST_SPOOL_ID.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "agit-upload-pack-{}-{}.bin",
        std::process::id(),
        seq
    ));
    path
}

// ── TLS helpers ─────────────────────────────────────────────────────────────

/// Recv with patience — retries on timeout, checking if connection is alive.
fn tls_recv_patient(handle: u32, sock: u32, buf: &mut [u8]) -> i32 {
    const STALL_TIMEOUT_MS: u32 = 300_000;
    let start_ms = anyos_std::sys::uptime_ms();

    loop {
        let n = libtls::recv(handle, buf);
        if n > 0 {
            return n;
        }
        if n == 0 {
            return 0;
        } // EOF

        // No complete TLS record yet, or a transient transport timeout.
        // Keep the HTTP stream alive while the TCP socket is still established,
        // but fail with a bounded stall instead of looking hung forever.
        let avail = anyos_std::net::tcp_recv_available(sock);
        if avail == u32::MAX || avail == u32::MAX - 1 {
            return 0;
        }
        let elapsed = anyos_std::sys::uptime_ms().wrapping_sub(start_ms);
        if elapsed >= STALL_TIMEOUT_MS {
            if crate::pack::verbose() {
                anyos_std::println!(
                    "[stream] TLS receive stalled for {} ms (socket {})",
                    elapsed,
                    sock
                );
            }
            return -1;
        }
        anyos_std::process::sleep(200);
    }
}

#[cfg(not(feature = "host"))]
fn parse_http_status(headers: &str) -> u16 {
    // "HTTP/1.1 200 OK"
    if let Some(line) = headers.lines().next() {
        if let Some(rest) = line
            .strip_prefix("HTTP/1.1 ")
            .or_else(|| line.strip_prefix("HTTP/1.0 "))
        {
            if let Ok(code) = rest[..3.min(rest.len())].parse::<u16>() {
                return code;
            }
        }
    }
    0
}

// TLS transport callbacks
#[cfg(not(feature = "host"))]
fn tls_send_cb(fd: u32, data: &[u8]) -> i32 {
    let n = anyos_std::net::tcp_send(fd, data);
    if n == u32::MAX {
        -1
    } else {
        n as i32
    }
}

#[cfg(not(feature = "host"))]
fn tls_recv_cb(fd: u32, buf: &mut [u8]) -> i32 {
    let avail = anyos_std::net::tcp_recv_available(fd);
    match avail {
        u32::MAX => -1,
        0xFFFFFFFE => 0,
        0 => tcp_recv_cb_blocking(fd, buf),
        _ => {
            tcp_recv_cb_blocking(fd, buf)
        }
    }
}

#[cfg(not(feature = "host"))]
fn tcp_recv_cb_blocking(fd: u32, buf: &mut [u8]) -> i32 {
    let n = anyos_std::net::tcp_recv(fd, buf);
    if n == 0 {
        0
    } else if n != u32::MAX {
        n as i32
    } else {
        -1
    }
}

#[cfg(not(feature = "host"))]
fn tls_sleep_cb(ms: u32) {
    anyos_std::process::sleep(ms);
}
#[cfg(not(feature = "host"))]
fn tls_random_cb(buf: &mut [u8]) -> u32 {
    anyos_std::sys::random(buf)
}
