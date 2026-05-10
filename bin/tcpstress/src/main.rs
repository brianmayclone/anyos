#![no_std]
#![no_main]

use alloc::format;
use anyos_std::{crypto, net, println, process, sys, String, Vec};

anyos_std::entry!(main);

const VERSION: &str = "0.1";
const DEFAULT_URL: &str =
    "http://archive.debian.org/debian/dists/wheezy/main/binary-amd64/Packages.gz";
const RECV_BUF_SIZE: usize = 64 * 1024;
const MAX_HEADER_SIZE: usize = 16 * 1024;
const MAX_BODY_SIZE: usize = 32 * 1024 * 1024;
const CONNECT_TIMEOUT_MS: u32 = 10_000;
const TAIL_RETRIES: u32 = 30;
const LOOPBACK_PORT: u16 = 18181;
const LOOPBACK_BODY_LEN: usize = 128 * 1024;

struct Config {
    repeat: u32,
    url: String,
    keep_body: bool,
}

impl Config {
    fn default() -> Self {
        Self {
            repeat: 5,
            url: String::from(DEFAULT_URL),
            keep_body: true,
        }
    }
}

#[derive(Clone, Copy)]
struct RunOptions {
    keep_body: bool,
    read_limit: usize,
    split_send: bool,
}

struct Url {
    host: String,
    port: u16,
    path: String,
}

#[derive(Clone)]
struct RunResult {
    round: u32,
    status: u16,
    bytes: u32,
    header_bytes: u32,
    content_length: Option<u32>,
    ms: u32,
    error: &'static str,
    md5: [u8; 32],
    gzip_magic: bool,
}

struct Summary {
    tests: u32,
    failures: u32,
    warnings: u32,
}

impl Summary {
    fn new() -> Self {
        Self {
            tests: 0,
            failures: 0,
            warnings: 0,
        }
    }

    fn ok(&mut self, name: &str, detail: &str) {
        self.tests += 1;
        println!("  [OK]   {:<20} {}", name, detail);
    }

    fn fail(&mut self, name: &str, detail: &str) {
        self.tests += 1;
        self.failures += 1;
        println!("  [FAIL] {:<20} {}", name, detail);
    }
}

fn main() {
    let Some(cfg) = parse_config() else {
        return;
    };
    let started = sys::uptime_ms();
    let mut summary = Summary::new();

    println!();
    println!("tcpstress {} - TCP Diagnose", VERSION);
    println!("===========================");
    println!("  repeat: {}", cfg.repeat);
    println!("  url:    {}", cfg.url);
    println!(
        "  mode:   {}",
        if cfg.keep_body {
            "body-md5"
        } else {
            "drain-only"
        }
    );
    println!();

    let url = match parse_url(&cfg.url) {
        Some(u) => u,
        None => {
            summary.fail("url", "nur http://host[:port]/path wird unterstuetzt");
            print_summary(&summary, started);
            return;
        }
    };

    let mut ip = [0u8; 4];
    if net::dns(&url.host, &mut ip) != 0 {
        summary.fail(
            "dns",
            &format!("{} konnte nicht aufgeloest werden", url.host),
        );
        print_summary(&summary, started);
        return;
    }
    summary.ok(
        "dns",
        &format!("{} -> {}.{}.{}.{}", url.host, ip[0], ip[1], ip[2], ip[3]),
    );

    println!();
    println!("--- TCP-Runden ---");
    let mut results = Vec::new();
    for round in 1..=cfg.repeat {
        let result = run_round(round, &url, &ip, cfg.keep_body);
        print_round(&result);
        if result.error == "ok" {
            summary.ok(
                "round",
                &format!("{} bytes status={}", result.bytes, result.status),
            );
        } else {
            summary.fail(
                "round",
                &format!("round={} error={}", result.round, result.error),
            );
        }
        results.push(result);
    }

    compare_results(&results, &mut summary);

    run_extended_tests(&url, &ip, &mut summary);

    print_protocol(&results);
    print_summary(&summary, started);
}

