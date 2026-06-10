//! Filesystem (VFS) syscall handlers.
//!
//! Covers path-based operations: readdir, stat, lstat, symlink, readlink,
//! getcwd, chdir, mkdir, unlink, truncate, rename, mount, umount.

use super::helpers::{copy_to_user_bytes, fs_err, read_user_str, resolve_path};
use alloc::string::String;

pub fn sys_readdir(path_ptr: u64, buf_ptr: u64, buf_size: u32) -> u32 {
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    // Permission check: need PERM_READ on directory
    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(&path) {
        if !crate::fs::permissions::check_permission(
            uid,
            gid,
            mode,
            crate::fs::permissions::PERM_READ,
        ) {
            return 0;
        }
    }

    match crate::fs::vfs::read_dir(&path) {
        Ok(entries) => {
            if path.ends_with("/bin") || path.ends_with("/sbin") {
                crate::serial_println!(
                    "[readdir] '{}': {} entries, buf_size={}",
                    path,
                    entries.len(),
                    buf_size
                );
            }
            let entry_size = 64usize;
            if buf_ptr != 0 && buf_size > 0 {
                let max_entries = (buf_size as usize) / entry_size;
                let written = entries.len().min(max_entries);
                if written == 0 {
                    return entries.len() as u32;
                }
                let mut out = alloc::vec![0u8; written * entry_size];
                for (i, entry) in entries.iter().enumerate().take(written) {
                    let off = i * entry_size;
                    out[off] = match entry.file_type {
                        crate::fs::file::FileType::Regular => 0,
                        crate::fs::file::FileType::Directory => 1,
                        crate::fs::file::FileType::Device => 2,
                    };
                    let name_bytes = entry.name.as_bytes();
                    let name_len = name_bytes.len().min(55);
                    out[off + 1] = name_len as u8;
                    out[off + 2] = if entry.is_symlink { 1 } else { 0 }; // flags: bit 0 = symlink
                    out[off + 3] = 0;
                    // Legacy 32-bit syscall ABI carries entry sizes as u32 —
                    // saturate values beyond 4 GiB.
                    let size = entry.size.min(u32::MAX as u64) as u32;
                    out[off + 4..off + 8].copy_from_slice(&size.to_le_bytes());
                    out[off + 8..off + 8 + name_len].copy_from_slice(&name_bytes[..name_len]);
                    out[off + 8 + name_len] = 0;
                }
                if !copy_to_user_bytes(buf_ptr, &out, buf_size as usize) {
                    return fs_err(crate::fs::vfs::FsError::InvalidPath);
                }
                // Return only the count actually written to the buffer
                written as u32
            } else {
                entries.len() as u32
            }
        }
        Err(e) => fs_err(e),
    }
}

/// Long-name directory listing.
///
/// Entry format is 264 bytes:
/// [type:u8, flags:u8, name_len:u16, size:u32, name:256bytes]
pub fn sys_readdir_long(path_ptr: u64, buf_ptr: u64, buf_size: u32) -> u32 {
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(&path) {
        if !crate::fs::permissions::check_permission(
            uid,
            gid,
            mode,
            crate::fs::permissions::PERM_READ,
        ) {
            return 0;
        }
    }

    match crate::fs::vfs::read_dir(&path) {
        Ok(entries) => {
            let entry_size = 264usize;
            if buf_ptr != 0 && buf_size > 0 {
                let max_entries = (buf_size as usize) / entry_size;
                let written = entries.len().min(max_entries);
                if written == 0 {
                    return entries.len() as u32;
                }
                let mut out = alloc::vec![0u8; written * entry_size];
                for (i, entry) in entries.iter().enumerate().take(written) {
                    let off = i * entry_size;
                    out[off] = match entry.file_type {
                        crate::fs::file::FileType::Regular => 0,
                        crate::fs::file::FileType::Directory => 1,
                        crate::fs::file::FileType::Device => 2,
                    };
                    out[off + 1] = if entry.is_symlink { 1 } else { 0 };
                    let name_bytes = entry.name.as_bytes();
                    let name_len = name_bytes.len().min(256);
                    out[off + 2..off + 4].copy_from_slice(&(name_len as u16).to_le_bytes());
                    // Legacy 32-bit syscall ABI carries entry sizes as u32 —
                    // saturate values beyond 4 GiB.
                    let size = entry.size.min(u32::MAX as u64) as u32;
                    out[off + 4..off + 8].copy_from_slice(&size.to_le_bytes());
                    out[off + 8..off + 8 + name_len].copy_from_slice(&name_bytes[..name_len]);
                    if name_len < 256 {
                        out[off + 8 + name_len] = 0;
                    }
                }
                if !copy_to_user_bytes(buf_ptr, &out, buf_size as usize) {
                    return fs_err(crate::fs::vfs::FsError::InvalidPath);
                }
                written as u32
            } else {
                entries.len() as u32
            }
        }
        Err(e) => fs_err(e),
    }
}

