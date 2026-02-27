//! Package manager configuration: paths, mirror list management.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{fs, println};

/// Base directory for apkg state and configuration.
pub const APKG_DIR: &str = "/System/etc/apkg";
/// Mirror list file.
pub const MIRRORS_PATH: &str = "/System/etc/apkg/mirrors.conf";
/// Cached remote index.
pub const INDEX_PATH: &str = "/System/etc/apkg/index.json";
/// Installed package database.
pub const INSTALLED_PATH: &str = "/System/etc/apkg/installed.json";
/// Download cache directory.
pub const CACHE_DIR: &str = "/System/etc/apkg/cache";
/// Backup directory for system packages.
pub const BACKUP_DIR: &str = "/System/etc/apkg/backup";

/// Ensure all apkg directories exist.
pub fn ensure_dirs() {
    fs::mkdir(APKG_DIR);
    fs::mkdir(CACHE_DIR);
    fs::mkdir(BACKUP_DIR);
}

/// Read the list of mirror URLs from mirrors.conf.
/// Returns an empty vec if the file doesn't exist.
pub fn read_mirrors() -> Vec<String> {
    let mut mirrors = Vec::new();
    let content = match fs::read_to_string(MIRRORS_PATH) {
        Ok(s) => s,
        Err(_) => return mirrors,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        mirrors.push(String::from(line));
    }
    mirrors
}

/// Write the mirror list back to mirrors.conf, preserving comments.
fn write_mirrors_raw(content: &str) -> bool {
    match fs::File::create(MIRRORS_PATH) {
        Ok(mut f) => {
            use anyos_std::fs::Write;
            f.write_all(content.as_bytes()).is_ok()
        }
        Err(_) => false,
    }
}

/// Add a mirror URL to the list. Returns false if already present.
pub fn add_mirror(url: &str) -> bool {
    let existing = read_mirrors();
    for m in &existing {
        if m == url {
            return false;
        }
    }
    // Append to file
    let mut content = match fs::read_to_string(MIRRORS_PATH) {
        Ok(s) => s,
        Err(_) => String::new(),
    };
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(url);
    content.push('\n');
    write_mirrors_raw(&content)
}

/// Remove a mirror URL from the list. Returns false if not found.
pub fn remove_mirror(url: &str) -> bool {
    let content = match fs::read_to_string(MIRRORS_PATH) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let mut new_content = String::new();
    let mut found = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == url {
            found = true;
            continue;
        }
        new_content.push_str(line);
        new_content.push('\n');
    }
    if found {
        write_mirrors_raw(&new_content);
    }
    found
}

/// Build the full URL for a package file on a given mirror.
/// `mirror_url` = base URL, `arch` = "x86_64" or "aarch64",
/// `filename` = package archive filename.
pub fn package_url(mirror_url: &str, arch: &str, filename: &str) -> String {
    let base = mirror_url.trim_end_matches('/');
    alloc::format!("{}/packages/{}/{}", base, arch, filename)
}

/// Build the full URL for the index.json on a given mirror.
pub fn index_url(mirror_url: &str) -> String {
    let base = mirror_url.trim_end_matches('/');
    alloc::format!("{}/index.json", base)
}

/// Get the current architecture string.
pub fn arch() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    { "x86_64" }
    #[cfg(target_arch = "aarch64")]
    { "aarch64" }
}
