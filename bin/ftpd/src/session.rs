//! FTP session handler — RFC 959 server-side protocol.
//!
//! Each session runs in a forked child process. It handles one client
//! connection on the control socket until the client disconnects or sends QUIT.

use anyos_std::{net, fs, process, log_info, log_warn};
use alloc::vec::Vec;
use alloc::string::String;
use crate::config::FtpdConfig;

const BUF_SIZE: usize = 4096;
const CMD_BUF: usize = 512;

struct Session<'a> {
    ctrl: u32,
    cfg: &'a FtpdConfig,
    username: [u8; 64],
    username_len: usize,
    authenticated: bool,
    anonymous: bool,
    /// Current working directory (absolute path).
    cwd: [u8; 256],
    cwd_len: usize,
    /// Passive port counter for cycling through passive port range.
    pasv_port_counter: u16,
    /// Pending RNFR path for RNTO.
    rnfr: [u8; 256],
    rnfr_len: usize,
    /// Remote IP address for log messages.
    remote_ip: [u8; 4],
}

impl<'a> Session<'a> {
    fn new(ctrl: u32, cfg: &'a FtpdConfig, remote_ip: [u8; 4]) -> Self {
        let mut s = Session {
            ctrl,
            cfg,
            username: [0; 64],
            username_len: 0,
            authenticated: false,
            anonymous: false,
            cwd: [0; 256],
            cwd_len: 1,
            pasv_port_counter: 0,
            rnfr: [0; 256],
            rnfr_len: 0,
            remote_ip,
        };
        s.cwd[0] = b'/';
        s
    }

    fn log_info(&self, msg: &str) {
        let ip = self.remote_ip;
        let user = self.username_str();
        log_info!("{}.{}.{}.{} [{}] {}", ip[0], ip[1], ip[2], ip[3], user, msg);
    }

    fn log_warn(&self, msg: &str) {
        let ip = self.remote_ip;
        let user = self.username_str();
        log_warn!("{}.{}.{}.{} [{}] {}", ip[0], ip[1], ip[2], ip[3], user, msg);
    }

    fn username_str(&self) -> &str {
        core::str::from_utf8(&self.username[..self.username_len]).unwrap_or("")
    }

    fn cwd_str(&self) -> &str {
        core::str::from_utf8(&self.cwd[..self.cwd_len]).unwrap_or("/")
    }

    fn set_cwd(&mut self, path: &str) {
        let plen = path.len().min(256);
        self.cwd[..plen].copy_from_slice(&path.as_bytes()[..plen]);
        self.cwd_len = plen;
    }

    /// Resolve a relative or absolute client path against current working directory.
    fn resolve_path(&self, path: &str) -> String {
        let abs = if path.starts_with('/') {
            String::from(path)
        } else {
            let cwd = self.cwd_str();
            let mut s = String::from(cwd);
            if !cwd.ends_with('/') {
                s.push('/');
            }
            s.push_str(path);
            s
        };
        normalize_path(&abs)
    }

    /// Check if authenticated user can access path (read or write).
    fn can_access(&self, path: &str, write: bool) -> bool {
        if self.anonymous {
            let root = self.cfg.anonymous_root_str();
            // Anonymous: read-only within anonymous_root (with proper boundary)
            if write { return false; }
            return path == root
                || (path.starts_with(root)
                    && path.as_bytes().get(root.len()) == Some(&b'/'));
        }
        // Share permissions are always enforced.
        // chroot_users only controls whether the user can navigate *outside*
        // their share root — it does not bypass permission checks.
        if self.cfg.shares_count == 0 {
            // No shares configured at all: allow everything (open server).
            return true;
        }
        self.cfg.check_access(self.username_str(), path, write)
    }

    fn send_raw(&mut self, data: &[u8]) {
        net::tcp_send(self.ctrl, data);
    }

    fn reply(&mut self, code: &str, msg: &str) {
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(code.as_bytes());
        buf.push(b' ');
        buf.extend_from_slice(msg.as_bytes());
        buf.push(b'\r');
        buf.push(b'\n');
        self.send_raw(&buf);
    }

    /// Open a PASV listener on a port from the configured range.
    /// Sends the 227 reply and returns (listener_socket) if successful.
    fn open_pasv_listener(&mut self) -> Option<u32> {
        let range = self.cfg.passive_port_max.saturating_sub(self.cfg.passive_port_min).max(1);

        // Try up to `range` ports starting from the current counter offset,
        // so concurrent sessions don't all collide on port 50000.
        let start_offset = self.pasv_port_counter % range;
        let mut listener = u32::MAX;
        let mut chosen_port = self.cfg.passive_port_min;
        for attempt in 0..range {
            let offset = (start_offset + attempt) % range;
            let port = self.cfg.passive_port_min + offset;
            let l = net::tcp_listen(port, 1);
            if l != u32::MAX {
                listener = l;
                chosen_port = port;
                self.pasv_port_counter = offset + 1;
                break;
            }
        }

        if listener == u32::MAX {
            self.reply("425", "Can't open data connection.");
            return None;
        }
        let port = chosen_port;

        // Use masquerade_ip if configured (non-zero), otherwise use actual NIC IP.
        let ip = if self.cfg.masquerade_ip != [0u8; 4] {
            self.cfg.masquerade_ip
        } else {
            let mut net_buf = [0u8; 24];
            if net::get_config(&mut net_buf) == 0 {
                [net_buf[0], net_buf[1], net_buf[2], net_buf[3]]
            } else {
                [127u8, 0, 0, 1]
            }
        };

        self.log_info(&alloc::format!("passive mode: listening on port {}", port));

        let p1 = (port >> 8) as u8;
        let p2 = (port & 0xFF) as u8;
        let resp = format_227(&ip, p1, p2);
        self.send_raw(resp.as_bytes());
        Some(listener)
    }

