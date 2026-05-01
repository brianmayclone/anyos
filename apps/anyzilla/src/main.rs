// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
#![no_std]
#![no_main]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyos_std::json::Value;
use anyos_std::{env, fs, net};
use anyui::{IconType, Widget};
use libanyui_client as anyui;
use libconf_schema::{default_int, default_string, manifest, RegistryScope, ServiceSchema};

anyos_std::entry!(main);

// ─── FTP Constants ────────────────────────────────────────────────────────────

const FTP_PORT: u16 = 21;
const CONNECT_TIMEOUT: u32 = 10000;
const RECV_BUF: usize = 4096;

// ─── Data Structures ──────────────────────────────────────────────────────────

#[derive(Clone)]
struct FileEntry {
    name: String,
    size: u64,
    is_dir: bool,
    modified: String,
}

const SITES_FILE: &str = ".anyzilla_sites.json";
const PREFS_FILE: &str = ".anyzilla_prefs.json";
const ANYZILLA_DIRS: &[&str] = &["config"];
const ANYZILLA_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("config/sites_json", ""),
    default_string("config/prefs_json", ""),
    default_int("config/win_x", -1),
    default_int("config/win_y", -1),
    default_int("config/win_w", 1100),
    default_int("config/win_h", 680),
];
const ANYZILLA_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const ANYZILLA_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "apps/anyzilla",
    RegistryScope::User,
    1,
    ANYZILLA_DIRS,
    ANYZILLA_DEFAULTS,
    ANYZILLA_MIGRATIONS,
);

fn config_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("anyzilla", &ANYZILLA_MANIFEST)
}

#[derive(Clone, Copy, PartialEq)]
enum SortColumn {
    Name,
    Size,
    Date,
}

#[derive(Clone, Copy, PartialEq)]
enum SortOrder {
    Asc,
    Desc,
}

#[derive(Clone)]
struct SiteProfile {
    name: String,
    host: String,
    port: u16,
    user: String,
    pass: String,
    remote_dir: String,
}

impl SiteProfile {
    fn to_json(&self) -> Value {
        let mut obj = Value::new_object();
        obj.set("name", Value::from(self.name.as_str()));
        obj.set("host", Value::from(self.host.as_str()));
        obj.set("port", (self.port as i64).into());
        obj.set("user", Value::from(self.user.as_str()));
        obj.set("pass", Value::from(self.pass.as_str()));
        if !self.remote_dir.is_empty() {
            obj.set("remote_dir", Value::from(self.remote_dir.as_str()));
        }
        obj
    }

    fn from_json(val: &Value) -> Option<SiteProfile> {
        let name = val["name"].as_str()?.to_string();
        let host = val["host"].as_str()?.to_string();
        let port = val["port"].as_i64().unwrap_or(21) as u16;
        let user = val["user"].as_str().unwrap_or("anonymous").to_string();
        let pass = val["pass"].as_str().unwrap_or("").to_string();
        let remote_dir = val["remote_dir"].as_str().unwrap_or("").to_string();
        if host.is_empty() {
            return None;
        }
        Some(SiteProfile {
            name,
            host,
            port,
            user,
            pass,
            remote_dir,
        })
    }
}

fn sites_path() -> String {
    let home = get_home_dir();
    format!("{}/{}", home, SITES_FILE)
}

fn load_sites() -> Vec<SiteProfile> {
    let _ = config_schema().register();
    if let Some(text) = config_schema().read_string("config/sites_json") {
        if !text.trim().is_empty() {
            return parse_sites_json(&text);
        }
    }
    let path = sites_path();
    let text = read_legacy_json(&path);
    let sites = parse_sites_json(&text);
    if !sites.is_empty() {
        save_sites(&sites);
    }
    sites
}

fn parse_sites_json(text: &str) -> Vec<SiteProfile> {
    let val = match Value::parse(text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut sites = Vec::new();
    if let Some(arr) = val["sites"].as_array() {
        for item in arr {
            if let Some(site) = SiteProfile::from_json(item) {
                sites.push(site);
            }
        }
    }
    sites
}

fn save_sites(sites: &[SiteProfile]) {
    let _ = config_schema().register();
    let mut root = Value::new_object();
    let mut arr = Value::new_array();
    for site in sites {
        arr.push(site.to_json());
    }
    root.set("sites", arr);
    let json = root.to_json_string_pretty();
    let _ = config_schema().write_string("config/sites_json", &json);
}

struct Prefs {
    win_x: i32,
    win_y: i32,
    win_w: u32,
    win_h: u32,
    last_host: String,
    last_port: u16,
    last_user: String,
    last_pass: String,
}

fn prefs_path() -> String {
    let home = get_home_dir();
    format!("{}/{}", home, PREFS_FILE)
}

fn load_prefs() -> Prefs {
    let _ = config_schema().register();
    if let Some(text) = config_schema().read_string("config/prefs_json") {
        if !text.trim().is_empty() {
            return parse_prefs_json(&text);
        }
    }
    let path = prefs_path();
    let prefs = parse_prefs_json(&read_legacy_json(&path));
    if prefs.win_x != -1 || !prefs.last_host.is_empty() {
        save_prefs(&prefs);
    }
    prefs
}

fn parse_prefs_json(text: &str) -> Prefs {
    let val = match Value::parse(text.trim()) {
        Ok(v) => v,
        Err(_) => return default_prefs(),
    };
    let mut p = Prefs {
        win_x: val["win_x"].as_i64().unwrap_or(-1) as i32,
        win_y: val["win_y"].as_i64().unwrap_or(-1) as i32,
        win_w: val["win_w"].as_i64().unwrap_or(1100) as u32,
        win_h: val["win_h"].as_i64().unwrap_or(680) as u32,
        last_host: val["last_host"].as_str().unwrap_or("").to_string(),
        last_port: val["last_port"].as_i64().unwrap_or(21) as u16,
        last_user: val["last_user"].as_str().unwrap_or("").to_string(),
        last_pass: val["last_pass"].as_str().unwrap_or("").to_string(),
    };
    // Guard against 0×0 window (can happen if prefs were saved during teardown)
    if p.win_w < 200 {
        p.win_w = 1100;
    }
    if p.win_h < 100 {
        p.win_h = 680;
    }
    p
}

fn save_prefs(p: &Prefs) {
    let _ = config_schema().register();
    let mut root = Value::new_object();
    root.set("win_x", (p.win_x as i64).into());
    root.set("win_y", (p.win_y as i64).into());
    root.set("win_w", (p.win_w as i64).into());
    root.set("win_h", (p.win_h as i64).into());
    if !p.last_host.is_empty() {
        root.set("last_host", Value::from(p.last_host.as_str()));
        root.set("last_port", (p.last_port as i64).into());
        root.set("last_user", Value::from(p.last_user.as_str()));
        root.set("last_pass", Value::from(p.last_pass.as_str()));
    }
    let json = root.to_json_string_pretty();
    let _ = config_schema().write_string("config/prefs_json", &json);
    let _ = config_schema().write_i64("config/win_x", p.win_x as i64);
    let _ = config_schema().write_i64("config/win_y", p.win_y as i64);
    let _ = config_schema().write_i64("config/win_w", p.win_w as i64);
    let _ = config_schema().write_i64("config/win_h", p.win_h as i64);
}

fn default_prefs() -> Prefs {
    Prefs {
        win_x: -1,
        win_y: -1,
        win_w: 1100,
        win_h: 680,
        last_host: String::new(),
        last_port: 21,
        last_user: String::new(),
        last_pass: String::new(),
    }
}

fn read_legacy_json(path: &str) -> String {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return String::new();
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    core::str::from_utf8(&data).unwrap_or("").to_string()
}

fn sort_entries(entries: &mut [FileEntry], col: SortColumn, order: SortOrder) {
    entries.sort_by(|a, b| {
        // ".." always first
        if a.name == ".." {
            return core::cmp::Ordering::Less;
        }
        if b.name == ".." {
            return core::cmp::Ordering::Greater;
        }
        // Directories before files
        match (a.is_dir, b.is_dir) {
            (true, false) => return core::cmp::Ordering::Less,
            (false, true) => return core::cmp::Ordering::Greater,
            _ => {}
        }
        let cmp = match col {
            SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortColumn::Size => a.size.cmp(&b.size),
            SortColumn::Date => a.modified.cmp(&b.modified),
        };
        match order {
            SortOrder::Asc => cmp,
            SortOrder::Desc => cmp.reverse(),
        }
    });
}

struct FtpClient {
    ctrl: u32,
}

// ─── FTP Protocol ─────────────────────────────────────────────────────────────

impl FtpClient {
    fn connect(ip: &[u8; 4], port: u16) -> Option<FtpClient> {
        let sock = net::tcp_connect(ip, port, CONNECT_TIMEOUT);
        if sock == u32::MAX {
            return None;
        }
        let mut client = FtpClient { ctrl: sock };
        let resp = client.read_response();
        if !resp.starts_with("220") {
            net::tcp_close(sock);
            return None;
        }
        Some(client)
    }

    fn login(&mut self, user: &str, pass: &str) -> bool {
        self.send_command("USER ", user);
        let resp = self.read_response();
        if resp.starts_with("230") {
            return true;
        }
        if !resp.starts_with("331") {
            return false;
        }
        self.send_command("PASS ", pass);
        let resp = self.read_response();
        resp.starts_with("230")
    }

    fn send_command(&mut self, cmd: &str, arg: &str) {
        let mut buf = Vec::with_capacity(cmd.len() + arg.len() + 2);
        buf.extend_from_slice(cmd.as_bytes());
        buf.extend_from_slice(arg.as_bytes());
        buf.push(b'\r');
        buf.push(b'\n');
        net::tcp_send(self.ctrl, &buf);
    }

    fn send_cmd_only(&mut self, cmd: &str) {
        let mut buf = Vec::with_capacity(cmd.len() + 2);
        buf.extend_from_slice(cmd.as_bytes());
        buf.push(b'\r');
        buf.push(b'\n');
        net::tcp_send(self.ctrl, &buf);
    }

    fn read_response(&mut self) -> String {
        let mut result = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = net::tcp_recv(self.ctrl, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            result.extend_from_slice(&buf[..n as usize]);
            if is_complete_response(&result) {
                break;
            }
        }
        String::from_utf8_lossy(&result).into_owned()
    }

    /// Send PASV and return (ip, port) without connecting yet.
    fn pasv_addr(&mut self) -> Option<([u8; 4], u16)> {
        self.send_cmd_only("PASV");
        let resp = self.read_response();
        if !resp.starts_with("227") {
            return None;
        }
        parse_pasv(&resp)
    }

    fn list_dir(&mut self) -> Vec<FileEntry> {
        self.list_dir_ex(false)
    }

    fn list_dir_ex(&mut self, show_hidden: bool) -> Vec<FileEntry> {
        let (ip, port) = match self.pasv_addr() {
            Some(v) => v,
            None => return Vec::new(),
        };
        // Send command first, then open data connection (RFC 959 / server expects this order).
        if show_hidden {
            self.send_cmd_only("LIST -a");
        } else {
            self.send_cmd_only("LIST");
        }
        let data_sock = net::tcp_connect(&ip, port, CONNECT_TIMEOUT);
        if data_sock == u32::MAX {
            return Vec::new();
        }
        let resp = self.read_response();
        if !resp.starts_with("150") && !resp.starts_with("125") {
            net::tcp_close(data_sock);
            return Vec::new();
        }
        let mut raw = Vec::new();
        let mut buf = [0u8; RECV_BUF];
        loop {
            let n = net::tcp_recv(data_sock, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            raw.extend_from_slice(&buf[..n as usize]);
        }
        net::tcp_close(data_sock);
        let _ = self.read_response();

        let text = String::from_utf8_lossy(&raw).into_owned();
        let mut entries = Vec::new();
        entries.push(FileEntry {
            name: "..".into(),
            size: 0,
            is_dir: true,
            modified: "".into(),
        });
        for line in text.lines() {
            if let Some(e) = parse_list_line(line) {
                if e.name != "." && e.name != ".." {
                    entries.push(e);
                }
            }
        }
        entries
    }

    fn pwd(&mut self) -> String {
        self.send_cmd_only("PWD");
        let resp = self.read_response();
        if let Some(start) = resp.find('"') {
            if let Some(end) = resp[start + 1..].find('"') {
                return resp[start + 1..start + 1 + end].to_string();
            }
        }
        "/".to_string()
    }

    fn cd(&mut self, path: &str) -> bool {
        if path == ".." {
            self.send_cmd_only("CDUP");
        } else {
            self.send_command("CWD ", path);
        }
        let resp = self.read_response();
        resp.starts_with("200") || resp.starts_with("250")
    }

    fn mkdir(&mut self, name: &str) -> bool {
        self.send_command("MKD ", name);
        let resp = self.read_response();
        resp.starts_with("257")
    }

    fn delete_file(&mut self, name: &str) -> bool {
        self.send_command("DELE ", name);
        let resp = self.read_response();
        resp.starts_with("250")
    }

    fn delete_dir(&mut self, name: &str) -> bool {
        self.send_command("RMD ", name);
        let resp = self.read_response();
        resp.starts_with("250")
    }

    fn rename(&mut self, old: &str, new_name: &str) -> bool {
        self.send_command("RNFR ", old);
        let resp = self.read_response();
        if !resp.starts_with("350") {
            return false;
        }
        self.send_command("RNTO ", new_name);
        let resp = self.read_response();
        resp.starts_with("250")
    }

    fn download(&mut self, remote_name: &str, local_path: &str) -> u32 {
        self.set_binary_mode();
        let (ip, port) = match self.pasv_addr() {
            Some(v) => v,
            None => return 0,
        };
        self.send_command("RETR ", remote_name);
        let data_sock = net::tcp_connect(&ip, port, CONNECT_TIMEOUT);
        if data_sock == u32::MAX {
            return 0;
        }
        let resp = self.read_response();
        if !resp.starts_with("150") && !resp.starts_with("125") {
            net::tcp_close(data_sock);
            return 0;
        }
        let fd = fs::open(local_path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            net::tcp_close(data_sock);
            return 0;
        }
        let mut total = 0u32;
        let mut buf = [0u8; RECV_BUF];
        loop {
            let n = net::tcp_recv(data_sock, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            fs::write(fd, &buf[..n as usize]);
            total += n;
        }
        fs::close(fd);
        net::tcp_close(data_sock);
        let _ = self.read_response();
        total
    }

    fn upload(&mut self, local_path: &str, remote_name: &str) -> u32 {
        self.set_binary_mode();
        let fd = fs::open(local_path, 0);
        if fd == u32::MAX {
            return 0;
        }
        let mut file_data = Vec::new();
        let mut buf = [0u8; RECV_BUF];
        loop {
            let n = fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            file_data.extend_from_slice(&buf[..n as usize]);
        }
        fs::close(fd);
        let (ip, port) = match self.pasv_addr() {
            Some(v) => v,
            None => {
                return 0;
            }
        };
        self.send_command("STOR ", remote_name);
        let data_sock = net::tcp_connect(&ip, port, CONNECT_TIMEOUT);
        if data_sock == u32::MAX {
            return 0;
        }
        let resp = self.read_response();
        if !resp.starts_with("150") && !resp.starts_with("125") {
            net::tcp_close(data_sock);
            return 0;
        }
        let total = file_data.len() as u32;
        let mut offset = 0;
        while offset < file_data.len() {
            let end = (offset + 1460).min(file_data.len());
            let sent = net::tcp_send(data_sock, &file_data[offset..end]);
            if sent == u32::MAX {
                break;
            }
            offset = end;
        }
        net::tcp_close(data_sock);
        let _ = self.read_response();
        total
    }

    fn set_binary_mode(&mut self) {
        self.send_cmd_only("TYPE I");
        let _ = self.read_response();
    }

    fn disconnect(&mut self) {
        self.send_cmd_only("QUIT");
        let _ = self.read_response();
        net::tcp_close(self.ctrl);
    }
}

// ─── FTP Helpers ──────────────────────────────────────────────────────────────

fn is_complete_response(data: &[u8]) -> bool {
    let s = core::str::from_utf8(data).unwrap_or("");
    for line in s.lines() {
        if line.len() >= 4 {
            let code_ok = line.as_bytes()[0..3].iter().all(|b| b.is_ascii_digit());
            if code_ok && line.as_bytes()[3] == b' ' {
                return true;
            }
        }
    }
    false
}

fn parse_pasv(resp: &str) -> Option<([u8; 4], u16)> {
    let start = resp.find('(')? + 1;
    let end = resp.find(')')?;
    let inner = &resp[start..end];
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() < 6 {
        return None;
    }
    let ip = [
        parts[0].trim().parse::<u8>().ok()?,
        parts[1].trim().parse::<u8>().ok()?,
        parts[2].trim().parse::<u8>().ok()?,
        parts[3].trim().parse::<u8>().ok()?,
    ];
    let p1: u16 = parts[4].trim().parse().ok()?;
    let p2: u16 = parts[5].trim().parse().ok()?;
    Some((ip, (p1 << 8) | p2))
}

fn parse_list_line(line: &str) -> Option<FileEntry> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 9 {
        return None;
    }
    let is_dir = parts[0].starts_with('d');
    let size: u64 = parts[4].parse().ok()?;
    let modified = format!("{} {} {}", parts[5], parts[6], parts[7]);
    let name = parts[8..].join(" ");
    if name.is_empty() {
        return None;
    }
    Some(FileEntry {
        name,
        size,
        is_dir,
        modified,
    })
}

fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "-".to_string();
    }
    if bytes >= 1024 * 1024 * 1024 {
        let gb = bytes / (1024 * 1024 * 1024);
        let rem = (bytes % (1024 * 1024 * 1024)) / (1024 * 1024 / 10);
        format!("{}.{} GB", gb, rem % 10)
    } else if bytes >= 1024 * 1024 {
        let mb = bytes / (1024 * 1024);
        let rem = (bytes % (1024 * 1024)) / (1024 / 10);
        format!("{}.{} MB", mb, rem % 10)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}

// ─── File Type Icons (14×14 ARGB) ─────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum FileIcon {
    ParentDir,
    Folder,
    Text,
    Image,
    Archive,
    Executable,
    Audio,
    Video,
    Default,
}

