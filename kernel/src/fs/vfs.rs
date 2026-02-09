//! Virtual File System (VFS) -- unified interface for file descriptors, open/read/write/close.
//! Delegates to the mounted FAT16 filesystem and manages the global open file table.

use crate::fs::fat::FatFs;
use crate::fs::file::{DirEntry, FileDescriptor, FileFlags, FileType, OpenFile, SeekFrom};
use crate::sync::spinlock::Spinlock;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Maximum number of simultaneously open file descriptors.
const MAX_OPEN_FILES: usize = 256;

/// FAT16 partition start sector (must match mkimage.py --fs-start)
const FAT16_PARTITION_LBA: u32 = 4096;

static VFS: Spinlock<Option<VfsState>> = Spinlock::new(None);

struct VfsState {
    open_files: Vec<Option<OpenFile>>,
    next_fd: FileDescriptor,
    mount_points: Vec<MountPoint>,
    fat_fs: Option<FatFs>,
}

struct MountPoint {
    path: String,
    fs_type: FsType,
    device_id: u32,
}

/// Supported filesystem types for mount points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsType {
    /// FAT12/16/32 filesystem on disk.
    Fat,
    /// In-memory device filesystem (/dev).
    DevFs,
}

/// Trait that all filesystem drivers must implement for the VFS.
pub trait Filesystem {
    /// Read bytes from a file identified by inode at the given offset.
    fn read(&self, inode: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError>;
    /// Write bytes to a file identified by inode at the given offset.
    fn write(&self, inode: u32, offset: u32, buf: &[u8]) -> Result<usize, FsError>;
    /// Look up a path and return `(inode, file_type, size)`.
    fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError>;
    /// List entries in a directory identified by inode.
    fn readdir(&self, inode: u32) -> Result<Vec<DirEntry>, FsError>;
    /// Create a new file or directory under the given parent inode.
    fn create(&self, parent_inode: u32, name: &str, file_type: FileType) -> Result<u32, FsError>;
    /// Delete a file or directory by name under the given parent inode.
    fn delete(&self, parent_inode: u32, name: &str) -> Result<(), FsError>;
}

/// Filesystem operation error codes.
#[derive(Debug)]
pub enum FsError {
    /// File or directory not found.
    NotFound,
    /// Insufficient permissions for the operation.
    PermissionDenied,
    /// A file or directory with that name already exists.
    AlreadyExists,
    /// Expected a directory but found a file.
    NotADirectory,
    /// Expected a file but found a directory.
    IsADirectory,
    /// No free clusters or directory entry slots remaining.
    NoSpace,
    /// Low-level disk I/O failure.
    IoError,
    /// Malformed or empty path.
    InvalidPath,
    /// Open file table is full.
    TooManyOpenFiles,
    /// File descriptor is not valid or not open.
    BadFd,
}

/// Split a path into (parent_dir, filename).
/// "/system/hello.txt" → ("/system", "hello.txt")
/// "/hello.txt" → ("/", "hello.txt")
fn split_parent_name(path: &str) -> Result<(&str, &str), FsError> {
    let path = path.trim_end_matches('/');
    if path.is_empty() || path == "/" {
        return Err(FsError::InvalidPath);
    }
    match path.rfind('/') {
        Some(0) => Ok(("/", &path[1..])),
        Some(pos) => Ok((&path[..pos], &path[pos + 1..])),
        None => Err(FsError::InvalidPath),
    }
}

/// Initialize the VFS, reserving file descriptors 0-2 for stdin/stdout/stderr.
pub fn init() {
    let mut vfs = VFS.lock();
    *vfs = Some(VfsState {
        open_files: Vec::new(),
        next_fd: 3, // 0=stdin, 1=stdout, 2=stderr
        mount_points: Vec::new(),
        fat_fs: None,
    });

    // Reserve fd 0, 1, 2
    let state = vfs.as_mut().unwrap();
    for _ in 0..3 {
        state.open_files.push(None);
    }

    crate::serial_println!("[OK] VFS initialized");
}

/// Mount a filesystem at the given path. For FAT, reads the BPB from disk.
pub fn mount(path: &str, fs_type: FsType, device_id: u32) {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().expect("VFS not initialized");

    if fs_type == FsType::Fat {
        match FatFs::new(device_id, FAT16_PARTITION_LBA) {
            Ok(fat) => {
                state.fat_fs = Some(fat);
                crate::serial_println!("  Mounted FAT16 at '{}'", path);
            }
            Err(_) => {
                crate::serial_println!("  Failed to mount FAT16 at '{}'", path);
            }
        }
    }

    state.mount_points.push(MountPoint {
        path: String::from(path),
        fs_type,
        device_id,
    });
}

/// Open a file by path with the given flags. Returns a file descriptor on success.
pub fn open(path: &str, flags: FileFlags) -> Result<FileDescriptor, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    if state.open_files.len() >= MAX_OPEN_FILES {
        return Err(FsError::TooManyOpenFiles);
    }

    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;