    /// Main session event loop.
    pub fn run(&mut self) {
        let ip = self.remote_ip;
        log_info!("{}.{}.{}.{} connected", ip[0], ip[1], ip[2], ip[3]);
        self.reply("220", "anyOS FTP Server ready.");

        let mut recv_buf = [0u8; CMD_BUF];
        let mut acc: Vec<u8> = Vec::new();

        loop {
            let n = net::tcp_recv(self.ctrl, &mut recv_buf);
            if n == 0 || n == u32::MAX {
                self.log_info("connection closed");
                break;
            }
            acc.extend_from_slice(&recv_buf[..n as usize]);

            // Process all complete lines
            loop {
                match find_crlf(&acc) {
                    None => break,
                    Some(end) => {
                        let line_bytes: Vec<u8> = acc[..end].to_vec();
                        acc.drain(..end + 2);
                        let line = core::str::from_utf8(&line_bytes).unwrap_or("").trim();
                        if line.is_empty() { continue; }
                        let (cmd, arg) = split_cmd(line);
                        let done = self.dispatch(cmd, arg);
                        if done { return; }
                    }
                }
            }
        }
    }

    /// Dispatch a FTP command. Returns true to end session.
    fn dispatch(&mut self, cmd: &str, arg: &str) -> bool {
        // Commands that don't need authentication
        match cmd {
            "USER" => return self.cmd_user(arg),
            "PASS" => return self.cmd_pass(arg),
            "QUIT" => {
                self.log_info("disconnected");
                self.reply("221", "Goodbye.");
                return true;
            }
            "SYST" => { self.reply("215", "UNIX Type: L8"); return false; }
            "FEAT" => {
                self.send_raw(b"211-Features:\r\n SIZE\r\n MDTM\r\n MLSD\r\n PASV\r\n EPSV\r\n EPRT\r\n REST STREAM\r\n UTF8\r\n211 End\r\n");
                return false;
            }
            "OPTS" => {
                // OPTS UTF8 ON — acknowledge UTF-8 support
                if arg.eq_ignore_ascii_case("UTF8 ON") || arg.eq_ignore_ascii_case("UTF-8 ON") {
                    self.reply("200", "UTF-8 enabled.");
                } else {
                    self.reply("501", "Unknown option.");
                }
                return false;
            }
            "NOOP" => { self.reply("200", "NOOP ok."); return false; }
            "TYPE" => { self.reply("200", "Type set."); return false; }
            _ => {}
        }

        if !self.authenticated {
            self.reply("530", "Please login with USER and PASS.");
            return false;
        }

        match cmd {
            "PWD" | "XPWD" => self.cmd_pwd(),
            "CWD" | "XCWD" => self.cmd_cwd(arg),
            "CDUP" | "XCUP" => self.cmd_cdup(),
            "PASV" => {
                if self.cfg.passive_mode {
                    self.cmd_pasv_list(arg)
                } else {
                    self.reply("502", "Passive mode disabled.");
                }
            }
            "EPSV" => {
                if self.cfg.passive_mode {
                    self.cmd_epsv_list(arg)
                } else {
                    self.reply("502", "Passive mode disabled.");
                }
            }
            "PORT" => self.cmd_port_list(arg),
            "LIST" | "NLST" => self.cmd_list_auto(arg),
            "RETR" => self.cmd_retr_auto(arg),
            "STOR" | "STOU" => self.cmd_stor_auto(arg),
            "DELE" => self.cmd_dele(arg),
            "MKD" | "XMKD" => self.cmd_mkd(arg),
            "RMD" | "XRMD" => self.cmd_rmd(arg),
            "RNFR" => self.cmd_rnfr(arg),
            "RNTO" => self.cmd_rnto(arg),
            "SIZE" => self.cmd_size(arg),
            "MDTM" => self.cmd_mdtm(arg),
            "MLSD" => self.cmd_mlsd_auto(arg),
            "MLST" => self.cmd_mlst(arg),
            "EPRT" => self.cmd_eprt_list(arg),
            "REST" => { self.reply("350", "Restart position accepted."); }
            "ABOR" => { self.reply("226", "ABOR ok."); }
            _ => { self.reply("502", "Command not implemented."); }
        }
        false
    }

    fn cmd_user(&mut self, arg: &str) -> bool {
        self.authenticated = false;
        self.anonymous = false;
        let ulen = arg.len().min(64);
        self.username[..ulen].copy_from_slice(&arg.as_bytes()[..ulen]);
        self.username_len = ulen;

        if arg == "anonymous" || arg == "ftp" {
            if self.cfg.allow_anonymous {
                self.reply("331", "Guest login ok, send email as password.");
            } else {
                self.reply("530", "Anonymous access disabled.");
                self.username_len = 0;
            }
        } else {
            self.reply("331", "Password required.");
        }
        false
    }

    fn cmd_pass(&mut self, arg: &str) -> bool {
        if self.username_len == 0 {
            self.reply("503", "Login with USER first.");
            return false;
        }
        let uname = core::str::from_utf8(&self.username[..self.username_len]).unwrap_or("");
        if uname == "anonymous" || uname == "ftp" {
            if self.cfg.allow_anonymous {
                self.authenticated = true;
                self.anonymous = true;
                let root = self.cfg.anonymous_root_str();
                let rlen = root.len().min(256);
                self.cwd[..rlen].copy_from_slice(&root.as_bytes()[..rlen]);
                self.cwd_len = rlen;
                self.reply("230", "Guest login ok.");
                self.log_info("login ok (anonymous)");
            } else {
                self.reply("530", "Anonymous access disabled.");
                self.log_warn("login denied (anonymous disabled)");
            }
        } else if process::authenticate(uname, arg) {
            self.authenticated = true;
            let root = self.cfg.user_root(uname);
            let rlen = root.len().min(256);
            self.cwd[..rlen].copy_from_slice(&root.as_bytes()[..rlen]);
            self.cwd_len = rlen;
            self.reply("230", "Login successful.");
            self.log_info("login ok");
        } else {
            self.reply("530", "Login incorrect.");
            self.log_warn("login failed (wrong password)");
            self.username_len = 0;
        }
        false
    }

