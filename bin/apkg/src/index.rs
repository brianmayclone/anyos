//! Repository index parsing and querying.
//!
//! Reads `index.json` from cache and provides lookup by name, search, etc.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;
use anyos_std::fs;
use crate::config;
use crate::version::Version;

/// A single package entry from the repository index.
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub version: Version,
    pub version_str: String,
    pub description: String,
    pub category: String,
    pub pkg_type: String,
    pub arch: String,
    pub depends: Vec<String>,
    pub provides: Vec<String>,
    pub size: u64,
    pub size_installed: u64,
    pub md5: String,
    pub filename: String,
    pub min_os_version: String,
}

/// Parsed repository index.
pub struct Index {
    pub repository: String,
    pub packages: Vec<PackageInfo>,
}

impl Index {
    /// Load the cached index from disk.
    pub fn load() -> Option<Index> {
        let content = fs::read_to_string(config::INDEX_PATH).ok()?;
        Self::parse(&content)
    }

    /// Parse an index from JSON string.
    pub fn parse(json_str: &str) -> Option<Index> {
        let val = Value::parse(json_str).ok()?;
        let repository = val["repository"].as_str().unwrap_or("unknown");
        let packages_arr = val["packages"].as_array()?;

        let mut packages = Vec::with_capacity(packages_arr.len());
        for pkg in packages_arr {
            let name = pkg["name"].as_str().unwrap_or("").into();
            let version_str: String = pkg["version"].as_str().unwrap_or("0.0.0").into();
            let version = Version::parse(&version_str).unwrap_or(Version { major: 0, minor: 0, patch: 0 });
            let description = pkg["description"].as_str().unwrap_or("").into();
            let category = pkg["category"].as_str().unwrap_or("").into();
            let pkg_type = pkg["type"].as_str().unwrap_or("bin").into();
            let arch = pkg["arch"].as_str().unwrap_or("x86_64").into();
            let size = pkg["size"].as_u64().unwrap_or(0);
            let size_installed = pkg["size_installed"].as_u64().unwrap_or(0);
            let md5 = pkg["md5"].as_str().unwrap_or("").into();
            let filename = pkg["filename"].as_str().unwrap_or("").into();
            let min_os_version = pkg["min_os_version"].as_str().unwrap_or("0.0.0").into();

            let depends = parse_string_array(&pkg["depends"]);
            let provides = parse_string_array(&pkg["provides"]);

            packages.push(PackageInfo {
                name, version, version_str, description, category, pkg_type,
                arch, depends, provides, size, size_installed, md5, filename,
                min_os_version,
            });
        }

        Some(Index {
            repository: String::from(repository),
            packages,
        })
    }

    /// Find a package by exact name.
    pub fn find(&self, name: &str) -> Option<&PackageInfo> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Find a package by name or by a name that a package provides.
    pub fn find_provider(&self, name: &str) -> Option<&PackageInfo> {
        // Exact name match first
        if let Some(p) = self.find(name) {
            return Some(p);
        }
        // Check provides
        self.packages.iter().find(|p| p.provides.iter().any(|prov| prov == name))
    }

    /// Search packages by name or description substring (case-insensitive).
    pub fn search(&self, term: &str) -> Vec<&PackageInfo> {
        let term_lower = to_lower(term);
        self.packages.iter().filter(|p| {
            to_lower(&p.name).contains(&term_lower)
                || to_lower(&p.description).contains(&term_lower)
        }).collect()
    }

    /// List all packages filtered by architecture.
    pub fn list_for_arch(&self, arch: &str) -> Vec<&PackageInfo> {
        self.packages.iter().filter(|p| p.arch == arch).collect()
    }
}

/// Parse a JSON array of strings.
fn parse_string_array(val: &Value) -> Vec<String> {
    match val.as_array() {
        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        None => Vec::new(),
    }
}

/// Simple ASCII lowercase conversion.
fn to_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b >= b'A' && b <= b'Z' {
            out.push((b + 32) as char);
        } else {
            out.push(b as char);
        }
    }
    out
}
