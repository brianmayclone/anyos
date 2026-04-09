//! std::fs compatible filesystem operations.
//!
//! Wraps anyos_std::fs with std-compatible types and traits.

use crate::io::{self, Read, Write, Seek, SeekFrom, BufReader};
use crate::path::{Path, PathBuf};
use alloc::string::String;
use alloc::vec::Vec;

// ── File ────────────────────────────────────────────────────────────────────

/// An open file handle, mirroring std::fs::File.
pub struct File {
    inner: anyos_std::fs::File,
}

impl File {
    /// Open a file in read-only mode.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<File> {
        let path_str = path.as_ref().as_str();
        let inner = anyos_std::fs::File::open(path_str).map_err(io::Error::from_anyos)?;
        Ok(File { inner })
    }

    /// Create a new file or truncate an existing one.
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<File> {
        let path_str = path.as_ref().as_str();
        let inner = anyos_std::fs::File::create(path_str).map_err(io::Error::from_anyos)?;
        Ok(File { inner })
    }

    /// Open a file with specific options.
    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }

    /// Get file metadata.
    pub fn metadata(&self) -> io::Result<Metadata> {
        let meta = self.inner.metadata().map_err(io::Error::from_anyos)?;
        Ok(Metadata::from_raw(meta))
    }

    /// Sync all data to disk.
    pub fn sync_all(&self) -> io::Result<()> {
        anyos_std::fs::fsync(self.inner.fd() as i32);
        Ok(())
    }

    /// Sync data (not metadata) to disk.
    pub fn sync_data(&self) -> io::Result<()> {
        self.sync_all()
    }

    /// Set the file length.
    pub fn set_len(&self, _size: u64) -> io::Result<()> {
        // anyOS doesn't have ftruncate, but truncate exists for paths
        Err(io::Error::new(io::ErrorKind::Other, "set_len not supported on fd"))
    }

    /// Try to clone this file handle.
    pub fn try_clone(&self) -> io::Result<File> {
        // anyOS doesn't have dup() — open the same fd
        Err(io::Error::new(io::ErrorKind::Other, "try_clone not supported"))
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        anyos_std::fs::Read::read(&mut self.inner, buf).map_err(io::Error::from_anyos)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        anyos_std::fs::Write::write(&mut self.inner, buf).map_err(io::Error::from_anyos)
    }

    fn flush(&mut self) -> io::Result<()> {
        anyos_std::fs::fsync(self.inner.fd() as i32);
        Ok(())
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let (offset, whence) = match pos {
            SeekFrom::Start(n) => (n as i32, anyos_std::fs::SEEK_SET),
            SeekFrom::End(n) => (n as i32, anyos_std::fs::SEEK_END),
            SeekFrom::Current(n) => (n as i32, anyos_std::fs::SEEK_CUR),
        };
        let result = anyos_std::fs::lseek(self.inner.fd(), offset, whence);
        if result == u32::MAX {
            Err(io::Error::from_kind(io::ErrorKind::InvalidInput))
        } else {
            Ok(result as u64)
        }
    }
}

// ── OpenOptions ─────────────────────────────────────────────────────────────

/// Builder for opening files with specific options.
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
}

impl OpenOptions {
    pub fn new() -> Self {
        OpenOptions {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,
            create_new: false,
        }
    }

    pub fn read(&mut self, read: bool) -> &mut Self { self.read = read; self }
    pub fn write(&mut self, write: bool) -> &mut Self { self.write = write; self }
    pub fn append(&mut self, append: bool) -> &mut Self { self.append = append; self }
    pub fn truncate(&mut self, truncate: bool) -> &mut Self { self.truncate = truncate; self }
    pub fn create(&mut self, create: bool) -> &mut Self { self.create = create; self }
    pub fn create_new(&mut self, create_new: bool) -> &mut Self { self.create_new = create_new; self }

    pub fn open<P: AsRef<Path>>(&self, path: P) -> io::Result<File> {
        let mut flags = 0u32;
        if self.write || self.append {
            flags |= anyos_std::fs::O_WRITE;
        }
        if self.append {
            flags |= anyos_std::fs::O_APPEND;
        }
        if self.create || self.create_new {
            flags |= anyos_std::fs::O_CREATE;
        }
        if self.truncate {
            flags |= anyos_std::fs::O_TRUNC;
        }

        let path_str = path.as_ref().as_str();

        // Handle create_new: check if file exists first
        if self.create_new {
            let mut stat_buf = [0u32; 7];
            if anyos_std::fs::stat(path_str, &mut stat_buf) != u32::MAX {
                return Err(io::Error::from_kind(io::ErrorKind::AlreadyExists));
            }
        }

        let inner = anyos_std::fs::File::open_with(path_str, flags)
            .map_err(io::Error::from_anyos)?;
        Ok(File { inner })
    }
}

