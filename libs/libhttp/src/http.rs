//! HTTP/1.1 client core for libhttp.
//!
//! Supports GET and POST requests with automatic redirect following,
//! Content-Length and chunked transfer-encoding body reading,
//! gzip/deflate content-encoding decompression, and HTTPS via BearSSL.

use alloc::string::String;
use alloc::vec::Vec;

use crate::syscall;
use crate::tls;
use crate::url::{
    Url, clone_url, find_header_value, parse_hex, parse_u32, parse_url,
    push_u32, resolve_url, starts_with_ignore_case, parse_ip,
};
use crate::deflate;

// ── Error codes ─────────────────────────────────────────────────────────────

/// Error codes returned by HTTP operations.
pub const ERR_NONE: u32 = 0;
pub const ERR_INVALID_URL: u32 = 1;
pub const ERR_DNS_FAILURE: u32 = 2;
pub const ERR_CONNECT_FAILURE: u32 = 3;
pub const ERR_SEND_FAILURE: u32 = 4;
pub const ERR_NO_RESPONSE: u32 = 5;
pub const ERR_TOO_MANY_REDIRECTS: u32 = 6;
pub const ERR_TLS_HANDSHAKE_FAILED: u32 = 7;
pub const ERR_BUFFER_TOO_SMALL: u32 = 8;
pub const ERR_FILE_WRITE: u32 = 9;

// ── Constants ───────────────────────────────────────────────────────────────

const MAX_REDIRECTS: usize = 10;
const CONNECT_TIMEOUT_MS: u32 = 10_000;
const MAX_HEADER_SIZE: usize = 16384;
const RECV_BUF_SIZE: usize = 16384;

// ── Last request state ──────────────────────────────────────────────────────

static mut LAST_STATUS: u32 = 0;
static mut LAST_ERROR: u32 = 0;

/// Progress callback set by `download_to_file()` for use by body readers.
static mut PROGRESS_CB: Option<extern "C" fn(u32, u32, u64)> = None;
/// Userdata passed to the progress callback.
static mut PROGRESS_UD: u64 = 0;

/// Set the last HTTP status code.
pub(crate) fn set_status(status: u32) {
    unsafe { LAST_STATUS = status; }
}

/// Set the last error code.
pub(crate) fn set_error(err: u32) {
    unsafe { LAST_ERROR = err; }
}

/// Get the last HTTP status code.
pub fn last_status() -> u32 {
    unsafe { LAST_STATUS }
}

/// Get the last error code.
pub fn last_error() -> u32 {
    unsafe { LAST_ERROR }
}

// ── HTTP GET to buffer ──────────────────────────────────────────────────────

/// Perform an HTTP(S) GET request and return the response body.
/// Sets last_status and last_error.
pub fn get(url_str: &str) -> Option<Vec<u8>> {
    set_status(0);
    set_error(ERR_NONE);

    let url = match parse_url(url_str) {
        Some(u) => u,
        None => {
            set_error(ERR_INVALID_URL);
            return None;
        }
    };

    match fetch_get(&url, false) {
        Ok((status, body)) => {
            set_status(status as u32);
            Some(body)
        }
        Err(err) => {
            set_error(err);
            None
        }
    }
}

/// Perform an HTTP(S) GET and write the response body directly to a file.
/// Uses raw mode (no Accept-Encoding, no decompression) so the file is
/// stored with exactly the bytes the server sends.
pub fn download(url_str: &str, path: &str) -> bool {
    download_to_file(url_str, path, None, 0)
}

/// Download a URL to a file with an optional progress callback.
/// The callback receives `(received_bytes, total_bytes, userdata)` after each chunk.
/// `total_bytes` is 0 if the server did not send a Content-Length header.
///
/// Uses the same proven receive path as `get()` (accumulate in Vec),
/// with progress callbacks fired during body reception.
pub fn download_to_file(
    url_str: &str,
    path: &str,
    callback: Option<extern "C" fn(u32, u32, u64)>,
    userdata: u64,
) -> bool {
    set_status(0);
    set_error(ERR_NONE);

    let url = match parse_url(url_str) {
        Some(u) => u,
        None => {
            set_error(ERR_INVALID_URL);
            return false;
        }
    };

    // Store callback in global state for use by read_body/read_chunked_body
    unsafe {
        PROGRESS_CB = callback;
        PROGRESS_UD = userdata;
    }

    let result = fetch_get(&url, true);

    // Clear callback
    unsafe {
        PROGRESS_CB = None;
    }

    match result {
        Ok((status, body)) => {
            set_status(status as u32);
            if status >= 400 {
                return false;
            }
            write_to_file(path, &body)
        }
        Err(err) => {
            set_error(err);
            false
        }
    }
}