fn parse_config() -> Option<Config> {
    let mut buf = [0u8; 512];
    let raw = process::args(&mut buf);
    if raw.contains("--help") || raw.contains("-h") {
        print_usage();
        return None;
    }

    let mut cfg = Config::default();
    let args: Vec<&str> = raw.split_ascii_whitespace().collect();
    let mut i = 0usize;
    while i < args.len() {
        match args[i] {
            "--repeat" | "-r" => {
                i += 1;
                if i >= args.len() {
                    println!("tcpstress: --repeat braucht eine Zahl");
                    return None;
                }
                cfg.repeat = clamp(parse_u32(args[i]).unwrap_or(cfg.repeat), 1, 1000);
            }
            "--url" | "-u" => {
                i += 1;
                if i >= args.len() {
                    println!("tcpstress: --url braucht eine URL");
                    return None;
                }
                cfg.url = String::from(args[i]);
            }
            "--drain-only" => cfg.keep_body = false,
            other => {
                println!("tcpstress: unbekannte Option '{}'", other);
                return None;
            }
        }
        i += 1;
    }
    Some(cfg)
}

fn print_usage() {
    println!("tcpstress - roher TCP/HTTP Download-Stresstest");
    println!();
    println!("Usage: tcpstress [options]");
    println!("  --repeat N       Wiederholungen (default: 5)");
    println!("  --url URL        http://host[:port]/path");
    println!("  --drain-only     Body nicht speichern, nur Bytes zaehlen");
    println!("  --help, -h       diese Hilfe anzeigen");
}

fn parse_url(raw: &str) -> Option<Url> {
    let rest = raw.strip_prefix("http://")?;
    let slash = rest.find('/').unwrap_or(rest.len());
    let host_port = &rest[..slash];
    let path = if slash < rest.len() {
        &rest[slash..]
    } else {
        "/"
    };
    if host_port.is_empty() {
        return None;
    }

    let mut host = host_port;
    let mut port = 80u16;
    if let Some(colon) = host_port.rfind(':') {
        let p = parse_u32(&host_port[colon + 1..])?;
        if p == 0 || p > u16::MAX as u32 {
            return None;
        }
        host = &host_port[..colon];
        port = p as u16;
    }
    if host.is_empty() {
        return None;
    }
    Some(Url {
        host: String::from(host),
        port,
        path: String::from(path),
    })
}

fn run_round(round: u32, url: &Url, ip: &[u8; 4], keep_body: bool) -> RunResult {
    run_round_with_options(
        round,
        url,
        ip,
        RunOptions {
            keep_body,
            read_limit: RECV_BUF_SIZE,
            split_send: false,
        },
    )
}

fn run_round_with_options(round: u32, url: &Url, ip: &[u8; 4], opts: RunOptions) -> RunResult {
    let start = sys::uptime_ms();
    let sock = net::tcp_connect(ip, url.port, CONNECT_TIMEOUT_MS);
    if sock == u32::MAX {
        return failed(round, elapsed_ms(start), "connect");
    }

    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: tcpstress/{}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        url.path, url.host, VERSION
    );
    let sent = if opts.split_send {
        send_split(sock, request.as_bytes(), 1, 2)
    } else {
        send_all(sock, request.as_bytes())
    };
    if !sent {
        let _ = net::tcp_close(sock);
        return failed(round, elapsed_ms(start), "send");
    }

    let mut recv_buf = [0u8; RECV_BUF_SIZE];
    let read_limit = opts.read_limit.clamp(1, RECV_BUF_SIZE);
    let mut header = Vec::new();
    let header_end;
    loop {
        let n = recv_some(sock, &mut recv_buf[..read_limit]);
        if n == 0 {
            let _ = net::tcp_close(sock);
            return failed(round, elapsed_ms(start), "header");
        }
        header.extend_from_slice(&recv_buf[..n]);
        if let Some(end) = find_header_end(&header) {
            header_end = end;
            break;
        }
        if header.len() > MAX_HEADER_SIZE {
            let _ = net::tcp_close(sock);
            return failed(round, elapsed_ms(start), "header-large");
        }
    }

    let header_text = core::str::from_utf8(&header[..header_end]).unwrap_or("");
    let status = parse_status(header_text);
    let content_length = parse_content_length(header_text);
    let mut body_len = 0usize;
    let mut body = Vec::new();
    if opts.keep_body {
        let cap = content_length
            .map(|n| n as usize)
            .unwrap_or(RECV_BUF_SIZE)
            .min(MAX_BODY_SIZE);
        let _ = body.try_reserve_exact(cap);
    }

    if header_end < header.len() {
        let trailing = &header[header_end..];
        body_len += trailing.len();
        if opts.keep_body {
            body.extend_from_slice(trailing);
        }
    }

    let mut tail_retries = 0u32;
    loop {
        if let Some(cl) = content_length {
            if body_len >= cl as usize {
                break;
            }
        }

        let n = recv_some(sock, &mut recv_buf[..read_limit]);
        if n == 0 {
            if let Some(cl) = content_length {
                if body_len < cl as usize && tail_retries < TAIL_RETRIES {
                    tail_retries += 1;
                    process::sleep(100);
                    continue;
                }
            }
            break;
        }
        tail_retries = 0;
        let take = if let Some(cl) = content_length {
            n.min((cl as usize).saturating_sub(body_len))
        } else {
            n
        };
        body_len += take;
        if opts.keep_body {
            body.extend_from_slice(&recv_buf[..take]);
        }
    }
    let _ = net::tcp_close(sock);

    let mut error = "ok";
    if let Some(cl) = content_length {
        if body_len != cl as usize {
            error = "short-body";
        }
    }

    let md5 = if opts.keep_body && error == "ok" {
        crypto::md5_hex(&body)
    } else {
        [b'-'; 32]
    };
    let gzip_magic = opts.keep_body && body.len() >= 2 && body[0] == 0x1f && body[1] == 0x8b;

    RunResult {
        round,
        status,
        bytes: body_len.min(u32::MAX as usize) as u32,
        header_bytes: header_end.min(u32::MAX as usize) as u32,
        content_length,
        ms: elapsed_ms(start),
        error,
        md5,
        gzip_magic,
    }
}

