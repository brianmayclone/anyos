//! vdagent configuration — backed by confd via libconf_schema.
//!
//! Registry namespace: `services/vdagent`. Reachable via:
//!
//!   confctl get  /services/vdagent/config/enabled
//!   confctl set  /services/vdagent/config/port_name "com.redhat.spice.0"
//!   confctl set  /services/vdagent/config/log_level "debug"
//!
//! All values fall back to compiled-in defaults when confd is unavailable
//! (e.g. very early boot, or before the manifest has been registered).

use alloc::string::String;
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};

const VDAGENT_DIRS: &[&str] = &["config"];
const VDAGENT_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_bool("config/enabled", true),
    // Host-side virtio-serial port name. The kernel exposes this as
    // /dev/virtio-ports/<name> and (for well-known names) as /dev/vport-<alias>.
    default_string("config/port_name", "com.redhat.spice.0"),
    // Kernel device path to open. If empty, we derive it from `port_name`.
    default_string("config/device_path", ""),
    // Polling interval in milliseconds for compositor clipboard changes.
    default_int("config/clipboard_poll_ms", 500),
    // Main loop sleep when idle (lower = more responsive, higher = lower CPU).
    // 10 ms keeps mouse input fluid; bump to 50+ when only clipboard sync is
    // needed and CPU matters.
    default_int("config/idle_sleep_ms", 10),
    // Log verbosity: "error", "warn", "info", "debug", "trace".
    default_string("config/log_level", "info"),
    // Maximum clipboard payload accepted from host (bytes). Hard cap.
    default_int("config/max_clipboard_bytes", 65536),
];
const VDAGENT_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const VDAGENT_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/vdagent",
    RegistryScope::System,
    1,
    VDAGENT_DIRS,
    VDAGENT_DEFAULTS,
    VDAGENT_MIGRATIONS,
);
const VDAGENT_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("vdagent", &VDAGENT_MANIFEST);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn from_str(s: &str) -> Self {
        match s {
            "error" | "ERROR" => LogLevel::Error,
            "warn" | "WARN" | "warning" => LogLevel::Warn,
            "debug" | "DEBUG" => LogLevel::Debug,
            "trace" | "TRACE" => LogLevel::Trace,
            _ => LogLevel::Info,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            LogLevel::Error => 0,
            LogLevel::Warn => 1,
            LogLevel::Info => 2,
            LogLevel::Debug => 3,
            LogLevel::Trace => 4,
        }
    }

    pub fn enabled(&self, target: LogLevel) -> bool {
        target.rank() <= self.rank()
    }
}

#[derive(Clone)]
pub struct VdAgentConfig {
    pub enabled: bool,
    pub port_name: String,
    pub device_path: String,
    pub clipboard_poll_ms: u32,
    pub idle_sleep_ms: u32,
    pub log_level: LogLevel,
    pub max_clipboard_bytes: usize,
}

impl VdAgentConfig {
    pub fn defaults() -> Self {
        Self {
            enabled: true,
            port_name: String::from("com.redhat.spice.0"),
            device_path: String::new(),
            clipboard_poll_ms: 500,
            idle_sleep_ms: 10,
            log_level: LogLevel::Info,
            max_clipboard_bytes: 65536,
        }
    }

    /// Effective device path. If `device_path` was set explicitly, use that;
    /// otherwise build `/dev/virtio-ports/<port_name>`. The kernel registers
    /// devices under this canonical path for every PORT_NAME control message.
    pub fn effective_device_path(&self) -> String {
        if !self.device_path.is_empty() {
            return self.device_path.clone();
        }
        let mut s = String::from("/dev/virtio-ports/");
        s.push_str(&self.port_name);
        s
    }

    /// Short well-known alias (`/dev/vport-spice`, etc.) tried as a second
    /// preference before falling back to `/dev/vport0`. Returns None when the
    /// configured `port_name` has no registered alias.
    pub fn alias_device_path(&self) -> Option<String> {
        let alias = match self.port_name.as_str() {
            "com.redhat.spice.0" => "spice",
            "org.spice-space.webdav.0" => "webdav",
            "org.qemu.guest_agent.0" => "qga",
            _ => return None,
        };
        let mut s = String::from("/dev/vport-");
        s.push_str(alias);
        Some(s)
    }
}

/// Register the vdagent schema with confd. Idempotent — safe to call on
/// every start. Returns true on success.
///
/// Failure is logged but non-fatal: we fall back to compiled-in defaults
/// (see `VdAgentConfig::defaults()`), so the daemon still works even when
/// confd is unavailable or denies the registration. The most common reason
/// for a failure is a permission denial (`forbidden`) when the daemon is
/// not running with sufficient privileges to register a System-scope
/// manifest.
pub fn register_manifest() -> bool {
    match VDAGENT_SCHEMA.register() {
        Ok((schema_version, applied_version)) => {
            anyos_std::println!(
                "vdagent[INFO]: confd schema registered (namespace=services/vdagent, schema={}, applied={})",
                schema_version,
                applied_version
            );
            true
        }
        Err(err) => {
            anyos_std::println!(
                "vdagent[WARN]: confd schema registration failed: {:?} — using built-in defaults",
                err
            );
            false
        }
    }
}

pub fn load() -> VdAgentConfig {
    let mut cfg = VdAgentConfig::defaults();
    if let Some(v) = VDAGENT_SCHEMA.read_bool("config/enabled") {
        cfg.enabled = v;
    }
    if let Some(v) = VDAGENT_SCHEMA.read_string("config/port_name") {
        if !v.is_empty() {
            cfg.port_name = v;
        }
    }
    if let Some(v) = VDAGENT_SCHEMA.read_string("config/device_path") {
        cfg.device_path = v;
    }
    if let Some(v) = VDAGENT_SCHEMA.read_i64("config/clipboard_poll_ms") {
        if (50..=60_000).contains(&v) {
            cfg.clipboard_poll_ms = v as u32;
        }
    }
    if let Some(v) = VDAGENT_SCHEMA.read_i64("config/idle_sleep_ms") {
        if (1..=5_000).contains(&v) {
            cfg.idle_sleep_ms = v as u32;
        }
    }
    if let Some(v) = VDAGENT_SCHEMA.read_string("config/log_level") {
        cfg.log_level = LogLevel::from_str(&v);
    }
    if let Some(v) = VDAGENT_SCHEMA.read_i64("config/max_clipboard_bytes") {
        if (256..=4 * 1024 * 1024).contains(&v) {
            cfg.max_clipboard_bytes = v as usize;
        }
    }
    cfg
}
