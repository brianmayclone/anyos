use super::*;

pub(super) fn linux_open(path_ptr: u64, linux_flags: u64) -> u64 {
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path.is_empty() {
        return linux_err(ENOENT);
    }
    linux_open_linux_path(raw_path, linux_flags)
}

pub(super) fn linux_openat(dirfd: i32, path_ptr: u64, linux_flags: u64) -> u64 {
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path.is_empty() {
        return linux_err(ENOENT);
    }
    if dirfd == LINUX_AT_FDCWD || raw_path.starts_with('/') {
        return linux_open_linux_path(raw_path, linux_flags);
    }
    if let Some(result) = linux_proc_at_path(dirfd, &raw_path) {
        return match result {
            Ok(abs) => linux_open_linux_path(&abs, linux_flags),
            Err(errno) => linux_err(errno),
        };
    }
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_open_translated(&path, linux_flags, raw_path)
}

pub(super) fn linux_stat(path_ptr: u64, stat_ptr: u64, nofollow: bool) -> u64 {
    if path_ptr == 0 || stat_ptr == 0 {
        return linux_err(EFAULT);
    }
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path.is_empty() {
        return linux_err(ENOENT);
    }
    let abs = linux_absolute_path(raw_path);
    if let Some((file, pid)) = linux_proc_node(&abs) {
        return linux_write_proc_stat(stat_ptr, file, pid);
    }
    let path = linux_translate_path(&abs);
    linux_stat_translated(&path, stat_ptr, nofollow)
}

pub(super) fn linux_newfstatat(dirfd: i32, path_ptr: u64, stat_ptr: u64, flags: u64) -> u64 {
    if stat_ptr == 0 {
        return linux_err(EFAULT);
    }
    let raw_path = if path_ptr == 0 {
        ""
    } else {
        match super::handlers::helpers::read_user_str_safe(path_ptr) {
            Some(path) => path,
            None => return linux_err(EFAULT),
        }
    };
    if raw_path.is_empty() && (flags & LINUX_AT_EMPTY_PATH) != 0 {
        return linux_fstat(dirfd as u32, stat_ptr);
    }
    if raw_path.starts_with('/') || dirfd == LINUX_AT_FDCWD {
        let abs = linux_absolute_path(&raw_path);
        if let Some((file, pid)) = linux_proc_node(&abs) {
            return linux_write_proc_stat(stat_ptr, file, pid);
        }
    } else if let Some(result) = linux_proc_at_path(dirfd, &raw_path) {
        match result {
            Ok(abs) => {
                if let Some((file, pid)) = linux_proc_node(&abs) {
                    return linux_write_proc_stat(stat_ptr, file, pid);
                }
                return linux_err(ENOENT);
            }
            Err(errno) => return linux_err(errno),
        }
    }
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_stat_translated(&path, stat_ptr, (flags & LINUX_AT_SYMLINK_NOFOLLOW) != 0)
}

