use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleKind {
    JavaScript,
    Json,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModule {
    pub id: String,
    pub filename: String,
    pub dirname: String,
    pub source: String,
    pub kind: ModuleKind,
}

#[derive(Clone, Debug)]
pub struct ModuleResolver {
    cwd: String,
}

impl ModuleResolver {
    pub fn new(cwd: &str) -> Self {
        Self {
            cwd: normalize_path(cwd),
        }
    }

    pub fn resolve(&self, specifier: &str, from_dir: &str) -> Option<ResolvedModule> {
        if is_core_module(specifier) {
            return None;
        }
        if is_relative_or_absolute(specifier) {
            let base = if specifier.starts_with('/') {
                normalize_path(specifier)
            } else {
                normalize_path(&join_path(from_dir, specifier))
            };
            return self.load_as_file_or_directory(&base, specifier);
        }
        self.load_node_modules(specifier, from_dir)
    }

    fn load_node_modules(&self, specifier: &str, from_dir: &str) -> Option<ResolvedModule> {
        let mut current = normalize_path(from_dir);
        loop {
            let candidate = join_path(&join_path(&current, "node_modules"), specifier);
            if let Some(module) = self.load_as_file_or_directory(&candidate, specifier) {
                return Some(module);
            }
            if current == "/" || current == "." || current.is_empty() {
                break;
            }
            let parent = dirname(&current);
            if parent == current {
                break;
            }
            current = parent;
        }
        let candidate = join_path(&join_path(&self.cwd, "node_modules"), specifier);
        if let Some(module) = self.load_as_file_or_directory(&candidate, specifier) {
            return Some(module);
        }
        for base in system_node_module_bases() {
            let candidate = join_path(&base, specifier);
            if let Some(module) = self.load_as_file_or_directory(&candidate, specifier) {
                return Some(module);
            }
        }
        None
    }

    fn load_as_file_or_directory(&self, base: &str, id: &str) -> Option<ResolvedModule> {
        if let Some(module) = load_file(base, id) {
            return Some(module);
        }
        if let Some(module) = load_file(&format!("{}.js", base), id) {
            return Some(module);
        }
        if let Some(module) = load_file(&format!("{}.json", base), id) {
            return Some(module);
        }
        self.load_directory(base, id)
    }

    fn load_directory(&self, dir: &str, id: &str) -> Option<ResolvedModule> {
        let package_json = join_path(dir, "package.json");
        if let Ok(package) = anyos_std::fs::read_to_string(&package_json) {
            if let Some(entry) = package_entry(&package) {
                let entry_path = join_path(dir, &entry);
                if let Some(module) = self.load_as_file_or_directory(&entry_path, id) {
                    return Some(module);
                }
            }
        }
        load_file(&join_path(dir, "index.js"), id)
            .or_else(|| load_file(&join_path(dir, "index.json"), id))
    }
}

pub fn find_require_specifiers(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= bytes.len() {
        if bytes[i] == b'\'' || bytes[i] == b'"' || bytes[i] == b'`' {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if &bytes[i..i + 7] == b"require" {
            let before_ok = i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric()
                    || bytes[i - 1] == b'_'
                    || bytes[i - 1] == b'$');
            if !before_ok {
                i += 1;
                continue;
            }
            let mut j = i + 7;
            let after_ok = j >= bytes.len()
                || !(bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'$');
            if !after_ok {
                i += 1;
                continue;
            }
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j + 8 <= bytes.len() && &bytes[j..j + 8] == b".resolve" {
                j += 8;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
            }
            if j < bytes.len() && bytes[j] == b'(' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'\'' || bytes[j] == b'"') {
                    let quote = bytes[j];
                    j += 1;
                    let start = j;
                    while j < bytes.len() && bytes[j] != quote {
                        if bytes[j] == b'\\' {
                            j += 1;
                        }
                        j += 1;
                    }
                    if j <= bytes.len() {
                        if let Ok(specifier) = core::str::from_utf8(&bytes[start..j]) {
                            out.push(String::from(specifier));
                        }
                    }
                }
            }
        }
        i += 1;
    }
    out
}

pub fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => String::from("/"),
        Some(idx) => String::from(&trimmed[..idx]),
        None => String::from("."),
    }
}

pub fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(idx) => String::from(&trimmed[idx + 1..]),
        None => String::from(trimmed),
    }
}

pub fn extname(path: &str) -> String {
    let base = basename(path);
    if let Some(idx) = base.rfind('.') {
        if idx > 0 {
            return String::from(&base[idx..]);
        }
    }
    String::new()
}

pub fn join_path(left: &str, right: &str) -> String {
    if right.starts_with('/') {
        return normalize_path(right);
    }
    if left.is_empty() || left == "." {
        normalize_path(right)
    } else if left.ends_with('/') {
        normalize_path(&format!("{}{}", left, right))
    } else {
        normalize_path(&format!("{}/{}", left, right))
    }
}

