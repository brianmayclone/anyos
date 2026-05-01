//! VNC daemon configuration loader and writer.
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};

/// Maximum number of allowed users we track.
pub const MAX_ALLOWED_USERS: usize = 32;

/// Maximum length of a username.
pub const MAX_USERNAME_LEN: usize = 32;

const VNCD_DIRS: &[&str] = &["config"];
const VNCD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_bool("config/enabled", false),
    default_int("config/port", 5900),
    default_bool("config/allow_root", false),
    default_string("config/password", "anyos"),
    default_string("config/allowed_users_csv", ""),
];
const VNCD_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const VNCD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/vncd",
    RegistryScope::System,
    1,
    VNCD_DIRS,
    VNCD_DEFAULTS,
    VNCD_MIGRATIONS,
);
const VNCD_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("vncd", &VNCD_MANIFEST);

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
            let len = stored
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(MAX_USERNAME_LEN);
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
    if any {
        Some(val as u16)
    } else {
        None
    }
}

/// Return `true` if the byte slice equals `yes` (case-insensitive ASCII).
fn is_yes(s: &[u8]) -> bool {
    matches!(
        s,
        b"yes" | b"Yes" | b"YES" | b"1" | b"true" | b"True" | b"TRUE"
    )
}

/// Trim leading and trailing ASCII whitespace from a byte slice.
fn trim(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|&b| b > b' ').unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|&b| b > b' ')
        .map(|i| i + 1)
        .unwrap_or(start);
    &s[start..end]
}

pub fn load() -> VncConfig {
    let mut cfg = VncConfig::default();
    let _ = load_from_confd(&mut cfg);
    cfg
}

pub fn register_manifest() {
    let _ = VNCD_SCHEMA.register();
}

fn load_from_confd(cfg: &mut VncConfig) -> bool {
    let mut loaded = false;
    if let Some(v) = VNCD_SCHEMA.read_bool("config/enabled") {
        cfg.enabled = v;
        loaded = true;
    }
    if let Some(v) = VNCD_SCHEMA.read_i64("config/port") {
        if v > 0 && v <= u16::MAX as i64 {
            cfg.port = v as u16;
            loaded = true;
        }
    }
    if let Some(v) = VNCD_SCHEMA.read_bool("config/allow_root") {
        cfg.allow_root = v;
        loaded = true;
    }
    if let Some(v) = VNCD_SCHEMA.read_string("config/password") {
        cfg.set_password(v.as_bytes());
        loaded = true;
    }
    if let Some(v) = VNCD_SCHEMA.read_string("config/allowed_users_csv") {
        parse_allowed_users(cfg, v.as_bytes());
        loaded = true;
    }
    loaded
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

fn parse_allowed_users(cfg: &mut VncConfig, raw: &[u8]) {
    cfg.allowed_users_count = 0;
    let mut idx = 0usize;
    let mut seg_start = 0usize;
    while idx <= raw.len() && cfg.allowed_users_count < MAX_ALLOWED_USERS {
        let at_sep = idx == raw.len() || raw[idx] == b',';
        if at_sep {
            let user = trim(&raw[seg_start..idx]);
            if !user.is_empty() {
                let ulen = user.len().min(MAX_USERNAME_LEN);
                let slot = cfg.allowed_users_count;
                cfg.allowed_users[slot] = [0u8; MAX_USERNAME_LEN];
                cfg.allowed_users[slot][..ulen].copy_from_slice(&user[..ulen]);
                cfg.allowed_users_count += 1;
            }
            seg_start = idx + 1;
        }
        idx += 1;
    }
}
