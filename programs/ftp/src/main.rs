#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{println, net, fs};

anyos_std::entry!(main);

const FTP_PORT: u16 = 21;
const CONNECT_TIMEOUT: u32 = 10000; // 10s

struct FtpClient {
    ctrl: u32, // control socket
}

impl FtpClient {
    fn connect(ip: &[u8; 4]) -> Option<FtpClient> {
        let sock = net::tcp_connect(ip, FTP_PORT, CONNECT_TIMEOUT);
        if sock == u32::MAX {
            println!("Failed to connect to FTP server");
            return None;
        }

        let mut client = FtpClient { ctrl: sock };

        // Read 220 banner
        let resp = client.read_response();
        if !resp.starts_with("220") {
            println!("Unexpected banner: {}", resp);
            net::tcp_close(sock);
            return None;
        }

        Some(client)
    }

    fn login(&mut self, user: &str, pass: &str) -> bool {
        // USER
        self.send_command("USER ", user);
        let resp = self.read_response();
        if resp.starts_with("230") {
            return true; // Already logged in
        }
        if !resp.starts_with("331") {
            println!("USER failed: {}", resp);
            return false;
        }

        // PASS
        self.send_command("PASS ", pass);
        let resp = self.read_response();
        if !resp.starts_with("230") {
            println!("PASS failed: {}", resp);
            return false;
        }
        true
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
        let mut recv_buf = [0u8; 1024];

        // Read until we get a complete response (line starting with 3-digit code + space)
        loop {
            let n = net::tcp_recv(self.ctrl, &mut recv_buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            result.extend_from_slice(&recv_buf[..n as usize]);

            // Check if we have a complete response
            if is_complete_response(&result) {
                break;
            }
        }

        String::from_utf8_lossy(&result).into_owned()
    }

    fn open_data_connection(&mut self) -> Option<u32> {
        // Send PASV
        self.send_cmd_only("PASV");
        let resp = self.read_response();
        if !resp.starts_with("227") {
            println!("PASV failed: {}", resp);
            return None;
        }

        // Parse "227 Entering Passive Mode (h1,h2,h3,h4,p1,p2)"
        let (ip, port) = parse_pasv(&resp)?;
        let sock = net::tcp_connect(&ip, port, CONNECT_TIMEOUT);
        if sock == u32::MAX {
            println!("Failed to connect to data port {}:{}",
                     format_ip(&ip), port);
            return None;
        }
        Some(sock)
    }

    fn set_binary_mode(&mut self) -> bool {
        self.send_cmd_only("TYPE I");
        let resp = self.read_response();
        resp.starts_with("200")
    }

    fn list(&mut self) {
        let data_sock = match self.open_data_connection() {
            Some(s) => s,
            None => return,
        };

        self.send_cmd_only("LIST");
        let resp = self.read_response();
        if !resp.starts_with("150") && !resp.starts_with("125") {
            println!("LIST failed: {}", resp);
            net::tcp_close(data_sock);
            return;
        }

        // Read data
        let mut buf = [0u8; 2048];
        loop {
            let n = net::tcp_recv(data_sock, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            // Print received data
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                print_str(s);
            }
        }
        net::tcp_close(data_sock);

        // Read 226 Transfer complete
        let resp = self.read_response();
        if !resp.starts_with("226") {
            println!("Transfer not complete: {}", resp);
        }
    }

    fn get(&mut self, remote_path: &str, local_path: &str) {
        self.set_binary_mode();

        let data_sock = match self.open_data_connection() {
            Some(s) => s,
            None => return,
        };

        self.send_command("RETR ", remote_path);
        let resp = self.read_response();
        if !resp.starts_with("150") && !resp.starts_with("125") {
            println!("RETR failed: {}", resp);
            net::tcp_close(data_sock);
            return;
        }

        // Open local file for writing
        let fd = fs::open(local_path, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            println!("Failed to open local file: {}", local_path);
            net::tcp_close(data_sock);
            return;
        }

        let mut total = 0u32;
        let mut buf = [0u8; 2048];
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

        // Read 226 Transfer complete
        let resp = self.read_response();
        if resp.starts_with("226") {
            println!("Downloaded {} bytes to {}", total, local_path);
        } else {
            println!("Transfer issue: {}", resp);
        }
    }

    fn put(&mut self, local_path: &str, remote_path: &str) {
        self.set_binary_mode();

        // Read local file
        let fd = fs::open(local_path, 0);
        if fd == u32::MAX {
            println!("Failed to open local file: {}", local_path);
            return;
        }

        let mut file_data = Vec::new();
        let mut buf = [0u8; 2048];
        loop {
            let n = fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            file_data.extend_from_slice(&buf[..n as usize]);
        }
        fs::close(fd);

        let data_sock = match self.open_data_connection() {
            Some(s) => s,
            None => return,
        };

        self.send_command("STOR ", remote_path);
        let resp = self.read_response();
        if !resp.starts_with("150") && !resp.starts_with("125") {
            println!("STOR failed: {}", resp);
            net::tcp_close(data_sock);
            return;
        }

        // Send file data
        let mut offset = 0;
        while offset < file_data.len() {
            let end = (offset + 1460).min(file_data.len());
            let sent = net::tcp_send(data_sock, &file_data[offset..end]);
            if sent == u32::MAX {
                println!("Send error at offset {}", offset);
                break;
            }
            offset = end;
        }
        net::tcp_close(data_sock);

        // Read 226 Transfer complete
        let resp = self.read_response();
        if resp.starts_with("226") {
            println!("Uploaded {} bytes from {}", file_data.len(), local_path);
        } else {
            println!("Transfer issue: {}", resp);
        }
    }

    fn pwd(&mut self) {
        self.send_cmd_only("PWD");
        let resp = self.read_response();
        println!("{}", resp.trim_end());
    }

    fn cd(&mut self, path: &str) {
        self.send_command("CWD ", path);
        let resp = self.read_response();
        if !resp.starts_with("250") {
            println!("CWD failed: {}", resp.trim_end());
        }
    }

    fn disconnect(&mut self) {
        self.send_cmd_only("QUIT");
        // Try to read response but don't block long
        let _ = self.read_response();
        net::tcp_close(self.ctrl);
    }
}

/// Check if FTP response is complete (has a line matching "NNN " pattern)
fn is_complete_response(data: &[u8]) -> bool {
    // Look for a line starting with 3 digits followed by a space
    let mut i = 0;
    while i < data.len() {
        // Find start of line
        let line_start = i;
        // Find end of line
        while i < data.len() && data[i] != b'\n' {
            i += 1;
        }
        let line_end = i;
        if i < data.len() {
            i += 1; // skip \n
        }

        let line = &data[line_start..line_end];
        if line.len() >= 4
            && line[0].is_ascii_digit()
            && line[1].is_ascii_digit()
            && line[2].is_ascii_digit()
            && line[3] == b' '
        {
            return true;
        }
    }
    false
}

/// Parse PASV response: "227 Entering Passive Mode (h1,h2,h3,h4,p1,p2)"
fn parse_pasv(resp: &str) -> Option<([u8; 4], u16)> {
    // Find opening parenthesis
    let start = resp.find('(')?;
    let end = resp.find(')')?;
    if end <= start + 1 {
        return None;
    }

    let nums_str = &resp[start + 1..end];
    let mut nums = [0u32; 6];
    let mut idx = 0;
    let mut current = 0u32;

    for b in nums_str.bytes() {
        match b {
            b'0'..=b'9' => {
                current = current * 10 + (b - b'0') as u32;
            }
            b',' => {
                if idx >= 6 { return None; }
                nums[idx] = current;
                idx += 1;
                current = 0;
            }
            _ => {}
        }
    }
    if idx == 5 {
        nums[5] = current;
    } else {
        return None;
    }

    let ip = [nums[0] as u8, nums[1] as u8, nums[2] as u8, nums[3] as u8];
    let port = (nums[4] as u16) * 256 + nums[5] as u16;
    Some((ip, port))
}

fn format_ip(ip: &[u8; 4]) -> String {
    let mut s = String::new();
    for (i, &b) in ip.iter().enumerate() {
        if i > 0 { s.push('.'); }
        write_u32(&mut s, b as u32);
    }
    s
}

fn write_u32(s: &mut String, val: u32) {
    if val >= 10 {
        write_u32(s, val / 10);
    }
    s.push((b'0' + (val % 10) as u8) as char);
}

fn print_str(s: &str) {
    // Print without adding newline
    anyos_std::fs::write(1, s.as_bytes());
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

fn main() {
    let mut args_buf = [0u8; 256];
    let args_len = anyos_std::process::getargs(&mut args_buf);
    let args = core::str::from_utf8(&args_buf[..args_len]).unwrap_or("").trim();

    if args.is_empty() {
        println!("Usage: ftp <host> [command] [args...]");
        println!("Commands:");
        println!("  ls              List files (default)");
        println!("  get <remote> <local>  Download file");
        println!("  put <local> <remote>  Upload file");
        println!("  pwd             Print working directory");
        println!("  cd <path>       Change directory");
        return;
    }

    // Parse arguments: first is IP, rest is command
    let mut parts = args.splitn(2, ' ');
    let host_str = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();

    let ip = match parse_ip(host_str) {
        Some(ip) => ip,
        None => {
            // Try DNS resolve
            let mut resolved = [0u8; 4];
            if net::dns(host_str, &mut resolved) == 0 {
                resolved
            } else {
                println!("Cannot resolve host: {}", host_str);
                return;
            }
        }
    };

    println!("Connecting to {}...", host_str);
    let mut client = match FtpClient::connect(&ip) {
        Some(c) => c,
        None => return,
    };

    println!("Connected. Logging in...");
    if !client.login("anonymous", "user@anyos") {
        client.disconnect();
        return;
    }
    println!("Logged in.");

    // Parse command
    if rest.is_empty() || rest == "ls" {
        client.list();
    } else if rest == "pwd" {
        client.pwd();
    } else if rest.starts_with("cd ") {
        let path = rest[3..].trim();
        client.cd(path);
    } else if rest.starts_with("get ") {
        let args_str = rest[4..].trim();
        let mut cmd_parts = args_str.splitn(2, ' ');
        let remote = cmd_parts.next().unwrap_or("").trim();
        let local = cmd_parts.next().unwrap_or("").trim();
        if remote.is_empty() || local.is_empty() {
            println!("Usage: ftp <host> get <remote_path> <local_path>");
        } else {
            client.get(remote, local);
        }
    } else if rest.starts_with("put ") {
        let args_str = rest[4..].trim();
        let mut cmd_parts = args_str.splitn(2, ' ');
        let local = cmd_parts.next().unwrap_or("").trim();
        let remote = cmd_parts.next().unwrap_or("").trim();
        if local.is_empty() || remote.is_empty() {
            println!("Usage: ftp <host> put <local_path> <remote_path>");
        } else {
            client.put(local, remote);
        }
    } else {
        println!("Unknown command: {}", rest);
    }

    client.disconnect();
}
