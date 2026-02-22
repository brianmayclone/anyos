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

/// Translate kernel error codes to u32::MAX for backward compatibility.
/// Kernel returns (-errno) as u32 on error (top bit set). Convert to u32::MAX
/// so all existing Rust callers (`== u32::MAX`) keep working.
#[inline]
fn sys_err(v: u32) -> u32 {
    if (v as i32) < 0 { u32::MAX } else { v }
}

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
    let cwd_len = if (cwd_len as i32) >= 0 { cwd_len as usize } else { 0 };
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
    sys_err(syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64))
}

pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    sys_err(syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64))
}

pub fn open(path: &str, flags: u32) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    sys_err(syscall3(SYS_OPEN, buf.as_ptr() as u64, flags as u64, 0))
}

pub fn close(fd: u32) -> u32 {
    sys_err(syscall1(SYS_CLOSE, fd as u64))
}

/// Read directory entries. Returns number of entries (or u32::MAX on error).
/// Each entry is 64 bytes: [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes]
pub fn readdir(path: &str, buf: &mut [u8]) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    sys_err(syscall3(SYS_READDIR, path_buf.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64))
}

/// Get file status (follows symlinks). Returns 0 on success.
/// Writes [type:u32, size:u32, flags:u32, uid:u32, gid:u32, mode:u32, mtime:u32] to buf.
/// flags: bit 0 = is_symlink
pub fn stat(path: &str, stat_buf: &mut [u32; 7]) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    sys_err(syscall2(SYS_STAT, path_buf.as_ptr() as u64, stat_buf.as_mut_ptr() as u64))
}

/// Get file status WITHOUT following final symlink. Returns 0 on success.
/// Writes [type:u32, size:u32, flags:u32, uid:u32, gid:u32, mode:u32, mtime:u32] to buf.
/// flags: bit 0 = is_symlink
pub fn lstat(path: &str, stat_buf: &mut [u32; 7]) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    sys_err(syscall2(SYS_LSTAT, path_buf.as_ptr() as u64, stat_buf.as_mut_ptr() as u64))
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

    sys_err(syscall2(SYS_SYMLINK, target_buf.as_ptr() as u64, link_buf.as_ptr() as u64))
}

/// Read the target of a symbolic link. Returns bytes written, or u32::MAX on error.
pub fn readlink(path: &str, buf: &mut [u8]) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    sys_err(syscall3(SYS_READLINK, path_buf.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64))
}

/// Create a directory. Returns 0 on success, u32::MAX on error.
pub fn mkdir(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    sys_err(syscall1(SYS_MKDIR, buf.as_ptr() as u64))
}

/// Delete a file. Returns 0 on success, u32::MAX on error.
pub fn unlink(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    sys_err(syscall1(SYS_UNLINK, buf.as_ptr() as u64))
}

/// Rename (move) a file or directory. Returns 0 on success, u32::MAX on error.
pub fn rename(old_path: &str, new_path: &str) -> u32 {
    let mut old_buf = [0u8; 257];
    let mut new_buf = [0u8; 257];
    prepare_path(old_path, &mut old_buf);
    prepare_path(new_path, &mut new_buf);
    sys_err(syscall2(SYS_RENAME, old_buf.as_ptr() as u64, new_buf.as_ptr() as u64))
}

/// Truncate a file to zero length. Returns 0 on success, u32::MAX on error.
pub fn truncate(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    sys_err(syscall1(SYS_TRUNCATE, buf.as_ptr() as u64))
}

// Seek whence constants
pub const SEEK_SET: u32 = 0;
pub const SEEK_CUR: u32 = 1;
pub const SEEK_END: u32 = 2;

/// Seek within an open file. Returns new position or u32::MAX on error.
pub fn lseek(fd: u32, offset: i32, whence: u32) -> u32 {
    sys_err(syscall3(SYS_LSEEK, fd as u64, offset as i64 as u64, whence as u64))
}

/// Get file information by fd. Returns 0 on success.
/// Writes [type:u32, size:u32, position:u32, mtime:u32] to stat_buf.
pub fn fstat(fd: u32, stat_buf: &mut [u32; 4]) -> u32 {
    sys_err(syscall2(SYS_FSTAT, fd as u64, stat_buf.as_mut_ptr() as u64))
}

/// Get current working directory. Returns length or u32::MAX on error.
pub fn getcwd(buf: &mut [u8]) -> u32 {
    sys_err(syscall2(SYS_GETCWD, buf.as_mut_ptr() as u64, buf.len() as u64))
}

