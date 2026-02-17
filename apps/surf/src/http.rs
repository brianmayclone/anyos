//! HTTP/1.1 client for the Surf web browser.
//!
//! Built on `anyos_std::net` (tcp_connect, tcp_send, tcp_recv, tcp_close, dns).
//! Supports GET requests with automatic redirect following (up to 20 hops),
//! Content-Length and chunked transfer-encoding body reading, gzip/deflate
//! content-encoding decompression, and cookie persistence.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::net;

use crate::deflate;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub struct Url {
    pub scheme: String, // "http"
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub struct Response {
    pub status: u16,
    pub headers: String,
    pub body: Vec<u8>,
}

pub enum FetchError {
    InvalidUrl,
    DnsFailure,
    ConnectFailure,
    SendFailure,
    NoResponse,
    TooManyRedirects,
    TlsHandshakeFailed,
}

// ---------------------------------------------------------------------------
// Cookie jar
// ---------------------------------------------------------------------------

pub struct Cookie {
    pub domain: String,
    pub path: String,
    pub name: String,
    pub value: String,
    pub secure: bool,
    pub http_only: bool,
}

pub struct CookieJar {
    pub cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        CookieJar { cookies: Vec::new() }
    }

    /// Parse and store cookies from Set-Cookie headers.
    pub fn store_from_headers(&mut self, headers: &str, request_host: &str, request_path: &str) {
        for line in headers.split('\n') {
            let line = line.trim_end_matches('\r');
            if !starts_with_ignore_case(line, "set-cookie") { continue; }
            let rest = &line["set-cookie".len()..];
            if !rest.starts_with(':') { continue; }
            let val = rest[1..].trim_start();
            self.parse_set_cookie(val, request_host, request_path);
        }
    }

    fn parse_set_cookie(&mut self, header: &str, request_host: &str, request_path: &str) {
        // Format: name=value; Path=/; Domain=.example.com; Secure; HttpOnly
        let mut parts = header.splitn(2, ';');
        let name_value = match parts.next() {
            Some(nv) => nv.trim(),
            None => return,
        };
        let (name, value) = match name_value.find('=') {
            Some(eq) => (name_value[..eq].trim(), name_value[eq + 1..].trim()),
            None => return,
        };
        if name.is_empty() { return; }

        let mut domain = String::from(request_host);
        let mut path = String::from(request_path);
        // Trim path to directory
        if let Some(slash) = path.rfind('/') {
            path.truncate(slash + 1);
        }
        let mut secure = false;
        let mut http_only = false;

        // Parse attributes
        if let Some(attrs) = parts.next() {
            for attr in attrs.split(';') {
                let attr = attr.trim();
                if starts_with_ignore_case(attr, "domain=") {
                    let d = attr[7..].trim().trim_start_matches('.');
                    if !d.is_empty() {
                        domain = String::from(d);
                    }
                } else if starts_with_ignore_case(attr, "path=") {
                    let p = attr[5..].trim();
                    if !p.is_empty() {
                        path = String::from(p);
                    }
                } else if eq_ignore_case(attr, "secure") {
                    secure = true;
                } else if eq_ignore_case(attr, "httponly") {
                    http_only = true;
                }
                // Max-Age, Expires not handled (session cookies only)
            }
        }

        // Remove existing cookie with same name+domain+path
        self.cookies.retain(|c| {
            !(c.name == name && c.domain == domain && c.path == path)
        });

        self.cookies.push(Cookie {
            domain: domain.to_ascii_lowercase(),
            path,
            name: String::from(name),
            value: String::from(value),
            secure,
            http_only,
        });
    }

    /// Build the Cookie header value for a request.
    pub fn cookie_header(&self, host: &str, path: &str, is_secure: bool) -> Option<String> {
        let host_lower = host.to_ascii_lowercase();
        let mut pairs: Vec<(&str, &str)> = Vec::new();

        for c in &self.cookies {
            // Domain match: host ends with cookie domain
            if !domain_matches(&host_lower, &c.domain) { continue; }
            // Path match: request path starts with cookie path
            if !path.starts_with(c.path.as_str()) { continue; }
            // Secure check
            if c.secure && !is_secure { continue; }
            pairs.push((&c.name, &c.value));
        }

        if pairs.is_empty() { return None; }

        let mut s = String::new();
        for (i, (name, value)) in pairs.iter().enumerate() {
            if i > 0 { s.push_str("; "); }
            s.push_str(name);
            s.push('=');
            s.push_str(value);
        }
        Some(s)
    }
}

