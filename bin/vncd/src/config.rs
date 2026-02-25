//! VNC daemon configuration loader and writer.
//!
//! Configuration file: `/System/etc/vncd.conf` (key=value, one per line).
//!
//! Supported keys:
//! - `enabled`       — `yes` / `no` (default: no)
//! - `port`          — TCP port 1–65535 (default: 5900)
//! - `allow_root`    — `yes` / `no` (default: no)
//! - `allowed_users` — comma-separated usernames, empty = none (default: empty)
//! - `password`      — up to 8 ASCII chars used as the DES key (default: "anyos")

use anyos_std::fs;

/// Path to the VNC daemon configuration file.
pub const CONF_PATH: &str = "/System/etc/vncd.conf";

/// Maximum number of allowed users we track.
pub const MAX_ALLOWED_USERS: usize = 32;

/// Maximum length of a username.
pub const MAX_USERNAME_LEN: usize = 32;

/// VNC daemon runtime configuration.
#[derive(Clone)]
pub struct VncConfig {
    /// Whether the VNC server accepts incoming connections.
    pub enabled: bool,
    /// TCP port to listen on (default 5900).
    pub port: u16,
    /// Whether the root account may authenticate via VNC.
    pub allow_root: bool,
    /// Usernames permitted to authenticate (must also exist locally).
    /// An empty list means no ordinary user may connect (only root, if allowed).
    pub allowed_users: [[u8; MAX_USERNAME_LEN]; MAX_ALLOWED_USERS],
    /// Number of valid entries in `allowed_users`.
    pub allowed_users_count: usize,
    /// 8-byte DES key derived from the configured password (padded with zeros).
    pub password: [u8; 8],
}

impl VncConfig {
    /// Return the default configuration (VNC disabled, port 5900, no users).
    pub fn default() -> Self {
        let mut cfg = VncConfig {
            enabled: false,
            port: 5900,
            allow_root: false,
            allowed_users: [[0u8; MAX_USERNAME_LEN]; MAX_ALLOWED_USERS],
            allowed_users_count: 0,
            password: [0u8; 8],
        };
        // Default password: "anyos"
        cfg.set_password(b"anyos");
        cfg
    }

    /// Store a password string (up to 8 bytes) in the DES key field.
    pub fn set_password(&mut self, pw: &[u8]) {
        self.password = [0u8; 8];
        let n = pw.len().min(8);
        self.password[..n].copy_from_slice(&pw[..n]);
    }

    /// Return whether `username` is in the allowed-users list.
    pub fn is_user_allowed(&self, username: &[u8]) -> bool {
        for i in 0..self.allowed_users_count {
            let stored = &self.allowed_users[i];
            // Find the stored name length (null-terminated).
            let len = stored.iter().position(|&b| b == 0).unwrap_or(MAX_USERNAME_LEN);
            if len == username.len() && &stored[..len] == username {
                return true;
            }
        }
        false
    }
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

/// Parse a decimal string slice into a `u16`.
fn parse_u16(s: &[u8]) -> Option<u16> {
    let mut val: u32 = 0;
    let mut any = false;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as u32;
            if val > 65535 {
                return None;
            }
            any = true;
        } else {
            break;
        }
    }
    if any { Some(val as u16) } else { None }
}

/// Return `true` if the byte slice equals `yes` (case-insensitive ASCII).
fn is_yes(s: &[u8]) -> bool {
    matches!(s, b"yes" | b"Yes" | b"YES" | b"1" | b"true" | b"True" | b"TRUE")
}

/// Trim leading and trailing ASCII whitespace from a byte slice.
fn trim(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|&b| b > b' ').unwrap_or(s.len());
    let end = s.iter().rposition(|&b| b > b' ').map(|i| i + 1).unwrap_or(start);
    &s[start..end]
}

// ── File I/O via anyos_std::fs ────────────────────────────────────────────────