/// Perform an HTTP(S) POST request.
pub fn post(url_str: &str, body: &[u8], content_type: &str) -> Option<Vec<u8>> {
    set_status(0);
    set_error(ERR_NONE);

    let url = match parse_url(url_str) {
        Some(u) => u,
        None => {
            set_error(ERR_INVALID_URL);
            return None;
        }
    };

    match fetch_post_inner(&url, body, content_type) {
        Ok((status, resp_body)) => {
            set_status(status as u32);
            Some(resp_body)
        }
        Err(err) => {
            set_error(err);
            None
        }
    }
}

// ── Internal fetch logic ────────────────────────────────────────────────────

/// Core GET implementation with redirect following.
/// When `raw` is true, no Accept-Encoding is sent and the response body is
/// returned as-is (no decompression). Use this for file downloads where the
/// caller expects the exact bytes from the server.
fn fetch_get(url: &Url, raw: bool) -> Result<(u16, Vec<u8>), u32> {
    let mut current = clone_url(url);

    for _redirect_n in 0..MAX_REDIRECTS {
        let is_https = current.scheme == "https";

        // 1. Connect
        let sock = connect_to(&current.host, current.port, is_https)?;

        // 2. Build and send GET request
        let request = build_get_request(&current, raw);
        if !send_data(sock, request.as_bytes(), is_https) {
            close_conn(sock, is_https);
            return Err(ERR_SEND_FAILURE);
        }

        // 3. Receive and parse response
        match receive_response(sock, is_https, raw)? {
            ResponseAction::Redirect(location) => {
                close_conn(sock, is_https);
                current = resolve_url(&current, &location);
                continue;
            }
            ResponseAction::Complete(status, body) => {
                close_conn(sock, is_https);
                return Ok((status, body));
            }
        }
    }

    Err(ERR_TOO_MANY_REDIRECTS)
}

/// Core POST implementation with redirect following.
fn fetch_post_inner(url: &Url, body: &[u8], content_type: &str) -> Result<(u16, Vec<u8>), u32> {
    let mut current = clone_url(url);
    let mut is_first = true;

    for _redirect_n in 0..MAX_REDIRECTS {
        let is_https = current.scheme == "https";

        let sock = connect_to(&current.host, current.port, is_https)?;

        let request = if is_first {
            build_post_request(&current, body, content_type)
        } else {
            build_get_request(&current, false)
        };

        if !send_data(sock, request.as_bytes(), is_https) {
            close_conn(sock, is_https);
            return Err(ERR_SEND_FAILURE);
        }

        // Send POST body after headers (only on first request, not redirects)
        if is_first && !body.is_empty() {
            if !send_data(sock, body, is_https) {
                close_conn(sock, is_https);
                return Err(ERR_SEND_FAILURE);
            }
        }

        match receive_response(sock, is_https, false)? {
            ResponseAction::Redirect(location) => {
                close_conn(sock, is_https);
                current = resolve_url(&current, &location);
                is_first = false;
                continue;
            }
            ResponseAction::Complete(status, resp_body) => {
                close_conn(sock, is_https);
                return Ok((status, resp_body));
            }
        }
    }

    Err(ERR_TOO_MANY_REDIRECTS)
}

// ── Connection management ───────────────────────────────────────────────────

/// Establish a TCP connection (+ TLS handshake for HTTPS).
fn connect_to(host: &str, port: u16, is_https: bool) -> Result<u32, u32> {
    let ip = resolve_host(host).ok_or(ERR_DNS_FAILURE)?;
    let sock = syscall::tcp_connect(&ip, port, CONNECT_TIMEOUT_MS);
    if sock == u32::MAX {
        return Err(ERR_CONNECT_FAILURE);
    }
    if is_https {
        let ret = tls::connect(sock, host);
        if ret != 0 {
            syscall::tcp_close(sock);
            return Err(ERR_TLS_HANDSHAKE_FAILED);
        }
    }
    Ok(sock)
}

/// Close a connection (TLS + TCP).
fn close_conn(sock: u32, is_https: bool) {
    if is_https { tls::close(); }
    syscall::tcp_close(sock);
}

