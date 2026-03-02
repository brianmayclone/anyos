#![no_std]
#![no_main]

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;
use core::sync::atomic::{AtomicU32, AtomicBool, Ordering};

use anyos_std::{net, fs, env};
use libanyui_client as anyui;
use anyui::IconType;

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

struct FtpClient {
    ctrl: u32,
}

// ─── FTP Protocol ─────────────────────────────────────────────────────────────

impl FtpClient {
    fn connect(ip: &[u8; 4], port: u16) -> Option<FtpClient> {
        let sock = net::tcp_connect(ip, port, CONNECT_TIMEOUT);
        if sock == u32::MAX { return None; }
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
        if resp.starts_with("230") { return true; }
        if !resp.starts_with("331") { return false; }
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
            if n == 0 || n == u32::MAX { break; }
            result.extend_from_slice(&buf[..n as usize]);
            if is_complete_response(&result) { break; }
        }
        String::from_utf8_lossy(&result).into_owned()
    }

    /// Send PASV and return (ip, port) without connecting yet.
    fn pasv_addr(&mut self) -> Option<([u8; 4], u16)> {
        self.send_cmd_only("PASV");
        let resp = self.read_response();
        if !resp.starts_with("227") { return None; }
        parse_pasv(&resp)
    }

    fn list_dir(&mut self) -> Vec<FileEntry> {
        let (ip, port) = match self.pasv_addr() { Some(v) => v, None => return Vec::new() };
        // Send command first, then open data connection (RFC 959 / server expects this order).
        self.send_cmd_only("LIST");
        let data_sock = net::tcp_connect(&ip, port, CONNECT_TIMEOUT);
        if data_sock == u32::MAX { return Vec::new(); }
        let resp = self.read_response();
        if !resp.starts_with("150") && !resp.starts_with("125") {
            net::tcp_close(data_sock);
            return Vec::new();
        }
        let mut raw = Vec::new();
        let mut buf = [0u8; RECV_BUF];
        loop {
            let n = net::tcp_recv(data_sock, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            raw.extend_from_slice(&buf[..n as usize]);
        }
        net::tcp_close(data_sock);
        let _ = self.read_response();

        let text = String::from_utf8_lossy(&raw).into_owned();
        let mut entries = Vec::new();
        entries.push(FileEntry { name: "..".into(), size: 0, is_dir: true, modified: "".into() });
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
        if !resp.starts_with("350") { return false; }
        self.send_command("RNTO ", new_name);
        let resp = self.read_response();
        resp.starts_with("250")
    }

    fn download(&mut self, remote_name: &str, local_path: &str) -> u32 {
        self.set_binary_mode();
        let (ip, port) = match self.pasv_addr() { Some(v) => v, None => return 0 };
        self.send_command("RETR ", remote_name);
        let data_sock = net::tcp_connect(&ip, port, CONNECT_TIMEOUT);
        if data_sock == u32::MAX { return 0; }
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
            if n == 0 || n == u32::MAX { break; }
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
        if fd == u32::MAX { return 0; }
        let mut file_data = Vec::new();
        let mut buf = [0u8; RECV_BUF];
        loop {
            let n = fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            file_data.extend_from_slice(&buf[..n as usize]);
        }
        fs::close(fd);
        let (ip, port) = match self.pasv_addr() { Some(v) => v, None => return 0 };
        self.send_command("STOR ", remote_name);
        let data_sock = net::tcp_connect(&ip, port, CONNECT_TIMEOUT);
        if data_sock == u32::MAX { return 0; }
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
            if sent == u32::MAX { break; }
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
    if parts.len() < 6 { return None; }
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
    if parts.len() < 9 { return None; }
    let is_dir = parts[0].starts_with('d');
    let size: u64 = parts[4].parse().ok()?;
    let modified = format!("{} {} {}", parts[5], parts[6], parts[7]);
    let name = parts[8..].join(" ");
    if name.is_empty() { return None; }
    Some(FileEntry { name, size, is_dir, modified })
}

fn format_size(bytes: u64) -> String {
    if bytes == 0 { return "-".to_string(); }
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

// ─── Local Filesystem ─────────────────────────────────────────────────────────

fn list_local_dir(path: &str) -> Vec<FileEntry> {
    let mut buf = [0u8; 64 * 256];
    let count = fs::readdir(path, &mut buf);
    if count == u32::MAX { return Vec::new(); }
    let mut entries = Vec::new();
    if path != "/" {
        entries.push(FileEntry { name: "..".into(), size: 0, is_dir: true, modified: "".into() });
    }
    for i in 0..count as usize {
        let base = i * 64;
        if base + 64 > buf.len() { break; }
        let entry_type = buf[base];
        let name_len = buf[base + 1] as usize;
        let size = u32::from_le_bytes([buf[base + 4], buf[base + 5], buf[base + 6], buf[base + 7]]) as u64;
        if name_len == 0 || name_len > 56 { continue; }
        let name_bytes = &buf[base + 8..base + 8 + name_len];
        let name = match core::str::from_utf8(name_bytes) {
            Ok(s) => s.to_string(),
            Err(_) => continue,
        };
        if name == "." { continue; }
        let is_dir = entry_type == 2;
        entries.push(FileEntry { name, size, is_dir, modified: "".into() });
    }
    entries.sort_by(|a, b| {
        if a.name == ".." { return core::cmp::Ordering::Less; }
        if b.name == ".." { return core::cmp::Ordering::Greater; }
        match (a.is_dir, b.is_dir) {
            (true, false) => core::cmp::Ordering::Less,
            (false, true) => core::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });
    entries
}

fn join_path(base: &str, name: &str) -> String {
    if name == ".." {
        if base == "/" { return "/".to_string(); }
        if let Some(pos) = base.rfind('/') {
            if pos == 0 { return "/".to_string(); }
            return base[..pos].to_string();
        }
        return "/".to_string();
    }
    if base.ends_with('/') { format!("{}{}", base, name) }
    else { format!("{}/{}", base, name) }
}

fn get_home_dir() -> String {
    let mut buf = [0u8; 256];
    let len = env::get("HOME", &mut buf);
    if len > 0 {
        if let Ok(s) = core::str::from_utf8(&buf[..len as usize]) {
            return s.trim().to_string();
        }
    }
    "/home/user".to_string()
}

fn get_field_text(field: &anyui::TextField) -> String {
    let mut buf = [0u8; 512];
    let len = field.get_text(&mut buf) as usize;
    let len = len.min(buf.len());
    core::str::from_utf8(&buf[..len]).unwrap_or("").trim().to_string()
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
const CMD_IDLE: u32      = 0;
const CMD_CONNECT: u32   = 1;
const CMD_LIST: u32      = 2;
const CMD_DOWNLOAD: u32  = 3;
const CMD_UPLOAD: u32    = 4;
const CMD_CD: u32        = 5;
const CMD_MKDIR: u32     = 6;
const CMD_DELETE: u32    = 7;
const CMD_RENAME: u32    = 8;
const CMD_DISCONNECT: u32 = 9;

// Result codes
const RES_NONE: u32    = 0;
const RES_OK: u32      = 1;
const RES_ERROR: u32   = 2;
const RES_BUSY: u32    = 3;

static WORKER_CMD:    AtomicU32 = AtomicU32::new(CMD_IDLE);
static WORKER_RESULT: AtomicU32 = AtomicU32::new(RES_NONE);
static WORKER_BUSY:   AtomicBool = AtomicBool::new(false);

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
    // Each call to this function handles exactly one command, then exits.
    // A new thread is spawned for each command.
    let cmd = WORKER_CMD.load(Ordering::Acquire);

    match cmd {
        CMD_CONNECT => {
            let host = unsafe { read_cstr(&PARAM1) }.to_string();
            let user = unsafe { read_cstr(&PARAM2) }.to_string();
            let pass = unsafe { read_cstr(&PARAM3) }.to_string();
            let port = PARAM_PORT.load(Ordering::Relaxed) as u16;

            let mut ip = [0u8; 4];
            if net::dns(&host, &mut ip) != 0 {
                unsafe { write_result_str(&format!("DNS-Fehler: {}", host)); }
                WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                WORKER_CMD.store(CMD_IDLE, Ordering::Release);
                WORKER_BUSY.store(false, Ordering::Release);
                return;
            }

            match FtpClient::connect(&ip, port) {
                None => {
                    unsafe { write_result_str(&format!("Verbindungsfehler: {}:{}", host, port)); }
                    WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                }
                Some(mut ftp) => {
                    if !ftp.login(&user, &pass) {
                        unsafe { write_result_str("Login fehlgeschlagen"); }
                        ftp.disconnect();
                        WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                    } else {
                        // Store the connected FTP client
                        unsafe { FTP_CLIENT = Some(ftp); }
                        // Get initial directory listing
                        do_list_in_worker();
                        WORKER_RESULT.store(RES_OK, Ordering::Release);
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
                let ok = ftp.cd(&path);
                if ok {
                    do_list_in_worker();
                    WORKER_RESULT.store(RES_OK, Ordering::Release);
                } else {
                    unsafe { write_result_str("Verzeichnis wechsel fehlgeschlagen"); }
                    WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                }
            }
        }

        CMD_DOWNLOAD => {
            let remote_name = unsafe { read_cstr(&PARAM1) }.to_string();
            let local_path  = unsafe { read_cstr(&PARAM2) }.to_string();
            if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                let bytes = ftp.download(&remote_name, &local_path);
                if bytes > 0 {
                    unsafe { write_result_str(&format!("OK Download: {} ({} Bytes)", remote_name, bytes)); }
                    WORKER_RESULT.store(RES_OK, Ordering::Release);
                } else {
                    unsafe { write_result_str(&format!("FEHLER Download: {}", remote_name)); }
                    WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                }
            }
        }

        CMD_UPLOAD => {
            let local_path  = unsafe { read_cstr(&PARAM1) }.to_string();
            let remote_name = unsafe { read_cstr(&PARAM2) }.to_string();
            if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                let bytes = ftp.upload(&local_path, &remote_name);
                if bytes > 0 {
                    unsafe { write_result_str(&format!("OK Upload: {} ({} Bytes)", remote_name, bytes)); }
                    WORKER_RESULT.store(RES_OK, Ordering::Release);
                } else {
                    unsafe { write_result_str(&format!("FEHLER Upload: {}", remote_name)); }
                    WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                }
            }
        }

        CMD_MKDIR => {
            let name = unsafe { read_cstr(&PARAM1) }.to_string();
            if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                let ok = ftp.mkdir(&name);
                if ok {
                    unsafe { write_result_str(&format!("Ordner erstellt: {}", name)); }
                    do_list_in_worker();
                    WORKER_RESULT.store(RES_OK, Ordering::Release);
                } else {
                    unsafe { write_result_str(&format!("Ordner erstellen fehlgeschlagen: {}", name)); }
                    WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                }
            }
        }

        CMD_DELETE => {
            let name   = unsafe { read_cstr(&PARAM1) }.to_string();
            let is_dir = PARAM_PORT.load(Ordering::Relaxed) == 1;
            if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                let ok = if is_dir { ftp.delete_dir(&name) } else { ftp.delete_file(&name) };
                if ok {
                    unsafe { write_result_str(&format!("Geloescht: {}", name)); }
                    do_list_in_worker();
                    WORKER_RESULT.store(RES_OK, Ordering::Release);
                } else {
                    unsafe { write_result_str(&format!("Loeschen fehlgeschlagen: {}", name)); }
                    WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                }
            }
        }

        CMD_RENAME => {
            let old  = unsafe { read_cstr(&PARAM1) }.to_string();
            let new  = unsafe { read_cstr(&PARAM2) }.to_string();
            if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
                let ok = ftp.rename(&old, &new);
                if ok {
                    unsafe { write_result_str(&format!("Umbenannt: {} -> {}", old, new)); }
                    do_list_in_worker();
                    WORKER_RESULT.store(RES_OK, Ordering::Release);
                } else {
                    unsafe { write_result_str(&format!("Umbenennen fehlgeschlagen: {}", old)); }
                    WORKER_RESULT.store(RES_ERROR, Ordering::Release);
                }
            }
        }

        CMD_DISCONNECT => {
            if let Some(mut ftp) = unsafe { FTP_CLIENT.take() } {
                ftp.disconnect();
            }
            unsafe { write_result_str("Verbindung getrennt"); }
            WORKER_RESULT.store(RES_OK, Ordering::Release);
        }

        _ => {}
    }

    WORKER_CMD.store(CMD_IDLE, Ordering::Release);
    WORKER_BUSY.store(false, Ordering::Release);
}

