//! Git smart HTTP transport protocol.
//!
//! Implements the git smart HTTP protocol for clone, fetch, and push:
//! - Reference discovery via GET /info/refs
//! - Upload-pack (fetch) via POST /git-upload-pack
//! - Receive-pack (push) via POST /git-receive-pack

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::oid::Oid;
use crate::remote::GitUrl;
use crate::repo::{Result, Error};

/// A reference advertised by the remote.
#[derive(Debug, Clone)]
pub struct RemoteRef {
    pub oid: Oid,
    pub name: String,
}

/// Capabilities advertised by the remote.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub multi_ack: bool,
    pub thin_pack: bool,
    pub side_band: bool,
    pub side_band_64k: bool,
    pub ofs_delta: bool,
    pub shallow: bool,
    pub no_progress: bool,
    pub include_tag: bool,
    pub allow_tip_sha1_in_want: bool,
    pub allow_reachable_sha1_in_want: bool,
    pub no_done: bool,
    pub symref_head: Option<String>,
}

impl Capabilities {
    fn new() -> Self {
        Capabilities {
            multi_ack: false,
            thin_pack: false,
            side_band: false,
            side_band_64k: false,
            ofs_delta: false,
            shallow: false,
            no_progress: false,
            include_tag: false,
            allow_tip_sha1_in_want: false,
            allow_reachable_sha1_in_want: false,
            no_done: false,
            symref_head: None,
        }
    }

    fn parse(caps_str: &str) -> Self {
        let mut caps = Capabilities::new();
        for cap in caps_str.split(' ') {
            match cap {
                "multi_ack" => caps.multi_ack = true,
                "thin-pack" => caps.thin_pack = true,
                "side-band" => caps.side_band = true,
                "side-band-64k" => caps.side_band_64k = true,
                "ofs-delta" => caps.ofs_delta = true,
                "shallow" => caps.shallow = true,
                "no-progress" => caps.no_progress = true,
                "include-tag" => caps.include_tag = true,
                "allow-tip-sha1-in-want" => caps.allow_tip_sha1_in_want = true,
                "allow-reachable-sha1-in-want" => caps.allow_reachable_sha1_in_want = true,
                "no-done" => caps.no_done = true,
                s if s.starts_with("symref=HEAD:") => {
                    caps.symref_head = Some(String::from(&s[12..]));
                }
                _ => {}
            }
        }
        caps
    }
}

/// Discover references from a remote via smart HTTP.
pub fn discover_refs(url: &GitUrl, service: &str) -> Result<(Vec<RemoteRef>, Capabilities)> {
    let info_url = url.info_refs_url(service);

    libhttp_client::init();
    let response = libhttp_client::get(&info_url)
        .ok_or(Error::Other(format!("HTTP GET failed: {}", info_url)))?;

    let text = core::str::from_utf8(&response)
        .map_err(|_| Error::Other(String::from("invalid UTF-8 in response")))?;

    parse_ref_discovery(text, service)
}

/// Parse the reference discovery response.
fn parse_ref_discovery(text: &str, _service: &str) -> Result<(Vec<RemoteRef>, Capabilities)> {
    let mut refs = Vec::new();
    let mut caps = Capabilities::new();
    let mut first_line = true;

    for line in text.lines() {
        // Skip service announcement and flush lines
        if line.starts_with('#') || line.len() < 4 {
            continue;
        }

        // Parse pkt-line: first 4 chars are hex length
        let content = if line.len() >= 4 {
            let len_hex = &line[..4];
            if let Ok(len) = usize::from_str_radix(len_hex, 16) {
                if len == 0 {
                    continue; // flush
                }
                &line[4..]
            } else {
                line
            }
        } else {
            line
        };

        // Skip empty or service lines
        if content.is_empty() || content.starts_with("# ") {
            continue;
        }

        // First ref line may contain capabilities after \0
        let (ref_part, caps_part) = if let Some(null_pos) = content.find('\0') {
            (&content[..null_pos], Some(&content[null_pos + 1..]))
        } else {
            (content, None)
        };

        // Parse capabilities from first line
        if first_line {
            if let Some(caps_str) = caps_part {
                caps = Capabilities::parse(caps_str.trim());
            }
            first_line = false;
        }

        // Parse "sha1 refname"
        let ref_part = ref_part.trim();
        if ref_part.len() >= 41 {
            let hex = &ref_part[..40];
            let name = ref_part[41..].trim();
            if let Some(oid) = Oid::from_hex(hex) {
                refs.push(RemoteRef {
                    oid,
                    name: String::from(name),
                });
            }
        }
    }

    Ok((refs, caps))
}

