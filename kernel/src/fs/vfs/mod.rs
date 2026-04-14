//! Virtual File System (VFS) -- unified interface for file descriptors, open/read/write/close.
//! Delegates to the mounted filesystem (exFAT or FAT16) and manages the global open file table.

mod cache;
mod path;
mod types;

use crate::fs::devfs::DevFs;
use crate::fs::exfat::ExFatFs;
use crate::fs::fat::FatFs;
use crate::fs::iso9660::Iso9660Fs;
use crate::fs::ntfs::NtfsFs;
use crate::fs::overlayfs::OverlayFs;
use crate::fs::smbfs::SmbFs;
use crate::fs::file::{DirEntry, FileDescriptor, FileFlags, FileType, OpenFile};
use crate::sync::mutex::Mutex;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use self::path::{dev_name, find_mnt_mount, is_dev_path, resolve_exfat_path, split_parent_name};
pub use self::types::{Filesystem, FsError, FsType, StatFs, StatResult};

/// Maximum number of simultaneously open file descriptors (system-wide).
const MAX_OPEN_FILES: usize = 1024;

/// Counter for write operations — triggers periodic metadata flush.
/// After every FLUSH_INTERVAL writes, dirty metadata is flushed to disk.
/// This prevents data loss without slowing down every individual write.
static WRITE_COUNTER: AtomicU32 = AtomicU32::new(0);
const FLUSH_INTERVAL: u32 = 512;

/// Default partition start sector (used when no MBR/GPT partition table is found).
/// Must match mkimage.py --fs-start for backward compatibility.
const DEFAULT_PARTITION_LBA: u32 = 8192;

/// The actual partition LBA used for the root filesystem (set at boot from
/// partition table or fallback to DEFAULT_PARTITION_LBA).
static ROOT_PARTITION_LBA: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(8192);

fn queue_disk_flush(disks: &mut Vec<u8>, disk_id: u8) {
    if !disks.contains(&disk_id) {
        disks.push(disk_id);
    }
}

fn flush_blockcache_for_disks(disks: &[u8]) {
    for &disk_id in disks {
        crate::fs::blockcache::writeback_flush(disk_id);
    }
}

fn commit_open_exfat_entry(
    state: &mut VfsState,
    slot_id: usize,
    durable: bool,
) -> Result<Option<u8>, FsError> {
    let (fs_id, file_path, parent_cluster, inode, size, entry_dirty) = {
        let file = state.open_files.get(slot_id)
            .and_then(|e| e.as_ref())
            .ok_or(FsError::BadFd)?;
        (
            file.fs_id,
            file.path.clone(),
            file.parent_cluster,
            file.inode,
            file.size,
            file.entry_dirty,
        )
    };

    let is_exfat = fs_id == 3 || fs_id == 6;
    if !is_exfat {
        return Ok(None);
    }

    let filename = file_path.rsplit('/').next().unwrap_or("");
    if filename.is_empty() {
        return Ok(None);
    }

    let disk_id = if fs_id == 3 {
        let exfat = state.exfat_fs.as_mut().ok_or(FsError::IoError)?;
        if entry_dirty {
            exfat.update_entry(parent_cluster, filename, size, inode)?;
        }
        if durable && exfat.metadata_dirty {
            exfat.flush_metadata()?;
        }
        exfat.device_id as u8
    } else {
        let exfat = state.mounted_exfat.iter_mut()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, fs)| fs)
            .ok_or(FsError::IoError)?;
        if entry_dirty {
            exfat.update_entry(parent_cluster, filename, size, inode)?;
        }
        if durable && exfat.metadata_dirty {
            exfat.flush_metadata()?;
        }
        exfat.device_id as u8
    };

    if entry_dirty {
        if let Some(Some(file)) = state.open_files.get_mut(slot_id) {
            file.entry_dirty = false;
        }
    }

    Ok(Some(disk_id))
}

/// Set the root partition LBA (called from main.rs after partition scanning).
pub fn set_root_partition_lba(lba: u32) {
    ROOT_PARTITION_LBA.store(lba, core::sync::atomic::Ordering::Relaxed);
    crate::serial_verbose_println!("[VFS] root partition LBA set to {}", lba);
}

/// Get the current root partition LBA.
pub fn root_partition_lba() -> u32 {
    ROOT_PARTITION_LBA.load(core::sync::atomic::Ordering::Relaxed)
}

/// Invalidate all directory cache entries after path topology changes.
pub fn dir_cache_invalidate() {
    cache::dir_cache_invalidate();
}

static VFS: Mutex<Option<VfsState>> = Mutex::new(None);

struct VfsState {
    open_files: Vec<Option<OpenFile>>,
    mount_points: Vec<MountPoint>,
    exfat_fs: Option<ExFatFs>,
    fat_fs: Option<FatFs>,
    iso9660_fs: Option<Iso9660Fs>,
    ntfs_fs: Option<NtfsFs>,
    devfs: Option<DevFs>,
    /// SMB network filesystem instances (mount_path, instance).
    /// Vec because multiple different SMB shares can be mounted simultaneously.
    smbfs: Vec<(String, SmbFs)>,
    /// Mounted exFAT instances (mount_path, instance) for additional partitions.
    /// Separate from `exfat_fs` which is the root filesystem.
    mounted_exfat: Vec<(String, ExFatFs)>,
    /// OverlayFS: writable RAM layer over ISO 9660 (active when booting from CD).
    overlay_fs: Option<OverlayFs>,
    /// CoreFS-Treiber (read-only Mount via corefs-core). Optional, weil noch
    /// nicht alle VFS-Pfade das Dispatch auf diesen Treiber ausrollen — siehe
    /// `fs::corefs::CoreFsDriver`.
    corefs_driver: Option<crate::fs::corefs::CoreFsDriver>,
    /// Free slot stack for O(1) open file slot allocation.
    /// Contains indices of free (None) entries in open_files.
    free_slots: Vec<u32>,
}

impl VfsState {
    /// Allocate a free slot in the global open_files table. O(1) via free stack.
    /// Returns the slot index (global_id), or None if the table is full.
    fn alloc_slot(&mut self) -> Option<u32> {
        // Fast path: pop from free stack (O(1))
        if let Some(idx) = self.free_slots.pop() {
            return Some(idx);
        }
        // Slow path: extend table if below limit
        if self.open_files.len() < MAX_OPEN_FILES {
            let idx = self.open_files.len() as u32;
            self.open_files.push(None);
            return Some(idx);
        }
        None
    }

    /// Return a slot to the free stack when a file is closed.
    fn free_slot(&mut self, idx: u32) {
        let i = idx as usize;
        if i < self.open_files.len() {
            self.open_files[i] = None;
            self.free_slots.push(idx);
        }
    }
}

pub(crate) struct MountPoint {
    pub(crate) path: String,
    pub(crate) fs_type: FsType,
    pub(crate) device_id: u32,
}

/// Result of resolving an exFAT path with symlink handling.
pub(crate) struct ResolvedEntry {
    pub(crate) inode: u32,
    pub(crate) file_type: FileType,
    pub(crate) size: u32,
    pub(crate) is_symlink: bool,
    pub(crate) uid: u16,
    pub(crate) gid: u16,
    pub(crate) mode: u16,
    pub(crate) mtime: u32,
}


/// Initialize the VFS, reserving file descriptors 0-2 for stdin/stdout/stderr.
pub fn init() {
    let mut vfs = VFS.lock();
    *vfs = Some(VfsState {
        open_files: Vec::new(),
        mount_points: Vec::new(),
        exfat_fs: None,
        fat_fs: None,
        iso9660_fs: None,
        ntfs_fs: None,
        devfs: None,
        smbfs: Vec::new(),
        mounted_exfat: Vec::new(),
        overlay_fs: None,
        corefs_driver: None,
        free_slots: Vec::new(),
    });

    // Reserve fd 0, 1, 2
    let state = vfs.as_mut().expect("VFS must be initialized in init()");
    for _ in 0..3 {
        state.open_files.push(None);
    }

    crate::serial_verbose_println!("[OK] VFS initialized");
}

/// Check if a root disk filesystem is mounted.
pub fn has_root_fs() -> bool {
    let vfs = VFS.lock();
    if let Some(ref state) = *vfs {
        state.exfat_fs.is_some() || state.fat_fs.is_some() || state.ntfs_fs.is_some()
    } else {
        false
    }
}

