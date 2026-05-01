//! Path resolution and normalisation helpers.
//!
//! anyOS uses a traditional Unix-style path hierarchy.  These helpers keep the
//! path manipulation logic in one place so it can be unit-tested and reused
//! without pulling in the full shell context.

use anyos_std::format;
use anyos_std::String;
use anyos_std::Vec;

/// Resolve `rel` against `cwd` and return a canonical absolute path.
///
/// - Absolute paths (`rel` starting with `/`) are normalised and returned
///   directly.
/// - Relative paths are joined to `cwd` first, then normalised.
pub fn resolve_path(cwd: &str, rel: &str) -> String {
    if rel.starts_with('/') {
        return normalize_path(rel);
    }
    let base = if cwd.ends_with('/') {
        format!("{}{}", cwd, rel)
    } else {
        format!("{}/{}", cwd, rel)
    };
    normalize_path(&base)
}

/// Collapse `.` and `..` segments and remove duplicate slashes from `path`.
///
/// Always returns an absolute path starting with `/`.
pub fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return String::from("/");
    }
    let mut out = String::new();
    for p in &parts {
        out.push('/');
        out.push_str(p);
    }
    out
}
