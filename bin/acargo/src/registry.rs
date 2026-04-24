//! crates.io sparse registry client.
//!
//! Fetches crate metadata from the crates.io sparse index (RFC 2789),
//! downloads `.crate` archives, extracts them, and caches everything
//! locally under `/System/var/ccargo/registry/`.
//!
//! ## Index URL scheme
//!
//! Base: `https://index.crates.io/`
//! - 1-char names: `1/{name}`
//! - 2-char names: `2/{name}`
//! - 3-char names: `3/{first_char}/{name}`
//! - 4+ char names: `{first_two}/{next_two}/{name}`
//!
//! ## Download URL
//!
//! `https://static.crates.io/crates/{name}/{name}-{version}.crate`
//!
//! `.crate` files are gzip-compressed tar archives.

use crate::prelude::*;
use crate::fs;
use crate::semver::{Version, VersionReq};
use anyos_std::println;
use anyos_std::collections::HashMap;

/// Registry base URLs.
const INDEX_BASE: &str = "https://index.crates.io";
const DOWNLOAD_BASE: &str = "https://static.crates.io/crates";

/// Local cache directory.
const CACHE_DIR: &str = "/System/var/ccargo/registry";
const SRC_CACHE_DIR: &str = "/System/var/ccargo/registry/src";
const INDEX_CACHE_DIR: &str = "/System/var/ccargo/registry/index";
const CRATE_CACHE_DIR: &str = "/System/var/ccargo/registry/cache";

/// A single version entry from the crates.io index.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub name: String,
    pub version: Version,
    pub deps: Vec<IndexDep>,
    pub checksum: String,
    pub features: Vec<(String, Vec<String>)>,
    pub yanked: bool,
}

/// A dependency as listed in the crates.io index.
#[derive(Debug, Clone)]
pub struct IndexDep {
    pub name: String,
    pub req: String,
    pub features: Vec<String>,
    pub optional: bool,
    pub default_features: bool,
    pub kind: String,  // "normal", "dev", "build"
    /// Actual package name if renamed (package = "real_name" in Cargo.toml).
    pub package: Option<String>,
}

/// Initialize the registry cache directories.
pub fn init_cache() {
    fs::mkdir_p(CACHE_DIR);
    fs::mkdir_p(SRC_CACHE_DIR);
    fs::mkdir_p(INDEX_CACHE_DIR);
    fs::mkdir_p(CRATE_CACHE_DIR);
}

/// Compute the sparse index path prefix for a crate name.
fn index_prefix(name: &str) -> String {
    let lower = name.to_lowercase();
    let len = lower.len();
    match len {
        1 => format!("1"),
        2 => format!("2"),
        3 => {
            let first = &lower[..1];
            format!("3/{}", first)
        }
        _ => {
            let a = &lower[..2];
            let b = &lower[2..4];
            format!("{}/{}", a, b)
        }
    }
}

/// Fetch index entries for a crate from the sparse index.
/// Returns all published versions (including yanked).
pub fn fetch_index(name: &str) -> Option<Vec<IndexEntry>> {
    let lower = name.to_lowercase();
    let prefix = index_prefix(&lower);
    let url = format!("{}/{}/{}", INDEX_BASE, prefix, lower);

    // Check local cache first
    let cache_path = format!("{}/{}/{}", INDEX_CACHE_DIR, prefix, lower);
    let index_data = if let Some(cached) = fs::read_file(&cache_path) {
        cached
    } else {
        // Fetch from network
        libhttp_client::init();
        let data = libhttp_client::get(&url)?;
        let text = alloc::string::String::from_utf8(data).ok()?;

        // Cache it
        let cache_dir = format!("{}/{}", INDEX_CACHE_DIR, prefix);
        fs::mkdir_p(&cache_dir);
        fs::write_file(&cache_path, text.as_bytes());

        text
    };

    // Parse NDJSON (one JSON object per line)
    let mut entries = Vec::new();
    for line in index_data.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Some(entry) = parse_index_line(line) {
            entries.push(entry);
        }
    }

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Invalidate the cached index for a crate (force re-fetch).
pub fn invalidate_index(name: &str) {
    let lower = name.to_lowercase();
    let prefix = index_prefix(&lower);
    let cache_path = format!("{}/{}/{}", INDEX_CACHE_DIR, prefix, lower);
    anyos_std::fs::unlink(&cache_path);
}

/// Resolve a version requirement against the index.
/// Returns the best matching (newest non-yanked) version and its entry.
pub fn resolve_version(name: &str, req_str: &str) -> Option<IndexEntry> {
    let entries = fetch_index(name)?;
    let req = VersionReq::parse(req_str)?;

    let versions: Vec<(Version, bool)> = entries.iter()
        .map(|e| (e.version.clone(), e.yanked))
        .collect();

    let best = crate::semver::resolve_best(&versions, &req)?;

    entries.into_iter().find(|e| e.version == best)
}