/// Mount a filesystem at the given path.
/// For disk partitions, auto-detects exFAT vs FAT16 by reading the OEM name.
pub fn mount(path: &str, fs_type: FsType, device_id: u32) {
    crate::debug_println!("  [VFS] mount: path='{}' fs_type={:?} device_id={}", path, fs_type, device_id);
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().expect("VFS not initialized");

    let actual_type = if fs_type == FsType::Fat || fs_type == FsType::ExFat {
        // Auto-detect: read first sector to check OEM name
        crate::debug_println!("  [VFS] mount: reading VBR at LBA={}", root_partition_lba());
        #[allow(unused_mut)]
        let mut buf = [0u8; 512];
        #[cfg(target_arch = "x86_64")]
        let vbr_ok = crate::drivers::storage::read_sectors(root_partition_lba(), 1, &mut buf);
        #[cfg(target_arch = "aarch64")]
        let vbr_ok = crate::drivers::arm::storage::read_sectors(root_partition_lba(), 1, &mut buf);
        if vbr_ok {
            crate::serial_verbose_println!("  VFS auto-detect: OEM bytes = {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9], buf[10]);
            if &buf[3..11] == b"EXFAT   " {
                crate::debug_println!("  [VFS] mount: detected exFAT, calling ExFatFs::new()");
                match ExFatFs::new(device_id, root_partition_lba()) {
                    Ok(exfat) => {
                        crate::debug_println!("  [VFS] mount: ExFatFs::new() succeeded");
                        state.exfat_fs = Some(exfat);
                        crate::serial_verbose_println!("  Mounted exFAT at '{}'", path);
                    }
                    Err(_e) => {
                        crate::debug_println!("  [VFS] mount: ExFatFs::new() FAILED: {:?}", _e);
                        crate::serial_verbose_println!("  Failed to mount exFAT at '{}'", path);
                    }
                }
                FsType::ExFat
            } else if &buf[3..11] == b"NTFS    " {
                crate::debug_println!("  [VFS] mount: detected NTFS");
                match NtfsFs::new(device_id, root_partition_lba()) {
                    Ok(ntfs) => {
                        state.ntfs_fs = Some(ntfs);
                        crate::serial_verbose_println!("  Mounted NTFS (read-only) at '{}'", path);
                    }
                    Err(_e) => {
                        crate::serial_verbose_println!("  Failed to mount NTFS at '{}'", path);
                    }
                }
                FsType::Ntfs
            } else {
                match FatFs::new(device_id, root_partition_lba()) {
                    Ok(fat) => {
                        let type_name = match fat.fat_type {
                            crate::fs::fat::FatType::Fat12 => "FAT12",
                            crate::fs::fat::FatType::Fat16 => "FAT16",
                            crate::fs::fat::FatType::Fat32 => "FAT32",
                        };
                        crate::serial_verbose_println!("  Mounted {} at '{}'", type_name, path);
                        state.fat_fs = Some(fat);
                    }
                    Err(_) => {
                        crate::serial_verbose_println!("  Failed to mount FAT at '{}'", path);
                    }
                }
                FsType::Fat
            }
        } else {
            crate::serial_verbose_println!("  Failed to read partition at LBA {}", root_partition_lba());
            FsType::Fat
        }
    } else if fs_type == FsType::Iso9660 {
        match Iso9660Fs::new() {
            Ok(iso) => {
                state.iso9660_fs = Some(iso);
                crate::serial_verbose_println!("  Mounted ISO 9660 at '{}'", path);
            }
            Err(_) => {
                crate::serial_verbose_println!("  Failed to mount ISO 9660 at '{}'", path);
            }
        }
        FsType::Iso9660
    } else {
        fs_type
    };

    state.mount_points.push(MountPoint {
        path: String::from(path),
        fs_type: actual_type,
        device_id,
    });
}

/// Remove a mount point entry by path (used to clean up failed mounts).
pub fn remove_mount(path: &str) {
    let mut vfs = VFS.lock();
    if let Some(ref mut state) = *vfs {
        state.mount_points.retain(|mp| mp.path != path);
    }
}

/// Mount a CoreFS volume read-only.
///
/// Boot-Code ruft diese Funktion auf, nachdem
/// [`crate::fs::corefs::probe::detect`] für die Partition `true` geliefert
/// hat. `partition_sectors` beschreibt die Länge der Partition in 512-Byte-
/// Sektoren (aus MBR/GPT). Bei Erfolg ist der Treiber unter `path` im VFS
/// registriert; read/write-Dispatch durch die klassischen VFS-Pfade folgt
/// in einem separaten Schritt (aktuell exponiert der Treiber seinen Read-
/// Pfad über [`crate::fs::corefs::CoreFsDriver`] direkt).
pub fn mount_corefs(
    path: &str,
    disk_id: u8,
    partition_lba: u32,
    partition_sectors: u64,
    device_id: u32,
) -> Result<(), FsError> {
    let adapter = crate::fs::corefs::BlockDeviceAdapter::new(
        disk_id,
        partition_lba,
        partition_sectors,
        /* read_only = */ true,
    )
    .map_err(|e| crate::fs::corefs::corefs_to_fs_error(&e))?;
    let driver = crate::fs::corefs::CoreFsDriver::mount_read_only(adapter)?;
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    state.corefs_driver = Some(driver);
    state.mount_points.push(MountPoint {
        path: String::from(path),
        fs_type: FsType::CoreFs,
        device_id,
    });
    crate::serial_verbose_println!(
        "  Mounted CoreFS (read-only) at '{}' (disk={}, lba={}, sectors={})",
        path,
        disk_id,
        partition_lba,
        partition_sectors
    );
    Ok(())
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
    crate::serial_verbose_println!("  Mounted DevFs at '/dev'");
}

/// Enable the OverlayFS: writable RAM layer over the ISO 9660 root filesystem.
/// Call this after mounting ISO 9660 as root (CD-ROM boot without a disk filesystem).
pub fn enable_overlay() {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().expect("VFS not initialized");
    if state.iso9660_fs.is_some() && state.overlay_fs.is_none() {
        state.overlay_fs = Some(OverlayFs::new());
        crate::serial_println!("[VFS] OverlayFS enabled (RAM overlay over ISO 9660)");
    }
}

/// Check if the overlay filesystem is active.
pub fn has_overlay() -> bool {
    let vfs = VFS.lock();
    if let Some(ref state) = *vfs {
        state.overlay_fs.is_some()
    } else {
        false
    }
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

        let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;

        let file = OpenFile {
            fd: slot_id,
            path: String::from(path),
            file_type: FileType::Device,
            flags,
            position: 0,
            size: 0,
            fs_id: 1, // DevFs
            inode: idx as u32,
            parent_cluster: 0,
            refcount: 1,
            seek_cache_offset: 0,
            seek_cache_cluster: 0,
            entry_dirty: false,
        };

        state.open_files[slot_id as usize] = Some(file);
        return Ok(slot_id);
    }

    // --- Mount point path (e.g. /mnt/cdrom0/..., /mnt/share/...) ---
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_mnt_mount(path, &state.mount_points) {
        match mnt_fs_type {
            FsType::Iso9660 => {
                if let Some(ref iso) = state.iso9660_fs {
                    let (inode, file_type, size) = iso.lookup(relative_path)?;
                    let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
                    let file = OpenFile {
                        fd: slot_id,
                        path: String::from(path),
                        file_type,
                        flags,
                        position: 0,
                        size,
                        fs_id: 2, // ISO 9660
                        inode,
                        parent_cluster: 0,
                        refcount: 1,
                        seek_cache_offset: 0,
                        seek_cache_cluster: 0,
                        entry_dirty: false,
                    };
                    state.open_files[slot_id as usize] = Some(file);
                    return Ok(slot_id);
                }
                return Err(FsError::NotFound);
            }
            FsType::Ntfs => {
                if let Some(ref ntfs) = state.ntfs_fs {
                    if flags.write || flags.create || flags.truncate || flags.append {
                        return Err(FsError::PermissionDenied);
                    }
                    let (inode, file_type, size) = ntfs.lookup(relative_path)?;
                    let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
                    let file = OpenFile {
                        fd: slot_id,
                        path: String::from(path),
                        file_type,
                        flags,
                        position: 0,
                        size,
                        fs_id: 4, // NTFS
                        inode,
                        parent_cluster: 0,
                        refcount: 1,
                        seek_cache_offset: 0,
                        seek_cache_cluster: 0,
                        entry_dirty: false,
                    };
                    state.open_files[slot_id as usize] = Some(file);
                    return Ok(slot_id);
                }
                return Err(FsError::NotFound);
            }
            FsType::ExFat => {
                let mount_path_owned = String::from(mount_path);
                let exfat = state.mounted_exfat.iter_mut()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, fs)| fs)
                    .ok_or(FsError::IoError)?;
                let lookup_result = exfat.lookup(relative_path);
                let (inode, file_type, size, parent_cluster) = match lookup_result {
                    Ok((inode, file_type, size)) => {
                        if flags.truncate && flags.write {
                            let (parent_path, filename) = split_parent_name(relative_path)?;
                            let (pr_inode, _, _) = exfat.lookup(parent_path)?;
                            let (pc, _) = crate::fs::exfat::decode_inode(pr_inode);
                            exfat.truncate_file(pc, filename)?;
                            (0u32, file_type, 0u32, pc)
                        } else {
                            let pc = if flags.write {
                                let (parent_path, _) = split_parent_name(relative_path)?;
                                exfat.lookup(parent_path)
                                    .map(|(i, _, _)| crate::fs::exfat::decode_inode(i).0)
                                    .unwrap_or(0)
                            } else { 0 };
                            (inode, file_type, size, pc)
                        }
                    }
                    Err(FsError::NotFound) if flags.create => {
                        let (parent_path, filename) = split_parent_name(relative_path)?;
                        let (pr_inode, pr_type, _) = exfat.lookup(parent_path)?;
                        if pr_type != FileType::Directory {
                            return Err(FsError::NotADirectory);
                        }
                        let pc = crate::fs::exfat::decode_inode(pr_inode).0;
                        exfat.create_file(pc, filename)?;
                        (0u32, FileType::Regular, 0u32, pc)
                    }
                    Err(e) => return Err(e),
                };
                let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
                let position = if flags.append { size } else { 0 };
                let file = OpenFile {
                    fd: slot_id,
                    path: String::from(path),
                    file_type,
                    flags,
                    position,
                    size,
                    fs_id: 6, // Mounted exFAT
                    inode,
                    parent_cluster,
                    refcount: 1,
                    seek_cache_offset: 0,
                    seek_cache_cluster: 0,
                    entry_dirty: false,
                };
                state.open_files[slot_id as usize] = Some(file);
                return Ok(slot_id);
            }
            FsType::Smb => {
                let mount_path_owned = String::from(mount_path);
                let relative_path_owned = String::from(relative_path);
                let smb = state.smbfs.iter_mut()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, s)| s)
                    .ok_or(FsError::IoError)?;
                let lookup_result = smb.lookup(&relative_path_owned);
                let (inode, file_type, size) = match lookup_result {
                    Ok(r) => r,
                    Err(FsError::NotFound) if flags.create => {
                        // Create file on the SMB share
                        let rel = relative_path_owned.trim_end_matches('/');
                        let (parent, name) = match rel.rfind('/') {
                            Some(0) => ("/", &rel[1..]),
                            Some(pos) => (&rel[..pos], &rel[pos + 1..]),
                            None => ("/", rel),
                        };
                        let (parent_inode, _, _) = smb.lookup(parent)?;
                        let new_inode = smb.create_entry(parent_inode, name, FileType::Regular)?;
                        (new_inode, FileType::Regular, 0)
                    }
                    Err(e) => return Err(e),
                };
                let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
                let file = OpenFile {
                    fd: slot_id,
                    path: String::from(path),
                    file_type,
                    flags,
                    position: 0,
                    size,
                    fs_id: 5, // SMB
                    inode,
                    parent_cluster: 0,
                    refcount: 1,
                    seek_cache_offset: 0,
                    seek_cache_cluster: 0,
                    entry_dirty: false,
                };
                state.open_files[slot_id as usize] = Some(file);
                return Ok(slot_id);
            }
            _ => {
                return Err(FsError::NotFound);
            }
        }
    }

    // --- exFAT path (primary OS filesystem, with symlink resolution) ---
    if let Some(ref mut exfat) = state.exfat_fs {
        // Resolve symlinks in the path before opening
        let lookup_result = resolve_exfat_path(exfat, path, true);

        let (inode, file_type, size, parent_cluster) = match lookup_result {
            Ok(r) => {
                if flags.truncate && flags.write {
                    let (parent_path, filename) = split_parent_name(path)?;
                    let pr = resolve_exfat_path(exfat, parent_path, true)?;
                    let (pc, _) = crate::fs::exfat::decode_inode(pr.inode);
                    exfat.truncate_file(pc, filename)?;
                    (0u32, r.file_type, 0u32, pc)
                } else {
                    let parent_cluster = if flags.write {
                        let (parent_path, _) = split_parent_name(path)?;
                        resolve_exfat_path(exfat, parent_path, true)
                            .map(|pr| crate::fs::exfat::decode_inode(pr.inode).0)
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    (r.inode, r.file_type, r.size, parent_cluster)
                }
            }
            Err(FsError::NotFound) if flags.create => {
                let (parent_path, filename) = split_parent_name(path)?;
                let pr = resolve_exfat_path(exfat, parent_path, true)?;
                if pr.file_type != FileType::Directory {
                    return Err(FsError::NotADirectory);
                }
                let pc = crate::fs::exfat::decode_inode(pr.inode).0;
                exfat.create_file(pc, filename)?;
                (0u32, FileType::Regular, 0u32, pc)
            }
            Err(e) => return Err(e),
        };

        let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
        let position = if flags.append { size } else { 0 };
        let file = OpenFile {
            fd: slot_id,
            path: String::from(path),
            file_type,
            flags,
            position,
            size,
            fs_id: 3, // exFAT
            inode,
            parent_cluster,
            refcount: 1,
            seek_cache_offset: 0,
            seek_cache_cluster: 0,
            entry_dirty: false,
        };
        state.open_files[slot_id as usize] = Some(file);
        return Ok(slot_id);
    }

    // --- FAT16 path (fallback / secondary mounts) ---
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

        let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;

        let position = if flags.append { size } else { 0 };

        let file = OpenFile {
            fd: slot_id,
            path: String::from(path),
            file_type,
            flags,
            position,
            size,
            fs_id: 0,
            inode,
            parent_cluster,
            refcount: 1,
            seek_cache_offset: 0,
            seek_cache_cluster: 0,
            entry_dirty: false,
        };

        state.open_files[slot_id as usize] = Some(file);
        return Ok(slot_id);
    }

    // --- NTFS path (read-only) ---
    if let Some(ref ntfs) = state.ntfs_fs {
        if flags.write || flags.create || flags.truncate || flags.append {
            return Err(FsError::PermissionDenied);
        }
        let (inode, file_type, size) = ntfs.lookup(path)?;
        let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
        let file = OpenFile {
            fd: slot_id,
            path: String::from(path),
            file_type,
            flags,
            position: 0,
            size,
            fs_id: 4, // NTFS
            inode,
            parent_cluster: 0,
            refcount: 1,
            seek_cache_offset: 0,
            seek_cache_cluster: 0,
            entry_dirty: false,
        };
        state.open_files[slot_id as usize] = Some(file);
        return Ok(slot_id);
    }

    // --- OverlayFS root (CD-ROM boot with writable RAM overlay) ---
    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_mut().ok_or(FsError::IoError)?;

        let lookup_result = overlay.lookup(iso, path);
        let (inode, file_type, size) = match lookup_result {
            Ok(r) => {
                if flags.truncate && flags.write {
                    overlay.truncate(iso, path)?;
                    let (i, ft, _) = overlay.lookup(iso, path)?;
                    (i, ft, 0u32)
                } else {
                    r
                }
            }
            Err(FsError::NotFound) if flags.create => {
                let new_inode = overlay.create_file(path)?;
                (new_inode, FileType::Regular, 0u32)
            }
            Err(e) => return Err(e),
        };

        let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
        let position = if flags.append { size } else { 0 };
        let file = OpenFile {
            fd: slot_id,
            path: String::from(path),
            file_type,
            flags,
            position,
            size,
            fs_id: 7, // OverlayFS
            inode,
            parent_cluster: 0,
            refcount: 1,
            seek_cache_offset: 0,
            seek_cache_cluster: 0,
            entry_dirty: false,
        };
        state.open_files[slot_id as usize] = Some(file);
        return Ok(slot_id);
    }

    // --- ISO 9660 root fallback (CD-ROM boot without overlay, read-only) ---
    if let Some(ref iso) = state.iso9660_fs {
        if flags.write || flags.create || flags.truncate || flags.append {
            return Err(FsError::PermissionDenied);
        }
        let (inode, file_type, size) = iso.lookup(path)?;
        let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
        let file = OpenFile {
            fd: slot_id,
            path: String::from(path),
            file_type,
            flags,
            position: 0,
            size,
            fs_id: 2, // ISO 9660
            inode,
            parent_cluster: 0,
            refcount: 1,
            seek_cache_offset: 0,
            seek_cache_cluster: 0,
            entry_dirty: false,
        };
        state.open_files[slot_id as usize] = Some(file);
        return Ok(slot_id);
    }

    Err(FsError::IoError)
}