fn run_extended_tests(url: &Url, ip: &[u8; 4], summary: &mut Summary) {
    println!();
    println!("--- Zusatztests ---");

    let drain = run_round_with_options(
        1,
        url,
        ip,
        RunOptions {
            keep_body: false,
            read_limit: RECV_BUF_SIZE,
            split_send: false,
        },
    );
    if drain.error == "ok" && drain.bytes > 0 {
        summary.ok(
            "remote drain",
            &format!("{} bytes in {} ms", drain.bytes, drain.ms),
        );
    } else {
        summary.fail(
            "remote drain",
            &format!("error={} bytes={}", drain.error, drain.bytes),
        );
    }

    let split = run_round_with_options(
        1,
        url,
        ip,
        RunOptions {
            keep_body: false,
            read_limit: RECV_BUF_SIZE,
            split_send: true,
        },
    );
    if split.error == "ok" && split.bytes > 0 {
        summary.ok(
            "split request",
            &format!("{} bytes in {} ms", split.bytes, split.ms),
        );
    } else {
        summary.fail(
            "split request",
            &format!("error={} bytes={}", split.error, split.bytes),
        );
    }

    match loopback_case() {
        Ok(detail) => summary.ok("loopback tcp", &detail),
        Err(detail) => summary.fail("loopback tcp", &detail),
    }
}

fn loopback_case() -> Result<String, String> {
    let server = match process::Thread::spawn(loopback_server, "tcpstress-lb") {
        Ok(t) => t,
        Err(_) => return Err(String::from("thread spawn")),
    };

    process::sleep(50);
    let ip = [127, 0, 0, 1];
    let start = sys::uptime_ms();
    let sock = net::tcp_connect(&ip, LOOPBACK_PORT, 2_000);
    if sock == u32::MAX {
        let rc = server.join();
        return Err(format!("connect server_rc={}", rc));
    }

    let request = b"GET /loopback HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if !send_split(sock, request, 3, 1) {
        let _ = net::tcp_close(sock);
        let rc = server.join();
        return Err(format!("client send server_rc={}", rc));
    }

    let mut buf = [0u8; 257];
    let mut received = 0usize;
    let mut body = Vec::new();
    let _ = body.try_reserve_exact(LOOPBACK_BODY_LEN);
    let mut header = Vec::new();
    let mut body_started = false;
    let mut expected = None;
    loop {
        let n = recv_some(sock, &mut buf);
        if n == 0 {
            break;
        }
        if !body_started {
            header.extend_from_slice(&buf[..n]);
            if let Some(end) = find_header_end(&header) {
                let header_text = core::str::from_utf8(&header[..end]).unwrap_or("");
                expected = parse_content_length(header_text);
                let trailing = &header[end..];
                received += trailing.len();
                body.extend_from_slice(trailing);
                body_started = true;
            }
        } else {
            received += n;
            body.extend_from_slice(&buf[..n]);
        }
        if received >= LOOPBACK_BODY_LEN {
            break;
        }
    }
    let _ = net::tcp_close(sock);
    let join = server.join();

    if join != 0 {
        return Err(format!("server rc={}", join));
    }
    if elapsed_ms(start) > 10_000 {
        return Err(String::from("timeout"));
    }
    if expected != Some(LOOPBACK_BODY_LEN as u32) {
        return Err(format!("content-length expected={:?}", expected));
    }
    if received != LOOPBACK_BODY_LEN {
        return Err(format!("short-body bytes={}", received));
    }
    if crypto::md5_hex(&body) != loopback_md5() {
        return Err(String::from("md5"));
    }
    Ok(String::from("listen/accept/send/recv/close ok"))
}

