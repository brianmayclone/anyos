//! User identity, management, and permission syscall handlers.
//!
//! Covers user/group CRUD, authentication, file ownership (chmod/chown),
//! capability queries, and application permission management.

use super::helpers::{fs_err, is_valid_user_ptr, read_user_str_safe, resolve_path};

/// SYS_GET_CAPABILITIES: Return the calling thread's capability bitmask.
pub fn sys_get_capabilities() -> u32 {
    crate::task::scheduler::current_thread_capabilities()
}

// =========================================================================
// User identity & management
// =========================================================================

/// SYS_GETUID: Return the calling thread's user ID.
pub fn sys_getuid() -> u32 {
    crate::task::scheduler::current_thread_uid() as u32
}

/// SYS_GETGID: Return the calling thread's group ID.
pub fn sys_getgid() -> u32 {
    crate::task::scheduler::current_thread_gid() as u32
}

/// SYS_AUTHENTICATE: Verify credentials and set uid/gid on the calling process.
/// arg1 = username_ptr (null-terminated), arg2 = password_ptr (null-terminated).
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_authenticate(username_ptr: u32, password_ptr: u32) -> u32 {
    let username = match read_user_str_safe(username_ptr) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let password = match read_user_str_safe(password_ptr) {
        Some(s) => s,
        None => return u32::MAX,
    };

    match crate::task::users::authenticate(username, password) {
        Some((uid, gid)) => {
            let tid = crate::task::scheduler::current_tid();
            crate::task::scheduler::set_process_identity(tid, uid, gid);
            crate::serial_println!("  AUTH: T{} authenticated as {}(uid={}, gid={})", tid, username, uid, gid);
            0
        }
        None => {
            crate::serial_println!("  AUTH: Failed authentication for '{}'", username);
            u32::MAX
        }
    }
}