fn domain_matches(host: &str, domain: &str) -> bool {
    if host == domain { return true; }
    if host.ends_with(domain) {
        let prefix_len = host.len() - domain.len();
        if prefix_len > 0 && host.as_bytes()[prefix_len - 1] == b'.' {
            return true;
        }
    }
    false
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() { return false; }
    starts_with_ignore_case(a, b)
}

// Helper for to_ascii_lowercase without std
trait AsciiLowerStr {
    fn to_ascii_lowercase(&self) -> String;
}

impl AsciiLowerStr for str {
    fn to_ascii_lowercase(&self) -> String {
        let mut s = String::with_capacity(self.len());
        for b in self.bytes() {
            s.push(ascii_lower(b) as char);
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_REDIRECTS: usize = 20;
const CONNECT_TIMEOUT_MS: u32 = 10_000;
const MAX_HEADER_SIZE: usize = 16384;
const RECV_BUF_SIZE: usize = 4096;

// ---------------------------------------------------------------------------
// URL parsing
// ---------------------------------------------------------------------------

pub fn parse_url(s: &str) -> Result<Url, FetchError> {
    let (scheme, rest) = if starts_with_ignore_case(s, "https://") {
        ("https", &s[8..])
    } else if starts_with_ignore_case(s, "http://") {
        ("http", &s[7..])
    } else {
        ("http", s)
    };

    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

    let default_port: u16 = if scheme == "https" { 443 } else { 80 };

    let (host, port) = match host_port.find(':') {
        Some(idx) => {
            let port_str = &host_port[idx + 1..];
            let port = parse_u16(port_str).ok_or(FetchError::InvalidUrl)?;
            (&host_port[..idx], port)
        }
        None => (host_port, default_port),
    };

    if host.is_empty() {
        return Err(FetchError::InvalidUrl);
    }

    Ok(Url {
        scheme: String::from(scheme),
        host: String::from(host),
        port,
        path: String::from(path),
    })
}

pub fn resolve_url(base: &Url, relative: &str) -> Url {
    if starts_with_ignore_case(relative, "http://")
        || starts_with_ignore_case(relative, "https://")
    {
        match parse_url(relative) {
            Ok(u) => return u,
            Err(_) => return clone_url(base),
        }
    }

    if relative.starts_with('#') {
        let mut path = base.path.clone();
        if let Some(hash) = path.find('#') {
            path.truncate(hash);
        }
        path.push_str(relative);
        return Url {
            scheme: base.scheme.clone(),
            host: base.host.clone(),
            port: base.port,
            path,
        };
    }

    if relative.starts_with('/') {
        return Url {
            scheme: base.scheme.clone(),
            host: base.host.clone(),
            port: base.port,
            path: String::from(relative),
        };
    }

    let base_dir = match base.path.rfind('/') {
        Some(i) => &base.path[..i + 1],
        None => "/",
    };

    let mut segments: Vec<&str> = Vec::new();
    for seg in base_dir.split('/') {
        if !seg.is_empty() {
            segments.push(seg);
        }
    }

    for seg in relative.split('/') {
        if seg == "." || seg.is_empty() {
            continue;
        } else if seg == ".." {
            segments.pop();
        } else {
            segments.push(seg);
        }
    }

    let mut path = String::from("/");
    for (i, seg) in segments.iter().enumerate() {
        path.push_str(seg);
        if i + 1 < segments.len() {
            path.push('/');
        }
    }

    if relative.ends_with('/') && !path.ends_with('/') {
        path.push('/');
    }

    Url {
        scheme: base.scheme.clone(),
        host: base.host.clone(),
        port: base.port,
        path,
    }
}

// ---------------------------------------------------------------------------
// HTTP fetch
// ---------------------------------------------------------------------------

pub fn fetch(url: &Url, cookies: &mut CookieJar) -> Result<Response, FetchError> {
    let mut current = clone_url(url);

    for redirect_n in 0..MAX_REDIRECTS {
        let is_https = current.scheme == "https";
        anyos_std::println!("[http] {} GET {}:{}{}", if is_https { "HTTPS" } else { "HTTP" },
            current.host, current.port, current.path);

        // 1. DNS resolve
        let ip = match resolve_host(&current.host) {
            Some(ip) => {
                anyos_std::println!("[http] DNS: {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                ip
            }
            None => {
                anyos_std::println!("[http] DNS failed for {}", current.host);
                return Err(FetchError::DnsFailure);
            }
        };

        // 2. TCP connect
        let sock = net::tcp_connect(&ip, current.port, CONNECT_TIMEOUT_MS);
        if sock == u32::MAX {
            anyos_std::println!("[http] TCP connect failed");
            return Err(FetchError::ConnectFailure);
        }
        anyos_std::println!("[http] TCP connected (sock={})", sock);

        // 3. TLS handshake for HTTPS
        if is_https {
            anyos_std::println!("[http] TLS handshake with {}...", current.host);
            let ret = crate::tls::connect(sock, &current.host);
            if ret != 0 {
                anyos_std::println!("[http] TLS handshake FAILED (err={})", ret);
                net::tcp_close(sock);
                return Err(FetchError::TlsHandshakeFailed);
            }
            anyos_std::println!("[http] TLS handshake OK");
        }

        // 4. Build and send GET request
        let request = build_request(&current, cookies);
        let send_ok = if is_https {
            crate::tls::send(request.as_bytes()) >= 0
        } else {
            net::tcp_send(sock, request.as_bytes()) != u32::MAX
        };
        if !send_ok {
            anyos_std::println!("[http] send failed");
            if is_https { crate::tls::close(); }
            net::tcp_close(sock);
            return Err(FetchError::SendFailure);
        }
        anyos_std::println!("[http] sent {} bytes", request.len());

        // 5. Receive headers
        let mut response_buf: Vec<u8> = Vec::new();
        let mut recv_buf = [0u8; RECV_BUF_SIZE];
        let header_end;

        loop {
            let n = if is_https {
                let ret = crate::tls::recv(&mut recv_buf);
                if ret <= 0 { 0u32 } else { ret as u32 }
            } else {
                let r = net::tcp_recv(sock, &mut recv_buf);
                if r == u32::MAX { 0 } else { r }
            };
            if n == 0 {
                anyos_std::println!("[http] recv failed (buf={}B)", response_buf.len());
                if is_https { crate::tls::close(); }
                net::tcp_close(sock);
                return Err(FetchError::NoResponse);
            }
            response_buf.extend_from_slice(&recv_buf[..n as usize]);

            if let Some(end) = find_header_end(&response_buf) {
                header_end = end;
                break;
            }
            if response_buf.len() > MAX_HEADER_SIZE {
                anyos_std::println!("[http] headers too large ({}B)", response_buf.len());
                if is_https { crate::tls::close(); }
                net::tcp_close(sock);
                return Err(FetchError::NoResponse);
            }
        }

        // 6. Parse status + headers
        let header_str = core::str::from_utf8(&response_buf[..header_end]).unwrap_or("");
        let (status, _reason) = parse_status_line(header_str);
        let headers = String::from(header_str);
        anyos_std::println!("[http] HTTP {} {}", status, _reason);

        // Store cookies
        cookies.store_from_headers(header_str, &current.host, &current.path);

        // 7. Handle redirects
        if is_redirect(status) {
            if is_https { crate::tls::close(); }
            net::tcp_close(sock);
            if let Some(location) = find_header_value(header_str, "location") {
                anyos_std::println!("[http] redirect #{} -> {}", redirect_n + 1, location);
                current = resolve_url(&current, location);
                continue;
            }
            return Ok(Response { status, headers, body: Vec::new() });
        }

        // 8. Read body (chunked or content-length or until close)
        let is_chunked = find_header_value(header_str, "transfer-encoding")
            .map(|v| v.contains("chunked"))
            .unwrap_or(false);
        let content_length = parse_content_length(header_str);
        let content_encoding = find_header_value(header_str, "content-encoding")
            .map(|v| String::from(v));

        if is_chunked {
            anyos_std::println!("[http] body: chunked transfer-encoding");
        } else if let Some(cl) = content_length {
            anyos_std::println!("[http] body: content-length={}", cl);
        } else {
            anyos_std::println!("[http] body: read until close");
        }
        if let Some(ref enc) = content_encoding {
            anyos_std::println!("[http] content-encoding: {}", enc);
        }

        // Remaining data already read past the header
        let mut trailing = Vec::new();
        if header_end < response_buf.len() {
            trailing.extend_from_slice(&response_buf[header_end..]);
        }

        let raw_body = if is_chunked {
            if is_https {
                read_chunked_body_tls(&trailing)
            } else {
                read_chunked_body(sock, &trailing)
            }
        } else if is_https {
            read_body_tls(&trailing, content_length)
        } else {
            read_body(sock, &trailing, content_length)
        };

        if is_https { crate::tls::close(); }
        net::tcp_close(sock);
        anyos_std::println!("[http] raw body: {} bytes", raw_body.len());

        // 9. Decompress if content-encoded
        let body = if let Some(ref enc) = content_encoding {
            let enc_lower = enc.to_ascii_lowercase();
            if enc_lower.contains("gzip") {
                match deflate::decompress_gzip(&raw_body) {
                    Some(decoded) => {
                        anyos_std::println!("[http] gzip decompressed: {} -> {} bytes", raw_body.len(), decoded.len());
                        decoded
                    }
                    None => {
                        anyos_std::println!("[http] gzip decompression FAILED, using raw");
                        raw_body
                    }
                }
            } else if enc_lower.contains("deflate") {
                match deflate::decompress_zlib(&raw_body)
                    .or_else(|| deflate::decompress_deflate(&raw_body))
                {
                    Some(decoded) => {
                        anyos_std::println!("[http] deflate decompressed: {} -> {} bytes", raw_body.len(), decoded.len());
                        decoded
                    }
                    None => {
                        anyos_std::println!("[http] deflate decompression FAILED, using raw");
                        raw_body
                    }
                }
            } else {
                raw_body
            }
        } else {
            raw_body
        };

        anyos_std::println!("[http] final body: {} bytes", body.len());
        return Ok(Response { status, headers, body });
    }

    anyos_std::println!("[http] too many redirects");
    Err(FetchError::TooManyRedirects)
}

// ---------------------------------------------------------------------------
// Body reading
// ---------------------------------------------------------------------------

fn read_body(sock: u32, initial: &[u8], content_length: Option<u32>) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(initial);

    if let Some(cl) = content_length {
        let cl = cl as usize;
        if cl > body.len() && cl < 32 * 1024 * 1024 {
            body.reserve(cl - body.len());
        }
    }

    let mut recv_buf = [0u8; RECV_BUF_SIZE];
    loop {
        if let Some(cl) = content_length {
            if body.len() >= cl as usize { break; }
        }
        let n = net::tcp_recv(sock, &mut recv_buf);
        if n == 0 || n == u32::MAX { break; }
        body.extend_from_slice(&recv_buf[..n as usize]);
    }
    body
}

/// Read a chunked transfer-encoded body.
fn read_chunked_body(sock: u32, initial: &[u8]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(initial);
    let mut body: Vec<u8> = Vec::new();
    let mut recv_buf = [0u8; RECV_BUF_SIZE];

    loop {
        // Find chunk size line
        let chunk_size;
        loop {
            if let Some(crlf) = find_crlf(&buf) {
                let size_str = core::str::from_utf8(&buf[..crlf]).unwrap_or("0");
                // Chunk size is hex before any ';' extension
                let hex_str = match size_str.find(';') {
                    Some(i) => &size_str[..i],
                    None => size_str,
                };
                chunk_size = parse_hex(hex_str.trim());
                // Consume the size line + CRLF
                let new_buf = buf[crlf + 2..].to_vec();
                buf = new_buf;
                break;
            }
            // Need more data
            let n = net::tcp_recv(sock, &mut recv_buf);
            if n == 0 || n == u32::MAX { return body; }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        if chunk_size == 0 { break; } // final chunk

        // Read chunk_size bytes of data
        while buf.len() < chunk_size {
            let n = net::tcp_recv(sock, &mut recv_buf);
            if n == 0 || n == u32::MAX { break; }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        let take = chunk_size.min(buf.len());
        body.extend_from_slice(&buf[..take]);
        let new_buf = buf[take..].to_vec();
        buf = new_buf;

        // Skip trailing CRLF after chunk data
        loop {
            if buf.len() >= 2 {
                if buf[0] == b'\r' && buf[1] == b'\n' {
                    let new_buf = buf[2..].to_vec();
                    buf = new_buf;
                }
                break;
            }
            let n = net::tcp_recv(sock, &mut recv_buf);
            if n == 0 || n == u32::MAX { break; }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }
    }

    body
}

// ---------------------------------------------------------------------------
// TLS body reading (uses crate::tls::recv instead of tcp_recv)
// ---------------------------------------------------------------------------

fn read_body_tls(initial: &[u8], content_length: Option<u32>) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(initial);

    if let Some(cl) = content_length {
        let cl = cl as usize;
        if cl > body.len() && cl < 32 * 1024 * 1024 {
            body.reserve(cl - body.len());
        }
    }

    let mut recv_buf = [0u8; RECV_BUF_SIZE];
    loop {
        if let Some(cl) = content_length {
            if body.len() >= cl as usize { break; }
        }
        let n = crate::tls::recv(&mut recv_buf);
        if n <= 0 { break; }
        body.extend_from_slice(&recv_buf[..n as usize]);
    }
    body
}

fn read_chunked_body_tls(initial: &[u8]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(initial);
    let mut body: Vec<u8> = Vec::new();
    let mut recv_buf = [0u8; RECV_BUF_SIZE];

    loop {
        let chunk_size;
        loop {
            if let Some(crlf) = find_crlf(&buf) {
                let size_str = core::str::from_utf8(&buf[..crlf]).unwrap_or("0");
                let hex_str = match size_str.find(';') {
                    Some(i) => &size_str[..i],
                    None => size_str,
                };
                chunk_size = parse_hex(hex_str.trim());
                let new_buf = buf[crlf + 2..].to_vec();
                buf = new_buf;
                break;
            }
            let n = crate::tls::recv(&mut recv_buf);
            if n <= 0 { return body; }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        if chunk_size == 0 { break; }

        while buf.len() < chunk_size {
            let n = crate::tls::recv(&mut recv_buf);
            if n <= 0 { break; }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        let take = chunk_size.min(buf.len());
        body.extend_from_slice(&buf[..take]);
        let new_buf = buf[take..].to_vec();
        buf = new_buf;

        loop {
            if buf.len() >= 2 {
                if buf[0] == b'\r' && buf[1] == b'\n' {
                    let new_buf = buf[2..].to_vec();
                    buf = new_buf;
                }
                break;
            }
            let n = crate::tls::recv(&mut recv_buf);
            if n <= 0 { break; }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }
    }

    body
}

fn find_crlf(data: &[u8]) -> Option<usize> {
    if data.len() < 2 { return None; }
    for i in 0..data.len() - 1 {
        if data[i] == b'\r' && data[i + 1] == b'\n' {
            return Some(i);
        }
    }
    None
}

fn parse_hex(s: &str) -> usize {
    let mut val: usize = 0;
    for b in s.bytes() {
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as usize,
            b'a'..=b'f' => (b - b'a' + 10) as usize,
            b'A'..=b'F' => (b - b'A' + 10) as usize,
            _ => break,
        };
        val = val.wrapping_mul(16).wrapping_add(digit);
    }
    val
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn resolve_host(host: &str) -> Option<[u8; 4]> {
    if let Some(ip) = parse_ip(host) {
        return Some(ip);
    }
    let mut resolved = [0u8; 4];
    if net::dns(host, &mut resolved) == 0 {
        Some(resolved)
    } else {
        None
    }
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    if data.len() < 4 { return None; }
    for i in 0..data.len() - 3 {
        if data[i] == b'\r' && data[i + 1] == b'\n'
            && data[i + 2] == b'\r' && data[i + 3] == b'\n'
        {
            return Some(i + 4);
        }
    }
    None
}

fn parse_status_line(header: &str) -> (u16, &str) {
    let first_line = match header.find('\r') {
        Some(i) => &header[..i],
        None => match header.find('\n') {
            Some(i) => &header[..i],
            None => header,
        },
    };
    let mut parts = first_line.splitn(3, ' ');
    let _version = parts.next().unwrap_or("");
    let code_str = parts.next().unwrap_or("0");
    let reason = parts.next().unwrap_or("Unknown");
    let code = parse_u16(&code_str[..code_str.len().min(3)]).unwrap_or(0);
    (code, reason)
}

pub fn find_header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    for line in headers.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.len() > name.len() + 1 && starts_with_ignore_case(line, name) {
            let rest = &line[name.len()..];
            if rest.starts_with(':') {
                let val = rest[1..].trim_start();
                return Some(val);
            }
        }
    }
    None
}

fn parse_content_length(headers: &str) -> Option<u32> {
    let val = find_header_value(headers, "content-length")?;
    parse_u32(val)
}

fn build_request(url: &Url, cookies: &CookieJar) -> String {
    let mut req = String::new();
    req.push_str("GET ");
    req.push_str(&url.path);
    req.push_str(" HTTP/1.1\r\nHost: ");
    req.push_str(&url.host);
    if (url.scheme == "http" && url.port != 80) || (url.scheme == "https" && url.port != 443) {
        req.push(':');
        push_u32(&mut req, url.port as u32);
    }
    req.push_str("\r\nUser-Agent: Surf/1.0 (anyOS)");
    req.push_str("\r\nAccept: text/html,application/xhtml+xml,*/*");
    req.push_str("\r\nAccept-Encoding: gzip, deflate");
    req.push_str("\r\nConnection: close");

    // Append cookies
    let is_secure = url.scheme == "https";
    if let Some(cookie_val) = cookies.cookie_header(&url.host, &url.path, is_secure) {
        req.push_str("\r\nCookie: ");
        req.push_str(&cookie_val);
    }

    req.push_str("\r\n\r\n");
    req
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

pub fn clone_url(url: &Url) -> Url {
    Url {
        scheme: url.scheme.clone(),
        host: url.host.clone(),
        port: url.port,
        path: url.path.clone(),
    }
}

// ---------------------------------------------------------------------------
// Numeric / string helpers
// ---------------------------------------------------------------------------

fn parse_u16(s: &str) -> Option<u16> {
    let mut val: u32 = 0;
    if s.is_empty() { return None; }
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                val = val * 10 + (b - b'0') as u32;
                if val > 65535 { return None; }
            }
            _ => return None,
        }
    }
    Some(val as u16)
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val: u32 = 0;
    if s.is_empty() { return None; }
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
            }
            b'\r' | b'\n' | b' ' => break,
            _ => return None,
        }
    }
    Some(val)
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut parts = [0u8; 4];
    let mut idx = 0;
    let mut num: u32 = 0;
    let mut has_digit = false;
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                num = num * 10 + (b - b'0') as u32;
                if num > 255 { return None; }
                has_digit = true;
            }
            b'.' => {
                if !has_digit || idx >= 3 { return None; }
                parts[idx] = num as u8;
                idx += 1;
                num = 0;
                has_digit = false;
            }
            _ => return None,
        }
    }
    if !has_digit || idx != 3 { return None; }
    parts[3] = num as u8;
    Some(parts)
}

fn ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    if s.len() < prefix.len() { return false; }
    let sb = s.as_bytes();
    let pb = prefix.as_bytes();
    for i in 0..pb.len() {
        if ascii_lower(sb[i]) != ascii_lower(pb[i]) { return false; }
    }
    true
}

pub fn push_u32(s: &mut String, val: u32) {
    if val >= 10 { push_u32(s, val / 10); }
    s.push((b'0' + (val % 10) as u8) as char);
}