fn loopback_server() {
    let listener = net::tcp_listen(LOOPBACK_PORT, 1);
    if listener == u32::MAX {
        process::exit(1);
    }
    let accept_start = sys::uptime_ms();
    let sock = loop {
        let (sock, _, _) = net::tcp_accept_nowait(listener);
        if sock != u32::MAX {
            break sock;
        }
        if elapsed_ms(accept_start) > 3_000 {
            let _ = net::tcp_close(listener);
            process::exit(2);
        }
        process::sleep(10);
    };
    if sock == u32::MAX {
        let _ = net::tcp_close(listener);
        process::exit(2);
    }

    let mut req = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = recv_some(sock, &mut buf);
        if n == 0 {
            let _ = net::tcp_close(sock);
            let _ = net::tcp_close(listener);
            process::exit(3);
        }
        req.extend_from_slice(&buf[..n]);
        if find_header_end(&req).is_some() {
            break;
        }
        if req.len() > 1024 {
            let _ = net::tcp_close(sock);
            let _ = net::tcp_close(listener);
            process::exit(4);
        }
    }

    let header = format!(
        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        LOOPBACK_BODY_LEN
    );
    if !send_all(sock, header.as_bytes()) {
        let _ = net::tcp_close(sock);
        let _ = net::tcp_close(listener);
        process::exit(5);
    }

    let mut chunk = [0u8; 1024];
    let mut sent = 0usize;
    while sent < LOOPBACK_BODY_LEN {
        let n = (LOOPBACK_BODY_LEN - sent).min(chunk.len());
        fill_loopback_pattern(sent, &mut chunk[..n]);
        if !send_all(sock, &chunk[..n]) {
            let _ = net::tcp_close(sock);
            let _ = net::tcp_close(listener);
            process::exit(6);
        }
        sent += n;
        if sent % (16 * 1024) == 0 {
            process::yield_cpu();
        }
    }

    let _ = net::tcp_close(sock);
    let _ = net::tcp_close(listener);
    process::exit(0);
}

fn failed(round: u32, ms: u32, error: &'static str) -> RunResult {
    RunResult {
        round,
        status: 0,
        bytes: 0,
        header_bytes: 0,
        content_length: None,
        ms,
        error,
        md5: [b'-'; 32],
        gzip_magic: false,
    }
}

fn send_all(sock: u32, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        let n = net::tcp_send(sock, data);
        if n == u32::MAX || n == 0 {
            return false;
        }
        let n = (n as usize).min(data.len());
        data = &data[n..];
    }
    true
}

fn send_split(sock: u32, mut data: &[u8], chunk: usize, pause_ms: u32) -> bool {
    let chunk = chunk.max(1);
    while !data.is_empty() {
        let n = data.len().min(chunk);
        if !send_all(sock, &data[..n]) {
            return false;
        }
        data = &data[n..];
        if pause_ms > 0 {
            process::sleep(pause_ms);
        }
    }
    true
}

fn recv_some(sock: u32, buf: &mut [u8]) -> usize {
    for _ in 0..3 {
        let n = net::tcp_recv(sock, buf);
        if n == 0 {
            return 0;
        }
        if n != u32::MAX {
            return n as usize;
        }
        let avail = net::tcp_recv_available(sock);
        if avail == u32::MAX || avail == u32::MAX - 1 {
            return 0;
        }
        process::sleep(100);
    }
    0
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    for i in 0..buf.len().saturating_sub(3) {
        if &buf[i..i + 4] == b"\r\n\r\n" {
            return Some(i + 4);
        }
    }
    None
}

fn parse_status(headers: &str) -> u16 {
    let line = headers.lines().next().unwrap_or("");
    let mut parts = line.split_ascii_whitespace();
    let _http = parts.next();
    parts.next().and_then(parse_u32).unwrap_or(0).min(999) as u16
}

