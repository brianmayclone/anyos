//! HTTP/1.1 client for the Surf web browser.
//!
//! Built on `anyos_std::net` (tcp_connect, tcp_send, tcp_recv, tcp_close, dns).
//! Supports GET requests with automatic redirect following (up to 20 hops),
//! Content-Length and chunked transfer-encoding body reading, gzip/deflate
//! content-encoding decompression, and cookie persistence.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::net;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::deflate;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Clone)]
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
    /// The final URL after all redirects.
    pub final_url: Option<Url>,
    pub timing: RequestTiming,
}

#[derive(Clone, Copy, Default)]
pub struct RequestTiming {
    pub request_id: u32,
    pub submitted_ms: u32,
    pub dequeued_ms: u32,
    /// Absolute timestamp when the worker started this HTTP transaction.
    pub start_ms: u32,
    pub fetch_done_ms: u32,
    pub result_enqueued_ms: u32,
    pub ui_done_ms: u32,
    pub dns_ms: u32,
    pub connect_ms: u32,
    pub tls_ms: u32,
    pub send_ms: u32,
    /// Time from request sent until response headers were complete.
    pub wait_ms: u32,
    pub body_ms: u32,
    pub decode_ms: u32,
    pub reused_connection: bool,
    pub redirects: u32,
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FetchDestination {
    Document,
    Style,
    Image,
    Font,
    Script,
}

impl FetchDestination {
    fn accept_header(self) -> &'static str {
        match self {
            FetchDestination::Document => {
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/png,image/jpeg,image/gif,image/svg+xml,*/*;q=0.8"
            }
            FetchDestination::Style => "text/css,*/*;q=0.1",
            FetchDestination::Image => {
                "image/png,image/jpeg,image/gif,image/svg+xml,image/bmp,image/x-icon,image/vnd.microsoft.icon,*/*;q=0.5"
            }
            FetchDestination::Font => {
                "font/ttf,font/otf,font/woff,application/font-woff,application/octet-stream;q=0.8,*/*;q=0.5"
            }
            FetchDestination::Script => "*/*",
        }
    }

    fn sec_fetch_dest(self) -> &'static str {
        match self {
            FetchDestination::Document => "document",
            FetchDestination::Style => "style",
            FetchDestination::Image => "image",
            FetchDestination::Font => "font",
            FetchDestination::Script => "script",
        }
    }

    fn sec_fetch_mode(self) -> &'static str {
        match self {
            FetchDestination::Document => "navigate",
            FetchDestination::Style
            | FetchDestination::Image
            | FetchDestination::Font
            | FetchDestination::Script => "no-cors",
        }
    }
}

// ---------------------------------------------------------------------------
// Cookie jar
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Cookie {
    pub domain: String,
    pub path: String,
    pub name: String,
    pub value: String,
    pub secure: bool,
    pub http_only: bool,
}

#[derive(Clone)]
pub struct CookieJar {
    pub cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        CookieJar {
            cookies: Vec::new(),
        }
    }

    /// Parse and store cookies from Set-Cookie headers.
    pub fn store_from_headers(&mut self, headers: &str, request_host: &str, request_path: &str) {
        for line in headers.split('\n') {
            let line = line.trim_end_matches('\r');
            if !starts_with_ignore_case(line, "set-cookie") {
                continue;
            }
            let rest = &line["set-cookie".len()..];
            if !rest.starts_with(':') {
                continue;
            }
            let val = rest[1..].trim_start();
            self.parse_set_cookie(val, request_host, request_path);
        }
    }

    /// Store a cookie written by JavaScript via `document.cookie`.
    pub fn store_from_document_cookie(
        &mut self,
        cookie: &str,
        request_host: &str,
        request_path: &str,
    ) {
        self.parse_set_cookie(cookie, request_host, request_path);
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
        if name.is_empty() {
            return;
        }

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
        self.cookies
            .retain(|c| !(c.name == name && c.domain == domain && c.path == path));

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
            if !domain_matches(&host_lower, &c.domain) {
                continue;
            }
            // Path match: request path starts with cookie path
            if !path.starts_with(c.path.as_str()) {
                continue;
            }
            // Secure check
            if c.secure && !is_secure {
                continue;
            }
            pairs.push((&c.name, &c.value));
        }

        if pairs.is_empty() {
            return None;
        }

        let mut s = String::new();
        for (i, (name, value)) in pairs.iter().enumerate() {
            if i > 0 {
                s.push_str("; ");
            }
            s.push_str(name);
            s.push('=');
            s.push_str(value);
        }
        Some(s)
    }
}

