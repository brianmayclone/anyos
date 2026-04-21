//! Per-user, per-app runtime permission storage.
//!
//! Permission files are stored at `/System/users/perm/{uid}/{app_id}` and contain
//! a granted capability bitmask. The kernel creates directories as needed.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::task::capabilities::CapSet;
use crate::fs::vfs;
use crate::fs::file::FileFlags;

/// Build the filesystem path for a given uid and app_id.
fn perm_path(uid: u16, app_id: &str) -> String {
    format!("/System/users/perm/{}/{}", uid, app_id)
}

/// Ensure the directory `/System/users/perm/{uid}/` exists, creating
/// intermediate directories as needed.
///
/// Each level is created explicitly and errors are swallowed only for
/// AlreadyExists — a missing parent (e.g. a read-only / not-yet-mounted
/// `/System`) is logged so the downstream `write_stored_perms` NotFound
/// failure has an obvious breadcrumb instead of failing silently.
fn ensure_perm_dir(uid: u16) {
    for dir in [
        "/System/users",
        "/System/users/perm",
    ] {
        match vfs::mkdir(dir) {
            Ok(_) => {}
            Err(crate::fs::vfs::FsError::AlreadyExists) => {}
            Err(e) => {
                crate::serial_println!(
                    "PERM: ensure_perm_dir: mkdir('{}') failed: {:?}", dir, e
                );
            }
        }
    }
    let dir = format!("/System/users/perm/{}", uid);
    match vfs::mkdir(&dir) {
        Ok(_) => {}
        Err(crate::fs::vfs::FsError::AlreadyExists) => {}
        Err(e) => {
            crate::serial_println!(
                "PERM: ensure_perm_dir: mkdir('{}') failed: {:?}", dir, e
            );
        }
    }
}

/// Read stored granted capabilities for (uid, app_id).
/// Returns `Some(granted_caps)` if the permission file exists and parses
/// correctly, `None` otherwise.
pub fn read_stored_perms(uid: u16, app_id: &str) -> Option<CapSet> {
    let path = perm_path(uid, app_id);
    let data = vfs::read_file_to_vec(&path).ok()?;
    let text = core::str::from_utf8(&data).ok()?;
    for line in text.split('\n') {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("granted=0x") {
            return u32::from_str_radix(rest, 16).ok();
        }
        if let Some(rest) = line.strip_prefix("granted=") {
            return rest.parse::<u32>().ok();
        }
    }
    None
}

/// Write granted capabilities for (uid, app_id). Creates directories as needed.
/// Returns `true` on success.
pub fn write_stored_perms(uid: u16, app_id: &str, granted: CapSet) -> bool {
    ensure_perm_dir(uid);
    let path = perm_path(uid, app_id);
    let content = format!("granted=0x{:x}\n", granted);
    let flags = FileFlags {
        read: true,
        write: true,
        append: false,
        create: true,
        truncate: true,
        sync: false,
    };
    match vfs::open(&path, flags) {
        Ok(fd) => {
            let wrote = vfs::write(fd, content.as_bytes());
            let closed = vfs::close(fd);
            match (wrote, closed) {
                (Ok(n), Ok(())) if n == content.len() => true,
                // Short write or close error — surface as a warning so a
                // half-written permission file doesn't silently cause sys_spawn
                // to loop back into the permdialog forever.
                (Ok(n), Ok(())) => {
                    crate::serial_println!(
                        "PERM: short write path='{}' wrote={} expected={}",
                        path, n, content.len()
                    );
                    false
                }
                (Err(e), _) => {
                    crate::serial_println!(
                        "PERM: write failed path='{}' err={:?}", path, e
                    );
                    false
                }
                (Ok(_), Err(e)) => {
                    crate::serial_println!(
                        "PERM: close failed path='{}' err={:?}", path, e
                    );
                    false
                }
            }
        }
        Err(e) => {
            crate::serial_println!("PERM: open failed path='{}' err={:?}", path, e);
            false
        }
    }
}

/// Delete stored permissions for (uid, app_id). Returns `true` if the file was deleted.
pub fn delete_stored_perms(uid: u16, app_id: &str) -> bool {
    let path = perm_path(uid, app_id);
    vfs::delete(&path).is_ok()
}

/// List all app IDs with stored permissions for a given uid.
/// Returns a vector of `(app_id, granted_caps)` pairs.
pub fn list_apps_with_perms(uid: u16) -> Vec<(String, CapSet)> {
    let dir = format!("/System/users/perm/{}", uid);
    let entries = match vfs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    for entry in &entries {
        if entry.file_type == crate::fs::file::FileType::Regular {
            let app_id = &entry.name;
            if let Some(granted) = read_stored_perms(uid, app_id) {
                result.push((app_id.clone(), granted));
            }
        }
    }
    result
}