fn parse_content_length(headers: &str) -> Option<u32> {
    for line in headers.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if eq_ignore_ascii_case(name.trim(), "content-length") {
                return parse_u32(value.trim());
            }
        }
    }
    None
}

fn compare_results(results: &[RunResult], summary: &mut Summary) {
    println!();
    println!("--- Konsistenz ---");
    let ok: Vec<&RunResult> = results.iter().filter(|r| r.error == "ok").collect();
    if ok.is_empty() {
        summary.fail("success count", "keine erfolgreichen Runden");
        return;
    }
    summary.ok(
        "success count",
        &format!("{} erfolgreiche Runden", ok.len()),
    );

    let first = ok[0];
    let mut size_mismatch = 0u32;
    let mut md5_mismatch = 0u32;
    let mut gzip_mismatch = 0u32;
    for r in &ok {
        if r.bytes != first.bytes {
            size_mismatch += 1;
        }
        if r.md5 != first.md5 {
            md5_mismatch += 1;
        }
        if r.gzip_magic != first.gzip_magic {
            gzip_mismatch += 1;
        }
    }

    if size_mismatch == 0 {
        summary.ok("size stable", &format!("{} bytes", first.bytes));
    } else {
        summary.fail("size stable", &format!("{} mismatches", size_mismatch));
    }
    if md5_mismatch == 0 {
        summary.ok("md5 stable", &format!("{}", hex_str(&first.md5)));
    } else {
        summary.fail("md5 stable", &format!("{} mismatches", md5_mismatch));
    }
    if gzip_mismatch == 0 {
        summary.ok(
            "gzip magic",
            if first.gzip_magic {
                "present"
            } else {
                "not present"
            },
        );
    } else {
        summary.fail("gzip magic", &format!("{} mismatches", gzip_mismatch));
    }
}

fn print_round(r: &RunResult) {
    println!(
        "  {:>3}: bytes={} ms={} status={} clen={} hdr={} md5={} gzip={} err={}",
        r.round,
        r.bytes,
        r.ms,
        r.status,
        r.content_length.unwrap_or(0),
        r.header_bytes,
        hex_str(&r.md5),
        if r.gzip_magic { "yes" } else { "no" },
        r.error
    );
}

fn print_protocol(results: &[RunResult]) {
    println!();
    println!("--- Protokoll ---");
    println!("  round | bytes | ms | status | clen | hdr | md5 | gzip | error");
    for r in results {
        println!(
            "  {} | {} | {} | {} | {} | {} | {} | {} | {}",
            r.round,
            r.bytes,
            r.ms,
            r.status,
            r.content_length.unwrap_or(0),
            r.header_bytes,
            hex_str(&r.md5),
            if r.gzip_magic { "yes" } else { "no" },
            r.error
        );
    }
}

fn print_summary(summary: &Summary, started: u32) {
    println!();
    println!("=== Zusammenfassung ===");
    println!("  Tests:    {}", summary.tests);
    println!("  Warns:    {}", summary.warnings);
    println!("  Fehler:   {}", summary.failures);
    println!("  Laufzeit: {} ms", elapsed_ms(started));
    if summary.failures == 0 {
        println!("  Ergebnis: PASS");
    } else {
        println!("  Ergebnis: FAIL");
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut value = 0u32;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(value)
}

fn clamp(v: u32, lo: u32, hi: u32) -> u32 {
    v.max(lo).min(hi)
}

fn elapsed_ms(start: u32) -> u32 {
    sys::uptime_ms().wrapping_sub(start)
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

fn hex_str(bytes: &[u8; 32]) -> &str {
    core::str::from_utf8(bytes).unwrap_or("--------------------------------")
}

fn fill_loopback_pattern(offset: usize, out: &mut [u8]) {
    for (i, b) in out.iter_mut().enumerate() {
        let x = (offset + i) as u32;
        *b = (x.wrapping_mul(37).wrapping_add(x >> 7).wrapping_add(0x5a) & 0xff) as u8;
    }
}

fn loopback_md5() -> [u8; 32] {
    let mut body = Vec::new();
    let _ = body.try_reserve_exact(LOOPBACK_BODY_LEN);
    let mut chunk = [0u8; 1024];
    let mut offset = 0usize;
    while offset < LOOPBACK_BODY_LEN {
        let n = (LOOPBACK_BODY_LEN - offset).min(chunk.len());
        fill_loopback_pattern(offset, &mut chunk[..n]);
        body.extend_from_slice(&chunk[..n]);
        offset += n;
    }
    crypto::md5_hex(&body)
}
