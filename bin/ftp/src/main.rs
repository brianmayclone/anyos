#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{println, format, net, fs};

anyos_std::entry!(main);

const FTP_PORT: u16 = 21;
const CONNECT_TIMEOUT: u32 = 10000; // 10s

struct FtpClient {
    ctrl: u32, // control socket
    passive: bool,
    // For active mode: our listen port
    active_port: u16,
    local_ip: [u8; 4],
}

impl FtpClient {
    fn connect(ip: &[u8; 4]) -> Option<FtpClient> {
        let sock = net::tcp_connect(ip, FTP_PORT, CONNECT_TIMEOUT);
        if sock == u32::MAX {
            println!("Failed to connect to FTP server");
            return None;
        }

        // Get local IP for active mode PORT command
        let mut net_cfg = [0u8; 24];
        let local_ip = if net::get_config(&mut net_cfg) == 0 {
            [net_cfg[0], net_cfg[1], net_cfg[2], net_cfg[3]]
        } else {
            [127, 0, 0, 1]
        };

        let mut client = FtpClient { ctrl: sock, passive: true, active_port: 0, local_ip };

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
            return true; // Already logged in (e.g. anonymous without password)
        }
        if !resp.starts_with("331") {
            println!("USER failed: {}", resp);
            return false;
        }

        // PASS
        self.send_command("PASS ", pass);
        let resp = self.read_response();
        if !resp.starts_with("230") {
            println!("PASS failed: {}", resp.trim_end());
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

        loop {
            let n = net::tcp_recv(self.ctrl, &mut recv_buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            result.extend_from_slice(&recv_buf[..n as usize]);
            if is_complete_response(&result) {
                break;
            }
        }

        String::from_utf8_lossy(&result).into_owned()
    }

    fn open_data_connection(&mut self) -> Option<u32> {
        if self.passive {
            self.open_pasv()
        } else {
            self.open_port()
        }
    }

    fn open_pasv(&mut self) -> Option<u32> {
        self.send_cmd_only("PASV");
        let resp = self.read_response();
        if !resp.starts_with("227") {
            println!("PASV failed: {}", resp.trim_end());
            return None;
        }
        let (ip, port) = parse_pasv(&resp)?;
        let sock = net::tcp_connect(&ip, port, CONNECT_TIMEOUT);
        if sock == u32::MAX {
            println!("Failed to connect to data port {}:{}", format_ip(&ip), port);
            return None;
        }
        Some(sock)
    }

    fn open_port(&mut self) -> Option<u32> {
        // Listen on a local port for active mode
        // Use port range 50100-50200 for active data connections
        let port = 50100u16 + (self.active_port % 100);
        self.active_port += 1;
        let listener = net::tcp_listen(port, 1);
        if listener == u32::MAX {
            println!("Failed to bind data port {}", port);
            return None;
        }

        // Send PORT command: h1,h2,h3,h4,p1,p2
        let ip = self.local_ip;
        let p1 = (port >> 8) as u8;
        let p2 = (port & 0xFF) as u8;
        let mut port_cmd = String::new();
        port_cmd.push_str(&format!("{},{},{},{},{},{}",
            ip[0], ip[1], ip[2], ip[3], p1, p2));
        self.send_command("PORT ", &port_cmd);
        let resp = self.read_response();
        if !resp.starts_with("200") {
            println!("PORT failed: {}", resp.trim_end());
            net::tcp_close(listener);
            return None;
        }

        // Server will connect back; accept the incoming connection
        let (data_sock, _, _) = net::tcp_accept(listener);
        net::tcp_close(listener); // Close the listener after accepting
        if data_sock == u32::MAX {
            println!("Failed to accept data connection");
            return None;
        }
        Some(data_sock)
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
            println!("LIST failed: {}", resp.trim_end());
            net::tcp_close(data_sock);
            return;
        }

        let mut buf = [0u8; 2048];
        loop {
            let n = net::tcp_recv(data_sock, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                print_str(s);
            }
        }
        net::tcp_close(data_sock);

        let resp = self.read_response();
        if !resp.starts_with("226") {
            println!("Transfer not complete: {}", resp.trim_end());
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
            println!("RETR failed: {}", resp.trim_end());
            net::tcp_close(data_sock);
            return;
        }

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

        let resp = self.read_response();
        if resp.starts_with("226") {
            println!("Downloaded {} bytes to {}", total, local_path);
        } else {
            println!("Transfer issue: {}", resp.trim_end());
        }
    }