/// Download and extract a crate to the source cache.
/// Returns the path to the extracted source directory.
pub fn fetch_crate(name: &str, version: &Version) -> Option<String> {
    let ver_str = version.to_string();
    let src_dir = format!("{}/{}-{}", SRC_CACHE_DIR, name, ver_str);

    // Check if already extracted
    let cargo_toml = format!("{}/Cargo.toml", src_dir);
    if fs::file_exists(&cargo_toml) {
        return Some(src_dir);
    }

    // Check if .crate is already downloaded
    let crate_filename = format!("{}-{}.crate", name, ver_str);
    let crate_path = format!("{}/{}", CRATE_CACHE_DIR, crate_filename);

    if !fs::file_exists(&crate_path) {
        // Download
        let url = format!("{}/{}/{}", DOWNLOAD_BASE, name, crate_filename);
        println!("  Downloading {} v{}", name, ver_str);

        libhttp_client::init();
        if !libhttp_client::download(&url, &crate_path) {
            let status = libhttp_client::last_status();
            let err = libhttp_client::last_error();
            println!("ccargo: error downloading {}: HTTP {} (err {})", name, status, err);
            return None;
        }
    }

    // Extract .crate (gzip tar) to source cache
    println!("   Extracting {} v{}", name, ver_str);
    fs::mkdir_p(&src_dir);

    let reader = libzip_client::TarReader::open(&crate_path)?;
    let count = reader.entry_count();

    for i in 0..count {
        let entry_name = reader.entry_name(i);
        let is_dir = reader.entry_is_dir(i);

        // .crate files have a top-level {name}-{version}/ directory.
        // Strip that prefix so files go directly into src_dir.
        let prefix = format!("{}-{}/", name, ver_str);
        let relative = if entry_name.starts_with(&prefix) {
            &entry_name[prefix.len()..]
        } else {
            &entry_name
        };

        if relative.is_empty() { continue; }

        let out_path = format!("{}/{}", src_dir, relative);

        if is_dir {
            fs::mkdir_p(&out_path);
        } else {
            // Ensure parent directory exists
            if let Some(slash_pos) = out_path.rfind('/') {
                fs::mkdir_p(&out_path[..slash_pos]);
            }
            reader.extract_to_file(i, &out_path);
        }
    }

    // Verify Cargo.toml exists
    if fs::file_exists(&cargo_toml) {
        Some(src_dir)
    } else {
        println!("ccargo: warning: no Cargo.toml found in {}", name);
        None
    }
}

/// Download and extract a crate, returning the path to its source directory.
/// Convenience wrapper that combines resolve + fetch.
pub fn get_crate(name: &str, version_req: &str) -> Option<(String, IndexEntry)> {
    if let Some(resolved) = find_cached_source(name, version_req) {
        return Some(resolved);
    }

    let entry = resolve_version(name, version_req)?;
    let src_dir = fetch_crate(name, &entry.version)?;
    Some((src_dir, entry))
}

fn find_cached_source(name: &str, version_req: &str) -> Option<(String, IndexEntry)> {
    let req = VersionReq::parse(version_req)?;
    let mut candidates: Vec<(Version, String)> = Vec::new();

    for root in registry_source_roots() {
        collect_cached_sources(name, &root, &mut candidates);

        for registry_dir in list_dir_names(&root) {
            let nested = format!("{}/{}", root, registry_dir);
            if fs::dir_exists(&nested) {
                collect_cached_sources(name, &nested, &mut candidates);
            }
        }
    }

    let versions: Vec<(Version, bool)> = candidates
        .iter()
        .map(|(version, _)| (version.clone(), false))
        .collect();
    let best = crate::semver::resolve_best(&versions, &req)?;
    let src_dir = candidates
        .into_iter()
        .find(|(version, _)| *version == best)
        .map(|(_, path)| path)?;

    Some((src_dir, IndexEntry {
        name: name.to_string(),
        version: best,
        deps: Vec::new(),
        checksum: String::new(),
        features: Vec::new(),
        yanked: false,
    }))
}

fn registry_source_roots() -> Vec<String> {
    let mut roots = vec![SRC_CACHE_DIR.to_string()];

    if let Some(path) = env_var("CCARGO_REGISTRY_SRC") {
        roots.push(path);
    }
    if let Some(home) = env_var("CARGO_HOME") {
        roots.push(format!("{}/registry/src", home));
    }
    if let Some(home) = env_var("HOME") {
        roots.push(format!("{}/.cargo/registry/src", home));
    }

    roots
}