pub fn normalize_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let mut out = String::new();
    if absolute {
        out.push('/');
    }
    out.push_str(&parts.join("/"));
    if out.is_empty() {
        String::from(if absolute { "/" } else { "." })
    } else {
        out
    }
}

pub fn is_core_module(specifier: &str) -> bool {
    core_module_canonical_name(specifier).is_some()
}

pub fn core_module_canonical_name(specifier: &str) -> Option<&'static str> {
    match specifier {
        "assert" | "node:assert" => Some("assert"),
        "assert/strict" | "node:assert/strict" => Some("assert/strict"),
        "buffer" | "node:buffer" => Some("buffer"),
        "child_process" | "node:child_process" => Some("child_process"),
        "constants" | "node:constants" => Some("constants"),
        "crypto" | "node:crypto" => Some("crypto"),
        "dns" | "node:dns" => Some("dns"),
        "dns/promises" | "node:dns/promises" => Some("dns/promises"),
        "events" | "node:events" => Some("events"),
        "fs" | "node:fs" => Some("fs"),
        "fs/promises" | "node:fs/promises" => Some("fs/promises"),
        "http" | "node:http" => Some("http"),
        "https" | "node:https" => Some("https"),
        "net" | "node:net" => Some("net"),
        "module" | "node:module" => Some("module"),
        "os" | "node:os" => Some("os"),
        "path" | "node:path" => Some("path"),
        "path/posix" | "node:path/posix" => Some("path/posix"),
        "path/win32" | "node:path/win32" => Some("path/win32"),
        "process" | "node:process" => Some("process"),
        "querystring" | "node:querystring" => Some("querystring"),
        "stream" | "node:stream" => Some("stream"),
        "stream/consumers" | "node:stream/consumers" => Some("stream/consumers"),
        "stream/promises" | "node:stream/promises" => Some("stream/promises"),
        "stream/web" | "node:stream/web" => Some("stream/web"),
        "string_decoder" | "node:string_decoder" => Some("string_decoder"),
        "timers" | "node:timers" => Some("timers"),
        "timers/promises" | "node:timers/promises" => Some("timers/promises"),
        "tls" | "node:tls" => Some("tls"),
        "util" | "node:util" => Some("util"),
        "util/types" | "node:util/types" => Some("util/types"),
        "url" | "node:url" => Some("url"),
        "zlib" | "node:zlib" => Some("zlib"),
        "@anyos/ffi" => Some("@anyos/ffi"),
        "@anyos/anyui" => Some("@anyos/anyui"),
        "@anyos/image" => Some("@anyos/image"),
        "node:anyos-ffi" => Some("node:anyos-ffi"),
        "node:anyos-image" => Some("node:anyos-image"),
        "node:anyui" => Some("node:anyui"),
        "node:uv" => Some("node:uv"),
        _ => None,
    }
}

fn system_node_module_bases() -> Vec<String> {
    let mut bases = Vec::new();
    #[cfg(feature = "host")]
    if let Ok(value) = std::env::var("ANYOS_NODE_SYSTEM_PACKAGES") {
        for base in value.split(':') {
            let base = base.trim();
            if !base.is_empty() {
                bases.push(normalize_path(base));
            }
        }
    }
    bases.push(String::from("/System/Library/node_modules"));
    bases
}

fn is_relative_or_absolute(specifier: &str) -> bool {
    specifier.starts_with("./") || specifier.starts_with("../") || specifier.starts_with('/')
}

fn load_file(path: &str, id: &str) -> Option<ResolvedModule> {
    let source = anyos_std::fs::read_to_string(path).ok()?;
    let kind = if path.ends_with(".json") {
        ModuleKind::Json
    } else {
        ModuleKind::JavaScript
    };
    Some(ResolvedModule {
        id: String::from(id),
        filename: normalize_path(path),
        dirname: dirname(path),
        source,
        kind,
    })
}

fn package_entry(package: &str) -> Option<String> {
    json_string_field(package, "exports").or_else(|| json_string_field(package, "main"))
}

fn json_string_field(source: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\"", field);
    let pos = source.find(&needle)?;
    let after = &source[pos + needle.len()..];
    let colon = after.find(':')?;
    let after_colon = after[colon + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let rest = &after_colon[1..];
    let end = rest.find('"')?;
    Some(String::from(&rest[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_literal_requires() {
        let specs = find_require_specifiers(
            "const x = require('./x'); require(\"pkg\"); require.resolve('./r')",
        );
        assert_eq!(
            specs,
            vec![
                String::from("./x"),
                String::from("pkg"),
                String::from("./r")
            ]
        );
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize_path("/a/./b/../c"), "/a/c");
        assert_eq!(join_path("/a/b", "../c"), "/a/c");
    }

    #[test]
    fn reads_package_entry() {
        assert_eq!(
            package_entry("{\"name\":\"x\",\"main\":\"src/index.js\"}"),
            Some(String::from("src/index.js"))
        );
    }
}
