//! Virtual File System (VFS) -- unified interface for file descriptors, open/read/write/close.
//! Delegates to the mounted FAT16 filesystem and manages the global open file table.

use crate::fs::devfs::DevFs;
use crate::fs::fat::FatFs;
use crate::fs::iso9660::Iso9660Fs;
use crate::fs::file::{DirEntry, FileDescriptor, FileFlags, FileType, OpenFile, SeekFrom};
use crate::sync::mutex::Mutex;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Maximum number of simultaneously open file descriptors.
const MAX_OPEN_FILES: usize = 256;

/// FAT16 partition start sector (must match mkimage.py --fs-start)
const FAT16_PARTITION_LBA: u32 = 8192;

static VFS: Mutex<Option<VfsState>> = Mutex::new(None);

struct VfsState {
    open_files: Vec<Option<OpenFile>>,
    next_fd: FileDescriptor,
    mount_points: Vec<MountPoint>,
    fat_fs: Option<FatFs>,
    iso9660_fs: Option<Iso9660Fs>,
    devfs: Option<DevFs>,
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
    /// ISO 9660 filesystem (CD-ROM/DVD-ROM, read-only).
    Iso9660,
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

/// Check if a path targets the /dev filesystem.
fn is_dev_path(path: &str) -> bool {
    path == "/dev" || path.starts_with("/dev/")
}

/// Extract the device name from a /dev path (strips "/dev/" prefix).
fn dev_name(path: &str) -> &str {
    if path.len() > 5 { &path[5..] } else { "" }
}

/// Check if a path targets a /mnt/ mount point.
/// Returns (mount_path, relative_path) if matched.
/// The relative_path always starts with "/" (e.g. "/" for the mount root, "/file.txt" for a file).
fn find_mnt_mount<'a>(path: &'a str, mount_points: &[MountPoint]) -> Option<(&'a str, &'a str)> {
    // Find longest matching mount point under /mnt/
    let mut best_len: usize = 0;
    let mut found = false;
    for mp in mount_points {
        if !mp.path.starts_with("/mnt/") {
            continue;
        }
        let mp_path = mp.path.as_str();
        // Match exact path or path with trailing /
        if path == mp_path {
            if mp_path.len() > best_len {
                best_len = mp_path.len();
                found = true;
            }
        } else if path.len() > mp_path.len()
            && path.as_bytes()[mp_path.len()] == b'/'
            && path.starts_with(mp_path)
        {
            if mp_path.len() > best_len {
                best_len = mp_path.len();
                found = true;
            }
        }
    }
    if found {
        let relative = if path.len() > best_len {
            &path[best_len..]  // starts with "/"
        } else {
            "/"
        };
        Some((&path[..best_len], relative))
    } else {
        None
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
        iso9660_fs: None,
        devfs: None,
    });

    // Reserve fd 0, 1, 2
    let state = vfs.as_mut().unwrap();
    for _ in 0..3 {
        state.open_files.push(None);
    }

    crate::serial_println!("[OK] VFS initialized");
}

/// Check if the root FAT16 filesystem is mounted.
pub fn has_root_fat() -> bool {
    let vfs = VFS.lock();
    if let Some(ref state) = *vfs {
        state.fat_fs.is_some()
    } else {
        false
    }
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
    } else if fs_type == FsType::Iso9660 {
        match Iso9660Fs::new() {
            Ok(iso) => {
                state.iso9660_fs = Some(iso);
                crate::serial_println!("  Mounted ISO 9660 at '{}'", path);
            }
            Err(_) => {
                crate::serial_println!("  Failed to mount ISO 9660 at '{}'", path);
            }
        }
    }

    state.mount_points.push(MountPoint {
        path: String::from(path),
        fs_type,
        device_id,
    });
}

/// Mount the device filesystem at /dev, bridging built-in virtual devices
/// with HAL-registered hardware devices.
pub fn mount_devfs() {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().expect("VFS not initialized");
    let mut devfs = DevFs::new();
    devfs.populate_from_hal();
    state.devfs = Some(devfs);
    state.mount_points.push(MountPoint {
        path: String::from("/dev"),
        fs_type: FsType::DevFs,
        device_id: 0,
    });
    crate::serial_println!("  Mounted DevFs at '/dev'");
}

