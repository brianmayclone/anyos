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
    pub fs_id: u32,     // Which filesystem this belongs to
    pub inode: u32,     // Filesystem-specific identifier
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: u32,
}