fn domain_matches(host: &str, domain: &str) -> bool {
    if host == domain {
        return true;
    }
    if host.ends_with(domain) {
        let prefix_len = host.len() - domain.len();
        if prefix_len > 0 && host.as_bytes()[prefix_len - 1] == b'.' {
            return true;
        }
    }
    false
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
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
const HEADER_RECV_TIMEOUT_MS: u32 = 5_000;
const RECV_POLL_MS: u32 = 10;
const MAX_HEADER_SIZE: usize = 16384;
const RECV_BUF_SIZE: usize = 32768;

fn should_log_body_details(url: &Url) -> bool {
    url.path.ends_with(".js")
        || url.path.contains(".js?")
        || url.path.ends_with(".mjs")
        || url.path.contains(".mjs?")
        || url.path.ends_with(".css")
        || url.path.contains(".css?")
}

// ---------------------------------------------------------------------------
// Connection pool — reuses TCP (and TLS) connections across requests
// ---------------------------------------------------------------------------

const MAX_POOL_SIZE: usize = 6;
const MAX_GLOBAL_POOL_SIZE: usize = 32;
const MAX_GLOBAL_POOL_PER_HOST: usize = 8;
const MAX_POOL_IDLE_MS: u32 = 2_000;

struct PoolEntry {
    host: String,
    port: u16,
    sock: u32,
    is_https: bool,
    tls_handle: Option<crate::tls::TlsHandle>,
    idle_since_ms: u32,
}

/// Cached DNS resolution entry (hostname → IPv4 address).
struct DnsCacheEntry {
    host: String,
    ip: [u8; 4],
}

static GLOBAL_POOL_LOCK: AtomicBool = AtomicBool::new(false);
static mut GLOBAL_CONN_POOL: Option<Vec<PoolEntry>> = None;
static mut GLOBAL_DNS_CACHE: Option<Vec<DnsCacheEntry>> = None;

/// Maximum number of cached DNS entries.
const MAX_DNS_CACHE: usize = 64;

pub struct ConnPool {
    entries: Vec<PoolEntry>,
    /// Per-pool DNS cache — avoids repeated syscalls for the same hostname.
    dns_cache: Vec<DnsCacheEntry>,
}

impl ConnPool {
    pub fn new() -> Self {
        ConnPool {
            entries: Vec::new(),
            dns_cache: Vec::new(),
        }
    }

    /// Look up a hostname in the DNS cache, falling back to a syscall.
    /// Caches the result on success.
    fn resolve_cached(&mut self, host: &str) -> Option<[u8; 4]> {
        // Check cache first.
        if let Some(entry) = self.dns_cache.iter().find(|e| e.host == host) {
            return Some(entry.ip);
        }
        if let Some(ip) = global_dns_get(host) {
            self.remember_dns(host, ip);
            return Some(ip);
        }
        // Syscall fallback.
        let ip = resolve_host(host)?;
        // Cache the result.
        self.remember_dns(host, ip);
        global_dns_put(host, ip);
        Some(ip)
    }

    fn remember_dns(&mut self, host: &str, ip: [u8; 4]) {
        if self.dns_cache.iter().any(|e| e.host == host) {
            return;
        }
        if self.dns_cache.len() >= MAX_DNS_CACHE {
            self.dns_cache.remove(0);
        }
        self.dns_cache.push(DnsCacheEntry {
            host: String::from(host),
            ip,
        });
    }

    /// Take a reusable connection for the given host/port/scheme.
    fn take(&mut self, host: &str, port: u16, is_https: bool) -> Option<PoolEntry> {
        self.prune_stale();
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.host == host && e.port == port && e.is_https == is_https)
        {
            return Some(self.entries.remove(pos));
        }
        global_pool_take(host, port, is_https)
    }

    fn prune_stale(&mut self) {
        let now = anyos_std::sys::uptime_ms();
        let mut idx = 0usize;
        while idx < self.entries.len() {
            if now.wrapping_sub(self.entries[idx].idle_since_ms) > MAX_POOL_IDLE_MS {
                let old = self.entries.remove(idx);
                close_tls_handle(old.tls_handle);
                net::tcp_close(old.sock);
            } else {
                idx += 1;
            }
        }
    }

    /// Return a connection to the pool for reuse.
    fn put(
        &mut self,
        host: String,
        port: u16,
        sock: u32,
        is_https: bool,
        tls_handle: Option<crate::tls::TlsHandle>,
    ) {
        if global_pool_put(host.clone(), port, sock, is_https, tls_handle) {
            return;
        }
        while self.entries.len() >= MAX_POOL_SIZE {
            let old = self.entries.remove(0);
            close_tls_handle(old.tls_handle);
            net::tcp_close(old.sock);
        }
        self.entries.push(PoolEntry {
            host,
            port,
            sock,
            is_https,
            tls_handle,
            idle_since_ms: anyos_std::sys::uptime_ms(),
        });
    }

    fn close_all(&mut self) {
        while let Some(entry) = self.entries.pop() {
            close_tls_handle(entry.tls_handle);
            net::tcp_close(entry.sock);
        }
    }
}