fn do_list_in_worker() {
    if let Some(ftp) = unsafe { FTP_CLIENT.as_mut() } {
        let files = ftp.list_dir();
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
        if row.is_empty() { continue; }
        let cols: Vec<&str> = row.split('\x1F').collect();
        if cols.len() < 4 { continue; }
        let name = cols[0].to_string();
        let size_str = cols[1];
        let modified = cols[2].to_string();
        let is_dir = cols[3] == "1";
        // Approximate size back (display only)
        let size: u64 = 0; // size is only for display, already formatted
        let _ = size_str;
        entries.push(FileEntry { name, size, is_dir, modified });
    }
    entries
}

fn spawn_worker() {
    WORKER_BUSY.store(true, Ordering::Release);
    WORKER_RESULT.store(RES_NONE, Ordering::Release);
    if let Ok(handle) = anyos_std::process::Thread::spawn_with_stack(worker_entry, 64 * 1024, "ftp-worker") {
        core::mem::forget(handle);
    } else {
        WORKER_BUSY.store(false, Ordering::Release);
    }
}

// Static FTP client (only accessed from worker thread)
static mut FTP_CLIENT: Option<FtpClient> = None;

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
    // UI handles
    local_grid: anyui::DataGrid,
    remote_grid: anyui::DataGrid,
    local_path_label: anyui::Label,
    remote_path_label: anyui::Label,
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

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState { unsafe { APP.as_mut().unwrap() } }