fn icon_for_name(name: &str, is_dir: bool) -> FileIcon {
    if name == ".." {
        return FileIcon::ParentDir;
    }
    if is_dir {
        return FileIcon::Folder;
    }
    let ext = match name.rfind('.') {
        Some(i) => &name[i + 1..],
        None => "",
    };
    match ext {
        "txt" | "md" | "log" | "cfg" | "conf" | "ini" | "csv" | "json" | "xml" | "yml" | "yaml"
        | "toml" => FileIcon::Text,
        "rs" | "c" | "h" | "cpp" | "hpp" | "py" | "js" | "ts" | "java" | "go" | "sh" | "bash"
        | "rb" | "php" | "css" | "html" | "htm" | "sql" | "asm" | "s" | "S" => FileIcon::Text,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "svg" | "webp" | "tga" | "tiff" => {
            FileIcon::Image
        }
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" | "deb" | "rpm" => {
            FileIcon::Archive
        }
        "exe" | "elf" | "bin" | "com" | "app" | "out" => FileIcon::Executable,
        "mp3" | "wav" | "ogg" | "flac" | "aac" | "wma" | "m4a" => FileIcon::Audio,
        "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => FileIcon::Video,
        _ => FileIcon::Default,
    }
}

/// Generate a 14×14 ARGB icon for the given file type.
fn generate_icon(icon: FileIcon) -> [u32; 196] {
    const W: usize = 14;
    const H: usize = 14;
    let mut px = [0u32; W * H];

    match icon {
        FileIcon::ParentDir => {
            // Gray up-arrow
            let c = 0xFF888888u32;
            // Arrow shaft: col 6-7, rows 4-12
            for y in 4..12 {
                px[y * W + 6] = c;
                px[y * W + 7] = c;
            }
            // Arrow head: row 2-6
            for i in 0..5u32 {
                let y = (2 + i) as usize;
                let left = 7i32 - i as i32 - 1;
                let right = 6i32 + i as i32 + 1;
                if left >= 0 {
                    px[y * W + left as usize] = c;
                }
                if (right as usize) < W {
                    px[y * W + right as usize] = c;
                }
            }
        }
        FileIcon::Folder => {
            let c = 0xFFDDAA44u32;
            let cd = 0xFFBB8822u32;
            // Tab: row 2, cols 1-5
            for x in 1..6 {
                px[2 * W + x] = c;
            }
            // Top edge: row 3, cols 1-12
            for x in 1..13 {
                px[3 * W + x] = c;
            }
            // Body: rows 4-11
            for y in 4..12 {
                px[y * W + 1] = c;
                px[y * W + 12] = c;
                for x in 2..12 {
                    px[y * W + x] = cd;
                }
            }
            // Bottom: row 12
            for x in 1..13 {
                px[12 * W + x] = c;
            }
        }
        FileIcon::Text => {
            let border = 0xFFAAAACCu32;
            let fill = 0xFF333344u32;
            let line_c = 0xFF8888AAu32;
            // Document outline: rows 1-12, cols 2-11
            for y in 1..13 {
                px[y * W + 2] = border;
                px[y * W + 11] = border;
            }
            for x in 2..12 {
                px[1 * W + x] = border;
                px[12 * W + x] = border;
            }
            // Fill
            for y in 2..12 {
                for x in 3..11 {
                    px[y * W + x] = fill;
                }
            }
            // Text lines
            for x in 4..10 {
                px[4 * W + x] = line_c;
            }
            for x in 4..9 {
                px[6 * W + x] = line_c;
            }
            for x in 4..10 {
                px[8 * W + x] = line_c;
            }
            for x in 4..7 {
                px[10 * W + x] = line_c;
            }
        }
        FileIcon::Image => {
            let border = 0xFF44AA44u32;
            let fill = 0xFF223322u32;
            let mtn = 0xFF66CC66u32;
            let sun = 0xFFEEDD44u32;
            // Frame
            for y in 1..13 {
                px[y * W + 1] = border;
                px[y * W + 12] = border;
            }
            for x in 1..13 {
                px[1 * W + x] = border;
                px[12 * W + x] = border;
            }
            for y in 2..12 {
                for x in 2..12 {
                    px[y * W + x] = fill;
                }
            }
            // Sun (circle-ish at 4,4)
            px[3 * W + 4] = sun;
            px[3 * W + 5] = sun;
            px[4 * W + 3] = sun;
            px[4 * W + 4] = sun;
            px[4 * W + 5] = sun;
            px[4 * W + 6] = sun;
            px[5 * W + 4] = sun;
            px[5 * W + 5] = sun;
            // Mountain
            for i in 0..5u32 {
                let y = (11 - i) as usize;
                let cx = 7;
                let left = cx - i as usize;
                let right = cx + i as usize;
                if left >= 2 && right < 12 {
                    px[y * W + left] = mtn;
                    px[y * W + right] = mtn;
                    for x in (left + 1)..right {
                        px[y * W + x] = mtn;
                    }
                }
            }
        }
        FileIcon::Archive => {
            let border = 0xFFCC6644u32;
            let fill = 0xFF442211u32;
            let stripe = 0xFFCC6644u32;
            // Box outline
            for y in 2..12 {
                px[y * W + 2] = border;
                px[y * W + 11] = border;
            }
            for x in 2..12 {
                px[2 * W + x] = border;
                px[11 * W + x] = border;
            }
            for y in 3..11 {
                for x in 3..11 {
                    px[y * W + x] = fill;
                }
            }
            // Horizontal stripes
            for x in 3..11 {
                px[5 * W + x] = stripe;
                px[8 * W + x] = stripe;
            }
            // Zipper (center vertical)
            for y in 3..11 {
                px[y * W + 6] = stripe;
                px[y * W + 7] = stripe;
            }
        }
        FileIcon::Executable => {
            let c = 0xFF4488CCu32;
            let fill = 0xFF223344u32;
            // Gear shape: circle with teeth
            for y in 2..12 {
                for x in 2..12 {
                    let dx = x as i32 - 7;
                    let dy = y as i32 - 7;
                    let d = dx * dx + dy * dy;
                    if d <= 20 && d >= 4 {
                        px[y * W + x] = c;
                    } else if d < 4 {
                        px[y * W + x] = fill;
                    }
                }
            }
            // Teeth (N/S/E/W)
            px[1 * W + 6] = c;
            px[1 * W + 7] = c;
            px[12 * W + 6] = c;
            px[12 * W + 7] = c;
            px[6 * W + 1] = c;
            px[7 * W + 1] = c;
            px[6 * W + 12] = c;
            px[7 * W + 12] = c;
        }
        FileIcon::Audio => {
            let c = 0xFF44AACC;
            // Musical note shape
            // Note head at (4,10)-(6,11)
            for y in 9..12 {
                for x in 3..7 {
                    px[y * W + x] = c;
                }
            }
            // Stem: col 6, rows 3-9
            for y in 3..10 {
                px[y * W + 6] = c;
            }
            // Flag: rows 3-5, cols 7-9
            px[3 * W + 7] = c;
            px[3 * W + 8] = c;
            px[3 * W + 9] = c;
            px[4 * W + 8] = c;
            px[4 * W + 9] = c;
            px[5 * W + 9] = c;
        }
        FileIcon::Video => {
            let c = 0xFFCC44AA;
            let fill = 0xFF331133;
            // Film frame outline
            for y in 2..12 {
                px[y * W + 1] = c;
                px[y * W + 12] = c;
            }
            for x in 1..13 {
                px[2 * W + x] = c;
                px[11 * W + x] = c;
            }
            for y in 3..11 {
                for x in 2..12 {
                    px[y * W + x] = fill;
                }
            }
            // Play triangle in center
            px[5 * W + 5] = c;
            px[5 * W + 6] = c;
            px[6 * W + 5] = c;
            px[6 * W + 6] = c;
            px[6 * W + 7] = c;
            px[6 * W + 8] = c;
            px[7 * W + 5] = c;
            px[7 * W + 6] = c;
            px[7 * W + 7] = c;
            px[7 * W + 8] = c;
            px[8 * W + 5] = c;
            px[8 * W + 6] = c;
            // Sprocket holes
            for &y in &[3usize, 5, 7, 9] {
                px[y * W + 2] = c;
                px[y * W + 11] = c;
            }
        }
        FileIcon::Default => {
            let border = 0xFF888888u32;
            let fill = 0xFF333333u32;
            // Generic document
            for y in 1..13 {
                px[y * W + 3] = border;
                px[y * W + 10] = border;
            }
            for x in 3..11 {
                px[1 * W + x] = border;
                px[12 * W + x] = border;
            }
            for y in 2..12 {
                for x in 4..10 {
                    px[y * W + x] = fill;
                }
            }
            // Dog-ear at top-right
            px[1 * W + 8] = 0;
            px[1 * W + 9] = 0;
            px[1 * W + 10] = 0;
            px[2 * W + 9] = border;
            px[2 * W + 10] = border;
            px[3 * W + 10] = border;
        }
    }

    px
}

