//! Filesystem operations — open, close, read, write, readdir, stat, mkdir, unlink.
//!
//! All path-based functions resolve relative paths (`.`, `./foo`, `foo/bar`)
//! against the process's current working directory via `getcwd`.

use crate::raw::*;

// Open flags (bit flags, match kernel/src/syscall/handlers.rs sys_open)
pub const O_WRITE: u32 = 1;
pub const O_APPEND: u32 = 2;
pub const O_CREATE: u32 = 4;
pub const O_TRUNC: u32 = 8;

/// Resolve a path into a null-terminated absolute path in `buf`.
/// Handles ".", "./relative", and bare relative paths by prepending CWD.
/// Returns the length of the resolved path (not including the null terminator).
pub fn prepare_path(path: &str, buf: &mut [u8; 257]) -> usize {
    if path.is_empty() || path.starts_with('/') {
        let len = path.len().min(256);
        buf[..len].copy_from_slice(&path.as_bytes()[..len]);
        buf[len] = 0;
        return len;
    }

    // Relative path — get CWD
    let mut cwd = [0u8; 256];
    let cwd_len = syscall2(SYS_GETCWD, cwd.as_mut_ptr() as u64, 256);
    let cwd_len = if cwd_len != u32::MAX { cwd_len as usize } else { 0 };
    if cwd_len == 0 {
        cwd[0] = b'/';
    }
    let cwd_len = if cwd_len == 0 { 1 } else { cwd_len };

    // "." alone means just CWD
    if path == "." {
        let len = cwd_len.min(256);
        buf[..len].copy_from_slice(&cwd[..len]);
        buf[len] = 0;
        return len;
    }

    // Build: cwd + "/" + relative_part
    let mut pos = 0;
    for i in 0..cwd_len.min(255) {
        buf[pos] = cwd[i];
        pos += 1;
    }
    if pos > 0 && buf[pos - 1] != b'/' {
        buf[pos] = b'/';
        pos += 1;
    }
    let rel = if path.starts_with("./") {
        &path.as_bytes()[2..]
    } else {
        path.as_bytes()
    };
    for &b in rel {
        if pos >= 256 {
            break;
        }
        buf[pos] = b;
        pos += 1;
    }
    buf[pos] = 0;
    pos
}

pub fn write(fd: u32, buf: &[u8]) -> u32 {
    syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64)
}

pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

pub fn open(path: &str, flags: u32) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    syscall3(SYS_OPEN, buf.as_ptr() as u64, flags as u64, 0)
}

pub fn close(fd: u32) -> u32 {
    syscall1(SYS_CLOSE, fd as u64)
}

/// Read directory entries. Returns number of entries (or u32::MAX on error).
/// Each entry is 64 bytes: [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes]
pub fn readdir(path: &str, buf: &mut [u8]) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    syscall3(SYS_READDIR, path_buf.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Get file status (follows symlinks). Returns 0 on success.
/// Writes [type:u32, size:u32, flags:u32] to buf.
/// flags: bit 0 = is_symlink
pub fn stat(path: &str, stat_buf: &mut [u32; 3]) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    syscall2(SYS_STAT, path_buf.as_ptr() as u64, stat_buf.as_mut_ptr() as u64)
}

/// Get file status WITHOUT following final symlink. Returns 0 on success.
/// Writes [type:u32, size:u32, flags:u32] to buf.
/// flags: bit 0 = is_symlink
pub fn lstat(path: &str, stat_buf: &mut [u32; 3]) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    syscall2(SYS_LSTAT, path_buf.as_ptr() as u64, stat_buf.as_mut_ptr() as u64)
}

/// Create a symbolic link. Returns 0 on success, u32::MAX on error.
/// `target` is the path the symlink points to.
/// `link_path` is where the symlink will be created.
pub fn symlink(target: &str, link_path: &str) -> u32 {
    let mut target_buf = [0u8; 257];
    let tlen = target.len().min(256);
    target_buf[..tlen].copy_from_slice(&target.as_bytes()[..tlen]);
    target_buf[tlen] = 0;

    let mut link_buf = [0u8; 257];
    prepare_path(link_path, &mut link_buf);

    syscall2(SYS_SYMLINK, target_buf.as_ptr() as u64, link_buf.as_ptr() as u64)
}

/// Read the target of a symbolic link. Returns bytes written, or u32::MAX on error.
pub fn readlink(path: &str, buf: &mut [u8]) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    syscall3(SYS_READLINK, path_buf.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Create a directory. Returns 0 on success, u32::MAX on error.
pub fn mkdir(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    syscall1(SYS_MKDIR, buf.as_ptr() as u64)
}

/// Delete a file. Returns 0 on success, u32::MAX on error.
pub fn unlink(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    syscall1(SYS_UNLINK, buf.as_ptr() as u64)
}

/// Truncate a file to zero length. Returns 0 on success, u32::MAX on error.
pub fn truncate(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    syscall1(SYS_TRUNCATE, buf.as_ptr() as u64)
}

// Seek whence constants
pub const SEEK_SET: u32 = 0;
pub const SEEK_CUR: u32 = 1;
pub const SEEK_END: u32 = 2;

/// Seek within an open file. Returns new position or u32::MAX on error.
pub fn lseek(fd: u32, offset: i32, whence: u32) -> u32 {
    syscall3(SYS_LSEEK, fd as u64, offset as i64 as u64, whence as u64)
}

/// Get file information by fd. Returns 0 on success.
/// Writes [type:u32, size:u32, position:u32] to stat_buf.
pub fn fstat(fd: u32, stat_buf: &mut [u32; 3]) -> u32 {
    syscall2(SYS_FSTAT, fd as u64, stat_buf.as_mut_ptr() as u64)
}

/// Get current working directory. Returns length or u32::MAX on error.
pub fn getcwd(buf: &mut [u8]) -> u32 {
    syscall2(SYS_GETCWD, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Change current working directory. Returns 0 on success, u32::MAX on error.
pub fn chdir(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    syscall1(SYS_CHDIR, buf.as_ptr() as u64)
}

/// Check if fd refers to a terminal. Returns 1 for tty, 0 otherwise.
pub fn isatty(fd: u32) -> u32 {
    syscall1(SYS_ISATTY, fd as u64)
}

/// Mount a filesystem.
/// `mount_path`: where to mount (e.g. "/mnt/cdrom0")
/// `device`: device path (e.g. "/dev/cdrom0")
/// `fs_type`: filesystem type (0=fat, 1=iso9660)
/// Returns 0 on success, u32::MAX on error.
pub fn mount(mount_path: &str, device: &str, fs_type: u32) -> u32 {
    let mut mp_buf = [0u8; 257];
    prepare_path(mount_path, &mut mp_buf);

    let mut dev_buf = [0u8; 257];
    prepare_path(device, &mut dev_buf);

    syscall3(SYS_MOUNT, mp_buf.as_ptr() as u64, dev_buf.as_ptr() as u64, fs_type as u64)
}

/// Unmount a filesystem.
/// `mount_path`: the mount point to unmount (e.g. "/mnt/cdrom0")
/// Returns 0 on success, u32::MAX on error.
pub fn umount(mount_path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(mount_path, &mut buf);
    syscall1(SYS_UMOUNT, buf.as_ptr() as u64)
}

/// List all mount points. Writes tab-separated "path\tfstype\n" entries to buf.
/// Returns bytes written, or u32::MAX on error.
pub fn list_mounts(buf: &mut [u8]) -> u32 {
    syscall2(SYS_LIST_MOUNTS, buf.as_mut_ptr() as u64, buf.len() as u64)
}