// ─── Grid Population ──────────────────────────────────────────────────────────

fn populate_grid(grid: &anyui::DataGrid, files: &[FileEntry]) {
    let mut data: Vec<u8> = Vec::new();
    let mut colors: Vec<u32> = Vec::new();

    for entry in files {
        let col = if entry.name == ".." { 0xFF888888u32 }
                  else if entry.is_dir  { 0xFFDDAA44u32 }
                  else                  { 0xFFCCCCCCu32 };

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

        colors.push(col); colors.push(col); colors.push(col);
    }

    grid.set_row_count(files.len() as u32);
    grid.set_data_raw(&data);
    grid.set_cell_colors(&colors);
}

fn refresh_local() {
    let a = app();
    a.local_files = list_local_dir(&a.local_dir.clone());
    let label = format!("Lokal: {}", a.local_dir);
    a.local_path_label.set_text(&label);
    let files = a.local_files.clone();
    populate_grid(&a.local_grid, &files);
    update_status();
}

fn update_status() {
    let a = app();
    let text = if a.is_connected {
        let rc = a.remote_files.len().saturating_sub(1);
        let lc = a.local_files.len().saturating_sub(1);
        format!("Verbunden: {} | Remote: {} | Lokal: {}", a.remote_host, rc, lc)
    } else {
        "Nicht verbunden".to_string()
    };
    a.status_label.set_text(&text);
}

