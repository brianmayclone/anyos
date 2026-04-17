//! Path handling and symlink resolution helpers for the VFS.

use alloc::format;
use crate::fs::exfat::ExFatFs;
use crate::fs::file::FileType;
use alloc::string::String;
use alloc::vec::Vec;
use super::{FsError, FsType, MountPoint, ResolvedEntry};

/// Maximum depth for symlink resolution (prevents infinite loops).
pub const MAX_SYMLINK_DEPTH: u32 = 20;

/// Split a path into (parent_dir, filename).
/// "/System/hello.txt" -> ("/System", "hello.txt")
/// "/hello.txt" -> ("/", "hello.txt")
pub fn split_parent_name(path: &str) -> Result<(&str, &str), FsError> {
    let path = path.trim_end_matches('/');
    if path.is_empty() || path == "/" {
        return Err(FsError::InvalidPath);
    }
    match path.rfind('/') {
        Some(0) => Ok(("/", &path[1..])),
        Some(pos) => Ok((&path[..pos], &path[pos + 1..])),
        None => Err(FsError::InvalidPath),
    }
}

/// Check if a path targets the /dev filesystem.
pub fn is_dev_path(path: &str) -> bool {
    path == "/dev" || path.starts_with("/dev/")
}

/// Extract the device name from a /dev path (strips "/dev/" prefix).
pub fn dev_name(path: &str) -> &str {
    if path.len() > 5 { &path[5..] } else { "" }
}

/// Finde den Sub-Mount-Point, dessen Pfad ein Präfix von `path` ist (longest match).
///
/// Unterstützt beliebige Mount-Pfade außer `/` (Root-Dispatch läuft separat)
/// und `/dev` (DevFs wird über `is_dev_path` abgehandelt). Damit können sowohl
/// klassische `/mnt/*`-Mounts als auch System-Mounts wie `/System` auf einem
/// eigenen FS-Treiber liegen.
///
/// Returns `(mount_path, relative_path, fs_type)` bei Match, sonst `None`.
pub fn find_submount<'a>(
    path: &'a str,
    mount_points: &[MountPoint],
) -> Option<(&'a str, &'a str, FsType)> {
    let mut best_len = 0usize;
    let mut best_type = FsType::Fat;
    let mut found = false;
    for mount_point in mount_points {
        // Root und /dev werden über eigene Dispatch-Pfade behandelt — hier nur
        // echte Sub-Mounts betrachten, egal ob /mnt/*, /System, /Applications usw.
        if mount_point.path == "/" || mount_point.path == "/dev" {
            continue;
        }
        let mount_path = mount_point.path.as_str();
        if path == mount_path {
            if mount_path.len() > best_len {
                best_len = mount_path.len();
                best_type = mount_point.fs_type;
                found = true;
            }
        } else if path.len() > mount_path.len()
            && path.as_bytes()[mount_path.len()] == b'/'
            && path.starts_with(mount_path)
            && mount_path.len() > best_len
        {
            best_len = mount_path.len();
            best_type = mount_point.fs_type;
            found = true;
        }
    }

    if found {
        let relative = if path.len() > best_len {
            &path[best_len..]
        } else {
            "/"
        };
        Some((&path[..best_len], relative, best_type))
    } else {
        None
    }
}

/// Normalize a path by resolving `.` and `..` components.
pub fn normalize_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !parts.is_empty() {
                    parts.pop();
                }
            }
            _ => parts.push(part),
        }
    }

    if parts.is_empty() {
        String::from("/")
    } else {
        let mut result = String::new();
        for part in &parts {
            result.push('/');
            result.push_str(part);
        }
        result
    }
}

/// Resolve a path on exFAT, following symlinks at intermediate components.
/// If `follow_last` is true, also follow a symlink at the final component.
pub fn resolve_exfat_path(
    exfat: &ExFatFs,
    path: &str,
    follow_last: bool,
) -> Result<ResolvedEntry, FsError> {
    resolve_exfat_inner(exfat, path, follow_last, 0)
}

fn resolve_exfat_inner(
    exfat: &ExFatFs,
    path: &str,
    follow_last: bool,
    depth: u32,
) -> Result<ResolvedEntry, FsError> {
    if depth > MAX_SYMLINK_DEPTH {
        return Err(FsError::IoError);
    }

    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Ok(ResolvedEntry {
            inode: exfat.root_cluster(),
            file_type: FileType::Directory,
            size: 0,
            is_symlink: false,
            uid: 0,
            gid: 0,
            mode: 0xFFF,
            mtime: 0,
        });
    }

    let components: Vec<&str> = path.split('/').filter(|segment| !segment.is_empty()).collect();
    let mut current_cluster = exfat.root_cluster();

    for (idx, component) in components.iter().enumerate() {
        let is_last = idx == components.len() - 1;
        let (inode, file_type, size, is_symlink, uid, gid, mode, mtime) =
            exfat.lookup_in_dir(current_cluster, component)?;

        if is_symlink && (!is_last || follow_last) {
            let target = exfat.readlink(inode, size)?;
            let remaining = components[idx + 1..].join("/");
            let next_path = if remaining.is_empty() {
                target
            } else if target.ends_with('/') {
                format!("{}{}", target, remaining)
            } else {
                format!("{}/{}", target, remaining)
            };
            return resolve_exfat_inner(exfat, &next_path, true, depth + 1);
        }

        if !is_last && file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }

        if is_last {
            return Ok(ResolvedEntry {
                inode,
                file_type,
                size,
                is_symlink,
                uid,
                gid,
                mode,
                mtime,
            });
        }

        let (cluster, _) = crate::fs::exfat::decode_inode(inode);
        current_cluster = cluster;
    }

    Err(FsError::NotFound)
}