/// Build an upload-pack request body (want/have negotiation).
pub fn build_upload_pack_request(
    wants: &[Oid],
    haves: &[Oid],
    caps: &Capabilities,
) -> Vec<u8> {
    let mut body = Vec::new();

    // Minimal capabilities for initial clone (no haves)
    let cap_str = String::from("ofs-delta agent=agit/1.0");

    // Want lines
    for (i, oid) in wants.iter().enumerate() {
        let line = if i == 0 {
            format!("want {} {}\n", oid.to_hex(), cap_str)
        } else {
            format!("want {}\n", oid.to_hex())
        };
        write_pkt_line(&mut body, &line);
    }

    // Flush after wants
    write_flush(&mut body);

    // Have lines
    for oid in haves {
        let line = format!("have {}\n", oid.to_hex());
        write_pkt_line(&mut body, &line);
    }

    // Done
    write_pkt_line(&mut body, "done\n");

    body
}

/// Build a receive-pack request body (push).
pub fn build_receive_pack_request(
    updates: &[(Oid, Oid, String)], // (old_oid, new_oid, refname)
    pack_data: &[u8],
) -> Vec<u8> {
    let mut body = Vec::new();

    // Reference update lines
    let cap_str = "report-status side-band-64k";
    for (i, (old, new, refname)) in updates.iter().enumerate() {
        let line = if i == 0 {
            format!("{} {} {}\0{}\n", old.to_hex(), new.to_hex(), refname, cap_str)
        } else {
            format!("{} {} {}\n", old.to_hex(), new.to_hex(), refname)
        };
        write_pkt_line(&mut body, &line);
    }

    // Flush after ref updates
    write_flush(&mut body);

    // Pack data follows
    body.extend_from_slice(pack_data);

    body
}

/// Perform git-upload-pack (fetch objects from remote).
pub fn fetch_pack(url: &GitUrl, wants: &[Oid], haves: &[Oid]) -> Result<Vec<u8>> {
    // Step 1: Discover refs and capabilities
    let (_, caps) = discover_refs(url, "git-upload-pack")?;

    // Step 2: Build request body
    let request_body = build_upload_pack_request(wants, haves, &caps);

    // Step 3: POST to git-upload-pack — use raw TCP+TLS to avoid libhttp issues
    let service_path = format!("{}/git-upload-pack", url.path.trim_end_matches('/'));

    if crate::pack::verbose() {
        anyos_std::println!("[fetch] POST https://{}{} ({} bytes body)", url.host, service_path, request_body.len());
    }

    let response = raw_https_post(&url.host, &service_path, &request_body)?;

    if crate::pack::verbose() {
        anyos_std::println!("[fetch] response: {} bytes", response.len());
    }

    if crate::pack::verbose() {
        anyos_std::println!("[fetch] response: {} bytes", response.len());
        let show = core::cmp::min(response.len(), 128);
        anyos_std::println!("[fetch] first {} bytes (hex):", show);
        for i in 0..show {
            anyos_std::print!("{:02x} ", response[i]);
            if (i + 1) % 32 == 0 { anyos_std::println!(); }
        }
        anyos_std::println!();
        // Also show as text if possible
        if let Ok(text) = core::str::from_utf8(&response[..core::cmp::min(256, response.len())]) {
            anyos_std::println!("[fetch] as text: {}", text);
        }
    }

    // Step 4: Process response — check for side-band data
    // We requested minimal caps (no side-band), but check anyway
    if false && (caps.side_band_64k || caps.side_band) {
        let (pack_data, progress, errors) = crate::pack::demux_sideband(&response);
        if crate::pack::verbose() {
            anyos_std::println!("[fetch] sideband: pack={} progress={} errors={}", pack_data.len(), progress.len(), errors.len());
        }
        if !errors.is_empty() {
            let err_msg = core::str::from_utf8(&errors).unwrap_or("remote error");
            return Err(Error::Other(String::from(err_msg)));
        }
        if crate::pack::verbose() {
            if let Ok(p) = core::str::from_utf8(&progress) {
                anyos_std::println!("[fetch] progress: {}", p);
            }
        }
        Ok(pack_data)
    } else {
        // No side-band — response is raw pack data
        // Skip any pkt-line NAK/ACK headers
        let pack_start = find_pack_start(&response);
        if crate::pack::verbose() {
            anyos_std::println!("[fetch] no sideband, pack starts at offset {}", pack_start);
        }
        Ok(response[pack_start..].to_vec())
    }
}