/// Change current working directory. Returns 0 on success, u32::MAX on error.
pub fn chdir(path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(path, &mut buf);
    sys_err(syscall1(SYS_CHDIR, buf.as_ptr() as u64))
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

    sys_err(syscall3(SYS_MOUNT, mp_buf.as_ptr() as u64, dev_buf.as_ptr() as u64, fs_type as u64))
}

/// Unmount a filesystem.
/// `mount_path`: the mount point to unmount (e.g. "/mnt/cdrom0")
/// Returns 0 on success, u32::MAX on error.
pub fn umount(mount_path: &str) -> u32 {
    let mut buf = [0u8; 257];
    prepare_path(mount_path, &mut buf);
    sys_err(syscall1(SYS_UMOUNT, buf.as_ptr() as u64))
}

/// List all mount points. Writes tab-separated "path\tfstype\n" entries to buf.
/// Returns bytes written, or u32::MAX on error.
pub fn list_mounts(buf: &mut [u8]) -> u32 {
    sys_err(syscall2(SYS_LIST_MOUNTS, buf.as_mut_ptr() as u64, buf.len() as u64))
}

/// Change file permissions. Returns 0 on success.
pub fn chmod(path: &str, mode: u16) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    sys_err(syscall2(SYS_CHMOD, path_buf.as_ptr() as u64, mode as u64))
}

/// Change file owner (root only). Returns 0 on success.
pub fn chown(path: &str, uid: u16, gid: u16) -> u32 {
    let mut path_buf = [0u8; 257];
    prepare_path(path, &mut path_buf);
    sys_err(syscall3(SYS_CHOWN, path_buf.as_ptr() as u64, uid as u64, gid as u64))
}

// =========================================================================
// High-level types — Read/Write traits, File, DirEntry, ReadDir
// =========================================================================

use crate::error;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Trait for types that can read bytes.
pub trait Read {
    /// Read some bytes into `buf`. Returns the number of bytes read (0 = EOF).
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize>;

    /// Read all bytes until EOF into a `Vec<u8>`.
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> error::Result<usize> {
        let mut total = 0;
        let mut tmp = [0u8; 1024];
        loop {
            let n = self.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&tmp[..n]);
            total += n;
        }
        Ok(total)
    }

    /// Read all bytes until EOF into a `String`.
    fn read_to_string(&mut self, out: &mut String) -> error::Result<usize> {
        let mut bytes = Vec::new();
        let n = self.read_to_end(&mut bytes)?;
        // Best-effort UTF-8 — replace invalid sequences
        let s = String::from_utf8(bytes).unwrap_or_else(|e| {
            String::from_utf8_lossy(&e.into_bytes()).into_owned()
        });
        out.push_str(&s);
        Ok(n)
    }
}

/// Trait for types that can write bytes.
pub trait Write {
    /// Write some bytes from `buf`. Returns the number of bytes written.
    fn write(&mut self, buf: &[u8]) -> error::Result<usize>;

    /// Write all bytes from `buf`, retrying until complete.
    fn write_all(&mut self, mut buf: &[u8]) -> error::Result<()> {
        while !buf.is_empty() {
            let n = self.write(buf)?;
            if n == 0 {
                return Err(error::Error::BrokenPipe);
            }
            buf = &buf[n..];
        }
        Ok(())
    }

    /// Flush any buffered data. Default is a no-op.
    fn flush(&mut self) -> error::Result<()> {
        Ok(())
    }
}

/// An open file handle with RAII (auto-close on drop).
pub struct File {
    fd: u32,
}

impl File {
    /// Open an existing file for reading.
    pub fn open(path: &str) -> error::Result<File> {
        let fd = open(path, 0);
        if fd == u32::MAX {
            // Re-do the syscall to get the actual errno
            let mut buf = [0u8; 257];
            prepare_path(path, &mut buf);
            let raw = syscall3(SYS_OPEN, buf.as_ptr() as u64, 0, 0);
            return Err(error::Error::from_errno(-(raw as i32) as u32));
        }
        Ok(File { fd })
    }