/// Resolve a hostname to an IPv4 address.
fn resolve_host(host: &str) -> Option<[u8; 4]> {
    if let Some(ip) = parse_ip(host) {
        return Some(ip);
    }
    let mut resolved = [0u8; 4];
    if syscall::dns_resolve(host, &mut resolved) == 0 {
        Some(resolved)
    } else {
        None
    }
}

// ── Data transport ──────────────────────────────────────────────────────────

/// Send data over plain TCP or TLS.
fn send_data(sock: u32, data: &[u8], is_https: bool) -> bool {
    if is_https {
        tls::send(data) >= 0
    } else {
        syscall::tcp_send(sock, data) != u32::MAX
    }
}

/// Receive data from plain TCP or TLS.
/// Returns bytes received, or 0 on true EOF/error.
///
/// For plain TCP, retries up to 3 times on timeout (u32::MAX) if the
/// connection is still alive, to handle transient delays during large transfers.
fn recv_some(sock: u32, buf: &mut [u8], is_https: bool) -> usize {
    if is_https {
        // TLS path — retry logic is in anyos_tcp_recv callback
        let n = tls::recv(buf);
        if n <= 0 { 0 } else { n as usize }
    } else {
        // Plain TCP path — retry on transient timeouts
        for _ in 0..3 {
            let n = syscall::tcp_recv(sock, buf);
            if n == 0 {
                return 0; // EOF
            }
            if n != u32::MAX {
                return n as usize; // Got data
            }
            // Timeout — check if connection is still alive
            let avail = syscall::tcp_recv_available(sock);
            if avail == u32::MAX || avail == u32::MAX - 1 {
                return 0; // Error or EOF
            }
            // Connection alive, retry after brief delay
            syscall::sleep(100);
        }
        0
    }
}

// ── Response parsing ────────────────────────────────────────────────────────

enum ResponseAction {
    Redirect(String),
    Complete(u16, Vec<u8>),
}

/// Receive and parse an HTTP response (headers + body).
/// When `raw` is true, body decompression is skipped.
fn receive_response(sock: u32, is_https: bool, raw: bool) -> Result<ResponseAction, u32> {
    // Receive headers
    let mut response_buf: Vec<u8> = Vec::new();
    let mut recv_buf = [0u8; RECV_BUF_SIZE];
    let header_end;

    loop {
        let n = recv_some(sock, &mut recv_buf, is_https);
        if n == 0 {
            return Err(ERR_NO_RESPONSE);
        }
        response_buf.extend_from_slice(&recv_buf[..n]);

        if let Some(end) = find_header_end_bytes(&response_buf) {
            header_end = end;
            break;
        }
        if response_buf.len() > MAX_HEADER_SIZE {
            return Err(ERR_NO_RESPONSE);
        }
    }

    // Parse status line
    let header_str = core::str::from_utf8(&response_buf[..header_end]).unwrap_or("");
    let (status, _reason) = parse_status_line(header_str);

    // Handle redirects
    if is_redirect(status) {
        if let Some(location) = find_header_value(header_str, "location") {
            return Ok(ResponseAction::Redirect(String::from(location)));
        }
        return Ok(ResponseAction::Complete(status, Vec::new()));
    }

    // Read body
    let is_chunked = find_header_value(header_str, "transfer-encoding")
        .map(|v| v.contains("chunked"))
        .unwrap_or(false);
    let content_length = parse_content_length(header_str);
    let content_encoding = find_header_value(header_str, "content-encoding")
        .map(|v| String::from(v));

    let mut trailing = Vec::new();
    if header_end < response_buf.len() {
        trailing.extend_from_slice(&response_buf[header_end..]);
    }

    let raw_body = if is_chunked {
        read_chunked_body(sock, &trailing, is_https)
    } else {
        read_body(sock, &trailing, content_length, is_https)
    };

    // Decompress if content-encoded (skip in raw mode for file downloads)
    let body = if raw { raw_body } else { decompress_body(raw_body, &content_encoding) };

    Ok(ResponseAction::Complete(status, body))
}

// ── Request building ────────────────────────────────────────────────────────

/// Build an HTTP GET request string.
/// When `raw` is true, Accept-Encoding is omitted so the server sends
/// uncompressed bytes (important for file downloads).
fn build_get_request(url: &Url, raw: bool) -> String {
    let mut req = String::new();
    req.push_str("GET ");
    req.push_str(&url.path);
    req.push_str(" HTTP/1.1\r\nHost: ");
    req.push_str(&url.host);
    if (url.scheme == "http" && url.port != 80) || (url.scheme == "https" && url.port != 443) {
        req.push(':');
        push_u32(&mut req, url.port as u32);
    }
    req.push_str("\r\nUser-Agent: libhttp/1.0 (anyOS)");
    req.push_str("\r\nAccept: */*");
    if !raw {
        req.push_str("\r\nAccept-Encoding: gzip, deflate");
    }
    req.push_str("\r\nConnection: close");
    req.push_str("\r\n\r\n");
    req
}

