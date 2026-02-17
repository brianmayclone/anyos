use alloc::string::String;

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
}

impl FileFlags {
    pub const READ_ONLY: FileFlags = FileFlags {
        read: true,
        write: false,
        append: false,
        create: false,
        truncate: false,
    };

    pub const READ_WRITE: FileFlags = FileFlags {
        read: true,
        write: true,
        append: false,
        create: false,
        truncate: false,
    };

    pub const CREATE_WRITE: FileFlags = FileFlags {
        read: false,
        write: true,
        append: false,
        create: true,
        truncate: true,
    };
}

/// An open file descriptor entry
pub struct OpenFile {
    pub fd: FileDescriptor,
    pub path: String,
    pub file_type: FileType,
    pub flags: FileFlags,
    pub position: u32,
    pub size: u32,
    pub fs_id: u32,        // Which filesystem this belongs to
    pub inode: u32,        // Filesystem-specific identifier (start cluster for FAT)
    pub parent_cluster: u32, // Parent directory cluster (0 = root)
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
