use super::*;

pub(super) fn linux_translate_user_path(path_ptr: u64) -> Option<String> {
    super::handlers::helpers::read_user_str_safe(path_ptr).map(linux_translate_path)
}

pub(super) fn linux_translate_at_path(dirfd: u64, path_ptr: u64) -> Result<String, i32> {
    let path = super::handlers::helpers::read_user_str_safe(path_ptr).ok_or(EFAULT)?;
    if path.starts_with('/') || (dirfd as i32) == LINUX_AT_FDCWD {
        return Ok(linux_translate_path(path));
    }
    let base = linux_fd_path(dirfd as u32)?;
    Ok(crate::fs::path::normalize(&alloc::format!(
        "{}/{}", base, path
    )))
}

pub(super) fn linux_translate_path(path: &str) -> String {
    let abs = linux_absolute_path(path);
    linux_translate_absolute_path(&abs)
}

pub(super) fn linux_translate_absolute_path(abs: &str) -> String {
    match abs {
        "/dev/null" | "/dev/zero" | "/dev/console" => return String::from(abs),
        "/dev/tty" => return String::from("/dev/console"),
        _ => {}
    }
    let rootfs = current_linux_rootfs();
    if abs == "/" {
        rootfs
    } else {
        alloc::format!("{}{}", rootfs, abs)
    }
}

pub(super) fn linux_absolute_path(path: &str) -> String {
    if path.starts_with('/') {
        crate::fs::path::normalize(path)
    } else {
        let mut cwd_buf = [0u8; 512];
        let cwd_len = crate::task::scheduler::current_thread_cwd(&mut cwd_buf);
        let cwd = core::str::from_utf8(&cwd_buf[..cwd_len]).unwrap_or("/");
        if cwd == "/" || cwd.is_empty() {
            crate::fs::path::normalize(&alloc::format!("/{}", path))
        } else {
            crate::fs::path::normalize(&alloc::format!("{}/{}", cwd, path))
        }
    }
}

pub(super) fn linux_fd_path(fd: u32) -> Result<String, i32> {
    let entry = crate::task::scheduler::current_fd_get(fd).ok_or(EBADF)?;
    let global_id = match entry.kind {
        crate::fs::fd_table::FdKind::File { global_id } => global_id,
        _ => return Err(EBADF),
    };
    crate::fs::vfs::get_fd_path(global_id).map_err(fs_errno)
}

pub(super) fn linux_strip_rootfs(path: &str) -> String {
    let rootfs = current_linux_rootfs();
    if path == rootfs {
        String::from("/")
    } else if path.starts_with(&rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/') {
        String::from(&path[rootfs.len()..])
    } else {
        String::from(path)
    }
}

pub(super) fn linux_resolve_translated_path(
    path: &str,
    follow_last: bool,
    allow_missing_final: bool,
) -> Result<String, i32> {
    let rootfs = current_linux_rootfs();
    let normalized = crate::fs::path::normalize(path);
    if !linux_path_under_rootfs(&rootfs, &normalized) {
        return Ok(normalized);
    }
    linux_resolve_translated_path_inner(&rootfs, &normalized, follow_last, allow_missing_final, 0)
}

pub(super) fn linux_resolve_translated_path_inner(
    rootfs: &str,
    path: &str,
    follow_last: bool,
    allow_missing_final: bool,
    depth: u32,
) -> Result<String, i32> {
    if depth > 16 {
        return Err(ELOOP);
    }
    let normalized = crate::fs::path::normalize(path);
    let rel = linux_rootfs_relative(rootfs, &normalized);
    let components: Vec<&str> = rel
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    let mut current = String::from(rootfs);
    if components.is_empty() {
        return Ok(current);
    }

    for (idx, component) in components.iter().enumerate() {
        let is_last = idx == components.len() - 1;
        let candidate = linux_join_path(&current, component);
        match crate::fs::vfs::lstat(&candidate) {
            Ok(st) if st.is_symlink && (!is_last || follow_last) => {
                let target = crate::fs::vfs::readlink(&candidate).map_err(fs_errno)?;
                let parent = linux_parent_path(&candidate);
                let next =
                    linux_resolve_link_target(rootfs, &parent, &target, &components[idx + 1..]);
                return linux_resolve_translated_path_inner(
                    rootfs,
                    &next,
                    true,
                    allow_missing_final,
                    depth + 1,
                );
            }
            Ok(_) => {
                current = candidate;
            }
            Err(crate::fs::vfs::FsError::NotFound) if is_last && allow_missing_final => {
                return Ok(candidate);
            }
            Err(e) => return Err(fs_errno(e)),
        }
    }
    Ok(current)
}