/// Close a global open file slot (by slot_id). Decrements refcount, frees if 0.
pub fn close(slot_id: FileDescriptor) -> Result<(), FsError> {
    let mut do_writeback = false;
    let mut disks_to_flush: Vec<u8> = Vec::new();
    {
        let mut vfs = VFS.lock();
        let state = vfs.as_mut().ok_or(FsError::IoError)?;

        let (refcount, fs_id, was_writable) = {
            let file = state.open_files.get(slot_id as usize)
                .and_then(|e| e.as_ref())
                .ok_or(FsError::BadFd)?;
            (file.refcount, file.fs_id, file.flags.write)
        };

        if refcount > 1 {
            let file = state.open_files.get_mut(slot_id as usize)
                .and_then(|e| e.as_mut())
                .ok_or(FsError::BadFd)?;
            file.refcount -= 1;
        } else {
            if was_writable && (fs_id == 3 || fs_id == 6) {
                if let Some(disk_id) = commit_open_exfat_entry(state, slot_id as usize, true)? {
                    queue_disk_flush(&mut disks_to_flush, disk_id);
                }
                do_writeback = true;
            }
            state.free_slot(slot_id);
        }
    } // VFS lock released

    // Flush write-back cache outside VFS lock (may block on disk I/O)
    if do_writeback {
        flush_blockcache_for_disks(&disks_to_flush);
    }
    Ok(())
}