fn acquire_global_pool() {
    while GLOBAL_POOL_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

fn release_global_pool() {
    GLOBAL_POOL_LOCK.store(false, Ordering::Release);
}

fn global_dns_get(host: &str) -> Option<[u8; 4]> {
    acquire_global_pool();
    let ip = unsafe {
        GLOBAL_DNS_CACHE
            .as_ref()
            .and_then(|cache| cache.iter().find(|e| e.host == host).map(|e| e.ip))
    };
    release_global_pool();
    ip
}

fn global_dns_put(host: &str, ip: [u8; 4]) {
    acquire_global_pool();
    unsafe {
        let cache = GLOBAL_DNS_CACHE.get_or_insert_with(Vec::new);
        if !cache.iter().any(|e| e.host == host) {
            if cache.len() >= MAX_DNS_CACHE {
                cache.remove(0);
            }
            cache.push(DnsCacheEntry {
                host: String::from(host),
                ip,
            });
        }
    }
    release_global_pool();
}

fn global_pool_take(host: &str, port: u16, is_https: bool) -> Option<PoolEntry> {
    acquire_global_pool();
    let now = anyos_std::sys::uptime_ms();
    let mut evicted: Vec<PoolEntry> = Vec::new();
    let entry = unsafe {
        let pool = GLOBAL_CONN_POOL.get_or_insert_with(Vec::new);
        let mut idx = 0usize;
        while idx < pool.len() {
            if now.wrapping_sub(pool[idx].idle_since_ms) > MAX_POOL_IDLE_MS {
                evicted.push(pool.remove(idx));
            } else {
                idx += 1;
            }
        }
        pool.iter()
            .position(|e| e.host == host && e.port == port && e.is_https == is_https)
            .map(|pos| pool.remove(pos))
    };
    release_global_pool();
    for entry in evicted {
        close_tls_handle(entry.tls_handle);
        net::tcp_close(entry.sock);
    }
    entry
}

fn global_pool_put(
    host: String,
    port: u16,
    sock: u32,
    is_https: bool,
    tls_handle: Option<crate::tls::TlsHandle>,
) -> bool {
    acquire_global_pool();
    let mut evicted: Vec<PoolEntry> = Vec::new();
    unsafe {
        let pool = GLOBAL_CONN_POOL.get_or_insert_with(Vec::new);
        while pool
            .iter()
            .filter(|e| e.host == host && e.port == port && e.is_https == is_https)
            .count()
            >= MAX_GLOBAL_POOL_PER_HOST
        {
            if let Some(pos) = pool
                .iter()
                .position(|e| e.host == host && e.port == port && e.is_https == is_https)
            {
                evicted.push(pool.remove(pos));
            } else {
                break;
            }
        }
        while pool.len() >= MAX_GLOBAL_POOL_SIZE {
            evicted.push(pool.remove(0));
        }
        pool.push(PoolEntry {
            host,
            port,
            sock,
            is_https,
            tls_handle,
            idle_since_ms: anyos_std::sys::uptime_ms(),
        });
    }
    release_global_pool();

    for entry in evicted {
        close_tls_handle(entry.tls_handle);
        net::tcp_close(entry.sock);
    }
    true
}

/// Open a fresh TCP connection (+ TLS handshake for HTTPS).
///
/// Uses the pool's DNS cache to avoid redundant DNS syscalls.
fn connect_fresh(
    pool: &mut ConnPool,
    host: &str,
    port: u16,
    is_https: bool,
    timing: &mut RequestTiming,
) -> Result<(u32, Option<crate::tls::TlsHandle>), FetchError> {
    let dns_start = anyos_std::sys::uptime_ms();
    let ip = match pool.resolve_cached(host) {
        Some(ip) => ip,
        None => {
            anyos_std::println!("[http] DNS failed for {}", host);
            timing.dns_ms = timing
                .dns_ms
                .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(dns_start));
            return Err(FetchError::DnsFailure);
        }
    };
    timing.dns_ms = timing
        .dns_ms
        .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(dns_start));
    let attempts = if is_https { 2 } else { 1 };
    for attempt in 0..attempts {
        anyos_std::println!(
            "[http] connect attempt {}/{} to {}:{} ({})",
            attempt + 1,
            attempts,
            host,
            port,
            if is_https { "https" } else { "http" }
        );
        let connect_start = anyos_std::sys::uptime_ms();
        let sock = net::tcp_connect(&ip, port, CONNECT_TIMEOUT_MS);
        timing.connect_ms = timing
            .connect_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(connect_start));
        if sock == u32::MAX {
            anyos_std::println!("[http] TCP connect failed");
            return Err(FetchError::ConnectFailure);
        }
        if !is_https {
            if attempt > 0 {
                anyos_std::println!(
                    "[http] retry succeeded for {}:{} on attempt {}",
                    host,
                    port,
                    attempt + 1
                );
            }
            return Ok((sock, None));
        }
        let tls_start = anyos_std::sys::uptime_ms();
        let ret = crate::tls::connect(sock, host);
        timing.tls_ms = timing
            .tls_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(tls_start));
        if ret > 0 {
            let tls_handle = ret as crate::tls::TlsHandle;
            if attempt > 0 {
                anyos_std::println!(
                    "[http] TLS retry succeeded for {}:{} on attempt {}",
                    host,
                    port,
                    attempt + 1
                );
            } else {
                anyos_std::println!("[http] TLS handshake OK for {}:{}", host, port);
            }
            return Ok((sock, Some(tls_handle)));
        }

        anyos_std::println!(
            "[http] TLS handshake FAILED (err={}, attempt {}/{})",
            ret,
            attempt + 1,
            attempts
        );
        // Reset the global BearSSL wrapper state even on failed handshakes
        // before trying the next connection.
        net::tcp_close(sock);

        if attempt + 1 < attempts {
            anyos_std::println!(
                "[http] retrying TLS connection to {}:{} after handshake failure",
                host,
                port
            );
            anyos_std::process::sleep(20);
        }
    }

    anyos_std::println!(
        "[http] giving up TLS connection to {}:{} after {} attempt(s)",
        host,
        port,
        attempts
    );
    Err(FetchError::TlsHandshakeFailed)
}

fn close_tls_handle(tls_handle: Option<crate::tls::TlsHandle>) {
    if let Some(handle) = tls_handle {
        crate::tls::close(handle);
    }
}

fn close_conn(sock: u32, tls_handle: Option<crate::tls::TlsHandle>) {
    close_tls_handle(tls_handle);
    net::tcp_close(sock);
}

fn send_data(sock: u32, tls_handle: Option<crate::tls::TlsHandle>, data: &[u8]) -> bool {
    if let Some(handle) = tls_handle {
        crate::tls::send(handle, data) >= 0
    } else {
        net::tcp_send(sock, data) != u32::MAX
    }
}