pub fn sys_stat(path_ptr: u64, buf_ptr: u64) -> u32 {
    let raw_path = unsafe { read_user_str(path_ptr) };
    let path = resolve_path(raw_path);

    match crate::fs::vfs::stat(&path) {
        Ok(st) => {
            if buf_ptr != 0 {
                let type_val: u32 = match st.file_type {
                    crate::fs::file::FileType::Directory => 1,
                    crate::fs::file::FileType::Device => 2,
                    _ => 0, // Regular
                };
                let flags: u32 = if st.is_symlink { 1 } else { 0 };
                // Filesystems without UID storage (FAT, NTFS, ISO) always return
                // uid=0.  Substitute the caller's real UID so that ownership
                // checks (e.g. libgit2) work correctly for non-root users.
                let caller_uid = crate::task::scheduler::current_thread_uid() as u32;
                let file_uid = if st.uid == 0 {
                    caller_uid
                } else {
                    st.uid as u32
                };
                let mut out = [0u8; 28];
                // Legacy 32-bit syscall ABI carries the size as u32 —
                // saturate values beyond 4 GiB.
                let size32 = st.size.min(u32::MAX as u64) as u32;
                out[0..4].copy_from_slice(&type_val.to_le_bytes());
                out[4..8].copy_from_slice(&size32.to_le_bytes());
                out[8..12].copy_from_slice(&flags.to_le_bytes());
                out[12..16].copy_from_slice(&file_uid.to_le_bytes());
                out[16..20].copy_from_slice(&(st.gid as u32).to_le_bytes());
                out[20..24].copy_from_slice(&(st.mode as u32).to_le_bytes());
                out[24..28].copy_from_slice(&st.mtime.to_le_bytes());
                if !copy_to_user_bytes(buf_ptr, &out, 28) {
                    return fs_err(crate::fs::vfs::FsError::InvalidPath);
                }
            }
            0
        }
        Err(e) => fs_err(e),
    }
}

pub fn sys_lstat(path_ptr: u64, buf_ptr: u64) -> u32 {
    let raw_path = unsafe { read_user_str(path_ptr) };
    let path = resolve_path(raw_path);

    match crate::fs::vfs::lstat(&path) {
        Ok(st) => {
            if buf_ptr != 0 {
                let type_val: u32 = match st.file_type {
                    crate::fs::file::FileType::Directory => 1,
                    crate::fs::file::FileType::Device => 2,
                    _ => 0, // Regular
                };
                let flags: u32 = if st.is_symlink { 1 } else { 0 };
                let caller_uid = crate::task::scheduler::current_thread_uid() as u32;
                let file_uid = if st.uid == 0 {
                    caller_uid
                } else {
                    st.uid as u32
                };
                let mut out = [0u8; 28];
                // Legacy 32-bit syscall ABI carries the size as u32 —
                // saturate values beyond 4 GiB.
                let size32 = st.size.min(u32::MAX as u64) as u32;
                out[0..4].copy_from_slice(&type_val.to_le_bytes());
                out[4..8].copy_from_slice(&size32.to_le_bytes());
                out[8..12].copy_from_slice(&flags.to_le_bytes());
                out[12..16].copy_from_slice(&file_uid.to_le_bytes());
                out[16..20].copy_from_slice(&(st.gid as u32).to_le_bytes());
                out[20..24].copy_from_slice(&(st.mode as u32).to_le_bytes());
                out[24..28].copy_from_slice(&st.mtime.to_le_bytes());
                if !copy_to_user_bytes(buf_ptr, &out, 28) {
                    return fs_err(crate::fs::vfs::FsError::InvalidPath);
                }
            }
            0
        }
        Err(e) => fs_err(e),
    }
}

