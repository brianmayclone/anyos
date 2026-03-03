//! ftpd configuration loader.
//!
//! Main config:  `/System/etc/ftpd/ftpd.conf`
//! Share config: `/System/etc/ftpd/shares.conf`

use anyos_std::fs;

pub const CONF_PATH: &str = "/System/etc/ftpd/ftpd.conf";
pub const SHARES_PATH: &str = "/System/etc/ftpd/shares.conf";

pub const MAX_SHARES: usize = 64;
pub const MAX_PATH_LEN: usize = 256;
pub const MAX_USERNAME_LEN: usize = 32;

/// A single share entry: user, path, read/write permissions.
#[derive(Clone, Copy)]
pub struct Share {
    /// Username — "*" means all users.
    pub user: [u8; MAX_USERNAME_LEN],
    pub user_len: usize,
    /// Absolute path of the shared directory.
    pub path: [u8; MAX_PATH_LEN],
    pub path_len: usize,
    pub can_read: bool,
    pub can_write: bool,
}

impl Share {
    pub const fn empty() -> Self {
        Share {
            user: [0; MAX_USERNAME_LEN],
            user_len: 0,
            path: [0; MAX_PATH_LEN],
            path_len: 0,
            can_read: false,
            can_write: false,
        }
    }

    pub fn user_str(&self) -> &str {
        core::str::from_utf8(&self.user[..self.user_len]).unwrap_or("")
    }

    pub fn path_str(&self) -> &str {
        core::str::from_utf8(&self.path[..self.path_len]).unwrap_or("")
    }

    pub fn matches_user(&self, username: &str) -> bool {
        self.user_str() == "*" || self.user_str() == username
    }
}

#[derive(Clone)]
pub struct FtpdConfig {
    pub enabled: bool,
    pub port: u16,
    /// true = PASV (passive), false = PORT (active)
    pub passive_mode: bool,
    pub passive_port_min: u16,
    pub passive_port_max: u16,
    pub allow_anonymous: bool,
    /// Root directory for anonymous users.
    pub anonymous_root: [u8; MAX_PATH_LEN],
    pub anonymous_root_len: usize,
    pub max_clients: u16,
    /// Restrict users to their share paths (chroot-like).
    pub chroot_users: bool,
    pub shares: [Share; MAX_SHARES],
    pub shares_count: usize,
    /// Override IP in 227 PASV reply (NAT/port-forwarding scenarios).
    /// [0,0,0,0] means "use actual NIC IP".
    pub masquerade_ip: [u8; 4],
}

impl FtpdConfig {
    pub fn default() -> Self {
        let mut cfg = FtpdConfig {
            enabled: true,
            port: 21,
            passive_mode: false,
            passive_port_min: 50000,
            passive_port_max: 50010,
            allow_anonymous: true,
            anonymous_root: [0; MAX_PATH_LEN],
            anonymous_root_len: 0,
            max_clients: 10,
            chroot_users: false,
            shares: [Share::empty(); MAX_SHARES],
            shares_count: 0,
            masquerade_ip: [0; 4],
        };
        let root = b"/users/shared/ftp";
        cfg.anonymous_root_len = root.len();
        cfg.anonymous_root[..root.len()].copy_from_slice(root);
        cfg
    }

    pub fn anonymous_root_str(&self) -> &str {
        core::str::from_utf8(&self.anonymous_root[..self.anonymous_root_len]).unwrap_or("/users/shared/ftp")
    }

    /// Find the first share that matches username and contains the given path.
    /// Returns (can_read, can_write) or (false, false) if no match.
    pub fn check_access(&self, username: &str, path: &str, write: bool) -> bool {
        for i in 0..self.shares_count {
            let share = &self.shares[i];
            if !share.matches_user(username) {
                continue;
            }
            // Check path is within share root (with proper boundary check).
            let share_root = share.path_str();
            let under = if share_root == "/" {
                true
            } else if path == share_root {
                true
            } else {
                path.starts_with(share_root) && path.as_bytes().get(share_root.len()) == Some(&b'/')
            };
            if under {
                return if write { share.can_write } else { share.can_read };
            }
        }
        false
    }

    /// Get the first readable root for a user (for their initial directory).
    /// Prefers an exact-user match over wildcard (*) matches.
    pub fn user_root<'a>(&'a self, username: &str) -> &'a str {
        // First: prefer a share explicitly for this user
        for i in 0..self.shares_count {
            let share = &self.shares[i];
            if share.user_str() == username && share.can_read {
                return share.path_str();
            }
        }
        // Second: wildcard share
        for i in 0..self.shares_count {
            let share = &self.shares[i];
            if share.user_str() == "*" && share.can_read {
                return share.path_str();
            }
        }
        "/"
    }
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