/// Increment the reference count on a global open file slot (for fork/dup).
pub fn incref(slot_id: u32) {
    let mut vfs = VFS.lock();
    if let Some(state) = vfs.as_mut() {
        if let Some(Some(file)) = state.open_files.get_mut(slot_id as usize) {
            file.refcount += 1;
        }
    }
}

/// Decrement the reference count on a global open file slot (for close/exit).
/// Frees the slot if refcount drops to 0. On last close of a writable exFAT
/// file, flushes deferred metadata to disk.
pub fn decref(slot_id: u32) {
    let mut do_writeback = false;
    let mut disks_to_flush: Vec<u8> = Vec::new();
    let mut vfs = VFS.lock();
    if let Some(state) = vfs.as_mut() {
        let snapshot = state.open_files.get(slot_id as usize)
            .and_then(|e| e.as_ref())
            .map(|file| (file.refcount, file.fs_id, file.flags.write));
        if let Some((refcount, fs_id, was_writable)) = snapshot {
            if refcount > 1 {
                if let Some(Some(file)) = state.open_files.get_mut(slot_id as usize) {
                    file.refcount -= 1;
                }
            } else {
                if was_writable && (fs_id == 3 || fs_id == 6) {
                    if let Ok(Some(disk_id)) = commit_open_exfat_entry(state, slot_id as usize, true) {
                        queue_disk_flush(&mut disks_to_flush, disk_id);
                    }
                    do_writeback = true;
                }
                state.free_slot(slot_id);
            }
        }
    }
    drop(vfs);
    if do_writeback {
        flush_blockcache_for_disks(&disks_to_flush);
    }
}

/// Read bytes from an open file into `buf`. `slot_id` is the global open_files index.
/// Returns the number of bytes read (0 at EOF).
pub fn read(slot_id: FileDescriptor, buf: &mut [u8]) -> Result<usize, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Direct index lookup
    let file = state.open_files.get_mut(slot_id as usize)
        .and_then(|e| e.as_mut())
        .ok_or(FsError::BadFd)?;

    // --- DevFs file ---
    if file.fs_id == 1 {
        let name = dev_name(&file.path);
        let devfs = state.devfs.as_ref().ok_or(FsError::IoError)?;
        return devfs.read(name, buf).ok_or(FsError::IoError);
    }

    // --- Mounted exFAT file ---
    if file.fs_id == 6 {
        if file.position >= file.size {
            return Ok(0);
        }
        let remaining = (file.size - file.position) as usize;
        let to_read = buf.len().min(remaining);
        let file_path = file.path.clone();
        let file_inode = file.inode;
        let file_position = file.position;
        let exfat = state.mounted_exfat.iter()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, fs)| fs)
            .ok_or(FsError::IoError)?;
        let bytes_read = exfat.read_file(file_inode, file_position, &mut buf[..to_read])?;
        let file = state.open_files.get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.position += bytes_read as u32;
        return Ok(bytes_read);
    }

    // --- OverlayFS file (RAM + ISO 9660) ---
    if file.fs_id == 7 {
        if file.position >= file.size {
            return Ok(0);
        }
        let remaining = (file.size - file.position) as usize;
        let to_read = buf.len().min(remaining);
        let file_inode = file.inode;
        let file_position = file.position;
        let file_size = file.size;
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_ref().ok_or(FsError::IoError)?;
        let bytes_read = overlay.read_file(iso, file_inode, file_position, &mut buf[..to_read], file_size)?;
        let file = state.open_files.get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.position += bytes_read as u32;
        return Ok(bytes_read);
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

    // --- NTFS file (read-only) ---
    if file.fs_id == 4 {
        if file.position >= file.size {
            return Ok(0);
        }
        let remaining = (file.size - file.position) as usize;
        let to_read = buf.len().min(remaining);
        let ntfs = state.ntfs_fs.as_ref().ok_or(FsError::IoError)?;
        let bytes_read = ntfs.read_file(file.inode, file.position, &mut buf[..to_read])?;
        file.position += bytes_read as u32;
        return Ok(bytes_read);
    }

    // --- SMB file (network) ---
    if file.fs_id == 5 {
        if file.position >= file.size {
            return Ok(0);
        }
        let remaining = (file.size - file.position) as usize;
        let to_read = buf.len().min(remaining);
        let file_inode = file.inode;
        let file_position = file.position;
        let file_path = file.path.clone();
        let smb = state.smbfs.iter_mut()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, s)| s)
            .ok_or(FsError::IoError)?;
        let bytes_read = smb.read_file(file_inode, file_position, &mut buf[..to_read])?;
        // Re-borrow file after mutable smb use
        let file = state.open_files.get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.position += bytes_read as u32;
        return Ok(bytes_read);
    }

    // --- exFAT / FAT file ---
    if file.position >= file.size {
        return Ok(0); // EOF
    }

    let remaining = (file.size - file.position) as usize;
    let to_read = buf.len().min(remaining);

    let bytes_read = if file.fs_id == 3 {
        let exfat = state.exfat_fs.as_ref().ok_or(FsError::IoError)?;
        exfat.read_file(file.inode, file.position, &mut buf[..to_read])?
    } else if let Some(ref fat) = state.fat_fs {
        fat.read_file(file.inode, file.position, &mut buf[..to_read])?
    } else {
        return Err(FsError::IoError);
    };

    file.position += bytes_read as u32;
    Ok(bytes_read)
}

/// Write bytes from `buf` to an open file. `slot_id` is the global open_files index.
/// Returns the number of bytes written.
pub fn write(slot_id: FileDescriptor, buf: &[u8]) -> Result<usize, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Direct index lookup
    let file = state.open_files.get_mut(slot_id as usize)
        .and_then(|e| e.as_mut())
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

    // --- NTFS is read-only ---
    if file.fs_id == 4 {
        return Err(FsError::PermissionDenied);
    }

    // --- OverlayFS file (copy-on-write to RAM) ---
    if file.fs_id == 7 {
        let old_inode = file.inode;
        let old_size = file.size;
        let position = file.position;
        let file_path = file.path.clone();
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_mut().ok_or(FsError::IoError)?;
        let (new_inode, new_size) = overlay.write_file(iso, old_inode, position, buf, old_size, &file_path)?;
        let file = state.open_files.get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.inode = new_inode;
        file.size = new_size;
        file.position = position + buf.len() as u32;
        return Ok(buf.len());
    }

    // --- Mounted exFAT file ---
    if file.fs_id == 6 {
        let old_inode = file.inode;
        let old_size = file.size;
        let position = file.position;
        let file_path = file.path.clone();
        let hint = if file.seek_cache_cluster >= 2 && file.seek_cache_offset <= position {
            Some((file.seek_cache_offset, file.seek_cache_cluster))
        } else {
            None
        };
        let sync_write = file.flags.sync;
        let exfat = state.mounted_exfat.iter_mut()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, fs)| fs)
            .ok_or(FsError::IoError)?;
        let (new_cluster, new_size, hint_offset, hint_cluster) =
            exfat.write_file_with_hint(old_inode, position, buf, old_size, hint)?;
        let file = state.open_files.get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.inode = new_cluster;
        file.size = new_size;
        file.position = position + buf.len() as u32;
        file.seek_cache_offset = hint_offset;
        file.seek_cache_cluster = hint_cluster;
        if new_cluster != old_inode || new_size != old_size {
            file.entry_dirty = true;
        }
        if sync_write {
            let mut disks_to_flush = Vec::new();
            if let Some(disk_id) = commit_open_exfat_entry(state, slot_id as usize, true)? {
                queue_disk_flush(&mut disks_to_flush, disk_id);
            }
            drop(vfs);
            flush_blockcache_for_disks(&disks_to_flush);
            crate::drivers::storage::flush();
            return Ok(buf.len());
        }
        return Ok(buf.len());
    }

    // --- SMB file (network) ---
    if file.fs_id == 5 {
        let file_inode = file.inode;
        let file_position = file.position;
        let file_path = file.path.clone();
        let smb = state.smbfs.iter_mut()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, s)| s)
            .ok_or(FsError::IoError)?;
        let bytes_written = smb.write_file(file_inode, file_position, buf)?;
        // Re-borrow file after mutable smb use
        let file = state.open_files.get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.position += bytes_written as u32;
        if file.position > file.size {
            file.size = file.position;
        }
        return Ok(bytes_written);
    }

    // --- exFAT / FAT file ---
    let old_inode = file.inode;
    let old_size = file.size;
    let position = file.position;
    let parent_cluster = file.parent_cluster;
    let fs_id = file.fs_id;

    // Extract filename from path for directory entry update
    let path_clone = file.path.clone();
    let filename = path_clone.rsplit('/').next().unwrap_or("");

    let hint = if file.seek_cache_cluster >= 2 && file.seek_cache_offset <= position {
            Some((file.seek_cache_offset, file.seek_cache_cluster))
        } else {
            None
        };
    let sync_write = file.flags.sync;

    if fs_id == 3 {
        let exfat = state.exfat_fs.as_mut().ok_or(FsError::IoError)?;
        let (new_cluster, new_size, hint_offset, hint_cluster) =
            exfat.write_file_with_hint(old_inode, position, buf, old_size, hint)?;
        let file = state.open_files.get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.inode = new_cluster;
        file.size = new_size;
        file.position = position + buf.len() as u32;
        file.seek_cache_offset = hint_offset;
        file.seek_cache_cluster = hint_cluster;
        if new_cluster != old_inode || new_size != old_size {
            file.entry_dirty = true;
        }
        if sync_write {
            let mut disks_to_flush = Vec::new();
            if let Some(disk_id) = commit_open_exfat_entry(state, slot_id as usize, true)? {
                queue_disk_flush(&mut disks_to_flush, disk_id);
            }
            drop(vfs);
            flush_blockcache_for_disks(&disks_to_flush);
            crate::drivers::storage::flush();
            return Ok(buf.len());
        }
    } else {
        let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;
        let (new_cluster, new_size) = fat.write_file(old_inode, position, buf, old_size)?;
        if new_cluster != old_inode || new_size != old_size {
            fat.update_entry(parent_cluster, filename, new_size, new_cluster)?;
        }
        let file = state.open_files.get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.inode = new_cluster;
        file.size = new_size;
        file.position = position + buf.len() as u32;
    }

    // Periodic writeback: every FLUSH_INTERVAL writes, flush metadata to FAT/bitmap.
    // The actual block cache writeback happens separately (not under VFS lock).
    let wc = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    if wc % FLUSH_INTERVAL == FLUSH_INTERVAL - 1 {
        if let Some(ref mut exfat) = state.exfat_fs {
            if exfat.metadata_dirty {
                let _ = exfat.flush_metadata();
            }
        }
        for (_path, exfat) in &mut state.mounted_exfat {
            if exfat.metadata_dirty {
                let _ = exfat.flush_metadata();
            }
        }
    }
    Ok(buf.len())
}

