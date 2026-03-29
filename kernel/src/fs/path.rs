//! Path manipulation utilities -- normalize, parent, and filename extraction.

use alloc::string::String;
use alloc::vec::Vec;

/// Normalize a path by resolving `.` and `..` components.
/// Returns an absolute path if the input was absolute, otherwise relative.
///
/// Fast path: if the path contains no `.` or `..`, avoids Vec allocation entirely.
pub fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');

    // Fast path: simple paths without . or .. (common case ~90% of opens)
    if !path.contains("/.") && !path.contains("//") {
        // No normalization needed — just trim trailing slash
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            return String::from("/");
        }
        return String::from(trimmed);
    }

    // Slow path: resolve . and .. with Vec
    let mut components: Vec<&str> = Vec::new();

    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }

    let mut result = String::new();
    if absolute {
        result.push('/');
    }
    for (i, comp) in components.iter().enumerate() {
        if i > 0 {
            result.push('/');
        }
        result.push_str(comp);
    }

    if result.is_empty() {
        String::from("/")
    } else {
        result
    }
}

/// Get the parent directory of a path
pub fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(pos) => &path[..pos],
        None => ".",
    }
}

/// Get the file name from a path
pub fn filename(path: &str) -> &str {
    match path.rfind('/') {
        Some(pos) => &path[pos + 1..],
        None => path,
    }
}
