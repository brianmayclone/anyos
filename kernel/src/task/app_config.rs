//! Parse .app bundle Info.conf files.
//!
//! Used by the loader to determine the exec binary name, working directory,
//! and other metadata when spawning .app bundles.

use alloc::string::String;

/// Parsed fields from an .app bundle's Info.conf file.
pub struct AppConfig {
    /// Stable app identifier (e.g., "com.anyos.terminal").
    pub id: Option<String>,
    /// Display name.
    pub name: Option<String>,
    /// Binary filename in bundle root (e.g., "Terminal").
    pub exec: Option<String>,
    /// Version string (semver or build-id).
    pub version: Option<String>,
    /// Icon filename in bundle (default: Icon.ico).
    pub icon: Option<String>,
    /// Default arguments passed to the binary.
    pub args: Option<String>,
    /// Working directory: "bundle" (default), "home", or an explicit path.
    pub working_dir: Option<String>,
    /// Application category (System, Games, Utilities, etc.).
    pub category: Option<String>,
    /// Comma-separated file extension associations (e.g., ".txt,.conf,.log").
    pub file_associations: Option<String>,
    /// Comma-separated capabilities (e.g., "network,audio,filesystem").
    pub capabilities: Option<String>,
    /// Minimum OS version required.
    pub min_os_version: Option<String>,
}

impl AppConfig {
    /// Create an empty config with all fields set to None.
    pub fn empty() -> Self {
        AppConfig {
            id: None,
            name: None,
            exec: None,
            version: None,
            icon: None,
            args: None,
            working_dir: None,
            category: None,
            file_associations: None,
            capabilities: None,
            min_os_version: None,
        }
    }
}

/// Read and parse Info.conf from a bundle directory.
/// Returns None if the file doesn't exist or can't be parsed.
pub fn parse_info_conf(bundle_path: &str) -> Option<AppConfig> {
    let conf_path = alloc::format!("{}/Info.conf", bundle_path);
    let data = crate::fs::vfs::read_file_to_vec(&conf_path).ok()?;
    let text = core::str::from_utf8(&data).ok()?;

    let mut config = AppConfig::empty();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(idx) = line.find('=') {
            let key = line[..idx].trim();
            let val = line[idx + 1..].trim();
            if val.is_empty() {
                continue;
            }
            let val_str = String::from(val);
            match key {
                "id" => config.id = Some(val_str),
                "name" => config.name = Some(val_str),
                "exec" => config.exec = Some(val_str),
                "version" => config.version = Some(val_str),
                "icon" => config.icon = Some(val_str),
                "args" => config.args = Some(val_str),
                "working_dir" => config.working_dir = Some(val_str),
                "category" => config.category = Some(val_str),
                "file_associations" => config.file_associations = Some(val_str),
                "capabilities" => config.capabilities = Some(val_str),
                "min_os_version" => config.min_os_version = Some(val_str),
                _ => {} // ignore unknown keys for forward compatibility
            }
        }
    }
    Some(config)
}
