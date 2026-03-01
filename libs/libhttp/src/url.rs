//! URL parsing and resolution for HTTP/HTTPS.
//!
//! Supports `http://` and `https://` schemes with host:port parsing
//! and relative URL resolution.

use alloc::string::String;
use alloc::vec::Vec;

/// Parsed URL components.
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

/// Parse a URL string into components.
/// Supports `http://` and `https://` schemes.
pub fn parse_url(s: &str) -> Option<Url> {
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
            let port = parse_u16(port_str)?;
            (&host_port[..idx], port)
        }
        None => (host_port, default_port),
    };

    if host.is_empty() {
        return None;
    }

    Some(Url {
        scheme: String::from(scheme),
        host: String::from(host),
        port,
        path: String::from(path),
    })
}

/// Resolve a relative URL against a base URL.
pub fn resolve_url(base: &Url, relative: &str) -> Url {
    if starts_with_ignore_case(relative, "http://")
        || starts_with_ignore_case(relative, "https://")
    {
        match parse_url(relative) {
            Some(u) => return u,
            None => return clone_url(base),
        }
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

/// Clone a URL.
pub fn clone_url(url: &Url) -> Url {
    Url {
        scheme: url.scheme.clone(),
        host: url.host.clone(),
        port: url.port,
        path: url.path.clone(),
    }
}

// ── Numeric / string helpers ─────────────────────────────────────────────────

/// Parse an ASCII decimal string into u16.
pub fn parse_u16(s: &str) -> Option<u16> {
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

/// Parse an ASCII decimal string into u32.
pub fn parse_u32(s: &str) -> Option<u32> {
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

/// Parse a dotted-decimal IPv4 address string.
pub fn parse_ip(s: &str) -> Option<[u8; 4]> {
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

/// Convert ASCII byte to lowercase.
pub fn ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

/// Case-insensitive prefix check.
pub fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    if s.len() < prefix.len() { return false; }
    let sb = s.as_bytes();
    let pb = prefix.as_bytes();
    for i in 0..pb.len() {
        if ascii_lower(sb[i]) != ascii_lower(pb[i]) { return false; }
    }
    true
}

/// Append a u32 as decimal digits to a String.
pub fn push_u32(s: &mut String, val: u32) {
    if val >= 10 { push_u32(s, val / 10); }
    s.push((b'0' + (val % 10) as u8) as char);
}

/// Parse a hexadecimal string (for chunked transfer-encoding).
pub fn parse_hex(s: &str) -> usize {
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

/// Find the value of an HTTP header by name (case-insensitive).
/// Returns the trimmed value after the `:` separator.
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