    fn cmd_pwd(&mut self) {
        let cwd = String::from(self.cwd_str());
        let mut msg = String::from("\"");
        msg.push_str(&cwd);
        msg.push_str("\" is current directory");
        self.reply("257", &msg);
    }

    fn cmd_cwd(&mut self, arg: &str) {
        let new_path = self.resolve_path(arg);
        if !self.can_access(&new_path, false) {
            self.reply("550", "Permission denied.");
            return;
        }
        let mut stat_buf = [0u32; 7];
        if fs::stat(&new_path, &mut stat_buf) == u32::MAX {
            self.reply("550", "No such file or directory.");
            return;
        }
        self.set_cwd(&new_path);
        self.reply("250", "CWD command successful.");
    }

    fn cmd_cdup(&mut self) {
        let cwd = String::from(self.cwd_str());
        let parent = String::from(parent_dir(&cwd));
        if !self.can_access(&parent, false) {
            self.reply("550", "Permission denied.");
            return;
        }
        self.set_cwd(&parent);
        self.reply("250", "CDUP command successful.");
    }

    /// EPSV — Extended Passive Mode (RFC 2428).
    ///
    /// Like PASV but the 229 reply carries only the port number, no IP.
    /// The client reuses the control-connection IP for the data port.
    /// This works correctly through NAT/port-forwarding because the client
    /// connects to the same IP it used for the control connection.
    fn cmd_epsv_list(&mut self, _arg: &str) {
        let (listener, port) = match self.open_epsv_listener() {
            Some(v) => v,
            None => return,
        };
        let next = self.read_one_command();
        if next.is_empty() {
            net::tcp_close(listener);
            return;
        }
        let (cmd, arg) = split_cmd_owned(&next);
        match cmd.as_str() {
            "LIST" | "NLST" => self.do_list_pasv(listener, &arg),
            "MLSD"          => self.do_mlsd_pasv(listener, &arg),
            "RETR"          => self.do_retr_pasv(listener, &arg),
            "STOR" | "STOU" => self.do_stor_pasv(listener, &arg),
            _ => {
                net::tcp_close(listener);
                self.reply("503", "Bad sequence of commands.");
            }
        }
        let _ = port;
    }

    /// Open a PASV listener and send 229 (EPSV) — port only, no IP.
    fn open_epsv_listener(&mut self) -> Option<(u32, u16)> {
        let range = self.cfg.passive_port_max.saturating_sub(self.cfg.passive_port_min).max(1);
        let start_offset = self.pasv_port_counter % range;
        let mut listener = u32::MAX;
        let mut chosen_port = self.cfg.passive_port_min;
        for attempt in 0..range {
            let offset = (start_offset + attempt) % range;
            let port = self.cfg.passive_port_min + offset;
            let l = net::tcp_listen(port, 1);
            if l != u32::MAX {
                listener = l;
                chosen_port = port;
                self.pasv_port_counter = offset + 1;
                break;
            }
        }
        if listener == u32::MAX {
            self.reply("425", "Can't open data connection.");
            return None;
        }
        self.log_info(&alloc::format!("epsv: listening on port {}", chosen_port));
        // 229 reply: port only, no IP — client uses control-connection IP
        let mut resp = String::from("229 Entering Extended Passive Mode (|||");
        let mut port_buf = [0u8; 6];
        let ps = fmt_u32(chosen_port as u32, &mut port_buf);
        resp.push_str(core::str::from_utf8(ps).unwrap_or("0"));
        resp.push_str("|)\r\n");
        self.send_raw(resp.as_bytes());
        Some((listener, chosen_port))
    }

    /// PASV + wait for next data command inline (LIST/RETR/STOR).
    ///
    /// RFC 959 flow (what FileZilla / WinSCP do):
    ///   1. PASV  → 227 (server opens listener)
    ///   2. LIST  → server reads command on control socket
    ///   3. 150   → server sends 150, which triggers the client to connect data port
    ///   4. data connection accepted (blocking)
    ///   5. transfer → 226
    ///
    /// The listener is passed down to do_list/do_retr/do_stor so they can
    /// send 150 *before* accepting — the 150 is what signals the client to
    /// connect to the data port.
    fn cmd_pasv_list(&mut self, _arg: &str) {
        let listener = match self.open_pasv_listener() {
            Some(l) => l,
            None => return,
        };

        // Read the transfer command from the control socket (blocking, up to 30 s).
        let next = self.read_one_command();
        if next.is_empty() {
            net::tcp_close(listener);
            return;
        }
        let (cmd, arg) = split_cmd_owned(&next);
        match cmd.as_str() {
            "LIST" | "NLST" => self.do_list_pasv(listener, &arg),
            "MLSD"          => self.do_mlsd_pasv(listener, &arg),
            "RETR"          => self.do_retr_pasv(listener, &arg),
            "STOR" | "STOU" => self.do_stor_pasv(listener, &arg),
            _ => {
                net::tcp_close(listener);
                self.reply("503", "Bad sequence of commands.");
            }
        }
    }

    /// Accept a data connection from a PASV listener (blocking, 30 s timeout).
    /// Closes the listener regardless. Returns the data socket or u32::MAX.
    fn accept_pasv_listener(&mut self, listener: u32) -> u32 {
        let (sock, _, _) = net::tcp_accept(listener);
        net::tcp_close(listener);
        if sock == u32::MAX {
            self.reply("425", "Failed to establish data connection.");
        }
        sock
    }