    fn put(&mut self, local_path: &str, remote_path: &str) {
        // Check if local path is a directory
        let mut stat_buf = [0u32; 7];
        if fs::stat(local_path, &mut stat_buf) != u32::MAX {
            // stat_buf[0] = type: 0=file, 1=directory, 2=device
            let is_dir = stat_buf[0] == 1;
            if is_dir {
                self.put_dir(local_path, remote_path);
                return;
            }
        }

        self.put_file(local_path, remote_path);
    }

    fn put_file(&mut self, local_path: &str, remote_path: &str) {
        self.set_binary_mode();

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
            println!("STOR failed: {}", resp.trim_end());
            net::tcp_close(data_sock);
            return;
        }

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

        let resp = self.read_response();
        if resp.starts_with("226") {
            println!("Uploaded {} bytes from {}", file_data.len(), local_path);
        } else {
            println!("Transfer issue: {}", resp.trim_end());
        }
    }

    /// Recursively upload a local directory to the remote server.
    fn put_dir(&mut self, local_path: &str, remote_path: &str) {
        // Create the remote directory (ignore error if it already exists)
        self.send_command("MKD ", remote_path);
        let resp = self.read_response();
        if resp.starts_with("550") {
            // Already exists or permission denied — check which
            if resp.contains("denied") || resp.contains("Permission") {
                println!("Cannot create remote directory: {}", resp.trim_end());
                return;
            }
            // Already exists: continue uploading into it
        } else if !resp.starts_with("257") && !resp.starts_with("250") {
            println!("MKD failed: {}", resp.trim_end());
            return;
        }
        println!("Directory: {}", remote_path);

        // Read local directory entries
        let mut dir_buf = [0u8; 64 * 256];
        let count = fs::readdir(local_path, &mut dir_buf);
        if count == u32::MAX {
            println!("Failed to read directory: {}", local_path);
            return;
        }

        for i in 0..count as usize {
            let entry_offset = i * 64;
            if entry_offset + 8 > dir_buf.len() { break; }
            let entry_type = dir_buf[entry_offset];
            let name_len = dir_buf[entry_offset + 1] as usize;
            if name_len == 0 || name_len > 56 { break; }
            if entry_offset + 8 + name_len > dir_buf.len() { break; }
            let name = match core::str::from_utf8(&dir_buf[entry_offset + 8..entry_offset + 8 + name_len]) {
                Ok(n) => n,
                Err(_) => continue,
            };
            if name.is_empty() { break; }
            // Skip . and ..
            if name == "." || name == ".." { continue; }

            // Build local and remote paths for this entry
            let mut local_child = String::from(local_path);
            if !local_child.ends_with('/') { local_child.push('/'); }
            local_child.push_str(name);

            let mut remote_child = String::from(remote_path);
            if !remote_child.ends_with('/') { remote_child.push('/'); }
            remote_child.push_str(name);

            if entry_type == 1 {
                // Subdirectory — recurse
                self.put_dir(&local_child, &remote_child);
            } else {
                // File (type 0) or device (type 2) — upload as file
                self.put_file(&local_child, &remote_child);
            }
        }
    }

    fn pwd(&mut self) {
        self.send_cmd_only("PWD");
        let resp = self.read_response();
        println!("{}", resp.trim_end());
    }

    fn cd(&mut self, path: &str) {
        if path == ".." {
            self.send_cmd_only("CDUP");
        } else {
            self.send_command("CWD ", path);
        }
        let resp = self.read_response();
        if !resp.starts_with("200") && !resp.starts_with("250") {
            println!("{}", resp.trim_end());
        }
    }

    fn mkdir(&mut self, path: &str) {
        self.send_command("MKD ", path);
        let resp = self.read_response();
        println!("{}", resp.trim_end());
    }

    fn delete(&mut self, path: &str) {
        self.send_command("DELE ", path);
        let resp = self.read_response();
        if !resp.starts_with("250") {
            println!("{}", resp.trim_end());
        }
    }

    fn rename(&mut self, old: &str, new: &str) {
        self.send_command("RNFR ", old);
        let resp = self.read_response();
        if !resp.starts_with("350") {
            println!("RNFR failed: {}", resp.trim_end());
            return;
        }
        self.send_command("RNTO ", new);
        let resp = self.read_response();
        if !resp.starts_with("250") {
            println!("RNTO failed: {}", resp.trim_end());
        }
    }

    fn rmdir(&mut self, path: &str) {
        self.send_command("RMD ", path);
        let resp = self.read_response();
        println!("{}", resp.trim_end());
    }

    fn size(&mut self, path: &str) {
        self.send_command("SIZE ", path);
        let resp = self.read_response();
        println!("{}", resp.trim_end());
    }

    fn disconnect(&mut self) {
        self.send_cmd_only("QUIT");
        let _ = self.read_response();
        net::tcp_close(self.ctrl);
    }

    /// Run interactive REPL shell
    fn interactive(&mut self) {
        println!("Type 'help' for available commands, 'bye' to quit.");
        let mut line_buf = [0u8; 512];

        loop {
            // Print prompt
            print_str("ftp> ");

            // Read line from stdin (fd 0)
            let n = fs::read(0, &mut line_buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            // Ctrl+C (0x03) or Ctrl+D (0x04) → quit
            if n > 0 && (line_buf[0] == 0x03 || line_buf[0] == 0x04) {
                break;
            }
            let line = core::str::from_utf8(&line_buf[..n as usize])
                .unwrap_or("")
                .trim();
            if line.is_empty() {
                continue;
            }

            // Split command and args
            let mut parts = line.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("").trim();
            let args = parts.next().unwrap_or("").trim();

            match cmd {
                "ls" | "dir" | "list" => {
                    self.list();
                }
                "pwd" => {
                    self.pwd();
                }
                "cd" => {
                    if args.is_empty() {
                        println!("Usage: cd <path>");
                    } else {
                        self.cd(args);
                    }
                }
                "get" => {
                    let mut a = args.splitn(2, ' ');
                    let remote = a.next().unwrap_or("").trim();
                    let local = a.next().unwrap_or("").trim();
                    if remote.is_empty() {
                        println!("Usage: get <remote_path> [local_path]");
                    } else {
                        // If no local path, use filename part of remote
                        let local_path = if local.is_empty() {
                            // Extract filename from path
                            remote.rfind('/').map(|i| &remote[i+1..]).unwrap_or(remote)
                        } else {
                            local
                        };
                        self.get(remote, local_path);
                    }
                }
                "put" | "upload" => {
                    let mut a = args.splitn(2, ' ');
                    let local = a.next().unwrap_or("").trim();
                    let remote = a.next().unwrap_or("").trim();
                    if local.is_empty() {
                        println!("Usage: put <local_path> [remote_path]");
                    } else {
                        let remote_path = if remote.is_empty() {
                            local.rfind('/').map(|i| &local[i+1..]).unwrap_or(local)
                        } else {
                            remote
                        };
                        self.put(local, remote_path);
                    }
                }
                "mkdir" | "md" => {
                    if args.is_empty() {
                        println!("Usage: mkdir <path>");
                    } else {
                        self.mkdir(args);
                    }
                }
                "rmdir" | "rd" => {
                    if args.is_empty() {
                        println!("Usage: rmdir <path>");
                    } else {
                        self.rmdir(args);
                    }
                }
                "delete" | "del" | "rm" => {
                    if args.is_empty() {
                        println!("Usage: delete <filename>");
                    } else {
                        self.delete(args);
                    }
                }
                "rename" | "ren" | "mv" => {
                    let mut a = args.splitn(2, ' ');
                    let old = a.next().unwrap_or("").trim();
                    let new = a.next().unwrap_or("").trim();
                    if old.is_empty() || new.is_empty() {
                        println!("Usage: rename <old> <new>");
                    } else {
                        self.rename(old, new);
                    }
                }
                "size" => {
                    if args.is_empty() {
                        println!("Usage: size <filename>");
                    } else {
                        self.size(args);
                    }
                }
                "passive" | "pasv" => {
                    self.passive = true;
                    println!("Passive mode enabled.");
                }
                "active" | "port" => {
                    self.passive = false;
                    println!("Active mode enabled.");
                }
                "binary" | "bin" => {
                    self.set_binary_mode();
                    println!("Binary mode.");
                }
                "ascii" => {
                    self.send_cmd_only("TYPE A");
                    let resp = self.read_response();
                    println!("{}", resp.trim_end());
                }
                "help" | "?" => {
                    println!("Commands:");
                    println!("  ls / dir / list         List remote directory");
                    println!("  pwd                     Print working directory");
                    println!("  cd <path>               Change directory");
                    println!("  get <remote> [local]    Download file");
                    println!("  put <local> [remote]    Upload file or directory (recursive)");
                    println!("  mkdir <path>            Create directory");
                    println!("  rmdir <path>            Remove directory");
                    println!("  delete <file>           Delete file");
                    println!("  rename <old> <new>      Rename file");
                    println!("  size <file>             Show file size");
                    println!("  passive                 Switch to passive mode");
                    println!("  active                  Switch to active mode");
                    println!("  binary                  Set binary transfer mode");
                    println!("  ascii                   Set ASCII transfer mode");
                    println!("  bye / quit / exit       Disconnect and exit");
                }
                "bye" | "quit" | "exit" | "q" => {
                    break;
                }
                _ => {
                    println!("Unknown command: '{}'. Type 'help' for commands.", cmd);
                }
            }
        }
    }
}

