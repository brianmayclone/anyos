#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{print, println, net, fs, sys, args};

anyos_std::entry!(main);

const HTTP_PORT: u16 = 80;
const CONNECT_TIMEOUT: u32 = 10000;
const MAX_REDIRECTS: usize = 20;
const VERSION: &str = "1.0";

// ── URL parsing ─────────────────────────────────────────────────────────────

struct Url {
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url_str: &str) -> Option<Url> {
    let rest = if starts_with_ignore_case(url_str, "https://") {
        // We don't support HTTPS but still parse the URL
        &url_str[8..]
    } else if starts_with_ignore_case(url_str, "http://") {
        &url_str[7..]
    } else {
        url_str
    };

    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

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
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    if s.len() < prefix.len() { return false; }
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
    if val < 10 { s.push('0'); }
    push_u32(s, val);
}

// ── HTTP header parsing ─────────────────────────────────────────────────────

fn find_header_end(data: &[u8]) -> Option<usize> {
    if data.len() < 4 { return None; }
    for i in 0..data.len() - 3 {
        if data[i] == b'\r' && data[i+1] == b'\n' && data[i+2] == b'\r' && data[i+3] == b'\n' {
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
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut len = 0;
    let mut v = val;
    while v > 0 {
        tmp[len] = b'0' + (v % 10) as u8;
        v /= 10;
        len += 1;
    }
    for i in 0..len {
        buf[i] = tmp[len - 1 - i];
    }
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

fn draw_progress(filename: &str, received: u32, total: Option<u32>, elapsed_ticks: u32, tick_hz: u32) {
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
            let pct = if received >= tot { 100u32 } else { (received as u64 * 100 / tot as u64) as u32 };
            // "100%" or " 42%"
            if pct < 10 { line.push(' '); line.push(' '); }
            else if pct < 100 { line.push(' '); }
            push_u32(&mut line, pct);
            line.push('%');

            // Bar: [====================>]
            line.push('[');
            let bar_width: u32 = 20;
            let filled = if received >= tot { bar_width } else { (received as u64 * bar_width as u64 / tot as u64) as u32 };
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

// ── Build HTTP request ──────────────────────────────────────────────────────

fn build_request(url: &Url, resume_offset: u32) -> String {
    let mut req = String::new();
    req.push_str("GET ");
    req.push_str(&url.path);
    req.push_str(" HTTP/1.1\r\nHost: ");
    req.push_str(&url.host);
    if url.port != HTTP_PORT {
        req.push(':');
        push_u32(&mut req, url.port as u32);
    }
    req.push_str("\r\nUser-Agent: Wget/");
    req.push_str(VERSION);
    req.push_str(" (anyOS)\r\nConnection: close\r\nAccept: */*\r\n");

    if resume_offset > 0 {
        req.push_str("Range: bytes=");
        push_u32(&mut req, resume_offset);
        req.push_str("-\r\n");
    }

    req.push_str("\r\n");
    req
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

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf).trim();
    let parsed = args::parse(raw, b"O");

    // Handle --help and --version as positional args
    for i in 0..parsed.pos_count {
        match parsed.positional[i] {
            "--help" => { print_usage(); return; }
            "--version" => { print_version(); return; }
            _ => {}
        }
    }

    if parsed.has(b'h') { print_usage(); return; }
    if parsed.has(b'V') { print_version(); return; }

    let quiet = parsed.has(b'q');
    let resume = parsed.has(b'c');
    let output_flag = parsed.opt(b'O');

    // Find the URL (first positional that looks like a URL or hostname)
    let url_str = parsed.first_or("");
    if url_str.is_empty() {
        println!("wget: missing URL");
        println!("Usage: wget [OPTION]... [URL]...");
        return;
    }

    // If URL starts with https://, warn
    if starts_with_ignore_case(url_str, "https://") {
        if !quiet {
            println!("ERROR: HTTPS support not available.");
        }
        return;
    }

    // Parse the initial URL
    let mut current_url = match parse_url(url_str) {
        Some(u) => u,
        None => {
            println!("wget: invalid URL '{}'", url_str);
            return;
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
        let mut stat_buf = [0u32; 6];
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
            println!("--{}--  http://{}{}", ts,
                if current_url.port != HTTP_PORT {
                    let mut h = current_url.host.clone();
                    h.push(':');
                    push_u32(&mut h, current_url.port as u32);
                    h
                } else {
                    current_url.host.clone()
                },
                current_url.path);
        }

        // DNS resolution
        if !quiet {
            print!("Resolving {} ({})... ", current_url.host, current_url.host);
        }
        let ip = match resolve_host(&current_url.host) {
            Some(ip) => {
                if !quiet {
                    println!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                }
                ip
            }
            None => {
                if !quiet { println!("failed: Name or service not known."); }
                println!("wget: unable to resolve host address '{}'", current_url.host);
                return;
            }
        };

        // Connect
        if !quiet {
            print!("Connecting to {} ({})|{}.{}.{}.{}|:{}... ",
                current_url.host, current_url.host,
                ip[0], ip[1], ip[2], ip[3], current_url.port);
        }
        let sock = net::tcp_connect(&ip, current_url.port, CONNECT_TIMEOUT);
        if sock == u32::MAX {
            if !quiet { println!("failed: Connection refused."); }
            println!("wget: unable to connect to {}:{}", current_url.host, current_url.port);
            return;
        }
        if !quiet { println!("connected."); }

        // Send request
        let request = build_request(&current_url, existing_size);
        if !quiet {
            print!("HTTP request sent, awaiting response... ");
        }
        let sent = net::tcp_send(sock, request.as_bytes());
        if sent == u32::MAX {
            if !quiet { println!("failed."); }
            net::tcp_close(sock);
            return;
        }

        // Receive headers
        let mut response = Vec::new();
        let mut recv_buf = [0u8; 4096];
        let header_end;
        loop {
            let n = net::tcp_recv(sock, &mut recv_buf);
            if n == 0 || n == u32::MAX {
                if !quiet { println!("no data received."); }
                net::tcp_close(sock);
                return;
            }
            response.extend_from_slice(&recv_buf[..n as usize]);
            if let Some(end) = find_header_end(&response) {
                header_end = end;
                break;
            }
            if response.len() > 16384 {
                if !quiet { println!("headers too large."); }
                net::tcp_close(sock);
                return;
            }
        }

        let header_str = core::str::from_utf8(&response[..header_end]).unwrap_or("");
        let (status, reason) = parse_status_line(header_str);

        if !quiet {
            println!("{} {}", status, reason);
        }

        // Handle redirect
        if is_redirect(status) {
            net::tcp_close(sock);
            if let Some(loc) = parse_location(header_str) {
                if !quiet {
                    println!("Location: {} [following]", loc);
                }
                // Parse new URL — could be relative or absolute
                if starts_with_ignore_case(loc, "http://") || starts_with_ignore_case(loc, "https://") {
                    if starts_with_ignore_case(loc, "https://") {
                        if !quiet { println!("ERROR: HTTPS support not available."); }
                        return;
                    }
                    current_url = match parse_url(loc) {
                        Some(u) => u,
                        None => {
                            println!("wget: invalid redirect URL");
                            return;
                        }
                    };
                } else {
                    // Relative redirect — keep same host/port
                    current_url.path = String::from(loc);
                }
                continue;
            } else {
                println!("wget: redirect with no Location header");
                return;
            }
        }

        // Handle errors
        if status >= 400 {
            let ts = fmt_timestamp();
            println!("{} ERROR {}: {}.", ts, status, reason);
            net::tcp_close(sock);
            return;
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
                net::tcp_close(sock);
                return;
            }
            f
        } else {
            let f = fs::open(&out_filename, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
            if f == u32::MAX {
                println!("wget: cannot open '{}' for writing", out_filename);
                net::tcp_close(sock);
                return;
            }
            f
        };

        // Write initial body data past headers
        let initial_body = &response[header_end..];
        let mut received: u32 = 0;
        if !initial_body.is_empty() {
            fs::write(fd, initial_body);
            received += initial_body.len() as u32;
        }

        let start_ticks = sys::uptime();
        let mut last_progress_bytes: u32 = 0;

        // Show initial progress
        if !quiet && !to_stdout {
            let display_received = received + existing_size;
            draw_progress(&out_filename, display_received, total_size, 0, tick_hz);
        }

        // Download body
        loop {
            let n = net::tcp_recv(sock, &mut recv_buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            fs::write(fd, &recv_buf[..n as usize]);
            received += n;

            // Update progress every 2048 bytes
            if !quiet && !to_stdout && received - last_progress_bytes >= 2048 {
                last_progress_bytes = received;
                let elapsed = sys::uptime().wrapping_sub(start_ticks);
                let display_received = received + existing_size;
                draw_progress(&out_filename, display_received, total_size, elapsed, tick_hz);
            }

            // Stop if we got everything
            if let Some(cl) = content_length {
                if received >= cl {
                    break;
                }
            }
        }

        // Final progress
        let elapsed = sys::uptime().wrapping_sub(start_ticks);
        if !quiet && !to_stdout {
            let display_received = received + existing_size;
            draw_progress(&out_filename, display_received, total_size, elapsed, tick_hz);
            println!();
            println!();
        }

        // Close file and socket
        if !to_stdout {
            fs::close(fd);
        }
        net::tcp_close(sock);

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
                println!("{} ({}) - '{}' saved [{}/{}]", ts, speed_str, out_filename, total, tot);
            } else {
                println!("{} ({}) - '{}' saved [{}]", ts, speed_str, out_filename, total);
            }
        }

        return; // Done!
    }

    // Exceeded max redirects
    println!("wget: maximum {} redirects exceeded", MAX_REDIRECTS);
}
