// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! ftpd configuration loader.
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};

pub const MAX_SHARES: usize = 64;
pub const MAX_PATH_LEN: usize = 256;
pub const MAX_USERNAME_LEN: usize = 32;

const FTPD_DIRS: &[&str] = &["config"];
const FTPD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_bool("config/enabled", true),
    default_int("config/port", 21),
    default_bool("config/passive_mode", true),
    default_int("config/passive_port_min", 50_000),
    default_int("config/passive_port_max", 50_010),
    default_bool("config/allow_anonymous", true),
    default_string("config/anonymous_root", "/users/shared/ftp"),
    default_int("config/max_clients", 10),
    default_bool("config/chroot_users", false),
    default_string("config/masquerade_ip", ""),
    default_string("config/shares_blob", "*:/users/shared/ftp:r\n"),
];
const FTPD_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const FTPD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/ftpd",
    RegistryScope::System,
    1,
    FTPD_DIRS,
    FTPD_DEFAULTS,
    FTPD_MIGRATIONS,
);
const FTPD_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("ftpd", &FTPD_MANIFEST);

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
            passive_mode: true,
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
        core::str::from_utf8(&self.anonymous_root[..self.anonymous_root_len])
            .unwrap_or("/users/shared/ftp")
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
                return if write {
                    share.can_write
                } else {
                    share.can_read
                };
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

// ── Main config loader ────────────────────────────────────────────────────────

pub fn load() -> alloc::boxed::Box<FtpdConfig> {
    let mut cfg = alloc::boxed::Box::new(FtpdConfig::default());
    let _ = load_from_confd(&mut cfg);
    cfg
}

pub fn register_manifest() {
    let _ = FTPD_SCHEMA.register();
}

fn load_from_confd(cfg: &mut FtpdConfig) -> bool {
    let mut loaded = false;

    if let Some(v) = FTPD_SCHEMA.read_bool("config/enabled") {
        cfg.enabled = v;
        loaded = true;
    }
    if let Some(v) = FTPD_SCHEMA.read_i64("config/port") {
        if v > 0 && v <= u16::MAX as i64 {
            cfg.port = v as u16;
            loaded = true;
        }
    }
    if let Some(v) = FTPD_SCHEMA.read_bool("config/passive_mode") {
        cfg.passive_mode = v;
        loaded = true;
    }
    if let Some(v) = FTPD_SCHEMA.read_i64("config/passive_port_min") {
        if v >= 0 && v <= u16::MAX as i64 {
            cfg.passive_port_min = v as u16;
            loaded = true;
        }
    }
    if let Some(v) = FTPD_SCHEMA.read_i64("config/passive_port_max") {
        if v >= 0 && v <= u16::MAX as i64 {
            cfg.passive_port_max = v as u16;
            loaded = true;
        }
    }
    if let Some(v) = FTPD_SCHEMA.read_bool("config/allow_anonymous") {
        cfg.allow_anonymous = v;
        loaded = true;
    }
    if let Some(v) = FTPD_SCHEMA.read_string("config/anonymous_root") {
        assign_path(&v, &mut cfg.anonymous_root, &mut cfg.anonymous_root_len);
        loaded = true;
    }
    if let Some(v) = FTPD_SCHEMA.read_i64("config/max_clients") {
        if v > 0 && v <= u16::MAX as i64 {
            cfg.max_clients = v as u16;
            loaded = true;
        }
    }
    if let Some(v) = FTPD_SCHEMA.read_bool("config/chroot_users") {
        cfg.chroot_users = v;
        loaded = true;
    }
    if let Some(v) = FTPD_SCHEMA.read_string("config/masquerade_ip") {
        if parse_masquerade_ip(v.as_bytes(), &mut cfg.masquerade_ip) {
            loaded = true;
        }
    }
    if let Some(v) = FTPD_SCHEMA.read_string("config/shares_blob") {
        parse_shares_blob(cfg, &v);
        loaded = true;
    }

    loaded
}

fn assign_path(value: &str, out: &mut [u8; MAX_PATH_LEN], len_out: &mut usize) {
    assign_path_bytes(value.as_bytes(), out, len_out);
}

fn assign_path_bytes(value: &[u8], out: &mut [u8; MAX_PATH_LEN], len_out: &mut usize) {
    let n = value.len().min(MAX_PATH_LEN);
    out[..n].copy_from_slice(&value[..n]);
    *len_out = n;
}

fn parse_masquerade_ip(val: &[u8], out: &mut [u8; 4]) -> bool {
    let mut octets = [0u8; 4];
    let mut idx = 0usize;
    let mut cur: u32 = 0;
    let mut any = false;
    for &b in val {
        if b.is_ascii_digit() {
            cur = cur * 10 + (b - b'0') as u32;
            any = true;
        } else if b == b'.' && idx < 3 && any {
            octets[idx] = cur.min(255) as u8;
            idx += 1;
            cur = 0;
            any = false;
        } else {
            return false;
        }
    }
    if any && idx == 3 {
        octets[3] = cur.min(255) as u8;
        *out = octets;
        true
    } else {
        false
    }
}

fn parse_shares_blob(cfg: &mut FtpdConfig, blob: &str) {
    cfg.shares_count = 0;
    for line in blob.split('\n') {
        if cfg.shares_count >= MAX_SHARES {
            break;
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, ':');
        let user = parts.next().unwrap_or("").trim();
        let path = parts.next().unwrap_or("").trim();
        let perms = parts.next().unwrap_or("").trim();
        if user.is_empty() || path.is_empty() {
            continue;
        }

        let mut share = Share::empty();
        let user_bytes = user.as_bytes();
        let user_len = user_bytes.len().min(MAX_USERNAME_LEN);
        share.user[..user_len].copy_from_slice(&user_bytes[..user_len]);
        share.user_len = user_len;
        let path_bytes = path.as_bytes();
        let path_len = path_bytes.len().min(MAX_PATH_LEN);
        share.path[..path_len].copy_from_slice(&path_bytes[..path_len]);
        share.path_len = path_len;
        share.can_read = perms.contains('r') || perms.contains('R');
        share.can_write = perms.contains('w') || perms.contains('W');
        cfg.shares[cfg.shares_count] = share;
        cfg.shares_count += 1;
    }
}