/// Check if FTP response is complete (has a line matching "NNN " pattern)
fn is_complete_response(data: &[u8]) -> bool {
    let mut i = 0;
    while i < data.len() {
        let line_start = i;
        while i < data.len() && data[i] != b'\n' {
            i += 1;
        }
        let line_end = i;
        if i < data.len() {
            i += 1;
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
    let args_raw = anyos_std::process::args(&mut args_buf);
    let args = args_raw.trim();

    if args.is_empty() {
        println!("Usage: ftp <host> [user] [password]");
        println!("       Connect and enter interactive mode.");
        println!("       If user/password omitted, logs in as anonymous.");
        return;
    }

    // Parse: ftp <host> [user] [password]
    let mut parts = args.splitn(3, ' ');
    let host_str = parts.next().unwrap_or("").trim();
    let user_arg = parts.next().map(|s| s.trim()).unwrap_or("");
    let pass_arg = parts.next().map(|s| s.trim()).unwrap_or("");

    let ip = match parse_ip(host_str) {
        Some(ip) => ip,
        None => {
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

    // Login
    let (user, pass) = if user_arg.is_empty() {
        ("anonymous", "user@anyos")
    } else {
        (user_arg, pass_arg)
    };

    println!("Logging in as {}...", user);
    if !client.login(user, pass) {
        client.disconnect();
        return;
    }
    println!("Logged in as {}.", user);

    // Enter interactive shell
    client.interactive();
    client.disconnect();
    println!("Goodbye.");
}
