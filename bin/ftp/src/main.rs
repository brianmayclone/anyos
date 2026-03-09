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

    /// List remote entries via LIST for tab completion.
    /// Returns (name, is_dir) pairs.
    fn list_entries(&mut self) -> Vec<(String, bool)> {
        let data_sock = match self.open_data_connection() {
            Some(s) => s,
            None => return Vec::new(),
        };
        self.send_cmd_only("LIST");
        let resp = self.read_response();
        if !resp.starts_with("150") && !resp.starts_with("125") {
            net::tcp_close(data_sock);
            return Vec::new();
        }
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = net::tcp_recv(data_sock, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            raw.extend_from_slice(&buf[..n as usize]);
        }
        net::tcp_close(data_sock);
        let _ = self.read_response();
        let text = String::from_utf8_lossy(&raw).into_owned();
        let mut entries = Vec::new();
        for line in text.lines() {
            let t = line.trim();
            // LIST format: "drwxrwxrwx ... name" or "-rwxrwxrwx ... name"
            if t.len() < 10 { continue; }
            let is_dir = t.as_bytes()[0] == b'd';
            // Name is after the last whitespace-separated field
            // Skip permissions, links, owner, group, size, month, day, time/year
            if let Some(name) = parse_list_name(t) {
                if name != "." && name != ".." {
                    entries.push((String::from(name), is_dir));
                }
            }
        }
        entries
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
            let line_pos = read_line_with_tab(&mut line_buf, self);
            if line_pos == usize::MAX {
                break;
            }
            let line = core::str::from_utf8(&line_buf[..line_pos])
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
                "lpwd" => {
                    let mut buf = [0u8; 256];
                    let len = fs::getcwd(&mut buf);
                    if len > 0 && (len as usize) <= buf.len() {
                        if let Ok(s) = core::str::from_utf8(&buf[..len as usize]) {
                            println!("{}", s);
                        }
                    } else {
                        println!("/");
                    }
                }
                "lcd" => {
                    if args.is_empty() {
                        println!("Usage: lcd <path>");
                    } else if fs::chdir(args) != 0 {
                        println!("lcd: {}: No such directory", args);
                    }
                }
                "lls" | "ldir" => {
                    let dir = if args.is_empty() { "." } else { args };
                    let mut dir_buf = [0u8; 64 * 128];
                    let count = fs::readdir(dir, &mut dir_buf);
                    if count == u32::MAX {
                        println!("lls: cannot read directory");
                    } else {
                        for i in 0..count as usize {
                            let off = i * 64;
                            if off + 8 > dir_buf.len() { break; }
                            let entry_type = dir_buf[off];
                            let name_len = dir_buf[off + 1] as usize;
                            if name_len == 0 || name_len > 56 { break; }
                            if off + 8 + name_len > dir_buf.len() { break; }
                            if let Ok(name) = core::str::from_utf8(&dir_buf[off + 8..off + 8 + name_len]) {
                                let size = u32::from_le_bytes([dir_buf[off+4], dir_buf[off+5], dir_buf[off+6], dir_buf[off+7]]);
                                if entry_type == 1 {
                                    println!("  [{}]", name);
                                } else {
                                    println!("  {:>8}  {}", size, name);
                                }
                            }
                        }
                    }
                }
                "lmkdir" => {
                    if args.is_empty() {
                        println!("Usage: lmkdir <path>");
                    } else if fs::mkdir(args) != 0 {
                        println!("lmkdir: failed to create {}", args);
                    }
                }
                "lrm" => {
                    if args.is_empty() {
                        println!("Usage: lrm <file>");
                    } else if fs::unlink(args) != 0 {
                        println!("lrm: failed to delete {}", args);
                    }
                }
                "help" | "?" => {
                    println!("Remote commands:");
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
                    println!("Local commands:");
                    println!("  lpwd                    Print local directory");
                    println!("  lcd <path>              Change local directory");
                    println!("  lls [path]              List local directory");
                    println!("  lmkdir <path>           Create local directory");
                    println!("  lrm <file>              Delete local file");
                    println!("Other:");
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

/// Extract filename from a LIST line.
/// Format: "drwxrwxrwx  user  group  size  month day time  name"
/// We skip the first 8 whitespace-separated fields, rest is the filename.
fn parse_list_name(line: &str) -> Option<&str> {
    let mut fields = 0;
    let mut in_space = true;
    for (i, b) in line.bytes().enumerate() {
        if b == b' ' || b == b'\t' {
            if !in_space {
                in_space = true;
            }
        } else {
            if in_space {
                fields += 1;
                if fields == 9 {
                    let name = line[i..].trim_end();
                    if !name.is_empty() {
                        return Some(name);
                    }
                    return None;
                }
                in_space = false;
            }
        }
    }
    None
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

// All FTP commands for command-name completion
const FTP_COMMANDS: &[&str] = &[
    "ls", "dir", "list", "pwd", "cd", "get", "put", "upload",
    "mkdir", "md", "rmdir", "rd", "delete", "del", "rm",
    "rename", "ren", "mv", "size", "passive", "pasv",
    "active", "port", "binary", "bin", "ascii",
    "lpwd", "lcd", "lls", "ldir", "lmkdir", "lrm",
    "help", "bye", "quit", "exit",
];

// Commands that take remote file arguments (all entries)
fn needs_remote_files(cmd: &str) -> bool {
    matches!(cmd, "get" | "delete" | "del" | "rm" | "size" | "rename" | "ren" | "mv")
}

// Commands that take remote directory arguments (dirs only)
fn needs_remote_dirs(cmd: &str) -> bool {
    matches!(cmd, "cd" | "rmdir" | "rd")
}

// Commands that take local file arguments (all entries)
fn needs_local_files(cmd: &str) -> bool {
    matches!(cmd, "put" | "upload" | "lls" | "ldir" | "lrm")
}

// Commands that take local directory arguments (dirs only)
fn needs_local_dirs(cmd: &str) -> bool {
    matches!(cmd, "lcd" | "lmkdir")
}

/// List local filenames in a directory for tab completion.
/// If `dirs_only`, only return directories.
fn local_file_names(dir: &str, dirs_only: bool) -> Vec<String> {
    let d = if dir.is_empty() { "." } else { dir };
    let mut dir_buf = [0u8; 64 * 128];
    let count = fs::readdir(d, &mut dir_buf);
    if count == u32::MAX { return Vec::new(); }
    let mut names = Vec::new();
    for i in 0..count as usize {
        let off = i * 64;
        if off + 8 > dir_buf.len() { break; }
        let name_len = dir_buf[off + 1] as usize;
        if name_len == 0 || name_len > 56 { break; }
        if off + 8 + name_len > dir_buf.len() { break; }
        if let Ok(name) = core::str::from_utf8(&dir_buf[off + 8..off + 8 + name_len]) {
            if name != "." && name != ".." {
                let is_dir = dir_buf[off] == 1;
                if dirs_only && !is_dir { continue; }
                let mut entry = String::from(name);
                if is_dir { entry.push('/'); }
                names.push(entry);
            }
        }
    }
    names
}

/// Erase `n` characters on screen (backspace + space + backspace).
fn erase_chars(n: usize) {
    for _ in 0..n {
        print_str("\x08 \x08");
    }
}

/// Read a line with tab completion. Returns length, or usize::MAX on EOF.
fn read_line_with_tab(buf: &mut [u8], ftp: &mut FtpClient) -> usize {
    let mut pos = 0usize;
    // Cache remote entries (fetched lazily on first remote-tab)
    let mut remote_entries: Option<Vec<(String, bool)>> = None;

    loop {
        let mut byte = [0u8; 1];
        let n = fs::read(0, &mut byte);
        if n == u32::MAX {
            return usize::MAX;
        }
        if n == 0 {
            anyos_std::process::sleep(10);
            continue;
        }
        match byte[0] {
            0x03 | 0x04 => {
                print_str("\n");
                return usize::MAX;
            }
            b'\n' => {
                print_str("\n");
                return pos;
            }
            b'\r' => {
                // Ignore CR — terminal sends \n for Enter.
                // Stray \r from VNC or other sources should not trigger line submit.
                continue;
            }
            8 | 127 => {
                if pos > 0 {
                    pos -= 1;
                    print_str("\x08 \x08");
                }
            }
            // Tab
            b'\t' => {
                let line = core::str::from_utf8(&buf[..pos]).unwrap_or("");
                if let Some(completion) = tab_complete(line, ftp, &mut remote_entries) {
                    // Erase current line on screen, replace with completed version
                    erase_chars(pos);
                    let clen = completion.len().min(buf.len());
                    buf[..clen].copy_from_slice(&completion.as_bytes()[..clen]);
                    pos = clen;
                    print_str(&completion[..clen]);
                }
            }
            c if c >= b' ' => {
                if pos < buf.len() {
                    buf[pos] = c;
                    pos += 1;
                    fs::write(1, &[c]);
                }
            }
            _ => {}
        }
    }
}

/// Find a tab completion for the current line.
fn tab_complete(line: &str, ftp: &mut FtpClient, remote_cache: &mut Option<Vec<(String, bool)>>) -> Option<String> {
    let trimmed = line.trim_start();
    if let Some(space_pos) = trimmed.find(' ') {
        // Command + partial argument
        let cmd = &trimmed[..space_pos];
        let arg_part = &trimmed[space_pos + 1..];

        // Remote completion
        if needs_remote_files(cmd) || needs_remote_dirs(cmd) {
            let dirs_only = needs_remote_dirs(cmd);
            if remote_cache.is_none() {
                *remote_cache = Some(ftp.list_entries());
            }
            let entries = remote_cache.as_ref()?;
            // Build candidate names (dirs get trailing /)
            let candidates: Vec<String> = entries.iter()
                .filter(|(_, is_dir)| !dirs_only || *is_dir)
                .map(|(name, is_dir)| {
                    if *is_dir {
                        let mut s = name.clone();
                        if !s.ends_with('/') { s.push('/'); }
                        s
                    } else {
                        name.clone()
                    }
                })
                .collect();
            return complete_from_list(cmd, arg_part, &candidates, line);
        }

        // Local completion
        if needs_local_files(cmd) || needs_local_dirs(cmd) {
            let dirs_only = needs_local_dirs(cmd);
            let dir = if let Some(slash) = arg_part.rfind('/') {
                &arg_part[..slash + 1]
            } else {
                ""
            };
            let names = local_file_names(dir, dirs_only);
            let full: Vec<String> = names.iter().map(|n| format!("{}{}", dir, n)).collect();
            return complete_from_list(cmd, arg_part, &full, line);
        }

        None
    } else {
        // Command name completion
        let matches: Vec<&&str> = FTP_COMMANDS.iter().filter(|c| c.starts_with(trimmed)).collect();
        if matches.len() == 1 {
            let mut result = String::from(*matches[0]);
            result.push(' ');
            return Some(result);
        } else if matches.len() > 1 {
            let strs: Vec<&str> = matches.iter().map(|s| **s).collect();
            let common = common_prefix(&strs);
            if common.len() > trimmed.len() {
                return Some(String::from(common));
            }
            show_candidates_str(&strs, line);
        }
        None
    }
}

/// Complete arg_part from a list of candidates. Returns full line if matched.
fn complete_from_list(cmd: &str, prefix: &str, candidates: &[String], line: &str) -> Option<String> {
    let matches: Vec<&String> = candidates.iter().filter(|n| n.starts_with(prefix)).collect();
    if matches.len() == 1 {
        return Some(format!("{} {}", cmd, matches[0]));
    } else if matches.len() > 1 {
        let strs: Vec<&str> = matches.iter().map(|s| s.as_str()).collect();
        let common = common_prefix(&strs);
        if common.len() > prefix.len() {
            return Some(format!("{} {}", cmd, common));
        }
        // Show short names (after last /)
        print_str("\n");
        for m in &matches {
            let name = m.rsplit('/').next().unwrap_or(m);
            print_str(name);
            print_str("  ");
        }
        print_str("\nftp> ");
        print_str(line);
    }
    None
}

fn show_candidates_str(candidates: &[&str], line: &str) {
    print_str("\n");
    for m in candidates {
        print_str(m);
        print_str("  ");
    }
    print_str("\nftp> ");
    print_str(line);
}

/// Find the longest common prefix of a set of strings.
fn common_prefix<'a>(strings: &[&'a str]) -> &'a str {
    if strings.is_empty() { return ""; }
    let first = strings[0];
    let mut len = first.len();
    for s in &strings[1..] {
        len = len.min(s.len());
        for (i, (a, b)) in first.bytes().zip(s.bytes()).enumerate() {
            if a != b {
                len = len.min(i);
                break;
            }
        }
    }
    &first[..len]
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