    /// Create a new file (or truncate existing) for writing.
    pub fn create(path: &str) -> error::Result<File> {
        let flags = O_WRITE | O_CREATE | O_TRUNC;
        let fd = open(path, flags);
        if fd == u32::MAX {
            let mut buf = [0u8; 257];
            prepare_path(path, &mut buf);
            let raw = syscall3(SYS_OPEN, buf.as_ptr() as u64, flags as u64, 0);
            return Err(error::Error::from_errno(-(raw as i32) as u32));
        }
        Ok(File { fd })
    }

    /// Open a file with explicit flags.
    pub fn open_with(path: &str, flags: u32) -> error::Result<File> {
        let fd = open(path, flags);
        if fd == u32::MAX {
            let mut buf = [0u8; 257];
            prepare_path(path, &mut buf);
            let raw = syscall3(SYS_OPEN, buf.as_ptr() as u64, flags as u64, 0);
            return Err(error::Error::from_errno(-(raw as i32) as u32));
        }
        Ok(File { fd })
    }

    /// Get the raw file descriptor.
    pub fn fd(&self) -> u32 {
        self.fd
    }

    /// Get file metadata via fstat.
    /// Returns `[type, size, position, mtime]`.
    pub fn metadata(&self) -> error::Result<[u32; 4]> {
        let mut stat_buf = [0u32; 4];
        let ret = fstat(self.fd, &mut stat_buf);
        if ret == u32::MAX {
            return Err(error::Error::NotFound);
        }
        Ok(stat_buf)
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize> {
        let ret = read(self.fd, buf);
        if ret == u32::MAX {
            return Err(error::Error::NotFound);
        }
        Ok(ret as usize)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> error::Result<usize> {
        let ret = write(self.fd, buf);
        if ret == u32::MAX {
            return Err(error::Error::BrokenPipe);
        }
        Ok(ret as usize)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        close(self.fd);
    }
}

/// Read an entire file into a `String`.
pub fn read_to_string(path: &str) -> error::Result<String> {
    let mut file = File::open(path)?;
    let mut s = String::new();
    file.read_to_string(&mut s)?;
    Ok(s)
}

/// Read an entire file into a `Vec<u8>`.
pub fn read_to_vec(path: &str) -> error::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut v = Vec::new();
    file.read_to_end(&mut v)?;
    Ok(v)
}

/// Write bytes to a file (creates or truncates).
pub fn write_bytes(path: &str, data: &[u8]) -> error::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(data)
}

// =========================================================================
// Directory iteration
// =========================================================================

/// A single directory entry.
pub struct DirEntry {
    /// Entry name.
    pub name: String,
    /// Entry type: 0 = file, 1 = directory, 2 = symlink.
    pub file_type: u8,
    /// File size in bytes.
    pub size: u32,
}

impl DirEntry {
    /// Returns true if this entry is a directory.
    pub fn is_dir(&self) -> bool {
        self.file_type == 1
    }

    /// Returns true if this entry is a regular file.
    pub fn is_file(&self) -> bool {
        self.file_type == 0
    }
}

/// Iterator over directory entries.
pub struct ReadDir {
    entries: Vec<DirEntry>,
    index: usize,
}

impl Iterator for ReadDir {
    type Item = DirEntry;

    fn next(&mut self) -> Option<DirEntry> {
        if self.index < self.entries.len() {
            let i = self.index;
            self.index += 1;
            // Move out of the vec by swapping with a dummy
            Some(core::mem::replace(
                &mut self.entries[i],
                DirEntry { name: String::new(), file_type: 0, size: 0 },
            ))
        } else {
            None
        }
    }
}

/// Read directory entries and return an iterator.
pub fn read_dir(path: &str) -> error::Result<ReadDir> {
    // Allocate buffer for up to 128 entries (128 * 64 = 8192 bytes)
    let mut buf = vec![0u8; 8192];
    let count = readdir(path, &mut buf);
    if count == u32::MAX {
        return Err(error::Error::NotFound);
    }
    let count = count as usize;
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let base = i * 64;
        if base + 64 > buf.len() {
            break;
        }
        let entry_type = buf[base];
        let name_len = buf[base + 1] as usize;
        let size = u32::from_le_bytes([
            buf[base + 4],
            buf[base + 5],
            buf[base + 6],
            buf[base + 7],
        ]);
        let name_start = base + 8;
        let name_end = (name_start + name_len).min(base + 64);
        let name = core::str::from_utf8(&buf[name_start..name_end])
            .unwrap_or("")
            .into();
        entries.push(DirEntry {
            name,
            file_type: entry_type,
            size,
        });
    }
    Ok(ReadDir { entries, index: 0 })
}