/// Read directory entries at a given path.
pub fn read_dir(path: &str) -> Result<Vec<DirEntry>, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

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
                        is_symlink: false,
                        uid: 0, gid: 0, mode: 0xFFF,
                    });
                }
            }
        }
        return Ok(entries);
    }

    // --- Mount point path (e.g. /mnt/cdrom0/..., /mnt/share/...) ---
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_mnt_mount(path, &state.mount_points) {
        match mnt_fs_type {
            FsType::Iso9660 => {
                if let Some(ref iso) = state.iso9660_fs {
                    let (lba, file_type, size) = iso.lookup(relative_path)?;
                    if file_type != FileType::Directory {
                        return Err(FsError::NotADirectory);
                    }
                    return iso.read_dir(lba, size);
                }
                return Err(FsError::NotFound);
            }
            FsType::Ntfs => {
                if let Some(ref ntfs) = state.ntfs_fs {
                    let (mft_rec, file_type, _size) = ntfs.lookup(relative_path)?;
                    if file_type != FileType::Directory {
                        return Err(FsError::NotADirectory);
                    }
                    return ntfs.read_dir(mft_rec as u64);
                }
                return Err(FsError::NotFound);
            }
            FsType::ExFat => {
                let mount_path_owned = String::from(mount_path);
                let exfat = state.mounted_exfat.iter()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, fs)| fs)
                    .ok_or(FsError::IoError)?;
                let (inode, file_type, _size) = exfat.lookup(relative_path)?;
                if file_type != FileType::Directory {
                    return Err(FsError::NotADirectory);
                }
                let (cluster, _) = crate::fs::exfat::decode_inode(inode);
                return exfat.read_dir(cluster);
            }
            FsType::Smb => {
                let mount_path_owned = String::from(mount_path);
                let smb = state.smbfs.iter_mut()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, s)| s)
                    .ok_or(FsError::IoError)?;
                let (inode, file_type, _size) = smb.lookup(relative_path)?;
                if file_type != FileType::Directory {
                    return Err(FsError::NotADirectory);
                }
                return smb.read_dir(inode);
            }
            _ => {
                return Err(FsError::NotFound);
            }
        }
    }

    // --- exFAT path (primary, with symlink resolution) ---
    if let Some(ref exfat) = state.exfat_fs {
        let r = resolve_exfat_path(exfat, path, true)?;
        if r.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let (cluster, _contiguous) = crate::fs::exfat::decode_inode(r.inode);
        let mut entries = exfat.read_dir(cluster)?;

        if path == "/" {
            add_virtual_root_entries(state, &mut entries);
        }
        return Ok(entries);
    }

    // --- NTFS path (read-only) ---
    if let Some(ref ntfs) = state.ntfs_fs {
        let (mft_rec, file_type, _size) = ntfs.lookup(path)?;
        if file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let mut entries = ntfs.read_dir(mft_rec as u64)?;
        if path == "/" {
            add_virtual_root_entries(state, &mut entries);
        }
        return Ok(entries);
    }

    // --- FAT16 path (fallback) ---
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

    // --- OverlayFS root (CD-ROM boot with writable RAM overlay) ---
    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_ref().ok_or(FsError::IoError)?;
        let mut entries = overlay.read_dir(iso, path)?;
        if path == "/" {
            add_virtual_root_entries(state, &mut entries);
        }
        return Ok(entries);
    }

    // --- ISO 9660 root fallback (CD-ROM boot without overlay, read-only) ---
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
            is_symlink: false,
            uid: 0, gid: 0, mode: 0xFFF,
        });
    }
    if state.mount_points.iter().any(|mp| mp.path.starts_with("/mnt/")) {
        entries.push(DirEntry {
            name: String::from("mnt"),
            file_type: FileType::Directory,
            size: 0,
            is_symlink: false,
            uid: 0, gid: 0, mode: 0xFFF,
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
    use crate::fs::exfat::ExFatReadPlan;
    use crate::fs::fat::FileReadPlan;
    use crate::fs::ntfs::NtfsReadPlan;
   
    enum ReadPlan {
        Fat(FileReadPlan),
        ExFat(ExFatReadPlan),
        Ntfs(NtfsReadPlan),
    }

    // Device files are streaming — can't read to vec
    if is_dev_path(path) {
        return Err(FsError::PermissionDenied);
    }

    // Try mount point path (e.g. /mnt/cdrom0/..., /mnt/share/...)
    {
        let mut vfs = VFS.lock();
        let state = vfs.as_mut().ok_or(FsError::IoError)?;
        if let Some((mount_path, relative_path, mnt_fs_type)) = find_mnt_mount(path, &state.mount_points) {
            match mnt_fs_type {
                FsType::Iso9660 => {
                    if let Some(ref iso) = state.iso9660_fs {
                        return iso.read_file_to_vec(relative_path);
                    }
                    return Err(FsError::NotFound);
                }
                FsType::ExFat => {
                    let mount_path_owned = String::from(mount_path);
                    let exfat = state.mounted_exfat.iter()
                        .find(|(p, _)| *p == mount_path_owned)
                        .map(|(_, fs)| fs)
                        .ok_or(FsError::IoError)?;
                    let (inode, file_type, size) = exfat.lookup(relative_path)?;
                    if file_type == FileType::Directory {
                        return Err(FsError::IsADirectory);
                    }
                    let mut buf = alloc::vec![0u8; size as usize];
                    let n = exfat.read_file(inode, 0, &mut buf)?;
                    buf.truncate(n);
                    return Ok(buf);
                }
                FsType::Smb => {
                    let mount_path_owned = String::from(mount_path);
                    let smb = state.smbfs.iter_mut()
                        .find(|(p, _)| *p == mount_path_owned)
                        .map(|(_, s)| s)
                        .ok_or(FsError::IoError)?;
                    let (inode, file_type, size) = smb.lookup(relative_path)?;
                    if file_type == FileType::Directory {
                        return Err(FsError::IsADirectory);
                    }
                    let mut buf = alloc::vec![0u8; size as usize];
                    let n = smb.read_file(inode, 0, &mut buf)?;
                    buf.truncate(n);
                    return Ok(buf);
                }
                _ => return Err(FsError::NotFound),
            }
        }
    }

    // Phase 1: Under VFS lock — lookup + build read plan (no disk I/O)
    let plan = {
        let vfs = VFS.lock();
        let state = vfs.as_ref().ok_or(FsError::IoError)?;
        if let Some(ref exfat) = state.exfat_fs {
            let r = resolve_exfat_path(exfat, path, true)?;
            if r.file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            ReadPlan::ExFat(exfat.get_file_read_plan(r.inode, r.size))
        } else if let Some(ref ntfs) = state.ntfs_fs {
            let (mft_rec, file_type, size) = ntfs.lookup(path)?;
            if file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            ReadPlan::Ntfs(ntfs.get_file_read_plan(mft_rec, size))
        } else if let Some(ref fat) = state.fat_fs {
            let (cluster, file_type, size) = fat.lookup(path)?;
            if file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            ReadPlan::Fat(fat.get_file_read_plan(cluster, size))
        } else if let Some(ref iso) = state.iso9660_fs {
            return iso.read_file_to_vec(path);
        } else {
            return Err(FsError::NotFound);
        }
    }; // VFS lock dropped — interrupts re-enabled

    // Phase 2: Without lock — perform disk I/O with interrupts enabled
    let result = match plan {
        ReadPlan::Fat(p) => p.execute(),
        ReadPlan::ExFat(p) => p.execute(),
        ReadPlan::Ntfs(p) => p.execute(),
    };
    result
}

/// Delete a file, directory, or symlink at the given path.
/// Symlinks are deleted without following (only the link is removed).
pub fn delete(path: &str) -> Result<(), FsError> {
    if is_dev_path(path) { return Err(FsError::PermissionDenied); }
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // --- Mount point path (SMB delete) ---
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_mnt_mount(path, &state.mount_points) {
        if mnt_fs_type == FsType::Smb {
            let mount_path_owned = String::from(mount_path);
            let rel_parent_name = {
                let rel = relative_path.trim_end_matches('/');
                match rel.rfind('/') {
                    Some(0) => ("/", &rel[1..]),
                    Some(pos) => (&rel[..pos], &rel[pos + 1..]),
                    None => ("/", rel),
                }
            };
            let smb = state.smbfs.iter_mut()
                .find(|(p, _)| *p == mount_path_owned)
                .map(|(_, s)| s)
                .ok_or(FsError::IoError)?;
            let (parent_inode, _, _) = smb.lookup(rel_parent_name.0)?;
            return smb.delete_entry(parent_inode, rel_parent_name.1);
        }
        return Err(FsError::PermissionDenied);
    }

    // --- OverlayFS delete (whiteout for ISO files) ---
    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_mut().ok_or(FsError::IoError)?;
        return overlay.delete(iso, path);
    }

    let (parent_path, filename) = split_parent_name(path)?;
    if let Some(ref mut exfat) = state.exfat_fs {
        // Resolve parent with symlink following, but the filename itself is not followed
        let pr = resolve_exfat_path(exfat, parent_path, true)?;
        let (pc, _) = crate::fs::exfat::decode_inode(pr.inode);
        return exfat.delete_file(pc, filename);
    }
    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;
    let (parent_cluster, _, _) = fat.lookup(parent_path)?;
    fat.delete_file(parent_cluster, filename)
}

/// Rename (move) a file or directory from old_path to new_path.
pub fn rename(old_path: &str, new_path: &str) -> Result<(), FsError> {
    if is_dev_path(old_path) || is_dev_path(new_path) {
        return Err(FsError::PermissionDenied);
    }
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // --- OverlayFS rename ---
    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_mut().ok_or(FsError::IoError)?;
        return overlay.rename(iso, old_path, new_path);
    }

    let (old_parent, old_name) = split_parent_name(old_path)?;
    let (new_parent, new_name) = split_parent_name(new_path)?;

    if let Some(ref mut exfat) = state.exfat_fs {
        let old_pr = resolve_exfat_path(exfat, old_parent, true)?;
        let (old_pc, _) = crate::fs::exfat::decode_inode(old_pr.inode);
        let new_pr = resolve_exfat_path(exfat, new_parent, true)?;
        let (new_pc, _) = crate::fs::exfat::decode_inode(new_pr.inode);
        return exfat.rename_entry(old_pc, old_name, new_pc, new_name);
    }
    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;
    let (old_pc, _, _) = fat.lookup(old_parent)?;
    let (new_pc, _, _) = fat.lookup(new_parent)?;
    fat.rename_entry(old_pc, old_name, new_pc, new_name)
}

