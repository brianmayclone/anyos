//! Filesystem (VFS) syscall handlers.
//!
//! Covers path-based operations: readdir, stat, lstat, symlink, readlink,
//! getcwd, chdir, mkdir, unlink, truncate, rename, mount, umount.

use alloc::string::String;
use super::helpers::{fs_err, is_valid_user_ptr, read_user_str, resolve_path};

pub fn sys_readdir(path_ptr: u32, buf_ptr: u32, buf_size: u32) -> u32 {
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    // Permission check: need PERM_READ on directory
    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(&path) {
        if !crate::fs::permissions::check_permission(uid, gid, mode, crate::fs::permissions::PERM_READ) {
            return 0;
        }
    }

    match crate::fs::vfs::read_dir(&path) {
        Ok(entries) => {
            let entry_size = 64u32;
            if buf_ptr != 0 && buf_size > 0
                && is_valid_user_ptr(buf_ptr as u64, buf_size as u64)
            {
                let max_entries = (buf_size / entry_size) as usize;
                let buf = unsafe {
                    core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize)
                };
                for (i, entry) in entries.iter().enumerate().take(max_entries) {
                    let off = i * entry_size as usize;
                    buf[off] = match entry.file_type {
                        crate::fs::file::FileType::Regular => 0,
                        crate::fs::file::FileType::Directory => 1,
                        crate::fs::file::FileType::Device => 2,
                    };
                    let name_bytes = entry.name.as_bytes();
                    let name_len = name_bytes.len().min(55);
                    buf[off + 1] = name_len as u8;
                    buf[off + 2] = if entry.is_symlink { 1 } else { 0 }; // flags: bit 0 = symlink
                    buf[off + 3] = 0;
                    let size = entry.size as u32;
                    buf[off + 4..off + 8].copy_from_slice(&size.to_le_bytes());
                    buf[off + 8..off + 8 + name_len].copy_from_slice(&name_bytes[..name_len]);
                    buf[off + 8 + name_len] = 0;
                }
            }
            entries.len() as u32
        }
        Err(e) => fs_err(e),
    }
}

pub fn sys_stat(path_ptr: u32, buf_ptr: u32) -> u32 {
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
                let file_uid = if st.uid == 0 { caller_uid } else { st.uid as u32 };
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    *buf = type_val;
                    *buf.add(1) = st.size;
                    *buf.add(2) = flags;
                    *buf.add(3) = file_uid;
                    *buf.add(4) = st.gid as u32;
                    *buf.add(5) = st.mode as u32;
                    *buf.add(6) = st.mtime;
                }
            }
            0
        }
        Err(e) => fs_err(e),
    }
}

pub fn sys_lstat(path_ptr: u32, buf_ptr: u32) -> u32 {
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
                let file_uid = if st.uid == 0 { caller_uid } else { st.uid as u32 };
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    *buf = type_val;
                    *buf.add(1) = st.size;
                    *buf.add(2) = flags;
                    *buf.add(3) = file_uid;
                    *buf.add(4) = st.gid as u32;
                    *buf.add(5) = st.mode as u32;
                    *buf.add(6) = st.mtime;
                }
            }
            0
        }
        Err(e) => fs_err(e),
    }
}