/// Build an HTTP POST request string.
fn build_post_request(url: &Url, body: &[u8], content_type: &str) -> String {
    let mut req = String::new();
    req.push_str("POST ");
    req.push_str(&url.path);
    req.push_str(" HTTP/1.1\r\nHost: ");
    req.push_str(&url.host);
    if (url.scheme == "http" && url.port != 80) || (url.scheme == "https" && url.port != 443) {
        req.push(':');
        push_u32(&mut req, url.port as u32);
    }
    req.push_str("\r\nUser-Agent: libhttp/1.0 (anyOS)");
    req.push_str("\r\nAccept: */*");
    req.push_str("\r\nContent-Type: ");
    req.push_str(content_type);
    req.push_str("\r\nContent-Length: ");
    push_u32(&mut req, body.len() as u32);
    req.push_str("\r\nConnection: close");
    req.push_str("\r\n\r\n");
    // Note: body bytes appended separately during send
    req
}

// ── Body reading ────────────────────────────────────────────────────────────

/// Read body with Content-Length or until connection close.
///
/// When Content-Length is known, retries up to 5 times on recv_some()==0
/// if the expected size hasn't been reached yet. This handles cases where
/// TCP timeouts cause transient recv failures during large downloads.
fn read_body(sock: u32, initial: &[u8], content_length: Option<u32>, is_https: bool) -> Vec<u8> {
    let capacity = content_length
        .map(|cl| (cl as usize).min(32 * 1024 * 1024))
        .unwrap_or(65536);
    let mut body: Vec<u8> = Vec::with_capacity(capacity);
    body.extend_from_slice(initial);

    let total = content_length.unwrap_or(0);

    // Fire progress callback for initial data
    unsafe {
        if let Some(cb) = PROGRESS_CB {
            cb(body.len() as u32, total, PROGRESS_UD);
        }
    }

    let mut recv_buf = [0u8; RECV_BUF_SIZE];
    let mut consecutive_failures = 0u32;
    const MAX_RETRIES: u32 = 5;

    loop {
        if let Some(cl) = content_length {
            if body.len() >= cl as usize { break; }
        }
        let n = recv_some(sock, &mut recv_buf, is_https);
        if n == 0 {
            // recv_some returned 0 — could be EOF or transient failure.
            // If we know Content-Length and haven't received enough, retry.
            if let Some(cl) = content_length {
                if body.len() < cl as usize && consecutive_failures < MAX_RETRIES {
                    consecutive_failures += 1;
                    syscall::sleep(200);
                    continue;
                }
            }
            break;
        }
        consecutive_failures = 0;
        body.extend_from_slice(&recv_buf[..n]);

        unsafe {
            if let Some(cb) = PROGRESS_CB {
                cb(body.len() as u32, total, PROGRESS_UD);
            }
        }
    }
    body
}