// ── Metadata ────────────────────────────────────────────────────────────────

/// File metadata, mirroring std::fs::Metadata.
#[derive(Debug, Clone)]
pub struct Metadata {
    file_type: u32,
    size: u64,
    mtime: u64,
    mode: u32,
}

impl Metadata {
    fn from_raw(raw: [u32; 4]) -> Self {
        Metadata {
            file_type: raw[0],
            size: raw[1] as u64,
            mtime: raw[3] as u64,
            mode: 0o644,
        }
    }

    fn from_stat(raw: [u32; 7]) -> Self {
        Metadata {
            file_type: raw[0],
            size: raw[1] as u64,
            mtime: raw[6] as u64,
            mode: raw[5],
        }
    }

    pub fn is_file(&self) -> bool { self.file_type == 0 }
    pub fn is_dir(&self) -> bool { self.file_type == 1 }
    pub fn is_symlink(&self) -> bool { self.file_type == 2 }

    pub fn len(&self) -> u64 { self.size }
    pub fn is_empty(&self) -> bool { self.size == 0 }

    pub fn file_type(&self) -> FileType {
        FileType { ty: self.file_type }
    }

    pub fn permissions(&self) -> Permissions {
        Permissions { mode: self.mode }
    }

    pub fn modified(&self) -> io::Result<crate::time::SystemTime> {
        Ok(crate::time::SystemTime { secs: self.mtime })
    }
}

/// File type information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileType {
    ty: u32,
}

impl FileType {
    pub fn is_file(&self) -> bool { self.ty == 0 }
    pub fn is_dir(&self) -> bool { self.ty == 1 }
    pub fn is_symlink(&self) -> bool { self.ty == 2 }
}

/// File permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    pub mode: u32,
}

impl Permissions {
    pub fn readonly(&self) -> bool {
        self.mode & 0o200 == 0
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        if readonly {
            self.mode &= !0o222;
        } else {
            self.mode |= 0o200;
        }
    }

    pub fn mode(&self) -> u32 {
        self.mode
    }
}

// ── DirEntry ────────────────────────────────────────────────────────────────

/// A directory entry, mirroring std::fs::DirEntry.
pub struct DirEntry {
    path: PathBuf,
    name: String,
    file_type: u8,
    size: u32,
}

impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn file_name(&self) -> crate::path::OsString {
        crate::path::OsString::from(self.name.clone())
    }

    pub fn metadata(&self) -> io::Result<Metadata> {
        metadata(&self.path)
    }

    pub fn file_type(&self) -> io::Result<FileType> {
        Ok(FileType { ty: self.file_type as u32 })
    }
}

/// Iterator over directory entries.
pub struct ReadDir {
    entries: alloc::vec::Vec<DirEntry>,
    index: usize,
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<io::Result<DirEntry>> {
        if self.index < self.entries.len() {
            let i = self.index;
            self.index += 1;
            let entry = core::mem::replace(&mut self.entries[i], DirEntry {
                path: PathBuf::new(),
                name: String::new(),
                file_type: 0,
                size: 0,
            });
            Some(Ok(entry))
        } else {
            None
        }
    }
}

// ── Free functions ──────────────────────────────────────────────────────────

/// Read directory entries.
pub fn read_dir<P: AsRef<Path>>(path: P) -> io::Result<ReadDir> {
    let path_str = path.as_ref().as_str();
    let dir = anyos_std::fs::read_dir(path_str).map_err(io::Error::from_anyos)?;

    let mut entries = Vec::new();
    for entry in dir {
        let entry_path = if path_str.ends_with('/') {
            PathBuf::from(alloc::format!("{}{}", path_str, entry.name))
        } else {
            PathBuf::from(alloc::format!("{}/{}", path_str, entry.name))
        };
        entries.push(DirEntry {
            path: entry_path,
            name: entry.name,
            file_type: entry.file_type,
            size: entry.size,
        });
    }
    Ok(ReadDir { entries, index: 0 })
}

/// Read entire file contents as bytes.
pub fn read<P: AsRef<Path>>(path: P) -> io::Result<Vec<u8>> {
    anyos_std::fs::read_to_vec(path.as_ref().as_str()).map_err(io::Error::from_anyos)
}

/// Read entire file contents as string.
pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    anyos_std::fs::read_to_string(path.as_ref().as_str()).map_err(io::Error::from_anyos)
}

/// Write bytes to a file (creates or truncates).
pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
    anyos_std::fs::write_bytes(path.as_ref().as_str(), contents.as_ref())
        .map_err(io::Error::from_anyos)
}

