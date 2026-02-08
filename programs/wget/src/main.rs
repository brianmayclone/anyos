#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{println, net, fs};

anyos_std::entry!(main);

const HTTP_PORT: u16 = 80;
const CONNECT_TIMEOUT: u32 = 10000; // 10s

struct Url {
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url_str: &str) -> Option<Url> {
    // Strip http:// prefix
    let rest = if url_str.starts_with("http://") {
        &url_str[7..]
    } else {
        url_str
    };

    // Split host+port from path
    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

    // Split host from port
    let (host, port) = match host_port.find(':') {
        Some(idx) => {
            let port_str = &host_port[idx + 1..];
            let port = parse_u16(port_str)?;
            (&host_port[..idx], port)
        }
        None => (host_port, HTTP_PORT),
    };

    if host.is_empty() {
        return None;
    }

    Some(Url {
        host: String::from(host),
        port,
        path: String::from(path),
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
    // Try as IP address first
    if let Some(ip) = parse_ip(host) {
        return Some(ip);
    }
    // DNS resolve
    let mut resolved = [0u8; 4];
    if net::dns(host, &mut resolved) == 0 {
        Some(resolved)
    } else {
        None
    }
}

/// Find \r\n\r\n (end of headers) in buffer. Returns index past the separator.
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

/// Parse HTTP status code from response line like "HTTP/1.0 200 OK\r\n"
fn parse_status_code(header: &str) -> Option<u16> {
    // Find first space, then parse 3 digits
    let rest = header.split(' ').nth(1)?;
    if rest.len() < 3 {
        return None;
    }
    parse_u16(&rest[..3])
}

/// Parse Content-Length from headers (case-insensitive)
fn parse_content_length(headers: &str) -> Option<u32> {
    for line in headers.split('\n') {
        let line = line.trim();
        // Case-insensitive check for "content-length:"
        if line.len() > 16 {
            let prefix = &line[..16];
            if eq_ignore_ascii_case(prefix, "content-length: ")
                || eq_ignore_ascii_case(prefix, "content-length:")
            {
                let val_str = if line.as_bytes()[15] == b' ' {
                    line[16..].trim()
                } else {
                    line[15..].trim()
                };
                let mut val: u32 = 0;
                for b in val_str.bytes() {
                    match b {
                        b'0'..=b'9' => val = val * 10 + (b - b'0') as u32,
                        b'\r' => break,
                        _ => break,
                    }
                }
                return Some(val);
            }
        }
    }
    None
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (ca, cb) in a.bytes().zip(b.bytes()) {
        if to_ascii_lower(ca) != to_ascii_lower(cb) {
            return false;
        }
    }
    true
}

fn to_ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' {
        b + 32
    } else {
        b
    }
}

fn write_u32_str(s: &mut String, val: u32) {
    if val >= 10 {
        write_u32_str(s, val / 10);
    }
    s.push((b'0' + (val % 10) as u8) as char);
}

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf).trim();

    if args.is_empty() {
        println!("Usage: wget <url> <output_path>");
        println!("Example: wget http://10.0.2.2:8080/file.txt /bin/file.txt");
        return;
    }

    // Parse arguments: <url> <output_path>
    let mut parts = args.splitn(2, ' ');
    let url_str = parts.next().unwrap_or("").trim();
    let output_path = parts.next().unwrap_or("").trim();

    if url_str.is_empty() || output_path.is_empty() {
        println!("Usage: wget <url> <output_path>");
        return;
    }

    let url = match parse_url(url_str) {
        Some(u) => u,
        None => {
            println!("Invalid URL: {}", url_str);
            return;
        }
    };

    // Resolve host
    let ip = match resolve_host(&url.host) {
        Some(ip) => ip,
        None => {
            println!("Cannot resolve host: {}", url.host);
            return;
        }
    };

    println!(
        "Connecting to {}.{}.{}.{}:{}...",
        ip[0], ip[1], ip[2], ip[3], url.port
    );

    // TCP connect
    let sock = net::tcp_connect(&ip, url.port, CONNECT_TIMEOUT);
    if sock == u32::MAX {
        println!("Connection failed");
        return;
    }

    // Build HTTP/1.0 GET request
    let mut request = String::new();
    request.push_str("GET ");
    request.push_str(&url.path);
    request.push_str(" HTTP/1.0\r\nHost: ");
    request.push_str(&url.host);
    if url.port != HTTP_PORT {
        request.push(':');
        write_u32_str(&mut request, url.port as u32);
    }
    request.push_str("\r\nConnection: close\r\nUser-Agent: anyOS-wget/1.0\r\n\r\n");

    // Send request
    let sent = net::tcp_send(sock, request.as_bytes());
    if sent == u32::MAX {
        println!("Failed to send request");
        net::tcp_close(sock);
        return;
    }

    // Receive response (headers + body)
    let mut response = Vec::new();
    let mut recv_buf = [0u8; 2048];

    // Read until we have all headers
    let header_end;
    loop {
        let n = net::tcp_recv(sock, &mut recv_buf);
        if n == 0 || n == u32::MAX {
            println!("Connection closed before headers received");
            net::tcp_close(sock);
            return;
        }
        response.extend_from_slice(&recv_buf[..n as usize]);

        if let Some(end) = find_header_end(&response) {
            header_end = end;
            break;
        }
    }

    // Parse headers
    let header_str = core::str::from_utf8(&response[..header_end]).unwrap_or("");
    let status_code = parse_status_code(header_str).unwrap_or(0);
    let content_length = parse_content_length(header_str);

    if status_code < 200 || status_code >= 400 {
        // Print first line of response
        let first_line = header_str.split('\n').next().unwrap_or("Unknown error");
        println!("HTTP error: {}", first_line.trim());
        net::tcp_close(sock);
        return;
    }

    // Redirects (301, 302, 303, 307, 308)
    if status_code >= 300 && status_code < 400 {
        println!("Redirect ({}), follow manually", status_code);
        net::tcp_close(sock);
        return;
    }

    // Open output file
    let fd = fs::open(output_path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        println!("Failed to open output file: {}", output_path);
        net::tcp_close(sock);
        return;
    }

    // Write any body data we already received past the headers
    let initial_body = &response[header_end..];
    let mut total: u32 = 0;
    if !initial_body.is_empty() {
        fs::write(fd, initial_body);
        total += initial_body.len() as u32;
    }

    // Read remaining body
    loop {
        let n = net::tcp_recv(sock, &mut recv_buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        fs::write(fd, &recv_buf[..n as usize]);
        total += n;

        // If we know content length and have received enough, stop
        if let Some(cl) = content_length {
            if total >= cl {
                break;
            }
        }
    }

    fs::close(fd);
    net::tcp_close(sock);

    if let Some(cl) = content_length {
        println!("Downloaded {} / {} bytes to {}", total, cl, output_path);
    } else {
        println!("Downloaded {} bytes to {}", total, output_path);
    }
}