fn log_line(msg: &str) {
    let a = app();
    let cur = get_editor_text(&a.log_editor);
    let new_text = if cur.is_empty() { msg.to_string() }
                   else { format!("{}\n{}", cur, msg) };
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
    if result == RES_NONE { return; }

    // Reset result so we don't process it twice
    WORKER_RESULT.store(RES_NONE, Ordering::Release);

    let cmd_was = WORKER_CMD.load(Ordering::Relaxed); // already CMD_IDLE by now
    let _ = cmd_was;

    // Read result string
    let rlen = unsafe { RESULT_STR_LEN.load(Ordering::Acquire) as usize };
    let msg = unsafe { core::str::from_utf8(&RESULT_STR[..rlen]).unwrap_or("").to_string() };

    match result {
        RES_OK => {
            log_line(&msg);
            // Update remote listing if we have new data
            let list_len = unsafe { RESULT_STR_LEN.load(Ordering::Acquire) as usize };
            let pwd_len  = unsafe { RESULT_STR2_LEN.load(Ordering::Acquire) as usize };
            if pwd_len > 0 {
                let files = unsafe { parse_file_list(&RESULT_STR, list_len) };
                let pwd = unsafe { core::str::from_utf8(&RESULT_STR2[..pwd_len]).unwrap_or("/").to_string() };
                let a = app();
                a.remote_dir = pwd.clone();
                a.remote_files = files.clone();
                let label = format!("Remote: {}", pwd);
                a.remote_path_label.set_text(&label);
                populate_grid(&a.remote_grid, &files);
                if !a.is_connected {
                    set_connected_state(true);
                }
                // Reset STR2 len so we don't re-apply it
                unsafe { RESULT_STR2_LEN.store(0, Ordering::Release); }
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
            anyui::MessageBox::show(anyui::MessageBoxType::Alert, &msg, None);
        }
        _ => {}
    }
}

// ─── Connect Dialog ───────────────────────────────────────────────────────────

fn show_connect_dialog() {
    let dlg = anyui::Window::new_with_flags("FTP Verbinden", -1, -1, 420, 270,
        anyui::WIN_FLAG_NOT_RESIZABLE | anyui::WIN_FLAG_NO_MAXIMIZE);

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
    btn_cancel.on_click(move |_| { dlg_c.destroy(); });

    let dlg_c2 = dlg.clone();
    let fh = fld_host.clone();
    let fp = fld_port.clone();
    let fu = fld_user.clone();
    let fpw = fld_pass.clone();
    btn_ok.on_click(move |_| {
        if WORKER_BUSY.load(Ordering::Relaxed) { return; }

        let host = get_field_text(&fh);
        let port_str = get_field_text(&fp);
        let user_in = get_field_text(&fu);
        let pass_in = get_field_text(&fpw);

        if host.is_empty() {
            anyui::MessageBox::show(anyui::MessageBoxType::Alert, "Bitte Host eingeben.", None);
            return;
        }
        let port: u16 = port_str.parse().unwrap_or(21);
        let user = if user_in.is_empty() { "anonymous".to_string() } else { user_in };
        let pass = if pass_in.is_empty() { "anonymous@".to_string() } else { pass_in };

        unsafe {
            write_param(&mut PARAM1, &host);
            write_param(&mut PARAM2, &user);
            write_param(&mut PARAM3, &pass);
        }
        PARAM_PORT.store(port as u32, Ordering::Relaxed);
        app().remote_host = host.clone();

        log_line(&format!("Verbinde mit {}:{}...", host, port));
        WORKER_CMD.store(CMD_CONNECT, Ordering::Release);
        spawn_worker();
        dlg_c2.destroy();
    });

    dlg.on_close(|_| {});
}

// ─── Rename Dialog ────────────────────────────────────────────────────────────

fn show_rename_dialog(current_name: String, is_remote: bool) {
    let dlg = anyui::Window::new_with_flags("Umbenennen", -1, -1, 380, 130,
        anyui::WIN_FLAG_NOT_RESIZABLE | anyui::WIN_FLAG_NO_MAXIMIZE);
    let panel = anyui::View::new();
    panel.set_dock(anyui::DOCK_FILL);
    panel.set_color(0xFF2D2D30);
    dlg.add(&panel);

    let lbl = anyui::Label::new("Neuer Name:");
    lbl.set_size(95, 24); lbl.set_position(12, 16);
    panel.add(&lbl);

    let fld = anyui::TextField::new();
    fld.set_size(250, 26); fld.set_position(112, 14);
    fld.set_text(&current_name);
    panel.add(&fld);

    let btn_cancel = anyui::Button::new("Abbrechen");
    btn_cancel.set_size(110, 30); btn_cancel.set_position(12, 54);
    panel.add(&btn_cancel);

    let btn_ok = anyui::Button::new("Umbenennen");
    btn_ok.set_size(110, 30); btn_ok.set_position(252, 54);
    panel.add(&btn_ok);

    let dlg_c = dlg.clone();
    btn_cancel.on_click(move |_| { dlg_c.destroy(); });

    let dlg_c2 = dlg.clone();
    let fld_c = fld.clone();
    let old = current_name.clone();
    btn_ok.on_click(move |_| {
        let new_name = get_field_text(&fld_c);
        if new_name.is_empty() || new_name == old { dlg_c2.destroy(); return; }
        if is_remote {
            if WORKER_BUSY.load(Ordering::Relaxed) { return; }
            unsafe { write_param(&mut PARAM1, &old); write_param(&mut PARAM2, &new_name); }
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
                anyui::MessageBox::show(anyui::MessageBoxType::Alert, "Umbenennen fehlgeschlagen.", None);
            }
        }
        dlg_c2.destroy();
    });
    dlg.on_close(|_| {});
}