    // Try to look up the file
    let lookup_result = fat.lookup(path);

    let (inode, file_type, size, parent_cluster) = match lookup_result {
        Ok((inode, file_type, size)) => {
            // File exists
            if flags.truncate && flags.write {
                // Truncate: need parent info
                let (parent_path, filename) = split_parent_name(path)?;
                let (parent_cluster, _, _) = fat.lookup(parent_path)?;
                fat.truncate_file(parent_cluster, filename)?;
                (0u32, file_type, 0u32, parent_cluster)
            } else {
                // Get parent cluster for potential writes
                let parent_cluster = if flags.write {
                    let (parent_path, _) = split_parent_name(path)?;
                    fat.lookup(parent_path).map(|(c, _, _)| c).unwrap_or(0)
                } else {
                    0
                };
                (inode, file_type, size, parent_cluster)
            }
        }
        Err(FsError::NotFound) if flags.create => {
            // File doesn't exist but create flag is set
            let (parent_path, filename) = split_parent_name(path)?;
            let (parent_cluster, parent_type, _) = fat.lookup(parent_path)?;
            if parent_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            fat.create_file(parent_cluster, filename)?;
            (0u32, FileType::Regular, 0u32, parent_cluster)
        }
        Err(e) => return Err(e),
    };

    let fd = state.next_fd;
    state.next_fd += 1;

    let position = if flags.append { size } else { 0 };

    let file = OpenFile {
        fd,
        path: String::from(path),
        file_type,
        flags,
        position,
        size,
        fs_id: 0,
        inode,
        parent_cluster,
    };

    state.open_files.push(Some(file));
    Ok(fd)
}

/// Close an open file descriptor, releasing its slot in the open file table.
pub fn close(fd: FileDescriptor) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    for entry in state.open_files.iter_mut() {
        if let Some(file) = entry {
            if file.fd == fd {
                *entry = None;
                return Ok(());
            }
        }
    }

    Err(FsError::BadFd)
}

/// Read bytes from an open file into `buf`. Returns the number of bytes read (0 at EOF).
pub fn read(fd: FileDescriptor, buf: &mut [u8]) -> Result<usize, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Find open file
    let file = state.open_files.iter_mut()
        .flatten()
        .find(|f| f.fd == fd)
        .ok_or(FsError::BadFd)?;

    if file.position >= file.size {
        return Ok(0); // EOF
    }

    let remaining = (file.size - file.position) as usize;
    let to_read = buf.len().min(remaining);

    let bytes_read = if let Some(ref fat) = state.fat_fs {
        fat.read_file(file.inode, file.position, &mut buf[..to_read])?
    } else {
        return Err(FsError::IoError);
    };

    file.position += bytes_read as u32;
    Ok(bytes_read)
}