pub fn sys_symlink(target_ptr: u32, link_path_ptr: u32) -> u32 {
    let target = unsafe { read_user_str(target_ptr) };
    let raw_link = unsafe { read_user_str(link_path_ptr) };
    let link_path = resolve_path(raw_link);

    match crate::fs::vfs::create_symlink(&link_path, target) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_readlink(path_ptr: u32, buf_ptr: u32, buf_size: u32) -> u32 {
    let raw_path = unsafe { read_user_str(path_ptr) };
    let path = resolve_path(raw_path);

    match crate::fs::vfs::readlink(&path) {
        Ok(target) => {
            let target_bytes = target.as_bytes();
            let to_copy = target_bytes.len().min(buf_size as usize);
            if buf_ptr != 0 && to_copy > 0
                && is_valid_user_ptr(buf_ptr as u64, buf_size as u64)
            {
                let buf = unsafe {
                    core::slice::from_raw_parts_mut(buf_ptr as *mut u8, to_copy)
                };
                buf[..to_copy].copy_from_slice(&target_bytes[..to_copy]);
                // Null-terminate if space
                if to_copy < buf_size as usize {
                    unsafe { *((buf_ptr as *mut u8).add(to_copy)) = 0; }
                }
            }
            to_copy as u32
        }
        Err(e) => fs_err(e),
    }
}

pub fn sys_getcwd(buf_ptr: u32, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        return u32::MAX;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
    let len = crate::task::scheduler::current_thread_cwd(buf);
    if len == 0 {
        // Fallback
        if buf_size >= 2 {
            buf[0] = b'/';
            buf[1] = 0;
        }
        return 1;
    }
    len as u32
}

pub fn sys_chdir(path_ptr: u32) -> u32 {
    if path_ptr == 0 { return u32::MAX; }
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

pub fn sys_mkdir(path_ptr: u32) -> u32 {
    if path_ptr == 0 { return u32::MAX; }
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    // Permission check: need PERM_CREATE on parent directory
    if let Some(parent_end) = path.rfind('/') {
        let parent = if parent_end == 0 { "/" } else { &path[..parent_end] };
        if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(parent) {
            if !crate::fs::permissions::check_permission(uid, gid, mode, crate::fs::permissions::PERM_CREATE) {
                return u32::MAX;
            }
        }
    }

    match crate::fs::vfs::mkdir(&path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_unlink(path_ptr: u32) -> u32 {
    if path_ptr == 0 { return u32::MAX; }
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    // Permission check: need PERM_DELETE on parent directory
    if let Some(parent_end) = path.rfind('/') {
        let parent = if parent_end == 0 { "/" } else { &path[..parent_end] };
        if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(parent) {
            if !crate::fs::permissions::check_permission(uid, gid, mode, crate::fs::permissions::PERM_DELETE) {
                return u32::MAX;
            }
        }
    }

    match crate::fs::vfs::delete(&path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_truncate(path_ptr: u32) -> u32 {
    if path_ptr == 0 { return u32::MAX; }
    let path = resolve_path(unsafe { read_user_str(path_ptr) });

    // Permission check: need PERM_MODIFY on the file
    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(&path) {
        if !crate::fs::permissions::check_permission(uid, gid, mode, crate::fs::permissions::PERM_MODIFY) {
            return u32::MAX;
        }
    }

    match crate::fs::vfs::truncate(&path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_rename(old_ptr: u32, new_ptr: u32) -> u32 {
    if old_ptr == 0 || new_ptr == 0 { return u32::MAX; }
    let old_path = resolve_path(unsafe { read_user_str(old_ptr) });
    let new_path = resolve_path(unsafe { read_user_str(new_ptr) });
    match crate::fs::vfs::rename(&old_path, &new_path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

pub fn sys_mount(mount_path_ptr: u32, device_path_ptr: u32, fs_type: u32) -> u32 {
    if mount_path_ptr == 0 { return u32::MAX; }
    let mount_path = resolve_path(unsafe { read_user_str(mount_path_ptr) });
    let device_path = if device_path_ptr != 0 {
        String::from(unsafe { read_user_str(device_path_ptr) })
    } else {
        String::new()
    };
    match crate::fs::vfs::mount_fs(&mount_path, &device_path, fs_type) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

/// sys_umount - Unmount a filesystem.
/// arg1=mount_path_ptr
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_umount(mount_path_ptr: u32) -> u32 {
    if mount_path_ptr == 0 { return u32::MAX; }
    let mount_path = resolve_path(unsafe { read_user_str(mount_path_ptr) });
    match crate::fs::vfs::umount_fs(&mount_path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

/// sys_list_mounts - List all mount points.
/// arg1=buf_ptr: output buffer
/// arg2=buf_len: buffer capacity
/// Returns number of bytes written, or u32::MAX on error.
///
/// Output format: "mount_path\tfs_type\n" for each mount, null-terminated.
pub fn sys_list_mounts(buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 { return u32::MAX; }
    if !is_valid_user_ptr(buf_ptr as u64, buf_len as u64) { return u32::MAX; }

    let mounts = crate::fs::vfs::list_mounts();
    let mut output = String::new();
    for (path, fs_type, _dev_id) in &mounts {
        output.push_str(path);
        output.push('\t');
        output.push_str(fs_type);
        output.push('\n');
    }

    let bytes = output.as_bytes();
    let to_copy = bytes.len().min(buf_len as usize - 1);
    unsafe {
        let dst = buf_ptr as *mut u8;
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, to_copy);
        *dst.add(to_copy) = 0; // null-terminate
    }
    to_copy as u32
}
