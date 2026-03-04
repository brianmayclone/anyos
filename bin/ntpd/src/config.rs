//! ntpd configuration loader.
//!
//! Config path: `/System/etc/ntp/ntpd.conf`

use anyos_std::fs;

pub const CONF_PATH: &str = "/System/etc/ntp/ntpd.conf";

pub const MAX_SERVERS: usize = 8;
pub const MAX_HOSTNAME_LEN: usize = 128;

/// A single NTP server entry.
#[derive(Clone, Copy)]
pub struct NtpServer {
    pub hostname: [u8; MAX_HOSTNAME_LEN],
    pub hostname_len: usize,
    pub ip: [u8; 4],
    pub resolved: bool,
}

impl NtpServer {
    pub const fn empty() -> Self {
        NtpServer {
            hostname: [0; MAX_HOSTNAME_LEN],
            hostname_len: 0,
            ip: [0; 4],
            resolved: false,
        }
    }

    pub fn hostname_str(&self) -> &str {
        core::str::from_utf8(&self.hostname[..self.hostname_len]).unwrap_or("")
    }
}

#[derive(Clone)]
pub struct NtpdConfig {
    pub enabled: bool,
    /// Poll interval in seconds.
    pub poll_interval: u32,
    /// NTP server entries.
    pub servers: [NtpServer; MAX_SERVERS],
    pub server_count: usize,
}

impl NtpdConfig {
    pub fn default() -> Self {
        let mut cfg = NtpdConfig {
            enabled: true,
            poll_interval: 64,
            servers: [NtpServer::empty(); MAX_SERVERS],
            server_count: 0,
        };
        // Default server: pool.ntp.org
        let host = b"pool.ntp.org";
        cfg.servers[0].hostname[..host.len()].copy_from_slice(host);
        cfg.servers[0].hostname_len = host.len();
        cfg.server_count = 1;
        cfg
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

fn parse_u32(s: &[u8]) -> Option<u32> {
    let mut val: u64 = 0;
    let mut any = false;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as u64;
            if val > u32::MAX as u64 { return None; }
            any = true;
        } else {
            break;
        }
    }
    if any { Some(val as u32) } else { None }
}

// ── Config loader ─────────────────────────────────────────────────────────────

pub fn load() -> NtpdConfig {
    let mut cfg = NtpdConfig::default();
    let mut buf = [0u8; 4096];
    let fd = fs::open(CONF_PATH, 0);
    if fd == u32::MAX { return cfg; }
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);
    if n == 0 || n > buf.len() { return cfg; }

    // Reset default servers — we only use them if no "server=" lines appear.
    let mut found_server = false;

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
            b"poll_interval" => {
                if let Some(v) = parse_u32(val) {
                    if v >= 16 { cfg.poll_interval = v; }
                }
            }
            b"server" => {
                if !found_server {
                    // Clear defaults on first explicit server line.
                    cfg.server_count = 0;
                    found_server = true;
                }
                if cfg.server_count < MAX_SERVERS && !val.is_empty() {
                    let len = val.len().min(MAX_HOSTNAME_LEN);
                    let mut srv = NtpServer::empty();
                    srv.hostname[..len].copy_from_slice(&val[..len]);
                    srv.hostname_len = len;
                    cfg.servers[cfg.server_count] = srv;
                    cfg.server_count += 1;
                }
            }
            _ => {}
        }
    }

    cfg
}