/// Write bytes from `buf` to an open file. Returns the number of bytes written.
pub fn write(fd: FileDescriptor, buf: &[u8]) -> Result<usize, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Find open file
    let file = state.open_files.iter_mut()
        .flatten()
        .find(|f| f.fd == fd)
        .ok_or(FsError::BadFd)?;

    if !file.flags.write {
        return Err(FsError::PermissionDenied);
    }

    let old_inode = file.inode;
    let old_size = file.size;
    let position = file.position;
    let parent_cluster = file.parent_cluster;

    // Extract filename from path for directory entry update
    let path_clone = file.path.clone();
    let filename = path_clone.rsplit('/').next().unwrap_or("");

    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;
    let (new_cluster, new_size) = fat.write_file(old_inode, position, buf, old_size)?;

    // Update directory entry if cluster or size changed
    if new_cluster != old_inode || new_size != old_size {
        fat.update_entry(parent_cluster, filename, new_size, new_cluster)?;
    }

    // Update open file metadata
    let file = state.open_files.iter_mut()
        .flatten()
        .find(|f| f.fd == fd)
        .ok_or(FsError::BadFd)?;
    file.inode = new_cluster;
    file.size = new_size;
    file.position = position + buf.len() as u32;

    Ok(buf.len())
}

/// Read directory entries at a given path.
pub fn read_dir(path: &str) -> Result<Vec<DirEntry>, FsError> {
    let vfs = VFS.lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    if let Some(ref fat) = state.fat_fs {
        let (cluster, file_type, _size) = fat.lookup(path)?;
        if file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        fat.read_dir(cluster)
    } else {
        Err(FsError::NotFound)
    }
}

/// Read an entire file into a Vec<u8>.
pub fn read_file_to_vec(path: &str) -> Result<Vec<u8>, FsError> {
    let vfs = VFS.lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    if let Some(ref fat) = state.fat_fs {
        let (cluster, file_type, size) = fat.lookup(path)?;
        if file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        fat.read_file_all(cluster, size)
    } else {
        Err(FsError::NotFound)
    }
}

/// Delete a file or empty directory at the given path.
pub fn delete(path: &str) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;

    let (parent_path, filename) = split_parent_name(path)?;
    let (parent_cluster, _, _) = fat.lookup(parent_path)?;
    fat.delete_file(parent_cluster, filename)
}

/// Create a directory at the given path.
pub fn mkdir(path: &str) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;

    let (parent_path, dirname) = split_parent_name(path)?;
    let (parent_cluster, parent_type, _) = fat.lookup(parent_path)?;
    if parent_type != FileType::Directory {
        return Err(FsError::NotADirectory);
    }
    fat.create_dir(parent_cluster, dirname)?;
    Ok(())
}

/// Seek within an open file. Returns new position.
pub fn lseek(fd: FileDescriptor, offset: i32, whence: u32) -> Result<u32, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    let file = state.open_files.iter_mut()
        .flatten()
        .find(|f| f.fd == fd)
        .ok_or(FsError::BadFd)?;

    let new_pos = match whence {
        0 => {
            // SEEK_SET
            if offset < 0 { return Err(FsError::InvalidPath); }
            offset as u32
        }
        1 => {
            // SEEK_CUR
            if offset < 0 {
                file.position.checked_sub((-offset) as u32).ok_or(FsError::InvalidPath)?
            } else {
                file.position + offset as u32
            }
        }
        2 => {
            // SEEK_END
            if offset < 0 {
                file.size.checked_sub((-offset) as u32).ok_or(FsError::InvalidPath)?
            } else {
                file.size + offset as u32
            }
        }
        _ => return Err(FsError::InvalidPath),
    };

    file.position = new_pos;
    Ok(new_pos)
}

/// Get file info by fd. Returns (file_type, size, position).
pub fn fstat(fd: FileDescriptor) -> Result<(FileType, u32, u32), FsError> {
    let vfs = VFS.lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    let file = state.open_files.iter()
        .flatten()
        .find(|f| f.fd == fd)
        .ok_or(FsError::BadFd)?;

    Ok((file.file_type, file.size, file.position))
}

/// Truncate a file to zero length.
pub fn truncate(path: &str) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;

    let (parent_path, filename) = split_parent_name(path)?;
    let (parent_cluster, _, _) = fat.lookup(parent_path)?;
    fat.truncate_file(parent_cluster, filename)
}