// ─── New Folder Dialog ────────────────────────────────────────────────────────

fn show_new_folder_dialog(is_remote: bool) {
    let dlg = anyui::Window::new_with_flags("Neuer Ordner", -1, -1, 360, 120,
        anyui::WIN_FLAG_NOT_RESIZABLE | anyui::WIN_FLAG_NO_MAXIMIZE);
    let panel = anyui::View::new();
    panel.set_dock(anyui::DOCK_FILL);
    panel.set_color(0xFF2D2D30);
    dlg.add(&panel);

    let lbl = anyui::Label::new("Ordnername:");
    lbl.set_size(95, 24); lbl.set_position(12, 16);
    panel.add(&lbl);

    let fld = anyui::TextField::new();
    fld.set_size(230, 26); fld.set_position(112, 14);
    fld.set_placeholder("Neuer Ordner");
    panel.add(&fld);

    let btn_cancel = anyui::Button::new("Abbrechen");
    btn_cancel.set_size(100, 30); btn_cancel.set_position(12, 50);
    panel.add(&btn_cancel);

    let btn_ok = anyui::Button::new("Erstellen");
    btn_ok.set_size(100, 30); btn_ok.set_position(244, 50);
    panel.add(&btn_ok);

    let dlg_c = dlg.clone();
    btn_cancel.on_click(move |_| { dlg_c.destroy(); });

    let dlg_c2 = dlg.clone();
    let fld_c = fld.clone();
    btn_ok.on_click(move |_| {
        let name = get_field_text(&fld_c);
        if name.is_empty() { dlg_c2.destroy(); return; }
        if is_remote {
            if WORKER_BUSY.load(Ordering::Relaxed) { return; }
            unsafe { write_param(&mut PARAM1, &name); }
            WORKER_CMD.store(CMD_MKDIR, Ordering::Release);
            spawn_worker();
        } else {
            let a = app();
            let path = join_path(&a.local_dir, &name);
            if fs::mkdir(&path) == 0 {
                log_line(&format!("Ordner erstellt: {}", name));
                refresh_local();
            } else {
                anyui::MessageBox::show(anyui::MessageBoxType::Alert, "Erstellen fehlgeschlagen.", None);
            }
        }
        dlg_c2.destroy();
    });
    dlg.on_close(|_| {});
}

