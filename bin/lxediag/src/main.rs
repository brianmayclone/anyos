#![no_std]
#![no_main]

use alloc::format;
use anyos_std::{fs, net, println, process, sys, String, Vec};

anyos_std::entry!(main);

const VERSION: &str = "0.1";
const DEFAULT_URL: &str = "http://deb.debian.org/debian/dists/bookworm/InRelease";
const DEFAULT_OUT_DIR: &str = "/System/var/lxe/rootfs/tmp";
const CONNECT_TIMEOUT_MS: u32 = 10_000;
const RECV_BUF_SIZE: usize = 32 * 1024;
const MAX_HEADER_SIZE: usize = 32 * 1024;
const MAX_BODY_SIZE: usize = 32 * 1024 * 1024;
const TAIL_RETRIES: u32 = 40;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

struct Config {
    repeat: u32,
    url: String,
    out_dir: String,
    write_disk: bool,
}

struct Url {
    host: String,
    port: u16,
    path: String,
}

struct DownloadResult {
    round: u32,
    status: u16,
    bytes: usize,
    expected: Option<u32>,
    net_hash: u64,
    disk_hash: u64,
    disk_bytes: usize,
    ms: u32,
    error: &'static str,
}

fn main() {
    let cfg = match parse_args() {
        Some(cfg) => cfg,
        None => return,
    };
    let url = match parse_url(&cfg.url) {
        Some(url) => url,
        None => {
            println!("lxediag: nur http://host[:port]/path wird unterstuetzt");
            return;
        }
    };
    let mut ip = [0u8; 4];
    if net::dns(&url.host, &mut ip) != 0 {
        println!("lxediag: DNS fehlgeschlagen fuer {}", url.host);
        return;
    }

    if cfg.write_disk {
        let _ = fs::mkdir(&cfg.out_dir);
    }

    println!("lxediag {} - LXE apt byte-path diagnostic", VERSION);
    println!(
        "url={} host={}.{}.{}.{} out={}",
        cfg.url, ip[0], ip[1], ip[2], ip[3], cfg.out_dir
    );
    println!("stages: net-download -> anyOS-write/fsync -> reopen/read");
    println!();

    let mut results: Vec<DownloadResult> = Vec::new();
    for round in 1..=cfg.repeat {
        let result = run_round(round, &url, &ip, &cfg);
        print_result(&result);
        results.push(result);
        process::sleep(250);
    }

    println!();
    summarize(&results, cfg.write_disk);
}

fn parse_args() -> Option<Config> {
    let mut raw_buf = [0u8; 768];
    let raw = process::args(&mut raw_buf);
    if raw.contains("--help") || raw.contains("-h") {
        println!("Usage: lxediag [--repeat N] [--url URL] [--out-dir PATH] [--no-write]");
        println!("Default URL: {}", DEFAULT_URL);
        return None;
    }

    let mut cfg = Config {
        repeat: 3,
        url: String::from(DEFAULT_URL),
        out_dir: String::from(DEFAULT_OUT_DIR),
        write_disk: true,
    };
    let args: Vec<&str> = raw.split_ascii_whitespace().collect();
    let mut i = 0usize;
    while i < args.len() {
        match args[i] {
            "--repeat" | "-r" => {
                i += 1;
                if i >= args.len() {
                    println!("lxediag: --repeat braucht eine Zahl");
                    return None;
                }
                cfg.repeat = parse_u32(args[i]).unwrap_or(cfg.repeat).clamp(1, 50);
            }
            "--url" | "-u" => {
                i += 1;
                if i >= args.len() {
                    println!("lxediag: --url braucht eine URL");
                    return None;
                }
                cfg.url = String::from(args[i]);
            }
            "--out-dir" | "-o" => {
                i += 1;
                if i >= args.len() {
                    println!("lxediag: --out-dir braucht einen Pfad");
                    return None;
                }
                cfg.out_dir = String::from(args[i]);
            }
            "--no-write" => cfg.write_disk = false,
            other => {
                println!("lxediag: unbekannte Option '{}'", other);
                return None;
            }
        }
        i += 1;
    }
    Some(cfg)
}