fn recv_some(sock: u32, tls_handle: Option<crate::tls::TlsHandle>, buf: &mut [u8]) -> usize {
    if let Some(handle) = tls_handle {
        let n = crate::tls::recv(handle, buf);
        if n <= 0 {
            0
        } else {
            n as usize
        }
    } else {
        let start = anyos_std::sys::uptime_ms();
        loop {
            let avail = net::tcp_recv_available(sock);
            match avail {
                u32::MAX => return 0,
                0xFFFF_FFFE => return 0,
                n if n > 0 => {
                    let read_len = (n as usize).min(buf.len());
                    let n = net::tcp_recv(sock, &mut buf[..read_len]);
                    if n == 0 || n == u32::MAX {
                        return 0;
                    }
                    return n as usize;
                }
                _ => {}
            }
            if anyos_std::sys::uptime_ms().wrapping_sub(start) >= HEADER_RECV_TIMEOUT_MS {
                return 0;
            }
            anyos_std::process::sleep(RECV_POLL_MS);
        }
    }
}

/// Check if the response says "Connection: close".
fn response_says_close(headers: &str) -> bool {
    if let Some(val) = find_header_value(headers, "connection") {
        let lower = val.to_ascii_lowercase();
        lower.contains("close")
    } else {
        false
    }
}

/// Decompress body based on Content-Encoding header.
///
/// When decompression fails, checks whether the raw bytes look like valid
/// text before returning them.  Returning compressed binary as HTML causes
/// the HTML parser to produce garbage DOM structures that overflow the
/// layout engine's stack.
fn decompress_body(raw: Vec<u8>, content_encoding: &Option<String>) -> Vec<u8> {
    if let Some(ref enc) = content_encoding {
        let enc_lower = enc.to_ascii_lowercase();
        if enc_lower.contains("gzip") {
            match deflate::decompress_gzip(&raw) {
                Some(decoded) => return decoded,
                None => {
                    anyos_std::println!(
                        "[http] gzip decompression FAILED ({}B, header: {:02X?})",
                        raw.len(),
                        &raw[..raw.len().min(10)]
                    );
                    if !looks_like_text(&raw) {
                        anyos_std::println!("[http] raw data is binary, returning empty body");
                        return Vec::new();
                    }
                }
            }
        } else if enc_lower.contains("deflate") {
            if let Some(decoded) =
                deflate::decompress_zlib(&raw).or_else(|| deflate::decompress_deflate(&raw))
            {
                return decoded;
            }
            anyos_std::println!("[http] deflate decompression FAILED ({}B)", raw.len());
            if !looks_like_text(&raw) {
                anyos_std::println!("[http] raw data is binary, returning empty body");
                return Vec::new();
            }
        }
    }
    raw
}