pub fn sys_symlink(target_ptr: u64, link_path_ptr: u64) -> u32 {
    let target = unsafe { read_user_str(target_ptr) };
    let raw_link = unsafe { read_user_str(link_path_ptr) };
    let link_path = resolve_path(raw_link);

    match crate::fs::vfs::create_symlink(&link_path, target) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_readlink(path_ptr: u64, buf_ptr: u64, buf_size: u32) -> u32 {
    let raw_path = unsafe { read_user_str(path_ptr) };
    let path = resolve_path(raw_path);

    match crate::fs::vfs::readlink(&path) {
        Ok(target) => {
            let target_bytes = target.as_bytes();
            let to_copy = target_bytes.len().min(buf_size as usize);
            if buf_ptr != 0 && to_copy > 0 {
                // Include the NUL terminator in the single validated copy if there
                // is room left in the user buffer (matches the original behavior:
                // terminator written only when to_copy < buf_size).
                let total = if to_copy < buf_size as usize {
                    to_copy + 1
                } else {
                    to_copy
                };
                let mut out = alloc::vec![0u8; total];
                out[..to_copy].copy_from_slice(&target_bytes[..to_copy]);
                if total > to_copy {
                    out[to_copy] = 0;
                }
                if !copy_to_user_bytes(buf_ptr, &out, buf_size as usize) {
                    return fs_err(crate::fs::vfs::FsError::InvalidPath);
                }
            }
            to_copy as u32
        }
        Err(e) => fs_err(e),
    }
}

pub fn sys_getcwd(buf_ptr: u64, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size == 0 {
        return u32::MAX;
    }
    // Fill a kernel-local buffer first, then copy out via a mapping-validated
    // copy. Bound the temp allocation so a huge user-supplied buf_size cannot
    // trigger an unbounded kernel allocation.
    let cap = (buf_size as usize).min(4096);
    let mut tmp = alloc::vec![0u8; cap];
    let len = crate::task::scheduler::current_thread_cwd(&mut tmp);
    if len == 0 {
        // Fallback
        if buf_size >= 2 && cap >= 2 {
            tmp[0] = b'/';
            tmp[1] = 0;
            if !copy_to_user_bytes(buf_ptr, &tmp[..2], buf_size as usize) {
                return u32::MAX;
            }
        }
        return 1;
    }
    let nul = len.min(cap.saturating_sub(1));
    tmp[nul] = 0;
    if !copy_to_user_bytes(buf_ptr, &tmp[..=nul], buf_size as usize) {
        return u32::MAX;
    }
    nul as u32
}

pub fn sys_chdir(path_ptr: u64) -> u32 {
    if path_ptr == 0 {
        return u32::MAX;
    }
    let raw_path = unsafe { read_user_str(path_ptr) };
    let path = resolve_path(raw_path);
    // Verify the directory exists
    match crate::fs::vfs::read_dir(&path) {
        Ok(_) => {
            let tid = crate::task::scheduler::current_tid();
            crate::task::scheduler::set_thread_cwd(tid, &path);
            0
        }
        Err(e) => fs_err(e),
    }
}

pub fn sys_mkdir(path_ptr: u64) -> u32 {
    if path_ptr == 0 {
        return u32::MAX;
    }
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    // Permission check: need PERM_CREATE on parent directory
    if let Some(parent_end) = path.rfind('/') {
        let parent = if parent_end == 0 {
            "/"
        } else {
            &path[..parent_end]
        };
        if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(parent) {
            if !crate::fs::permissions::check_permission(
                uid,
                gid,
                mode,
                crate::fs::permissions::PERM_CREATE,
            ) {
                return u32::MAX;
            }
        }
    }

    match crate::fs::vfs::mkdir(&path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_unlink(path_ptr: u64) -> u32 {
    if path_ptr == 0 {
        return u32::MAX;
    }
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    // Permission check: need PERM_DELETE on parent directory
    if let Some(parent_end) = path.rfind('/') {
        let parent = if parent_end == 0 {
            "/"
        } else {
            &path[..parent_end]
        };
        if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(parent) {
            if !crate::fs::permissions::check_permission(
                uid,
                gid,
                mode,
                crate::fs::permissions::PERM_DELETE,
            ) {
                return u32::MAX;
            }
        }
    }

    match crate::fs::vfs::delete(&path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_truncate(path_ptr: u64) -> u32 {
    if path_ptr == 0 {
        return u32::MAX;
    }
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    // Permission check: need PERM_MODIFY on the file
    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(&path) {
        if !crate::fs::permissions::check_permission(
            uid,
            gid,
            mode,
            crate::fs::permissions::PERM_MODIFY,
        ) {
            return u32::MAX;
        }
    }

    match crate::fs::vfs::truncate(&path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_rename(old_ptr: u64, new_ptr: u64) -> u32 {
    if old_ptr == 0 || new_ptr == 0 {
        return u32::MAX;
    }
    let old_path = resolve_path(unsafe { read_user_str(old_ptr) });
    let new_path = resolve_path(unsafe { read_user_str(new_ptr) });
    match crate::fs::vfs::rename(&old_path, &new_path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_mount(mount_path_ptr: u64, device_path_ptr: u64, fs_type: u32) -> u32 {
    if mount_path_ptr == 0 {
        return u32::MAX;
    }
    let mount_path = resolve_path(unsafe { read_user_str(mount_path_ptr) });
    let device_path = if device_path_ptr != 0 {
        String::from(unsafe { read_user_str(device_path_ptr) })
    } else {
        String::new()
    };
    match crate::fs::vfs::mount_fs(&mount_path, &device_path, fs_type) {
        Ok(()) => {
            // Emit volume mounted event
            crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
                crate::ipc::event_bus::EVT_VOLUME_MOUNTED,
                fs_type,
                0,
                0,
                0,
            ));
            0
        }
        Err(e) => fs_err(e),
    }
}

/// sys_umount - Unmount a filesystem.
/// arg1=mount_path_ptr
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_umount(mount_path_ptr: u64) -> u32 {
    if mount_path_ptr == 0 {
        return u32::MAX;
    }
    let mount_path = resolve_path(unsafe { read_user_str(mount_path_ptr) });
    match crate::fs::vfs::umount_fs(&mount_path) {
        Ok(()) => {
            // Emit volume unmounted event
            crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
                crate::ipc::event_bus::EVT_VOLUME_UNMOUNTED,
                0,
                0,
                0,
                0,
            ));
            0
        }
        Err(e) => fs_err(e),
    }
}

/// Append a decimal u32 to `out`.
fn push_u32(out: &mut String, mut n: u32) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut digits = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        out.push(digits[i] as char);
    }
}

/// sys_list_mounts - List all mount points.
/// arg1=buf_ptr: output buffer
/// arg2=buf_len: buffer capacity
/// Returns number of bytes written, or u32::MAX on error.
///
/// Output format: "mount_path\tfs_type\tdev_id\n" for each mount, null-terminated.
/// `dev_id` is the decimal block-device ID (same ID space as SYS_DISK_LIST). For
/// pseudo-mounts without a backing block device (smb, fuse, devfs), it is 0.
pub fn sys_list_mounts(buf_ptr: u64, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 {
        return u32::MAX;
    }

    let mounts = crate::fs::vfs::list_mounts();
    let mut output = String::new();
    for (path, fs_type, dev_id) in &mounts {
        output.push_str(path);
        output.push('\t');
        output.push_str(fs_type);
        output.push('\t');
        push_u32(&mut output, *dev_id);
        output.push('\n');
    }

    let bytes = output.as_bytes();
    let to_copy = bytes.len().min(buf_len as usize - 1);
    let mut out = alloc::vec::Vec::with_capacity(to_copy + 1);
    out.extend_from_slice(&bytes[..to_copy]);
    out.push(0); // null-terminate
    if !copy_to_user_bytes(buf_ptr, &out, buf_len as usize) {
        return u32::MAX;
    }
    to_copy as u32
}

/// sys_statfs - Get filesystem statistics for a mount point.
/// arg1=path_ptr (null-terminated string)
/// arg2=unused
/// arg3=buf_ptr (output: 3 x u64 LE: total_bytes, used_bytes, free_bytes = 24 bytes)
/// Returns 0 on success, u32::MAX on error.
pub fn sys_statfs(path_ptr: u64, _path_len: u32, buf_ptr: u64) -> u32 {
    if path_ptr == 0 || buf_ptr == 0 {
        return u32::MAX;
    }

    let path = unsafe { read_user_str(path_ptr) };
    if path.is_empty() {
        return u32::MAX;
    }
    let path = resolve_path(path);

    match crate::fs::vfs::statfs(&path) {
        Some(st) => {
            let mut out = [0u8; 24];
            out[0..8].copy_from_slice(&st.total_bytes.to_le_bytes());
            out[8..16].copy_from_slice(&st.used_bytes.to_le_bytes());
            out[16..24].copy_from_slice(&st.free_bytes.to_le_bytes());
            if !copy_to_user_bytes(buf_ptr, &out, 24) {
                return u32::MAX;
            }
            0
        }
        None => u32::MAX,
    }
}