fn run_round(round: u32, url: &Url, ip: &[u8; 4], cfg: &Config) -> DownloadResult {
    let start = sys::uptime_ms();
    let (mut result, body) = match download(url, ip) {
        Ok((status, expected, body, hash)) => (
            DownloadResult {
                round,
                status,
                bytes: body.len(),
                expected,
                net_hash: hash,
                disk_hash: 0,
                disk_bytes: 0,
                ms: elapsed_ms(start),
                error: "ok",
            },
            body,
        ),
        Err(error) => {
            return DownloadResult {
                round,
                status: 0,
                bytes: 0,
                expected: None,
                net_hash: 0,
                disk_hash: 0,
                disk_bytes: 0,
                ms: elapsed_ms(start),
                error,
            };
        }
    };

    if result.status != 200 {
        result.error = "http-status";
        return result;
    }
    if let Some(expected) = result.expected {
        if result.bytes != expected as usize {
            result.error = "short-body";
            return result;
        }
    }
    if !cfg.write_disk {
        return result;
    }

    let path = format!("{}/lxediag-InRelease-{}.bin", cfg.out_dir, round);
    match write_and_read_back(&path, &body) {
        Ok((disk_bytes, disk_hash)) => {
            result.disk_bytes = disk_bytes;
            result.disk_hash = disk_hash;
            if disk_bytes != result.bytes || disk_hash != result.net_hash {
                result.error = "disk-mismatch";
            }
        }
        Err(error) => result.error = error,
    }
    result.ms = elapsed_ms(start);
    result
}

fn download(url: &Url, ip: &[u8; 4]) -> Result<(u16, Option<u32>, Vec<u8>, u64), &'static str> {
    let sock = net::tcp_connect(ip, url.port, CONNECT_TIMEOUT_MS);
    if sock == u32::MAX {
        return Err("connect");
    }
    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: lxediag/{}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        url.path, url.host, VERSION
    );
    if !send_all(sock, request.as_bytes()) {
        let _ = net::tcp_close(sock);
        return Err("send");
    }

    let mut recv_buf = Vec::new();
    if recv_buf.try_reserve_exact(RECV_BUF_SIZE).is_err() {
        let _ = net::tcp_close(sock);
        return Err("alloc");
    }
    recv_buf.resize(RECV_BUF_SIZE, 0);

    let mut header = Vec::new();
    let header_end;
    loop {
        let n = recv_some(sock, &mut recv_buf);
        if n == 0 {
            let _ = net::tcp_close(sock);
            return Err("header");
        }
        header.extend_from_slice(&recv_buf[..n]);
        if let Some(end) = find_header_end(&header) {
            header_end = end;
            break;
        }
        if header.len() > MAX_HEADER_SIZE {
            let _ = net::tcp_close(sock);
            return Err("header-large");
        }
    }

    let headers = core::str::from_utf8(&header[..header_end]).unwrap_or("");
    let status = parse_status(headers);
    let expected = parse_content_length(headers);
    let mut body = Vec::new();
    let cap = expected
        .map(|n| n as usize)
        .unwrap_or(RECV_BUF_SIZE)
        .min(MAX_BODY_SIZE);
    let _ = body.try_reserve_exact(cap);
    let mut hash = FNV_OFFSET;

    if header_end < header.len() {
        let trailing = &header[header_end..];
        body.extend_from_slice(trailing);
        hash = fnv_update(hash, trailing);
    }

    let mut tail_retries = 0u32;
    loop {
        if let Some(expected) = expected {
            if body.len() >= expected as usize {
                break;
            }
        }
        if body.len() >= MAX_BODY_SIZE {
            let _ = net::tcp_close(sock);
            return Err("body-large");
        }

        let n = recv_some(sock, &mut recv_buf);
        if n == 0 {
            if let Some(expected) = expected {
                if body.len() < expected as usize && tail_retries < TAIL_RETRIES {
                    tail_retries += 1;
                    process::sleep(100);
                    continue;
                }
            }
            break;
        }
        tail_retries = 0;
        let take = if let Some(expected) = expected {
            n.min((expected as usize).saturating_sub(body.len()))
        } else {
            n
        };
        hash = fnv_update(hash, &recv_buf[..take]);
        body.extend_from_slice(&recv_buf[..take]);
    }
    let _ = net::tcp_close(sock);
    Ok((status, expected, body, hash))
}

