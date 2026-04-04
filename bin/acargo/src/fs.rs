//! File system helpers for acargo.
//! Wraps anyos_std::fs with higher-level operations.

use crate::prelude::*;

/// Read an entire file to a String.
pub fn read_file(path: &str) -> Option<String> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);
    String::from_utf8(data).ok()
}

/// Read an entire file to bytes.
pub fn read_file_bytes(path: &str) -> Option<Vec<u8>> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);
    Some(data)
}

/// Write data to a file, creating or truncating it.
pub fn write_file(path: &str, data: &[u8]) {
    let fd = anyos_std::fs::open(
        path,
        anyos_std::fs::O_WRITE | anyos_std::fs::O_CREATE | anyos_std::fs::O_TRUNC,
    );
    if fd == u32::MAX {
        anyos_std::println!("acargo: error: cannot write {}", path);
        return;
    }
    anyos_std::fs::write(fd, data);
    anyos_std::fs::close(fd);
}

/// Check if a file exists.
pub fn file_exists(path: &str) -> bool {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        false
    } else {
        anyos_std::fs::close(fd);
        true
    }
}

/// Check if a directory exists.
pub fn dir_exists(path: &str) -> bool {
    let mut buf = [0u8; 64];
    let count = anyos_std::fs::readdir(path, &mut buf);
    count != u32::MAX
}

/// Create a directory (and parents if needed).
pub fn mkdir_p(path: &str) {
    // Create parent directories iteratively
    let mut current = String::new();
    for part in path.split('/') {
        if part.is_empty() {
            current.push('/');
            continue;
        }
        if !current.is_empty() && !current.ends_with('/') {
            current.push('/');
        }
        current.push_str(part);
        anyos_std::fs::mkdir(&current);
    }
}

/// Remove a directory and all its contents recursively.
pub fn rm_rf(path: &str) {
    // Read directory entries and remove them
    let mut buf = [0u8; 64 * 128]; // up to 128 entries
    let count = anyos_std::fs::readdir(path, &mut buf);
    if count == u32::MAX {
        // Not a directory or doesn't exist, try as file
        anyos_std::fs::unlink(path);
        return;
    }

    let entry_size = 64;
    for i in 0..count as usize {
        let off = i * entry_size;
        let file_type = buf[off]; // 0=file, 1=dir
        let name_len = buf[off + 1] as usize;
        if name_len == 0 { continue; }
        let name_bytes = &buf[off + 8..off + 8 + name_len];
        if let Ok(name) = core::str::from_utf8(name_bytes) {
            if name == "." || name == ".." { continue; }
            let child = format!("{}/{}", path, name);
            if file_type == 1 {
                rm_rf(&child);
            } else {
                anyos_std::fs::unlink(&child);
            }
        }
    }
    // Remove the now-empty directory itself
    // Use unlink on the dir path (kernel handles rmdir via unlink for directories)
    anyos_std::fs::unlink(path);
}

/// Resolve a relative path against a base directory.
/// Handles ".." and "." components.
pub fn resolve_path(base: &str, relative: &str) -> String {
    if relative.starts_with('/') {
        return String::from(relative);
    }

    let mut components: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();

    for part in relative.split('/') {
        match part {
            ".." => { components.pop(); }
            "." | "" => {}
            other => components.push(other),
        }
    }

    if base.starts_with('/') {
        format!("/{}", components.join("/"))
    } else {
        components.join("/").into()
    }
}