/// Open a file by path with the given flags. Returns a file descriptor on success.
pub fn open(path: &str, flags: FileFlags) -> Result<FileDescriptor, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Count actually occupied slots (not None holes left by close())
    let active_count = state.open_files.iter().filter(|e| e.is_some()).count();
    if active_count >= MAX_OPEN_FILES {
        return Err(FsError::TooManyOpenFiles);
    }

    // --- DevFs path ---
    if is_dev_path(path) {
        let name = dev_name(path);
        if name.is_empty() {
            return Err(FsError::IsADirectory);
        }
        let devfs = state.devfs.as_ref().ok_or(FsError::NotFound)?;
        let idx = devfs.lookup(name).ok_or(FsError::NotFound)?;

        let fd = state.next_fd;
        state.next_fd += 1;

        let file = OpenFile {
            fd,
            path: String::from(path),
            file_type: FileType::Device,
            flags,
            position: 0,
            size: 0,
            fs_id: 1, // DevFs
            inode: idx as u32,
            parent_cluster: 0,
        };

        if let Some(slot) = state.open_files.iter_mut().find(|e| e.is_none()) {
            *slot = Some(file);
        } else {
            state.open_files.push(Some(file));
        }
        return Ok(fd);
    }

    // --- Mount point path (e.g. /mnt/cdrom0/...) ---
    if let Some((_mount_path, relative_path)) = find_mnt_mount(path, &state.mount_points) {
        if let Some(ref iso) = state.iso9660_fs {
            let (inode, file_type, size) = iso.lookup(relative_path)?;
            let fd = state.next_fd;
            state.next_fd += 1;
            let file = OpenFile {
                fd,
                path: String::from(path),
                file_type,
                flags,
                position: 0,
                size,
                fs_id: 2, // ISO 9660
                inode,
                parent_cluster: 0,
            };
            if let Some(slot) = state.open_files.iter_mut().find(|e| e.is_none()) {
                *slot = Some(file);
            } else {
                state.open_files.push(Some(file));
            }
            return Ok(fd);
        }
        return Err(FsError::NotFound);
    }

    // --- FAT path ---
    if let Some(ref mut fat) = state.fat_fs {
        let lookup_result = fat.lookup(path);

        let (inode, file_type, size, parent_cluster) = match lookup_result {
            Ok((inode, file_type, size)) => {
                // File exists
                if flags.truncate && flags.write {
                    let (parent_path, filename) = split_parent_name(path)?;
                    let (parent_cluster, _, _) = fat.lookup(parent_path)?;
                    fat.truncate_file(parent_cluster, filename)?;
                    (0u32, file_type, 0u32, parent_cluster)
                } else {
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

        if let Some(slot) = state.open_files.iter_mut().find(|e| e.is_none()) {
            *slot = Some(file);
        } else {
            state.open_files.push(Some(file));
        }
        return Ok(fd);
    }

    // --- ISO 9660 root fallback (CD-ROM boot, no FAT16 disk) ---
    if let Some(ref iso) = state.iso9660_fs {
        if flags.write || flags.create || flags.truncate || flags.append {
            return Err(FsError::PermissionDenied);
        }
        let (inode, file_type, size) = iso.lookup(path)?;
        let fd = state.next_fd;
        state.next_fd += 1;
        let file = OpenFile {
            fd,
            path: String::from(path),
            file_type,
            flags,
            position: 0,
            size,
            fs_id: 2, // ISO 9660
            inode,
            parent_cluster: 0,
        };
        if let Some(slot) = state.open_files.iter_mut().find(|e| e.is_none()) {
            *slot = Some(file);
        } else {
            state.open_files.push(Some(file));
        }
        return Ok(fd);
    }

    Err(FsError::IoError)
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

    // --- DevFs file ---
    if file.fs_id == 1 {
        let name = dev_name(&file.path);
        let devfs = state.devfs.as_ref().ok_or(FsError::IoError)?;
        return devfs.read(name, buf).ok_or(FsError::IoError);
    }

    // --- ISO 9660 file ---
    if file.fs_id == 2 {
        if file.position >= file.size {
            return Ok(0);
        }
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let bytes_read = iso.read_file(file.inode, file.position, buf, file.size)?;
        file.position += bytes_read as u32;
        return Ok(bytes_read);
    }

    // --- FAT file ---
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

    // --- DevFs file ---
    if file.fs_id == 1 {
        let name = dev_name(&file.path);
        let devfs = state.devfs.as_ref().ok_or(FsError::IoError)?;
        return devfs.write(name, buf).ok_or(FsError::IoError);
    }

    // --- FAT file ---
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

    // --- /dev directory ---
    if path == "/dev" || path == "/dev/" {
        let devfs = state.devfs.as_ref().ok_or(FsError::NotFound)?;
        return Ok(devfs.list());
    }

    // --- /mnt listing ---
    if path == "/mnt" || path == "/mnt/" {
        let mut entries = Vec::new();
        for mp in &state.mount_points {
            if mp.path.starts_with("/mnt/") {
                let name = &mp.path[5..]; // strip "/mnt/"
                if !name.contains('/') && !name.is_empty() {
                    entries.push(DirEntry {
                        name: String::from(name),
                        file_type: FileType::Directory,
                        size: 0,
                    });
                }
            }
        }
        return Ok(entries);
    }

    // --- Mount point path (e.g. /mnt/cdrom0/...) ---
    if let Some((_mount_path, relative_path)) = find_mnt_mount(path, &state.mount_points) {
        if let Some(ref iso) = state.iso9660_fs {
            let (lba, file_type, size) = iso.lookup(relative_path)?;
            if file_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            return iso.read_dir(lba, size);
        }
        return Err(FsError::NotFound);
    }

    // --- FAT path ---
    if let Some(ref fat) = state.fat_fs {
        let (cluster, file_type, _size) = fat.lookup(path)?;
        if file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let mut entries = fat.read_dir(cluster)?;
        if path == "/" {
            add_virtual_root_entries(state, &mut entries);
        }
        return Ok(entries);
    }

    // --- ISO 9660 root fallback (CD-ROM boot, no FAT16 disk) ---
    if let Some(ref iso) = state.iso9660_fs {
        let (lba, file_type, size) = iso.lookup(path)?;
        if file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let mut entries = iso.read_dir(lba, size)?;
        if path == "/" {
            add_virtual_root_entries(state, &mut entries);
        }
        return Ok(entries);
    }

    Err(FsError::NotFound)
}

/// Add virtual directory entries (dev, mnt) to root directory listing.
fn add_virtual_root_entries(state: &VfsState, entries: &mut Vec<DirEntry>) {
    if state.devfs.is_some() {
        entries.push(DirEntry {
            name: String::from("dev"),
            file_type: FileType::Directory,
            size: 0,
        });
    }
    if state.mount_points.iter().any(|mp| mp.path.starts_with("/mnt/")) {
        entries.push(DirEntry {
            name: String::from("mnt"),
            file_type: FileType::Directory,
            size: 0,
        });
    }
}

/// Read an entire file into a Vec<u8>.
///
/// Phase 1 holds the VFS Mutex during lookup (directory traversal, may do
/// disk I/O) and cluster-chain plan building (in-memory FAT cache).
/// Phase 2 releases the lock and performs the actual data read.
///
/// Because the VFS uses a scheduler-integrated [`Mutex`] (not a spinlock),
/// interrupts remain enabled even during Phase 1 disk I/O.
pub fn read_file_to_vec(path: &str) -> Result<Vec<u8>, FsError> {
    // Device files are streaming — can't read to vec
    if is_dev_path(path) {
        return Err(FsError::PermissionDenied);
    }

    // Try mount point path (e.g. /mnt/cdrom0/...)
    {
        let vfs = VFS.lock();
        let state = vfs.as_ref().ok_or(FsError::IoError)?;
        if let Some((_mount_path, relative_path)) = find_mnt_mount(path, &state.mount_points) {
            if let Some(ref iso) = state.iso9660_fs {
                return iso.read_file_to_vec(relative_path);
            }
            return Err(FsError::NotFound);
        }
    }

    // Phase 1: Under VFS lock — lookup + build read plan (no disk I/O)
    let plan = {
        let vfs = VFS.lock();
        let state = vfs.as_ref().ok_or(FsError::IoError)?;
        if let Some(ref fat) = state.fat_fs {
            let (cluster, file_type, size) = fat.lookup(path)?;
            if file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            fat.get_file_read_plan(cluster, size)
        } else if let Some(ref iso) = state.iso9660_fs {
            // ISO 9660 root fallback (CD-ROM boot, no FAT16 disk)
            return iso.read_file_to_vec(path);
        } else {
            return Err(FsError::NotFound);
        }
    }; // VFS lock dropped — interrupts re-enabled

    // Phase 2: Without lock — perform disk I/O with interrupts enabled
    plan.execute()
}

/// Delete a file or empty directory at the given path.
pub fn delete(path: &str) -> Result<(), FsError> {
    if is_dev_path(path) { return Err(FsError::PermissionDenied); }
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;

    let (parent_path, filename) = split_parent_name(path)?;
    let (parent_cluster, _, _) = fat.lookup(parent_path)?;
    fat.delete_file(parent_cluster, filename)
}

/// Create a directory at the given path.
pub fn mkdir(path: &str) -> Result<(), FsError> {
    if is_dev_path(path) { return Err(FsError::PermissionDenied); }
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

    // Device files don't support seeking
    if file.fs_id == 1 {
        return Ok(0);
    }

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
    if is_dev_path(path) { return Err(FsError::PermissionDenied); }
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;

    let (parent_path, filename) = split_parent_name(path)?;
    let (parent_cluster, _, _) = fat.lookup(parent_path)?;
    fat.truncate_file(parent_cluster, filename)
}

/// Mount a filesystem at the given path from userspace (syscall handler).
///
/// `mount_path`: where to mount (e.g. "/mnt/cdrom0")
/// `device`: device path (e.g. "/dev/cdrom0") — currently only used for identification
/// `fs_type_id`: 0=FAT, 1=ISO9660
///
/// Returns Ok(()) on success.
pub fn mount_fs(mount_path: &str, _device: &str, fs_type_id: u32) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Check for duplicate mount point
    for mp in &state.mount_points {
        if mp.path == mount_path {
            return Err(FsError::AlreadyExists);
        }
    }

    match fs_type_id {
        0 => {
            // FAT mount — not supported as additional mount yet
            return Err(FsError::PermissionDenied);
        }
        1 => {
            // ISO 9660 (CD-ROM)
            if state.iso9660_fs.is_some() {
                // Already have an ISO fs instance — just add mount point
            } else {
                match Iso9660Fs::new() {
                    Ok(iso) => {
                        state.iso9660_fs = Some(iso);
                    }
                    Err(e) => return Err(e),
                }
            }
            state.mount_points.push(MountPoint {
                path: String::from(mount_path),
                fs_type: FsType::Iso9660,
                device_id: 0,
            });
            crate::serial_println!("  Mounted ISO 9660 at '{}'", mount_path);
            Ok(())
        }
        _ => Err(FsError::InvalidPath),
    }
}

/// Unmount a filesystem at the given path.
pub fn umount_fs(mount_path: &str) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Don't allow unmounting root or /dev
    if mount_path == "/" || mount_path == "/dev" {
        return Err(FsError::PermissionDenied);
    }

    // Find and remove the mount point
    let pos = state.mount_points.iter().position(|mp| mp.path == mount_path);
    if let Some(idx) = pos {
        let mp = state.mount_points.remove(idx);

        // If it was ISO 9660 and no other ISO mounts remain, drop the fs instance
        if mp.fs_type == FsType::Iso9660 {
            let has_other_iso = state.mount_points.iter().any(|m| m.fs_type == FsType::Iso9660);
            if !has_other_iso {
                state.iso9660_fs = None;
            }
        }

        crate::serial_println!("  Unmounted '{}'", mount_path);
        Ok(())
    } else {
        Err(FsError::NotFound)
    }
}

/// List all current mount points. Returns Vec of (mount_path, fs_type_name, device_id).
pub fn list_mounts() -> Vec<(String, &'static str, u32)> {
    let vfs = VFS.lock();
    if let Some(ref state) = *vfs {
        state.mount_points.iter().map(|mp| {
            let fs_name = match mp.fs_type {
                FsType::Fat => "fat16",
                FsType::Iso9660 => "iso9660",
                FsType::DevFs => "devfs",
            };
            (mp.path.clone(), fs_name, mp.device_id)
        }).collect()
    } else {
        Vec::new()
    }
}