/// Create a directory at the given path.
pub fn mkdir(path: &str) -> Result<(), FsError> {
    if is_dev_path(path) { return Err(FsError::PermissionDenied); }
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // --- Mount point path (e.g. /mnt/target/...) ---
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_mnt_mount(path, &state.mount_points) {
        if mnt_fs_type == FsType::ExFat {
            let mount_path_owned = String::from(mount_path);
            let (parent_rel, dirname) = split_parent_name(relative_path)?;
            let exfat = state.mounted_exfat.iter_mut()
                .find(|(p, _)| *p == mount_path_owned)
                .map(|(_, fs)| fs)
                .ok_or(FsError::IoError)?;
            let (pr_inode, pr_type, _) = exfat.lookup(parent_rel)?;
            if pr_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            let (pc, _) = crate::fs::exfat::decode_inode(pr_inode);
            exfat.create_dir(pc, dirname)?;
            return Ok(());
        }
        return Err(FsError::PermissionDenied); // read-only mount points
    }

    // --- OverlayFS mkdir (writable RAM overlay) ---
    if state.overlay_fs.is_some() {
        let overlay = state.overlay_fs.as_mut().ok_or(FsError::IoError)?;
        return overlay.mkdir(path);
    }

    let (parent_path, dirname) = split_parent_name(path)?;
    if let Some(ref mut exfat) = state.exfat_fs {
        let pr = resolve_exfat_path(exfat, parent_path, true)?;
        if pr.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let (pc, _) = crate::fs::exfat::decode_inode(pr.inode);
        exfat.create_dir(pc, dirname)?;
        return Ok(());
    }
    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;
    let (parent_cluster, parent_type, _) = fat.lookup(parent_path)?;
    if parent_type != FileType::Directory {
        return Err(FsError::NotADirectory);
    }
    fat.create_dir(parent_cluster, dirname)?;
    Ok(())
}

/// Seek within an open file. `slot_id` is the global open_files index.
/// Returns new position.
pub fn lseek(slot_id: FileDescriptor, offset: i32, whence: u32) -> Result<u32, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    let file = state.open_files.get_mut(slot_id as usize)
        .and_then(|e| e.as_mut())
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

/// Get file type and size by path, following symlinks.
pub fn stat(path: &str) -> Result<StatResult, FsError> {
    stat_inner(path, true)
}

/// Get file type and size by path WITHOUT following the final symlink.
pub fn lstat(path: &str) -> Result<StatResult, FsError> {
    stat_inner(path, false)
}

fn stat_inner(path: &str, follow_last: bool) -> Result<StatResult, FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    let default_stat = |ft, sz, sym| StatResult {
        file_type: ft, size: sz, is_symlink: sym,
        uid: 0, gid: 0, mode: 0xFFF, mtime: 0,
    };

    // --- DevFs path ---
    if is_dev_path(path) {
        let name = dev_name(path);
        if name.is_empty() {
            return Ok(default_stat(FileType::Directory, 0, false));
        }
        let devfs = state.devfs.as_ref().ok_or(FsError::NotFound)?;
        if devfs.lookup(name).is_some() {
            return Ok(default_stat(FileType::Device, 0, false));
        }
        return Err(FsError::NotFound);
    }

    // Virtual directory paths
    if path == "/" { return Ok(default_stat(FileType::Directory, 0, false)); }
    if path == "/mnt" || path == "/mnt/" { return Ok(default_stat(FileType::Directory, 0, false)); }
    if path == "/dev" || path == "/dev/" { return Ok(default_stat(FileType::Directory, 0, false)); }

    // --- Mount point path ---
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_mnt_mount(path, &state.mount_points) {
        match mnt_fs_type {
            FsType::Iso9660 => {
                if let Some(ref iso) = state.iso9660_fs {
                    let (_inode, file_type, size) = iso.lookup(relative_path)?;
                    return Ok(default_stat(file_type, size, false));
                }
                return Err(FsError::NotFound);
            }
            FsType::Ntfs => {
                if let Some(ref ntfs) = state.ntfs_fs {
                    let (_inode, file_type, size) = ntfs.lookup(relative_path)?;
                    return Ok(default_stat(file_type, size, false));
                }
                return Err(FsError::NotFound);
            }
            FsType::ExFat => {
                let mount_path_owned = String::from(mount_path);
                let exfat = state.mounted_exfat.iter()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, fs)| fs)
                    .ok_or(FsError::IoError)?;
                let (_inode, file_type, size) = exfat.lookup(relative_path)?;
                return Ok(default_stat(file_type, size, false));
            }
            FsType::Smb => {
                let mount_path_owned = String::from(mount_path);
                let smb = state.smbfs.iter_mut()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, s)| s)
                    .ok_or(FsError::IoError)?;
                let (_inode, file_type, size) = smb.lookup(relative_path)?;
                return Ok(default_stat(file_type, size, false));
            }
            _ => return Err(FsError::NotFound),
        }
    }

    // --- exFAT path (with symlink resolution) ---
    if let Some(ref exfat) = state.exfat_fs {
        let r = resolve_exfat_path(exfat, path, follow_last)?;
        return Ok(StatResult {
            file_type: r.file_type,
            size: r.size,
            is_symlink: r.is_symlink,
            uid: r.uid,
            gid: r.gid,
            mode: r.mode,
            mtime: r.mtime,
        });
    }
    if let Some(ref ntfs) = state.ntfs_fs {
        let (file_type, size, _created, modified, _accessed) = ntfs.stat_path(path)?;
        return Ok(StatResult {
            file_type, size, is_symlink: false,
            uid: 0, gid: 0, mode: 0o555, mtime: modified,
        });
    }
    if let Some(ref fat) = state.fat_fs {
        let (_inode, file_type, size, mtime) = fat.stat_path(path)?;
        return Ok(StatResult {
            file_type, size, is_symlink: false,
            uid: 0, gid: 0, mode: 0xFFF, mtime,
        });
    }

    // --- OverlayFS root (CD-ROM boot with writable RAM overlay) ---
    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_ref().ok_or(FsError::IoError)?;
        let (file_type, size) = overlay.stat(iso, path)?;
        return Ok(StatResult {
            file_type, size, is_symlink: false,
            uid: 0, gid: 0, mode: 0xFFF, mtime: 0,
        });
    }

    // --- ISO 9660 root fallback (CD-ROM boot without overlay, read-only) ---
    if let Some(ref iso) = state.iso9660_fs {
        let (_inode, file_type, size) = iso.lookup(path)?;
        return Ok(StatResult {
            file_type, size, is_symlink: false,
            uid: 0, gid: 0, mode: 0o555, mtime: 0,
        });
    }

    Err(FsError::NotFound)
}