/// Cached icon pixel arrays — generated once, reused for every grid refresh.
static mut ICON_CACHE: Option<[[u32; 196]; 9]> = None;

fn get_icon_cache() -> &'static [[u32; 196]; 9] {
    unsafe {
        if ICON_CACHE.is_none() {
            ICON_CACHE = Some([
                generate_icon(FileIcon::ParentDir),
                generate_icon(FileIcon::Folder),
                generate_icon(FileIcon::Text),
                generate_icon(FileIcon::Image),
                generate_icon(FileIcon::Archive),
                generate_icon(FileIcon::Executable),
                generate_icon(FileIcon::Audio),
                generate_icon(FileIcon::Video),
                generate_icon(FileIcon::Default),
            ]);
        }
        ICON_CACHE.as_ref().unwrap()
    }
}

fn icon_index(icon: FileIcon) -> usize {
    match icon {
        FileIcon::ParentDir => 0,
        FileIcon::Folder => 1,
        FileIcon::Text => 2,
        FileIcon::Image => 3,
        FileIcon::Archive => 4,
        FileIcon::Executable => 5,
        FileIcon::Audio => 6,
        FileIcon::Video => 7,
        FileIcon::Default => 8,
    }
}

/// Apply file type icons to all rows in a grid after populate_grid.
fn apply_file_icons(grid: &anyui::DataGrid, files: &[FileEntry]) {
    let cache = get_icon_cache();
    for (row, entry) in files.iter().enumerate() {
        let icon_type = icon_for_name(&entry.name, entry.is_dir);
        let pixels = &cache[icon_index(icon_type)];
        grid.set_cell_icon(row as u32, 0, pixels, 14, 14);
    }
}

// ─── Local Filesystem ─────────────────────────────────────────────────────────

fn format_mtime(ts: u32) -> String {
    if ts == 0 {
        return String::new();
    }
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut secs = ts;
    let sec = secs % 60;
    let _ = sec;
    secs /= 60;
    let min = secs % 60;
    secs /= 60;
    let hour = secs % 24;
    secs /= 24;
    let mut days = secs;
    let mut year = 1970u32;
    loop {
        let ydays = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
            366
        } else {
            365
        };
        if days < ydays {
            break;
        }
        days -= ydays;
        year += 1;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let mdays: [u32; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0usize;
    for m in 0..12 {
        if days < mdays[m] {
            month = m;
            break;
        }
        days -= mdays[m];
        if m == 11 {
            month = 11;
        }
    }
    let day = days + 1;
    format!(
        "{} {:02} {:02}:{:02} {}",
        months[month], day, hour, min, year
    )
}

fn list_local_dir(path: &str) -> Vec<FileEntry> {
    let show_hidden = SHOW_HIDDEN.load(Ordering::Relaxed);
    let mut buf = [0u8; 64 * 256];
    let count = fs::readdir(path, &mut buf);
    if count == u32::MAX {
        return Vec::new();
    }
    let mut entries = Vec::new();
    if path != "/" {
        entries.push(FileEntry {
            name: "..".into(),
            size: 0,
            is_dir: true,
            modified: "".into(),
        });
    }
    for i in 0..count as usize {
        let base = i * 64;
        if base + 64 > buf.len() {
            break;
        }
        let entry_type = buf[base];
        let name_len = buf[base + 1] as usize;
        let size =
            u32::from_le_bytes([buf[base + 4], buf[base + 5], buf[base + 6], buf[base + 7]]) as u64;
        if name_len == 0 || name_len > 56 {
            continue;
        }
        let name_bytes = &buf[base + 8..base + 8 + name_len];
        // Trim trailing null bytes
        let trimmed = match name_bytes.iter().position(|&b| b == 0) {
            Some(pos) => &name_bytes[..pos],
            None => name_bytes,
        };
        if trimmed.is_empty() {
            continue;
        }
        let name = match core::str::from_utf8(trimmed) {
            Ok(s) => s.to_string(),
            Err(_) => continue,
        };
        if name == "." || name == ".." {
            continue;
        }
        // Filter hidden files (starting with .)
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let is_dir = entry_type == 1;
        // Get mtime via stat
        let mut child_path = String::from(path);
        if !child_path.ends_with('/') {
            child_path.push('/');
        }
        child_path.push_str(&name);
        let mut st = [0u32; 7];
        let modified = if fs::stat(&child_path, &mut st) != u32::MAX && st[6] != 0 {
            format_mtime(st[6])
        } else {
            String::new()
        };
        entries.push(FileEntry {
            name,
            size,
            is_dir,
            modified,
        });
    }
    entries
}

fn join_path(base: &str, name: &str) -> String {
    if name == ".." {
        if base == "/" {
            return "/".to_string();
        }
        if let Some(pos) = base.rfind('/') {
            if pos == 0 {
                return "/".to_string();
            }
            return base[..pos].to_string();
        }
        return "/".to_string();
    }
    if base.ends_with('/') {
        format!("{}{}", base, name)
    } else {
        format!("{}/{}", base, name)
    }
}

fn get_home_dir() -> String {
    let mut buf = [0u8; 256];
    let len = env::get("HOME", &mut buf);
    if len > 0 && (len as usize) <= buf.len() {
        if let Ok(s) = core::str::from_utf8(&buf[..len as usize]) {
            return s.trim().to_string();
        }
    }
    "/".to_string()
}

fn get_field_text(field: &anyui::TextField) -> String {
    let mut buf = [0u8; 512];
    let len = field.get_text(&mut buf) as usize;
    let len = len.min(buf.len());
    core::str::from_utf8(&buf[..len])
        .unwrap_or("")
        .trim()
        .to_string()
}

fn get_editor_text(editor: &anyui::TextEditor) -> String {
    let mut buf = [0u8; 32768];
    let len = editor.get_text(&mut buf) as usize;
    let len = len.min(buf.len());
    core::str::from_utf8(&buf[..len]).unwrap_or("").to_string()
}

// ─── Worker Thread State ──────────────────────────────────────────────────────
//
// The worker thread runs all blocking FTP operations.
// Communication with the UI thread via atomics + a lock-free result buffer.
//
// CMD values:
const CMD_IDLE: u32 = 0;
const CMD_CONNECT: u32 = 1;
const CMD_LIST: u32 = 2;
const CMD_DOWNLOAD: u32 = 3;
const CMD_UPLOAD: u32 = 4;
const CMD_CD: u32 = 5;
const CMD_MKDIR: u32 = 6;
const CMD_DELETE: u32 = 7;
const CMD_RENAME: u32 = 8;
const CMD_DISCONNECT: u32 = 9;
const CMD_EXIT: u32 = 10;

// Result codes
const RES_NONE: u32 = 0;
const RES_OK: u32 = 1;
const RES_ERROR: u32 = 2;
const RES_BUSY: u32 = 3;

static WORKER_CMD: AtomicU32 = AtomicU32::new(CMD_IDLE);
static WORKER_RESULT: AtomicU32 = AtomicU32::new(RES_NONE);
static WORKER_BUSY: AtomicBool = AtomicBool::new(false);

// Shared string buffers (64-byte aligned, max 512 chars each)
// We use fixed-size static buffers to avoid heap allocation from multiple threads.
// The UI thread writes before setting CMD; the worker reads after seeing CMD != IDLE.
// The worker writes result strings; the UI thread reads after WORKER_RESULT != NONE.

static mut PARAM1: [u8; 512] = [0u8; 512]; // host / remote_path / old_name
static mut PARAM2: [u8; 512] = [0u8; 512]; // user / local_path  / new_name
static mut PARAM3: [u8; 512] = [0u8; 512]; // pass / remote_name
static PARAM_PORT: AtomicU32 = AtomicU32::new(21);

// Result strings (written by worker)
static mut RESULT_STR: [u8; 8192] = [0u8; 8192]; // file list or log message
static mut RESULT_STR_LEN: AtomicU32 = AtomicU32::new(0);
static mut RESULT_STR2: [u8; 512] = [0u8; 512]; // remote pwd
static mut RESULT_STR2_LEN: AtomicU32 = AtomicU32::new(0);

fn write_param(buf: &mut [u8; 512], s: &str) {
    let len = s.len().min(511);
    buf[..len].copy_from_slice(&s.as_bytes()[..len]);
    buf[len] = 0;
}

fn read_cstr(buf: &[u8]) -> &str {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    core::str::from_utf8(&buf[..end]).unwrap_or("")
}

// ─── Worker Thread Entry ──────────────────────────────────────────────────────

fn worker_entry() {
    // Persistent worker thread: loops waiting for commands.
    // This thread owns the FTP control socket — killing it would RST the connection.
    loop {
        let cmd = WORKER_CMD.load(Ordering::Acquire);
        if cmd == CMD_IDLE {
            anyos_std::process::sleep(10);
            continue;
        }

        match cmd {
            CMD_CONNECT => {
                let host = unsafe { read_cstr(&PARAM1) }.to_string();
                let user = unsafe { read_cstr(&PARAM2) }.to_string();
                let pass = unsafe { read_cstr(&PARAM3) }.to_string();
                let port = PARAM_PORT.load(Ordering::Relaxed) as u16;

                let mut ip = [0u8; 4];
                if net::dns(&host, &mut ip) != 0 {
                    unsafe {
                        write_result_str(&format!("DNS-Fehler: {}", host));
                    }
                    WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                } else {
                    match FtpClient::connect(&ip, port) {
                        None => {
                            unsafe {
                                write_result_str(&format!("Verbindungsfehler: {}:{}", host, port));
                            }
                            WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                        }
                        Some(mut ftp) => {
                            if !ftp.login(&user, &pass) {
                                unsafe {
                                    write_result_str("Login fehlgeschlagen");
                                }
                                ftp.disconnect();
                                WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                            } else {
                                // Store the connected FTP client
                                unsafe {
                                    FTP_CLIENT = Some(ftp);
                                }
                                // Get initial directory listing
                                do_list_in_worker();
                                WORKER_RESULT.store(RES_OK, Ordering::Release);
                            }
                        }
                    }
                }
            }

            CMD_LIST => {
                do_list_in_worker();
                WORKER_RESULT.store(RES_OK, Ordering::Release);
            }

            CMD_CD => {
                let path = unsafe { read_cstr(&PARAM1) }.to_string();
                if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                    if ftp.cd(&path) {
                        do_list_in_worker();
                        WORKER_RESULT.store(RES_OK, Ordering::Release);
                    } else {
                        unsafe {
                            write_result_str("Verzeichniswechsel fehlgeschlagen");
                        }
                        WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                    }
                }
            }

            CMD_DOWNLOAD => {
                let remote_name = unsafe { read_cstr(&PARAM1) }.to_string();
                let local_path = unsafe { read_cstr(&PARAM2) }.to_string();
                if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                    let bytes = ftp.download(&remote_name, &local_path);
                    if bytes > 0 {
                        unsafe {
                            write_result_str(&format!(
                                "OK Download: {} ({} Bytes)",
                                remote_name, bytes
                            ));
                        }
                        do_list_in_worker();
                        WORKER_RESULT.store(RES_OK, Ordering::Release);
                    } else {
                        unsafe {
                            write_result_str(&format!("FEHLER Download: {}", remote_name));
                        }
                        WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                    }
                }
            }

            CMD_UPLOAD => {
                let local_path = unsafe { read_cstr(&PARAM1) }.to_string();
                let remote_name = unsafe { read_cstr(&PARAM2) }.to_string();
                if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                    let bytes = ftp.upload(&local_path, &remote_name);
                    if bytes > 0 {
                        unsafe {
                            write_result_str(&format!(
                                "OK Upload: {} ({} Bytes)",
                                remote_name, bytes
                            ));
                        }
                        do_list_in_worker();
                        WORKER_RESULT.store(RES_OK, Ordering::Release);
                    } else {
                        unsafe {
                            write_result_str(&format!("FEHLER Upload: {}", remote_name));
                        }
                        WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                    }
                }
            }

            CMD_MKDIR => {
                let name = unsafe { read_cstr(&PARAM1) }.to_string();
                if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                    let ok = ftp.mkdir(&name);
                    if ok {
                        unsafe {
                            write_result_str(&format!("Ordner erstellt: {}", name));
                        }
                        do_list_in_worker();
                        WORKER_RESULT.store(RES_OK, Ordering::Release);
                    } else {
                        unsafe {
                            write_result_str(&format!("Ordner erstellen fehlgeschlagen: {}", name));
                        }
                        WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                    }
                }
            }

            CMD_DELETE => {
                let name = unsafe { read_cstr(&PARAM1) }.to_string();
                let is_dir = PARAM_PORT.load(Ordering::Relaxed) == 1;
                if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                    let ok = if is_dir {
                        ftp.delete_dir(&name)
                    } else {
                        ftp.delete_file(&name)
                    };
                    if ok {
                        unsafe {
                            write_result_str(&format!("Geloescht: {}", name));
                        }
                        do_list_in_worker();
                        WORKER_RESULT.store(RES_OK, Ordering::Release);
                    } else {
                        unsafe {
                            write_result_str(&format!("Loeschen fehlgeschlagen: {}", name));
                        }
                        WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                    }
                }
            }

            CMD_RENAME => {
                let old = unsafe { read_cstr(&PARAM1) }.to_string();
                let new = unsafe { read_cstr(&PARAM2) }.to_string();
                if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                    let ok = ftp.rename(&old, &new);
                    if ok {
                        unsafe {
                            write_result_str(&format!("Umbenannt: {} -> {}", old, new));
                        }
                        do_list_in_worker();
                        WORKER_RESULT.store(RES_OK, Ordering::Release);
                    } else {
                        unsafe {
                            write_result_str(&format!("Umbenennen fehlgeschlagen: {}", old));
                        }
                        WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                    }
                }
            }

            CMD_DISCONNECT => {
                if let Some(mut ftp) = unsafe { FTP_CLIENT.take() } {
                    ftp.disconnect();
                }
                unsafe {
                    write_result_str("Verbindung getrennt");
                }
                WORKER_RESULT.store(RES_OK, Ordering::Release);
            }

            CMD_EXIT => {
                if let Some(mut ftp) = unsafe { FTP_CLIENT.take() } {
                    ftp.disconnect();
                }
                WORKER_CMD.store(CMD_IDLE, Ordering::Release);
                WORKER_BUSY.store(false, Ordering::Release);
                return; // exit worker thread
            }

            _ => {}
        }

        WORKER_CMD.store(CMD_IDLE, Ordering::Release);
        WORKER_BUSY.store(false, Ordering::Release);
    } // end loop
}