/// Heuristic: check if the first bytes look like valid text (ASCII/UTF-8)
/// rather than compressed binary data.
fn looks_like_text(data: &[u8]) -> bool {
    if data.is_empty() {
        return true;
    }
    let check_len = data.len().min(256);
    let mut non_text_count = 0u32;
    for &b in &data[..check_len] {
        // Allow printable ASCII, whitespace (tab, newline, carriage return)
        if b < 0x20 && b != b'\t' && b != b'\n' && b != b'\r' {
            non_text_count += 1;
        }
    }
    // If more than 10% of bytes are non-text control characters, it's binary.
    non_text_count * 10 < check_len as u32
}

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
    if relative.trim().is_empty() {
        let mut resolved = clone_url(base);
        if let Some(hash) = resolved.path.find('#') {
            resolved.path.truncate(hash);
        }
        return resolved;
    }

    if starts_with_ignore_case(relative, "http://") || starts_with_ignore_case(relative, "https://")
    {
        match parse_url(relative) {
            Ok(u) => return u,
            Err(_) => return clone_url(base),
        }
    }

    // Protocol-relative URL: //host/path → inherit scheme from base
    if relative.starts_with("//") {
        let rest = &relative[2..];
        let (host, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let port = if base.scheme == "https" { 443 } else { 80 };
        return Url {
            scheme: base.scheme.clone(),
            host: String::from(host),
            port,
            path: String::from(path),
        };
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

pub fn fetch(
    url: &Url,
    cookies: &mut CookieJar,
    pool: &mut ConnPool,
) -> Result<Response, FetchError> {
    fetch_with_destination(url, cookies, pool, FetchDestination::Document)
}

pub fn fetch_with_destination(
    url: &Url,
    cookies: &mut CookieJar,
    pool: &mut ConnPool,
    destination: FetchDestination,
) -> Result<Response, FetchError> {
    let mut current = clone_url(url);
    let mut timing = RequestTiming {
        start_ms: anyos_std::sys::uptime_ms(),
        ..RequestTiming::default()
    };

    for redirect_n in 0..MAX_REDIRECTS {
        timing.redirects = redirect_n as u32;
        let is_https = current.scheme == "https";
        anyos_std::println!(
            "[http] {} GET {}:{}{}",
            if is_https { "HTTPS" } else { "HTTP" },
            current.host,
            current.port,
            current.path
        );

        // 1. Get connection from pool or create fresh.
        let (mut sock, mut tls_handle, from_pool) =
            match pool.take(&current.host, current.port, is_https) {
                Some(entry) => {
                    anyos_std::println!(
                        "[http] reusing connection to {}:{}",
                        current.host,
                        current.port
                    );
                    (entry.sock, entry.tls_handle, true)
                }
                None => {
                    let (sock, tls_handle) =
                        connect_fresh(pool, &current.host, current.port, is_https, &mut timing)?;
                    (sock, tls_handle, false)
                }
            };
        timing.reused_connection = timing.reused_connection || from_pool;

        // 2. Build and send GET request.
        let request = build_request(&current, cookies, destination);
        let send_start = anyos_std::sys::uptime_ms();
        let mut send_ok = send_data(sock, tls_handle, request.as_bytes());
        timing.send_ms = timing
            .send_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(send_start));
        let mut retried_stale_pool = false;

        // Retry on stale pooled connection.
        if !send_ok && from_pool {
            close_conn(sock, tls_handle);
            let (new_sock, new_tls_handle) =
                connect_fresh(pool, &current.host, current.port, is_https, &mut timing)?;
            sock = new_sock;
            tls_handle = new_tls_handle;
            let retry_send_start = anyos_std::sys::uptime_ms();
            send_ok = send_data(sock, tls_handle, request.as_bytes());
            timing.send_ms = timing
                .send_ms
                .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(retry_send_start));
        }
        if !send_ok {
            anyos_std::println!("[http] send failed");
            close_conn(sock, tls_handle);
            return Err(FetchError::SendFailure);
        }

        // 3. Receive headers.
        let mut response_buf: Vec<u8> = Vec::new();
        let mut recv_buf = [0u8; RECV_BUF_SIZE];
        let header_end;
        let header_wait_start = anyos_std::sys::uptime_ms();

        loop {
            let n = recv_some(sock, tls_handle, &mut recv_buf);
            if n == 0 {
                if from_pool && !retried_stale_pool && response_buf.is_empty() {
                    anyos_std::println!(
                        "[http] pooled connection stale while waiting for headers; reconnecting to {}:{}",
                        current.host,
                        current.port
                    );
                    close_conn(sock, tls_handle);
                    let (new_sock, new_tls_handle) =
                        connect_fresh(pool, &current.host, current.port, is_https, &mut timing)?;
                    sock = new_sock;
                    tls_handle = new_tls_handle;
                    retried_stale_pool = true;
                    let retry_send_start = anyos_std::sys::uptime_ms();
                    if !send_data(sock, tls_handle, request.as_bytes()) {
                        close_conn(sock, tls_handle);
                        return Err(FetchError::SendFailure);
                    }
                    timing.send_ms = timing
                        .send_ms
                        .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(retry_send_start));
                    continue;
                }
                anyos_std::println!("[http] recv failed (buf={}B)", response_buf.len());
                close_conn(sock, tls_handle);
                return Err(FetchError::NoResponse);
            }
            response_buf.extend_from_slice(&recv_buf[..n]);

            if let Some(end) = find_header_end(&response_buf) {
                header_end = end;
                break;
            }
            if response_buf.len() > MAX_HEADER_SIZE {
                anyos_std::println!("[http] headers too large ({}B)", response_buf.len());
                close_conn(sock, tls_handle);
                return Err(FetchError::NoResponse);
            }
        }
        timing.wait_ms = timing
            .wait_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(header_wait_start));

        // 4. Parse status + headers.
        let header_str = core::str::from_utf8(&response_buf[..header_end]).unwrap_or("");
        let (status, _reason) = parse_status_line(header_str);
        let headers = String::from(header_str);
        anyos_std::println!("[http] HTTP {} {}", status, _reason);

        // Store cookies.
        cookies.store_from_headers(header_str, &current.host, &current.path);

        // 5. Handle redirects — close connection, don't pool.
        if is_redirect(status) {
            close_conn(sock, tls_handle);
            if let Some(location) = find_header_value(header_str, "location") {
                current = resolve_url(&current, location);
                continue;
            }
            timing.fetch_done_ms = anyos_std::sys::uptime_ms();
            return Ok(Response {
                status,
                headers,
                body: Vec::new(),
                final_url: Some(clone_url(&current)),
                timing,
            });
        }

        // 6. Read body (chunked or content-length or until close).
        let transfer_encoding =
            find_header_value(header_str, "transfer-encoding").map(|v| String::from(v));
        let is_chunked = transfer_encoding
            .as_deref()
            .map(|v| v.contains("chunked"))
            .unwrap_or(false);
        let content_length = parse_content_length(header_str);
        let content_encoding =
            find_header_value(header_str, "content-encoding").map(|v| String::from(v));

        let mut trailing = Vec::new();
        if header_end < response_buf.len() {
            trailing.extend_from_slice(&response_buf[header_end..]);
        }

        if should_log_body_details(&current) {
            anyos_std::println!(
                "[http] body read start: path={} mode={} content-length={} transfer-encoding={} content-encoding={} trailing={}B",
                current.path,
                if is_chunked {
                    "chunked"
                } else if let Some(cl) = content_length {
                    if cl > 0 { "content-length" } else { "empty" }
                } else {
                    "until-close"
                },
                content_length.unwrap_or(0),
                transfer_encoding.as_deref().unwrap_or(""),
                content_encoding.as_deref().unwrap_or(""),
                trailing.len()
            );
        }

        let body_start = anyos_std::sys::uptime_ms();
        let raw_body = if is_chunked {
            if let Some(handle) = tls_handle {
                read_chunked_body_tls(handle, &trailing)
            } else {
                read_chunked_body(sock, &trailing)
            }
        } else if let Some(handle) = tls_handle {
            read_body_tls(handle, &trailing, content_length)
        } else {
            read_body(sock, &trailing, content_length)
        };
        timing.body_ms = timing
            .body_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(body_start));

        if should_log_body_details(&current) {
            anyos_std::println!(
                "[http] body read done: path={} raw={}B",
                current.path,
                raw_body.len()
            );
        }

        // 7. Pool connection if reusable, otherwise close.
        let reusable = (content_length.is_some() || is_chunked) && !response_says_close(header_str);
        if reusable {
            pool.put(
                current.host.clone(),
                current.port,
                sock,
                is_https,
                tls_handle,
            );
        } else {
            close_conn(sock, tls_handle);
        }

        // 8. Decompress if content-encoded.
        let decode_start = anyos_std::sys::uptime_ms();
        let body = decompress_body(raw_body, &content_encoding);
        timing.decode_ms = timing
            .decode_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(decode_start));
        timing.fetch_done_ms = anyos_std::sys::uptime_ms();

        return Ok(Response {
            status,
            headers,
            body,
            final_url: Some(clone_url(&current)),
            timing,
        });
    }

    anyos_std::println!("[http] too many redirects");
    Err(FetchError::TooManyRedirects)
}