// ─── Transfer Actions ─────────────────────────────────────────────────────────

fn do_download() {
    if WORKER_BUSY.load(Ordering::Relaxed) { return; }
    let a = app();
    if !a.is_connected { return; }
    let row = a.remote_grid.selected_row() as usize;
    if row >= a.remote_files.len() { return; }
    let entry = a.remote_files[row].clone();
    if entry.is_dir || entry.name == ".." { return; }
    let local_path = join_path(&a.local_dir, &entry.name);
    log_line(&format!("Download: {} -> {}", entry.name, local_path));
    unsafe { write_param(&mut PARAM1, &entry.name); write_param(&mut PARAM2, &local_path); }
    WORKER_CMD.store(CMD_DOWNLOAD, Ordering::Release);
    spawn_worker();
}

fn do_upload() {
    if WORKER_BUSY.load(Ordering::Relaxed) { return; }
    let a = app();
    if !a.is_connected { return; }
    let row = a.local_grid.selected_row() as usize;
    if row >= a.local_files.len() { return; }
    let entry = a.local_files[row].clone();
    if entry.is_dir || entry.name == ".." { return; }
    let local_path = join_path(&a.local_dir, &entry.name);
    log_line(&format!("Upload: {} -> {}/{}", local_path, a.remote_dir, entry.name));
    unsafe { write_param(&mut PARAM1, &local_path); write_param(&mut PARAM2, &entry.name); }
    WORKER_CMD.store(CMD_UPLOAD, Ordering::Release);
    spawn_worker();
}