/// Get file info by slot_id (global open_files index).
/// Returns (file_type, size, position, mtime).
pub fn fstat(slot_id: FileDescriptor) -> Result<(FileType, u32, u32, u32), FsError> {
    let vfs = VFS.lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    let file = state.open_files.get(slot_id as usize)
        .and_then(|e| e.as_ref())
        .ok_or(FsError::BadFd)?;

    let path = file.path.clone();
    let ft = file.file_type;
    let sz = file.size;
    let pos = file.position;

    // Look up mtime from the filesystem
    let mtime = if let Some(ref exfat) = state.exfat_fs {
        resolve_exfat_path(exfat, &path, true).map(|r| r.mtime).unwrap_or(0)
    } else if let Some(ref ntfs) = state.ntfs_fs {
        ntfs.stat_path(&path).map(|(_, _, _, m, _)| m).unwrap_or(0)
    } else if let Some(ref fat) = state.fat_fs {
        fat.stat_path(&path).map(|(_, _, _, m)| m).unwrap_or(0)
    } else {
        0
    };

    Ok((ft, sz, pos, mtime))
}

/// Get the path associated with an open file descriptor.
pub fn get_fd_path(slot_id: FileDescriptor) -> Result<alloc::string::String, FsError> {
    let vfs = VFS.lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;
    let file = state.open_files.get(slot_id as usize)
        .and_then(|e| e.as_ref())
        .ok_or(FsError::BadFd)?;
    Ok(file.path.clone())
}

/// Truncate a file to zero length.
pub fn truncate(path: &str) -> Result<(), FsError> {
    if is_dev_path(path) { return Err(FsError::PermissionDenied); }
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // --- OverlayFS truncate ---
    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_mut().ok_or(FsError::IoError)?;
        return overlay.truncate(iso, path);
    }

    let (parent_path, filename) = split_parent_name(path)?;
    if let Some(ref mut exfat) = state.exfat_fs {
        let pr = resolve_exfat_path(exfat, parent_path, true)?;
        let (pc, _) = crate::fs::exfat::decode_inode(pr.inode);
        return exfat.truncate_file(pc, filename);
    }
    let fat = state.fat_fs.as_mut().ok_or(FsError::IoError)?;
    let (parent_cluster, _, _) = fat.lookup(parent_path)?;
    fat.truncate_file(parent_cluster, filename)
}

/// Mount a filesystem at the given path from userspace (syscall handler).
///
/// `mount_path`: where to mount (e.g. "/mnt/cdrom0")
/// `device`: device path (e.g. "/dev/cdrom0" or "//ip/share" for SMB)
/// `fs_type_id`: 0=FAT, 1=ISO9660, 4=NTFS, 5=SMB
///
/// Returns Ok(()) on success.
pub fn mount_fs(mount_path: &str, device: &str, fs_type_id: u32) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Check for duplicate mount point
    for mp in &state.mount_points {
        if mp.path == mount_path {
            return Err(FsError::AlreadyExists);
        }
    }

    match fs_type_id {
        0 | 7 => {
            // exFAT/FAT mount by device ID
            // device string = decimal device_id (from SYS_DISK_LIST)
            let dev_id: u8 = device.parse::<u8>().map_err(|_| {
                crate::serial_verbose_println!("  mount_fs: invalid device '{}' (expected numeric device_id)", device);
                FsError::InvalidPath
            })?;
            let bdev = crate::drivers::storage::blockdev::get_device(dev_id)
                .ok_or_else(|| {
                    crate::serial_verbose_println!("  mount_fs: device {} not found", dev_id);
                    FsError::NotFound
                })?;
            let start_lba = bdev.start_lba as u32;
            crate::serial_verbose_println!(
                "  mount_fs: exFAT device={} disk={} start_lba={}",
                dev_id, bdev.disk_id, start_lba
            );
            match ExFatFs::new(bdev.disk_id as u32, start_lba) {
                Ok(exfat) => {
                    state.mounted_exfat.push((String::from(mount_path), exfat));
                    state.mount_points.push(MountPoint {
                        path: String::from(mount_path),
                        fs_type: FsType::ExFat,
                        device_id: dev_id as u32,
                    });
                    crate::serial_verbose_println!("  Mounted exFAT at '{}'", mount_path);
                    Ok(())
                }
                Err(e) => {
                    crate::serial_verbose_println!("  mount_fs: ExFatFs::new() failed: {:?}", e);
                    Err(e)
                }
            }
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
            crate::serial_verbose_println!("  Mounted ISO 9660 at '{}'", mount_path);
            Ok(())
        }
        4 => {
            // NTFS (read-only)
            if state.ntfs_fs.is_some() {
                // Already have an NTFS instance — just add mount point
            } else {
                match NtfsFs::new(0, root_partition_lba()) {
                    Ok(ntfs) => {
                        state.ntfs_fs = Some(ntfs);
                    }
                    Err(e) => return Err(e),
                }
            }
            state.mount_points.push(MountPoint {
                path: String::from(mount_path),
                fs_type: FsType::Ntfs,
                device_id: 0,
            });
            crate::serial_verbose_println!("  Mounted NTFS (read-only) at '{}'", mount_path);
            Ok(())
        }
        5 => {
            // SMB (network filesystem)
            // device = "//server_ip/share_name"
            // Must drop VFS lock before TCP connect (TCP uses poll which might need VFS)
            drop(vfs);
            let smb = SmbFs::connect(device)?;
            // Re-acquire lock to update state
            let mut vfs = VFS.lock();
            let state = vfs.as_mut().ok_or(FsError::IoError)?;
            // Re-check for duplicate (could have been added while lock was dropped)
            for mp in &state.mount_points {
                if mp.path == mount_path {
                    smb.disconnect();
                    return Err(FsError::AlreadyExists);
                }
            }
            state.mount_points.push(MountPoint {
                path: String::from(mount_path),
                fs_type: FsType::Smb,
                device_id: 0,
            });
            state.smbfs.push((String::from(mount_path), smb));
            crate::serial_verbose_println!("  Mounted SMB at '{}'", mount_path);
            Ok(())
        }
        _ => Err(FsError::InvalidPath),
    }
}

/// Flush metadata for a specific open file to disk (fsync semantics).
/// Ensures all deferred FAT/bitmap writes for the file's filesystem are persisted.
pub fn fsync(slot_id: FileDescriptor) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    let file = state.open_files.get(slot_id as usize)
        .and_then(|e| e.as_ref())
        .ok_or(FsError::BadFd)?;

    let fs_id = file.fs_id;

    let mut disks_to_flush: Vec<u8> = Vec::new();
    match fs_id {
        3 => {
            if let Some(disk_id) = commit_open_exfat_entry(state, slot_id as usize, true)? {
                queue_disk_flush(&mut disks_to_flush, disk_id);
            }
        }
        6 => {
            if let Some(disk_id) = commit_open_exfat_entry(state, slot_id as usize, true)? {
                queue_disk_flush(&mut disks_to_flush, disk_id);
            }
        }
        _ => {} // Other filesystems flush synchronously already
    }

    // Flush write-back block cache, then storage hardware cache
    drop(vfs);
    flush_blockcache_for_disks(&disks_to_flush);
    crate::drivers::storage::flush();
    Ok(())
}