fn do_list_in_worker() {
    if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
        let show_hidden = SHOW_HIDDEN.load(Ordering::Relaxed);
        let files = ftp.list_dir_ex(show_hidden);
        let pwd = ftp.pwd();

        // Serialize file list into RESULT_STR: "name\x1Fsize\x1Fdate\x1Eis_dir\x1E..."
        let mut out: Vec<u8> = Vec::new();
        for entry in &files {
            out.extend_from_slice(entry.name.as_bytes());
            out.push(0x1F);
            out.extend_from_slice(format_size(entry.size).as_bytes());
            out.push(0x1F);
            out.extend_from_slice(entry.modified.as_bytes());
            out.push(0x1F);
            out.push(if entry.is_dir { b'1' } else { b'0' });
            out.push(0x1E);
        }
        unsafe {
            let len = out.len().min(RESULT_STR.len());
            RESULT_STR[..len].copy_from_slice(&out[..len]);
            RESULT_STR_LEN.store(len as u32, Ordering::Release);

            let plen = pwd.len().min(RESULT_STR2.len() - 1);
            RESULT_STR2[..plen].copy_from_slice(&pwd.as_bytes()[..plen]);
            RESULT_STR2[plen] = 0;
            RESULT_STR2_LEN.store(plen as u32, Ordering::Release);
        }
    }
}

unsafe fn write_result_str(s: &str) {
    let len = s.len().min(RESULT_STR.len() - 1);
    RESULT_STR[..len].copy_from_slice(&s.as_bytes()[..len]);
    RESULT_STR[len] = 0;
    RESULT_STR_LEN.store(len as u32, Ordering::Release);
}

fn parse_file_list(raw: &[u8], len: usize) -> Vec<FileEntry> {
    let s = core::str::from_utf8(&raw[..len]).unwrap_or("");
    let mut entries = Vec::new();
    for row in s.split('\x1E') {
        if row.is_empty() {
            continue;
        }
        let cols: Vec<&str> = row.split('\x1F').collect();
        if cols.len() < 4 {
            continue;
        }
        let name = cols[0].to_string();
        let size_str = cols[1];
        let modified = cols[2].to_string();
        let is_dir = cols[3] == "1";
        // Approximate size back (display only)
        let size: u64 = 0; // size is only for display, already formatted
        let _ = size_str;
        entries.push(FileEntry {
            name,
            size,
            is_dir,
            modified,
        });
    }
    entries
}

static WORKER_SPAWNED: AtomicBool = AtomicBool::new(false);
static WORKER_TID: AtomicU32 = AtomicU32::new(0);

fn spawn_worker() {
    WORKER_BUSY.store(true, Ordering::Release);
    WORKER_RESULT.store(RES_NONE, Ordering::Release);
    // Only spawn the worker thread once; it loops forever waiting for commands.
    if !WORKER_SPAWNED.load(Ordering::Relaxed) {
        if let Ok(handle) =
            anyos_std::process::Thread::spawn_with_stack(worker_entry, 64 * 1024, "ftp-worker")
        {
            WORKER_TID.store(handle.tid(), Ordering::Release);
            core::mem::forget(handle);
            WORKER_SPAWNED.store(true, Ordering::Release);
        } else {
            WORKER_BUSY.store(false, Ordering::Release);
        }
    }
    // If already spawned, the worker thread will pick up CMD from WORKER_CMD.
}

/// Force-kill the worker thread so the process can exit cleanly.
fn kill_worker() {
    let tid = WORKER_TID.load(Ordering::Relaxed);
    if tid != 0 {
        anyos_std::process::kill(tid);
        WORKER_SPAWNED.store(false, Ordering::Release);
        WORKER_TID.store(0, Ordering::Release);
    }
}

// Static FTP client (only accessed from worker thread)
static mut FTP_CLIENT: Option<FtpClient> = None;
static SHOW_HIDDEN: AtomicBool = AtomicBool::new(false);

// ─── App State ────────────────────────────────────────────────────────────────

struct AppState {
    remote_host: String,
    remote_dir: String,
    remote_files: Vec<FileEntry>,
    local_dir: String,
    local_files: Vec<FileEntry>,
    focus_pane: u8,
    log_visible: bool,
    is_connected: bool,
    startup_timer_id: u32,
    poll_timer_id: u32,
    // Connection management
    sites: Vec<SiteProfile>,
    last_host: String,
    last_port: u16,
    last_user: String,
    last_pass: String,
    reconnect_attempts: u32,
    initial_remote_dir: String,
    // Sorting
    local_sort_col: SortColumn,
    local_sort_order: SortOrder,
    remote_sort_col: SortColumn,
    remote_sort_order: SortOrder,
    // Hidden files
    show_hidden: bool,
    // Path editing
    local_path_editing: bool,
    remote_path_editing: bool,
    // Quick connect bar
    qc_host: anyui::TextField,
    qc_port: anyui::TextField,
    qc_user: anyui::TextField,
    qc_pass: anyui::TextField,
    // Breadcrumb / path bar
    local_path_bar: anyui::View,
    local_path_label: anyui::Label,
    local_path_field: anyui::TextField,
    remote_path_bar: anyui::View,
    remote_path_label: anyui::Label,
    remote_path_field: anyui::TextField,
    // UI handles
    win: anyui::Window,
    local_grid: anyui::DataGrid,
    remote_grid: anyui::DataGrid,
    log_view: anyui::View,
    log_editor: anyui::TextEditor,
    status_label: anyui::Label,
    btn_disconnect: anyui::IconButton,
    btn_upload: anyui::IconButton,
    btn_download: anyui::IconButton,
    btn_delete: anyui::IconButton,
    btn_rename: anyui::IconButton,
    btn_newfolder: anyui::IconButton,
}

anyos_std::global_app_state!(AppState);

// ─── Grid Population ──────────────────────────────────────────────────────────

fn populate_grid(grid: &anyui::DataGrid, files: &[FileEntry]) {
    let mut data: Vec<u8> = Vec::new();
    let mut colors: Vec<u32> = Vec::new();

    for entry in files {
        let col = if entry.name == ".." {
            0xFF888888u32
        } else if entry.is_dir {
            0xFFDDAA44u32
        } else {
            0xFFCCCCCCu32
        };

        if entry.name == ".." {
            data.extend_from_slice(b"[..] Ordner hoch");
        } else if entry.is_dir {
            data.push(b'[');
            data.extend_from_slice(entry.name.as_bytes());
            data.push(b']');
        } else {
            data.extend_from_slice(entry.name.as_bytes());
        }
        data.push(0x1F);
        if entry.is_dir || entry.name == ".." {
            data.extend_from_slice(b"<DIR>");
        } else {
            data.extend_from_slice(format_size(entry.size).as_bytes());
        }
        data.push(0x1F);
        data.extend_from_slice(entry.modified.as_bytes());
        data.push(0x1E);

        colors.push(col);
        colors.push(col);
        colors.push(col);
    }

    grid.set_row_count(files.len() as u32);
    grid.set_data_raw(&data);
    grid.set_cell_colors(&colors);
    apply_file_icons(grid, files);
}

fn refresh_local() {
    let a = app();
    a.local_files = list_local_dir(&a.local_dir.clone());
    let col = a.local_sort_col;
    let order = a.local_sort_order;
    sort_entries(&mut a.local_files, col, order);
    let label = format!("Lokal: {}", a.local_dir);
    a.local_path_label.set_text(&label);
    let files = a.local_files.clone();
    populate_grid(&a.local_grid, &files);
    update_status();
}

fn resort_remote() {
    let a = app();
    let col = a.remote_sort_col;
    let order = a.remote_sort_order;
    sort_entries(&mut a.remote_files, col, order);
    let files = a.remote_files.clone();
    populate_grid(&a.remote_grid, &files);
}

fn cycle_sort(pane: u8, col: SortColumn) {
    let a = app();
    let (cur_col, cur_order) = if pane == 0 {
        (a.local_sort_col, a.local_sort_order)
    } else {
        (a.remote_sort_col, a.remote_sort_order)
    };
    let new_order = if cur_col == col {
        match cur_order {
            SortOrder::Asc => SortOrder::Desc,
            SortOrder::Desc => SortOrder::Asc,
        }
    } else {
        SortOrder::Asc
    };
    if pane == 0 {
        a.local_sort_col = col;
        a.local_sort_order = new_order;
        sort_entries(&mut a.local_files, col, new_order);
        let files = a.local_files.clone();
        populate_grid(&a.local_grid, &files);
    } else {
        a.remote_sort_col = col;
        a.remote_sort_order = new_order;
        sort_entries(&mut a.remote_files, col, new_order);
        let files = a.remote_files.clone();
        populate_grid(&a.remote_grid, &files);
    }
    let arrow = match new_order {
        SortOrder::Asc => "^",
        SortOrder::Desc => "v",
    };
    let col_name = match col {
        SortColumn::Name => "Name",
        SortColumn::Size => "Groesse",
        SortColumn::Date => "Datum",
    };
    let side = if pane == 0 { "Lokal" } else { "Remote" };
    log_line(&format!("{}: Sortiert nach {} {}", side, col_name, arrow));
}

fn toggle_path_editing(pane: u8) {
    let a = app();
    if pane == 0 {
        a.local_path_editing = !a.local_path_editing;
        if a.local_path_editing {
            a.local_path_label.set_visible(false);
            a.local_path_field.set_visible(true);
            a.local_path_field.set_text(&a.local_dir);
            a.local_path_field.focus();
        } else {
            a.local_path_label.set_visible(true);
            a.local_path_field.set_visible(false);
        }
    } else {
        a.remote_path_editing = !a.remote_path_editing;
        if a.remote_path_editing {
            a.remote_path_label.set_visible(false);
            a.remote_path_field.set_visible(true);
            a.remote_path_field.set_text(&a.remote_dir);
            a.remote_path_field.focus();
        } else {
            a.remote_path_label.set_visible(true);
            a.remote_path_field.set_visible(false);
        }
    }
}

fn update_status() {
    let a = app();
    let text = if a.is_connected {
        let rc = a.remote_files.len().saturating_sub(1);
        let lc = a.local_files.len().saturating_sub(1);
        format!(
            "Verbunden: {} | Remote: {} | Lokal: {}",
            a.remote_host, rc, lc
        )
    } else {
        "Nicht verbunden".to_string()
    };
    a.status_label.set_text(&text);
}

fn log_line(msg: &str) {
    let a = app();
    let cur = get_editor_text(&a.log_editor);
    let new_text = if cur.is_empty() {
        msg.to_string()
    } else {
        format!("{}\n{}", cur, msg)
    };
    a.log_editor.set_text(&new_text);
}

fn set_connected_state(connected: bool) {
    let a = app();
    a.is_connected = connected;
    a.btn_disconnect.set_enabled(connected);
    a.btn_upload.set_enabled(connected);
    a.btn_download.set_enabled(connected);
    a.btn_delete.set_enabled(connected);
    a.btn_rename.set_enabled(connected);
    a.btn_newfolder.set_enabled(connected);
}

// ─── Worker Result Polling ────────────────────────────────────────────────────

