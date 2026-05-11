#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{args, fs, net, print, println, process, sys};

mod tls;

anyos_std::entry!(main);

const HTTP_PORT: u16 = 80;
const HTTPS_PORT: u16 = 443;
const CONNECT_TIMEOUT: u32 = 10000;
const MAX_REDIRECTS: usize = 20;
const MAX_HTTP_REQUEST: usize = 2048;
const RECV_BUF_SIZE: usize = 32 * 1024;
const FILE_WRITE_CHUNK: usize = 16 * 1024;
const VERSION: &str = "1.0";

// ── URL parsing ─────────────────────────────────────────────────────────────

struct Url {
    host: String,
    port: u16,
    path: String,
    https: bool,
}

fn parse_url(url_str: &str) -> Option<Url> {
    let (rest, https) = if starts_with_ignore_case(url_str, "https://") {
        (&url_str[8..], true)
    } else if starts_with_ignore_case(url_str, "http://") {
        (&url_str[7..], false)
    } else {
        (url_str, false)
    };

    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

    let default_port = if https { HTTPS_PORT } else { HTTP_PORT };
    let (host, port) = match host_port.find(':') {
        Some(idx) => {
            let port_str = &host_port[idx + 1..];
            let port = parse_u16(port_str)?;
            (&host_port[..idx], port)
        }
        None => (host_port, default_port),
    };

    if host.is_empty() {
        return None;
    }

    Some(Url {
        host: String::from(host),
        port,
        path: String::from(path),
        https,
    })
}

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

// ── IP parsing & DNS ────────────────────────────────────────────────────────

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

// ── String helpers ──────────────────────────────────────────────────────────

fn to_ascii_lower(b: u8) -> u8 {
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
        if to_ascii_lower(sb[i]) != to_ascii_lower(pb[i]) {
            return false;
        }
    }
    true
}

fn push_u32(s: &mut String, val: u32) {
    if val >= 10 {
        push_u32(s, val / 10);
    }
    s.push((b'0' + (val % 10) as u8) as char);
}

fn push_u32_pad2(s: &mut String, val: u32) {
    if val < 10 {
        s.push('0');
    }
    push_u32(s, val);
}

// ── HTTP header parsing ─────────────────────────────────────────────────────

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

/// Returns (status_code, reason_phrase)
fn parse_status_line(header: &str) -> (u16, &str) {
    // "HTTP/1.x NNN Reason\r\n..."
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

fn find_header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
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

fn parse_content_type<'a>(headers: &'a str) -> &'a str {
    find_header_value(headers, "content-type").unwrap_or("application/octet-stream")
}

fn parse_location<'a>(headers: &'a str) -> Option<&'a str> {
    find_header_value(headers, "location")
}