/// Fetch a URL using POST with a form-urlencoded body.
pub fn fetch_post(
    url: &Url,
    body: &str,
    cookies: &mut CookieJar,
    pool: &mut ConnPool,
) -> Result<Response, FetchError> {
    let mut current = clone_url(url);
    let mut timing = RequestTiming {
        start_ms: anyos_std::sys::uptime_ms(),
        ..RequestTiming::default()
    };

    for redirect_n in 0..MAX_REDIRECTS {
        timing.redirects = redirect_n as u32;
        let is_https = current.scheme == "https";
        anyos_std::println!(
            "[http] {} POST {}:{}{}",
            if is_https { "HTTPS" } else { "HTTP" },
            current.host,
            current.port,
            current.path
        );

        // 1. Get connection from pool or create fresh.
        let (mut sock, mut tls_handle, from_pool) =
            match pool.take(&current.host, current.port, is_https) {
                Some(entry) => {
                    anyos_std::println!(
                        "[http] reusing connection to {}:{}",
                        current.host,
                        current.port
                    );
                    (entry.sock, entry.tls_handle, true)
                }
                None => {
                    let (sock, tls_handle) =
                        connect_fresh(pool, &current.host, current.port, is_https, &mut timing)?;
                    (sock, tls_handle, false)
                }
            };
        timing.reused_connection = timing.reused_connection || from_pool;

        // Use POST on first request, but follow redirects as GET.
        let request = if redirect_n == 0 {
            build_post_request(&current, body, cookies)
        } else {
            build_request(&current, cookies, FetchDestination::Document)
        };

        let send_start = anyos_std::sys::uptime_ms();
        let mut send_ok = send_data(sock, tls_handle, request.as_bytes());
        timing.send_ms = timing
            .send_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(send_start));

        // Retry on stale pooled connection.
        if !send_ok && from_pool {
            close_conn(sock, tls_handle);
            let (new_sock, new_tls_handle) =
                connect_fresh(pool, &current.host, current.port, is_https, &mut timing)?;
            sock = new_sock;
            tls_handle = new_tls_handle;
            let retry_send_start = anyos_std::sys::uptime_ms();
            send_ok = send_data(sock, tls_handle, request.as_bytes());
            timing.send_ms = timing
                .send_ms
                .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(retry_send_start));
        }
        if !send_ok {
            close_conn(sock, tls_handle);
            return Err(FetchError::SendFailure);
        }

        let mut response_buf: Vec<u8> = Vec::new();
        let mut recv_buf = [0u8; RECV_BUF_SIZE];
        let header_end;
        let header_wait_start = anyos_std::sys::uptime_ms();

        loop {
            let n = recv_some(sock, tls_handle, &mut recv_buf);
            if n == 0 {
                close_conn(sock, tls_handle);
                return Err(FetchError::NoResponse);
            }
            response_buf.extend_from_slice(&recv_buf[..n]);
            if let Some(end) = find_header_end(&response_buf) {
                header_end = end;
                break;
            }
            if response_buf.len() > MAX_HEADER_SIZE {
                close_conn(sock, tls_handle);
                return Err(FetchError::NoResponse);
            }
        }
        timing.wait_ms = timing
            .wait_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(header_wait_start));

        let header_str = core::str::from_utf8(&response_buf[..header_end]).unwrap_or("");
        let (status, _reason) = parse_status_line(header_str);
        let headers = String::from(header_str);
        anyos_std::println!("[http] HTTP {} {}", status, _reason);

        cookies.store_from_headers(header_str, &current.host, &current.path);

        if is_redirect(status) {
            close_conn(sock, tls_handle);
            if let Some(location) = find_header_value(header_str, "location") {
                current = resolve_url(&current, location);
                continue;
            }
            timing.fetch_done_ms = anyos_std::sys::uptime_ms();
            return Ok(Response {
                status,
                headers,
                body: Vec::new(),
                final_url: Some(clone_url(&current)),
                timing,
            });
        }

        let transfer_encoding =
            find_header_value(header_str, "transfer-encoding").map(|v| String::from(v));
        let is_chunked = transfer_encoding
            .as_deref()
            .map(|v| v.contains("chunked"))
            .unwrap_or(false);
        let content_length = parse_content_length(header_str);
        let content_encoding =
            find_header_value(header_str, "content-encoding").map(|v| String::from(v));

        let mut trailing = Vec::new();
        if header_end < response_buf.len() {
            trailing.extend_from_slice(&response_buf[header_end..]);
        }

        if should_log_body_details(&current) {
            anyos_std::println!(
                "[http] body read start: path={} mode={} content-length={} transfer-encoding={} content-encoding={} trailing={}B",
                current.path,
                if is_chunked {
                    "chunked"
                } else if let Some(cl) = content_length {
                    if cl > 0 { "content-length" } else { "empty" }
                } else {
                    "until-close"
                },
                content_length.unwrap_or(0),
                transfer_encoding.as_deref().unwrap_or(""),
                content_encoding.as_deref().unwrap_or(""),
                trailing.len()
            );
        }

        let body_start = anyos_std::sys::uptime_ms();
        let raw_body = if is_chunked {
            if let Some(handle) = tls_handle {
                read_chunked_body_tls(handle, &trailing)
            } else {
                read_chunked_body(sock, &trailing)
            }
        } else if let Some(handle) = tls_handle {
            read_body_tls(handle, &trailing, content_length)
        } else {
            read_body(sock, &trailing, content_length)
        };
        timing.body_ms = timing
            .body_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(body_start));

        if should_log_body_details(&current) {
            anyos_std::println!(
                "[http] body read done: path={} raw={}B",
                current.path,
                raw_body.len()
            );
        }

        // Pool connection if reusable, otherwise close.
        let reusable = (content_length.is_some() || is_chunked) && !response_says_close(header_str);
        if reusable {
            pool.put(
                current.host.clone(),
                current.port,
                sock,
                is_https,
                tls_handle,
            );
        } else {
            close_conn(sock, tls_handle);
        }

        let decode_start = anyos_std::sys::uptime_ms();
        let resp_body = decompress_body(raw_body, &content_encoding);
        timing.decode_ms = timing
            .decode_ms
            .wrapping_add(anyos_std::sys::uptime_ms().wrapping_sub(decode_start));
        timing.fetch_done_ms = anyos_std::sys::uptime_ms();
        return Ok(Response {
            status,
            headers,
            body: resp_body,
            final_url: Some(clone_url(&current)),
            timing,
        });
    }

    Err(FetchError::TooManyRedirects)
}