/// Perform git-receive-pack (push objects to remote).
pub fn push_pack(
    url: &GitUrl,
    updates: &[(Oid, Oid, String)],
    pack_data: &[u8],
) -> Result<String> {
    let request_body = build_receive_pack_request(updates, pack_data);
    let service_url = url.service_url("git-receive-pack");
    let content_type = "application/x-git-receive-pack-request";

    libhttp_client::init();
    let response = libhttp_client::post(&service_url, &request_body, content_type)
        .ok_or(Error::Other(format!("POST failed: {}", service_url)))?;

    let text = core::str::from_utf8(&response).unwrap_or("(binary)");
    Ok(String::from(text))
}

// ── pkt-line encoding ───────────────────────────────────────────────────────

fn write_pkt_line(buf: &mut Vec<u8>, line: &str) {
    let len = line.len() + 4; // 4 hex digits for length
    let hex = format!("{:04x}", len);
    buf.extend_from_slice(hex.as_bytes());
    buf.extend_from_slice(line.as_bytes());
}

fn write_flush(buf: &mut Vec<u8>) {
    buf.extend_from_slice(b"0000");
}

/// Perform an HTTPS POST directly using anyos_std TCP + libtls.
/// Bypasses libhttp to avoid potential issues with POST handling.
fn raw_https_post(host: &str, path: &str, body: &[u8]) -> Result<Vec<u8>> {
    // DNS resolve
    let mut ip = [0u8; 4];
    let ret = anyos_std::net::dns(host, &mut ip);
    if crate::pack::verbose() {
        anyos_std::println!("[raw] DNS {} -> ret={} ip={}.{}.{}.{}", host, ret, ip[0], ip[1], ip[2], ip[3]);
    }
    if ip[0] == 0 && ip[1] == 0 && ip[2] == 0 && ip[3] == 0 {
        return Err(Error::Other(format!("DNS failed for {}", host)));
    }

    if crate::pack::verbose() {
        anyos_std::println!("[raw] connecting to {}.{}.{}.{}:443", ip[0], ip[1], ip[2], ip[3]);
    }

    // TCP connect
    let sock = anyos_std::net::tcp_connect(&ip, 443, 15000);
    if sock == u32::MAX {
        return Err(Error::Other(String::from("TCP connect failed")));
    }

    if crate::pack::verbose() {
        anyos_std::println!("[raw] TCP connected, socket={}", sock);
    }

    // TLS handshake — initialize libtls transport first
    libtls::set_transport(tls_send, tls_recv, tls_sleep, tls_random);
    let tls_handle = libtls::connect(sock, host);
    if tls_handle < 0 {
        anyos_std::net::tcp_close(sock);
        return Err(Error::Other(format!("TLS handshake failed ({})", tls_handle)));
    }
    let tls_handle = tls_handle as u32;

    if crate::pack::verbose() {
        anyos_std::println!("[raw] TLS connected, handle={}", tls_handle);
    }

    // Build HTTP request (headers + body combined)
    let request = format!(
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Content-Type: application/x-git-upload-pack-request\r\n\
         Accept: application/x-git-upload-pack-result\r\n\
         Content-Length: {}\r\n\
         User-Agent: agit/1.0\r\n\
         Connection: close\r\n\
         \r\n",
        path, host, body.len()
    );

    // Send headers + body as one blob
    let mut send_buf = Vec::with_capacity(request.len() + body.len());
    send_buf.extend_from_slice(request.as_bytes());
    send_buf.extend_from_slice(body);

    if crate::pack::verbose() {
        anyos_std::println!("[raw] sending {} bytes (headers={}, body={})", send_buf.len(), request.len(), body.len());
    }

    let sent = libtls::send(tls_handle, &send_buf);
    if sent <= 0 {
        libtls::close(tls_handle);
        anyos_std::net::tcp_close(sock);
        return Err(Error::Other(format!("TLS send failed ({})", sent)));
    }

    // If partial send, send remaining
    let mut total_sent = sent as usize;
    while total_sent < send_buf.len() {
        let n = libtls::send(tls_handle, &send_buf[total_sent..]);
        if n <= 0 {
            break;
        }
        total_sent += n as usize;
    }

    if crate::pack::verbose() {
        anyos_std::println!("[raw] sent {} bytes total", total_sent);
    }

    // Receive full HTTP response (headers + body).
    // GitHub uses Transfer-Encoding: chunked and does NOT close the connection,
    // so we must parse the chunked encoding to know when the body ends.
    let mut raw = Vec::with_capacity(64 * 1024);
    let mut recv_buf = [0u8; 16384];

    // Step A: Read until we have the full HTTP headers (\r\n\r\n)
    let header_end;
    loop {
        let n = libtls::recv(tls_handle, &mut recv_buf);
        if crate::pack::verbose() {
            anyos_std::println!("[raw] recv={} total={}", n, raw.len());
        }
        if n <= 0 {
            libtls::close(tls_handle);
            anyos_std::net::tcp_close(sock);
            return Err(Error::Other(String::from("connection closed before headers")));
        }
        raw.extend_from_slice(&recv_buf[..n as usize]);

        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
    }

    let header_str = core::str::from_utf8(&raw[..header_end]).unwrap_or("");
    if crate::pack::verbose() {
        anyos_std::println!("[raw] headers:\n{}", header_str);
    }

    // Parse headers: check for chunked encoding and content-length
    let is_chunked = header_str.to_lowercase().contains("transfer-encoding: chunked");
    let content_length = parse_content_length(header_str);

    if crate::pack::verbose() {
        anyos_std::println!("[raw] chunked={} content_length={:?}", is_chunked, content_length);
    }

    // Step B: Read body
    let body = if is_chunked {
        // Chunked transfer-encoding: read chunks until "0\r\n"
        read_chunked_body(tls_handle, &raw[header_end..])
    } else if let Some(cl) = content_length {
        // Fixed Content-Length
        let mut body = raw[header_end..].to_vec();
        while body.len() < cl {
            let n = libtls::recv(tls_handle, &mut recv_buf);
            if n <= 0 { break; }
            body.extend_from_slice(&recv_buf[..n as usize]);
        }
        body
    } else {
        // No content-length, no chunked — read until EOF/timeout
        let mut body = raw[header_end..].to_vec();
        let mut failures = 0u32;
        loop {
            let n = libtls::recv(tls_handle, &mut recv_buf);
            if n > 0 {
                body.extend_from_slice(&recv_buf[..n as usize]);
                failures = 0;
            } else if n == 0 {
                break;
            } else {
                failures += 1;
                if failures > 5 { break; }
                anyos_std::process::sleep(200);
            }
        }
        body
    };

    libtls::close(tls_handle);
    anyos_std::net::tcp_close(sock);

    if crate::pack::verbose() {
        anyos_std::println!("[raw] body: {} bytes", body.len());
    }

    Ok(body)
}

