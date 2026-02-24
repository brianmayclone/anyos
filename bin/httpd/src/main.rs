#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::ipc;
use anyos_std::net;
use anyos_std::process;
use anyos_std::println;
use anyos_std::fs;
use anyos_std::{String, Vec, format, vec};

// ─── Constants ──────────────────────────────────────────────────────

const CONFIG_DIR: &str = "/System/etc/httpd";
const SITES_DIR: &str = "/System/etc/httpd/sites";
const GLOBAL_CONFIG: &str = "/System/etc/httpd/httpd.conf";
const IPC_PIPE_NAME: &str = "httpd";
const SERVER_NAME: &str = "anyOS-httpd/1.0";
const MAX_REQUEST_SIZE: usize = 8192;
const MAX_RESPONSE_HEADER: usize = 1024;
const FILE_READ_BUF: usize = 4096;

// ─── Data Structures ────────────────────────────────────────────────

struct GlobalConfig {
    default_index: Vec<String>,
    log: bool,
}

struct RewriteRule {
    pattern: String,
    target: String,
    is_prefix: bool, // pattern ends with /*
}

struct SiteConfig {
    name: String,
    port: u16,
    root: String,
    index_files: Vec<String>,
    enabled: bool,
    rewrites: Vec<RewriteRule>,
}

struct WorkerInfo {
    port: u16,
    tid: u32,
}

// ─── Config Parsing ─────────────────────────────────────────────────

fn parse_global_config() -> GlobalConfig {
    let mut cfg = GlobalConfig {
        default_index: vec![String::from("index.html"), String::from("index.htm")],
        log: true,
    };

    if let Ok(content) = fs::read_to_string(GLOBAL_CONFIG) {
        for line in content.split('\n') {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(val) = line.strip_prefix("default_index=") {
                cfg.default_index.clear();
                for item in val.split(',') {
                    let item = item.trim();
                    if !item.is_empty() {
                        cfg.default_index.push(String::from(item));
                    }
                }
            } else if let Some(val) = line.strip_prefix("log=") {
                cfg.log = val.trim() == "true";
            }
        }
    }
    cfg
}

fn parse_rewrite_rule(line: &str) -> Option<RewriteRule> {
    // Format: "rewrite=/pattern /target"
    let val = line.strip_prefix("rewrite=")?;
    let val = val.trim();

    // Split on first space
    let space = val.find(' ')?;
    let pattern = val[..space].trim();
    let target = val[space + 1..].trim();
    if pattern.is_empty() || target.is_empty() {
        return None;
    }

    let is_prefix = pattern.ends_with("/*");
    let pattern_str = if is_prefix {
        String::from(&pattern[..pattern.len() - 2])
    } else {
        String::from(pattern)
    };
    let target_str = if is_prefix && target.ends_with("/*") {
        String::from(&target[..target.len() - 2])
    } else {
        String::from(target)
    };

    Some(RewriteRule {
        pattern: pattern_str,
        target: target_str,
        is_prefix,
    })
}

fn parse_site_config(content: &str, default_index: &[String]) -> Option<SiteConfig> {
    let mut site = SiteConfig {
        name: String::new(),
        port: 80,
        root: String::new(),
        index_files: Vec::new(),
        enabled: true,
        rewrites: Vec::new(),
    };

    for line in content.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(val) = line.strip_prefix("name=") {
            site.name = String::from(val.trim());
        } else if let Some(val) = line.strip_prefix("port=") {
            site.port = parse_u16(val.trim()).unwrap_or(80);
        } else if let Some(val) = line.strip_prefix("root=") {
            site.root = String::from(val.trim());
        } else if let Some(val) = line.strip_prefix("index=") {
            site.index_files.clear();
            for item in val.split(',') {
                let item = item.trim();
                if !item.is_empty() {
                    site.index_files.push(String::from(item));
                }
            }
        } else if let Some(val) = line.strip_prefix("enabled=") {
            site.enabled = val.trim() == "true";
        } else if line.starts_with("rewrite=") {
            if let Some(rule) = parse_rewrite_rule(line) {
                site.rewrites.push(rule);
            }
        }
    }

    if site.name.is_empty() || site.root.is_empty() {
        return None;
    }

    if site.index_files.is_empty() {
        site.index_files = default_index.to_vec();
    }

    Some(site)
}