    /// PORT + wait for next data command inline
    fn cmd_port_list(&mut self, arg: &str) {
        let (port_ip, port) = match parse_port_arg(arg) {
            Some(v) => v,
            None => { self.reply("501", "Syntax error in PORT."); return; }
        };
        // If the client reports a loopback or unspecified address (common when the
        // client itself is behind NAT and sees its own connection as 127.x), use the
        // actual remote IP of the control connection instead.
        let ip = if port_ip[0] == 127 || port_ip == [0, 0, 0, 0] {
            self.remote_ip
        } else {
            port_ip
        };
        self.log_info(&alloc::format!(
            "active mode: connecting to {}.{}.{}.{}:{}",
            ip[0], ip[1], ip[2], ip[3], port
        ));
        self.reply("200", "PORT command successful.");
        let next = self.read_one_command();
        let (cmd, darg) = split_cmd_owned(&next);
        let data_sock = net::tcp_connect(&ip, port, 5000);
        if data_sock == u32::MAX {
            self.reply("425", "Can't open data connection.");
            return;
        }
        match cmd.as_str() {
            "LIST" | "NLST" => self.do_list(data_sock, &darg),
            "MLSD"          => self.do_mlsd(data_sock, &darg),
            "RETR" => self.do_retr(data_sock, &darg),
            "STOR" | "STOU" => self.do_stor(data_sock, &darg),
            _ => {
                net::tcp_close(data_sock);
                self.reply("503", "Bad sequence of commands.");
            }
        }
    }

    /// LIST/RETR/STOR without prior PASV/PORT — client must use PASV or PORT first.
    fn cmd_list_auto(&mut self, _arg: &str) {
        self.reply("425", "Use PORT or PASV first.");
    }

    fn cmd_retr_auto(&mut self, _arg: &str) {
        self.reply("425", "Use PORT or PASV first.");
    }

    fn cmd_stor_auto(&mut self, _arg: &str) {
        self.reply("425", "Use PORT or PASV first.");
    }

    fn cmd_dele(&mut self, arg: &str) {
        let path = self.resolve_path(arg);
        if !self.can_access(&path, true) {
            self.reply("550", "Permission denied.");
            self.log_warn(&alloc::format!("delete denied: {}", path));
            return;
        }
        if fs::unlink(&path) == 0 {
            self.reply("250", "DELE command successful.");
            self.log_info(&alloc::format!("deleted: {}", path));
        } else {
            self.reply("550", "Delete failed.");
            self.log_warn(&alloc::format!("delete failed: {}", path));
        }
    }

    fn cmd_mkd(&mut self, arg: &str) {
        let path = self.resolve_path(arg);
        if !self.can_access(&path, true) {
            self.reply("550", "Permission denied.");
            self.log_warn(&alloc::format!("mkdir denied: {}", path));
            return;
        }
        if fs::mkdir(&path) == 0 {
            let mut msg = String::from("\"");
            msg.push_str(&path);
            msg.push_str("\" directory created.");
            self.reply("257", &msg);
            self.log_info(&alloc::format!("mkdir: {}", path));
        } else {
            self.reply("550", "MKD failed.");
            self.log_warn(&alloc::format!("mkdir failed: {}", path));
        }
    }

    fn cmd_rmd(&mut self, arg: &str) {
        let path = self.resolve_path(arg);
        if !self.can_access(&path, true) {
            self.reply("550", "Permission denied.");
            self.log_warn(&alloc::format!("rmdir denied: {}", path));
            return;
        }
        if fs::unlink(&path) == 0 {
            self.reply("250", "RMD command successful.");
            self.log_info(&alloc::format!("rmdir: {}", path));
        } else {
            self.reply("550", "RMD failed.");
            self.log_warn(&alloc::format!("rmdir failed: {}", path));
        }
    }

    fn cmd_rnfr(&mut self, arg: &str) {
        let path = self.resolve_path(arg);
        if !self.can_access(&path, true) {
            self.reply("550", "Permission denied.");
            return;
        }
        let plen = path.len().min(256);
        self.rnfr[..plen].copy_from_slice(&path.as_bytes()[..plen]);
        self.rnfr_len = plen;
        self.reply("350", "Ready for RNTO.");
    }

    fn cmd_rnto(&mut self, arg: &str) {
        if self.rnfr_len == 0 {
            self.reply("503", "Need RNFR first.");
            return;
        }
        let old = String::from(
            core::str::from_utf8(&self.rnfr[..self.rnfr_len]).unwrap_or("")
        );
        let new_path = self.resolve_path(arg);
        self.rnfr_len = 0;
        if fs::rename(&old, &new_path) == 0 {
            self.reply("250", "RNTO command successful.");
            self.log_info(&alloc::format!("renamed: {} -> {}", old, new_path));
        } else {
            self.reply("550", "Rename failed.");
            self.log_warn(&alloc::format!("rename failed: {} -> {}", old, new_path));
        }
    }

    fn cmd_size(&mut self, arg: &str) {
        let path = self.resolve_path(arg);
        if !self.can_access(&path, false) {
            self.reply("550", "Permission denied.");
            return;
        }
        let mut stat_buf = [0u32; 7];
        if fs::stat(&path, &mut stat_buf) == u32::MAX {
            self.reply("550", "No such file.");
            return;
        }
        // stat_buf[1] = file size (low 32 bits)
        let size = stat_buf[1];
        let mut buf = [0u8; 12];
        let s = fmt_u32(size, &mut buf);
        self.reply("213", core::str::from_utf8(s).unwrap_or("0"));
    }

