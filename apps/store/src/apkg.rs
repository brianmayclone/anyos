//! Package data layer — reads apkg index/installed JSON, provides version
//! comparison and CLI wrappers for install/remove/upgrade.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;
use anyos_std::fs;

// ─── Paths (mirror bin/apkg/src/config.rs) ─────────────────────────

const INDEX_PATH: &str = "/System/etc/apkg/index.json";
const INSTALLED_PATH: &str = "/System/etc/apkg/installed.json";

// ─── Data Structures ───────────────────────────────────────────────

/// A package from the repository index.
#[derive(Clone)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub category: String,
    pub pkg_type: String,
    pub size: u64,
    pub size_installed: u64,
    pub depends: Vec<String>,
}

/// An installed package record.
#[derive(Clone)]
pub struct InstalledEntry {
    pub name: String,
    pub version: String,
    pub pkg_type: String,
}

/// Installation status of a package.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PkgStatus {
    /// Not installed — action: "Get".
    Available,
    /// Installed and up-to-date — action: "Installed".
    Installed,
    /// Installed but a newer version exists — action: "Update".
    Updatable,
}

// ─── Index Loading ─────────────────────────────────────────────────

/// Load available packages from the cached repository index.
pub fn load_index() -> Vec<PackageInfo> {
    let content = match fs::read_to_string(INDEX_PATH) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let val = match Value::parse(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let packages_arr = match val["packages"].as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut out = Vec::with_capacity(packages_arr.len());
    for pkg in packages_arr {
        let name: String = pkg["name"].as_str().unwrap_or("").into();
        if name.is_empty() {
            continue;
        }
        let version: String = pkg["version"].as_str().unwrap_or("0.0.0").into();
        let description: String = pkg["description"].as_str().unwrap_or("").into();
        let category: String = pkg["category"].as_str().unwrap_or("misc").into();
        let pkg_type: String = pkg["type"].as_str().unwrap_or("bin").into();
        let size = pkg["size"].as_u64().unwrap_or(0);
        let size_installed = pkg["size_installed"].as_u64().unwrap_or(0);
        let depends = parse_string_array(&pkg["depends"]);

        out.push(PackageInfo {
            name, version, description, category, pkg_type,
            size, size_installed, depends,
        });
    }
    out
}

/// Load installed packages from the database.
pub fn load_installed() -> Vec<InstalledEntry> {
    let content = match fs::read_to_string(INSTALLED_PATH) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let val = match Value::parse(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let pkgs_obj = match val["packages"].as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut out = Vec::new();
    for (name, pkg_val) in pkgs_obj.iter() {
        let version: String = pkg_val["version"].as_str().unwrap_or("0.0.0").into();
        let pkg_type: String = pkg_val["type"].as_str().unwrap_or("bin").into();
        out.push(InstalledEntry {
            name: String::from(name),
            version,
            pkg_type,
        });
    }
    out
}

// ─── Status & Version Comparison ───────────────────────────────────

/// Determine installation status of a package.
pub fn get_status(pkg: &PackageInfo, installed: &[InstalledEntry]) -> PkgStatus {
    match installed.iter().find(|i| i.name == pkg.name) {
        None => PkgStatus::Available,
        Some(inst) => {
            if version_less_than(&inst.version, &pkg.version) {
                PkgStatus::Updatable
            } else {
                PkgStatus::Installed
            }
        }
    }
}

/// Simple semantic version comparison: returns true if `a < b`.
fn version_less_than(a: &str, b: &str) -> bool {
    let pa = parse_version(a);
    let pb = parse_version(b);
    pa < pb
}

/// Parse "MAJOR.MINOR.PATCH" into a tuple for comparison.
fn parse_version(s: &str) -> (u32, u32, u32) {
    let mut parts = s.split('.');
    let major = parts.next().and_then(parse_u32).unwrap_or(0);
    let minor = parts.next().and_then(parse_u32).unwrap_or(0);
    let patch = parts.next().and_then(parse_u32).unwrap_or(0);
    (major, minor, patch)
}

fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

fn parse_string_array(val: &Value) -> Vec<String> {
    match val.as_array() {
        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        None => Vec::new(),
    }
}

// ─── CLI Wrappers ──────────────────────────────────────────────────

/// Install a package via the apkg CLI. Returns the exit code.
pub fn install_package(name: &str) -> u32 {
    run_apkg(&alloc::format!("install {} -y", name))
}

/// Remove a package via the apkg CLI. Returns the exit code.
pub fn remove_package(name: &str) -> u32 {
    run_apkg(&alloc::format!("remove {} -y", name))
}

/// Upgrade a package via the apkg CLI. Returns the exit code.
pub fn upgrade_package(name: &str) -> u32 {
    run_apkg(&alloc::format!("upgrade {} -y", name))
}

/// Update the package index via the apkg CLI. Returns the exit code.
pub fn update_index() -> u32 {
    run_apkg("update")
}

/// Spawn apkg with the given arguments, wait for completion, return exit code.
fn run_apkg(args: &str) -> u32 {
    let pid = anyos_std::process::spawn("/System/bin/apkg", args);
    if pid == u32::MAX {
        return 1;
    }
    anyos_std::process::waitpid(pid)
}

// ─── Utilities ─────────────────────────────────────────────────────

/// Format a byte size for display (e.g. "1.2 MB", "340 KB").
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        let mb = bytes / 1_048_576;
        let frac = (bytes % 1_048_576) * 10 / 1_048_576;
        alloc::format!("{}.{} MB", mb, frac)
    } else if bytes >= 1024 {
        alloc::format!("{} KB", bytes / 1024)
    } else {
        alloc::format!("{} B", bytes)
    }
}

/// Simple ASCII case-insensitive substring search.
pub fn matches_search(text: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query_lower = to_lower(query);
    let text_lower = to_lower(text);
    text_lower.contains(&*query_lower)
}

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