/// Flush all dirty filesystem metadata and storage write caches.
pub fn sync_all() {
    let mut disks_to_flush: Vec<u8> = Vec::new();
    let mut vfs = VFS.lock();
    if let Some(state) = vfs.as_mut() {
        for idx in 0..state.open_files.len() {
            let needs_commit = state.open_files.get(idx)
                .and_then(|e| e.as_ref())
                .map(|file| (file.fs_id == 3 || file.fs_id == 6) && file.entry_dirty)
                .unwrap_or(false);
            if needs_commit {
                if let Ok(Some(disk_id)) = commit_open_exfat_entry(state, idx, false) {
                    queue_disk_flush(&mut disks_to_flush, disk_id);
                }
            }
        }
        // Flush root exFAT metadata
        if let Some(ref mut exfat) = state.exfat_fs {
            let _ = exfat.flush_metadata();
            queue_disk_flush(&mut disks_to_flush, exfat.device_id as u8);
        }
        // Flush all mounted exFAT filesystems
        for (_path, exfat) in &mut state.mounted_exfat {
            let _ = exfat.flush_metadata();
            queue_disk_flush(&mut disks_to_flush, exfat.device_id as u8);
        }
    }
    drop(vfs);
    // Flush write-back block cache to disk (coalesced writes)
    flush_blockcache_for_disks(&disks_to_flush);
    // Then flush the drive's hardware write cache to persistent media
    crate::drivers::storage::flush();
}

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

        // If it was mounted exFAT, flush metadata and remove the fs instance
        if mp.fs_type == FsType::ExFat {
            if let Some(idx) = state.mounted_exfat.iter().position(|(p, _)| p == mount_path) {
                // Flush any pending metadata before dropping
                let _ = state.mounted_exfat[idx].1.flush_metadata();
                state.mounted_exfat.remove(idx);
            }
        }

        // If it was SMB, remove and disconnect the SmbFs instance
        if mp.fs_type == FsType::Smb {
            if let Some(idx) = state.smbfs.iter().position(|(p, _)| p == mount_path) {
                let (_, smb) = state.smbfs.remove(idx);
                // Drop VFS lock before TCP close (which may do network I/O)
                drop(vfs);
                smb.disconnect();
                // Re-log after disconnect
                crate::serial_verbose_println!("  Unmounted SMB '{}'", mount_path);
                return Ok(());
            }
        }

        crate::serial_verbose_println!("  Unmounted '{}'", mount_path);
        Ok(())
    } else {
        Err(FsError::NotFound)
    }
}

/// Create a symbolic link at `link_path` pointing to `target`.
/// Only supported on exFAT filesystems.
pub fn create_symlink(link_path: &str, target: &str) -> Result<(), FsError> {
    if is_dev_path(link_path) { return Err(FsError::PermissionDenied); }
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    let (parent_path, link_name) = split_parent_name(link_path)?;
    if let Some(ref mut exfat) = state.exfat_fs {
        let pr = resolve_exfat_path(exfat, parent_path, true)?;
        if pr.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let (pc, _) = crate::fs::exfat::decode_inode(pr.inode);
        return exfat.create_symlink(pc, link_name, target);
    }
    // FAT16 does not support symlinks
    Err(FsError::PermissionDenied)
}

/// Read the target of a symbolic link WITHOUT following it.
/// Returns the target path string.
pub fn readlink(path: &str) -> Result<String, FsError> {
    if is_dev_path(path) { return Err(FsError::InvalidPath); }
    let vfs = VFS.lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    if let Some(ref exfat) = state.exfat_fs {
        // Resolve all path components EXCEPT the final one
        let r = resolve_exfat_path(exfat, path, false)?;
        if !r.is_symlink {
            return Err(FsError::InvalidPath); // Not a symlink
        }
        return exfat.readlink(r.inode, r.size);
    }
    Err(FsError::PermissionDenied)
}

/// Get (uid, gid, mode) for a path. Returns defaults for non-exFAT filesystems.
pub fn get_permissions(path: &str) -> Result<(u16, u16, u16), FsError> {
    let vfs = VFS.lock();
    let state = vfs.as_ref().ok_or(FsError::NotFound)?;

    // Virtual paths always have root/full-access
    if path == "/dev" || path.starts_with("/dev/") || path == "/mnt" || path.starts_with("/mnt/") {
        return Ok((0, 0, 0xFFF));
    }

    if let Some(ref exfat) = state.exfat_fs {
        return exfat.get_permissions(path);
    }

    // FAT16 / other: no permission support
    Ok((0, 0, 0xFFF))
}

/// Set the mode bits for a path.
pub fn set_mode(path: &str, mode: u16) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::NotFound)?;

    if let Some(ref mut exfat) = state.exfat_fs {
        return exfat.set_mode(path, mode);
    }
    Err(FsError::PermissionDenied)
}

/// Set the owner (uid, gid) for a path.
pub fn set_owner(path: &str, uid: u16, gid: u16) -> Result<(), FsError> {
    let mut vfs = VFS.lock();
    let state = vfs.as_mut().ok_or(FsError::NotFound)?;

    if let Some(ref mut exfat) = state.exfat_fs {
        return exfat.set_owner(path, uid, gid);
    }
    Err(FsError::PermissionDenied)
}

/// Returns `true` if the root filesystem is ISO 9660 (live-CD / read-only boot).
/// Used by the permission system to skip persisted-permission checks.
pub fn root_is_iso9660() -> bool {
    let vfs = VFS.lock();
    if let Some(ref state) = *vfs {
        state.mount_points.iter().any(|mp| mp.path == "/" && mp.fs_type == FsType::Iso9660)
    } else {
        false
    }
}

/// Get filesystem statistics for a mount point path.
/// Returns `None` if the path is not a valid mount point or no stats available.
pub fn statfs(path: &str) -> Option<StatFs> {
    let vfs = VFS.lock();
    let state = vfs.as_ref()?;

    // Try all mount points matching the path (there can be multiple, e.g.
    // a failed disk mount + a successful ISO mount both at "/").
    for mp in state.mount_points.iter().filter(|mp| mp.path == path) {
        let result = match mp.fs_type {
            FsType::ExFat => {
                if path == "/" || path.is_empty() {
                    if let Some(ref fs) = state.exfat_fs {
                        let (total, free) = fs.fs_stats();
                        Some(StatFs { total_bytes: total, used_bytes: total - free, free_bytes: free })
                    } else {
                        None
                    }
                } else {
                    state.mounted_exfat.iter()
                        .find(|(mnt_path, _)| mnt_path == path)
                        .map(|(_, fs)| {
                            let (total, free) = fs.fs_stats();
                            StatFs { total_bytes: total, used_bytes: total - free, free_bytes: free }
                        })
                }
            }
            FsType::Iso9660 => {
                state.iso9660_fs.as_ref().map(|iso| {
                    let total = iso.total_blocks as u64 * 2048;
                    StatFs { total_bytes: total, used_bytes: total, free_bytes: 0 }
                })
            }
            FsType::Ntfs => {
                state.ntfs_fs.as_ref().map(|ntfs| {
                    let total = ntfs.total_sectors as u64 * 512;
                    StatFs { total_bytes: total, used_bytes: total, free_bytes: 0 }
                })
            }
            FsType::Fat => {
                state.fat_fs.as_ref().map(|fat| {
                    let cluster_bytes = fat.sectors_per_cluster as u64 * fat.bytes_per_sector as u64;
                    let total = fat.total_clusters as u64 * cluster_bytes;
                    StatFs { total_bytes: total, used_bytes: total, free_bytes: 0 }
                })
            }
            FsType::DevFs | FsType::Smb | FsType::Overlay => None,
            // CoreFS-Mount-Pfad ist aktuell read-only und liefert (noch) keine
            // StatFs-Werte; sobald `CoreFsDriver` total/free bereitstellt, wird
            // das hier befüllt.
            FsType::CoreFs => None,
        };
        if result.is_some() {
            return result;
        }
    }
    None
}

/// List all current mount points. Returns Vec of (mount_path, fs_type_name, device_id).
pub fn list_mounts() -> Vec<(String, &'static str, u32)> {
    let vfs = VFS.lock();
    if let Some(ref state) = *vfs {
        state.mount_points.iter().map(|mp| {
            let fs_name = match mp.fs_type {
                FsType::ExFat => "exfat",
                FsType::Fat => "fat16",
                FsType::Iso9660 => "iso9660",
                FsType::Ntfs => "ntfs",
                FsType::DevFs => "devfs",
                FsType::Smb => "smb",
                FsType::Overlay => "overlay",
                FsType::CoreFs => "corefs",
            };
            (mp.path.clone(), fs_name, mp.device_id)
        }).collect()
    } else {
        Vec::new()
    }
}