// TLS transport callbacks for raw HTTPS
fn tls_send(fd: u32, data: &[u8]) -> i32 {
    let n = anyos_std::net::tcp_send(fd, data);
    if n == u32::MAX { -1 } else { n as i32 }
}

fn tls_recv(fd: u32, buf: &mut [u8]) -> i32 {
    // First try: blocking recv (kernel has 30s timeout)
    let n = anyos_std::net::tcp_recv(fd, buf);
    if n == 0 { return 0; }
    if n != u32::MAX { return n as i32; }

    // Timeout — check if connection is still alive
    let avail = anyos_std::net::tcp_recv_available(fd);
    match avail {
        u32::MAX => -1,        // Error
        0xFFFFFFFE => 0,       // EOF
        _ => {
            // Connection alive but no data yet, retry once after short delay
            anyos_std::process::sleep(50);
            let n = anyos_std::net::tcp_recv(fd, buf);
            if n == 0 { 0 }
            else if n != u32::MAX { n as i32 }
            else { -1 }
        }
    }
}

/// Parse Content-Length from HTTP headers.
fn parse_content_length(headers: &str) -> Option<usize> {
    for line in headers.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("content-length:") {
            let val = line[15..].trim();
            return val.parse().ok();
        }
    }
    None
}

/// Read a chunked HTTP body from TLS.
/// `initial` contains any body bytes already read with the headers.
fn read_chunked_body(tls_handle: u32, initial: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(initial.len() + 4096);
    buf.extend_from_slice(initial);
    let mut body = Vec::with_capacity(64 * 1024);
    let mut cursor = 0usize;
    let mut recv_buf = [0u8; 16384];

    loop {
        // Ensure we have enough data to parse chunk header
        while !has_crlf(&buf[cursor..]) {
            let n = libtls::recv(tls_handle, &mut recv_buf);
            if n <= 0 { return body; } // EOF or error
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        // Parse chunk size (hex)
        let crlf_pos = find_crlf_pos(&buf[cursor..]).unwrap();
        let size_str = core::str::from_utf8(&buf[cursor..cursor + crlf_pos]).unwrap_or("0");
        let size_str = match size_str.find(';') {
            Some(i) => &size_str[..i],
            None => size_str,
        };
        let chunk_size = usize::from_str_radix(size_str.trim(), 16).unwrap_or(0);
        cursor += crlf_pos + 2; // Skip size line + \r\n

        if crate::pack::verbose() {
            anyos_std::println!("[raw] chunk size={} (body so far={})", chunk_size, body.len());
        }

        // Progress display (like git)
        let mb = body.len() / (1024 * 1024);
        let kb = (body.len() % (1024 * 1024)) / 1024;
        if mb > 0 {
            anyos_std::print!("\rReceiving objects: {}.{:02} MiB", mb, kb * 100 / 1024);
        } else {
            anyos_std::print!("\rReceiving objects: {} KiB", body.len() / 1024);
        }

        if chunk_size == 0 {
            // Final progress line
            let mb = body.len() / (1024 * 1024);
            let kb = (body.len() % (1024 * 1024)) / 1024;
            if mb > 0 {
                anyos_std::println!("\rReceiving objects: {}.{:02} MiB, done.", mb, kb * 100 / 1024);
            } else {
                anyos_std::println!("\rReceiving objects: {} KiB, done.", body.len() / 1024);
            }
            break;
        }

        // Read chunk data
        while buf.len() - cursor < chunk_size + 2 { // +2 for trailing \r\n
            let n = libtls::recv(tls_handle, &mut recv_buf);
            if n <= 0 { return body; }
            buf.extend_from_slice(&recv_buf[..n as usize]);
        }

        body.extend_from_slice(&buf[cursor..cursor + chunk_size]);
        cursor += chunk_size + 2; // Skip data + \r\n

        // Compact buffer periodically
        if cursor > 32768 {
            buf = buf[cursor..].to_vec();
            cursor = 0;
        }
    }

    body
}

fn has_crlf(data: &[u8]) -> bool {
    data.windows(2).any(|w| w == b"\r\n")
}

fn find_crlf_pos(data: &[u8]) -> Option<usize> {
    data.windows(2).position(|w| w == b"\r\n")
}

fn tls_sleep(ms: u32) { anyos_std::process::sleep(ms); }
fn tls_random(buf: &mut [u8]) -> u32 { anyos_std::sys::random(buf) }

/// Find where "PACK" header starts in response data.
fn find_pack_start(data: &[u8]) -> usize {
    for i in 0..data.len().saturating_sub(4) {
        if &data[i..i + 4] == b"PACK" {
            return i;
        }
    }
    0
}

/// Parse a pkt-line response to extract lines.
pub fn parse_pkt_lines(data: &[u8]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut pos = 0;

    while pos + 4 <= data.len() {
        let hex = core::str::from_utf8(&data[pos..pos + 4]).unwrap_or("0000");
        let len = usize::from_str_radix(hex, 16).unwrap_or(0);
        pos += 4;

        if len == 0 {
            continue; // flush
        }
        if len == 1 {
            continue; // delimiter
        }

        let payload_len = len.saturating_sub(4);
        let end = core::cmp::min(pos + payload_len, data.len());
        lines.push(data[pos..end].to_vec());
        pos = end;
    }

    lines
}