pub(super) fn linux_path_under_rootfs(rootfs: &str, path: &str) -> bool {
    path == rootfs || (path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/'))
}

fn linux_rootfs_relative<'a>(rootfs: &str, path: &'a str) -> &'a str {
    if path == rootfs {
        ""
    } else if path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/') {
        &path[rootfs.len() + 1..]
    } else {
        path.trim_start_matches('/')
    }
}

pub(super) fn linux_join_path(base: &str, component: &str) -> String {
    if base == "/" {
        alloc::format!("/{}", component)
    } else if base.ends_with('/') {
        alloc::format!("{}{}", base, component)
    } else {
        alloc::format!("{}/{}", base, component)
    }
}

pub(super) fn linux_parent_path(path: &str) -> String {
    let normalized = crate::fs::path::normalize(path);
    match normalized.rfind('/') {
        Some(0) | None => String::from("/"),
        Some(idx) => String::from(&normalized[..idx]),
    }
}

pub(super) fn linux_resolve_link_target(
    rootfs: &str,
    parent: &str,
    target: &str,
    remaining: &[&str],
) -> String {
    let mut path = if target.starts_with('/') {
        if target == "/" {
            String::from(rootfs)
        } else {
            alloc::format!("{}{}", rootfs, target)
        }
    } else {
        linux_join_path(parent, target)
    };
    for component in remaining {
        path = linux_join_path(&path, component);
    }
    crate::fs::path::normalize(&path)
}

pub(super) fn linux_log_path_error(
    op: &str,
    linux_path: &str,
    translated_path: &str,
    resolved_path: &str,
    errno: i32,
) {
    crate::serial_println!(
        "licof linux {}: failed errno={} linux='{}' translated='{}' resolved='{}'",
        op,
        errno,
        linux_path,
        translated_path,
        resolved_path
    );
}

pub(super) fn current_linux_rootfs() -> String {
    let mut buf = [0u8; 512];
    let len = crate::task::scheduler::current_thread_linux_rootfs(&mut buf);
    if len == 0 {
        return String::from(LICOF_ROOTFS);
    }
    core::str::from_utf8(&buf[..len])
        .map(String::from)
        .unwrap_or_else(|_| String::from(LICOF_ROOTFS))
}

pub(super) fn linux_fs_err(e: crate::fs::vfs::FsError) -> u64 {
    linux_err(fs_errno(e))
}

pub(super) fn fs_errno(e: crate::fs::vfs::FsError) -> i32 {
    use crate::fs::vfs::FsError;
    match e {
        FsError::NotFound => 2,
        FsError::PermissionDenied => 13,
        FsError::AlreadyExists => 17,
        FsError::NotADirectory => 20,
        FsError::IsADirectory => 21,
        FsError::NoSpace => 28,
        FsError::IoError => 5,
        FsError::InvalidPath => 22,
        FsError::TooManyOpenFiles => 24,
        FsError::BadFd => 9,
        FsError::DirectoryNotEmpty => 39,
        FsError::NotSupported => 95,
    }
}

pub(super) fn linux_fd_identity(global_id: u32) -> (u64, u64) {
    match crate::fs::vfs::get_fd_path(global_id) {
        Ok(path) => linux_stat_identity(&path),
        Err(_) => (1, global_id as u64 + 2),
    }
}

pub(super) fn linux_stat_identity(path: &str) -> (u64, u64) {
    let dev = if path.starts_with(LICOF_ROOTFS) {
        0x1cf
    } else {
        1
    };
    (dev, linux_path_inode(path))
}

pub(super) fn linux_path_inode(path: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &b in path.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash & 0x7fff_ffff_ffff_ffff
}