pub(super) fn linux_access(path_ptr: u64, _mode: u64) -> u64 {
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path.is_empty() {
        return linux_err(ENOENT);
    }
    let abs = linux_absolute_path(raw_path);
    if linux_proc_node(&abs).is_some() {
        return 0;
    }
    let path = linux_translate_path(&abs);
    let resolved = match linux_resolve_translated_path(&path, true, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::stat(&resolved) {
        Ok(_) => 0,
        Err(crate::fs::vfs::FsError::NotFound) => linux_err(ENOENT),
        Err(crate::fs::vfs::FsError::PermissionDenied) => linux_err(EACCES),
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_open_linux_path(path: &str, linux_flags: u64) -> u64 {
    let abs = linux_absolute_path(path);
    if abs == "/dev/tty" {
        let pty_id = crate::task::scheduler::current_thread_pty_id();
        let kind = if pty_id != 0 {
            crate::fs::fd_table::FdKind::PtySlave { pty_id }
        } else {
            crate::fs::fd_table::FdKind::Tty
        };
        let fd = match crate::task::scheduler::current_fd_alloc(kind) {
            Some(fd) => fd,
            None => return linux_err(EBADF),
        };
        if (linux_flags & 0o2000000) != 0 {
            crate::task::scheduler::current_fd_set_cloexec(fd, true);
        }
        return fd as u64;
    }
    if let Some((proc_file, pid)) = linux_proc_node(&abs) {
        if let Some(ret) = linux_open_proc_fd_entry(&abs, proc_file, pid, linux_flags) {
            return ret;
        }
        return linux_open_proc_file(&abs, proc_file, pid, linux_flags);
    }
    let translated = linux_translate_absolute_path(&abs);
    linux_open_translated(&translated, linux_flags, &abs)
}

pub(super) fn linux_open_translated(path: &str, linux_flags: u64, linux_path: &str) -> u64 {
    const LINUX_O_DIRECTORY: u64 = 0o200000;

    let flags = map_open_flags(linux_flags);
    let accmode = linux_flags & 0x3;
    let cloexec = (flags & 0x10) != 0;
    let file_flags = crate::fs::file::FileFlags {
        read: accmode != 1,
        write: accmode == 1 || accmode == 2,
        append: (flags & 2) != 0,
        create: (flags & 4) != 0,
        truncate: (flags & 8) != 0,
        sync: (flags & 0x20) != 0,
    };
    let resolved_path = match linux_resolve_translated_path(path, true, file_flags.create) {
        Ok(path) => path,
        Err(errno) => {
            linux_log_path_error("open-resolve", linux_path, path, path, errno);
            return linux_err(errno);
        }
    };

    if (linux_flags & LINUX_O_DIRECTORY) != 0 || file_flags.write || file_flags.truncate {
        match crate::fs::vfs::stat(&resolved_path) {
            Ok(st) => {
                let is_dir = st.file_type == crate::fs::file::FileType::Directory && !st.is_symlink;
                if (linux_flags & LINUX_O_DIRECTORY) != 0 && !is_dir {
                    return linux_err(ENOTDIR);
                }
                if is_dir && (file_flags.write || file_flags.truncate) {
                    return linux_err(EISDIR);
                }
            }
            Err(crate::fs::vfs::FsError::NotFound) if file_flags.create => {}
            Err(e) => return linux_fs_err(e),
        }
    }

    let global_id = match crate::fs::vfs::open(&resolved_path, file_flags) {
        Ok(id) => id,
        Err(e) => {
            let errno = fs_errno(e);
            linux_log_path_error("open", linux_path, path, &resolved_path, errno);
            return linux_err(errno);
        }
    };
    if file_flags.create {
        linux_resolve_cache_invalidate_path(&resolved_path);
    }
    let local_fd =
        match crate::task::scheduler::current_fd_alloc(crate::fs::fd_table::FdKind::File {
            global_id,
        }) {
            Some(fd) => fd,
            None => {
                crate::fs::vfs::decref(global_id);
                return linux_err(EBADF);
            }
        };
    if cloexec {
        crate::task::scheduler::current_fd_set_cloexec(local_fd, true);
    }
    trace::trace_open(linux_path, &resolved_path, local_fd, global_id, linux_flags);
    linux_log_library_open(linux_path, &resolved_path, local_fd, global_id);
    local_fd as u64
}

fn linux_visible_mode(anyos_type: u32, mode: u32) -> u32 {
    if mode == 0 || mode == 0xFFF {
        return match anyos_type {
            1 => 0o755,
            2 => 0o666,
            3 => 0o777,
            _ => 0o644,
        };
    }
    mode & 0o777
}

pub(super) fn linux_fsync(fd: u32) -> u64 {
    linux_sync_fd(fd, true)
}

pub(super) fn linux_fdatasync(fd: u32) -> u64 {
    linux_sync_fd(fd, false)
}

pub(super) fn linux_sync() -> u64 {
    crate::fs::vfs::sync_all();
    0
}

pub(super) fn linux_syncfs(fd: u32) -> u64 {
    if crate::task::scheduler::current_fd_get(fd).is_none() {
        return linux_err(EBADF);
    }
    crate::fs::vfs::sync_all();
    0
}

pub(super) fn linux_sync_file_range(fd: u32, _offset: u64, _nbytes: u64, _flags: u64) -> u64 {
    if crate::task::scheduler::current_fd_get(fd).is_none() {
        return linux_err(EBADF);
    }
    // Linux allows this as a range writeback hint. Our block cache is sector
    // keyed and the VFS currently lacks a range-only flush API, so treat this
    // as a successful scheduling hint instead of forcing a global disk flush.
    0
}

fn linux_sync_fd(fd: u32, flush_hardware: bool) -> u64 {
    use crate::fs::fd_table::FdKind;

    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => {
                let result = if flush_hardware {
                    crate::fs::vfs::fsync(global_id)
                } else {
                    crate::fs::vfs::fdatasync(global_id)
                };
                let ret = match result {
                    Ok(()) => 0,
                    Err(e) => linux_fs_err(e),
                };
                trace::trace_fsync(
                    if flush_hardware { "fsync" } else { "fdatasync" },
                    fd,
                    global_id,
                    ret,
                );
                ret
            }
            FdKind::PipeRead { .. }
            | FdKind::PipeWrite { .. }
            | FdKind::Tty
            | FdKind::PtySlave { .. }
            | FdKind::LinuxSocket { .. }
            | FdKind::LinuxProc { .. } => 0,
            FdKind::None => linux_err(EBADF),
        },
        None => linux_err(EBADF),
    }
}

pub(super) fn linux_log_library_open(
    linux_path: &str,
    resolved_path: &str,
    fd: u32,
    global_id: u32,
) {
    if !linux_path.contains(".so") && !resolved_path.contains(".so") {
        return;
    }
    match crate::fs::vfs::fstat(global_id) {
        Ok((_file_type, size, _position, _mtime)) => {
            let (dev, ino) = linux_fd_identity(global_id);
            crate::serial_verbose_println!(
                "lxe linux open-resolve: ok linux='{}' resolved='{}' fd={} size={} dev={} ino={}",
                linux_path,
                resolved_path,
                fd,
                size,
                dev,
                ino
            );
        }
        Err(_) => {
            crate::serial_verbose_println!(
                "lxe linux open-resolve: ok linux='{}' resolved='{}' fd={}",
                linux_path,
                resolved_path,
                fd
            );
        }
    }
}

pub(super) fn linux_stat_translated(path: &str, stat_ptr: u64, nofollow: bool) -> u64 {
    let path = match linux_resolve_translated_path(path, !nofollow, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let ret = if nofollow {
        crate::fs::vfs::lstat(&path)
    } else {
        crate::fs::vfs::stat(&path)
    };
    match ret {
        Ok(st) => {
            let type_val = if st.is_symlink {
                3
            } else {
                match st.file_type {
                    crate::fs::file::FileType::Directory => 1,
                    crate::fs::file::FileType::Device => 2,
                    _ => 0,
                }
            };
            let uid = if st.uid == 0 {
                handlers::sys_getuid()
            } else {
                st.uid as u32
            };
            let (dev, ino) = linux_stat_identity(&path);
            write_linux_stat(
                stat_ptr,
                dev,
                ino,
                type_val,
                st.size as u64,
                uid,
                st.gid as u32,
                linux_visible_mode(type_val, st.mode as u32),
                st.mtime,
            );
            0
        }
        Err(crate::fs::vfs::FsError::NotFound) => linux_err(ENOENT),
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_fstat(fd: u32, stat_ptr: u64) -> u64 {
    if stat_ptr == 0 {
        return linux_err(EFAULT);
    }

    use crate::fs::fd_table::FdKind;
    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => match crate::fs::vfs::fstat(global_id) {
                Ok((file_type, size, _position, mtime)) => {
                    let (dev, ino) = linux_fd_identity(global_id);
                    let mtime = linux_fd_metadata_mtime(global_id).unwrap_or(mtime);
                    write_linux_stat(
                        stat_ptr,
                        dev,
                        ino,
                        anyos_file_type(file_type),
                        size as u64,
                        handlers::sys_getuid(),
                        handlers::sys_getgid(),
                        linux_visible_mode(anyos_file_type(file_type), 0),
                        mtime,
                    );
                    0
                }
                Err(_) => linux_err(EBADF),
            },
            FdKind::PipeRead { .. }
            | FdKind::PipeWrite { .. }
            | FdKind::Tty
            | FdKind::PtySlave { .. }
            | FdKind::LinuxSocket { .. } => {
                write_linux_stat(stat_ptr, 0, 2, 2, 0, 0, 0, 0o666, 0);
                0
            }
            FdKind::LinuxProc { file, pid, .. } => linux_write_proc_stat(stat_ptr, file, pid),
            FdKind::None => linux_err(EBADF),
        },
        None if fd < 3 => {
            write_linux_stat(stat_ptr, 0, 2, 2, 0, 0, 0, 0o666, 0);
            0
        }
        None => linux_err(EBADF),
    }
}

pub(super) fn linux_statx(
    dirfd: i32,
    path_ptr: u64,
    flags: u64,
    _mask: u64,
    statx_ptr: u64,
) -> u64 {
    const AT_NO_AUTOMOUNT: u64 = 0x800;
    const AT_STATX_SYNC_TYPE: u64 = 0x6000;
    const ALLOWED_FLAGS: u64 =
        LINUX_AT_SYMLINK_NOFOLLOW | LINUX_AT_EMPTY_PATH | AT_NO_AUTOMOUNT | AT_STATX_SYNC_TYPE;

    if statx_ptr == 0 || !super::handlers::helpers::is_user_range_accessible(statx_ptr, 256) {
        return linux_err(EFAULT);
    }
    if (flags & !ALLOWED_FLAGS) != 0 {
        return linux_err(EINVAL);
    }

    let raw_path = if path_ptr == 0 {
        ""
    } else {
        match super::handlers::helpers::read_user_str_safe(path_ptr) {
            Some(path) => path,
            None => return linux_err(EFAULT),
        }
    };

    if raw_path.is_empty() {
        if (flags & LINUX_AT_EMPTY_PATH) == 0 {
            return linux_err(ENOENT);
        }
        if dirfd == LINUX_AT_FDCWD {
            return linux_err(ENOENT);
        }
        return linux_fstatx(dirfd as u32, statx_ptr);
    }

    if raw_path.starts_with('/') || dirfd == LINUX_AT_FDCWD {
        let abs = linux_absolute_path(&raw_path);
        if let Some((file, pid)) = linux_proc_node(&abs) {
            return linux_write_proc_statx(statx_ptr, file, pid);
        }
    } else if let Some(result) = linux_proc_at_path(dirfd, &raw_path) {
        match result {
            Ok(abs) => {
                if let Some((file, pid)) = linux_proc_node(&abs) {
                    return linux_write_proc_statx(statx_ptr, file, pid);
                }
                return linux_err(ENOENT);
            }
            Err(errno) => return linux_err(errno),
        }
    }

    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_statx_translated(&path, statx_ptr, (flags & LINUX_AT_SYMLINK_NOFOLLOW) != 0)
}

fn linux_fstatx(fd: u32, statx_ptr: u64) -> u64 {
    use crate::fs::fd_table::FdKind;
    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => match crate::fs::vfs::fstat(global_id) {
                Ok((file_type, size, _position, mtime)) => {
                    let (dev, ino) = linux_fd_identity(global_id);
                    let mtime = linux_fd_metadata_mtime(global_id).unwrap_or(mtime);
                    write_linux_statx(
                        statx_ptr,
                        dev,
                        ino,
                        anyos_file_type(file_type),
                        size as u64,
                        handlers::sys_getuid(),
                        handlers::sys_getgid(),
                        linux_visible_mode(anyos_file_type(file_type), 0),
                        mtime,
                    );
                    0
                }
                Err(_) => linux_err(EBADF),
            },
            FdKind::PipeRead { .. }
            | FdKind::PipeWrite { .. }
            | FdKind::Tty
            | FdKind::PtySlave { .. }
            | FdKind::LinuxSocket { .. } => {
                write_linux_statx(statx_ptr, 0, 2, 2, 0, 0, 0, 0o666, 0);
                0
            }
            FdKind::LinuxProc { file, pid, .. } => linux_write_proc_statx(statx_ptr, file, pid),
            FdKind::None => linux_err(EBADF),
        },
        None if fd < 3 => {
            write_linux_statx(statx_ptr, 0, 2, 2, 0, 0, 0, 0o666, 0);
            0
        }
        None => linux_err(EBADF),
    }
}