/// Read a chunked transfer-encoded body.
///
/// Retries on transient recv failures (up to 5 consecutive) to handle
/// TCP timeouts during large chunked transfers.
fn read_chunked_body(sock: u32, initial: &[u8], is_https: bool) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::with_capacity(RECV_BUF_SIZE * 4);
    buf.extend_from_slice(initial);
    let mut cursor: usize = 0;
    let mut body: Vec<u8> = Vec::with_capacity(65536);
    let mut recv_buf = [0u8; RECV_BUF_SIZE];

    const MAX_RETRIES: u32 = 5;

    loop {
        // Find chunk size line
        let chunk_size;
        let mut failures = 0u32;
        loop {
            if let Some(crlf) = find_crlf(&buf[cursor..]) {
                let size_str = core::str::from_utf8(&buf[cursor..cursor + crlf]).unwrap_or("0");
                let hex_str = match size_str.find(';') {
                    Some(i) => &size_str[..i],
                    None => size_str,
                };
                chunk_size = parse_hex(hex_str.trim());
                cursor += crlf + 2;
                break;
            }
            let n = recv_some(sock, &mut recv_buf, is_https);
            if n == 0 {
                failures += 1;
                if failures >= MAX_RETRIES { return body; }
                syscall::sleep(200);
                continue;
            }
            failures = 0;
            buf.extend_from_slice(&recv_buf[..n]);
        }

        if chunk_size == 0 { break; }

        // Read chunk data
        failures = 0;
        while buf.len() - cursor < chunk_size {
            let n = recv_some(sock, &mut recv_buf, is_https);
            if n == 0 {
                failures += 1;
                if failures >= MAX_RETRIES { break; }
                syscall::sleep(200);
                continue;
            }
            failures = 0;
            buf.extend_from_slice(&recv_buf[..n]);
        }

        let available = (buf.len() - cursor).min(chunk_size);
        body.extend_from_slice(&buf[cursor..cursor + available]);
        cursor += available;

        // Fire progress callback after each chunk
        unsafe {
            if let Some(cb) = PROGRESS_CB {
                cb(body.len() as u32, 0, PROGRESS_UD);
            }
        }

        // Skip trailing CRLF
        failures = 0;
        while buf.len() - cursor < 2 {
            let n = recv_some(sock, &mut recv_buf, is_https);
            if n == 0 {
                failures += 1;
                if failures >= MAX_RETRIES { return body; }
                syscall::sleep(200);
                continue;
            }
            failures = 0;
            buf.extend_from_slice(&recv_buf[..n]);
        }
        if buf[cursor] == b'\r' && buf[cursor + 1] == b'\n' {
            cursor += 2;
        }

        // Compact buffer periodically
        if cursor > 65536 {
            buf.drain(..cursor);
            cursor = 0;
        }
    }

    body
}

// ── Header parsing helpers ──────────────────────────────────────────────────

/// Find the end of HTTP headers (\r\n\r\n) in raw bytes.
fn find_header_end_bytes(data: &[u8]) -> Option<usize> {
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

/// Parse the HTTP status line (e.g. "HTTP/1.1 200 OK").
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
    let code = crate::url::parse_u16(&code_str[..code_str.len().min(3)]).unwrap_or(0);
    (code, reason)
}

/// Parse Content-Length header value.
fn parse_content_length(headers: &str) -> Option<u32> {
    let val = find_header_value(headers, "content-length")?;
    parse_u32(val)
}

/// Check if an HTTP status code indicates a redirect.
fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Find \r\n in a byte slice.
fn find_crlf(data: &[u8]) -> Option<usize> {
    if data.len() < 2 { return None; }
    for i in 0..data.len() - 1 {
        if data[i] == b'\r' && data[i + 1] == b'\n' {
            return Some(i);
        }
    }
    None
}

/// Decompress body based on Content-Encoding header.
fn decompress_body(raw: Vec<u8>, content_encoding: &Option<String>) -> Vec<u8> {
    if let Some(ref enc) = content_encoding {
        let enc_bytes = enc.as_bytes();
        if contains_ignore_case(enc_bytes, b"gzip") {
            if let Some(decoded) = deflate::decompress_gzip(&raw) {
                return decoded;
            }
        } else if contains_ignore_case(enc_bytes, b"deflate") {
            if let Some(decoded) = deflate::decompress_zlib(&raw)
                .or_else(|| deflate::decompress_deflate(&raw))
            {
                return decoded;
            }
        }
    }
    raw
}

/// Case-insensitive contains check for byte slices.
fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() { return false; }
    for i in 0..=haystack.len() - needle.len() {
        let mut matched = true;
        for j in 0..needle.len() {
            if crate::url::ascii_lower(haystack[i + j]) != crate::url::ascii_lower(needle[j]) {
                matched = false;
                break;
            }
        }
        if matched { return true; }
    }
    false
}

// ── File I/O helpers ────────────────────────────────────────────────────────

/// Write all bytes to an open file descriptor. Returns false on error.
fn write_all(fd: u32, data: &[u8]) -> bool {
    let mut written = 0usize;
    while written < data.len() {
        let n = syscall::write(fd, &data[written..]);
        if n == u32::MAX { return false; }
        written += n as usize;
    }
    true
}

/// Write data to a file path.
fn write_to_file(path: &str, data: &[u8]) -> bool {
    let fd = syscall::open(path, syscall::O_WRITE | syscall::O_CREATE | syscall::O_TRUNC);
    if fd == u32::MAX {
        set_error(ERR_FILE_WRITE);
        return false;
    }
    let mut written = 0usize;
    while written < data.len() {
        let n = syscall::write(fd, &data[written..]);
        if n == u32::MAX { break; }
        written += n as usize;
    }
    syscall::close(fd);
    if written != data.len() {
        set_error(ERR_FILE_WRITE);
        return false;
    }
    true
}