    fn do_list(&mut self, data_sock: u32, _arg: &str) {
        let cwd = String::from(self.cwd_str());
        if !self.can_access(&cwd, false) {
            self.reply("550", "Permission denied.");
            net::tcp_close(data_sock);
            return;
        }

        self.reply("150", "Opening data connection for directory listing.");

        let mut dir_buf = [0u8; 64 * 256];
        let count = fs::readdir(&cwd, &mut dir_buf);

        let mut output: Vec<u8> = Vec::new();
        for i in 0..count as usize {
            // readdir entry layout: [type:u8, name_len:u8, flags:u8, pad:u8, size:u32, name:56bytes]
            let entry_offset = i * 64;
            if entry_offset + 8 > dir_buf.len() { break; }
            let entry_type = dir_buf[entry_offset];
            let name_len = dir_buf[entry_offset + 1] as usize;
            if name_len == 0 || name_len > 56 { break; }
            if entry_offset + 8 + name_len > dir_buf.len() { break; }

            let size_bytes = [
                dir_buf[entry_offset + 4],
                dir_buf[entry_offset + 5],
                dir_buf[entry_offset + 6],
                dir_buf[entry_offset + 7],
            ];
            let size = u32::from_le_bytes(size_bytes);
            let name = match core::str::from_utf8(&dir_buf[entry_offset + 8..entry_offset + 8 + name_len]) {
                Ok(n) => n,
                Err(_) => continue,
            };
            if name.is_empty() { break; }

            if entry_type == 1 {
                // Directory
                output.extend_from_slice(b"drwxr-xr-x  1 ftp  ftp          0 Jan 01 00:00 ");
            } else {
                // File (type 0) or device (type 2)
                output.extend_from_slice(b"-rw-r--r--  1 ftp  ftp  ");
                let mut size_buf = [0u8; 12];
                let s = fmt_u32(size, &mut size_buf);
                for _ in s.len()..8 { output.push(b' '); }
                output.extend_from_slice(s);
                output.extend_from_slice(b" Jan 01 00:00 ");
            }
            output.extend_from_slice(name.as_bytes());
            output.push(b'\r');
            output.push(b'\n');
        }

        net::tcp_send(data_sock, &output);
        net::tcp_close(data_sock);
        self.reply("226", "Transfer complete.");
    }

    fn do_retr(&mut self, data_sock: u32, path: &str) {
        let abs = self.resolve_path(path);
        if !self.can_access(&abs, false) {
            self.reply("550", "Permission denied.");
            net::tcp_close(data_sock);
            self.log_warn(&alloc::format!("download denied: {}", abs));
            return;
        }
        let fd = fs::open(&abs, 0);
        if fd == u32::MAX {
            self.reply("550", "No such file or directory.");
            net::tcp_close(data_sock);
            return;
        }
        self.reply("150", "Opening data connection.");
        let mut total: u64 = 0;
        let mut buf = [0u8; BUF_SIZE];
        loop {
            let n = fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            total += n as u64;
            if net::tcp_send(data_sock, &buf[..n as usize]) == u32::MAX { break; }
        }
        fs::close(fd);
        net::tcp_close(data_sock);
        self.reply("226", "Transfer complete.");
        self.log_info(&alloc::format!("download: {} ({} bytes)", abs, total));
    }