fn poll_worker() {
    let result = WORKER_RESULT.load(Ordering::Acquire);
    if result == RES_NONE {
        return;
    }

    // Reset result so we don't process it twice
    WORKER_RESULT.store(RES_NONE, Ordering::Release);

    let cmd_was = WORKER_CMD.load(Ordering::Relaxed); // already CMD_IDLE by now
    let _ = cmd_was;

    // Read result string
    let rlen = unsafe { RESULT_STR_LEN.load(Ordering::Acquire) as usize };
    let msg = unsafe {
        core::str::from_utf8(&RESULT_STR[..rlen])
            .unwrap_or("")
            .to_string()
    };

    match result {
        RES_OK => {
            // Update remote listing if we have new data
            let list_len = unsafe { RESULT_STR_LEN.load(Ordering::Acquire) as usize };
            let pwd_len = unsafe { RESULT_STR2_LEN.load(Ordering::Acquire) as usize };
            if pwd_len > 0 {
                let files = unsafe { parse_file_list(&RESULT_STR, list_len) };
                let pwd = unsafe {
                    core::str::from_utf8(&RESULT_STR2[..pwd_len])
                        .unwrap_or("/")
                        .to_string()
                };
                log_line(&format!(
                    "Verzeichnis empfangen: {} ({} Eintraege)",
                    pwd,
                    files.len()
                ));
                let a = app();
                a.remote_dir = pwd.clone();
                a.remote_files = files;
                let col = a.remote_sort_col;
                let order = a.remote_sort_order;
                sort_entries(&mut a.remote_files, col, order);
                let label = format!("Remote: {}", pwd);
                a.remote_path_label.set_text(&label);
                let sorted_files = a.remote_files.clone();
                populate_grid(&a.remote_grid, &sorted_files);
                log_line("Verzeichnisbaum aktualisiert");
                if !a.is_connected {
                    set_connected_state(true);
                    log_line("Verbindung hergestellt");
                    // After first connect, navigate to initial remote dir if set
                    let init_dir = a.initial_remote_dir.clone();
                    if !init_dir.is_empty() && init_dir != "/" {
                        a.initial_remote_dir = String::new();
                        if !WORKER_BUSY.load(Ordering::Relaxed) {
                            unsafe {
                                write_param(&mut PARAM1, &init_dir);
                            }
                            WORKER_CMD.store(CMD_CD, Ordering::Release);
                            spawn_worker();
                        }
                    }
                }
                // Reset STR2 len so we don't re-apply it
                unsafe {
                    RESULT_STR2_LEN.store(0, Ordering::Release);
                }
            } else {
                // Log the message if it's a human-readable result (not a file list)
                if !msg.is_empty() && !msg.contains('\x1E') {
                    log_line(&msg);
                }
            }
            // If disconnect was OK, clear remote grid
            if !app().is_connected {
                // Already disconnected
            }
            update_status();
            // After download, refresh local
            refresh_local();
        }
        RES_ERROR => {
            log_line(&format!("Fehler: {}", msg));
            // Check if this is a connection loss while connected → try reconnect
            let a = app();
            if a.is_connected && !a.last_host.is_empty() && a.reconnect_attempts < 3 {
                a.reconnect_attempts += 1;
                let attempt = a.reconnect_attempts;
                log_line(&format!(
                    "Verbindung verloren. Reconnect-Versuch {}/3...",
                    attempt
                ));
                set_connected_state(false);
                a.is_connected = false;
                unsafe {
                    FTP_CLIENT.take();
                } // drop broken client
                  // Reconnect after short delay
                let host = a.last_host.clone();
                let port = a.last_port;
                let user = a.last_user.clone();
                let pass = a.last_pass.clone();
                do_connect(&host, port, &user, &pass);
            } else {
                anyui::MessageBox::show(anyui::MessageBoxType::Alert, &msg, None);
            }
        }
        _ => {}
    }
}

// ─── Connect Helper ──────────────────────────────────────────────────────────

fn do_connect(host: &str, port: u16, user: &str, pass: &str) {
    if WORKER_BUSY.load(Ordering::Relaxed) {
        return;
    }
    if host.is_empty() {
        anyui::MessageBox::show(anyui::MessageBoxType::Alert, "Bitte Host eingeben.", None);
        return;
    }
    let user = if user.is_empty() { "anonymous" } else { user };
    let pass = if pass.is_empty() { "anonymous@" } else { pass };

    unsafe {
        write_param(&mut PARAM1, host);
        write_param(&mut PARAM2, user);
        write_param(&mut PARAM3, pass);
    }
    PARAM_PORT.store(port as u32, Ordering::Relaxed);

    let a = app();
    a.remote_host = host.to_string();
    a.last_host = host.to_string();
    a.last_port = port;
    a.last_user = user.to_string();
    a.last_pass = pass.to_string();
    a.reconnect_attempts = 0;

    // Prefs sofort speichern (robust gegen Absturz/Kill)
    let (w, h) = a.win.get_size();
    let (x, y) = a.win.get_position();
    save_prefs(&Prefs {
        win_x: x,
        win_y: y,
        win_w: w,
        win_h: h,
        last_host: a.last_host.clone(),
        last_port: a.last_port,
        last_user: a.last_user.clone(),
        last_pass: a.last_pass.clone(),
    });

    log_line(&format!("Verbinde mit {}:{}...", host, port));
    WORKER_CMD.store(CMD_CONNECT, Ordering::Release);
    spawn_worker();
}

fn do_quick_connect() {
    let a = app();
    let host = get_field_text(&a.qc_host);
    let port_str = get_field_text(&a.qc_port);
    let user = get_field_text(&a.qc_user);
    let pass = get_field_text(&a.qc_pass);
    let port: u16 = port_str.parse().unwrap_or(21);
    do_connect(&host, port, &user, &pass);
}

// ─── Site Manager Dialog ─────────────────────────────────────────────────────

fn show_site_manager() {
    let sites = load_sites();
    let dlg = anyui::Window::new_with_flags(
        "Site Manager",
        -1,
        -1,
        540,
        420,
        anyui::WIN_FLAG_NOT_RESIZABLE | anyui::WIN_FLAG_NO_MAXIMIZE,
    );

    let panel = anyui::View::new();
    panel.set_dock(anyui::DOCK_FILL);
    panel.set_color(0xFF2D2D30);
    dlg.add(&panel);

    // Site list grid
    let grid = anyui::DataGrid::new(510, 280);
    grid.set_size(510, 280);
    grid.set_position(14, 14);
    grid.set_color(0xFF1E1E1E);
    grid.set_columns(&[
        anyui::ColumnDef::new("Name").width(140),
        anyui::ColumnDef::new("Host").width(160),
        anyui::ColumnDef::new("Port")
            .width(50)
            .align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new("Benutzer").width(130),
    ]);
    panel.add(&grid);

    fn populate_site_grid(grid: &anyui::DataGrid, sites: &[SiteProfile]) {
        let mut data: Vec<u8> = Vec::new();
        for site in sites {
            data.extend_from_slice(site.name.as_bytes());
            data.push(0x1F);
            data.extend_from_slice(site.host.as_bytes());
            data.push(0x1F);
            data.extend_from_slice(format!("{}", site.port).as_bytes());
            data.push(0x1F);
            data.extend_from_slice(site.user.as_bytes());
            data.push(0x1E);
        }
        grid.set_row_count(sites.len() as u32);
        grid.set_data_raw(&data);
    }

    populate_site_grid(&grid, &sites);

    // Buttons
    let btn_connect = anyui::Button::new("Verbinden");
    btn_connect.set_size(100, 30);
    btn_connect.set_position(14, 306);
    panel.add(&btn_connect);

    let btn_add = anyui::Button::new("Hinzufuegen");
    btn_add.set_size(100, 30);
    btn_add.set_position(124, 306);
    panel.add(&btn_add);

    let btn_edit = anyui::Button::new("Bearbeiten");
    btn_edit.set_size(100, 30);
    btn_edit.set_position(234, 306);
    panel.add(&btn_edit);

    let btn_delete = anyui::Button::new("Loeschen");
    btn_delete.set_size(100, 30);
    btn_delete.set_position(344, 306);
    panel.add(&btn_delete);

    let btn_close = anyui::Button::new("Schliessen");
    btn_close.set_size(100, 30);
    btn_close.set_position(14, 380);
    panel.add(&btn_close);

    // Store sites in global static buffer for callbacks
    unsafe {
        SM_SITES = Some(sites);
    }

    let dlg_c = dlg.clone();
    btn_close.on_click(move |_| {
        dlg_c.destroy();
    });

    // Connect: use selected site
    let dlg_c2 = dlg.clone();
    let grid_c = grid.clone();
    btn_connect.on_click(move |_| {
        let row = grid_c.selected_row() as usize;
        let sites = unsafe { SM_SITES.as_ref().unwrap() };
        if row >= sites.len() {
            return;
        }
        let site = &sites[row];
        app().initial_remote_dir = site.remote_dir.clone();
        do_connect(&site.host, site.port, &site.user, &site.pass);
        // Update quick connect fields
        let a = app();
        a.qc_host.set_text(&site.host);
        a.qc_port.set_text(&format!("{}", site.port));
        a.qc_user.set_text(&site.user);
        a.qc_pass.set_text(&site.pass);
        dlg_c2.destroy();
    });

    // Double-click to connect
    let dlg_c3 = dlg.clone();
    grid.on_submit(move |e| {
        let row = e.index as usize;
        let sites = unsafe { SM_SITES.as_ref().unwrap() };
        if row >= sites.len() {
            return;
        }
        let site = &sites[row];
        app().initial_remote_dir = site.remote_dir.clone();
        do_connect(&site.host, site.port, &site.user, &site.pass);
        let a = app();
        a.qc_host.set_text(&site.host);
        a.qc_port.set_text(&format!("{}", site.port));
        a.qc_user.set_text(&site.user);
        a.qc_pass.set_text(&site.pass);
        dlg_c3.destroy();
    });

    // Add new site
    let grid_c2 = grid.clone();
    btn_add.on_click(move |_| {
        show_site_edit_dialog(None, grid_c2.clone());
    });

    // Edit selected
    let grid_c3 = grid.clone();
    btn_edit.on_click(move |_| {
        let row = grid_c3.selected_row() as usize;
        let sites = unsafe { SM_SITES.as_ref().unwrap() };
        if row >= sites.len() {
            return;
        }
        show_site_edit_dialog(Some(row), grid_c3.clone());
    });

    // Delete selected
    let grid_c4 = grid.clone();
    btn_delete.on_click(move |_| {
        let row = grid_c4.selected_row() as usize;
        let sites = unsafe { SM_SITES.as_mut().unwrap() };
        if row >= sites.len() {
            return;
        }
        sites.remove(row);
        save_sites(sites);
        populate_site_grid(&grid_c4, sites);
        // Update app state
        app().sites = sites.clone();
    });

    dlg.on_close(|_| {});
}

fn show_site_edit_dialog(edit_index: Option<usize>, parent_grid: anyui::DataGrid) {
    let editing = edit_index.is_some();
    let title = if editing {
        "Profil bearbeiten"
    } else {
        "Neues Profil"
    };
    let dlg = anyui::Window::new_with_flags(
        title,
        -1,
        -1,
        420,
        350,
        anyui::WIN_FLAG_NOT_RESIZABLE | anyui::WIN_FLAG_NO_MAXIMIZE,
    );

    let panel = anyui::View::new();
    panel.set_dock(anyui::DOCK_FILL);
    panel.set_color(0xFF2D2D30);
    dlg.add(&panel);

    let lbl_name = anyui::Label::new("Name:");
    lbl_name.set_size(90, 24);
    lbl_name.set_position(16, 20);
    panel.add(&lbl_name);
    let fld_name = anyui::TextField::new();
    fld_name.set_size(290, 26);
    fld_name.set_position(112, 18);
    fld_name.set_placeholder("Mein Server");
    panel.add(&fld_name);

    let lbl_host = anyui::Label::new("Host:");
    lbl_host.set_size(90, 24);
    lbl_host.set_position(16, 58);
    panel.add(&lbl_host);
    let fld_host = anyui::TextField::new();
    fld_host.set_size(290, 26);
    fld_host.set_position(112, 56);
    fld_host.set_placeholder("ftp.example.com");
    panel.add(&fld_host);

    let lbl_port = anyui::Label::new("Port:");
    lbl_port.set_size(90, 24);
    lbl_port.set_position(16, 96);
    panel.add(&lbl_port);
    let fld_port = anyui::TextField::new();
    fld_port.set_size(80, 26);
    fld_port.set_position(112, 94);
    fld_port.set_text("21");
    panel.add(&fld_port);

    let lbl_user = anyui::Label::new("Benutzer:");
    lbl_user.set_size(90, 24);
    lbl_user.set_position(16, 134);
    panel.add(&lbl_user);
    let fld_user = anyui::TextField::new();
    fld_user.set_size(290, 26);
    fld_user.set_position(112, 132);
    fld_user.set_placeholder("anonymous");
    panel.add(&fld_user);

    let lbl_pass = anyui::Label::new("Passwort:");
    lbl_pass.set_size(90, 24);
    lbl_pass.set_position(16, 172);
    panel.add(&lbl_pass);
    let fld_pass = anyui::TextField::new();
    fld_pass.set_size(290, 26);
    fld_pass.set_position(112, 170);
    fld_pass.set_password_mode(true);
    panel.add(&fld_pass);

    let lbl_rdir = anyui::Label::new("Remote-Pfad:");
    lbl_rdir.set_size(90, 24);
    lbl_rdir.set_position(16, 210);
    panel.add(&lbl_rdir);
    let fld_rdir = anyui::TextField::new();
    fld_rdir.set_size(290, 26);
    fld_rdir.set_position(112, 208);
    fld_rdir.set_placeholder("/");
    panel.add(&fld_rdir);

    // Fill in existing values if editing
    if let Some(idx) = edit_index {
        let sites = unsafe { SM_SITES.as_ref() };
        if let Some(sites_vec) = sites {
            if let Some(site) = sites_vec.get(idx) {
                fld_name.set_text(&site.name);
                fld_host.set_text(&site.host);
                fld_port.set_text(&format!("{}", site.port));
                fld_user.set_text(&site.user);
                fld_pass.set_text(&site.pass);
                if !site.remote_dir.is_empty() {
                    fld_rdir.set_text(&site.remote_dir);
                }
            }
        }
    }

    let btn_cancel = anyui::Button::new("Abbrechen");
    btn_cancel.set_size(120, 32);
    btn_cancel.set_position(16, 260);
    panel.add(&btn_cancel);

    let btn_ok = anyui::Button::new("Speichern");
    btn_ok.set_size(120, 32);
    btn_ok.set_position(282, 260);
    panel.add(&btn_ok);

    let dlg_c = dlg.clone();
    btn_cancel.on_click(move |_| {
        dlg_c.destroy();
    });

    let dlg_c2 = dlg.clone();
    let fn_c = fld_name.clone();
    let fh_c = fld_host.clone();
    let fp_c = fld_port.clone();
    let fu_c = fld_user.clone();
    let fpw_c = fld_pass.clone();
    let frd_c = fld_rdir.clone();
    let pg = parent_grid.clone();
    btn_ok.on_click(move |_| {
        let name = get_field_text(&fn_c);
        let host = get_field_text(&fh_c);
        let port_str = get_field_text(&fp_c);
        let user = get_field_text(&fu_c);
        let pass = get_field_text(&fpw_c);
        let remote_dir = get_field_text(&frd_c);

        if host.is_empty() {
            anyui::MessageBox::show(
                anyui::MessageBoxType::Alert,
                "Host darf nicht leer sein.",
                None,
            );
            return;
        }
        let port: u16 = port_str.parse().unwrap_or(21);
        let display_name = if name.is_empty() { host.clone() } else { name };
        let profile = SiteProfile {
            name: display_name,
            host,
            port,
            user,
            pass,
            remote_dir,
        };

        let sites = unsafe { SM_SITES.as_mut().unwrap() };
        if let Some(idx) = edit_index {
            if idx < sites.len() {
                sites[idx] = profile;
            }
        } else {
            sites.push(profile);
        }
        save_sites(sites);
        // Update grid
        fn populate_site_grid_inner(grid: &anyui::DataGrid, sites: &[SiteProfile]) {
            let mut data: Vec<u8> = Vec::new();
            for site in sites {
                data.extend_from_slice(site.name.as_bytes());
                data.push(0x1F);
                data.extend_from_slice(site.host.as_bytes());
                data.push(0x1F);
                data.extend_from_slice(format!("{}", site.port).as_bytes());
                data.push(0x1F);
                data.extend_from_slice(site.user.as_bytes());
                data.push(0x1E);
            }
            grid.set_row_count(sites.len() as u32);
            grid.set_data_raw(&data);
        }
        populate_site_grid_inner(&pg, sites);
        app().sites = sites.clone();
        dlg_c2.destroy();
    });

    dlg.on_close(|_| {});
}