fn collect_cached_sources(name: &str, root: &str, out: &mut Vec<(Version, String)>) {
    let prefix = format!("{}-", name);
    for entry in list_dir_names(root) {
        if !entry.starts_with(&prefix) {
            continue;
        }
        let version_str = &entry[prefix.len()..];
        let Some(version) = Version::parse(version_str) else {
            continue;
        };
        let path = format!("{}/{}", root, entry);
        if fs::file_exists(&format!("{}/Cargo.toml", path)) {
            out.push((version, path));
        }
    }
}

fn list_dir_names(path: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut buf = vec![0u8; 64 * 8192];
    let count = anyos_std::fs::readdir(path, &mut buf);
    if count == u32::MAX {
        return names;
    }

    let entry_size = 64;
    for i in 0..count as usize {
        let off = i * entry_size;
        let name_len = buf[off + 1] as usize;
        if name_len == 0 || off + 8 + name_len > buf.len() {
            continue;
        }
        let name_bytes = &buf[off + 8..off + 8 + name_len];
        let Ok(name) = core::str::from_utf8(name_bytes) else {
            continue;
        };
        if name != "." && name != ".." {
            names.push(name.to_string());
        }
    }
    names
}

fn env_var(key: &str) -> Option<String> {
    let mut buf = [0u8; 4096];
    let len = anyos_std::env::get(key, &mut buf);
    if len == u32::MAX || len as usize > buf.len() {
        return None;
    }
    core::str::from_utf8(&buf[..len as usize])
        .ok()
        .map(|value| value.to_string())
}

// ── Index JSON parsing ──
// Minimal JSON parsing for index entries. Each line is a flat JSON object.

fn parse_index_line(line: &str) -> Option<IndexEntry> {
    let obj = parse_json_object(line)?;

    let name = json_str(&obj, "name")?.to_string();
    let vers = json_str(&obj, "vers")?;
    let version = Version::parse(vers)?;
    let checksum = json_str(&obj, "cksum").unwrap_or("").to_string();
    let yanked = json_bool(&obj, "yanked").unwrap_or(false);

    let deps = if let Some(arr) = json_array(&obj, "deps") {
        arr.iter().filter_map(|d| parse_index_dep(d)).collect()
    } else {
        Vec::new()
    };

    let features = if let Some(feat_obj) = json_object(&obj, "features") {
        feat_obj.iter().map(|(k, v)| {
            let vals = match v {
                JsonValue::Array(arr) => arr.iter().filter_map(|x| {
                    if let JsonValue::String(s) = x { Some(s.clone()) } else { None }
                }).collect(),
                _ => Vec::new(),
            };
            (k.clone(), vals)
        }).collect()
    } else {
        Vec::new()
    };

    Some(IndexEntry { name, version, deps, checksum, features, yanked })
}

fn parse_index_dep(val: &JsonValue) -> Option<IndexDep> {
    let obj = if let JsonValue::Object(o) = val { o } else { return None };
    let name = json_str_from(obj, "name")?.to_string();
    let req = json_str_from(obj, "req").unwrap_or("*").to_string();
    let optional = json_bool_from(obj, "optional").unwrap_or(false);
    let default_features = json_bool_from(obj, "default_features").unwrap_or(true);
    let kind = json_str_from(obj, "kind").unwrap_or("normal").to_string();
    let package = json_str_from(obj, "package").map(|s| s.to_string());

    let features = if let Some(JsonValue::Array(arr)) = obj.get("features") {
        arr.iter().filter_map(|x| {
            if let JsonValue::String(s) = x { Some(s.clone()) } else { None }
        }).collect()
    } else {
        Vec::new()
    };

    Some(IndexDep { name, req, features, optional, default_features, kind, package })
}

// ── Minimal JSON parser ──

#[derive(Debug, Clone)]
enum JsonValue {
    String(String),
    Number(i64),
    Bool(bool),
    Null,
    Array(Vec<JsonValue>),
    Object(HashMap<String, JsonValue>),
}

fn parse_json_object(s: &str) -> Option<HashMap<String, JsonValue>> {
    let s = s.trim();
    if !s.starts_with('{') || !s.ends_with('}') { return None; }
    let inner = &s[1..s.len() - 1];
    let mut map = HashMap::new();
    let pairs = split_json_top(inner, b',');

    for pair in &pairs {
        let pair = pair.trim();
        if pair.is_empty() { continue; }
        // Find the colon separating key from value
        let (key, value) = split_json_kv(pair)?;
        let key = key.trim().trim_matches('"').to_string();
        let value = parse_json_value(value.trim());
        map.insert(key, value);
    }

    Some(map)
}