fn write_and_read_back(path: &str, body: &[u8]) -> Result<(usize, u64), &'static str> {
    let fd = fs::open(path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return Err("open-write");
    }
    let mut written = 0usize;
    while written < body.len() {
        let end = (written + 16 * 1024).min(body.len());
        let n = fs::write(fd, &body[written..end]);
        if n == u32::MAX || n == 0 {
            let _ = fs::close(fd);
            return Err("write");
        }
        written += n as usize;
    }
    if !fs::fsync(fd as i32) {
        let _ = fs::close(fd);
        return Err("fsync");
    }
    let _ = fs::close(fd);
    fs::sync();
    read_file_hash(path)
}

fn read_file_hash(path: &str) -> Result<(usize, u64), &'static str> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return Err("open-read");
    }
    let mut buf = [0u8; 16 * 1024];
    let mut total = 0usize;
    let mut hash = FNV_OFFSET;
    loop {
        let n = fs::read(fd, &mut buf);
        if n == u32::MAX {
            let _ = fs::close(fd);
            return Err("readback");
        }
        if n == 0 {
            break;
        }
        let n = n as usize;
        total += n;
        hash = fnv_update(hash, &buf[..n]);
    }
    let _ = fs::close(fd);
    Ok((total, hash))
}

fn print_result(r: &DownloadResult) {
    println!(
        "#{:02} status={} net={} hash={:016x} disk={} disk_hash={:016x} ms={} {}",
        r.round, r.status, r.bytes, r.net_hash, r.disk_bytes, r.disk_hash, r.ms, r.error
    );
}

fn summarize(results: &[DownloadResult], write_disk: bool) {
    let ok: Vec<&DownloadResult> = results.iter().filter(|r| r.error == "ok").collect();
    if ok.is_empty() {
        println!("DIAG: kein kompletter erfolgreicher Lauf; erster Fehler liegt vor oder in der gemeldeten Stage.");
        return;
    }
    let first = ok[0];
    let mut net_stable = true;
    let mut disk_stable = true;
    for r in ok.iter() {
        if r.bytes != first.bytes || r.net_hash != first.net_hash {
            net_stable = false;
        }
        if write_disk && (r.disk_bytes != r.bytes || r.disk_hash != r.net_hash) {
            disk_stable = false;
        }
    }

    if !net_stable {
        println!("DIAG: Netzwerk/TCP liefert inkonsistente Bytes schon vor dem Disk-Write.");
    } else if write_disk && !disk_stable {
        println!(
            "DIAG: Netzwerk stabil, aber Write/fsync/reopen/readback veraendert Bytes oder Laenge."
        );
    } else {
        println!(
            "DIAG: nativer Netzwerk- und Disk-Pfad stabil; naechster Fokus: LXE-Syscalls/Loader."
        );
    }
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
    let _ = parts.next();
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

fn parse_u32(s: &str) -> Option<u32> {
    let mut val = 0u32;
    let mut any = false;
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
                any = true;
            }
            b' ' | b'\r' | b'\n' | b'\t' if any => break,
            _ => return None,
        }
    }
    if any {
        Some(val)
    } else {
        None
    }
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

fn fnv_update(mut hash: u64, data: &[u8]) -> u64 {
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn elapsed_ms(start: u32) -> u32 {
    sys::uptime_ms().wrapping_sub(start)
}
