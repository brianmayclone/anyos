use alloc::string::String;
use alloc::vec::Vec;
use libconf_schema::{default_int, default_string, manifest, RegistryScope, ServiceSchema};

const SEARCHD_DIRS: &[&str] = &["config"];
const SEARCHD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/idle_timeout_ms", 10_000),
    default_int("config/max_entries", 1_000_000),
    default_string("config/excludes_csv", ""),
];
const SEARCHD_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const SEARCHD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/searchd",
    RegistryScope::System,
    1,
    SEARCHD_DIRS,
    SEARCHD_DEFAULTS,
    SEARCHD_MIGRATIONS,
);
const SEARCHD_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("searchd", &SEARCHD_MANIFEST);

/// Parsed configuration.
pub struct Config {
    /// Milliseconds of idle time before starting an index pass.
    pub idle_timeout_ms: u32,
    /// Maximum number of entries in the files table.
    pub max_entries: u32,
    /// Folders to exclude from indexing.
    pub excludes: Vec<String>,
}

impl Config {
    /// Default configuration used when no config file exists.
    pub fn defaults() -> Config {
        Config {
            idle_timeout_ms: 10000,
            max_entries: 1000000,
            excludes: Vec::new(),
        }
    }

    /// Load configuration from `/System/etc/searchd.conf`.
    /// Falls back to defaults for any missing values.
    pub fn load() -> Config {
        let mut cfg = Config::defaults();

        if let Some(v) = SEARCHD_SCHEMA.read_i64("config/idle_timeout_ms") {
            if v > 0 {
                cfg.idle_timeout_ms = v as u32;
            }
        }
        if let Some(v) = SEARCHD_SCHEMA.read_i64("config/max_entries") {
            if v > 0 {
                cfg.max_entries = v as u32;
            }
        }
        if let Some(v) = SEARCHD_SCHEMA.read_string("config/excludes_csv") {
            cfg.excludes = parse_excludes_csv(&v);
        }
        anyos_std::println!(
            "searchd: config loaded from confd (idle={}ms, max={}, excludes={})",
            cfg.idle_timeout_ms,
            cfg.max_entries,
            cfg.excludes.len()
        );

        cfg
    }
}

pub fn register_manifest() {
    let _ = SEARCHD_SCHEMA.register();
}

// ── Internal helpers ────────────────────────────────────────────────────────

fn parse_excludes_csv(value: &str) -> Vec<String> {
    let mut excludes = Vec::new();
    for part in value.split(',') {
        let trimmed = part.trim();
        if !trimmed.is_empty() {
            excludes.push(String::from(trimmed));
        }
    }
    excludes
}
