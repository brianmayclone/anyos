use alloc::string::{String, ToString};
use anyos_std::fs;
use anyos_std::json::Value;

use crate::model::ExistingInstallation;
use crate::util::{join_root, path_exists};

pub fn detect_existing_installation(root: &str) -> Option<ExistingInstallation> {
    if !path_exists(&join_root(root, "/System/krnl64")) {
        return None;
    }

    let version = read_system_version(root).unwrap_or_else(|| String::from("unknown"));
    let has_users = path_exists(&join_root(root, "/Users"));
    Some(ExistingInstallation { version, has_users })
}

pub fn read_system_version(root: &str) -> Option<String> {
    let manifest = join_root(root, "/System/etc/apkg/system-manifest.json");
    if let Ok(text) = fs::read_to_string(&manifest) {
        if let Ok(value) = Value::parse(&text) {
            if let Some(version) = value["version"].as_str() {
                return Some(String::from(version));
            }
        }
    }

    let version_file = join_root(root, "/VERSION");
    fs::read_to_string(&version_file)
        .ok()
        .map(|s| s.trim().to_string())
}