/// Load the VNC configuration from [`CONF_PATH`].
///
/// On any read error or missing file, returns [`VncConfig::default()`].
pub fn load() -> VncConfig {
    let mut cfg = VncConfig::default();

    // Read entire file into a stack buffer (config file is tiny).
    let mut buf = [0u8; 2048];
    // flags=0 → read-only (no O_WRITE flag set)
    let fd = fs::open(CONF_PATH, 0);
    if fd == u32::MAX {
        return cfg;
    }
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);

    if n == 0 || n > buf.len() {
        return cfg;
    }

    // Parse line by line.
    let mut pos = 0;
    while pos < n {
        // Find end of line.
        let line_end = buf[pos..n]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| pos + i)
            .unwrap_or(n);

        let line = trim(&buf[pos..line_end]);
        pos = line_end + 1;

        // Skip comments and empty lines.
        if line.is_empty() || line[0] == b'#' {
            continue;
        }

        // Split at '='.
        let eq = match line.iter().position(|&b| b == b'=') {
            Some(i) => i,
            None => continue,
        };
        let key = trim(&line[..eq]);
        let val = trim(&line[eq + 1..]);

        match key {
            b"enabled" => cfg.enabled = is_yes(val),
            b"port" => {
                if let Some(p) = parse_u16(val) {
                    if p > 0 {
                        cfg.port = p;
                    }
                }
            }
            b"allow_root" => cfg.allow_root = is_yes(val),
            b"password" => {
                let n = val.len().min(8);
                cfg.set_password(&val[..n]);
            }
            b"allowed_users" => {
                // Comma-separated list of usernames.
                let mut idx = 0usize;
                let mut seg_start = 0usize;
                while idx <= val.len() && cfg.allowed_users_count < MAX_ALLOWED_USERS {
                    let at_sep = idx == val.len() || val[idx] == b',';
                    if at_sep {
                        let user = trim(&val[seg_start..idx]);
                        if !user.is_empty() {
                            let ulen = user.len().min(MAX_USERNAME_LEN);
                            let slot = cfg.allowed_users_count;
                            cfg.allowed_users[slot][..ulen].copy_from_slice(&user[..ulen]);
                            cfg.allowed_users_count += 1;
                        }
                        seg_start = idx + 1;
                    }
                    idx += 1;
                }
            }
            _ => {} // Unknown key — ignore.
        }
    }

    cfg
}

/// Serialize `cfg` back to [`CONF_PATH`].
///
/// Overwrites the file. Returns `true` on success.
pub fn save(cfg: &VncConfig) -> bool {
    let mut buf = [0u8; 2048];
    let mut pos = 0;

    /// Append a byte slice to `buf`, advancing `pos`.
    macro_rules! append {
        ($s:expr) => {{
            let s: &[u8] = $s;
            let n = s.len().min(buf.len() - pos);
            buf[pos..pos + n].copy_from_slice(&s[..n]);
            pos += n;
        }};
    }

    append!(b"# anyOS VNC Server Configuration\n");
    append!(if cfg.enabled { b"enabled=yes\n" } else { b"enabled=no\n" });

    // Write port as decimal.
    let mut port_buf = [0u8; 6];
    let port_str = fmt_u16(cfg.port, &mut port_buf);
    append!(b"port=");
    append!(port_str);
    append!(b"\n");

    append!(if cfg.allow_root {
        b"allow_root=yes\n"
    } else {
        b"allow_root=no\n"
    });

    // Write allowed_users as comma-separated.
    append!(b"allowed_users=");
    for i in 0..cfg.allowed_users_count {
        if i > 0 {
            append!(b",");
        }
        let stored = &cfg.allowed_users[i];
        let len = stored.iter().position(|&b| b == 0).unwrap_or(MAX_USERNAME_LEN);
        append!(&stored[..len]);
    }
    append!(b"\n");

    // Write password (printable ASCII portion only).
    append!(b"password=");
    let pw_len = cfg.password.iter().position(|&b| b == 0).unwrap_or(8);
    append!(&cfg.password[..pw_len]);
    append!(b"\n");

    // Write to file.
    let fd = fs::open(CONF_PATH, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd == u32::MAX {
        return false;
    }
    let written = fs::write(fd, &buf[..pos]);
    fs::close(fd);
    written as usize == pos
}

/// Format a `u16` into `buf` and return the used slice.
fn fmt_u16(mut v: u16, buf: &mut [u8; 6]) -> &[u8] {
    if v == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut i = 6usize;
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    &buf[i..]
}
