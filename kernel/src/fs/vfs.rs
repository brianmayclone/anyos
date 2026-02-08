use crate::fs::fat::FatFs;
use crate::fs::file::{DirEntry, FileDescriptor, FileFlags, FileType, OpenFile, SeekFrom};
use crate::sync::spinlock::Spinlock;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsType {
    Fat,
    DevFs,
}

/// Filesystem driver trait
pub trait Filesystem {
    fn read(&self, inode: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write(&self, inode: u32, offset: u32, buf: &[u8]) -> Result<usize, FsError>;
    fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError>;
    fn readdir(&self, inode: u32) -> Result<Vec<DirEntry>, FsError>;
    fn create(&self, parent_inode: u32, name: &str, file_type: FileType) -> Result<u32, FsError>;
    fn delete(&self, parent_inode: u32, name: &str) -> Result<(), FsError>;
}

#[derive(Debug)]
pub enum FsError {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    NoSpace,
    IoError,
    InvalidPath,
    TooManyOpenFiles,
    BadFd,
}

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

pub fn open(path: &str, flags: FileFlags) -> Result<FileDescriptor, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    if state.open_files.len() >= MAX_OPEN_FILES {
        return Err(FsError::TooManyOpenFiles);
    }

    // Look up file in FAT filesystem
    let (inode, file_type, size) = if let Some(ref fat) = state.fat_fs {
        fat.lookup(path)?
    } else {
        return Err(FsError::NotFound);
    };

    let fd = state.next_fd;
    state.next_fd += 1;

    let file = OpenFile {
        fd,
        path: String::from(path),
        file_type,
        flags,
        position: 0,
        size,
        fs_id: 0,
        inode, // start cluster for FAT
    };

    state.open_files.push(Some(file));
    Ok(fd)
}

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

pub fn write(fd: FileDescriptor, buf: &[u8]) -> Result<usize, FsError> {
    let _vfs = VFS.lock();
    // Write is not yet supported for FAT16 (read-only for now)
    let _ = (fd, buf);
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
