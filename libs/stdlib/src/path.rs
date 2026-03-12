// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Path manipulation utilities.
//!
//! Zero-allocation where possible — `basename`, `parent`, and `extension`
//! return borrowed slices. Only `join` allocates.

use alloc::string::String;
use alloc::format;

/// Extract the last component of a path (filename or directory name).
///
/// Trailing slashes are ignored: `basename("/usr/bin/") == "bin"`.
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

/// Extract the parent directory of a path.
///
/// Returns `"."` if no separator is present, `"/"` for root paths.
pub fn parent(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/",
        Some(i) => &trimmed[..i],
        None => ".",
    }
}

/// Extract the file extension (without the leading dot).
///
/// Hidden files (e.g. `.gitignore`) return `None`.
pub fn extension(path: &str) -> Option<&str> {
    let name = basename(path);
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    Some(&name[dot + 1..])
}

/// Join a directory and a filename with a `/` separator.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}