/// SYS_CHMOD: Change file permission mode.
/// arg1 = path_ptr, arg2 = new_mode (u16).
/// Only owner or root can change permissions.
pub fn sys_chmod(path_ptr: u32, mode: u32) -> u32 {
    let path = match read_user_str_safe(path_ptr) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let resolved = resolve_path(path);
    let mode = mode as u16;

    // Check ownership: must be owner or root
    let thread_uid = crate::task::scheduler::current_thread_uid();
    if thread_uid != 0 {
        if let Ok(perms) = crate::fs::vfs::get_permissions(&resolved) {
            if perms.0 != thread_uid {
                return u32::MAX; // Not owner and not root
            }
        }
    }

    match crate::fs::vfs::set_mode(&resolved, mode) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

/// SYS_CHOWN: Change file owner/group. Root only.
/// arg1 = path_ptr, arg2 = new_uid (u16), arg3 = new_gid (u16).
pub fn sys_chown(path_ptr: u32, uid: u32, gid: u32) -> u32 {
    // Root-only check
    if crate::task::scheduler::current_thread_uid() != 0 {
        return u32::MAX;
    }
    let path = match read_user_str_safe(path_ptr) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let resolved = resolve_path(path);

    match crate::fs::vfs::set_owner(&resolved, uid as u16, gid as u16) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

/// SYS_ADDUSER: Add a user. Root only.
/// arg1 = ptr to packed struct: [username_ptr: u32, password_ptr: u32, fullname_ptr: u32, homedir_ptr: u32]
pub fn sys_adduser(data_ptr: u32) -> u32 {
    // Root-only check
    if crate::task::scheduler::current_thread_uid() != 0 {
        return u32::MAX;
    }
    if !is_valid_user_ptr(data_ptr as u64, 32) {
        return u32::MAX;
    }

    // Stdlib packs 4 pointers as [u64; 4] (x86_64 pointer size)
    let ptrs = unsafe { core::slice::from_raw_parts(data_ptr as *const u64, 4) };
    let username = match read_user_str_safe(ptrs[0] as u32) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let password = match read_user_str_safe(ptrs[1] as u32) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let fullname = match read_user_str_safe(ptrs[2] as u32) {
        Some(s) if !s.is_empty() => s,
        _ => username,
    };
    let homedir_default = alloc::format!("/Users/{}", username);
    let homedir = match read_user_str_safe(ptrs[3] as u32) {
        Some(s) if !s.is_empty() => s,
        _ => &homedir_default,
    };

    // Hash password
    let hash = if password.is_empty() {
        [0u8; 32] // empty hash = no password
    } else {
        crate::crypto::md5::md5_hex(password.as_bytes())
    };
    let hash_str = core::str::from_utf8(&hash).unwrap_or("");

    let uid = crate::task::users::next_uid();
    let gid = uid; // Default: same as uid

    if crate::task::users::add_user(username, hash_str, uid, gid, fullname, homedir) {
        // Create home directory
        let _ = crate::fs::vfs::mkdir(homedir);
        let docs = alloc::format!("{}/Documents", homedir);
        let _ = crate::fs::vfs::mkdir(&docs);

        // Set ownership on home directory
        let _ = crate::fs::vfs::set_owner(homedir, uid, gid);
        let _ = crate::fs::vfs::set_mode(homedir, 0xFF0); // owner+group full, others none

        crate::serial_println!("  ADDUSER: Created user '{}' uid={} gid={} home={}", username, uid, gid, homedir);
        uid as u32
    } else {
        u32::MAX
    }
}

/// SYS_DELUSER: Remove a user by UID. Root only.
pub fn sys_deluser(uid: u32) -> u32 {
    if crate::task::scheduler::current_thread_uid() != 0 {
        return u32::MAX;
    }
    if crate::task::users::remove_user(uid as u16) { 0 } else { u32::MAX }
}

/// SYS_CHPASSWD: Change a user's password.
/// data_ptr -> [username_ptr: u64, old_password_ptr: u64, new_password_ptr: u64]
/// Root can change any password (old_password ignored).
/// Non-root must provide correct old_password and can only change own password.
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_chpasswd(data_ptr: u32) -> u32 {
    if !is_valid_user_ptr(data_ptr as u64, 24) {
        return u32::MAX;
    }

    let ptrs = unsafe { core::slice::from_raw_parts(data_ptr as *const u64, 3) };
    let username = match read_user_str_safe(ptrs[0] as u32) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let old_password = match read_user_str_safe(ptrs[1] as u32) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let new_password = match read_user_str_safe(ptrs[2] as u32) {
        Some(s) => s,
        None => return u32::MAX,
    };

    let caller_uid = crate::task::scheduler::current_thread_uid();
    let is_root = caller_uid == 0;

    if !is_root {
        // Non-root: verify the target is the caller's own account
        let target = match crate::task::users::lookup_username(username) {
            Some(u) => u,
            None => return u32::MAX,
        };
        if target.uid != caller_uid {
            return u32::MAX; // can only change own password
        }
        // Verify old password
        if crate::task::users::authenticate(username, old_password).is_none() {
            return u32::MAX;
        }
    }

    // Hash new password
    let hash = if new_password.is_empty() {
        [0u8; 32]
    } else {
        crate::crypto::md5::md5_hex(new_password.as_bytes())
    };
    let hash_str = core::str::from_utf8(&hash).unwrap_or("");

    crate::serial_println!("  CHPASSWD: user='{}' new_pass='{}' hash='{}'", username, new_password, hash_str);

    if crate::task::users::change_password(username, hash_str) {
        crate::serial_println!("  CHPASSWD: Password changed for '{}'", username);
        0
    } else {
        u32::MAX
    }
}

/// SYS_LISTUSERS: List all users into a buffer.
/// arg1 = buf_ptr, arg2 = buf_len. Returns bytes written.
pub fn sys_listusers(buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_len as u64) {
        return 0;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize) };
    crate::task::users::list_users(buf) as u32
}

/// SYS_ADDGROUP: Add a group. Root only.
/// arg1 = ptr to packed struct: [name_ptr: u32, gid: u32]
pub fn sys_addgroup(data_ptr: u32) -> u32 {
    if crate::task::scheduler::current_thread_uid() != 0 {
        return u32::MAX;
    }
    if !is_valid_user_ptr(data_ptr as u64, 16) {
        return u32::MAX;
    }
    // Stdlib packs [name_ptr: u64, gid: u64]
    let ptrs = unsafe { core::slice::from_raw_parts(data_ptr as *const u64, 2) };
    let name = match read_user_str_safe(ptrs[0] as u32) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let gid = ptrs[1] as u16;
    if crate::task::users::add_group(name, gid) { 0 } else { u32::MAX }
}

/// SYS_DELGROUP: Remove a group by GID. Root only.
pub fn sys_delgroup(gid: u32) -> u32 {
    if crate::task::scheduler::current_thread_uid() != 0 {
        return u32::MAX;
    }
    if crate::task::users::remove_group(gid as u16) { 0 } else { u32::MAX }
}

/// SYS_LISTGROUPS: List all groups into a buffer.
pub fn sys_listgroups(buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_len as u64) {
        return 0;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize) };
    crate::task::users::list_groups(buf) as u32
}

/// SYS_GETUSERNAME: Get username for a UID.
/// arg1 = uid, arg2 = buf_ptr, arg3 = buf_len. Returns bytes written.
pub fn sys_getusername(uid: u32, buf_ptr: u32, buf_len: u32) -> u32 {
    if buf_ptr == 0 || buf_len == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_len as u64) {
        return 0;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize) };
    crate::task::users::get_username(uid as u16, buf) as u32
}