fn trim(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|&b| b > b' ').unwrap_or(s.len());
    let end = s.iter().rposition(|&b| b > b' ').map(|i| i + 1).unwrap_or(start);
    &s[start..end]
}

fn is_yes(s: &[u8]) -> bool {
    matches!(s, b"yes" | b"Yes" | b"YES" | b"1" | b"true" | b"True" | b"TRUE")
}

fn parse_u16(s: &[u8]) -> Option<u16> {
    let mut val: u32 = 0;
    let mut any = false;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as u32;
            if val > 65535 { return None; }
            any = true;
        } else {
            break;
        }
    }
    if any { Some(val as u16) } else { None }
}

// ── Main config loader ────────────────────────────────────────────────────────

pub fn load() -> alloc::boxed::Box<FtpdConfig> {
    let mut cfg = alloc::boxed::Box::new(FtpdConfig::default());
    load_main_conf(&mut cfg);
    load_shares_conf(&mut cfg);
    cfg
}

fn load_main_conf(cfg: &mut FtpdConfig) {
    let mut buf = [0u8; 4096];
    let fd = fs::open(CONF_PATH, 0);
    if fd == u32::MAX { return; }
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);
    if n == 0 || n > buf.len() { return; }

    let mut pos = 0;
    while pos < n {
        let line_end = buf[pos..n].iter().position(|&b| b == b'\n')
            .map(|i| pos + i).unwrap_or(n);
        let line = trim(&buf[pos..line_end]);
        pos = line_end + 1;
        if line.is_empty() || line[0] == b'#' { continue; }
        let eq = match line.iter().position(|&b| b == b'=') {
            Some(i) => i, None => continue,
        };
        let key = trim(&line[..eq]);
        let val = trim(&line[eq + 1..]);

        match key {
            b"enabled" => cfg.enabled = is_yes(val),
            b"port" => { if let Some(p) = parse_u16(val) { if p > 0 { cfg.port = p; } } }
            b"passive_mode" => cfg.passive_mode = is_yes(val),
            b"passive_port_min" => { if let Some(p) = parse_u16(val) { cfg.passive_port_min = p; } }
            b"passive_port_max" => { if let Some(p) = parse_u16(val) { cfg.passive_port_max = p; } }
            b"allow_anonymous" => cfg.allow_anonymous = is_yes(val),
            b"anonymous_root" => {
                let n = val.len().min(MAX_PATH_LEN);
                cfg.anonymous_root[..n].copy_from_slice(&val[..n]);
                cfg.anonymous_root_len = n;
            }
            b"max_clients" => { if let Some(v) = parse_u16(val) { if v > 0 { cfg.max_clients = v; } } }
            b"chroot_users" => cfg.chroot_users = is_yes(val),
            b"masquerade_ip" => {
                // Parse a.b.c.d into four bytes
                let mut octets = [0u8; 4];
                let mut idx = 0usize;
                let mut cur: u32 = 0;
                let mut any = false;
                for &b in val {
                    if b >= b'0' && b <= b'9' {
                        cur = cur * 10 + (b - b'0') as u32;
                        any = true;
                    } else if b == b'.' && idx < 3 && any {
                        octets[idx] = cur.min(255) as u8;
                        idx += 1;
                        cur = 0;
                        any = false;
                    } else {
                        break;
                    }
                }
                if any && idx == 3 {
                    octets[3] = cur.min(255) as u8;
                    cfg.masquerade_ip = octets;
                }
            }
            _ => {}
        }
    }
}

fn load_shares_conf(cfg: &mut FtpdConfig) {
    let mut buf = [0u8; 8192];
    let fd = fs::open(SHARES_PATH, 0);
    if fd == u32::MAX { return; }
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);
    if n == 0 || n > buf.len() { return; }

    let mut pos = 0;
    while pos < n && cfg.shares_count < MAX_SHARES {
        let line_end = buf[pos..n].iter().position(|&b| b == b'\n')
            .map(|i| pos + i).unwrap_or(n);
        let line = trim(&buf[pos..line_end]);
        pos = line_end + 1;
        if line.is_empty() || line[0] == b'#' { continue; }

        // Format: user:path:perms
        // Find first ':'
        let colon1 = match line.iter().position(|&b| b == b':') {
            Some(i) => i, None => continue,
        };
        let user = trim(&line[..colon1]);
        let rest = &line[colon1 + 1..];
        // Find second ':'
        let colon2 = match rest.iter().position(|&b| b == b':') {
            Some(i) => i, None => continue,
        };
        let path = trim(&rest[..colon2]);
        let perms = trim(&rest[colon2 + 1..]);

        if user.is_empty() || path.is_empty() { continue; }

        let mut share = Share::empty();
        let ulen = user.len().min(MAX_USERNAME_LEN);
        share.user[..ulen].copy_from_slice(&user[..ulen]);
        share.user_len = ulen;

        let plen = path.len().min(MAX_PATH_LEN);
        share.path[..plen].copy_from_slice(&path[..plen]);
        share.path_len = plen;

        for &b in perms {
            match b {
                b'r' | b'R' => share.can_read = true,
                b'w' | b'W' => share.can_write = true,
                _ => {}
            }
        }

        cfg.shares[cfg.shares_count] = share;
        cfg.shares_count += 1;
    }
}