fn load_all_sites(default_index: &[String]) -> Vec<SiteConfig> {
    let mut sites = Vec::new();
    let mut dir_buf = [0u8; 4096];
    let n = fs::readdir(SITES_DIR, &mut dir_buf);
    if n == u32::MAX {
        return sites;
    }

    let mut off = 0usize;
    for _ in 0..n as usize {
        if off + 64 > dir_buf.len() {
            break;
        }
        let entry_type = dir_buf[off];
        let name_len = dir_buf[off + 1] as usize;
        let name_bytes = &dir_buf[off + 8..off + 8 + name_len];

        if entry_type == 0 {
            // Regular file
            if let Ok(name) = core::str::from_utf8(name_bytes) {
                let path = format!("{}/{}", SITES_DIR, name);
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Some(site) = parse_site_config(&content, default_index) {
                        if site.enabled {
                            sites.push(site);
                        }
                    }
                }
            }
        }
        off += 64;
    }
    sites
}

// ─── MIME Types ─────────────────────────────────────────────────────

fn mime_type_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "txt" | "log" => "text/plain; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

// ─── HTTP Request Parsing ───────────────────────────────────────────

struct HttpRequest<'a> {
    method: &'a str,
    path: String,
    host: &'a str,
}

fn parse_request(buf: &[u8], len: usize) -> Option<HttpRequest<'_>> {
    let text = core::str::from_utf8(&buf[..len]).ok()?;

    // Parse request line: "GET /path HTTP/1.1\r\n"
    let first_line_end = text.find('\r').or_else(|| text.find('\n'))?;
    let first_line = &text[..first_line_end];
    let mut parts = first_line.split(' ');
    let method = parts.next()?;
    let raw_path = parts.next()?;

    // URL-decode path and strip query string
    let path_no_query = raw_path.split('?').next().unwrap_or(raw_path);
    let path = url_decode(path_no_query);

    // Parse Host header
    let mut host = "";
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if let Some(val) = line.strip_prefix("Host:").or_else(|| line.strip_prefix("host:")) {
            host = val.trim();
            // Strip port from host
            if let Some(colon) = host.rfind(':') {
                host = &host[..colon];
            }
            break;
        }
    }

    Some(HttpRequest { method, path, host })
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                result.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(result).unwrap_or_else(|_| String::from(s))
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ─── Rewrite Engine ─────────────────────────────────────────────────

fn apply_rewrites(path: &str, rules: &[RewriteRule]) -> String {
    for rule in rules {
        if rule.is_prefix {
            // Prefix match: /old/* → /new/*
            if path == rule.pattern || path.starts_with(&format!("{}/", rule.pattern)) {
                let suffix = &path[rule.pattern.len()..];
                return format!("{}{}", rule.target, suffix);
            }
        } else {
            // Exact match
            if path == rule.pattern {
                return rule.target.clone();
            }
        }
    }
    String::from(path)
}

// ─── Path Security ──────────────────────────────────────────────────

fn is_safe_path(path: &str) -> bool {
    // Block path traversal
    if path.contains("..") {
        return false;
    }
    // Must start with /
    if !path.starts_with('/') {
        return false;
    }
    // Block null bytes
    if path.contains('\0') {
        return false;
    }
    true
}

// ─── HTTP Response ──────────────────────────────────────────────────

fn send_error(sock: u32, code: u16, reason: &str) {
    let body = format!(
        "<!DOCTYPE html><html><head><title>{} {}</title></head>\
         <body><h1>{} {}</h1><hr><p>{}</p></body></html>",
        code, reason, code, reason, SERVER_NAME
    );
    let header = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Server: {}\r\n\
         Connection: close\r\n\r\n",
        code, reason, body.len(), SERVER_NAME
    );
    net::tcp_send(sock, header.as_bytes());
    net::tcp_send(sock, body.as_bytes());
}

fn send_redirect(sock: u32, location: &str) {
    let body = format!(
        "<!DOCTYPE html><html><head><title>301 Moved</title></head>\
         <body><h1>301 Moved Permanently</h1><p><a href=\"{}\">{}</a></p></body></html>",
        location, location
    );
    let header = format!(
        "HTTP/1.1 301 Moved Permanently\r\n\
         Location: {}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Server: {}\r\n\
         Connection: close\r\n\r\n",
        location, body.len(), SERVER_NAME
    );
    net::tcp_send(sock, header.as_bytes());
    net::tcp_send(sock, body.as_bytes());
}

fn serve_file(sock: u32, file_path: &str, file_size: u32) {
    let content_type = mime_type_for(file_path);
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Server: {}\r\n\
         Connection: close\r\n\r\n",
        content_type, file_size, SERVER_NAME
    );
    net::tcp_send(sock, header.as_bytes());

    // Stream file content in chunks
    let fd = fs::open(file_path, 0);
    if fd == u32::MAX {
        return;
    }
    let mut buf = [0u8; FILE_READ_BUF];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        let sent = net::tcp_send(sock, &buf[..n as usize]);
        if sent == u32::MAX {
            break;
        }
    }
    fs::close(fd);
}