/// Create a directory.
pub fn create_dir<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let ret = anyos_std::fs::mkdir(path.as_ref().as_str());
    if ret == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::Other))
    } else {
        Ok(())
    }
}

/// Create a directory and all parent directories.
pub fn create_dir_all<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let path = path.as_ref().as_str();
    let mut current = String::new();
    for component in path.split('/') {
        if component.is_empty() {
            current.push('/');
            continue;
        }
        if !current.is_empty() && !current.ends_with('/') {
            current.push('/');
        }
        current.push_str(component);

        let mut stat_buf = [0u32; 7];
        if anyos_std::fs::stat(&current, &mut stat_buf) == u32::MAX {
            anyos_std::fs::mkdir(&current);
        }
    }
    Ok(())
}

/// Remove a file.
pub fn remove_file<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let ret = anyos_std::fs::unlink(path.as_ref().as_str());
    if ret == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::NotFound))
    } else {
        Ok(())
    }
}

/// Remove an empty directory.
pub fn remove_dir<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let ret = anyos_std::fs::unlink(path.as_ref().as_str());
    if ret == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::NotFound))
    } else {
        Ok(())
    }
}

/// Remove a directory and all its contents.
pub fn remove_dir_all<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let path = path.as_ref();
    let entries = read_dir(path)?;
    for entry in entries {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            remove_dir_all(&entry.path())?;
        } else {
            remove_file(&entry.path())?;
        }
    }
    remove_dir(path)
}

/// Rename a file or directory.
pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> io::Result<()> {
    let ret = anyos_std::fs::rename(from.as_ref().as_str(), to.as_ref().as_str());
    if ret == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::NotFound))
    } else {
        Ok(())
    }
}

/// Copy a file.
pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> io::Result<u64> {
    let data = read(from)?;
    let len = data.len() as u64;
    write(to, &data)?;
    Ok(len)
}

/// Get file metadata (follows symlinks).
pub fn metadata<P: AsRef<Path>>(path: P) -> io::Result<Metadata> {
    let mut buf = [0u32; 7];
    let ret = anyos_std::fs::stat(path.as_ref().as_str(), &mut buf);
    if ret == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::NotFound))
    } else {
        Ok(Metadata::from_stat(buf))
    }
}

/// Get symlink metadata (does not follow symlinks).
pub fn symlink_metadata<P: AsRef<Path>>(path: P) -> io::Result<Metadata> {
    let mut buf = [0u32; 7];
    let ret = anyos_std::fs::lstat(path.as_ref().as_str(), &mut buf);
    if ret == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::NotFound))
    } else {
        Ok(Metadata::from_stat(buf))
    }
}

/// Create a hard link (not supported on anyOS, creates symlink instead).
pub fn hard_link<P: AsRef<Path>, Q: AsRef<Path>>(original: P, link: Q) -> io::Result<()> {
    let ret = anyos_std::fs::symlink(original.as_ref().as_str(), link.as_ref().as_str());
    if ret == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::Other))
    } else {
        Ok(())
    }
}

/// Read the target of a symbolic link.
pub fn read_link<P: AsRef<Path>>(path: P) -> io::Result<PathBuf> {
    let mut buf = [0u8; 256];
    let len = anyos_std::fs::readlink(path.as_ref().as_str(), &mut buf);
    if len == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::NotFound))
    } else {
        let s = core::str::from_utf8(&buf[..len as usize])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 in symlink"))?;
        Ok(PathBuf::from(s))
    }
}

/// Canonicalize a path (resolve symlinks, make absolute).
pub fn canonicalize<P: AsRef<Path>>(path: P) -> io::Result<PathBuf> {
    let path = path.as_ref();
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let mut cwd_buf = [0u8; 256];
        let len = anyos_std::fs::getcwd(&mut cwd_buf);
        if len == u32::MAX || len == 0 {
            return Err(io::Error::from_kind(io::ErrorKind::Other));
        }
        let cwd = core::str::from_utf8(&cwd_buf[..len as usize])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid CWD"))?;
        Ok(PathBuf::from(cwd).join(path))
    }
}

/// Set permissions on a file.
pub fn set_permissions<P: AsRef<Path>>(path: P, perm: Permissions) -> io::Result<()> {
    let ret = anyos_std::fs::chmod(path.as_ref().as_str(), perm.mode as u16);
    if ret == u32::MAX {
        Err(io::Error::from_kind(io::ErrorKind::NotFound))
    } else {
        Ok(())
    }
}

/// Check if a path exists.
pub fn exists<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref().exists()
}