// Shared static for site manager dialog
static mut SM_SITES: Option<Vec<SiteProfile>> = None;

// ─── Connect Dialog ───────────────────────────────────────────────────────────

fn show_connect_dialog() {
    let dlg = anyui::Window::new_with_flags(
        "FTP Verbinden",
        -1,
        -1,
        420,
        270,
        anyui::WIN_FLAG_NOT_RESIZABLE | anyui::WIN_FLAG_NO_MAXIMIZE,
    );

    let panel = anyui::View::new();
    panel.set_dock(anyui::DOCK_FILL);
    panel.set_color(0xFF2D2D30);
    dlg.add(&panel);

    let lbl_host = anyui::Label::new("Host:");
    lbl_host.set_size(90, 24);
    lbl_host.set_position(16, 20);
    panel.add(&lbl_host);

    let fld_host = anyui::TextField::new();
    fld_host.set_size(290, 26);
    fld_host.set_position(112, 18);
    fld_host.set_placeholder("ftp.example.com");
    panel.add(&fld_host);

    let lbl_port = anyui::Label::new("Port:");
    lbl_port.set_size(90, 24);
    lbl_port.set_position(16, 58);
    panel.add(&lbl_port);

    let fld_port = anyui::TextField::new();
    fld_port.set_size(80, 26);
    fld_port.set_position(112, 56);
    fld_port.set_text("21");
    panel.add(&fld_port);

    let lbl_user = anyui::Label::new("Benutzer:");
    lbl_user.set_size(90, 24);
    lbl_user.set_position(16, 98);
    panel.add(&lbl_user);

    let fld_user = anyui::TextField::new();
    fld_user.set_size(290, 26);
    fld_user.set_position(112, 96);
    fld_user.set_placeholder("anonymous");
    panel.add(&fld_user);

    let lbl_pass = anyui::Label::new("Passwort:");
    lbl_pass.set_size(90, 24);
    lbl_pass.set_position(16, 138);
    panel.add(&lbl_pass);

    let fld_pass = anyui::TextField::new();
    fld_pass.set_size(290, 26);
    fld_pass.set_position(112, 136);
    fld_pass.set_placeholder("(leer = anonym)");
    fld_pass.set_password_mode(true);
    panel.add(&fld_pass);

    let btn_cancel = anyui::Button::new("Abbrechen");
    btn_cancel.set_size(120, 32);
    btn_cancel.set_position(16, 196);
    panel.add(&btn_cancel);

    let btn_ok = anyui::Button::new("Verbinden");
    btn_ok.set_size(120, 32);
    btn_ok.set_position(280, 196);
    panel.add(&btn_ok);

    let dlg_c = dlg.clone();
    btn_cancel.on_click(move |_| {
        dlg_c.destroy();
    });

    let dlg_c2 = dlg.clone();
    let fh = fld_host.clone();
    let fp = fld_port.clone();
    let fu = fld_user.clone();
    let fpw = fld_pass.clone();
    btn_ok.on_click(move |_| {
        let host = get_field_text(&fh);
        let port_str = get_field_text(&fp);
        let user = get_field_text(&fu);
        let pass = get_field_text(&fpw);
        let port: u16 = port_str.parse().unwrap_or(21);
        do_connect(&host, port, &user, &pass);
        // Update quick connect fields
        let a = app();
        a.qc_host.set_text(&host);
        a.qc_port.set_text(&port_str);
        a.qc_user.set_text(&user);
        a.qc_pass.set_text(&pass);
        dlg_c2.destroy();
    });

    dlg.on_close(|_| {});
}

// ─── Rename Dialog ────────────────────────────────────────────────────────────

fn show_rename_dialog(current_name: String, is_remote: bool) {
    let dlg = anyui::Window::new_with_flags(
        "Umbenennen",
        -1,
        -1,
        380,
        130,
        anyui::WIN_FLAG_NOT_RESIZABLE | anyui::WIN_FLAG_NO_MAXIMIZE,
    );
    let panel = anyui::View::new();
    panel.set_dock(anyui::DOCK_FILL);
    panel.set_color(0xFF2D2D30);
    dlg.add(&panel);

    let lbl = anyui::Label::new("Neuer Name:");
    lbl.set_size(95, 24);
    lbl.set_position(12, 16);
    panel.add(&lbl);

    let fld = anyui::TextField::new();
    fld.set_size(250, 26);
    fld.set_position(112, 14);
    fld.set_text(&current_name);
    panel.add(&fld);

    let btn_cancel = anyui::Button::new("Abbrechen");
    btn_cancel.set_size(110, 30);
    btn_cancel.set_position(12, 54);
    panel.add(&btn_cancel);

    let btn_ok = anyui::Button::new("Umbenennen");
    btn_ok.set_size(110, 30);
    btn_ok.set_position(252, 54);
    panel.add(&btn_ok);

    let dlg_c = dlg.clone();
    btn_cancel.on_click(move |_| {
        dlg_c.destroy();
    });

    let dlg_c2 = dlg.clone();
    let fld_c = fld.clone();
    let old = current_name.clone();
    btn_ok.on_click(move |_| {
        let new_name = get_field_text(&fld_c);
        if new_name.is_empty() || new_name == old {
            dlg_c2.destroy();
            return;
        }
        if is_remote {
            if WORKER_BUSY.load(Ordering::Relaxed) {
                return;
            }
            unsafe {
                write_param(&mut PARAM1, &old);
                write_param(&mut PARAM2, &new_name);
            }
            WORKER_CMD.store(CMD_RENAME, Ordering::Release);
            spawn_worker();
        } else {
            let a = app();
            let op = join_path(&a.local_dir, &old);
            let np = join_path(&a.local_dir, &new_name);
            if fs::rename(&op, &np) == 0 {
                log_line(&format!("Umbenannt: {} -> {}", old, new_name));
                refresh_local();
            } else {
                anyui::MessageBox::show(
                    anyui::MessageBoxType::Alert,
                    "Umbenennen fehlgeschlagen.",
                    None,
                );
            }
        }
        dlg_c2.destroy();
    });
    dlg.on_close(|_| {});
}

// ─── New Folder Dialog ────────────────────────────────────────────────────────

fn show_new_folder_dialog(is_remote: bool) {
    let dlg = anyui::Window::new_with_flags(
        "Neuer Ordner",
        -1,
        -1,
        360,
        120,
        anyui::WIN_FLAG_NOT_RESIZABLE | anyui::WIN_FLAG_NO_MAXIMIZE,
    );
    let panel = anyui::View::new();
    panel.set_dock(anyui::DOCK_FILL);
    panel.set_color(0xFF2D2D30);
    dlg.add(&panel);

    let lbl = anyui::Label::new("Ordnername:");
    lbl.set_size(95, 24);
    lbl.set_position(12, 16);
    panel.add(&lbl);

    let fld = anyui::TextField::new();
    fld.set_size(230, 26);
    fld.set_position(112, 14);
    fld.set_placeholder("Neuer Ordner");
    panel.add(&fld);

    let btn_cancel = anyui::Button::new("Abbrechen");
    btn_cancel.set_size(100, 30);
    btn_cancel.set_position(12, 50);
    panel.add(&btn_cancel);

    let btn_ok = anyui::Button::new("Erstellen");
    btn_ok.set_size(100, 30);
    btn_ok.set_position(244, 50);
    panel.add(&btn_ok);

    let dlg_c = dlg.clone();
    btn_cancel.on_click(move |_| {
        dlg_c.destroy();
    });

    let dlg_c2 = dlg.clone();
    let fld_c = fld.clone();
    btn_ok.on_click(move |_| {
        let name = get_field_text(&fld_c);
        if name.is_empty() {
            dlg_c2.destroy();
            return;
        }
        if is_remote {
            if WORKER_BUSY.load(Ordering::Relaxed) {
                return;
            }
            unsafe {
                write_param(&mut PARAM1, &name);
            }
            WORKER_CMD.store(CMD_MKDIR, Ordering::Release);
            spawn_worker();
        } else {
            let a = app();
            let path = join_path(&a.local_dir, &name);
            if fs::mkdir(&path) == 0 {
                log_line(&format!("Ordner erstellt: {}", name));
                refresh_local();
            } else {
                anyui::MessageBox::show(
                    anyui::MessageBoxType::Alert,
                    "Erstellen fehlgeschlagen.",
                    None,
                );
            }
        }
        dlg_c2.destroy();
    });
    dlg.on_close(|_| {});
}

// ─── Transfer Actions ─────────────────────────────────────────────────────────

fn do_download() {
    if WORKER_BUSY.load(Ordering::Relaxed) {
        return;
    }
    let a = app();
    if !a.is_connected {
        return;
    }
    let row = a.remote_grid.selected_row() as usize;
    if row >= a.remote_files.len() {
        return;
    }
    let entry = a.remote_files[row].clone();
    if entry.is_dir || entry.name == ".." {
        return;
    }
    let local_path = join_path(&a.local_dir, &entry.name);
    log_line(&format!("Download: {} -> {}", entry.name, local_path));
    unsafe {
        write_param(&mut PARAM1, &entry.name);
        write_param(&mut PARAM2, &local_path);
    }
    WORKER_CMD.store(CMD_DOWNLOAD, Ordering::Release);
    spawn_worker();
}

fn do_upload() {
    if WORKER_BUSY.load(Ordering::Relaxed) {
        return;
    }
    let a = app();
    if !a.is_connected {
        return;
    }
    let row = a.local_grid.selected_row() as usize;
    if row >= a.local_files.len() {
        return;
    }
    let entry = a.local_files[row].clone();
    if entry.is_dir || entry.name == ".." {
        return;
    }
    let local_path = join_path(&a.local_dir, &entry.name);
    log_line(&format!(
        "Upload: {} -> {}/{}",
        local_path, a.remote_dir, entry.name
    ));
    unsafe {
        write_param(&mut PARAM1, &local_path);
        write_param(&mut PARAM2, &entry.name);
    }
    WORKER_CMD.store(CMD_UPLOAD, Ordering::Release);
    spawn_worker();
}

fn do_delete_remote(entry: FileEntry) {
    if WORKER_BUSY.load(Ordering::Relaxed) {
        return;
    }
    unsafe {
        write_param(&mut PARAM1, &entry.name);
    }
    PARAM_PORT.store(if entry.is_dir { 1 } else { 0 }, Ordering::Relaxed);
    WORKER_CMD.store(CMD_DELETE, Ordering::Release);
    spawn_worker();
}

fn do_delete() {
    let a = app();
    if a.focus_pane == 1 {
        if !a.is_connected {
            return;
        }
        let row = a.remote_grid.selected_row() as usize;
        if row >= a.remote_files.len() {
            return;
        }
        let entry = a.remote_files[row].clone();
        if entry.name == ".." {
            return;
        }
        do_delete_remote(entry);
    } else {
        let row = a.local_grid.selected_row() as usize;
        if row >= a.local_files.len() {
            return;
        }
        let entry = a.local_files[row].clone();
        if entry.name == ".." {
            return;
        }
        let path = join_path(&a.local_dir, &entry.name);
        if fs::unlink(&path) == 0 {
            log_line(&format!("Geloescht: {}", entry.name));
            refresh_local();
        } else {
            anyui::MessageBox::show(
                anyui::MessageBoxType::Alert,
                "Loeschen fehlgeschlagen.",
                None,
            );
        }
    }
}