// ─── Request Handler ────────────────────────────────────────────────

fn handle_request(sock: u32, sites: &[SiteConfig], port: u16) {
    let mut buf = [0u8; MAX_REQUEST_SIZE];
    let mut total = 0usize;

    // Read request (may arrive in multiple tcp_recv calls)
    loop {
        if total >= MAX_REQUEST_SIZE {
            break;
        }
        let n = net::tcp_recv(sock, &mut buf[total..]);
        if n == 0 || n == u32::MAX {
            if total == 0 {
                return; // No data at all
            }
            break;
        }
        total += n as usize;

        // Check if we have the full headers (ends with \r\n\r\n)
        if total >= 4 {
            let tail = &buf[..total];
            if tail.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
    }

    if total == 0 {
        return;
    }

    let request = match parse_request(&buf, total) {
        Some(r) => r,
        None => {
            send_error(sock, 400, "Bad Request");
            return;
        }
    };

    // Only support GET and HEAD
    if request.method != "GET" && request.method != "HEAD" {
        send_error(sock, 405, "Method Not Allowed");
        return;
    }

    // Find matching site for this port (first match wins; if multiple sites
    // share a port, match by Host header)
    let site = sites.iter().find(|s| {
        if s.port != port {
            return false;
        }
        // If there's only one site on this port, always match
        // Otherwise match by host header
        request.host.is_empty() || sites.iter().filter(|x| x.port == port).count() == 1
            || request.host.eq_ignore_ascii_case(&s.name)
    });

    let site = match site {
        Some(s) => s,
        None => {
            // Fallback: first site on this port
            match sites.iter().find(|s| s.port == port) {
                Some(s) => s,
                None => {
                    send_error(sock, 404, "Not Found");
                    return;
                }
            }
        }
    };

    // Apply rewrite rules
    let path = apply_rewrites(&request.path, &site.rewrites);

    // Security check
    if !is_safe_path(&path) {
        send_error(sock, 403, "Forbidden");
        return;
    }

    // Build filesystem path
    let mut fs_path = format!("{}{}", site.root, path);

    // Check if it's a directory
    let mut stat_buf = [0u32; 7];
    if fs::stat(&fs_path, &mut stat_buf) == 0 {
        if stat_buf[0] == 1 {
            // Directory — redirect if no trailing slash
            if !path.ends_with('/') {
                send_redirect(sock, &format!("{}/", path));
                return;
            }
            // Try index files
            let mut found = false;
            for idx in &site.index_files {
                let idx_path = format!("{}{}", fs_path, idx);
                if fs::stat(&idx_path, &mut stat_buf) == 0 && stat_buf[0] == 0 {
                    fs_path = idx_path;
                    found = true;
                    break;
                }
            }
            if !found {
                send_error(sock, 403, "Forbidden");
                return;
            }
        }
    } else {
        send_error(sock, 404, "Not Found");
        return;
    }

    // Verify it's a regular file and get size
    if fs::stat(&fs_path, &mut stat_buf) != 0 || stat_buf[0] != 0 {
        send_error(sock, 404, "Not Found");
        return;
    }

    let file_size = stat_buf[1];

    if request.method == "HEAD" {
        let content_type = mime_type_for(&fs_path);
        let header = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: {}\r\n\
             Content-Length: {}\r\n\
             Server: {}\r\n\
             Connection: close\r\n\r\n",
            content_type, file_size, SERVER_NAME
        );
        net::tcp_send(sock, header.as_bytes());
    } else {
        serve_file(sock, &fs_path, file_size);
    }
}

// ─── Worker Process ─────────────────────────────────────────────────

fn worker_main(port: u16, sites: Vec<SiteConfig>) -> ! {
    let listener = net::tcp_listen(port, 16);
    if listener == u32::MAX {
        println!("httpd: worker failed to listen on port {}", port);
        process::exit(1);
    }

    println!("httpd: worker listening on port {}", port);

    loop {
        let (sock, ip, rport) = net::tcp_accept(listener);
        if sock == u32::MAX {
            // Timeout — just retry
            continue;
        }

        handle_request(sock, &sites, port);
        net::tcp_close(sock);
    }
}