fn is_chunked(headers: &str) -> bool {
    match find_header_value(headers, "transfer-encoding") {
        Some(v) => {
            // Case-insensitive substring match for "chunked".
            let bytes = v.as_bytes();
            let pat = b"chunked";
            if bytes.len() < pat.len() {
                return false;
            }
            for i in 0..=bytes.len() - pat.len() {
                let mut ok = true;
                for j in 0..pat.len() {
                    if to_ascii_lower(bytes[i + j]) != pat[j] {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    return true;
                }
            }
            false
        }
        None => false,
    }
}

fn parse_hex_u32(s: &[u8]) -> Option<u32> {
    let mut val: u32 = 0;
    let mut any = false;
    for &b in s {
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            b';' | b'\r' | b' ' | b'\t' => break, // chunk extensions / line end
            _ => return None,
        };
        val = val.checked_mul(16)?.checked_add(d as u32)?;
        any = true;
    }
    if any {
        Some(val)
    } else {
        None
    }
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

// ── Filename derivation ─────────────────────────────────────────────────────

fn derive_filename(url: &Url) -> String {
    let path = url.path.as_str();
    // Find last '/' and take everything after it
    let name = match path.rfind('/') {
        Some(i) if i + 1 < path.len() => &path[i + 1..],
        _ => "",
    };
    // Strip query string
    let name = match name.find('?') {
        Some(i) => &name[..i],
        None => name,
    };
    if name.is_empty() {
        String::from("index.html")
    } else {
        String::from(name)
    }
}

// ── Timestamp formatting ────────────────────────────────────────────────────

fn fmt_timestamp() -> String {
    let mut t = [0u8; 8];
    sys::time(&mut t);
    let year = t[0] as u32 | ((t[1] as u32) << 8);
    let month = t[2] as u32;
    let day = t[3] as u32;
    let hour = t[4] as u32;
    let min = t[5] as u32;
    let sec = t[6] as u32;
    let mut s = String::new();
    push_u32(&mut s, year);
    s.push('-');
    push_u32_pad2(&mut s, month);
    s.push('-');
    push_u32_pad2(&mut s, day);
    s.push(' ');
    push_u32_pad2(&mut s, hour);
    s.push(':');
    push_u32_pad2(&mut s, min);
    s.push(':');
    push_u32_pad2(&mut s, sec);
    s
}

// ── Human-readable size & speed ─────────────────────────────────────────────

fn fmt_size(bytes: u32, buf: &mut [u8; 16]) -> usize {
    if bytes < 1024 {
        return fmt_u32_into(bytes, buf);
    }
    // Use integer math: size * 100 / 1024 for one decimal place
    if bytes < 1024 * 1024 {
        let kb100 = (bytes as u64 * 100) / 1024;
        return fmt_fixed_point(kb100 as u32, b'K', buf);
    }
    let mb100 = (bytes as u64 * 100) / (1024 * 1024);
    fmt_fixed_point(mb100 as u32, b'M', buf)
}

/// Format value*100 as "X.YYS" where S is suffix
fn fmt_fixed_point(val100: u32, suffix: u8, buf: &mut [u8; 16]) -> usize {
    let whole = val100 / 100;
    let frac = val100 % 100;
    let mut pos = 0;
    // Write whole part
    pos += fmt_u32_at(whole, &mut buf[pos..]);
    buf[pos] = b'.';
    pos += 1;
    // Write fractional part (2 digits)
    buf[pos] = b'0' + (frac / 10) as u8;
    pos += 1;
    buf[pos] = suffix;
    pos += 1;
    pos
}

fn fmt_u32_into(val: u32, buf: &mut [u8; 16]) -> usize {
    fmt_u32_at(val, buf)
}

fn fmt_u32_at(val: u32, buf: &mut [u8]) -> usize {
    let mut tmp = [0u8; 12];
    let s = anyos_std::fmt::fmt_u32(&mut tmp, val);
    let len = s.len();
    buf[..len].copy_from_slice(s.as_bytes());
    len
}

fn fmt_speed(bps: u32, buf: &mut [u8; 16]) -> usize {
    if bps == 0 {
        let s = b"--.-KB/s";
        buf[..s.len()].copy_from_slice(s);
        return s.len();
    }
    if bps < 1024 {
        let mut pos = fmt_u32_at(bps, buf);
        let s = b"B/s";
        buf[pos..pos + s.len()].copy_from_slice(s);
        pos += s.len();
        return pos;
    }
    if bps < 1024 * 1024 {
        let kb100 = (bps as u64 * 100) / 1024;
        let mut pos = fmt_fixed_point(kb100 as u32, b'K', buf);
        let s = b"B/s";
        buf[pos..pos + s.len()].copy_from_slice(s);
        pos += s.len();
        return pos;
    }
    let mb100 = (bps as u64 * 100) / (1024 * 1024);
    let mut pos = fmt_fixed_point(mb100 as u32, b'M', buf);
    let s = b"B/s";
    buf[pos..pos + s.len()].copy_from_slice(s);
    pos += s.len();
    pos
}

// ── Progress bar ────────────────────────────────────────────────────────────

fn draw_progress(
    filename: &str,
    received: u32,
    total: Option<u32>,
    elapsed_ticks: u32,
    tick_hz: u32,
) {
    // filename     XX%[========>          ] XXX.XK  XX.XKB/s    in Xs
    let mut line = String::new();
    line.push('\r');

    // Filename (padded/truncated to 20 chars)
    let name_bytes = filename.as_bytes();
    let name_len = name_bytes.len().min(20);
    for i in 0..name_len {
        line.push(name_bytes[i] as char);
    }
    for _ in name_len..20 {
        line.push(' ');
    }
    line.push(' ');

    // Percentage + bar (if total known)
    if let Some(tot) = total {
        if tot > 0 {
            let pct = if received >= tot {
                100u32
            } else {
                (received as u64 * 100 / tot as u64) as u32
            };
            // "100%" or " 42%"
            if pct < 10 {
                line.push(' ');
                line.push(' ');
            } else if pct < 100 {
                line.push(' ');
            }
            push_u32(&mut line, pct);
            line.push('%');

            // Bar: [====================>]
            line.push('[');
            let bar_width: u32 = 20;
            let filled = if received >= tot {
                bar_width
            } else {
                (received as u64 * bar_width as u64 / tot as u64) as u32
            };
            for i in 0..bar_width {
                if i < filled {
                    line.push('=');
                } else if i == filled {
                    line.push('>');
                } else {
                    line.push(' ');
                }
            }
            line.push(']');
        }
    } else {
        // Unknown total — just show received
        line.push_str("    [");
        line.push_str("<=>                 ");
        line.push(']');
    }

    line.push(' ');

    // Size received
    let mut sbuf = [0u8; 16];
    let slen = fmt_size(received, &mut sbuf);
    for i in 0..slen {
        line.push(sbuf[i] as char);
    }

    // Speed
    line.push_str("  ");
    let bps = if elapsed_ticks > 0 {
        (received as u64 * tick_hz as u64 / elapsed_ticks as u64) as u32
    } else {
        0
    };
    let mut spd = [0u8; 16];
    let spd_len = fmt_speed(bps, &mut spd);
    for i in 0..spd_len {
        line.push(spd[i] as char);
    }

    // Elapsed time
    if elapsed_ticks > 0 && tick_hz > 0 {
        let secs = elapsed_ticks / tick_hz;
        line.push_str("    in ");
        if secs >= 60 {
            push_u32(&mut line, secs / 60);
            line.push('m');
            push_u32(&mut line, secs % 60);
        } else {
            push_u32(&mut line, secs);
        }
        line.push('s');
    }

    // Pad to clear previous line
    let pad_to = 79;
    while line.len() < pad_to {
        line.push(' ');
    }

    print!("{}", line);
}

// ── Connection abstraction (HTTP / HTTPS) ───────────────────────────────────

struct Conn {
    sock: u32,
    tls: Option<tls::TlsHandle>,
}

impl Conn {
    fn connect(ip: &[u8; 4], port: u16, host: &str, https: bool, timeout_ms: u32) -> Option<Conn> {
        let sock = net::tcp_connect(ip, port, timeout_ms);
        if sock == u32::MAX {
            return None;
        }
        if !https {
            return Some(Conn { sock, tls: None });
        }
        let h = tls::connect(sock, host);
        if h < 0 {
            net::tcp_close(sock);
            return None;
        }
        Some(Conn {
            sock,
            tls: Some(h as tls::TlsHandle),
        })
    }

    fn send(&self, data: &[u8]) -> i32 {
        match self.tls {
            Some(h) => tls::send(h, data),
            None => {
                let n = net::tcp_send(self.sock, data);
                if n == u32::MAX {
                    -1
                } else {
                    n as i32
                }
            }
        }
    }

    fn recv(&self, buf: &mut [u8]) -> i32 {
        match self.tls {
            Some(h) => tls::recv(h, buf),
            None => {
                let n = net::tcp_recv(self.sock, buf);
                if n == u32::MAX {
                    -1
                } else {
                    n as i32
                }
            }
        }
    }

    fn close(self) {
        if let Some(h) = self.tls {
            tls::close(h);
        }
        net::tcp_close(self.sock);
    }
}

fn recv_with_retry(conn: &Conn, buf: &mut [u8]) -> i32 {
    const MAX_RETRIES: u32 = 10;
    let mut retries = 0u32;
    loop {
        let n = conn.recv(buf);
        if n >= 0 {
            return n;
        }

        let avail = net::tcp_recv_available(conn.sock);
        if avail == u32::MAX || avail == u32::MAX - 1 {
            return -1;
        }
        if retries >= MAX_RETRIES {
            return -1;
        }
        retries += 1;
        process::sleep(100);
    }
}

fn write_all(fd: u32, data: &[u8]) -> bool {
    let mut written = 0usize;
    while written < data.len() {
        let end = (written + FILE_WRITE_CHUNK).min(data.len());
        let n = fs::write(fd, &data[written..end]);
        if n == u32::MAX || n == 0 {
            return false;
        }
        written += n as usize;
    }
    true
}

fn buffer_file_write(fd: u32, file_buf: &mut Vec<u8>, data: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset < data.len() {
        if file_buf.len() >= FILE_WRITE_CHUNK && !flush_file_buffer(fd, file_buf) {
            return false;
        }

        let space = FILE_WRITE_CHUNK.saturating_sub(file_buf.len());
        if space == 0 {
            continue;
        }
        let end = (offset + space).min(data.len());
        file_buf.extend_from_slice(&data[offset..end]);
        offset = end;
    }
    true
}

fn flush_file_buffer(fd: u32, file_buf: &mut Vec<u8>) -> bool {
    if file_buf.is_empty() {
        return true;
    }
    let ok = write_all(fd, file_buf);
    file_buf.clear();
    ok
}

// ── Build HTTP request ──────────────────────────────────────────────────────

struct RequestBuf {
    data: [u8; MAX_HTTP_REQUEST],
    len: usize,
}

impl RequestBuf {
    fn new() -> Self {
        Self {
            data: [0; MAX_HTTP_REQUEST],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    fn push_str(&mut self, s: &str) -> bool {
        self.push_bytes(s.as_bytes())
    }

    fn push_byte(&mut self, byte: u8) -> bool {
        if self.len == self.data.len() {
            return false;
        }
        self.data[self.len] = byte;
        self.len += 1;
        true
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> bool {
        if self.len + bytes.len() > self.data.len() {
            return false;
        }
        self.data[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        true
    }

    fn push_u32(&mut self, val: u32) -> bool {
        let mut tmp = [0u8; 12];
        let s = anyos_std::fmt::fmt_u32(&mut tmp, val);
        self.push_str(s)
    }
}

fn build_request(url: &Url, resume_offset: u32) -> Option<RequestBuf> {
    let mut req = RequestBuf::new();
    if !req.push_str("GET ")
        || !req.push_str(&url.path)
        || !req.push_str(" HTTP/1.1\r\nHost: ")
        || !req.push_str(&url.host)
    {
        return None;
    }
    let default_port = if url.https { HTTPS_PORT } else { HTTP_PORT };
    if url.port != default_port {
        if !req.push_byte(b':') || !req.push_u32(url.port as u32) {
            return None;
        }
    }
    if !req.push_str("\r\nUser-Agent: Wget/")
        || !req.push_str(VERSION)
        || !req.push_str(" (anyOS)\r\nConnection: close\r\nAccept: */*\r\n")
    {
        return None;
    }

    if resume_offset > 0 {
        if !req.push_str("Range: bytes=") || !req.push_u32(resume_offset) || !req.push_str("-\r\n")
        {
            return None;
        }
    }

    if !req.push_str("\r\n") {
        return None;
    }
    Some(req)
}

// ── Usage ───────────────────────────────────────────────────────────────────

fn print_usage() {
    println!("GNU Wget {}, a non-interactive network retriever.", VERSION);
    println!("Usage: wget [OPTION]... [URL]...");
    println!();
    println!("  -O FILE     write documents to FILE ('-' for stdout)");
    println!("  -q          quiet (no output)");
    println!("  -c          resume getting a partially-downloaded file");
    println!("  -v          be verbose (default)");
    println!("  -h          print this help");
    println!("  -V          display the version of Wget and exit");
}

fn print_version() {
    println!("GNU Wget {} -- anyOS build", VERSION);
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() -> u32 {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf).trim();
    let parsed = args::parse(raw, b"O");

    // Handle --help and --version as positional args
    for i in 0..parsed.pos_count {
        match parsed.positional[i] {
            "--help" => {
                print_usage();
                return 0;
            }
            "--version" => {
                print_version();
                return 0;
            }
            _ => {}
        }
    }

    if parsed.has(b'h') {
        print_usage();
        return 0;
    }
    if parsed.has(b'V') {
        print_version();
        return 0;
    }

    let quiet = parsed.has(b'q');
    let resume = parsed.has(b'c');
    let output_flag = parsed.opt(b'O');

    // Find the URL (first positional that looks like a URL or hostname)
    let url_str = parsed.first_or("");
    if url_str.is_empty() {
        println!("wget: missing URL");
        println!("Usage: wget [OPTION]... [URL]...");
        return 1;
    }

    // Parse the initial URL
    let mut current_url = match parse_url(url_str) {
        Some(u) => u,
        None => {
            println!("wget: invalid URL '{}'", url_str);
            return 1;
        }
    };

    // Determine output filename early (for -c resume)
    let out_filename = match output_flag {
        Some(name) => String::from(name),
        None => derive_filename(&current_url),
    };
    let to_stdout = out_filename == "-";

    // Check existing file size for resume
    let mut existing_size: u32 = 0;
    if resume && !to_stdout {
        let mut stat_buf = [0u32; 7];
        if fs::stat(&out_filename, &mut stat_buf) == 0 {
            existing_size = stat_buf[1];
        }
    }

    let tick_hz = sys::tick_hz();

    // ── Redirect loop ───────────────────────────────────────────────────
    for _redir in 0..MAX_REDIRECTS {
        // Timestamp header
        if !quiet {
            let ts = fmt_timestamp();
            let scheme = if current_url.https { "https" } else { "http" };
            let default_port = if current_url.https {
                HTTPS_PORT
            } else {
                HTTP_PORT
            };
            println!(
                "--{}--  {}://{}{}",
                ts,
                scheme,
                if current_url.port != default_port {
                    let mut h = current_url.host.clone();
                    h.push(':');
                    push_u32(&mut h, current_url.port as u32);
                    h
                } else {
                    current_url.host.clone()
                },
                current_url.path
            );
        }

        // DNS resolution
        if !quiet {
            print!("Resolving {} ({})... ", current_url.host, current_url.host);
        }
        let t_dns = sys::uptime_ms();
        let ip = match resolve_host(&current_url.host) {
            Some(ip) => {
                if !quiet {
                    let dt = sys::uptime_ms().wrapping_sub(t_dns);
                    println!("{}.{}.{}.{} [{}ms]", ip[0], ip[1], ip[2], ip[3], dt);
                }
                ip
            }
            None => {
                if !quiet {
                    println!("failed: Name or service not known.");
                }
                println!(
                    "wget: unable to resolve host address '{}'",
                    current_url.host
                );
                return 4;
            }
        };

        // Connect (TCP + optional TLS handshake)
        if !quiet {
            print!(
                "Connecting to {} ({})|{}.{}.{}.{}|:{}... ",
                current_url.host, current_url.host, ip[0], ip[1], ip[2], ip[3], current_url.port
            );
        }
        let t_conn = sys::uptime_ms();
        let conn = match Conn::connect(
            &ip,
            current_url.port,
            &current_url.host,
            current_url.https,
            CONNECT_TIMEOUT,
        ) {
            Some(c) => c,
            None => {
                if !quiet {
                    println!("failed: Connection refused.");
                }
                println!(
                    "wget: unable to connect to {}:{}",
                    current_url.host, current_url.port
                );
                return 4;
            }
        };
        if !quiet {
            let dt = sys::uptime_ms().wrapping_sub(t_conn);
            if current_url.https {
                println!("connected. TLS established. [{}ms]", dt);
            } else {
                println!("connected. [{}ms]", dt);
            }
        }

        // Send request
        let request = match build_request(&current_url, existing_size) {
            Some(request) => request,
            None => {
                if !quiet {
                    println!("failed.");
                }
                println!("wget: HTTP request too large");
                conn.close();
                return 1;
            }
        };
        if !quiet {
            print!("HTTP request sent, awaiting response... ");
        }
        let t_req = sys::uptime_ms();
        if conn.send(request.as_bytes()) < 0 {
            if !quiet {
                println!("failed.");
            }
            conn.close();
            return 4;
        }

        // Receive headers
        let mut response = Vec::new();
        let mut recv_buf = Vec::new();
        recv_buf.resize(RECV_BUF_SIZE, 0);
        let header_end;
        loop {
            let n = recv_with_retry(&conn, &mut recv_buf);
            if n <= 0 {
                if !quiet {
                    println!("no data received.");
                }
                conn.close();
                return 4;
            }
            response.extend_from_slice(&recv_buf[..n as usize]);
            if let Some(end) = find_header_end(&response) {
                header_end = end;
                break;
            }
            if response.len() > 16384 {
                if !quiet {
                    println!("headers too large.");
                }
                conn.close();
                return 4;
            }
        }

        let header_str = core::str::from_utf8(&response[..header_end]).unwrap_or("");
        let (status, reason) = parse_status_line(header_str);

        if !quiet {
            let dt = sys::uptime_ms().wrapping_sub(t_req);
            println!("{} {} [{}ms]", status, reason, dt);
        }

        // Handle redirect
        if is_redirect(status) {
            conn.close();
            if let Some(loc) = parse_location(header_str) {
                if !quiet {
                    println!("Location: {} [following]", loc);
                }
                if starts_with_ignore_case(loc, "http://")
                    || starts_with_ignore_case(loc, "https://")
                {
                    current_url = match parse_url(loc) {
                        Some(u) => u,
                        None => {
                            println!("wget: invalid redirect URL");
                            return 1;
                        }
                    };
                } else {
                    current_url.path = String::from(loc);
                }
                continue;
            } else {
                println!("wget: redirect with no Location header");
                return 8;
            }
        }

        // Handle errors
        if status >= 400 {
            let ts = fmt_timestamp();
            println!("{} ERROR {}: {}.", ts, status, reason);
            conn.close();
            return 8;
        }

        // Handle 206 Partial Content for resume
        let resuming = status == 206 && existing_size > 0;

        // Parse response headers
        let content_length = parse_content_length(header_str);
        let content_type = parse_content_type(header_str);

        // If not resuming or server returned 200 (ignoring Range), start fresh
        if resume && !resuming && existing_size > 0 {
            existing_size = 0; // server doesn't support resume, redownload
        }

        // Total expected bytes (content_length may be body-only for 206)
        let total_size = if resuming {
            content_length.map(|cl| cl + existing_size)
        } else {
            content_length
        };

        // Print Length and Saving
        if !quiet {
            if let Some(tot) = total_size {
                let mut sbuf = [0u8; 16];
                let slen = fmt_size(tot, &mut sbuf);
                let size_str = core::str::from_utf8(&sbuf[..slen]).unwrap_or("?");
                print!("Length: {} ({})", tot, size_str);
            } else {
                print!("Length: unspecified");
            }
            println!(" [{}]", content_type);

            if to_stdout {
                println!("Saving to: 'STDOUT'");
            } else {
                println!("Saving to: '{}'", out_filename);
            }
            println!();
        }

        // Open output file
        let fd = if to_stdout {
            1 // stdout fd
        } else if resuming {
            let f = fs::open(&out_filename, fs::O_WRITE | fs::O_APPEND);
            if f == u32::MAX {
                println!("wget: cannot open '{}' for appending", out_filename);
                conn.close();
                return 3;
            }
            f
        } else {
            let f = fs::open(&out_filename, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
            if f == u32::MAX {
                println!("wget: cannot open '{}' for writing", out_filename);
                conn.close();
                return 3;
            }
            f
        };

        let chunked = is_chunked(header_str);

        // Bytes already read past the headers (start of the body stream).
        let mut carry: Vec<u8> = response[header_end..].to_vec();
        let mut file_buf = Vec::new();
        file_buf.reserve(FILE_WRITE_CHUNK);
        let mut received: u32 = 0;
        let start_ticks = sys::uptime();
        let mut last_progress_bytes: u32 = 0;

        // Helper: ensure carry has at least `n` bytes by recv()'ing more.
        // Returns false on EOF/error before reaching n.
        let pull_into_carry =
            |carry: &mut Vec<u8>, conn: &Conn, recv_buf: &mut [u8], n: usize| -> bool {
                while carry.len() < n {
                    let r = recv_with_retry(conn, recv_buf);
                    if r <= 0 {
                        return false;
                    }
                    carry.extend_from_slice(&recv_buf[..r as usize]);
                }
                true
            };

        // Show initial progress
        if !quiet && !to_stdout {
            draw_progress(&out_filename, existing_size, total_size, 0, tick_hz);
        }

        let mut transfer_ok = true;
        if chunked {
            // ── Chunked Transfer-Encoding ─────────────────────────────────
            'chunks: loop {
                // Read chunk size line, terminated by CRLF.
                let mut line_end;
                loop {
                    line_end = None;
                    if carry.len() >= 2 {
                        for i in 0..carry.len() - 1 {
                            if carry[i] == b'\r' && carry[i + 1] == b'\n' {
                                line_end = Some(i);
                                break;
                            }
                        }
                    }
                    if line_end.is_some() {
                        break;
                    }
                    let r = recv_with_retry(&conn, &mut recv_buf);
                    if r <= 0 {
                        transfer_ok = false;
                        break 'chunks;
                    }
                    carry.extend_from_slice(&recv_buf[..r as usize]);
                    if carry.len() > 65536 {
                        transfer_ok = false;
                        break 'chunks;
                    } // runaway guard
                }
                let line_end = line_end.unwrap();
                let size = match parse_hex_u32(&carry[..line_end]) {
                    Some(s) => s,
                    None => {
                        transfer_ok = false;
                        break;
                    }
                };
                carry.drain(..line_end + 2); // consume size line + CRLF

                if size == 0 {
                    // Final chunk — done. (Trailers/final CRLF ignored.)
                    break;
                }

                // Read `size` bytes of chunk data.
                let mut to_read = size as usize;
                while to_read > 0 {
                    if carry.is_empty() {
                        let r = recv_with_retry(&conn, &mut recv_buf);
                        if r <= 0 {
                            transfer_ok = false;
                            break 'chunks;
                        }
                        carry.extend_from_slice(&recv_buf[..r as usize]);
                    }
                    let take = carry.len().min(to_read);
                    if !buffer_file_write(fd, &mut file_buf, &carry[..take]) {
                        transfer_ok = false;
                        break 'chunks;
                    }
                    received += take as u32;
                    carry.drain(..take);
                    to_read -= take;

                    if !quiet && !to_stdout && received - last_progress_bytes >= 2048 {
                        last_progress_bytes = received;
                        let elapsed = sys::uptime().wrapping_sub(start_ticks);
                        let display_received = received + existing_size;
                        draw_progress(
                            &out_filename,
                            display_received,
                            total_size,
                            elapsed,
                            tick_hz,
                        );
                    }
                }

                // Consume the trailing CRLF after the chunk data.
                if !pull_into_carry(&mut carry, &conn, &mut recv_buf, 2) {
                    transfer_ok = false;
                    break;
                }
                if carry[0] != b'\r' || carry[1] != b'\n' {
                    transfer_ok = false;
                    break;
                }
                carry.drain(..2);
            }
        } else {
            // ── Plain body: Content-Length or read-until-close ───────────
            // Flush whatever is already in carry first.
            if !carry.is_empty() {
                let write_len = if let Some(cl) = content_length {
                    carry.len().min(cl as usize)
                } else {
                    carry.len()
                };
                if write_len > 0 && !buffer_file_write(fd, &mut file_buf, &carry[..write_len]) {
                    transfer_ok = false;
                }
                received += write_len as u32;
                carry.clear();
            }

            while transfer_ok {
                if let Some(cl) = content_length {
                    if received >= cl {
                        break;
                    }
                }
                let n = recv_with_retry(&conn, &mut recv_buf);
                if n <= 0 {
                    break;
                }
                let write_len = if let Some(cl) = content_length {
                    let remaining = cl.saturating_sub(received) as usize;
                    (n as usize).min(remaining)
                } else {
                    n as usize
                };
                if write_len > 0 && !buffer_file_write(fd, &mut file_buf, &recv_buf[..write_len]) {
                    transfer_ok = false;
                    break;
                }
                received += write_len as u32;

                if !quiet && !to_stdout && received - last_progress_bytes >= 2048 {
                    last_progress_bytes = received;
                    let elapsed = sys::uptime().wrapping_sub(start_ticks);
                    let display_received = received + existing_size;
                    draw_progress(
                        &out_filename,
                        display_received,
                        total_size,
                        elapsed,
                        tick_hz,
                    );
                }
            }

            if let Some(cl) = content_length {
                if received < cl {
                    transfer_ok = false;
                }
            }
        }

        if transfer_ok && !flush_file_buffer(fd, &mut file_buf) {
            transfer_ok = false;
        }

        // Final progress
        let elapsed = sys::uptime().wrapping_sub(start_ticks);
        if !quiet && !to_stdout {
            let display_received = received + existing_size;
            draw_progress(
                &out_filename,
                display_received,
                total_size,
                elapsed,
                tick_hz,
            );
            println!();
            println!();
        }

        // Close file and socket
        if !to_stdout {
            if !fs::fsync(fd as i32) {
                transfer_ok = false;
            }
            fs::close(fd);
        }
        conn.close();

        if !transfer_ok {
            println!(
                "wget: download incomplete for '{}' ({}/{})",
                out_filename,
                received + existing_size,
                total_size.unwrap_or(0)
            );
            return 4;
        }
        if !to_stdout {
            if let Some(tot) = total_size {
                let mut stat_buf = [0u32; 7];
                if fs::stat(&out_filename, &mut stat_buf) != 0 || stat_buf[1] != tot {
                    println!(
                        "wget: saved size mismatch for '{}' (stat={}, expected={})",
                        out_filename, stat_buf[1], tot
                    );
                    return 4;
                }
            }
        }

        // Summary line
        if !quiet {
            let ts = fmt_timestamp();
            let total = received + existing_size;
            let bps = if elapsed > 0 && tick_hz > 0 {
                (total as u64 * tick_hz as u64 / elapsed as u64) as u32
            } else {
                0
            };
            let mut spd = [0u8; 16];
            let spd_len = fmt_speed(bps, &mut spd);
            let speed_str = core::str::from_utf8(&spd[..spd_len]).unwrap_or("?");

            if to_stdout {
                // No summary for stdout
            } else if let Some(tot) = total_size {
                println!(
                    "{} ({}) - '{}' saved [{}/{}]",
                    ts, speed_str, out_filename, total, tot
                );
            } else {
                println!(
                    "{} ({}) - '{}' saved [{}]",
                    ts, speed_str, out_filename, total
                );
            }
        }

        return 0; // Done!
    }

    // Exceeded max redirects
    println!("wget: maximum {} redirects exceeded", MAX_REDIRECTS);
    8
}
