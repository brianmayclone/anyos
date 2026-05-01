//! ntpd configuration loader.
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};

pub const MAX_SERVERS: usize = 8;
pub const MAX_HOSTNAME_LEN: usize = 128;

const NTPD_DIRS: &[&str] = &["config"];
const NTPD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_bool("config/enabled", true),
    default_int("config/poll_interval", 64),
    default_string("config/servers_csv", "pool.ntp.org"),
];
const NTPD_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const NTPD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/ntpd",
    RegistryScope::System,
    1,
    NTPD_DIRS,
    NTPD_DEFAULTS,
    NTPD_MIGRATIONS,
);
const NTPD_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("ntpd", &NTPD_MANIFEST);

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
    let end = s
        .iter()
        .rposition(|&b| b > b' ')
        .map(|i| i + 1)
        .unwrap_or(start);
    &s[start..end]
}

fn is_yes(s: &[u8]) -> bool {
    matches!(
        s,
        b"yes" | b"Yes" | b"YES" | b"1" | b"true" | b"True" | b"TRUE"
    )
}

fn parse_u32(s: &[u8]) -> Option<u32> {
    let mut val: u64 = 0;
    let mut any = false;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as u64;
            if val > u32::MAX as u64 {
                return None;
            }
            any = true;
        } else {
            break;
        }
    }
    if any {
        Some(val as u32)
    } else {
        None
    }
}

// ── Config loader ─────────────────────────────────────────────────────────────

pub fn load() -> NtpdConfig {
    let mut cfg = NtpdConfig::default();
    let _ = load_from_confd(&mut cfg);
    cfg
}

pub fn register_manifest() {
    let _ = NTPD_SCHEMA.register();
}

fn load_from_confd(cfg: &mut NtpdConfig) -> bool {
    let mut loaded = false;
    if let Some(v) = NTPD_SCHEMA.read_bool("config/enabled") {
        cfg.enabled = v;
        loaded = true;
    }
    if let Some(v) = NTPD_SCHEMA.read_i64("config/poll_interval") {
        if v >= 16 && v <= u32::MAX as i64 {
            cfg.poll_interval = v as u32;
            loaded = true;
        }
    }
    if let Some(v) = NTPD_SCHEMA.read_string("config/servers_csv") {
        parse_servers_csv(cfg, &v);
        loaded = true;
    }
    loaded
}

fn parse_servers_csv(cfg: &mut NtpdConfig, csv: &str) {
    cfg.server_count = 0;
    for part in csv.split(',') {
        if cfg.server_count >= MAX_SERVERS {
            break;
        }
        let host = part.trim();
        if host.is_empty() {
            continue;
        }
        let len = host.len().min(MAX_HOSTNAME_LEN);
        let mut srv = NtpServer::empty();
        srv.hostname[..len].copy_from_slice(&host.as_bytes()[..len]);
        srv.hostname_len = len;
        cfg.servers[cfg.server_count] = srv;
        cfg.server_count += 1;
    }
    if cfg.server_count == 0 {
        *cfg = NtpdConfig::default();
    }
}