// ── Config writer ─────────────────────────────────────────────────────────────

pub fn save(cfg: &FtpdConfig) -> bool {
    save_main_conf(cfg) && save_shares_conf(cfg)
}

fn fmt_u16(mut v: u16, buf: &mut [u8; 6]) -> &[u8] {
    if v == 0 { buf[0] = b'0'; return &buf[..1]; }
    let mut i = 6usize;
    while v > 0 { i -= 1; buf[i] = b'0' + (v % 10) as u8; v /= 10; }
    &buf[i..]
}

fn save_main_conf(cfg: &FtpdConfig) -> bool {
    let mut out = [0u8; 4096];
    let mut pos = 0;

    macro_rules! append {
        ($s:expr) => {{
            let s: &[u8] = $s;
            let n = s.len().min(out.len() - pos);
            out[pos..pos + n].copy_from_slice(&s[..n]);
            pos += n;
        }};
    }

    append!(b"# anyOS FTP Server Configuration\n");
    append!(if cfg.enabled { b"enabled=yes\n" } else { b"enabled=no\n" });
    let mut port_buf = [0u8; 6];
    append!(b"port=");
    append!(fmt_u16(cfg.port, &mut port_buf));
    append!(b"\n");
    append!(if cfg.passive_mode { b"passive_mode=yes\n" } else { b"passive_mode=no\n" });
    let mut pb1 = [0u8; 6]; let mut pb2 = [0u8; 6];
    append!(b"passive_port_min=");
    append!(fmt_u16(cfg.passive_port_min, &mut pb1));
    append!(b"\n");
    append!(b"passive_port_max=");
    append!(fmt_u16(cfg.passive_port_max, &mut pb2));
    append!(b"\n");
    append!(if cfg.allow_anonymous { b"allow_anonymous=yes\n" } else { b"allow_anonymous=no\n" });
    append!(b"anonymous_root=");
    append!(&cfg.anonymous_root[..cfg.anonymous_root_len]);
    append!(b"\n");
    let mut mc = [0u8; 6];
    append!(b"max_clients=");
    append!(fmt_u16(cfg.max_clients, &mut mc));
    append!(b"\n");
    append!(if cfg.chroot_users { b"chroot_users=yes\n" } else { b"chroot_users=no\n" });
    // masquerade_ip: write as a.b.c.d (0.0.0.0 = disabled)
    append!(b"masquerade_ip=");
    let ip = cfg.masquerade_ip;
    let mut num_buf = [0u8; 6];
    append!(fmt_u16(ip[0] as u16, &mut num_buf));
    append!(b".");
    append!(fmt_u16(ip[1] as u16, &mut num_buf));
    append!(b".");
    append!(fmt_u16(ip[2] as u16, &mut num_buf));
    append!(b".");
    append!(fmt_u16(ip[3] as u16, &mut num_buf));
    append!(b"\n");

    let fd = fs::open(CONF_PATH, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX { return false; }
    let w = fs::write(fd, &out[..pos]);
    fs::close(fd);
    w as usize == pos
}

fn save_shares_conf(cfg: &FtpdConfig) -> bool {
    let mut out = [0u8; 8192];
    let mut pos = 0;

    macro_rules! append {
        ($s:expr) => {{
            let s: &[u8] = $s;
            let n = s.len().min(out.len() - pos);
            out[pos..pos + n].copy_from_slice(&s[..n]);
            pos += n;
        }};
    }

    append!(b"# anyOS FTP Shares Configuration\n");
    append!(b"# Format: username:path:permissions  (r=read, w=write, *=all users)\n");
    for i in 0..cfg.shares_count {
        let share = &cfg.shares[i];
        append!(&share.user[..share.user_len]);
        append!(b":");
        append!(&share.path[..share.path_len]);
        append!(b":");
        if share.can_read { append!(b"r"); }
        if share.can_write { append!(b"w"); }
        append!(b"\n");
    }

    let fd = fs::open(SHARES_PATH, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX { return false; }
    let w = fs::write(fd, &out[..pos]);
    fs::close(fd);
    w as usize == pos
}