// ─── Master Process ─────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let args = process::args(&mut args_buf);

    // Check for --help
    if args.contains("--help") || args.contains("-h") {
        println!("Usage: httpd [--foreground]");
        println!("  HTTP web server daemon for anyOS.");
        println!("  Reads config from {}", CONFIG_DIR);
        println!("  Manage via: svc start httpd / svc stop httpd");
        return;
    }

    println!("httpd: starting...");

    // Load configuration
    let global_cfg = parse_global_config();
    let sites = load_all_sites(&global_cfg.default_index);

    if sites.is_empty() {
        println!("httpd: no enabled sites found in {}", SITES_DIR);
        println!("httpd: create a site config and try again");
        return;
    }

    // Collect unique ports
    let mut ports: Vec<u16> = Vec::new();
    for site in &sites {
        if !ports.contains(&site.port) {
            ports.push(site.port);
        }
    }

    println!("httpd: {} site(s) on {} port(s)", sites.len(), ports.len());
    for site in &sites {
        println!("  {} -> {}:{}", site.name, site.root, site.port);
    }

    // Create IPC pipe for management commands
    let pipe_id = ipc::pipe_create(IPC_PIPE_NAME);

    // Fork a worker for each unique port
    let mut workers: Vec<WorkerInfo> = Vec::new();

    for &port in &ports {
        // Collect sites for this port
        let port_sites: Vec<SiteConfig> = sites
            .iter()
            .filter(|s| s.port == port)
            .map(|s| SiteConfig {
                name: s.name.clone(),
                port: s.port,
                root: s.root.clone(),
                index_files: s.index_files.clone(),
                enabled: s.enabled,
                rewrites: s.rewrites.iter().map(|r| RewriteRule {
                    pattern: r.pattern.clone(),
                    target: r.target.clone(),
                    is_prefix: r.is_prefix,
                }).collect(),
            })
            .collect();

        let tid = process::fork();
        if tid == 0 {
            // Child process — run worker
            worker_main(port, port_sites);
        } else {
            // Parent — record worker
            workers.push(WorkerInfo { port, tid });
            println!("httpd: forked worker pid {} for port {}", tid, port);
        }
    }

    println!("httpd: ready ({} worker(s))", workers.len());

    // Master loop: handle IPC commands
    let mut cmd_buf = [0u8; 256];
    loop {
        let n = ipc::pipe_read(pipe_id, &mut cmd_buf);
        if n > 0 && n != u32::MAX {
            if let Ok(cmd) = core::str::from_utf8(&cmd_buf[..n as usize]) {
                let cmd = cmd.trim();
                match cmd {
                    "stop" => {
                        println!("httpd: stopping all workers...");
                        for w in &workers {
                            process::kill(w.tid);
                        }
                        ipc::pipe_close(pipe_id);
                        println!("httpd: stopped");
                        return;
                    }
                    "reload" => {
                        println!("httpd: reload requested (restart workers)");
                        // Kill old workers
                        for w in &workers {
                            process::kill(w.tid);
                        }
                        workers.clear();

                        // Reload config
                        let global_cfg = parse_global_config();
                        let sites = load_all_sites(&global_cfg.default_index);
                        let mut new_ports: Vec<u16> = Vec::new();
                        for site in &sites {
                            if !new_ports.contains(&site.port) {
                                new_ports.push(site.port);
                            }
                        }

                        // Fork new workers
                        for &port in &new_ports {
                            let port_sites: Vec<SiteConfig> = sites
                                .iter()
                                .filter(|s| s.port == port)
                                .map(|s| SiteConfig {
                                    name: s.name.clone(),
                                    port: s.port,
                                    root: s.root.clone(),
                                    index_files: s.index_files.clone(),
                                    enabled: s.enabled,
                                    rewrites: s.rewrites.iter().map(|r| RewriteRule {
                                        pattern: r.pattern.clone(),
                                        target: r.target.clone(),
                                        is_prefix: r.is_prefix,
                                    }).collect(),
                                })
                                .collect();

                            let tid = process::fork();
                            if tid == 0 {
                                worker_main(port, port_sites);
                            } else {
                                workers.push(WorkerInfo { port, tid });
                            }
                        }
                        println!("httpd: reloaded ({} workers)", workers.len());
                    }
                    "status" => {
                        // Write status back to pipe
                        let status = format!(
                            "running workers={} ports={:?}",
                            workers.len(),
                            workers.iter().map(|w| w.port).collect::<Vec<_>>()
                        );
                        ipc::pipe_write(pipe_id, status.as_bytes());
                    }
                    _ => {
                        println!("httpd: unknown command: {}", cmd);
                    }
                }
            }
        }

        // Check if workers are still alive
        for w in &workers {
            let ret = process::try_waitpid(w.tid);
            if ret != u32::MAX {
                println!("httpd: worker for port {} (pid {}) exited", w.port, w.tid);
            }
        }

        process::sleep(500);
    }
}

// ─── Utilities ──────────────────────────────────────────────────────

fn parse_u16(s: &str) -> Option<u16> {
    let mut val: u32 = 0;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val * 10 + (b - b'0') as u32;
        if val > 65535 {
            return None;
        }
    }
    Some(val as u16)
}
