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
fn ensure_perm_dir(uid: u16) {
    // Create each level; errors are OK (AlreadyExists is expected).
    let _ = vfs::mkdir("/System/users");
    let _ = vfs::mkdir("/System/users/perm");
    let dir = format!("/System/users/perm/{}", uid);
    let _ = vfs::mkdir(&dir);
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
    };
    match vfs::open(&path, flags) {
        Ok(fd) => {
            let _ = vfs::write(fd, content.as_bytes());
            let _ = vfs::close(fd);
            true
        }
        Err(e) => {
            crate::serial_println!("PERM: write_stored_perms failed: path='{}' err={:?}", path, e);
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