// ---------------------------------------------------------------------------
// Body reading
// ---------------------------------------------------------------------------

fn read_body(sock: u32, initial: &[u8], content_length: Option<u32>) -> Vec<u8> {
    // Pre-allocate full body size if Content-Length is known.
    let capacity = content_length
        .map(|cl| (cl as usize).min(32 * 1024 * 1024))
        .unwrap_or(65536);
    let mut body: Vec<u8> = Vec::with_capacity(capacity);
    body.extend_from_slice(initial);

    let mut recv_buf = [0u8; RECV_BUF_SIZE];
    loop {
        if let Some(cl) = content_length {
            if body.len() >= cl as usize {
                break;
            }
        }
        let n = net::tcp_recv(sock, &mut recv_buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        body.extend_from_slice(&recv_buf[..n as usize]);
    }
    body
}

/// Read a chunked transfer-encoded body.
/// Uses a cursor into the buffer to avoid repeated allocations.
fn read_chunked_body(sock: u32, initial: &[u8]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::with_capacity(RECV_BUF_SIZE * 4);
    buf.extend_from_slice(initial);
    let mut cursor: usize = 0; // read position in buf
    let mut body: Vec<u8> = Vec::with_capacity(65536);
    let mut recv_buf = [0u8; RECV_BUF_SIZE];

    loop {
        // Find chunk size line
        let chunk_size;
        loop {
            if let Some(crlf) = find_crlf(&buf[cursor..]) {
                let size_str = core::str::from_utf8(&buf[cursor..cursor + crlf]).unwrap_or("0");
                let hex_str = match size_str.find(';') {
                    Some(i) => &size_str[..i],
                    None => size_str,
                };
                chunk_size = parse_hex(hex_str.trim());
                cursor += crlf + 2; // skip size line + CRLF
                break;
            }
            // Need more data
            let n = net::tcp_recv(sock, &mut recv_buf);
            if n == 0 || n == u32::MAX {
                return body;
            }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        if chunk_size == 0 {
            break;
        } // final chunk

        // Read chunk_size bytes of data
        while buf.len() - cursor < chunk_size {
            let n = net::tcp_recv(sock, &mut recv_buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        let available = (buf.len() - cursor).min(chunk_size);
        body.extend_from_slice(&buf[cursor..cursor + available]);
        cursor += available;

        // Skip trailing CRLF after chunk data
        while buf.len() - cursor < 2 {
            let n = net::tcp_recv(sock, &mut recv_buf);
            if n == 0 || n == u32::MAX {
                return body;
            }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }
        if buf[cursor] == b'\r' && buf[cursor + 1] == b'\n' {
            cursor += 2;
        }

        // Compact buffer periodically to prevent unbounded growth.
        if cursor > 65536 {
            buf.drain(..cursor);
            cursor = 0;
        }
    }

    body
}

// ---------------------------------------------------------------------------
// TLS body reading (uses crate::tls::recv instead of tcp_recv)
// ---------------------------------------------------------------------------

fn read_body_tls(
    tls_handle: crate::tls::TlsHandle,
    initial: &[u8],
    content_length: Option<u32>,
) -> Vec<u8> {
    let capacity = content_length
        .map(|cl| (cl as usize).min(32 * 1024 * 1024))
        .unwrap_or(65536);
    let mut body: Vec<u8> = Vec::with_capacity(capacity);
    body.extend_from_slice(initial);

    let mut recv_buf = [0u8; RECV_BUF_SIZE];
    let mut zero_count: u32 = 0;
    loop {
        if let Some(cl) = content_length {
            if body.len() >= cl as usize {
                break;
            }
        }
        let n = crate::tls::recv(tls_handle, &mut recv_buf);
        if n > 0 {
            body.extend_from_slice(&recv_buf[..n as usize]);
            zero_count = 0;
        } else {
            // n <= 0: TLS layer returned error or EOF.  If we know the
            // expected Content-Length and haven't reached it yet, retry
            // briefly — the BearSSL low_read may have given up on a
            // transient TCP timeout.
            if let Some(cl) = content_length {
                if body.len() < cl as usize && zero_count < 3 {
                    zero_count += 1;
                    anyos_std::process::sleep(50);
                    continue;
                }
            }
            break;
        }
    }
    body
}

fn read_chunked_body_tls(tls_handle: crate::tls::TlsHandle, initial: &[u8]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::with_capacity(RECV_BUF_SIZE * 4);
    buf.extend_from_slice(initial);
    let mut cursor: usize = 0;
    let mut body: Vec<u8> = Vec::with_capacity(65536);
    let mut recv_buf = [0u8; RECV_BUF_SIZE];

    loop {
        let chunk_size;
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
            let n = crate::tls::recv(tls_handle, &mut recv_buf);
            if n <= 0 {
                return body;
            }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        if chunk_size == 0 {
            break;
        }

        while buf.len() - cursor < chunk_size {
            let n = crate::tls::recv(tls_handle, &mut recv_buf);
            if n <= 0 {
                break;
            }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        let available = (buf.len() - cursor).min(chunk_size);
        body.extend_from_slice(&buf[cursor..cursor + available]);
        cursor += available;

        // Skip trailing CRLF
        while buf.len() - cursor < 2 {
            let n = crate::tls::recv(tls_handle, &mut recv_buf);
            if n <= 0 {
                return body;
            }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }
        if buf[cursor] == b'\r' && buf[cursor + 1] == b'\n' {
            cursor += 2;
        }

        if cursor > 65536 {
            buf.drain(..cursor);
            cursor = 0;
        }
    }

    body
}

fn find_crlf(data: &[u8]) -> Option<usize> {
    if data.len() < 2 {
        return None;
    }
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
    if data.len() < 4 {
        return None;
    }
    for i in 0..data.len() - 3 {
        if data[i] == b'\r' && data[i + 1] == b'\n' && data[i + 2] == b'\r' && data[i + 3] == b'\n'
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

fn build_request(url: &Url, cookies: &CookieJar, destination: FetchDestination) -> String {
    build_request_with_method(url, "GET", None, cookies, destination)
}

fn build_post_request(url: &Url, body: &str, cookies: &CookieJar) -> String {
    build_request_with_method(url, "POST", Some(body), cookies, FetchDestination::Document)
}

fn build_request_with_method(
    url: &Url,
    method: &str,
    body: Option<&str>,
    cookies: &CookieJar,
    destination: FetchDestination,
) -> String {
    let mut req = String::new();
    req.push_str(method);
    req.push(' ');
    req.push_str(&url.path);
    req.push_str(" HTTP/1.1\r\nHost: ");
    req.push_str(&url.host);
    if (url.scheme == "http" && url.port != 80) || (url.scheme == "https" && url.port != 443) {
        req.push(':');
        push_u32(&mut req, url.port as u32);
    }
    if url.host.eq_ignore_ascii_case("fonts.googleapis.com") {
        // Google Fonts serves WOFF2 unicode-range shards to modern browser UAs.
        // Our font stack handles TrueType reliably today, so ask for the
        // legacy CSS response until WOFF2 is fully covered.
        req.push_str("\r\nUser-Agent: curl/8.0");
    } else {
        req.push_str("\r\nUser-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124 Safari/537.36");
    }
    req.push_str("\r\nAccept: ");
    req.push_str(destination.accept_header());
    req.push_str("\r\nAccept-Language: de-DE,de;q=0.9,en-US;q=0.8,en;q=0.7");
    req.push_str("\r\nAccept-Encoding: gzip, deflate");
    if destination == FetchDestination::Document {
        req.push_str("\r\nUpgrade-Insecure-Requests: 1");
    }
    req.push_str("\r\nSec-Fetch-Dest: ");
    req.push_str(destination.sec_fetch_dest());
    req.push_str("\r\nSec-Fetch-Mode: ");
    req.push_str(destination.sec_fetch_mode());
    req.push_str("\r\nSec-Fetch-Site: none");
    if destination == FetchDestination::Document {
        req.push_str("\r\nSec-Fetch-User: ?1");
    }
    req.push_str("\r\nConnection: keep-alive");

    if let Some(body) = body {
        req.push_str("\r\nContent-Type: application/x-www-form-urlencoded");
        req.push_str("\r\nContent-Length: ");
        push_u32(&mut req, body.len() as u32);
    }

    // Append cookies
    let is_secure = url.scheme == "https";
    if let Some(cookie_val) = cookies.cookie_header(&url.host, &url.path, is_secure) {
        req.push_str("\r\nCookie: ");
        req.push_str(&cookie_val);
    }

    req.push_str("\r\n\r\n");

    if let Some(body) = body {
        req.push_str(body);
    }

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
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                val = val * 10 + (b - b'0') as u32;
                if val > 65535 {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(val as u16)
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val: u32 = 0;
    if s.is_empty() {
        return None;
    }
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
                if num > 255 {
                    return None;
                }
                has_digit = true;
            }
            b'.' => {
                if !has_digit || idx >= 3 {
                    return None;
                }
                parts[idx] = num as u8;
                idx += 1;
                num = 0;
                has_digit = false;
            }
            _ => return None,
        }
    }
    if !has_digit || idx != 3 {
        return None;
    }
    parts[3] = num as u8;
    Some(parts)
}

fn ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' {
        b + 32
    } else {
        b
    }
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    if s.len() < prefix.len() {
        return false;
    }
    let sb = s.as_bytes();
    let pb = prefix.as_bytes();
    for i in 0..pb.len() {
        if ascii_lower(sb[i]) != ascii_lower(pb[i]) {
            return false;
        }
    }
    true
}

pub fn push_u32(s: &mut String, val: u32) {
    if val >= 10 {
        push_u32(s, val / 10);
    }
    s.push((b'0' + (val % 10) as u8) as char);
}