/// SYS_SET_IDENTITY: Root-only. Set uid/gid on calling process.
/// arg1 = uid. The kernel looks up the user's gid from the user database.
/// Sets uid/gid on ALL threads sharing the caller's page_directory.
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_set_identity(uid: u32) -> u32 {
    // Only root can call this
    if crate::task::scheduler::current_thread_uid() != 0 {
        return u32::MAX;
    }

    let uid16 = uid as u16;

    // Look up gid from user database
    let gid = crate::task::users::lookup_gid_for_uid(uid16);

    // Get current thread's TID to find all threads in this process
    let tid = crate::task::scheduler::current_tid();
    crate::task::scheduler::set_process_identity(tid, uid16, gid);
    0
}

// =========================================================================
// App permission syscalls
// =========================================================================

/// SYS_PERM_CHECK (250): Check stored permissions for an app.
/// arg1 = app_id_ptr (null-terminated string), arg2 = uid (0 = use caller's uid).
/// Returns the granted capability bitmask, or u32::MAX if no permission file exists.
pub fn sys_perm_check(app_id_ptr: u32, uid_arg: u32) -> u32 {
    let app_id = match read_user_str_safe(app_id_ptr) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let uid = if uid_arg == 0 {
        crate::task::scheduler::current_thread_uid()
    } else {
        uid_arg as u16
    };

    match crate::task::permissions::read_stored_perms(uid, app_id) {
        Some(granted) => granted,
        None => u32::MAX,
    }
}

/// SYS_PERM_STORE (251): Store granted permissions for an app.
/// arg1 = app_id_ptr, arg2 = granted bitmask, arg3 = uid (0 = caller's uid).
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_perm_store(app_id_ptr: u32, granted: u32, uid_arg: u32) -> u32 {
    let app_id = match read_user_str_safe(app_id_ptr) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let uid = if uid_arg == 0 {
        crate::task::scheduler::current_thread_uid()
    } else {
        uid_arg as u16
    };

    if crate::task::permissions::write_stored_perms(uid, app_id, granted) {
        crate::serial_println!("PERM_STORE: uid={} app='{}' granted={:#x}", uid, app_id, granted);
        0
    } else {
        u32::MAX
    }
}

/// SYS_PERM_LIST (252): List all apps with stored permissions for the caller's uid.
/// arg1 = buf_ptr, arg2 = buf_size.
/// Writes entries as "app_id\x1Fgranted_hex\n" packed into the buffer.
/// Returns number of entries written.
pub fn sys_perm_list(buf_ptr: u32, buf_size: u32) -> u32 {
    let uid = crate::task::scheduler::current_thread_uid();
    let apps = crate::task::permissions::list_apps_with_perms(uid);

    if buf_ptr == 0 || buf_size == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        return apps.len() as u32;
    }

    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
    let mut offset = 0usize;
    let mut count = 0u32;

    for (app_id, granted) in &apps {
        // Format: "app_id\x1Fgranted_hex\n"
        let entry = alloc::format!("{}\x1F{:x}\n", app_id, granted);
        let bytes = entry.as_bytes();
        if offset + bytes.len() > buf.len() {
            break;
        }
        buf[offset..offset + bytes.len()].copy_from_slice(bytes);
        offset += bytes.len();
        count += 1;
    }

    count
}

/// SYS_PERM_DELETE (253): Delete stored permissions for an app.
/// arg1 = app_id_ptr. Uses caller's uid.
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_perm_delete(app_id_ptr: u32) -> u32 {
    let app_id = match read_user_str_safe(app_id_ptr) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let uid = crate::task::scheduler::current_thread_uid();

    if crate::task::permissions::delete_stored_perms(uid, app_id) {
        crate::serial_println!("PERM_DELETE: uid={} app='{}'", uid, app_id);
        0
    } else {
        u32::MAX
    }
}

/// SYS_PERM_PENDING_INFO (254): Read pending permission info from the calling thread.
/// arg1 = buf_ptr, arg2 = buf_size.
/// The pending info was stored by sys_spawn when it returned PERM_NEEDED.
/// Format: "app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path".
/// Returns bytes written (0 if no pending info).
pub fn sys_perm_pending_info(buf_ptr: u32, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        return 0;
    }

    let mut kernel_buf = [0u8; 512];
    let len = crate::task::scheduler::current_perm_pending(&mut kernel_buf);
    if len == 0 {
        return 0;
    }

    let copy = len.min(buf_size as usize);
    let dst = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy) };
    dst.copy_from_slice(&kernel_buf[..copy]);
    copy as u32
}
