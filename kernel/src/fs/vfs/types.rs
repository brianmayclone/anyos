//! Shared VFS-facing types and traits.

use crate::fs::file::{DirEntry, FileType};
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

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
    /// CoreFS (bare-metal native filesystem, `no_std`).
    ///
    /// Siehe [`crate::fs::corefs`]. Aktuell read-only-Mount über
    /// `corefs_core::storage::ondisk::reader::OdfReader`.
    CoreFs,
    /// FUSE userspace filesystem.
    ///
    /// Der tatsächliche Treiber lebt in einem Userspace-Daemon (z. B.
    /// `corefsd`), der Requests via `/dev/fuse` empfängt und Replies
    /// zurückschreibt. Der Kernel hält für diesen Mount nur die
    /// [`crate::fs::fuse::FuseSession`] und leitet VFS-Calls darauf
    /// weiter. Dispatch-Integration ist aktuell minimal (Phase 5.7) —
    /// siehe [`crate::fs::fuse`].
    Fuse,
}

/// Trait that all filesystem drivers must implement for the VFS.
///
/// A [`Filesystem`] instance is the complete driver for one mounted
/// volume (root or sub-mount) — it owns the block device, the dirty
/// state, any in-memory caches, and exposes everything the VFS
/// dispatch layer needs to service user syscalls against that mount.
///
/// # Design
///
/// The trait is deliberately **path- and inode-based**, with inodes
/// represented as opaque `u32` cookies.  Cluster-based filesystems
/// (FAT/exFAT/NTFS) encode their cluster number into the `u32`
/// internally — see `crate::fs::exfat::decode_inode`.  Path-based
/// filesystems (CoreFS) use a monotonic counter.  The VFS never
/// inspects the encoding.
///
/// # Defaults
///
/// Methods that require write access or filesystem-specific features
/// (symlinks, permissions, stat-time) have default implementations
/// returning [`FsError::PermissionDenied`].  Read-only drivers (NTFS,
/// ISO 9660 when surfaced via this trait) inherit those defaults as
/// "operation not supported" — keeping each driver's impl block
/// limited to the methods it genuinely supports.
///
/// The `Send + Sync` bound makes the trait object storable behind
/// `VfsState`'s `Mutex` so the whole root-FS API is dispatched
/// generically.
pub trait Filesystem: Send + Sync {
    // -----------------------------------------------------------------
    // Required: read, write, namespace
    // -----------------------------------------------------------------

    /// Read bytes from a file identified by inode at the given offset.
    fn read(&self, inode: u32, offset: u64, buf: &mut [u8]) -> Result<usize, FsError>;
    /// Write bytes to a file identified by inode at the given offset.
    fn write(&self, inode: u32, offset: u64, buf: &[u8]) -> Result<usize, FsError>;
    /// Look up a path and return `(inode, file_type, size)`.
    fn lookup(&self, path: &str) -> Result<(u32, FileType, u64), FsError>;
    /// List entries in a directory identified by inode.
    fn readdir(&self, inode: u32) -> Result<Vec<DirEntry>, FsError>;
    /// Create a new file or directory under the given parent inode.
    /// Returns the new child's inode.
    fn create(&self, parent_inode: u32, name: &str, file_type: FileType) -> Result<u32, FsError>;
    /// Delete a file or directory by name under the given parent inode.
    fn delete(&self, parent_inode: u32, name: &str) -> Result<(), FsError>;

    // -----------------------------------------------------------------
    // Metadata (default: derive minimal info from lookup + readdir)
    // -----------------------------------------------------------------

    /// Full stat for `path`, including permission bits and mtime.
    ///
    /// Default implementation falls back to [`Filesystem::lookup`] +
    /// zero'd permissions — drivers that track uid/gid/mode/mtime in
    /// their on-disk metadata should override.
    fn stat(&self, path: &str) -> Result<StatResult, FsError> {
        let (_inode, file_type, size) = self.lookup(path)?;
        Ok(StatResult {
            file_type,
            size,
            is_symlink: false,
            uid: 0,
            gid: 0,
            mode: 0o777,
            mtime: 0,
        })
    }

    /// Volume-level statistics (total / used / free).  Drivers that
    /// cannot compute this cheaply return [`FsError::NotSupported`].
    fn statfs(&self) -> Result<StatFs, FsError> {
        Err(FsError::NotSupported)
    }

    // -----------------------------------------------------------------
    // Mutating ops (default: not supported)
    // -----------------------------------------------------------------

