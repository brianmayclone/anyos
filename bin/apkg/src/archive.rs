//! Package archive extraction using libzip_client.
//!
//! `.tar.gz` archives are read via `TarReader`. File entries under the
//! `<name>-<version>/files/` prefix are extracted to the filesystem root.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use anyos_std::{fs, println, json::Value};

/// Result of extracting a package archive.
pub struct ExtractResult {
    /// Parsed pkg.json metadata.
    pub pkg_json: Value,
    /// List of installed file paths (absolute).
    pub installed_files: Vec<String>,
}

/// Extract the `pkg.json` metadata from a package archive without installing files.
pub fn read_pkg_json(archive_path: &str) -> Option<Value> {
    let reader = libzip_client::TarReader::open(archive_path)?;
    let count = reader.entry_count();

    for i in 0..count {
        let name = reader.entry_name(i);
        if name.ends_with("/pkg.json") || name == "pkg.json" {
            let data = reader.extract(i)?;
            let json_str = core::str::from_utf8(&data).ok()?;
            return Value::parse(json_str).ok();
        }
    }
    None
}

/// Extract package files to the filesystem.
///
/// Files under `<prefix>/files/` are extracted with the prefix stripped,
/// resulting in absolute paths from the filesystem root.
pub fn extract_package(archive_path: &str) -> Option<ExtractResult> {
    let reader = libzip_client::TarReader::open(archive_path)?;
    let count = reader.entry_count();

    // First pass: find and parse pkg.json, determine the files/ prefix
    let mut pkg_json: Option<Value> = None;
    let mut files_prefix: Option<String> = None;

    for i in 0..count {
        let name = reader.entry_name(i);
        if name.ends_with("/pkg.json") {
            if let Some(data) = reader.extract(i) {
                if let Ok(json_str) = core::str::from_utf8(&data) {
                    pkg_json = Value::parse(json_str).ok();
                }
            }
            // Derive the files prefix from the pkg.json path
            // e.g., "wget-1.2.0/pkg.json" → "wget-1.2.0/files/"
            if let Some(slash) = name.rfind('/') {
                files_prefix = Some(format!("{}/files/", &name[..slash]));
            }
        }
    }

    let pkg_json = pkg_json?;
    let files_prefix = files_prefix?;

    // Second pass: extract files
    let mut installed_files = Vec::new();

    for i in 0..count {
        let name = reader.entry_name(i);
        if !name.starts_with(&files_prefix) {
            continue;
        }

        // Strip the prefix to get the absolute filesystem path
        let rel_path = &name[files_prefix.len()..];
        if rel_path.is_empty() {
            continue;
        }

        let target_path = format!("/{}", rel_path);

        if reader.entry_is_dir(i) {
            ensure_dir(&target_path);
        } else {
            // Ensure parent directories exist
            ensure_parent_dirs(&target_path);
            if reader.extract_to_file(i, &target_path) {
                installed_files.push(target_path);
            } else {
                println!("apkg: failed to extract '{}'", target_path);
            }
        }
    }

    Some(ExtractResult { pkg_json, installed_files })
}

/// Ensure all parent directories for a path exist.
fn ensure_parent_dirs(path: &str) {
    let bytes = path.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] == b'/' && pos > 0 {
            let dir = &path[..pos];
            fs::mkdir(dir);
        }
        pos += 1;
    }
}

/// Ensure a directory exists (create it and all parents).
fn ensure_dir(path: &str) {
    ensure_parent_dirs(path);
    fs::mkdir(path);
}