fn linux_statx_translated(path: &str, statx_ptr: u64, nofollow: bool) -> u64 {
    let path = match linux_resolve_translated_path(path, !nofollow, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let ret = if nofollow {
        crate::fs::vfs::lstat(&path)
    } else {
        crate::fs::vfs::stat(&path)
    };
    match ret {
        Ok(st) => {
            let type_val = if st.is_symlink {
                3
            } else {
                match st.file_type {
                    crate::fs::file::FileType::Directory => 1,
                    crate::fs::file::FileType::Device => 2,
                    _ => 0,
                }
            };
            let uid = if st.uid == 0 {
                handlers::sys_getuid()
            } else {
                st.uid as u32
            };
            let (dev, ino) = linux_stat_identity(&path);
            write_linux_statx(
                statx_ptr,
                dev,
                ino,
                type_val,
                st.size as u64,
                uid,
                st.gid as u32,
                linux_visible_mode(type_val, st.mode as u32),
                st.mtime,
            );
            0
        }
        Err(crate::fs::vfs::FsError::NotFound) => linux_err(ENOENT),
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_getcwd(buf_ptr: u64, buf_size: u64) -> u64 {
    if buf_ptr == 0 || buf_size == 0 || buf_size > u32::MAX as u64 {
        return linux_err(EFAULT);
    }
    let ret = handlers::sys_getcwd(buf_ptr, buf_size as u32);
    if ret == u32::MAX {
        linux_err(EINVAL)
    } else {
        ret as u64 + 1
    }
}

pub(super) fn linux_chdir(path_ptr: u64) -> u64 {
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path.is_empty() {
        return linux_err(ENOENT);
    }
    let linux_path = linux_absolute_path(raw_path);
    let host_path = linux_translate_path(&linux_path);
    let resolved = match linux_resolve_translated_path(&host_path, true, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::read_dir(&resolved) {
        Ok(_) => {
            let cwd = linux_strip_rootfs(&resolved);
            crate::task::scheduler::set_thread_cwd(crate::task::scheduler::current_tid(), &cwd);
            0
        }
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_fchdir(fd: u32) -> u64 {
    let path = match linux_fd_path(fd) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let resolved = match linux_resolve_translated_path(&path, true, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::read_dir(&resolved) {
        Ok(_) => {
            let linux_path = linux_strip_rootfs(&resolved);
            crate::task::scheduler::set_thread_cwd(
                crate::task::scheduler::current_tid(),
                &linux_path,
            );
            0
        }
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_readlink(path_ptr: u64, buf_ptr: u64, buf_size: u64) -> u64 {
    if buf_ptr == 0 || buf_size == 0 {
        return linux_err(EFAULT);
    }
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path.is_empty() {
        return linux_err(ENOENT);
    }
    if raw_path == "/proc/self/exe" {
        return linux_err(ENOENT);
    }
    let abs = linux_absolute_path(raw_path);
    if let Some(result) = linux_proc_readlink(&abs) {
        return match result {
            Ok(target) => linux_copy_readlink_target(&target, buf_ptr, buf_size),
            Err(errno) => linux_err(errno),
        };
    }
    let path = linux_translate_absolute_path(&abs);
    match crate::fs::vfs::readlink(&path) {
        Ok(target) => linux_copy_readlink_target(&linux_strip_rootfs(&target), buf_ptr, buf_size),
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_readlinkat(dirfd: i32, path_ptr: u64, buf_ptr: u64, buf_size: u64) -> u64 {
    if buf_ptr == 0 || buf_size == 0 {
        return linux_err(EFAULT);
    }
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path.is_empty() {
        return linux_err(ENOENT);
    }
    if raw_path.starts_with('/') || dirfd == LINUX_AT_FDCWD {
        let abs = linux_absolute_path(&raw_path);
        if let Some(result) = linux_proc_readlink(&abs) {
            return match result {
                Ok(target) => linux_copy_readlink_target(&target, buf_ptr, buf_size),
                Err(errno) => linux_err(errno),
            };
        }
    } else if let Some(result) = linux_proc_at_path(dirfd, &raw_path) {
        return match result {
            Ok(abs) => match linux_proc_readlink(&abs) {
                Some(Ok(target)) => linux_copy_readlink_target(&target, buf_ptr, buf_size),
                Some(Err(errno)) => linux_err(errno),
                None => linux_err(ENOENT),
            },
            Err(errno) => linux_err(errno),
        };
    }
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_readlink_translated(&path, buf_ptr, buf_size)
}

pub(super) fn linux_readlink_translated(path: &str, buf_ptr: u64, buf_size: u64) -> u64 {
    if buf_ptr == 0 || buf_size == 0 {
        return linux_err(EFAULT);
    }
    match crate::fs::vfs::readlink(path) {
        Ok(target) => linux_copy_readlink_target(&linux_strip_rootfs(&target), buf_ptr, buf_size),
        Err(e) => linux_fs_err(e),
    }
}

fn linux_copy_readlink_target(target: &str, buf_ptr: u64, buf_size: u64) -> u64 {
    let bytes = target.as_bytes();
    let to_copy = bytes.len().min(buf_size as usize);
    if !super::handlers::helpers::copy_to_user_bytes(buf_ptr, &bytes[..to_copy], to_copy) {
        return linux_err(EFAULT);
    }
    to_copy as u64
}

pub(super) fn linux_rename(old_ptr: u64, new_ptr: u64) -> u64 {
    let old_path = match linux_translate_user_path(old_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let new_path = match linux_translate_user_path(new_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let ret = match crate::fs::vfs::rename(&old_path, &new_path) {
        Ok(()) => {
            linux_resolve_cache_invalidate_path(&old_path);
            linux_resolve_cache_invalidate_path(&new_path);
            0
        }
        Err(e) => linux_fs_err(e),
    };
    trace::trace_path_op("rename", &old_path, Some(&new_path), ret);
    ret
}

pub(super) fn linux_renameat(old_dirfd: i32, old_ptr: u64, new_dirfd: i32, new_ptr: u64) -> u64 {
    let old_path = match linux_translate_at_path(old_dirfd, old_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let new_path = match linux_translate_at_path(new_dirfd, new_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let ret = match crate::fs::vfs::rename(&old_path, &new_path) {
        Ok(()) => {
            linux_resolve_cache_invalidate_path(&old_path);
            linux_resolve_cache_invalidate_path(&new_path);
            0
        }
        Err(e) => linux_fs_err(e),
    };
    trace::trace_path_op("renameat", &old_path, Some(&new_path), ret);
    ret
}

pub(super) fn linux_link(old_ptr: u64, new_ptr: u64) -> u64 {
    let old_path = match linux_translate_user_path(old_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let new_path = match linux_translate_user_path(new_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_link_translated(&old_path, &new_path, false)
}

pub(super) fn linux_linkat(
    old_dirfd: i32,
    old_ptr: u64,
    new_dirfd: i32,
    new_ptr: u64,
    flags: u64,
) -> u64 {
    const AT_SYMLINK_FOLLOW: u64 = 0x400;
    if (flags & !AT_SYMLINK_FOLLOW) != 0 {
        return linux_err(EINVAL);
    }
    let old_path = match linux_translate_at_path(old_dirfd, old_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let new_path = match linux_translate_at_path(new_dirfd, new_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_link_translated(&old_path, &new_path, (flags & AT_SYMLINK_FOLLOW) == 0)
}

fn linux_link_translated(old_path: &str, new_path: &str, nofollow: bool) -> u64 {
    let old_resolved = match linux_resolve_translated_path(old_path, !nofollow, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let new_resolved = match linux_resolve_translated_path(new_path, true, true) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    if crate::fs::vfs::stat(&new_resolved).is_ok() || crate::fs::vfs::lstat(&new_resolved).is_ok() {
        return linux_err(EEXIST);
    }

    let st = match if nofollow {
        crate::fs::vfs::lstat(&old_resolved)
    } else {
        crate::fs::vfs::stat(&old_resolved)
    } {
        Ok(st) => st,
        Err(e) => return linux_fs_err(e),
    };
    if st.file_type == crate::fs::file::FileType::Directory {
        return linux_err(EPERM);
    }
    if st.is_symlink {
        let target = match crate::fs::vfs::readlink(&old_resolved) {
            Ok(target) => target,
            Err(e) => return linux_fs_err(e),
        };
        return match crate::fs::vfs::create_symlink(&new_resolved, &target) {
            Ok(()) => {
                linux_resolve_cache_invalidate_path(&new_resolved);
                0
            }
            Err(e) => linux_fs_err(e),
        };
    }

    let result = linux_copy_regular_file(&old_resolved, &new_resolved, st.mode, st.uid, st.gid);
    if result == 0 {
        linux_resolve_cache_invalidate_path(&new_resolved);
    }
    result
}

fn linux_copy_regular_file(old_path: &str, new_path: &str, mode: u16, uid: u16, gid: u16) -> u64 {
    let read_flags = crate::fs::file::FileFlags {
        read: true,
        write: false,
        append: false,
        create: false,
        truncate: false,
        sync: false,
    };
    let write_flags = crate::fs::file::FileFlags {
        read: false,
        write: true,
        append: false,
        create: true,
        truncate: true,
        sync: false,
    };
    let src = match crate::fs::vfs::open(old_path, read_flags) {
        Ok(fd) => fd,
        Err(e) => return linux_fs_err(e),
    };
    let dst = match crate::fs::vfs::open(new_path, write_flags) {
        Ok(fd) => fd,
        Err(e) => {
            let _ = crate::fs::vfs::close(src);
            return linux_fs_err(e);
        }
    };

    let mut buf = [0u8; 4096];
    let mut result = 0;
    loop {
        let n = match crate::fs::vfs::read(src, &mut buf) {
            Ok(n) => n,
            Err(e) => {
                result = linux_fs_err(e);
                break;
            }
        };
        if n == 0 {
            break;
        }
        match crate::fs::vfs::write(dst, &buf[..n]) {
            Ok(written) if written == n => {}
            Ok(_) => {
                result = linux_err(EIO);
                break;
            }
            Err(e) => {
                result = linux_fs_err(e);
                break;
            }
        }
    }
    let _ = crate::fs::vfs::close(src);
    let _ = crate::fs::vfs::close(dst);
    if result == 0 {
        let _ = crate::fs::vfs::set_mode(new_path, mode);
        let _ = crate::fs::vfs::set_owner(new_path, uid, gid);
    } else {
        let _ = crate::fs::vfs::delete(new_path);
    }
    result
}

pub(super) fn linux_symlink(target_ptr: u64, link_path_ptr: u64) -> u64 {
    let link_path = match linux_translate_user_path(link_path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_symlink_translated(target_ptr, &link_path)
}

pub(super) fn linux_symlinkat(target_ptr: u64, dirfd: i32, link_path_ptr: u64) -> u64 {
    let link_path = match linux_translate_at_path(dirfd, link_path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_symlink_translated(target_ptr, &link_path)
}

fn linux_symlink_translated(target_ptr: u64, link_path: &str) -> u64 {
    let target = match super::handlers::helpers::read_user_str_safe(target_ptr) {
        Some(target) if !target.is_empty() => target,
        Some(_) => return linux_err(ENOENT),
        None => return linux_err(EFAULT),
    };
    match crate::fs::vfs::create_symlink(link_path, target) {
        Ok(()) => {
            linux_resolve_cache_invalidate_path(link_path);
            0
        }
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_mkdir(path_ptr: u64, _mode: u64) -> u64 {
    let path = match linux_translate_user_path(path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::mkdir(&path) {
        Ok(()) => {
            linux_resolve_cache_invalidate_path(&path);
            0
        }
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_mkdirat(dirfd: i32, path_ptr: u64, _mode: u64) -> u64 {
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::mkdir(&path) {
        Ok(()) => {
            linux_resolve_cache_invalidate_path(&path);
            0
        }
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_unlink_path(path_ptr: u64) -> u64 {
    let path = match linux_translate_user_path(path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_unlink_translated(&path)
}

pub(super) fn linux_unlinkat(dirfd: i32, path_ptr: u64, flags: u64) -> u64 {
    if (flags & !LINUX_AT_REMOVEDIR) != 0 {
        return linux_err(EINVAL);
    }
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_unlink_translated(&path)
}

pub(super) fn linux_unlink_translated(path: &str) -> u64 {
    if linux_is_protected_rootfs_path(path) {
        crate::serial_verbose_println!(
            "lxe linux unlink: blocked protected rootfs path '{}'",
            path
        );
        return linux_err(EBUSY);
    }
    let ret = match crate::fs::vfs::delete(path) {
        Ok(()) => {
            linux_resolve_cache_invalidate_path(path);
            0
        }
        Err(e) => linux_fs_err(e),
    };
    trace::trace_path_op("unlink", path, None, ret);
    ret
}

fn linux_is_protected_rootfs_path(path: &str) -> bool {
    let rootfs = current_linux_rootfs();
    let normalized = crate::fs::path::normalize(path);
    if normalized == rootfs {
        return true;
    }
    if !linux_path_under_rootfs(&rootfs, &normalized) {
        return false;
    }
    let rel = &normalized[rootfs.len() + 1..];
    if rel.contains('/') {
        return false;
    }
    matches!(
        rel,
        "bin"
            | "boot"
            | "dev"
            | "etc"
            | "home"
            | "lib"
            | "lib64"
            | "proc"
            | "root"
            | "run"
            | "sbin"
            | "sys"
            | "tmp"
            | "usr"
            | "var"
    )
}

pub(super) fn linux_creat(path_ptr: u64) -> u64 {
    const O_CREAT_WRONLY_TRUNC: u64 = 0x40 | 0x01 | 0x200;
    linux_open(path_ptr, O_CREAT_WRONLY_TRUNC)
}

pub(super) fn linux_truncate(path_ptr: u64, len: u64) -> u64 {
    if len > u32::MAX as u64 {
        return linux_err(EFBIG);
    }
    let path = match linux_translate_user_path(path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::truncate_to(&path, len as u32) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_ftruncate(fd: u32, len: u64) -> u64 {
    use crate::fs::fd_table::FdKind;

    if len > u32::MAX as u64 {
        return linux_err(EFBIG);
    }
    let global_id = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => global_id,
            FdKind::None => return linux_err(EBADF),
            _ => return linux_err(EINVAL),
        },
        None => return linux_err(EBADF),
    };
    match crate::fs::vfs::ftruncate_to(global_id, len as u32) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_chmod(path_ptr: u64, mode: u64) -> u64 {
    let path = match linux_translate_user_path(path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::set_mode(&path, mode as u16) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_fchmod(fd: u32, mode: u64) -> u64 {
    let path = match linux_fd_path(fd) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::set_mode(&path, mode as u16) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_fchmodat(dirfd: i32, path_ptr: u64, mode: u64, _flags: u64) -> u64 {
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::set_mode(&path, mode as u16) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_chown(path_ptr: u64, uid: u32, gid: u32) -> u64 {
    let path = match linux_translate_user_path(path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_chown_translated(&path, uid, gid)
}

pub(super) fn linux_fchown(fd: u32, uid: u32, gid: u32) -> u64 {
    let path = match linux_fd_path(fd) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_chown_translated(&path, uid, gid)
}

pub(super) fn linux_fchownat(dirfd: i32, path_ptr: u64, uid: u32, gid: u32) -> u64 {
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_chown_translated(&path, uid, gid)
}

pub(super) fn linux_chown_translated(path: &str, uid: u32, gid: u32) -> u64 {
    let (current_uid, current_gid, _) = crate::fs::vfs::get_permissions(path).unwrap_or((0, 0, 0));
    let uid = if uid == u32::MAX {
        current_uid
    } else {
        uid as u16
    };
    let gid = if gid == u32::MAX {
        current_gid
    } else {
        gid as u16
    };
    match crate::fs::vfs::set_owner(path, uid, gid) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_faccessat(dirfd: i32, path_ptr: u64, mode: u64, flags: u64) -> u64 {
    linux_faccessat_impl(dirfd, path_ptr, mode, flags)
}

pub(super) fn linux_faccessat2(dirfd: i32, path_ptr: u64, mode: u64, flags: u64) -> u64 {
    linux_faccessat_impl(dirfd, path_ptr, mode, flags)
}

pub(super) fn linux_utime(path_ptr: u64, times_ptr: u64) -> u64 {
    if times_ptr != 0 && !super::handlers::helpers::is_user_range_accessible(times_ptr, 16) {
        return linux_err(EFAULT);
    }
    let path = match linux_translate_user_path(path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::stat(&path) {
        Ok(_) => 0,
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_futimesat(dirfd: i32, path_ptr: u64, times_ptr: u64) -> u64 {
    if times_ptr != 0 {
        if !super::handlers::helpers::is_user_range_accessible(times_ptr, 32) {
            return linux_err(EFAULT);
        }
        for idx in 0..2 {
            let base = times_ptr + idx * 16;
            let sec = unsafe { read_u64(base, 0) as i64 };
            let usec = unsafe { read_u64(base, 8) as i64 };
            if sec < 0 || !(0..1_000_000).contains(&usec) {
                return linux_err(EINVAL);
            }
        }
    }
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::stat(&path) {
        Ok(_) => 0,
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_utimensat(dirfd: i32, path_ptr: u64, times_ptr: u64, flags: u64) -> u64 {
    const AT_SYMLINK_NOFOLLOW: u64 = LINUX_AT_SYMLINK_NOFOLLOW;
    const UTIME_NOW: i64 = 1_073_741_823;
    const UTIME_OMIT: i64 = 1_073_741_822;

    if (flags & !AT_SYMLINK_NOFOLLOW) != 0 {
        return linux_err(EINVAL);
    }
    if times_ptr != 0 {
        if !super::handlers::helpers::is_user_range_accessible(times_ptr, 32) {
            return linux_err(EFAULT);
        }
        for idx in 0..2 {
            let base = times_ptr + idx * 16;
            let sec = unsafe { read_u64(base, 0) as i64 };
            let nsec = unsafe { read_u64(base, 8) as i64 };
            if sec < 0 && nsec != UTIME_NOW && nsec != UTIME_OMIT {
                return linux_err(EINVAL);
            }
            if !(0..1_000_000_000).contains(&nsec) && nsec != UTIME_NOW && nsec != UTIME_OMIT {
                return linux_err(EINVAL);
            }
        }
    }
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let ret = if (flags & AT_SYMLINK_NOFOLLOW) != 0 {
        crate::fs::vfs::lstat(&path)
    } else {
        crate::fs::vfs::stat(&path)
    };
    match ret {
        Ok(_) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_faccessat_impl(dirfd: i32, path_ptr: u64, mode: u64, flags: u64) -> u64 {
    const ACCESS_MODE_MASK: u64 = 0x7; // R_OK | W_OK | X_OK
    const ALLOWED_FLAGS: u64 = LINUX_AT_SYMLINK_NOFOLLOW | LINUX_AT_EACCESS | LINUX_AT_EMPTY_PATH;

    if (mode & !ACCESS_MODE_MASK) != 0 || (flags & !ALLOWED_FLAGS) != 0 {
        return linux_err(EINVAL);
    }
    if path_ptr == 0 {
        return linux_err(EFAULT);
    }
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path.is_empty() && (flags & LINUX_AT_EMPTY_PATH) != 0 {
        if dirfd == LINUX_AT_FDCWD || crate::task::scheduler::current_fd_get(dirfd as u32).is_some()
        {
            return 0;
        }
        return linux_err(EBADF);
    }
    if raw_path.starts_with('/') || dirfd == LINUX_AT_FDCWD {
        let abs = linux_absolute_path(&raw_path);
        if linux_proc_node(&abs).is_some() {
            return 0;
        }
    } else if let Some(result) = linux_proc_at_path(dirfd, &raw_path) {
        return match result {
            Ok(abs) if linux_proc_node(&abs).is_some() => 0,
            Ok(_) => linux_err(ENOENT),
            Err(errno) => linux_err(errno),
        };
    }
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let follow_last = (flags & LINUX_AT_SYMLINK_NOFOLLOW) == 0;
    let resolved = match linux_resolve_translated_path(&path, follow_last, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let ret = if follow_last {
        crate::fs::vfs::stat(&resolved)
    } else {
        crate::fs::vfs::lstat(&resolved)
    };
    match ret {
        Ok(_) => {
            let _ = mode;
            0
        }
        Err(e) => linux_fs_err(e),
    }
}

pub(super) fn linux_getdents64(fd: u32, dirent_ptr: u64, count: u64) -> u64 {
    if dirent_ptr == 0 || count < 24 || count > u32::MAX as u64 {
        return linux_err(EFAULT);
    }
    if !super::handlers::helpers::is_user_range_accessible(dirent_ptr, count) {
        return linux_err(EFAULT);
    }
    let entry = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    let global_id = match entry.kind {
        crate::fs::fd_table::FdKind::LinuxProc {
            file,
            pid,
            position,
        } => return linux_getdents64_proc(fd, file, pid, position, dirent_ptr, count),
        crate::fs::fd_table::FdKind::File { global_id } => global_id,
        _ => return linux_err(EBADF),
    };
    let (file_type, _size, position, _mtime) = match crate::fs::vfs::fstat(global_id) {
        Ok(st) => st,
        Err(e) => return linux_fs_err(e),
    };
    if file_type != crate::fs::file::FileType::Directory {
        return linux_err(ENOTTY);
    }
    let path = match crate::fs::vfs::get_fd_path(global_id) {
        Ok(path) => path,
        Err(e) => return linux_fs_err(e),
    };
    let entries = match crate::fs::vfs::read_dir(&path) {
        Ok(entries) => entries,
        Err(e) => return linux_fs_err(e),
    };

    let start = position as usize;
    let mut written = 0usize;
    let mut next_index = start;
    for (idx, entry) in entries.iter().enumerate().skip(start) {
        let name = entry.name.as_bytes();
        let reclen = align_up(19 + name.len() + 1, 8);
        if written + reclen > count as usize {
            break;
        }
        let ino = linux_dirent_inode(&path, &entry.name);
        let base = dirent_ptr + written as u64;
        unsafe {
            write_u64(base, 0, ino);
            write_u64(base, 8, (idx + 1) as u64);
            *((base + 16) as *mut u16) = reclen as u16;
            *((base + 18) as *mut u8) = linux_dir_type(entry.file_type);
            core::ptr::copy_nonoverlapping(name.as_ptr(), (base + 19) as *mut u8, name.len());
            *((base + 19 + name.len() as u64) as *mut u8) = 0;
            if reclen > 20 + name.len() {
                core::ptr::write_bytes(
                    (base + 20 + name.len() as u64) as *mut u8,
                    0,
                    reclen - 20 - name.len(),
                );
            }
        }
        written += reclen;
        next_index = idx + 1;
    }
    let _ = crate::fs::vfs::lseek(global_id, next_index as i32, 0);
    written as u64
}

pub(super) fn linux_getdents(fd: u32, dirent_ptr: u64, count: u64) -> u64 {
    if dirent_ptr == 0 || count < 24 || count > u32::MAX as u64 {
        return linux_err(EFAULT);
    }
    if !super::handlers::helpers::is_user_range_accessible(dirent_ptr, count) {
        return linux_err(EFAULT);
    }
    let entry = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    let global_id = match entry.kind {
        crate::fs::fd_table::FdKind::LinuxProc {
            file,
            pid,
            position,
        } => return linux_getdents_proc(fd, file, pid, position, dirent_ptr, count),
        crate::fs::fd_table::FdKind::File { global_id } => global_id,
        _ => return linux_err(EBADF),
    };
    let (file_type, _size, position, _mtime) = match crate::fs::vfs::fstat(global_id) {
        Ok(st) => st,
        Err(e) => return linux_fs_err(e),
    };
    if file_type != crate::fs::file::FileType::Directory {
        return linux_err(ENOTTY);
    }
    let path = match crate::fs::vfs::get_fd_path(global_id) {
        Ok(path) => path,
        Err(e) => return linux_fs_err(e),
    };
    let entries = match crate::fs::vfs::read_dir(&path) {
        Ok(entries) => entries,
        Err(e) => return linux_fs_err(e),
    };

    let start = position as usize;
    let mut written = 0usize;
    let mut next_index = start;
    for (idx, entry) in entries.iter().enumerate().skip(start) {
        let name = entry.name.as_bytes();
        let reclen = align_up(18 + name.len() + 2, 8);
        if written + reclen > count as usize {
            break;
        }
        let ino = linux_dirent_inode(&path, &entry.name);
        let base = dirent_ptr + written as u64;
        unsafe {
            write_u64(base, 0, ino);
            write_u64(base, 8, (idx + 1) as u64);
            *((base + 16) as *mut u16) = reclen as u16;
            core::ptr::copy_nonoverlapping(name.as_ptr(), (base + 18) as *mut u8, name.len());
            *((base + 18 + name.len() as u64) as *mut u8) = 0;
            *((base + reclen as u64 - 1) as *mut u8) = linux_dir_type(entry.file_type);
            if reclen > 19 + name.len() {
                core::ptr::write_bytes(
                    (base + 19 + name.len() as u64) as *mut u8,
                    0,
                    reclen - 20 - name.len(),
                );
            }
        }
        written += reclen;
        next_index = idx + 1;
    }
    let _ = crate::fs::vfs::lseek(global_id, next_index as i32, 0);
    written as u64
}

fn linux_dirent_inode(parent: &str, name: &str) -> u64 {
    if name == "." {
        return linux_path_inode(parent).max(1);
    }
    if name == ".." {
        let parent_path = match parent.rfind('/') {
            Some(0) | None => "/",
            Some(pos) => &parent[..pos],
        };
        return linux_path_inode(parent_path).max(1);
    }
    let full = if parent == "/" {
        alloc::format!("/{}", name)
    } else {
        alloc::format!("{}/{}", parent, name)
    };
    linux_path_inode(&full).max(1)
}

fn linux_fd_metadata_mtime(global_id: u32) -> Option<u32> {
    let path = crate::fs::vfs::get_fd_path(global_id).ok()?;
    if !trace::path_interesting(&path) {
        return None;
    }
    crate::fs::vfs::stat(&path).ok().map(|st| st.mtime)
}

pub(super) fn linux_statfs(path_ptr: u64, buf_ptr: u64) -> u64 {
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path.is_empty() {
        return linux_err(ENOENT);
    }
    let abs = linux_absolute_path(&raw_path);
    if linux_proc_node(&abs).is_some() {
        return linux_write_proc_statfs(buf_ptr);
    }
    let path = match linux_translate_user_path(path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_statfs_translated(&path, buf_ptr)
}

pub(super) fn linux_fstatfs(fd: u32, buf_ptr: u64) -> u64 {
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
        if let crate::fs::fd_table::FdKind::LinuxProc { .. } = entry.kind {
            return linux_write_proc_statfs(buf_ptr);
        }
    }
    let path = match linux_fd_path(fd) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_statfs_translated(&path, buf_ptr)
}

pub(super) fn linux_statfs_translated(path: &str, buf_ptr: u64) -> u64 {
    if !super::handlers::helpers::is_user_range_accessible(buf_ptr, 120) {
        return linux_err(EFAULT);
    }
    let st = crate::fs::vfs::statfs(path)
        .or_else(|| crate::fs::vfs::statfs("/"))
        .unwrap_or(crate::fs::vfs::StatFs {
            total_bytes: 64 * 1024 * 1024,
            used_bytes: 0,
            free_bytes: 64 * 1024 * 1024,
        });
    let block_size = 4096u64;
    unsafe {
        core::ptr::write_bytes(buf_ptr as *mut u8, 0, 120);
        write_u64(buf_ptr, 0, 0xEF53);
        write_u64(buf_ptr, 8, block_size);
        write_u64(buf_ptr, 16, st.total_bytes / block_size);
        write_u64(buf_ptr, 24, st.free_bytes / block_size);
        write_u64(buf_ptr, 32, st.free_bytes / block_size);
        write_u64(buf_ptr, 40, 1024 * 1024);
        write_u64(buf_ptr, 48, 1024 * 1024);
        write_u32(buf_ptr, 72, 255);
        write_u64(buf_ptr, 80, block_size);
    }
    0
}

pub(super) fn linux_setxattr(
    path_ptr: u64,
    name_ptr: u64,
    _value_ptr: u64,
    _size: u64,
    _flags: u64,
    nofollow: bool,
) -> u64 {
    match linux_check_xattr_path(path_ptr, nofollow) {
        Ok(()) => {}
        Err(errno) => return linux_err(errno),
    }
    if name_ptr == 0 || super::handlers::helpers::read_user_str_safe(name_ptr).is_none() {
        return linux_err(EFAULT);
    }
    linux_err(EOPNOTSUPP)
}

pub(super) fn linux_fsetxattr(
    fd: u32,
    name_ptr: u64,
    _value_ptr: u64,
    _size: u64,
    _flags: u64,
) -> u64 {
    if crate::task::scheduler::current_fd_get(fd).is_none() {
        return linux_err(EBADF);
    }
    if name_ptr == 0 || super::handlers::helpers::read_user_str_safe(name_ptr).is_none() {
        return linux_err(EFAULT);
    }
    linux_err(EOPNOTSUPP)
}

pub(super) fn linux_getxattr(
    path_ptr: u64,
    name_ptr: u64,
    _value_ptr: u64,
    _size: u64,
    nofollow: bool,
) -> u64 {
    match linux_check_xattr_path(path_ptr, nofollow) {
        Ok(()) => {}
        Err(errno) => return linux_err(errno),
    }
    if name_ptr == 0 || super::handlers::helpers::read_user_str_safe(name_ptr).is_none() {
        return linux_err(EFAULT);
    }
    linux_err(ENODATA)
}

pub(super) fn linux_fgetxattr(fd: u32, name_ptr: u64, _value_ptr: u64, _size: u64) -> u64 {
    if crate::task::scheduler::current_fd_get(fd).is_none() {
        return linux_err(EBADF);
    }
    if name_ptr == 0 || super::handlers::helpers::read_user_str_safe(name_ptr).is_none() {
        return linux_err(EFAULT);
    }
    linux_err(ENODATA)
}

pub(super) fn linux_listxattr(path_ptr: u64, list_ptr: u64, size: u64, nofollow: bool) -> u64 {
    match linux_check_xattr_path(path_ptr, nofollow) {
        Ok(()) => {}
        Err(errno) => return linux_err(errno),
    }
    if list_ptr != 0 && size != 0 && !handlers::helpers::is_user_range_accessible(list_ptr, size) {
        return linux_err(EFAULT);
    }
    0
}

pub(super) fn linux_flistxattr(fd: u32, list_ptr: u64, size: u64) -> u64 {
    if crate::task::scheduler::current_fd_get(fd).is_none() {
        return linux_err(EBADF);
    }
    if list_ptr != 0 && size != 0 && !handlers::helpers::is_user_range_accessible(list_ptr, size) {
        return linux_err(EFAULT);
    }
    0
}

pub(super) fn linux_removexattr(path_ptr: u64, name_ptr: u64, nofollow: bool) -> u64 {
    match linux_check_xattr_path(path_ptr, nofollow) {
        Ok(()) => {}
        Err(errno) => return linux_err(errno),
    }
    if name_ptr == 0 || super::handlers::helpers::read_user_str_safe(name_ptr).is_none() {
        return linux_err(EFAULT);
    }
    linux_err(ENODATA)
}

pub(super) fn linux_fremovexattr(fd: u32, name_ptr: u64) -> u64 {
    if crate::task::scheduler::current_fd_get(fd).is_none() {
        return linux_err(EBADF);
    }
    if name_ptr == 0 || super::handlers::helpers::read_user_str_safe(name_ptr).is_none() {
        return linux_err(EFAULT);
    }
    linux_err(ENODATA)
}

fn linux_check_xattr_path(path_ptr: u64, nofollow: bool) -> Result<(), i32> {
    if path_ptr == 0 {
        return Err(EFAULT);
    }
    let raw_path = super::handlers::helpers::read_user_str_safe(path_ptr).ok_or(EFAULT)?;
    if raw_path.is_empty() {
        return Err(ENOENT);
    }
    let abs = linux_absolute_path(raw_path);
    if linux_proc_file_id(&abs).is_some() {
        return Ok(());
    }
    let path = linux_translate_path(&abs);
    let path = linux_resolve_translated_path(&path, !nofollow, false)?;
    let ret = if nofollow {
        crate::fs::vfs::lstat(&path)
    } else {
        crate::fs::vfs::stat(&path)
    };
    ret.map(|_| ()).map_err(fs_errno)
}