    fn do_stor(&mut self, data_sock: u32, path: &str) {
        let abs = self.resolve_path(path);
        if !self.can_access(&abs, true) {
            self.reply("550", "Permission denied.");
            net::tcp_close(data_sock);
            self.log_warn(&alloc::format!("upload denied: {}", abs));
            return;
        }
        // Reject STOR if the path already exists as a directory.
        let mut stat_buf = [0u32; 7];
        if fs::stat(&abs, &mut stat_buf) != u32::MAX {
            // stat_buf[0] = type: 0=file, 1=directory, 2=device
            let is_dir = stat_buf[0] == 1;
            if is_dir {
                self.reply("553", "Is a directory.");
                net::tcp_close(data_sock);
                self.log_warn(&alloc::format!("upload denied (is a directory): {}", abs));
                return;
            }
        }
        let fd = fs::open(&abs, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            self.reply("550", "Could not create file.");
            net::tcp_close(data_sock);
            self.log_warn(&alloc::format!("upload failed (open): {}", abs));
            return;
        }
        self.reply("150", "Opening data connection.");
        let mut total: u64 = 0;
        let mut buf = [0u8; BUF_SIZE];
        loop {
            let n = net::tcp_recv(data_sock, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            total += n as u64;
            if fs::write(fd, &buf[..n as usize]) == u32::MAX { break; }
        }
        fs::close(fd);
        net::tcp_close(data_sock);
        self.reply("226", "Transfer complete.");
        self.log_info(&alloc::format!("upload: {} ({} bytes)", abs, total));
    }

    // ── PASV variants: send 150 first, THEN accept the data connection ───────
    //
    // RFC 959 says the server sends 150 *before* the data connection is open.
    // FileZilla and WinSCP wait for the 150 response before connecting to the
    // passive port, so we must follow this order:
    //   send 150  →  accept  →  transfer  →  226

    fn do_list_pasv(&mut self, listener: u32, arg: &str) {
        let cwd = String::from(self.cwd_str());
        if !self.can_access(&cwd, false) {
            net::tcp_close(listener);
            self.reply("550", "Permission denied.");
            return;
        }
        // Send 150 *before* accepting — this is what triggers the client to connect.
        self.reply("150", "Opening data connection for directory listing.");
        let data_sock = self.accept_pasv_listener(listener);
        if data_sock == u32::MAX { return; }

        let mut dir_buf = [0u8; 64 * 256];
        let count = fs::readdir(&cwd, &mut dir_buf);
        let mut output: Vec<u8> = Vec::new();
        for i in 0..count as usize {
            let entry_offset = i * 64;
            if entry_offset + 8 > dir_buf.len() { break; }
            let entry_type = dir_buf[entry_offset];
            let name_len = dir_buf[entry_offset + 1] as usize;
            if name_len == 0 || name_len > 56 { break; }
            if entry_offset + 8 + name_len > dir_buf.len() { break; }
            let size = u32::from_le_bytes([
                dir_buf[entry_offset + 4], dir_buf[entry_offset + 5],
                dir_buf[entry_offset + 6], dir_buf[entry_offset + 7],
            ]);
            let name = match core::str::from_utf8(&dir_buf[entry_offset + 8..entry_offset + 8 + name_len]) {
                Ok(n) => n, Err(_) => continue,
            };
            if name.is_empty() { break; }
            if entry_type == 1 {
                // Directory (type 1)
                output.extend_from_slice(b"drwxr-xr-x  1 ftp  ftp          0 Jan 01 00:00 ");
            } else {
                // File (type 0) or device (type 2)
                output.extend_from_slice(b"-rw-r--r--  1 ftp  ftp  ");
                let mut size_buf = [0u8; 12];
                let s = fmt_u32(size, &mut size_buf);
                for _ in s.len()..8 { output.push(b' '); }
                output.extend_from_slice(s);
                output.extend_from_slice(b" Jan 01 00:00 ");
            }
            output.extend_from_slice(name.as_bytes());
            output.push(b'\r');
            output.push(b'\n');
        }
        let _ = arg; // NLST variant would use path, LIST ignores it
        net::tcp_send(data_sock, &output);
        net::tcp_close(data_sock);
        self.reply("226", "Transfer complete.");
    }

    fn do_retr_pasv(&mut self, listener: u32, path: &str) {
        let abs = self.resolve_path(path);
        if !self.can_access(&abs, false) {
            net::tcp_close(listener);
            self.reply("550", "Permission denied.");
            self.log_warn(&alloc::format!("download denied: {}", abs));
            return;
        }
        let fd = fs::open(&abs, 0);
        if fd == u32::MAX {
            net::tcp_close(listener);
            self.reply("550", "No such file or directory.");
            return;
        }
        // Send 150 first, then accept — client connects after 150.
        self.reply("150", "Opening data connection.");
        let data_sock = self.accept_pasv_listener(listener);
        if data_sock == u32::MAX { fs::close(fd); return; }
        let mut total: u64 = 0;
        let mut buf = [0u8; BUF_SIZE];
        loop {
            let n = fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            total += n as u64;
            if net::tcp_send(data_sock, &buf[..n as usize]) == u32::MAX { break; }
        }
        fs::close(fd);
        net::tcp_close(data_sock);
        self.reply("226", "Transfer complete.");
        self.log_info(&alloc::format!("download: {} ({} bytes)", abs, total));
    }

    fn do_stor_pasv(&mut self, listener: u32, path: &str) {
        let abs = self.resolve_path(path);
        if !self.can_access(&abs, true) {
            net::tcp_close(listener);
            self.reply("550", "Permission denied.");
            self.log_warn(&alloc::format!("upload denied: {}", abs));
            return;
        }
        // Reject STOR if the path already exists as a directory.
        let mut stat_buf = [0u32; 7];
        if fs::stat(&abs, &mut stat_buf) != u32::MAX {
            // stat_buf[0] = type: 0=file, 1=directory, 2=device
            let is_dir = stat_buf[0] == 1;
            if is_dir {
                net::tcp_close(listener);
                self.reply("553", "Is a directory.");
                self.log_warn(&alloc::format!("upload denied (is a directory): {}", abs));
                return;
            }
        }
        let fd = fs::open(&abs, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
        if fd == u32::MAX {
            net::tcp_close(listener);
            self.reply("550", "Could not create file.");
            self.log_warn(&alloc::format!("upload failed (open): {}", abs));
            return;
        }
        // Send 150 first, then accept — client connects after 150.
        self.reply("150", "Opening data connection.");
        let data_sock = self.accept_pasv_listener(listener);
        if data_sock == u32::MAX { fs::close(fd); return; }
        let mut total: u64 = 0;
        let mut buf = [0u8; BUF_SIZE];
        loop {
            let n = net::tcp_recv(data_sock, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            total += n as u64;
            if fs::write(fd, &buf[..n as usize]) == u32::MAX { break; }
        }
        fs::close(fd);
        net::tcp_close(data_sock);
        self.reply("226", "Transfer complete.");
        self.log_info(&alloc::format!("upload: {} ({} bytes)", abs, total));
    }

    // ── MDTM — file modification time (RFC 3659) ────────────────────────────

    fn cmd_mdtm(&mut self, arg: &str) {
        let path = self.resolve_path(arg);
        if !self.can_access(&path, false) {
            self.reply("550", "Permission denied.");
            return;
        }
        let mut stat_buf = [0u32; 7];
        if fs::stat(&path, &mut stat_buf) == u32::MAX {
            self.reply("550", "No such file.");
            return;
        }
        // stat_buf[6] = mtime (unix timestamp)
        let ts = format_mdtm_timestamp(stat_buf[6]);
        self.reply("213", &ts);
    }

    // ── MLST — single entry info on control connection (RFC 3659) ─────────

    fn cmd_mlst(&mut self, arg: &str) {
        let path = if arg.is_empty() {
            String::from(self.cwd_str())
        } else {
            self.resolve_path(arg)
        };
        if !self.can_access(&path, false) {
            self.reply("550", "Permission denied.");
            return;
        }
        let mut stat_buf = [0u32; 7];
        if fs::stat(&path, &mut stat_buf) == u32::MAX {
            self.reply("550", "No such file.");
            return;
        }
        let name = path.rsplit('/').next().unwrap_or(&path);
        let entry = format_mlsd_entry(name, stat_buf[0], stat_buf[1], stat_buf[6]);
        let mut resp = String::from("250-Begin\r\n ");
        resp.push_str(&entry);
        resp.push_str("\r\n250 End\r\n");
        self.send_raw(resp.as_bytes());
    }

    // ── MLSD — machine-readable directory listing (RFC 3659) ──────────────

    fn cmd_mlsd_auto(&mut self, _arg: &str) {
        self.reply("425", "Use PORT or PASV first.");
    }

    fn do_mlsd(&mut self, data_sock: u32, arg: &str) {
        let dir = if arg.is_empty() {
            String::from(self.cwd_str())
        } else {
            self.resolve_path(arg)
        };
        if !self.can_access(&dir, false) {
            self.reply("550", "Permission denied.");
            net::tcp_close(data_sock);
            return;
        }
        self.reply("150", "Opening data connection for MLSD.");
        let output = self.build_mlsd_output(&dir);
        net::tcp_send(data_sock, &output);
        net::tcp_close(data_sock);
        self.reply("226", "Transfer complete.");
    }

    fn do_mlsd_pasv(&mut self, listener: u32, arg: &str) {
        let dir = if arg.is_empty() {
            String::from(self.cwd_str())
        } else {
            self.resolve_path(arg)
        };
        if !self.can_access(&dir, false) {
            net::tcp_close(listener);
            self.reply("550", "Permission denied.");
            return;
        }
        self.reply("150", "Opening data connection for MLSD.");
        let data_sock = self.accept_pasv_listener(listener);
        if data_sock == u32::MAX { return; }
        let output = self.build_mlsd_output(&dir);
        net::tcp_send(data_sock, &output);
        net::tcp_close(data_sock);
        self.reply("226", "Transfer complete.");
    }

    /// Build MLSD output for a directory.
    /// Format per RFC 3659: "type=<type>;size=<n>;modify=<ts>; <name>\r\n"
    fn build_mlsd_output(&self, dir: &str) -> Vec<u8> {
        let mut output: Vec<u8> = Vec::new();

        // Current directory entry
        output.extend_from_slice(b"type=cdir;modify=19700101000000; .\r\n");
        // Parent directory entry
        output.extend_from_slice(b"type=pdir;modify=19700101000000; ..\r\n");

        let mut dir_buf = [0u8; 64 * 256];
        let count = fs::readdir(dir, &mut dir_buf);
        if count == u32::MAX { return output; }

        for i in 0..count as usize {
            let entry_offset = i * 64;
            if entry_offset + 8 > dir_buf.len() { break; }
            let entry_type = dir_buf[entry_offset];
            let name_len = dir_buf[entry_offset + 1] as usize;
            if name_len == 0 || name_len > 56 { break; }
            if entry_offset + 8 + name_len > dir_buf.len() { break; }
            let size = u32::from_le_bytes([
                dir_buf[entry_offset + 4], dir_buf[entry_offset + 5],
                dir_buf[entry_offset + 6], dir_buf[entry_offset + 7],
            ]);
            let name = match core::str::from_utf8(&dir_buf[entry_offset + 8..entry_offset + 8 + name_len]) {
                Ok(n) => n, Err(_) => continue,
            };
            if name.is_empty() { break; }

            // Get mtime via stat for each entry
            let mut child_path = String::from(dir);
            if !child_path.ends_with('/') { child_path.push('/'); }
            child_path.push_str(name);
            let mut st = [0u32; 7];
            let mtime = if fs::stat(&child_path, &mut st) != u32::MAX { st[6] } else { 0 };

            let entry = format_mlsd_entry(name, entry_type as u32, size, mtime);
            output.extend_from_slice(entry.as_bytes());
            output.push(b'\r');
            output.push(b'\n');
        }
        output
    }

    // ── EPRT — Extended PORT (RFC 2428) ───────────────────────────────────

    fn cmd_eprt_list(&mut self, arg: &str) {
        // Format: EPRT |<net-prt>|<net-addr>|<tcp-port>|
        // Example: EPRT |1|192.168.1.2|50000|
        let (ip, port) = match parse_eprt_arg(arg) {
            Some(v) => v,
            None => { self.reply("501", "Syntax error in EPRT."); return; }
        };
        // Loopback fix (same as PORT)
        let connect_ip = if ip[0] == 127 || ip == [0, 0, 0, 0] {
            self.remote_ip
        } else {
            ip
        };
        self.log_info(&alloc::format!(
            "eprt: connecting to {}.{}.{}.{}:{}",
            connect_ip[0], connect_ip[1], connect_ip[2], connect_ip[3], port
        ));
        self.reply("200", "EPRT command successful.");
        let next = self.read_one_command();
        let (cmd, darg) = split_cmd_owned(&next);
        let data_sock = net::tcp_connect(&connect_ip, port, 5000);
        if data_sock == u32::MAX {
            self.reply("425", "Can't open data connection.");
            return;
        }
        match cmd.as_str() {
            "LIST" | "NLST" => self.do_list(data_sock, &darg),
            "MLSD"          => self.do_mlsd(data_sock, &darg),
            "RETR" => self.do_retr(data_sock, &darg),
            "STOR" | "STOU" => self.do_stor(data_sock, &darg),
            _ => {
                net::tcp_close(data_sock);
                self.reply("503", "Bad sequence of commands.");
            }
        }
    }

    /// Read exactly one complete FTP command line from the control socket.
    fn read_one_command(&mut self) -> String {
        let mut buf = [0u8; CMD_BUF];
        let mut acc: Vec<u8> = Vec::new();
        loop {
            let n = net::tcp_recv(self.ctrl, &mut buf);
            if n == 0 || n == u32::MAX {
                return String::new();
            }
            acc.extend_from_slice(&buf[..n as usize]);
            if let Some(end) = find_crlf(&acc) {
                let line = String::from(
                    core::str::from_utf8(&acc[..end]).unwrap_or("").trim()
                );
                return line;
            }
        }
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn run(ctrl: u32, cfg: &FtpdConfig, remote_ip: [u8; 4]) {
    let mut session = Session::new(ctrl, cfg, remote_ip);
    session.run();
    net::tcp_close(ctrl);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn find_crlf(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(1) {
        if data[i] == b'\r' && data[i + 1] == b'\n' {
            return Some(i);
        }
    }
    // Accept bare \n
    data.iter().position(|&b| b == b'\n')
}

fn split_cmd(line: &str) -> (&str, &str) {
    match line.find(' ') {
        Some(i) => (&line[..i], line[i + 1..].trim()),
        None => (line, ""),
    }
}

fn split_cmd_owned(line: &str) -> (String, String) {
    match line.find(' ') {
        Some(i) => (String::from(&line[..i]), String::from(line[i + 1..].trim())),
        None => (String::from(line), String::new()),
    }
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => { parts.pop(); }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return String::from("/");
    }
    let mut result = String::new();
    for p in &parts {
        result.push('/');
        result.push_str(p);
    }
    result
}

fn parent_dir(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &trimmed[..i],
    }
}

fn parse_port_arg(arg: &str) -> Option<([u8; 4], u16)> {
    let mut nums = [0u32; 6];
    let mut idx = 0;
    let mut cur = 0u32;
    for b in arg.bytes() {
        match b {
            b'0'..=b'9' => { cur = cur * 10 + (b - b'0') as u32; }
            b',' => {
                if idx >= 6 { return None; }
                nums[idx] = cur; idx += 1; cur = 0;
            }
            _ => {}
        }
    }
    if idx == 5 { nums[5] = cur; } else { return None; }
    let ip = [nums[0] as u8, nums[1] as u8, nums[2] as u8, nums[3] as u8];
    let port = (nums[4] as u16) * 256 + nums[5] as u16;
    Some((ip, port))
}

fn format_227(ip: &[u8; 4], p1: u8, p2: u8) -> String {
    let mut s = String::from("227 Entering Passive Mode (");
    push_u8_str(&mut s, ip[0]); s.push(',');
    push_u8_str(&mut s, ip[1]); s.push(',');
    push_u8_str(&mut s, ip[2]); s.push(',');
    push_u8_str(&mut s, ip[3]); s.push(',');
    push_u8_str(&mut s, p1); s.push(',');
    push_u8_str(&mut s, p2);
    s.push_str(").\r\n");
    s
}

fn push_u8_str(s: &mut String, v: u8) {
    let mut buf = [0u8; 4];
    let slice = fmt_u32(v as u32, &mut buf);
    s.push_str(core::str::from_utf8(slice).unwrap_or("0"));
}

fn fmt_u32(mut v: u32, buf: &mut [u8]) -> &[u8] {
    if v == 0 { buf[0] = b'0'; return &buf[..1]; }
    let mut i = buf.len();
    while v > 0 && i > 0 { i -= 1; buf[i] = b'0' + (v % 10) as u8; v /= 10; }
    &buf[i..]
}

/// Convert a Unix timestamp to MDTM format: "YYYYMMDDHHmmSS".
fn format_mdtm_timestamp(ts: u32) -> String {
    if ts == 0 {
        return String::from("19700101000000");
    }
    // Simple UTC breakdown (no leap seconds, assumes Unix epoch)
    let mut secs = ts;
    let sec = secs % 60; secs /= 60;
    let min = secs % 60; secs /= 60;
    let hour = secs % 24; secs /= 24;
    // Days since epoch → year/month/day
    let mut days = secs;
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year { break; }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let months: [u32; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0u32;
    for m in 0..12 {
        if days < months[m] { month = m as u32; break; }
        days -= months[m];
        if m == 11 { month = 11; }
    }
    let day = days + 1;
    month += 1;

    let mut s = String::with_capacity(14);
    push_pad4(&mut s, year);
    push_pad2(&mut s, month);
    push_pad2(&mut s, day);
    push_pad2(&mut s, hour);
    push_pad2(&mut s, min);
    push_pad2(&mut s, sec);
    s
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn push_pad2(s: &mut String, v: u32) {
    if v < 10 { s.push('0'); }
    let mut buf = [0u8; 4];
    let slice = fmt_u32(v, &mut buf);
    s.push_str(core::str::from_utf8(slice).unwrap_or("0"));
}

fn push_pad4(s: &mut String, v: u32) {
    if v < 1000 { s.push('0'); }
    if v < 100 { s.push('0'); }
    if v < 10 { s.push('0'); }
    let mut buf = [0u8; 6];
    let slice = fmt_u32(v, &mut buf);
    s.push_str(core::str::from_utf8(slice).unwrap_or("0"));
}

/// Format a single MLSD entry line (without CRLF).
/// RFC 3659: "type=<type>;size=<n>;modify=<ts>; <name>"
fn format_mlsd_entry(name: &str, entry_type: u32, size: u32, mtime: u32) -> String {
    let mut s = String::from("type=");
    if entry_type == 1 {
        s.push_str("dir");
    } else {
        s.push_str("file");
    }
    s.push_str(";size=");
    let mut buf = [0u8; 12];
    let sz = fmt_u32(size, &mut buf);
    s.push_str(core::str::from_utf8(sz).unwrap_or("0"));
    s.push_str(";modify=");
    let ts = format_mdtm_timestamp(mtime);
    s.push_str(&ts);
    s.push_str("; ");
    s.push_str(name);
    s
}

/// Parse EPRT argument: |<net-prt>|<net-addr>|<tcp-port>|
/// net-prt: 1=IPv4, 2=IPv6 (only IPv4 supported)
fn parse_eprt_arg(arg: &str) -> Option<([u8; 4], u16)> {
    let bytes = arg.as_bytes();
    if bytes.is_empty() { return None; }
    let delim = bytes[0]; // Usually '|'
    // Split by delimiter, skipping first empty part
    let mut parts: [&str; 4] = [""; 4];
    let mut idx = 0;
    let mut start = 1; // skip first delimiter
    for i in 1..arg.len() {
        if bytes[i] == delim {
            if idx < 4 {
                parts[idx] = &arg[start..i];
                idx += 1;
            }
            start = i + 1;
        }
    }
    if idx < 3 { return None; }
    // parts[0] = net-prt ("1"), parts[1] = address, parts[2] = port
    if parts[0] != "1" { return None; } // Only IPv4
    let ip = parse_ip_str(parts[1])?;
    let port = parse_u16(parts[2])?;
    Some((ip, port))
}

fn parse_ip_str(s: &str) -> Option<[u8; 4]> {
    let mut parts = [0u8; 4];
    let mut idx = 0;
    let mut num: u32 = 0;
    let mut has = false;
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => { num = num * 10 + (b - b'0') as u32; has = true; }
            b'.' => {
                if !has || idx >= 3 || num > 255 { return None; }
                parts[idx] = num as u8; idx += 1; num = 0; has = false;
            }
            _ => return None,
        }
    }
    if !has || idx != 3 || num > 255 { return None; }
    parts[3] = num as u8;
    Some(parts)
}

fn parse_u16(s: &str) -> Option<u16> {
    let mut val: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' { return None; }
        val = val * 10 + (b - b'0') as u32;
        if val > 65535 { return None; }
    }
    Some(val as u16)
}