    /// Move / rename an entry.  Cross-directory renames are allowed.
    fn rename(&self, _old_path: &str, _new_path: &str) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    /// Resize a file.  Growing is expected to zero-fill; shrinking
    /// releases any storage past `size`.
    fn truncate(&self, _inode: u32, _size: u64) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    /// Path-based variant of [`Filesystem::truncate`] (shrink to zero
    /// only — matches POSIX `truncate(path, 0)` use case).  Default impl
    /// resolves the path and delegates; overridden by filesystems whose
    /// on-disk layout keys off (parent, name).
    fn truncate_by_path(&self, path: &str) -> Result<(), FsError> {
        let (inode, ft, _) = self.lookup(path)?;
        if ft == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        self.truncate(inode, 0)
    }

    /// Update the POSIX mode bits (permissions + type flags).
    fn set_mode(&self, _inode: u32, _mode: u16) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    /// Update the owner / group of an inode.
    fn set_owner(&self, _inode: u32, _uid: u16, _gid: u16) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    /// Path-based variant of [`Filesystem::set_mode`].  Default impl
    /// resolves the path to an inode and delegates — filesystems whose
    /// on-disk layout keys metadata off `(parent, name)` (exFAT) can
    /// override to skip the intermediate inode encoding.
    fn set_mode_by_path(&self, path: &str, mode: u16) -> Result<(), FsError> {
        let (inode, _, _) = self.lookup(path)?;
        self.set_mode(inode, mode)
    }

    /// Path-based variant of [`Filesystem::set_owner`]; see
    /// [`Filesystem::set_mode_by_path`].
    fn set_owner_by_path(&self, path: &str, uid: u16, gid: u16) -> Result<(), FsError> {
        let (inode, _, _) = self.lookup(path)?;
        self.set_owner(inode, uid, gid)
    }

    // -----------------------------------------------------------------
    // Symlinks (default: not supported)
    // -----------------------------------------------------------------

    /// Create a symbolic link at `link_path` pointing to `target`.
    fn create_symlink(&self, _link_path: &str, _target: &str) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }

    /// Read the target of a symbolic-link inode.
    fn readlink(&self, _inode: u32) -> Result<String, FsError> {
        Err(FsError::NotSupported)
    }

    // -----------------------------------------------------------------
    // Sync / lifecycle
    // -----------------------------------------------------------------

    /// Flush dirty metadata / caches to the underlying device.
    /// Default is a no-op — drivers with in-memory state should
    /// override (CoreFS, ExFat).
    fn sync(&self) -> Result<(), FsError> {
        Ok(())
    }

    /// Returns `true` when the volume was opened read-only (e.g. NTFS
    /// is always read-only, an ISO 9660 CD is always read-only).
    fn read_only(&self) -> bool {
        false
    }

    // -----------------------------------------------------------------
    // Type-erasure helpers
    // -----------------------------------------------------------------

    /// Type-erased reference for downcasting.  Used by the VFS to
    /// access driver-specific APIs (e.g. exFAT cluster bookkeeping for
    /// close-time dirent commits) without resorting to per-FS branches
    /// at every call site.  Each implementation simply returns `self`.
    fn as_any(&self) -> &dyn Any;

    // -----------------------------------------------------------------
    // Close-time / write-time housekeeping
    // -----------------------------------------------------------------

    /// Commit the file's directory-entry metadata (size, first cluster,
    /// mtime) after writes.  Cluster-based filesystems (exFAT, FAT)
    /// override to write the dirent back to disk; transactional
    /// filesystems (CoreFS) inherit the no-op default because writes
    /// persist immediately via [`Filesystem::write`].
    ///
    /// `parent_inode`/`name` identify the directory entry; `new_inode`
    /// reflects any cluster reallocation that happened during writes.
    fn commit_open_file(
        &self,
        _parent_inode: u32,
        _name: &str,
        _new_inode: u32,
        _new_size: u64,
    ) -> Result<(), FsError> {
        Ok(())
    }
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
    /// Directory is not empty (e.g. rmdir / rename-over-non-empty-dir).
    DirectoryNotEmpty,
    /// The filesystem does not support the requested operation (e.g.
    /// symlinks on FAT/NTFS, statfs on FUSE without a Statfs reply).
    /// Distinct from [`FsError::PermissionDenied`], which implies an
    /// ACL / mode-bit rejection.
    NotSupported,
}

/// Stat result with permission info.
pub struct StatResult {
    pub file_type: FileType,
    pub size: u64,
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