fn do_delete_remote(entry: FileEntry) {
    if WORKER_BUSY.load(Ordering::Relaxed) { return; }
    unsafe { write_param(&mut PARAM1, &entry.name); }
    PARAM_PORT.store(if entry.is_dir { 1 } else { 0 }, Ordering::Relaxed);
    WORKER_CMD.store(CMD_DELETE, Ordering::Release);
    spawn_worker();
}

fn do_delete() {
    let a = app();
    if a.focus_pane == 1 {
        if !a.is_connected { return; }
        let row = a.remote_grid.selected_row() as usize;
        if row >= a.remote_files.len() { return; }
        let entry = a.remote_files[row].clone();
        if entry.name == ".." { return; }
        do_delete_remote(entry);
    } else {
        let row = a.local_grid.selected_row() as usize;
        if row >= a.local_files.len() { return; }
        let entry = a.local_files[row].clone();
        if entry.name == ".." { return; }
        let path = join_path(&a.local_dir, &entry.name);
        if fs::unlink(&path) == 0 {
            log_line(&format!("Geloescht: {}", entry.name));
            refresh_local();
        } else {
            anyui::MessageBox::show(anyui::MessageBoxType::Alert, "Loeschen fehlgeschlagen.", None);
        }
    }
}

fn do_rename() {
    let a = app();
    if a.focus_pane == 1 {
        if !a.is_connected { return; }
        let row = a.remote_grid.selected_row() as usize;
        if row >= a.remote_files.len() { return; }
        let name = a.remote_files[row].name.clone();
        if name == ".." { return; }
        show_rename_dialog(name, true);
    } else {
        let row = a.local_grid.selected_row() as usize;
        if row >= a.local_files.len() { return; }
        let name = a.local_files[row].name.clone();
        if name == ".." { return; }
        show_rename_dialog(name, false);
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() { return; }

    let win = anyui::Window::new("anyzilla", -1, -1, 1100, 680);

    // ── Toolbar ───────────────────────────────────────────────────────────────
    let toolbar = anyui::Toolbar::new();
    toolbar.set_dock(anyui::DOCK_TOP);
    toolbar.set_size(1100, 46);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(4, 4, 4, 4);

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

    win.add(&toolbar);

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
    log_view.set_dock(anyui::DOCK_BOTTOM);
    log_view.set_size(1100, 120);
    log_view.set_color(0xFF1A1A1A);
    let log_editor = anyui::TextEditor::new(1100, 120);
    log_editor.set_dock(anyui::DOCK_FILL);
    log_editor.set_color(0xFF1A1A1A);
    log_editor.set_text_color(0xFF88CC88);
    log_editor.set_read_only(true);
    log_view.add(&log_editor);
    win.add(&log_view);

    // ── Main split view ───────────────────────────────────────────────────────
    let split = anyui::SplitView::new();
    split.set_dock(anyui::DOCK_FILL);
    split.set_orientation(anyui::ORIENTATION_HORIZONTAL);
    split.set_split_ratio(50);

    // Local pane
    let local_pane = anyui::View::new();
    local_pane.set_color(0xFF1E1E1E);
    let local_header = anyui::View::new();
    local_header.set_dock(anyui::DOCK_TOP);
    local_header.set_size(500, 28);
    local_header.set_color(0xFF2D2D30);
    let local_path_label = anyui::Label::new("Lokal: /");
    local_path_label.set_dock(anyui::DOCK_FILL);
    local_path_label.set_color(0xFF2D2D30);
    local_path_label.set_text_color(0xFFCCCCCC);
    local_path_label.set_padding(8, 0, 0, 0);
    local_header.add(&local_path_label);
    local_pane.add(&local_header);
    let local_grid = anyui::DataGrid::new(500, 500);
    local_grid.set_dock(anyui::DOCK_FILL);
    local_grid.set_color(0xFF1E1E1E);
    local_grid.set_columns(&[
        anyui::ColumnDef::new("Name").width(280),
        anyui::ColumnDef::new("Groesse").width(80).align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new("Datum").width(110),
    ]);
    local_pane.add(&local_grid);
    split.add(&local_pane);

    // Remote pane
    let remote_pane = anyui::View::new();
    remote_pane.set_color(0xFF1E1E1E);
    let remote_header = anyui::View::new();
    remote_header.set_dock(anyui::DOCK_TOP);
    remote_header.set_size(500, 28);
    remote_header.set_color(0xFF2D2D30);
    let remote_path_label = anyui::Label::new("Remote: (nicht verbunden)");
    remote_path_label.set_dock(anyui::DOCK_FILL);
    remote_path_label.set_color(0xFF2D2D30);
    remote_path_label.set_text_color(0xFFCCCCCC);
    remote_path_label.set_padding(8, 0, 0, 0);
    remote_header.add(&remote_path_label);
    remote_pane.add(&remote_header);
    let remote_grid = anyui::DataGrid::new(500, 500);
    remote_grid.set_dock(anyui::DOCK_FILL);
    remote_grid.set_color(0xFF1E1E1E);
    remote_grid.set_columns(&[
        anyui::ColumnDef::new("Name").width(280),
        anyui::ColumnDef::new("Groesse").width(80).align(anyui::ALIGN_RIGHT),
        anyui::ColumnDef::new("Datum").width(110),
    ]);
    remote_pane.add(&remote_grid);
    split.add(&remote_pane);
    win.add(&split);

    // Context menus
    let local_menu = anyui::ContextMenu::new("Hochladen|Umbenennen|Loeschen|Neuer Ordner|Aktualisieren");
    local_grid.set_context_menu(&local_menu);
    let remote_menu = anyui::ContextMenu::new("Herunterladen|Umbenennen|Loeschen|Neuer Ordner|Aktualisieren");
    remote_grid.set_context_menu(&remote_menu);

    // ── Initialize App State ──────────────────────────────────────────────────
    let home = get_home_dir();
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
            local_grid: local_grid.clone(),
            remote_grid: remote_grid.clone(),
            local_path_label: local_path_label.clone(),
            remote_path_label: remote_path_label.clone(),
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

    btn_connect.on_click(|_| { show_connect_dialog(); });

    btn_disconnect.on_click(|_| {
        if WORKER_BUSY.load(Ordering::Relaxed) { return; }
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

    btn_upload.on_click(|_| { do_upload(); });
    btn_download.on_click(|_| { do_download(); });
    btn_delete.on_click(|_| { do_delete(); });
    btn_rename.on_click(|_| { do_rename(); });

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

    // ── Grid Callbacks ────────────────────────────────────────────────────────

    local_grid.on_selection_changed(|_| { app().focus_pane = 0; });

    local_grid.on_submit(|e| {
        let a = app();
        let row = e.index as usize;
        if row >= a.local_files.len() { return; }
        let entry = a.local_files[row].clone();
        if entry.is_dir {
            let new_dir = join_path(&a.local_dir, &entry.name);
            a.local_dir = new_dir;
            refresh_local();
        }
    });

    remote_grid.on_selection_changed(|_| { app().focus_pane = 1; });

    remote_grid.on_submit(|e| {
        let a = app();
        if !a.is_connected { return; }
        if WORKER_BUSY.load(Ordering::Relaxed) { return; }
        let row = e.index as usize;
        if row >= a.remote_files.len() { return; }
        let entry = a.remote_files[row].clone();
        if entry.is_dir {
            unsafe { write_param(&mut PARAM1, &entry.name); }
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
            2 => { app().focus_pane = 0; do_delete(); }
            3 => show_new_folder_dialog(false),
            4 => refresh_local(),
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
            2 => { app().focus_pane = 1; do_delete(); }
            3 => show_new_folder_dialog(true),
            4 => {
                if !WORKER_BUSY.load(Ordering::Relaxed) && app().is_connected {
                    WORKER_CMD.store(CMD_LIST, Ordering::Release);
                    spawn_worker();
                }
            }
            _ => {}
        }
    });

    // ── Keyboard Shortcuts ────────────────────────────────────────────────────

    win.on_key_down(|e| {
        match e.keycode {
            anyui::KEY_F2     => do_rename(),
            anyui::KEY_DELETE => do_delete(),
            anyui::KEY_F5     => {
                if app().focus_pane == 1 { do_download(); }
                else { do_upload(); }
            }
            anyui::KEY_F7 => show_new_folder_dialog(app().focus_pane == 1),
            _ => {}
        }
    });

    win.on_close(|_| {
        if let Some(mut ftp) = unsafe { FTP_CLIENT.take() } {
            ftp.disconnect();
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
}
