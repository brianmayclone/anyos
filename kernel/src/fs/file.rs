//! Shared filesystem data structures used by the VFS and concrete filesystems.

use alloc::string::String;
use alloc::vec::Vec;

pub type FileDescriptor = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Directory,
    Device,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekFrom {
    Start(u32),
    Current(i32),
    End(i32),
}

#[derive(Debug, Clone, Copy)]
pub struct FileFlags {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub create: bool,
    pub truncate: bool,
    /// Request durable writes: commit file metadata and flush caches eagerly.
    pub sync: bool,
}

impl FileFlags {
    pub const READ_ONLY: FileFlags = FileFlags {
        read: true,
        write: false,
        append: false,
        create: false,
        truncate: false,
        sync: false,
    };

    pub const READ_WRITE: FileFlags = FileFlags {
        read: true,
        write: true,
        append: false,
        create: false,
        truncate: false,
        sync: false,
    };

    pub const CREATE_WRITE: FileFlags = FileFlags {
        read: false,
        write: true,
        append: false,
        create: true,
        truncate: true,
        sync: false,
    };
}

/// An open file description (global, shared via refcount across fork/dup).
/// The `fd` field is the global slot index in the VFS open_files table.
pub struct OpenFile {
    pub fd: FileDescriptor,
    pub path: String,
    pub file_type: FileType,
    pub flags: FileFlags,
    pub position: u32,
    pub size: u32,
    pub fs_id: u32,          // Which filesystem this belongs to
    pub inode: u32,          // Filesystem-specific identifier (start cluster for FAT)
    pub parent_cluster: u32, // Parent directory cluster (0 = root)
    /// Reference count: how many per-process FD table entries point to this slot.
    /// Slot is freed when refcount drops to 0.
    pub refcount: u32,
    /// FAT chain seek cache: (byte_offset, cluster) — avoids re-walking
    /// the FAT chain from the start on sequential reads. Updated on every read.
    pub seek_cache_offset: u32,
    pub seek_cache_cluster: u32,
    /// exFAT: directory entry needs a deferred size/cluster commit.
    pub entry_dirty: bool,
    /// exFAT: small sequential append buffer.  This lets 4 KiB writers reach
    /// the filesystem as larger contiguous writes without changing userspace's
    /// visible file offset or size.
    pub append_buffer_offset: u32,
    pub append_buffer: Vec<u8>,
    /// Sequential read-ahead state for files whose backend can cheaply build
    /// sector plans outside the VFS lock.
    pub readahead: ReadAheadState,
}

/// Per-open-file read-ahead state.
///
/// This belongs to the open file description, not the inode, so duplicated file
/// descriptors share a sequential stream while independent opens do not poison
/// each other's prediction state.
#[derive(Debug, Clone, Copy)]
pub struct ReadAheadState {
    /// Offset expected for the next sequential read.
    pub next_offset: u32,
    /// Current adaptive window in bytes.
    pub window_bytes: u32,
    /// Highest file offset already scheduled for prefetch.
    pub prefetched_until: u32,
}

impl ReadAheadState {
    pub const fn new(position: u32) -> Self {
        Self {
            next_offset: position,
            window_bytes: 0,
            prefetched_until: position,
        }
    }

    pub fn reset(&mut self, position: u32) {
        *self = Self::new(position);
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: u32,
    /// True if this entry is a symbolic link.
    pub is_symlink: bool,
    /// Owner user ID.
    pub uid: u16,
    /// Owner group ID.
    pub gid: u16,
    /// Permission mode (12-bit: owner[8-11] | group[4-7] | others[0-3]).
    pub mode: u16,
}
