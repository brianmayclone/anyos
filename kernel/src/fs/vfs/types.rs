//! Shared VFS-facing types and traits.

use crate::fs::file::{DirEntry, FileType};
use alloc::vec::Vec;

/// Supported filesystem types for mount points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsType {
    /// exFAT filesystem on disk (default for OS image).
    ExFat,
    /// FAT12/16/32 filesystem on disk (secondary mounts).
    Fat,
    /// ISO 9660 filesystem (CD-ROM/DVD-ROM, read-only).
    Iso9660,
    /// NTFS filesystem (read-only).
    Ntfs,
    /// In-memory device filesystem (/dev).
    DevFs,
    /// SMB/CIFS network filesystem.
    Smb,
    /// Overlay filesystem (RamFS over ISO 9660).
    Overlay,
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

/// Stat result with permission info.
pub struct StatResult {
    pub file_type: FileType,
    pub size: u32,
    pub is_symlink: bool,
    pub uid: u16,
    pub gid: u16,
    pub mode: u16,
    /// Modification time as Unix timestamp (seconds since 1970-01-01).
    pub mtime: u32,
}

/// Filesystem statistics for a mount point.
pub struct StatFs {
    /// Total size in bytes.
    pub total_bytes: u64,
    /// Used bytes.
    pub used_bytes: u64,
    /// Free bytes.
    pub free_bytes: u64,
}