fn do_rename() {
    let a = app();
    if a.focus_pane == 1 {
        if !a.is_connected {
            return;
        }
        let row = a.remote_grid.selected_row() as usize;
        if row >= a.remote_files.len() {
            return;
        }
        let name = a.remote_files[row].name.clone();
        if name == ".." {
            return;
        }
        show_rename_dialog(name, true);
    } else {
        let row = a.local_grid.selected_row() as usize;
        if row >= a.local_files.len() {
            return;
        }
        let name = a.local_files[row].name.clone();
        if name == ".." {
            return;
        }
        show_rename_dialog(name, false);
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() {
        return;
    }

    let prefs = load_prefs();
    let win = anyui::Window::new(
        "anyzilla",
        prefs.win_x,
        prefs.win_y,
        prefs.win_w,
        prefs.win_h,
    );

    // ── Toolbar ───────────────────────────────────────────────────────────────
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(1100, 46);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(4, 4, 4, 4);

    let btn_sites = toolbar.add_icon_button("");
    btn_sites.set_size(34, 34);
    btn_sites.set_system_icon("server", IconType::Outline, 0xFF88AACC, 24);
    btn_sites.set_tooltip("Site Manager");

    let btn_connect = toolbar.add_icon_button("");
    btn_connect.set_size(34, 34);
    btn_connect.set_system_icon("plug", IconType::Outline, 0xFF88CC44, 24);
    btn_connect.set_tooltip("Verbinden");

    let btn_disconnect = toolbar.add_icon_button("");
    btn_disconnect.set_size(34, 34);
    btn_disconnect.set_system_icon("plug-x", IconType::Outline, 0xFFCC4444, 24);
    btn_disconnect.set_tooltip("Trennen");
    btn_disconnect.set_enabled(false);

    let _ = toolbar.add_separator();

    let btn_upload = toolbar.add_icon_button("");
    btn_upload.set_size(34, 34);
    btn_upload.set_system_icon("upload", IconType::Outline, 0xFF4488CC, 24);
    btn_upload.set_tooltip("Hochladen (lokal -> remote)");
    btn_upload.set_enabled(false);

    let btn_download = toolbar.add_icon_button("");
    btn_download.set_size(34, 34);
    btn_download.set_system_icon("download", IconType::Outline, 0xFF44AACC, 24);
    btn_download.set_tooltip("Herunterladen (remote -> lokal)");
    btn_download.set_enabled(false);

    let _ = toolbar.add_separator();

    let btn_delete = toolbar.add_icon_button("");
    btn_delete.set_size(34, 34);
    btn_delete.set_system_icon("trash", IconType::Outline, 0xFFCC6644, 24);
    btn_delete.set_tooltip("Loeschen (Entf)");
    btn_delete.set_enabled(false);

    let btn_rename = toolbar.add_icon_button("");
    btn_rename.set_size(34, 34);
    btn_rename.set_system_icon("pencil", IconType::Outline, 0xFFCCCCCC, 24);
    btn_rename.set_tooltip("Umbenennen (F2)");
    btn_rename.set_enabled(false);

    let btn_newfolder = toolbar.add_icon_button("");
    btn_newfolder.set_size(34, 34);
    btn_newfolder.set_system_icon("folder-plus", IconType::Outline, 0xFFDDAA44, 24);
    btn_newfolder.set_tooltip("Neuer Ordner (F7)");
    btn_newfolder.set_enabled(false);

    let _ = toolbar.add_separator();

    let btn_refresh = toolbar.add_icon_button("");
    btn_refresh.set_size(34, 34);
    btn_refresh.set_system_icon("refresh", IconType::Outline, 0xFFAAAAAA, 24);
    btn_refresh.set_tooltip("Aktualisieren");

    let _ = toolbar.add_separator();

    let btn_log_toggle = toolbar.add_icon_button("");
    btn_log_toggle.set_size(34, 34);
    btn_log_toggle.set_system_icon("terminal", IconType::Outline, 0xFF888888, 24);
    btn_log_toggle.set_tooltip("Log ein-/ausblenden");

    let _ = toolbar.add_separator();

    let btn_hidden = toolbar.add_icon_button("");
    btn_hidden.set_size(34, 34);
    btn_hidden.set_system_icon("eye", IconType::Outline, 0xFF888888, 24);
    btn_hidden.set_tooltip("Versteckte Dateien ein-/ausblenden");

    let btn_sort_name = toolbar.add_icon_button("");
    btn_sort_name.set_size(34, 34);
    btn_sort_name.set_system_icon("sort-ascending-letters", IconType::Outline, 0xFF888888, 24);
    btn_sort_name.set_tooltip("Nach Name sortieren");

    let btn_sort_size = toolbar.add_icon_button("");
    btn_sort_size.set_size(34, 34);
    btn_sort_size.set_system_icon("sort-ascending-numbers", IconType::Outline, 0xFF888888, 24);
    btn_sort_size.set_tooltip("Nach Groesse sortieren");

    let btn_sort_date = toolbar.add_icon_button("");
    btn_sort_date.set_size(34, 34);
    btn_sort_date.set_system_icon("calendar-event", IconType::Outline, 0xFF888888, 24);
    btn_sort_date.set_tooltip("Nach Datum sortieren");

    win.add(&toolbar);

    // ── Menu Bar ─────────────────────────────────────────────────────────────
    let mut mb = anyui::MenuBarBuilder::new()
        .menu("File")
        .item(1, "Site Manager", 0)
        .separator()
        .item(2, "Quit", 0)
        .end_menu()
        .menu("Server")
        .item(10, "Connect", 0)
        .item(11, "Disconnect", 0)
        .separator()
        .item(12, "Refresh", 0)
        .end_menu()
        .menu("Transfer")
        .item(20, "Upload", 0)
        .item(21, "Download", 0)
        .end_menu();
    let menu_data = mb.build();
    let menu = anyui::MenuBar::set(win.id(), menu_data);
    menu.on_item(|e| match e.item_id {
        1 => {
            show_site_manager();
        }
        2 => {
            anyui::quit();
        }
        10 => {
            show_connect_dialog();
        }
        11 => {
            if WORKER_BUSY.load(Ordering::Relaxed) {
                return;
            }
            WORKER_CMD.store(CMD_DISCONNECT, Ordering::Release);
            spawn_worker();
            let a = app();
            a.remote_files = Vec::new();
            let empty: Vec<FileEntry> = Vec::new();
            populate_grid(&a.remote_grid, &empty);
            a.remote_path_label.set_text("Remote: (nicht verbunden)");
            set_connected_state(false);
            update_status();
        }
        12 => {
            refresh_local();
            if !WORKER_BUSY.load(Ordering::Relaxed) && app().is_connected {
                WORKER_CMD.store(CMD_LIST, Ordering::Release);
                spawn_worker();
            }
        }
        20 => {
            do_upload();
        }
        21 => {
            do_download();
        }
        _ => {}
    });

    // ── Quick Connect Bar ────────────────────────────────────────────────────
    let qc_bar = anyui::View::new();
    qc_bar.set_dock(anyui::DOCK_TOP);
    qc_bar.set_size(1100, 34);
    qc_bar.set_color(0xFF2D2D30);

    let qc_lbl_host = anyui::Label::new("Host:");
    qc_lbl_host.set_size(36, 24);
    qc_lbl_host.set_position(8, 5);
    qc_lbl_host.set_text_color(0xFFAAAAAA);
    qc_bar.add(&qc_lbl_host);

    let qc_host = anyui::TextField::new();
    qc_host.set_size(200, 24);
    qc_host.set_position(48, 5);
    qc_host.set_placeholder("ftp.example.com");
    if !prefs.last_host.is_empty() {
        qc_host.set_text(&prefs.last_host);
    }
    qc_bar.add(&qc_host);

    let qc_lbl_port = anyui::Label::new("Port:");
    qc_lbl_port.set_size(32, 24);
    qc_lbl_port.set_position(258, 5);
    qc_lbl_port.set_text_color(0xFFAAAAAA);
    qc_bar.add(&qc_lbl_port);

    let qc_port = anyui::TextField::new();
    qc_port.set_size(50, 24);
    qc_port.set_position(294, 5);
    qc_port.set_text(&format!("{}", prefs.last_port));
    qc_bar.add(&qc_port);

    let qc_lbl_user = anyui::Label::new("User:");
    qc_lbl_user.set_size(36, 24);
    qc_lbl_user.set_position(354, 5);
    qc_lbl_user.set_text_color(0xFFAAAAAA);
    qc_bar.add(&qc_lbl_user);

    let qc_user = anyui::TextField::new();
    qc_user.set_size(140, 24);
    qc_user.set_position(394, 5);
    qc_user.set_placeholder("anonymous");
    if !prefs.last_user.is_empty() {
        qc_user.set_text(&prefs.last_user);
    }
    qc_bar.add(&qc_user);

    let qc_lbl_pass = anyui::Label::new("Pass:");
    qc_lbl_pass.set_size(36, 24);
    qc_lbl_pass.set_position(544, 5);
    qc_lbl_pass.set_text_color(0xFFAAAAAA);
    qc_bar.add(&qc_lbl_pass);

    let qc_pass = anyui::TextField::new();
    qc_pass.set_size(140, 24);
    qc_pass.set_position(584, 5);
    qc_pass.set_password_mode(true);
    if !prefs.last_pass.is_empty() {
        qc_pass.set_text(&prefs.last_pass);
    }
    qc_bar.add(&qc_pass);

    let qc_btn = anyui::Button::new("Verbinden");
    qc_btn.set_size(90, 24);
    qc_btn.set_position(738, 5);
    qc_bar.add(&qc_btn);

    let qc_btn_save = anyui::Button::new("Speichern");
    qc_btn_save.set_size(80, 24);
    qc_btn_save.set_position(838, 5);
    qc_bar.add(&qc_btn_save);

    win.add(&qc_bar);

    // ── Status bar ────────────────────────────────────────────────────────────
    let status_bar = anyui::View::new();
    status_bar.set_dock(anyui::DOCK_BOTTOM);
    status_bar.set_size(1100, 24);
    status_bar.set_color(0xFF252525);
    let status_label = anyui::Label::new("Nicht verbunden");
    status_label.set_dock(anyui::DOCK_FILL);
    status_label.set_color(0xFF252525);
    status_label.set_text_color(0xFF888888);
    status_label.set_padding(8, 0, 0, 0);
    status_bar.add(&status_label);
    win.add(&status_bar);

    // ── Log panel ─────────────────────────────────────────────────────────────
    let log_view = anyui::View::new();
    log_view.set_color(0xFF1A1A1A);
    let log_editor = anyui::TextEditor::new(1100, 120);
    log_editor.set_dock(anyui::DOCK_FILL);
    log_editor.set_color(0xFF1A1A1A);
    log_editor.set_text_color(0xFF88CC88);
    log_editor.set_read_only(true);
    log_view.add(&log_editor);

    // ── Main split view (local / remote) ─────────────────────────────────────
    let split = anyui::SplitView::new();
    split.set_dock(anyui::DOCK_FILL);
    split.set_orientation(anyui::ORIENTATION_HORIZONTAL);
    split.set_split_ratio(50);

    // Local pane
    let local_pane = anyui::View::new();
    local_pane.set_color(0xFF1E1E1E);
    let local_path_bar = anyui::View::new();
    local_path_bar.set_dock(anyui::DOCK_TOP);
    local_path_bar.set_size(500, 28);
    local_path_bar.set_color(0xFF2D2D30);
    let local_path_label = anyui::Label::new("Lokal: /");
    local_path_label.set_dock(anyui::DOCK_FILL);
    local_path_label.set_color(0xFF2D2D30);
    local_path_label.set_text_color(0xFFCCCCCC);
    local_path_label.set_padding(8, 0, 0, 0);
    local_path_bar.add(&local_path_label);
    let local_path_field = anyui::TextField::new();
    local_path_field.set_dock(anyui::DOCK_FILL);
    local_path_field.set_visible(false);
    local_path_bar.add(&local_path_field);
    local_pane.add(&local_path_bar);
    let local_grid = anyui::DataGrid::new(500, 500);
    local_grid.set_dock(anyui::DOCK_FILL);
    local_grid.set_color(0xFF1E1E1E);
    local_grid.set_columns(&[
        anyui::ColumnDef::new("Name").width(280),
        anyui::ColumnDef::new("Groesse")
            .width(80)
            .align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new("Datum").width(110),
    ]);
    local_pane.add(&local_grid);
    split.add(&local_pane);

    // Remote pane
    let remote_pane = anyui::View::new();
    remote_pane.set_color(0xFF1E1E1E);
    let remote_path_bar = anyui::View::new();
    remote_path_bar.set_dock(anyui::DOCK_TOP);
    remote_path_bar.set_size(500, 28);
    remote_path_bar.set_color(0xFF2D2D30);
    let remote_path_label = anyui::Label::new("Remote: (nicht verbunden)");
    remote_path_label.set_dock(anyui::DOCK_FILL);
    remote_path_label.set_color(0xFF2D2D30);
    remote_path_label.set_text_color(0xFFCCCCCC);
    remote_path_label.set_padding(8, 0, 0, 0);
    remote_path_bar.add(&remote_path_label);
    let remote_path_field = anyui::TextField::new();
    remote_path_field.set_dock(anyui::DOCK_FILL);
    remote_path_field.set_visible(false);
    remote_path_bar.add(&remote_path_field);
    remote_pane.add(&remote_path_bar);
    let remote_grid = anyui::DataGrid::new(500, 500);
    remote_grid.set_dock(anyui::DOCK_FILL);
    remote_grid.set_color(0xFF1E1E1E);
    remote_grid.set_columns(&[
        anyui::ColumnDef::new("Name").width(280),
        anyui::ColumnDef::new("Groesse")
            .width(80)
            .align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new("Datum").width(110),
    ]);
    remote_pane.add(&remote_grid);
    split.add(&remote_pane);

    // ── Vertical split: file panes (top) + log (bottom) ─────────────────────
    let vsplit = anyui::SplitView::new();
    vsplit.set_dock(anyui::DOCK_FILL);
    vsplit.set_orientation(anyui::ORIENTATION_VERTICAL);
    vsplit.set_split_ratio(75);
    vsplit.set_min_split(30);
    vsplit.set_max_split(95);
    vsplit.set_resizable(true);
    vsplit.add(&split);
    vsplit.add(&log_view);
    win.add(&vsplit);

    // Context menus (extended with sort options)
    let local_menu = anyui::ContextMenu::new("Hochladen|Umbenennen|Loeschen|Neuer Ordner|Aktualisieren|-|Sort: Name|Sort: Groesse|Sort: Datum|Pfad bearbeiten");
    local_grid.set_context_menu(&local_menu);
    let remote_menu = anyui::ContextMenu::new("Herunterladen|Umbenennen|Loeschen|Neuer Ordner|Aktualisieren|-|Sort: Name|Sort: Groesse|Sort: Datum|Pfad bearbeiten");
    remote_grid.set_context_menu(&remote_menu);

    // ── Initialize App State ──────────────────────────────────────────────────
    let home = get_home_dir();
    let initial_sites = load_sites();
    unsafe {
        APP = Some(AppState {
            remote_host: String::new(),
            remote_dir: "/".to_string(),
            remote_files: Vec::new(),
            local_dir: home.clone(),
            local_files: Vec::new(),
            focus_pane: 0,
            log_visible: true,
            is_connected: false,
            startup_timer_id: 0,
            poll_timer_id: 0,
            sites: initial_sites,
            last_host: String::new(),
            last_port: 21,
            last_user: String::new(),
            last_pass: String::new(),
            reconnect_attempts: 0,
            initial_remote_dir: String::new(),
            local_sort_col: SortColumn::Name,
            local_sort_order: SortOrder::Asc,
            remote_sort_col: SortColumn::Name,
            remote_sort_order: SortOrder::Asc,
            show_hidden: false,
            local_path_editing: false,
            remote_path_editing: false,
            qc_host: qc_host.clone(),
            qc_port: qc_port.clone(),
            qc_user: qc_user.clone(),
            qc_pass: qc_pass.clone(),
            local_path_bar: local_path_bar.clone(),
            local_path_label: local_path_label.clone(),
            local_path_field: local_path_field.clone(),
            remote_path_bar: remote_path_bar.clone(),
            remote_path_label: remote_path_label.clone(),
            remote_path_field: remote_path_field.clone(),
            win: win.clone(),
            local_grid: local_grid.clone(),
            remote_grid: remote_grid.clone(),
            log_view: log_view.clone(),
            log_editor: log_editor.clone(),
            status_label: status_label.clone(),
            btn_disconnect: btn_disconnect.clone(),
            btn_upload: btn_upload.clone(),
            btn_download: btn_download.clone(),
            btn_delete: btn_delete.clone(),
            btn_rename: btn_rename.clone(),
            btn_newfolder: btn_newfolder.clone(),
        });
    }

    // ── Toolbar Callbacks ─────────────────────────────────────────────────────

    btn_sites.on_click(|_| {
        show_site_manager();
    });

    btn_connect.on_click(|_| {
        show_connect_dialog();
    });

    // Quick connect bar callbacks
    qc_btn.on_click(|_| {
        do_quick_connect();
    });
    qc_host.on_submit(|_| {
        do_quick_connect();
    });
    qc_pass.on_submit(|_| {
        do_quick_connect();
    });

    let qc_host_c = qc_host.clone();
    let qc_port_c = qc_port.clone();
    let qc_user_c = qc_user.clone();
    let qc_pass_c = qc_pass.clone();
    qc_btn_save.on_click(move |_| {
        let host = get_field_text(&qc_host_c);
        let port_str = get_field_text(&qc_port_c);
        let user = get_field_text(&qc_user_c);
        let pass = get_field_text(&qc_pass_c);
        if host.is_empty() {
            anyui::MessageBox::show(
                anyui::MessageBoxType::Alert,
                "Host darf nicht leer sein.",
                None,
            );
            return;
        }
        let port: u16 = port_str.parse().unwrap_or(21);
        let profile = SiteProfile {
            name: host.clone(),
            host: host.clone(),
            port,
            user,
            pass,
            remote_dir: String::new(),
        };
        let a = app();
        a.sites.push(profile);
        save_sites(&a.sites);
        log_line(&format!("Profil gespeichert: {}", host));
    });

    btn_disconnect.on_click(|_| {
        if WORKER_BUSY.load(Ordering::Relaxed) {
            return;
        }
        WORKER_CMD.store(CMD_DISCONNECT, Ordering::Release);
        spawn_worker();
        let a = app();
        a.remote_files = Vec::new();
        let empty: Vec<FileEntry> = Vec::new();
        populate_grid(&a.remote_grid, &empty);
        a.remote_path_label.set_text("Remote: (nicht verbunden)");
        set_connected_state(false);
        update_status();
    });

    btn_upload.on_click(|_| {
        do_upload();
    });
    btn_download.on_click(|_| {
        do_download();
    });
    btn_delete.on_click(|_| {
        do_delete();
    });
    btn_rename.on_click(|_| {
        do_rename();
    });

    btn_newfolder.on_click(|_| {
        show_new_folder_dialog(app().focus_pane == 1);
    });

    btn_refresh.on_click(|_| {
        refresh_local();
        if !WORKER_BUSY.load(Ordering::Relaxed) && app().is_connected {
            WORKER_CMD.store(CMD_LIST, Ordering::Release);
            spawn_worker();
        }
    });

    btn_log_toggle.on_click(|_| {
        let a = app();
        a.log_visible = !a.log_visible;
        a.log_view.set_visible(a.log_visible);
    });

    btn_hidden.on_click(|_| {
        let a = app();
        a.show_hidden = !a.show_hidden;
        SHOW_HIDDEN.store(a.show_hidden, Ordering::Relaxed);
        let state = if a.show_hidden { "an" } else { "aus" };
        log_line(&format!("Versteckte Dateien: {}", state));
        refresh_local();
        if !WORKER_BUSY.load(Ordering::Relaxed) && a.is_connected {
            WORKER_CMD.store(CMD_LIST, Ordering::Release);
            spawn_worker();
        }
    });

    btn_sort_name.on_click(|_| {
        cycle_sort(app().focus_pane, SortColumn::Name);
    });
    btn_sort_size.on_click(|_| {
        cycle_sort(app().focus_pane, SortColumn::Size);
    });
    btn_sort_date.on_click(|_| {
        cycle_sort(app().focus_pane, SortColumn::Date);
    });

    // Path label click → toggle path editing
    local_path_label.on_click(|_| {
        toggle_path_editing(0);
    });
    remote_path_label.on_click(|_| {
        toggle_path_editing(1);
    });

    // Path field submit → navigate to entered path
    local_path_field.on_submit(|_| {
        let a = app();
        let path = get_field_text(&a.local_path_field);
        if !path.is_empty() {
            a.local_dir = path;
            refresh_local();
        }
        a.local_path_editing = false;
        a.local_path_label.set_visible(true);
        a.local_path_field.set_visible(false);
    });

    remote_path_field.on_submit(|_| {
        let a = app();
        if !a.is_connected {
            return;
        }
        let path = get_field_text(&a.remote_path_field);
        if !path.is_empty() && !WORKER_BUSY.load(Ordering::Relaxed) {
            unsafe {
                write_param(&mut PARAM1, &path);
            }
            WORKER_CMD.store(CMD_CD, Ordering::Release);
            spawn_worker();
        }
        a.remote_path_editing = false;
        a.remote_path_label.set_visible(true);
        a.remote_path_field.set_visible(false);
    });

    // ── Grid Callbacks ────────────────────────────────────────────────────────

    local_grid.on_selection_changed(|_| {
        app().focus_pane = 0;
    });

    local_grid.on_submit(|e| {
        let a = app();
        let row = e.index as usize;
        if row >= a.local_files.len() {
            return;
        }
        let entry = a.local_files[row].clone();
        if entry.is_dir {
            let new_dir = join_path(&a.local_dir, &entry.name);
            a.local_dir = new_dir;
            refresh_local();
        }
    });

    remote_grid.on_selection_changed(|_| {
        app().focus_pane = 1;
    });

    remote_grid.on_submit(|e| {
        let a = app();
        if !a.is_connected {
            return;
        }
        if WORKER_BUSY.load(Ordering::Relaxed) {
            return;
        }
        let row = e.index as usize;
        if row >= a.remote_files.len() {
            return;
        }
        let entry = a.remote_files[row].clone();
        if entry.is_dir {
            log_line(&format!("Wechsle in Verzeichnis: {}", entry.name));
            unsafe {
                write_param(&mut PARAM1, &entry.name);
            }
            WORKER_CMD.store(CMD_CD, Ordering::Release);
            spawn_worker();
        } else {
            do_download();
        }
    });

    // ── Context Menu Callbacks ────────────────────────────────────────────────

    local_menu.on_item_click(|e| {
        match e.index {
            0 => do_upload(),
            1 => {
                let a = app();
                let row = a.local_grid.selected_row() as usize;
                if row < a.local_files.len() {
                    let name = a.local_files[row].name.clone();
                    show_rename_dialog(name, false);
                }
            }
            2 => {
                app().focus_pane = 0;
                do_delete();
            }
            3 => show_new_folder_dialog(false),
            4 => refresh_local(),
            // 5 = separator
            6 => cycle_sort(0, SortColumn::Name),
            7 => cycle_sort(0, SortColumn::Size),
            8 => cycle_sort(0, SortColumn::Date),
            9 => toggle_path_editing(0),
            _ => {}
        }
    });

    remote_menu.on_item_click(|e| {
        match e.index {
            0 => do_download(),
            1 => {
                let a = app();
                let row = a.remote_grid.selected_row() as usize;
                if row < a.remote_files.len() {
                    let name = a.remote_files[row].name.clone();
                    show_rename_dialog(name, true);
                }
            }
            2 => {
                app().focus_pane = 1;
                do_delete();
            }
            3 => show_new_folder_dialog(true),
            4 => {
                if !WORKER_BUSY.load(Ordering::Relaxed) && app().is_connected {
                    WORKER_CMD.store(CMD_LIST, Ordering::Release);
                    spawn_worker();
                }
            }
            // 5 = separator
            6 => cycle_sort(1, SortColumn::Name),
            7 => cycle_sort(1, SortColumn::Size),
            8 => cycle_sort(1, SortColumn::Date),
            9 => toggle_path_editing(1),
            _ => {}
        }
    });

    // ── Keyboard Shortcuts ────────────────────────────────────────────────────

    win.on_key_down(|e| {
        match e.keycode {
            anyui::KEY_F2 => do_rename(),
            anyui::KEY_DELETE => do_delete(),
            anyui::KEY_F5 => {
                if app().focus_pane == 1 {
                    do_download();
                } else {
                    do_upload();
                }
            }
            anyui::KEY_F7 => show_new_folder_dialog(app().focus_pane == 1),
            anyui::KEY_ESCAPE => {
                // Close path editing if active
                let a = app();
                if a.local_path_editing {
                    a.local_path_editing = false;
                    a.local_path_label.set_visible(true);
                    a.local_path_field.set_visible(false);
                }
                if a.remote_path_editing {
                    a.remote_path_editing = false;
                    a.remote_path_label.set_visible(true);
                    a.remote_path_field.set_visible(false);
                }
            }
            _ => {
                // Ctrl+L → toggle path editing for focused pane
                if e.keycode == 0x4C && (e.modifiers & anyui::MOD_CTRL) != 0 {
                    toggle_path_editing(app().focus_pane);
                }
                // Ctrl+H → toggle hidden files
                if e.keycode == 0x48 && (e.modifiers & anyui::MOD_CTRL) != 0 {
                    let a = app();
                    a.show_hidden = !a.show_hidden;
                    SHOW_HIDDEN.store(a.show_hidden, Ordering::Relaxed);
                    refresh_local();
                    if !WORKER_BUSY.load(Ordering::Relaxed) && a.is_connected {
                        WORKER_CMD.store(CMD_LIST, Ordering::Release);
                        spawn_worker();
                    }
                }
            }
        }
    });

    let win_c = win.clone();
    win.on_close(move |_| {
        // Save window position/size (only if valid — compositor may return 0 during teardown)
        let (w, h) = win_c.get_size();
        let (x, y) = win_c.get_position();
        if w >= 200 && h >= 100 {
            let a = app();
            save_prefs(&Prefs {
                win_x: x,
                win_y: y,
                win_w: w,
                win_h: h,
                last_host: a.last_host.clone(),
                last_port: a.last_port,
                last_user: a.last_user.clone(),
                last_pass: a.last_pass.clone(),
            });
        }
        // Disconnect FTP and kill worker thread
        if WORKER_SPAWNED.load(Ordering::Relaxed) {
            WORKER_CMD.store(CMD_EXIT, Ordering::Release);
            anyos_std::process::sleep(50);
            // Force-kill worker if still blocked in tcp_recv
            kill_worker();
        } else {
            if let Some(mut ftp) = unsafe { FTP_CLIENT.take() } {
                ftp.disconnect();
            }
        }
        anyui::quit();
    });

    // ── Initial Load + Poll Timer ─────────────────────────────────────────────
    let startup_timer_id = anyui::set_timer(100, || {
        let tid = app().startup_timer_id;
        anyui::kill_timer(tid);
        app().startup_timer_id = 0;
        refresh_local();
        log_line("anyzilla bereit. Verbindung ueber Toolbar herstellen.");
    });
    app().startup_timer_id = startup_timer_id;

    // Poll worker results every 200ms
    let poll_id = anyui::set_timer(200, || {
        poll_worker();
    });
    app().poll_timer_id = poll_id;

    anyui::run();

    // Ensure worker thread is killed before process exit
    kill_worker();
}