fn parse_json_value(s: &str) -> JsonValue {
    let s = s.trim();
    if s.starts_with('"') {
        // String — handle escapes
        let inner = &s[1..s.len().saturating_sub(1)];
        let unescaped = unescape_json_str(inner);
        JsonValue::String(unescaped)
    } else if s == "true" {
        JsonValue::Bool(true)
    } else if s == "false" {
        JsonValue::Bool(false)
    } else if s == "null" {
        JsonValue::Null
    } else if s.starts_with('[') {
        // Array
        let inner = &s[1..s.len().saturating_sub(1)];
        let items = split_json_top(inner, b',');
        let arr = items.iter()
            .map(|item| parse_json_value(item.trim()))
            .filter(|v| !matches!(v, JsonValue::Null) || true)
            .collect();
        JsonValue::Array(arr)
    } else if s.starts_with('{') {
        // Nested object
        if let Some(obj) = parse_json_object(s) {
            JsonValue::Object(obj)
        } else {
            JsonValue::Null
        }
    } else {
        // Number
        if let Ok(n) = s.parse::<i64>() {
            JsonValue::Number(n)
        } else {
            JsonValue::Null
        }
    }
}

fn split_json_top(s: &str, sep: u8) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut start = 0;

    for (i, b) in s.bytes().enumerate() {
        if escape { escape = false; continue; }
        if b == b'\\' && in_string { escape = true; continue; }
        if b == b'"' { in_string = !in_string; continue; }
        if in_string { continue; }
        match b {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            _ if b == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn split_json_kv<'a>(s: &'a str) -> Option<(&'a str, &'a str)> {
    // Find the first : not inside a string
    let mut in_string = false;
    let mut escape = false;
    for (i, b) in s.bytes().enumerate() {
        if escape { escape = false; continue; }
        if b == b'\\' && in_string { escape = true; continue; }
        if b == b'"' { in_string = !in_string; continue; }
        if !in_string && b == b':' {
            return Some((&s[..i], &s[i + 1..]));
        }
    }
    None
}

fn unescape_json_str(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('/') => out.push('/'),
                Some(other) => { out.push('\\'); out.push(other); }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn json_str<'a>(map: &'a HashMap<String, JsonValue>, key: &str) -> Option<&'a str> {
    if let Some(JsonValue::String(s)) = map.get(key) { Some(s) } else { None }
}

fn json_bool(map: &HashMap<String, JsonValue>, key: &str) -> Option<bool> {
    if let Some(JsonValue::Bool(b)) = map.get(key) { Some(*b) } else { None }
}

fn json_array<'a>(map: &'a HashMap<String, JsonValue>, key: &str) -> Option<&'a Vec<JsonValue>> {
    if let Some(JsonValue::Array(a)) = map.get(key) { Some(a) } else { None }
}

fn json_object<'a>(map: &'a HashMap<String, JsonValue>, key: &str) -> Option<&'a HashMap<String, JsonValue>> {
    if let Some(JsonValue::Object(o)) = map.get(key) { Some(o) } else { None }
}

fn json_str_from<'a>(map: &'a HashMap<String, JsonValue>, key: &str) -> Option<&'a str> {
    json_str(map, key)
}

fn json_bool_from(map: &HashMap<String, JsonValue>, key: &str) -> Option<bool> {
    json_bool(map, key)
}

/// Clean the entire registry cache (index + sources + crates).
pub fn clean_cache() {
    fs::rm_rf(INDEX_CACHE_DIR);
    fs::rm_rf(SRC_CACHE_DIR);
    fs::rm_rf(CRATE_CACHE_DIR);
    init_cache();
    println!("     Cleaned registry cache");
}

/// List all cached crates.
pub fn list_cached() -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut buf = [0u8; 64 * 128];
    let count = anyos_std::fs::readdir(SRC_CACHE_DIR, &mut buf);
    if count == u32::MAX { return result; }

    let entry_size = 64;
    for i in 0..count as usize {
        let off = i * entry_size;
        let file_type = buf[off];
        let name_len = buf[off + 1] as usize;
        if name_len == 0 || file_type != 1 { continue; }
        let name_bytes = &buf[off + 8..off + 8 + name_len];
        if let Ok(name) = core::str::from_utf8(name_bytes) {
            if name == "." || name == ".." { continue; }
            // Parse "crate_name-version" format
            if let Some(last_dash) = name.rfind('-') {
                let crate_name = &name[..last_dash];
                let version = &name[last_dash + 1..];
                result.push((crate_name.to_string(), version.to_string()));
            }
        }
    }
    result
}
