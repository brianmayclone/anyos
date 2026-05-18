//! Virtual File System (VFS) -- unified interface for file descriptors, open/read/write/close.
//! Delegates to the mounted filesystem (exFAT or FAT16) and manages the global open file table.

mod cache;
pub(crate) mod path;
mod types;

use self::path::{dev_name, find_submount, is_dev_path, resolve_exfat_path, split_parent_name};
pub use self::types::{Filesystem, FsError, FsType, StatFs, StatResult};
use crate::fs::devfs::DevFs;
use crate::fs::exfat::{ExFatFs, ExFatFsDriver};
use crate::fs::fat::{FatFs, FatFsDriver};
use crate::fs::file::{DirEntry, FileDescriptor, FileFlags, FileType, OpenFile};
use crate::fs::iso9660::Iso9660Fs;
use crate::fs::ntfs::{NtfsFs, NtfsFsDriver};
use crate::fs::overlayfs::OverlayFs;
use crate::fs::smbfs::SmbFs;
use crate::sync::mutex::{Mutex, MutexGuard};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};
use core::panic::Location;
use core::sync::atomic::{AtomicU32, Ordering};

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
/// blockdev ID of the root partition, or u32::MAX if not resolved yet.
/// Reported by `list_mounts()` so userspace (df, mount) can display the
/// correct /dev/sdX name for "/" — the boot-time mount() call passes the
/// placeholder device_id=0 which would otherwise alias to the first
/// registered blockdev (often the tempdisk).
static ROOT_BLOCKDEV_ID: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// blockdev ID of the dedicated `/boot` partition (Partition 1 in a
/// dual-partition layout), or `u32::MAX` if the image uses the classic
/// single-partition layout.  Set by the boot-side partition scanner —
/// see `kernel/src/boot/x86/storage.rs::detect_and_register_root_partition`.
static BOOT_BLOCKDEV_ID: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

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

fn flush_hardware_for_disks(disks: &[u8]) {
    if disks.is_empty() {
        crate::drivers::storage::flush();
        return;
    }
    for &disk_id in disks {
        crate::drivers::storage::flush_disk(disk_id);
    }
}

struct DetachedExFatCommit {
    driver: Arc<ExFatFsDriver>,
    filename: String,
    parent_cluster: u32,
    inode: u32,
    size: u32,
    entry_dirty: bool,
    durable: bool,
}

fn snapshot_open_exfat_commit(
    state: &VfsState,
    slot_id: usize,
    durable: bool,
) -> Result<Option<DetachedExFatCommit>, FsError> {
    let file = state
        .open_files
        .get(slot_id)
        .and_then(|e| e.as_ref())
        .ok_or(FsError::BadFd)?;

    let driver = match file.fs_id {
        3 => state.exfat_fs.as_ref().map(Arc::clone),
        6 => state.mounted_exfat_handle_for_path(&file.path),
        _ => return Ok(None),
    }
    .ok_or(FsError::IoError)?;

    let filename = file.path.rsplit('/').next().unwrap_or("");
    if filename.is_empty() {
        return Ok(None);
    }

    Ok(Some(DetachedExFatCommit {
        driver,
        filename: String::from(filename),
        parent_cluster: file.parent_cluster,
        inode: file.inode,
        size: file.size,
        entry_dirty: file.entry_dirty,
        durable,
    }))
}

fn finish_detached_exfat_commit(commit: &DetachedExFatCommit) -> Result<u8, FsError> {
    let mut exfat = commit.driver.lock_inner();
    if commit.entry_dirty {
        exfat.update_entry(
            commit.parent_cluster,
            &commit.filename,
            commit.size,
            commit.inode,
        )?;
    }
    if commit.durable && exfat.metadata_dirty {
        exfat.flush_metadata()?;
    }
    Ok(exfat.device_id as u8)
}

fn mark_detached_exfat_commit_clean(
    state: &mut VfsState,
    slot_id: usize,
    commit: &DetachedExFatCommit,
) {
    if !commit.entry_dirty {
        return;
    }
    if let Some(Some(file)) = state.open_files.get_mut(slot_id) {
        let filename = file.path.rsplit('/').next().unwrap_or("");
        if filename == commit.filename
            && file.parent_cluster == commit.parent_cluster
            && file.inode == commit.inode
            && file.size == commit.size
        {
            file.entry_dirty = false;
        }
    }
}

fn commit_open_exfat_entry(
    state: &mut VfsState,
    slot_id: usize,
    durable: bool,
) -> Result<Option<u8>, FsError> {
    let (fs_id, file_path, parent_cluster, inode, size, entry_dirty) = {
        let file = state
            .open_files
            .get(slot_id)
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
        let driver = state.exfat_fs.as_ref().ok_or(FsError::IoError)?;
        let mut exfat = driver.lock_inner();
        if entry_dirty {
            exfat.update_entry(parent_cluster, filename, size, inode)?;
        }
        if durable && exfat.metadata_dirty {
            exfat.flush_metadata()?;
        }
        exfat.device_id as u8
    } else {
        let exfat = state
            .mounted_exfat
            .iter_mut()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, fs)| fs)
            .ok_or(FsError::IoError)?;
        let mut exfat = exfat.lock_inner();
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

/// Record the blockdev ID that backs the root mount.
pub fn set_root_blockdev_id(id: u8) {
    ROOT_BLOCKDEV_ID.store(id as u32, core::sync::atomic::Ordering::Relaxed);
}

/// blockdev ID of the root partition, or `None` if not recorded.
pub fn root_blockdev_id() -> Option<u8> {
    let v = ROOT_BLOCKDEV_ID.load(core::sync::atomic::Ordering::Relaxed);
    if v == u32::MAX {
        None
    } else {
        Some(v as u8)
    }
}

/// Record the blockdev ID that backs the dedicated `/boot` partition
/// (Partition 1 in a dual-partition layout).  When unset (default), the
/// image is assumed to use a single-partition layout and no `/boot`
/// mount is created.
pub fn set_boot_blockdev_id(id: u8) {
    BOOT_BLOCKDEV_ID.store(id as u32, core::sync::atomic::Ordering::Relaxed);
}

/// blockdev ID of the `/boot` partition, or `None` if the image has no
/// dedicated boot partition.
pub fn boot_blockdev_id() -> Option<u8> {
    let v = BOOT_BLOCKDEV_ID.load(core::sync::atomic::Ordering::Relaxed);
    if v == u32::MAX {
        None
    } else {
        Some(v as u8)
    }
}

/// Mount the `/boot` partition if one was discovered during partition
/// scanning.  Must be called after the root `/` mount is in place,
/// because the VFS-state must already be initialized.
///
/// Dispatch for `/boot/*` paths goes through
/// [`path::find_submount`] — the same mechanism that routes `/mnt/*`
/// and `/System` mounts to their respective FS drivers.
pub fn mount_boot_if_present() {
    let dev_id = match boot_blockdev_id() {
        Some(id) => id,
        None => return,
    };
    // fs_type_id=0 triggers the exFAT/FAT auto-detect path inside
    // mount_fs, which covers both FAT32 and exFAT boot partitions.
    let dev_str = alloc::format!("{}", dev_id as u32);
    match mount_fs("/boot", &dev_str, 0) {
        Ok(()) => crate::serial_println!("  Mounted /boot (device {})", dev_id),
        Err(e) => crate::serial_println!(
            "  Warning: /boot mount failed on device {}: {:?}",
            dev_id,
            e
        ),
    }
}

/// Invalidate all directory cache entries after path topology changes.
pub fn dir_cache_invalidate() {
    cache::dir_cache_invalidate();
}

static VFS: Mutex<Option<VfsState>> = Mutex::new(None);
static VFS_LOCK_OWNER_TID: AtomicU32 = AtomicU32::new(0);
static VFS_LOCK_OWNER_LINE: AtomicU32 = AtomicU32::new(0);
static VFS_LOCK_WAIT_LOGS: AtomicU32 = AtomicU32::new(0);
static VFS_LOCK_HOLD_LOGS: AtomicU32 = AtomicU32::new(0);
const VFS_LOCK_WAIT_WARN_MS: u32 = 50;
const VFS_LOCK_HOLD_WARN_MS: u32 = 50;
const VFS_LOCK_LOG_LIMIT: u32 = 128;

struct VfsLockGuard {
    guard: Option<MutexGuard<'static, Option<VfsState>>>,
    acquired_tick: u32,
    owner_tid: u32,
    caller_line: u32,
}

impl Deref for VfsLockGuard {
    type Target = Option<VfsState>;

    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().expect("VFS lock guard present")
    }
}

impl DerefMut for VfsLockGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_mut().expect("VFS lock guard present")
    }
}

impl Drop for VfsLockGuard {
    fn drop(&mut self) {
        let held_ms = vfs_ticks_to_ms(
            crate::arch::hal::timer_current_ticks().wrapping_sub(self.acquired_tick),
        );
        if held_ms >= VFS_LOCK_HOLD_WARN_MS
            && VFS_LOCK_HOLD_LOGS.fetch_add(1, Ordering::Relaxed) < VFS_LOCK_LOG_LIMIT
        {
            crate::serial_println!(
                "[vfs] VFS lock held {} ms tid={} line={}",
                held_ms,
                self.owner_tid,
                self.caller_line
            );
        }
        self.guard.take();
        let _ = VFS_LOCK_OWNER_TID.compare_exchange(
            self.owner_tid,
            0,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        let _ = VFS_LOCK_OWNER_LINE.compare_exchange(
            self.caller_line,
            0,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }
}

fn vfs_ticks_to_ms(ticks: u32) -> u32 {
    let hz = crate::arch::hal::timer_frequency_hz() as u32;
    if hz == 0 {
        ticks
    } else {
        ((ticks as u64 * 1000) / hz as u64) as u32
    }
}

#[track_caller]
fn vfs_lock() -> VfsLockGuard {
    let caller_line = Location::caller().line();
    let start = crate::arch::hal::timer_current_ticks();
    let owner_at_start = VFS_LOCK_OWNER_TID.load(Ordering::Relaxed);
    let owner_line_at_start = VFS_LOCK_OWNER_LINE.load(Ordering::Relaxed);
    let guard = VFS.lock();
    let acquired = crate::arch::hal::timer_current_ticks();
    let waited_ms = vfs_ticks_to_ms(acquired.wrapping_sub(start));
    let tid = crate::task::scheduler::current_tid();
    VFS_LOCK_OWNER_TID.store(tid, Ordering::Relaxed);
    VFS_LOCK_OWNER_LINE.store(caller_line, Ordering::Relaxed);
    if waited_ms >= VFS_LOCK_WAIT_WARN_MS
        && VFS_LOCK_WAIT_LOGS.fetch_add(1, Ordering::Relaxed) < VFS_LOCK_LOG_LIMIT
    {
        crate::serial_println!(
            "[vfs] VFS lock wait {} ms tid={} line={} owner_at_start={} owner_line={}",
            waited_ms,
            tid,
            caller_line,
            owner_at_start,
            owner_line_at_start
        );
    }
    VfsLockGuard {
        guard: Some(guard),
        acquired_tick: acquired,
        owner_tid: tid,
        caller_line,
    }
}

struct VfsState {
    open_files: Vec<Option<OpenFile>>,
    mount_points: Vec<MountPoint>,
    iso9660_fs: Option<Iso9660Fs>,
    devfs: Option<DevFs>,
    /// SMB network filesystem instances (mount_path, instance).
    /// Vec because multiple different SMB shares can be mounted simultaneously.
    smbfs: Vec<(String, SmbFs)>,
    /// Mounted exFAT instances (mount_path, instance) for additional partitions.
    /// Separate from `exfat_fs` which is the root filesystem.
    mounted_exfat: Vec<(String, Arc<ExFatFsDriver>)>,
    /// OverlayFS: writable RAM layer over ISO 9660 (active when booting from CD).
    overlay_fs: Option<OverlayFs>,
    /// CoreFS-Root-Treiber. Aktiv nur wenn die Root-Partition CoreFS ist.
    /// Für zusätzliche CoreFS-Sub-Mounts siehe `mounted_corefs`.
    corefs_driver: Option<Arc<crate::fs::corefs::CoreFsDriver>>,
    /// Zusätzliche CoreFS-Sub-Mounts (mount_path, driver). Analog zu
    /// `mounted_exfat` — jede Partition bekommt ihren eigenen Treiber, so
    /// dass `statfs` und File-I/O pro Mountpunkt korrekt dispatchen.
    mounted_corefs: Vec<(String, Arc<crate::fs::corefs::CoreFsDriver>)>,
    // -----------------------------------------------------------------
    // Per-FS typed root drivers (Phase 6).
    //
    // The root mount may be served by exactly one of these.  Per-FS
    // typed fields preserve Rust's field-level borrow splitting that
    // the legacy dispatch code relies on (`if let Some(ref mut x) =
    // state.exfat_fs { ... state.alloc_slot() ... }`), and let the
    // legacy code paths reach driver-specific APIs (cluster
    // bookkeeping, MFT records) without runtime downcast.  The
    // generic [`VfsState::root_fs`] accessor returns whichever is
    // active as a `&dyn Filesystem` for plain trait dispatch.
    /// exFAT root driver (active when root partition holds exFAT).
    exfat_fs: Option<Arc<ExFatFsDriver>>,
    /// FAT12/16/32 root driver.
    fat_fs: Option<FatFsDriver>,
    /// NTFS root driver (read-only).
    ntfs_fs: Option<NtfsFsDriver>,
    /// Catch-all slot for future plug-in filesystems whose driver
    /// does not need typed access (ext4, btrfs, …).  All access goes
    /// through the [`Filesystem`] trait.
    root_other: Option<alloc::boxed::Box<dyn Filesystem + Send + Sync>>,
    /// FsType of the active root filesystem.
    root_fs_type: Option<FsType>,
    /// Free slot stack for O(1) open file slot allocation.
    /// Contains indices of free (None) entries in open_files.
    free_slots: Vec<u32>,
}

enum DetachedReadBackend {
    CoreFs(Arc<crate::fs::corefs::CoreFsDriver>),
    ExFat(Arc<ExFatFsDriver>),
}

struct DetachedRead {
    backend: DetachedReadBackend,
    inode: u32,
    position: u32,
    size: u32,
    reserved: usize,
    path: String,
}

enum DetachedReadPrep {
    Unsupported,
    Eof,
    Ready(DetachedRead),
}

enum DetachedWriteBackend {
    CoreFs(Arc<crate::fs::corefs::CoreFsDriver>),
    ExFat(Arc<ExFatFsDriver>),
}

struct DetachedWrite {
    backend: DetachedWriteBackend,
    fs_id: u32,
    inode: u32,
    old_inode: u32,
    old_size: u32,
    position: u32,
    reserved: usize,
    path: String,
    parent_cluster: u32,
    seek_hint: Option<(u32, u32)>,
    sync_write: bool,
}

struct DetachedExFatWriteResult {
    new_cluster: u32,
    new_size: u32,
    hint_offset: u32,
    hint_cluster: u32,
}

enum DetachedWriteResult {
    CoreFs { bytes_written: usize },
    ExFat(DetachedExFatWriteResult),
}

enum DetachedWritePrep {
    Unsupported,
    Empty,
    Ready(DetachedWrite),
}

enum DetachedStatBackend {
    CoreFs(Arc<crate::fs::corefs::CoreFsDriver>),
    ExFat(Arc<ExFatFsDriver>),
}

struct DetachedStat {
    backend: DetachedStatBackend,
    path: String,
    follow_last: bool,
}

enum DetachedReadlinkBackend {
    CoreFs(Arc<crate::fs::corefs::CoreFsDriver>),
    ExFat(Arc<ExFatFsDriver>),
}

struct DetachedReadlink {
    backend: DetachedReadlinkBackend,
    path: String,
}

enum DetachedDeleteBackend {
    CoreFs(Arc<crate::fs::corefs::CoreFsDriver>),
    ExFat(Arc<ExFatFsDriver>),
}

enum DetachedOpenBackend {
    CoreFs(Arc<crate::fs::corefs::CoreFsDriver>),
    ExFat(Arc<ExFatFsDriver>),
}

struct DetachedOpen {
    backend: DetachedOpenBackend,
    path: String,
    lookup_path: String,
    flags: FileFlags,
    fs_id: u32,
}

struct DetachedOpenResult {
    path: String,
    file_type: FileType,
    flags: FileFlags,
    size: u32,
    fs_id: u32,
    inode: u32,
    parent_cluster: u32,
}

enum DetachedReadDirBackend {
    CoreFs(Arc<crate::fs::corefs::CoreFsDriver>),
    ExFat(Arc<ExFatFsDriver>),
}

enum DetachedRenameBackend {
    CoreFs(Arc<crate::fs::corefs::CoreFsDriver>),
    ExFat(Arc<ExFatFsDriver>),
}

enum DetachedMkdirBackend {
    CoreFs(Arc<crate::fs::corefs::CoreFsDriver>),
    ExFat(Arc<ExFatFsDriver>),
}

struct DetachedDelete {
    backend: DetachedDeleteBackend,
    parent_path: String,
    name: String,
}

struct DetachedReadDir {
    backend: DetachedReadDirBackend,
    path: String,
    add_dev: bool,
    add_mnt: bool,
}

struct DetachedRename {
    backend: DetachedRenameBackend,
    old_path: String,
    new_path: String,
}

struct DetachedMkdir {
    backend: DetachedMkdirBackend,
    parent_path: String,
    name: String,
}

fn root_stat_backend(state: &VfsState) -> Option<DetachedStatBackend> {
    if state.root_fs_type == Some(FsType::CoreFs) {
        return state
            .corefs_driver
            .as_ref()
            .map(|driver| DetachedStatBackend::CoreFs(Arc::clone(driver)));
    }
    if state.root_fs_type == Some(FsType::ExFat) {
        return state
            .exfat_fs
            .as_ref()
            .map(|driver| DetachedStatBackend::ExFat(Arc::clone(driver)));
    }
    None
}

fn split_delete_parent_name(path: &str) -> Result<(String, String), FsError> {
    let rel = if path.is_empty() { "/" } else { path }.trim_end_matches('/');
    let (parent, name) = match rel.rfind('/') {
        Some(0) => ("/", &rel[1..]),
        Some(pos) => (&rel[..pos], &rel[pos + 1..]),
        None => ("/", rel),
    };
    if name.is_empty() {
        return Err(FsError::InvalidPath);
    }
    Ok((String::from(parent), String::from(name)))
}

fn prepare_detached_open(path: &str, flags: FileFlags) -> Result<Option<DetachedOpen>, FsError> {
    if is_dev_path(path) {
        return Ok(None);
    }

    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    let active_count = state.open_files.iter().filter(|e| e.is_some()).count();
    if active_count >= MAX_OPEN_FILES {
        return Err(FsError::TooManyOpenFiles);
    }

    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        let q = if relative_path.is_empty() {
            "/"
        } else {
            relative_path
        };
        return match mnt_fs_type {
            FsType::CoreFs => {
                let driver = state
                    .mounted_corefs
                    .iter()
                    .find(|(p, _)| p == mount_path)
                    .map(|(_, d)| Arc::clone(d))
                    .ok_or(FsError::NotFound)?;
                Ok(Some(DetachedOpen {
                    backend: DetachedOpenBackend::CoreFs(driver),
                    path: String::from(path),
                    lookup_path: String::from(q),
                    flags,
                    fs_id: 8,
                }))
            }
            FsType::ExFat => {
                let driver = state
                    .mounted_exfat
                    .iter()
                    .find(|(p, _)| p == mount_path)
                    .map(|(_, d)| Arc::clone(d))
                    .ok_or(FsError::IoError)?;
                Ok(Some(DetachedOpen {
                    backend: DetachedOpenBackend::ExFat(driver),
                    path: String::from(path),
                    lookup_path: String::from(q),
                    flags,
                    fs_id: 6,
                }))
            }
            _ => Ok(None),
        };
    }

    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        return Ok(None);
    }

    if state.root_fs_type == Some(FsType::CoreFs) {
        return Ok(state.corefs_driver.as_ref().map(|driver| DetachedOpen {
            backend: DetachedOpenBackend::CoreFs(Arc::clone(driver)),
            path: String::from(path),
            lookup_path: String::from(if path.is_empty() { "/" } else { path }),
            flags,
            fs_id: 8,
        }));
    }
    if state.root_fs_type == Some(FsType::ExFat) {
        return Ok(state.exfat_fs.as_ref().map(|driver| DetachedOpen {
            backend: DetachedOpenBackend::ExFat(Arc::clone(driver)),
            path: String::from(path),
            lookup_path: String::from(if path.is_empty() { "/" } else { path }),
            flags,
            fs_id: 3,
        }));
    }

    Ok(None)
}

fn execute_detached_open(plan: DetachedOpen) -> Result<DetachedOpenResult, FsError> {
    match plan.backend {
        DetachedOpenBackend::CoreFs(driver) => {
            let q = if plan.lookup_path.is_empty() {
                "/"
            } else {
                plan.lookup_path.as_str()
            };
            let (inode, file_type, size) = match Filesystem::lookup(driver.as_ref(), q) {
                Ok(r) => {
                    if plan.flags.truncate && plan.flags.write {
                        driver.truncate_file(r.0, 0)?;
                        (r.0, r.1, 0)
                    } else {
                        r
                    }
                }
                Err(FsError::NotFound) if plan.flags.create => {
                    let (parent, name) = split_delete_parent_name(q)?;
                    let (parent_inode, parent_type, _) =
                        Filesystem::lookup(driver.as_ref(), &parent)?;
                    if parent_type != FileType::Directory {
                        return Err(FsError::NotADirectory);
                    }
                    let new_inode = Filesystem::create(
                        driver.as_ref(),
                        parent_inode,
                        &name,
                        FileType::Regular,
                    )?;
                    (new_inode, FileType::Regular, 0)
                }
                Err(e) => return Err(e),
            };

            Ok(DetachedOpenResult {
                path: plan.path,
                file_type,
                flags: plan.flags,
                size,
                fs_id: plan.fs_id,
                inode,
                parent_cluster: 0,
            })
        }
        DetachedOpenBackend::ExFat(driver) => {
            let mut exfat = driver.lock_inner();
            let q = if plan.lookup_path.is_empty() {
                "/"
            } else {
                plan.lookup_path.as_str()
            };
            let lookup_result = if plan.flags.write || plan.flags.create || plan.flags.truncate {
                exfat.lookup(q)
            } else {
                resolve_exfat_path(&exfat, q, true).map(|r| (r.inode, r.file_type, r.size))
            };
            let (inode, file_type, size, parent_cluster) = match lookup_result {
                Ok((inode, file_type, size)) => {
                    if plan.flags.truncate && plan.flags.write {
                        let (parent_path, filename) = split_parent_name(q)?;
                        let (pr_inode, _, _) = if plan.fs_id == 3 {
                            let pr = resolve_exfat_path(&exfat, parent_path, true)?;
                            (pr.inode, pr.file_type, pr.size)
                        } else {
                            exfat.lookup(parent_path)?
                        };
                        let (pc, _) = crate::fs::exfat::decode_inode(pr_inode);
                        exfat.truncate_file(pc, filename)?;
                        (0u32, file_type, 0u32, pc)
                    } else {
                        let parent_cluster = if plan.flags.write {
                            let (parent_path, _) = split_parent_name(q)?;
                            if plan.fs_id == 3 {
                                resolve_exfat_path(&exfat, parent_path, true)
                                    .map(|pr| crate::fs::exfat::decode_inode(pr.inode).0)
                                    .unwrap_or(0)
                            } else {
                                exfat
                                    .lookup(parent_path)
                                    .map(|(i, _, _)| crate::fs::exfat::decode_inode(i).0)
                                    .unwrap_or(0)
                            }
                        } else {
                            0
                        };
                        (inode, file_type, size, parent_cluster)
                    }
                }
                Err(FsError::NotFound) if plan.flags.create => {
                    let (parent_path, filename) = split_parent_name(q)?;
                    let (pr_inode, pr_type, _) = if plan.fs_id == 3 {
                        let pr = resolve_exfat_path(&exfat, parent_path, true)?;
                        (pr.inode, pr.file_type, pr.size)
                    } else {
                        exfat.lookup(parent_path)?
                    };
                    if pr_type != FileType::Directory {
                        return Err(FsError::NotADirectory);
                    }
                    let pc = crate::fs::exfat::decode_inode(pr_inode).0;
                    exfat.create_file(pc, filename)?;
                    (0u32, FileType::Regular, 0u32, pc)
                }
                Err(e) => return Err(e),
            };

            Ok(DetachedOpenResult {
                path: plan.path,
                file_type,
                flags: plan.flags,
                size,
                fs_id: plan.fs_id,
                inode,
                parent_cluster,
            })
        }
    }
}

fn insert_detached_open(result: DetachedOpenResult) -> Result<FileDescriptor, FsError> {
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    let active_count = state.open_files.iter().filter(|e| e.is_some()).count();
    if active_count >= MAX_OPEN_FILES {
        return Err(FsError::TooManyOpenFiles);
    }

    let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
    let position = if result.flags.append { result.size } else { 0 };
    let file = OpenFile {
        fd: slot_id,
        path: result.path,
        file_type: result.file_type,
        flags: result.flags,
        position,
        size: result.size,
        fs_id: result.fs_id,
        inode: result.inode,
        parent_cluster: result.parent_cluster,
        refcount: 1,
        seek_cache_offset: 0,
        seek_cache_cluster: 0,
        entry_dirty: false,
    };
    state.open_files[slot_id as usize] = Some(file);
    Ok(slot_id)
}

fn prepare_detached_delete(path: &str) -> Result<Option<DetachedDelete>, FsError> {
    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        return match mnt_fs_type {
            FsType::CoreFs => {
                let driver = state
                    .mounted_corefs
                    .iter()
                    .find(|(p, _)| p == mount_path)
                    .map(|(_, d)| Arc::clone(d))
                    .ok_or(FsError::NotFound)?;
                let rel = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                let (parent_path, name) = split_delete_parent_name(rel)?;
                Ok(Some(DetachedDelete {
                    backend: DetachedDeleteBackend::CoreFs(driver),
                    parent_path,
                    name,
                }))
            }
            FsType::ExFat => {
                let driver = state
                    .mounted_exfat
                    .iter()
                    .find(|(p, _)| p == mount_path)
                    .map(|(_, d)| Arc::clone(d))
                    .ok_or(FsError::IoError)?;
                let rel = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                let (parent_path, name) = split_delete_parent_name(rel)?;
                Ok(Some(DetachedDelete {
                    backend: DetachedDeleteBackend::ExFat(driver),
                    parent_path,
                    name,
                }))
            }
            _ => Ok(None),
        };
    }

    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        return Ok(None);
    }

    let (parent_path, name) = split_delete_parent_name(path)?;
    if state.root_fs_type == Some(FsType::CoreFs) {
        return Ok(state.corefs_driver.as_ref().map(|driver| DetachedDelete {
            backend: DetachedDeleteBackend::CoreFs(Arc::clone(driver)),
            parent_path,
            name,
        }));
    }
    if state.root_fs_type == Some(FsType::ExFat) {
        return Ok(state.exfat_fs.as_ref().map(|driver| DetachedDelete {
            backend: DetachedDeleteBackend::ExFat(Arc::clone(driver)),
            parent_path,
            name,
        }));
    }
    Ok(None)
}

fn execute_detached_delete(plan: DetachedDelete) -> Result<(), FsError> {
    match plan.backend {
        DetachedDeleteBackend::CoreFs(driver) => {
            let (parent_inode, parent_type, _) =
                Filesystem::lookup(driver.as_ref(), &plan.parent_path)?;
            if parent_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            Filesystem::delete(driver.as_ref(), parent_inode, &plan.name)
        }
        DetachedDeleteBackend::ExFat(driver) => {
            let (parent_inode, parent_type, _) =
                Filesystem::lookup(driver.as_ref(), &plan.parent_path)?;
            if parent_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            Filesystem::delete(driver.as_ref(), parent_inode, &plan.name)
        }
    }
}

fn prepare_detached_read_dir(path: &str) -> Result<Option<DetachedReadDir>, FsError> {
    if path == "/dev" || path == "/dev/" || path == "/mnt" || path == "/mnt/" {
        return Ok(None);
    }

    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        return match mnt_fs_type {
            FsType::CoreFs => {
                let driver = state
                    .mounted_corefs
                    .iter()
                    .find(|(p, _)| p == mount_path)
                    .map(|(_, d)| Arc::clone(d))
                    .ok_or(FsError::NotFound)?;
                let q = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                Ok(Some(DetachedReadDir {
                    backend: DetachedReadDirBackend::CoreFs(driver),
                    path: String::from(q),
                    add_dev: false,
                    add_mnt: false,
                }))
            }
            FsType::ExFat => {
                let driver = state
                    .mounted_exfat
                    .iter()
                    .find(|(p, _)| p == mount_path)
                    .map(|(_, d)| Arc::clone(d))
                    .ok_or(FsError::IoError)?;
                let q = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                Ok(Some(DetachedReadDir {
                    backend: DetachedReadDirBackend::ExFat(driver),
                    path: String::from(q),
                    add_dev: false,
                    add_mnt: false,
                }))
            }
            _ => Ok(None),
        };
    }

    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        return Ok(None);
    }

    let add_dev = path == "/" && state.devfs.is_some();
    let add_mnt = path == "/"
        && state
            .mount_points
            .iter()
            .any(|mp| mp.path.starts_with("/mnt/"));
    let q = if path.is_empty() { "/" } else { path };
    if state.root_fs_type == Some(FsType::CoreFs) {
        return Ok(state.corefs_driver.as_ref().map(|driver| DetachedReadDir {
            backend: DetachedReadDirBackend::CoreFs(Arc::clone(driver)),
            path: String::from(q),
            add_dev,
            add_mnt,
        }));
    }
    if state.root_fs_type == Some(FsType::ExFat) {
        return Ok(state.exfat_fs.as_ref().map(|driver| DetachedReadDir {
            backend: DetachedReadDirBackend::ExFat(Arc::clone(driver)),
            path: String::from(q),
            add_dev,
            add_mnt,
        }));
    }
    Ok(None)
}

fn execute_detached_read_dir(plan: DetachedReadDir) -> Result<Vec<DirEntry>, FsError> {
    let (inode, file_type, mut entries) = match plan.backend {
        DetachedReadDirBackend::CoreFs(driver) => {
            let (inode, file_type, _) = Filesystem::lookup(driver.as_ref(), &plan.path)?;
            let entries = if file_type == FileType::Directory {
                Filesystem::readdir(driver.as_ref(), inode)?
            } else {
                Vec::new()
            };
            (inode, file_type, entries)
        }
        DetachedReadDirBackend::ExFat(driver) => {
            let (inode, file_type, _) = Filesystem::lookup(driver.as_ref(), &plan.path)?;
            let entries = if file_type == FileType::Directory {
                Filesystem::readdir(driver.as_ref(), inode)?
            } else {
                Vec::new()
            };
            (inode, file_type, entries)
        }
    };
    let _ = inode;
    if file_type != FileType::Directory {
        return Err(FsError::NotADirectory);
    }
    add_virtual_root_entries_snapshot(plan.add_dev, plan.add_mnt, &mut entries);
    Ok(entries)
}

fn prepare_detached_rename(
    old_path: &str,
    new_path: &str,
) -> Result<Option<DetachedRename>, FsError> {
    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    let old_sub = find_submount(old_path, &state.mount_points)
        .map(|(mp, rel, t)| (String::from(mp), String::from(rel), t));
    let new_sub = find_submount(new_path, &state.mount_points)
        .map(|(mp, rel, t)| (String::from(mp), String::from(rel), t));

    match (old_sub, new_sub) {
        (Some((old_mp, old_rel, old_type)), Some((new_mp, new_rel, new_type))) => {
            if old_mp != new_mp || old_type != new_type {
                if matches!(old_type, FsType::CoreFs | FsType::ExFat)
                    || matches!(new_type, FsType::CoreFs | FsType::ExFat)
                {
                    return Err(FsError::PermissionDenied);
                }
                return Ok(None);
            }
            let old_q = if old_rel.is_empty() {
                "/"
            } else {
                old_rel.as_str()
            };
            let new_q = if new_rel.is_empty() {
                "/"
            } else {
                new_rel.as_str()
            };
            match old_type {
                FsType::CoreFs => {
                    let driver = state
                        .mounted_corefs
                        .iter()
                        .find(|(p, _)| p == &old_mp)
                        .map(|(_, d)| Arc::clone(d))
                        .ok_or(FsError::NotFound)?;
                    Ok(Some(DetachedRename {
                        backend: DetachedRenameBackend::CoreFs(driver),
                        old_path: String::from(old_q),
                        new_path: String::from(new_q),
                    }))
                }
                FsType::ExFat => {
                    let driver = state
                        .mounted_exfat
                        .iter()
                        .find(|(p, _)| p == &old_mp)
                        .map(|(_, d)| Arc::clone(d))
                        .ok_or(FsError::IoError)?;
                    Ok(Some(DetachedRename {
                        backend: DetachedRenameBackend::ExFat(driver),
                        old_path: String::from(old_q),
                        new_path: String::from(new_q),
                    }))
                }
                _ => Ok(None),
            }
        }
        (Some((_, _, t)), None) | (None, Some((_, _, t)))
            if matches!(t, FsType::CoreFs | FsType::ExFat) =>
        {
            Err(FsError::PermissionDenied)
        }
        (Some(_), None) | (None, Some(_)) => Ok(None),
        (None, None) => {
            if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
                return Ok(None);
            }
            if state.root_fs_type == Some(FsType::CoreFs) {
                return Ok(state.corefs_driver.as_ref().map(|driver| DetachedRename {
                    backend: DetachedRenameBackend::CoreFs(Arc::clone(driver)),
                    old_path: String::from(old_path),
                    new_path: String::from(new_path),
                }));
            }
            if state.root_fs_type == Some(FsType::ExFat) {
                return Ok(state.exfat_fs.as_ref().map(|driver| DetachedRename {
                    backend: DetachedRenameBackend::ExFat(Arc::clone(driver)),
                    old_path: String::from(old_path),
                    new_path: String::from(new_path),
                }));
            }
            Ok(None)
        }
    }
}

fn execute_detached_rename(plan: DetachedRename) -> Result<(), FsError> {
    match plan.backend {
        DetachedRenameBackend::CoreFs(driver) => {
            Filesystem::rename(driver.as_ref(), &plan.old_path, &plan.new_path)
        }
        DetachedRenameBackend::ExFat(driver) => {
            Filesystem::rename(driver.as_ref(), &plan.old_path, &plan.new_path)
        }
    }
}

fn prepare_detached_mkdir(path: &str) -> Result<Option<DetachedMkdir>, FsError> {
    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        return match mnt_fs_type {
            FsType::CoreFs => {
                let driver = state
                    .mounted_corefs
                    .iter()
                    .find(|(p, _)| p == mount_path)
                    .map(|(_, d)| Arc::clone(d))
                    .ok_or(FsError::NotFound)?;
                let rel = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                let (parent_path, name) = split_delete_parent_name(rel)?;
                Ok(Some(DetachedMkdir {
                    backend: DetachedMkdirBackend::CoreFs(driver),
                    parent_path,
                    name,
                }))
            }
            FsType::ExFat => {
                let driver = state
                    .mounted_exfat
                    .iter()
                    .find(|(p, _)| p == mount_path)
                    .map(|(_, d)| Arc::clone(d))
                    .ok_or(FsError::IoError)?;
                let rel = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                let (parent_path, name) = split_delete_parent_name(rel)?;
                Ok(Some(DetachedMkdir {
                    backend: DetachedMkdirBackend::ExFat(driver),
                    parent_path,
                    name,
                }))
            }
            _ => Ok(None),
        };
    }

    if state.overlay_fs.is_some() {
        return Ok(None);
    }

    let (parent_path, name) = split_delete_parent_name(path)?;
    if state.root_fs_type == Some(FsType::CoreFs) {
        return Ok(state.corefs_driver.as_ref().map(|driver| DetachedMkdir {
            backend: DetachedMkdirBackend::CoreFs(Arc::clone(driver)),
            parent_path,
            name,
        }));
    }
    if state.root_fs_type == Some(FsType::ExFat) {
        return Ok(state.exfat_fs.as_ref().map(|driver| DetachedMkdir {
            backend: DetachedMkdirBackend::ExFat(Arc::clone(driver)),
            parent_path,
            name,
        }));
    }
    Ok(None)
}

fn execute_detached_mkdir(plan: DetachedMkdir) -> Result<(), FsError> {
    match plan.backend {
        DetachedMkdirBackend::CoreFs(driver) => {
            let (parent_inode, parent_type, _) =
                Filesystem::lookup(driver.as_ref(), &plan.parent_path)?;
            if parent_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            Filesystem::create(
                driver.as_ref(),
                parent_inode,
                &plan.name,
                FileType::Directory,
            )?;
            Ok(())
        }
        DetachedMkdirBackend::ExFat(driver) => {
            let (parent_inode, parent_type, _) =
                Filesystem::lookup(driver.as_ref(), &plan.parent_path)?;
            if parent_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            Filesystem::create(
                driver.as_ref(),
                parent_inode,
                &plan.name,
                FileType::Directory,
            )?;
            Ok(())
        }
    }
}

fn execute_detached_stat(plan: &DetachedStat) -> Result<StatResult, FsError> {
    match &plan.backend {
        DetachedStatBackend::CoreFs(driver) => Filesystem::stat(driver.as_ref(), &plan.path),
        DetachedStatBackend::ExFat(driver) => {
            let fs = driver.lock_inner();
            let entry = resolve_exfat_path(&fs, &plan.path, plan.follow_last)?;
            Ok(StatResult {
                file_type: entry.file_type,
                size: entry.size,
                is_symlink: entry.is_symlink,
                uid: entry.uid,
                gid: entry.gid,
                mode: entry.mode,
                mtime: entry.mtime,
            })
        }
    }
}

fn prepare_detached_stat(path: &str, follow_last: bool) -> Option<DetachedStat> {
    if is_dev_path(path) {
        return None;
    }
    let vfs = vfs_lock();
    let state = vfs.as_ref()?;

    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        let q = if relative_path.is_empty() {
            String::from("/")
        } else {
            String::from(relative_path)
        };
        return match mnt_fs_type {
            FsType::CoreFs => state
                .mounted_corefs
                .iter()
                .find(|(p, _)| p == mount_path)
                .map(|(_, driver)| DetachedStat {
                    backend: DetachedStatBackend::CoreFs(Arc::clone(driver)),
                    path: q,
                    follow_last,
                }),
            FsType::ExFat => {
                let mount_path_owned = String::from(mount_path);
                state
                    .mounted_exfat
                    .iter()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, driver)| DetachedStat {
                        backend: DetachedStatBackend::ExFat(Arc::clone(driver)),
                        path: q,
                        follow_last,
                    })
            }
            _ => None,
        };
    }

    let q = if path.is_empty() { "/" } else { path };
    root_stat_backend(state).map(|backend| DetachedStat {
        backend,
        path: String::from(q),
        follow_last,
    })
}

fn prepare_detached_readlink(path: &str) -> Option<DetachedReadlink> {
    if is_dev_path(path) {
        return None;
    }
    let vfs = vfs_lock();
    let state = vfs.as_ref()?;

    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        let q = if relative_path.is_empty() {
            String::from("/")
        } else {
            String::from(relative_path)
        };
        return match mnt_fs_type {
            FsType::CoreFs => state
                .mounted_corefs
                .iter()
                .find(|(p, _)| p == mount_path)
                .map(|(_, driver)| DetachedReadlink {
                    backend: DetachedReadlinkBackend::CoreFs(Arc::clone(driver)),
                    path: q,
                }),
            FsType::ExFat => {
                let mount_path_owned = String::from(mount_path);
                state
                    .mounted_exfat
                    .iter()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, driver)| DetachedReadlink {
                        backend: DetachedReadlinkBackend::ExFat(Arc::clone(driver)),
                        path: q,
                    })
            }
            _ => None,
        };
    }

    if state.root_fs_type == Some(FsType::CoreFs) {
        return state.corefs_driver.as_ref().map(|driver| DetachedReadlink {
            backend: DetachedReadlinkBackend::CoreFs(Arc::clone(driver)),
            path: String::from(if path.is_empty() { "/" } else { path }),
        });
    }
    if state.root_fs_type == Some(FsType::ExFat) {
        return state.exfat_fs.as_ref().map(|driver| DetachedReadlink {
            backend: DetachedReadlinkBackend::ExFat(Arc::clone(driver)),
            path: String::from(if path.is_empty() { "/" } else { path }),
        });
    }
    None
}

fn execute_detached_readlink(plan: DetachedReadlink) -> Result<String, FsError> {
    match plan.backend {
        DetachedReadlinkBackend::CoreFs(driver) => {
            let st = Filesystem::stat(driver.as_ref(), &plan.path)?;
            if !st.is_symlink {
                return Err(FsError::InvalidPath);
            }
            let (inode, _, _) = Filesystem::lookup(driver.as_ref(), &plan.path)?;
            Filesystem::readlink(driver.as_ref(), inode)
        }
        DetachedReadlinkBackend::ExFat(driver) => {
            let fs = driver.lock_inner();
            let r = resolve_exfat_path(&fs, &plan.path, false)?;
            if !r.is_symlink {
                return Err(FsError::InvalidPath);
            }
            fs.readlink(r.inode, r.size)
        }
    }
}

impl VfsState {
    /// Returns the active root filesystem as a generic
    /// [`Filesystem`] reference, or `None` if no root has been
    /// mounted.  Used by the generic dispatch paths that don't need
    /// driver-specific APIs.
    fn root_fs(&self) -> Option<&(dyn Filesystem + Send + Sync)> {
        if let Some(d) = self.exfat_fs.as_ref() {
            return Some(d.as_ref());
        }
        if let Some(d) = self.fat_fs.as_ref() {
            return Some(d);
        }
        if let Some(d) = self.ntfs_fs.as_ref() {
            return Some(d);
        }
        if let Some(d) = self.corefs_driver.as_ref() {
            return Some(d.as_ref());
        }
        self.root_other.as_deref()
    }

    /// Returns whether any root filesystem is currently mounted.
    fn has_root(&self) -> bool {
        self.exfat_fs.is_some()
            || self.fat_fs.is_some()
            || self.ntfs_fs.is_some()
            || self.corefs_driver.is_some()
            || self.root_other.is_some()
    }

    /// Find the CoreFS driver serving `mount_path`. `mount_path` must be
    /// the *exact* mount point string (e.g. "/", "/mnt/corefs", "/System").
    /// Returns `None` if no CoreFS is mounted at that path.
    fn corefs_for_mount(&self, mount_path: &str) -> Option<&crate::fs::corefs::CoreFsDriver> {
        if mount_path == "/" {
            if self.root_fs_type == Some(FsType::CoreFs) {
                return self.corefs_driver.as_deref();
            }
            return None;
        }
        self.mounted_corefs
            .iter()
            .find(|(p, _)| p == mount_path)
            .map(|(_, d)| d.as_ref())
    }

    /// Find the CoreFS driver that serves the absolute file path `path`.
    /// Uses sub-mount dispatch when applicable and otherwise falls back to
    /// the CoreFS root driver. Returns `None` if no CoreFS driver owns the
    /// path (including when the root is not CoreFS and the path lives on
    /// another filesystem).
    fn corefs_for_path(&self, path: &str) -> Option<&crate::fs::corefs::CoreFsDriver> {
        if let Some((mp, _rel, t)) = path::find_submount(path, &self.mount_points) {
            if t == FsType::CoreFs {
                return self
                    .mounted_corefs
                    .iter()
                    .find(|(p, _)| p == mp)
                    .map(|(_, d)| d.as_ref());
            }
            return None;
        }
        if self.root_fs_type == Some(FsType::CoreFs) {
            self.corefs_driver.as_deref()
        } else {
            None
        }
    }

    fn corefs_handle_for_path(&self, path: &str) -> Option<Arc<crate::fs::corefs::CoreFsDriver>> {
        if let Some((mp, _rel, t)) = path::find_submount(path, &self.mount_points) {
            if t == FsType::CoreFs {
                return self
                    .mounted_corefs
                    .iter()
                    .find(|(p, _)| p == mp)
                    .map(|(_, d)| Arc::clone(d));
            }
            return None;
        }
        if self.root_fs_type == Some(FsType::CoreFs) {
            self.corefs_driver.as_ref().map(Arc::clone)
        } else {
            None
        }
    }

    fn mounted_exfat_handle_for_path(&self, path: &str) -> Option<Arc<ExFatFsDriver>> {
        self.mounted_exfat
            .iter()
            .find(|(p, _)| path.starts_with(p.as_str()))
            .map(|(_, d)| Arc::clone(d))
    }

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
    let mut vfs = vfs_lock();
    *vfs = Some(VfsState {
        open_files: Vec::new(),
        mount_points: Vec::new(),
        iso9660_fs: None,
        devfs: None,
        smbfs: Vec::new(),
        mounted_exfat: Vec::new(),
        mounted_corefs: Vec::new(),
        overlay_fs: None,
        corefs_driver: None,
        exfat_fs: None,
        fat_fs: None,
        ntfs_fs: None,
        root_other: None,
        root_fs_type: None,
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
    let vfs = vfs_lock();
    vfs.as_ref().map(|s| s.has_root()).unwrap_or(false)
}

// ───────────────────────────────────────────────────────────────────────
// Generic root-FS probe registry (Phase 6).
// ───────────────────────────────────────────────────────────────────────
//
// Adding a new filesystem driver is now a two-step affair:
//
//   1. Implement the `Filesystem` trait (see `kernel/src/fs/vfs/types.rs`).
//   2. Provide a `try_mount_root(device_id, partition_lba, sectors) ->
//      Option<Box<dyn Filesystem + Send + Sync>>` function that probes
//      the volume's on-disk magic and constructs the driver on success.
//
// Add an entry to `ROOT_FS_PROBES` and the boot-time `mount("/")` will
// pick the new FS up automatically.  No further VFS changes required.

struct RootFsProbe {
    /// Human-readable name for diagnostics ("exFAT", "CoreFS", …).
    name: &'static str,
    /// FsType to record in the resulting `MountPoint` (drives later
    /// dispatch decisions in `find_submount` etc.).
    fs_type: FsType,
    /// Probe + mount.  Returns `true` on success (and has populated
    /// the matching typed field on `state`); `false` means
    /// "not this FS — skip to next probe".
    try_mount: fn(
        state: &mut VfsState,
        device_id: u32,
        partition_lba: u32,
        partition_sectors: u64,
    ) -> bool,
}

/// Registry of root-mount-capable filesystems, ordered by uniqueness
/// of magic so the most-specific format wins.  CoreFS first
/// (4-byte ODF magic at LBA 1), then exFAT (8-byte OEM "EXFAT   "),
/// NTFS ("NTFS    "), and finally FAT (BPB heuristics — least
/// strict, must come last).
const ROOT_FS_PROBES: &[RootFsProbe] = &[
    RootFsProbe {
        name: "CoreFS",
        fs_type: FsType::CoreFs,
        try_mount: probe_mount_corefs,
    },
    RootFsProbe {
        name: "exFAT",
        fs_type: FsType::ExFat,
        try_mount: probe_mount_exfat,
    },
    RootFsProbe {
        name: "NTFS",
        fs_type: FsType::Ntfs,
        try_mount: probe_mount_ntfs,
    },
    RootFsProbe {
        name: "FAT",
        fs_type: FsType::Fat,
        try_mount: probe_mount_fat,
    },
];

fn probe_mount_corefs(state: &mut VfsState, dev: u32, lba: u32, sectors: u64) -> bool {
    match crate::fs::corefs::try_mount_root_typed(dev, lba, sectors) {
        Some(driver) => {
            state.corefs_driver = Some(Arc::new(driver));
            true
        }
        None => false,
    }
}

fn probe_mount_exfat(state: &mut VfsState, dev: u32, lba: u32, sectors: u64) -> bool {
    match crate::fs::exfat::try_mount_root_typed(dev, lba, sectors) {
        Some(driver) => {
            state.exfat_fs = Some(Arc::new(driver));
            true
        }
        None => false,
    }
}

fn probe_mount_ntfs(state: &mut VfsState, dev: u32, lba: u32, sectors: u64) -> bool {
    match crate::fs::ntfs::try_mount_root_typed(dev, lba, sectors) {
        Some(driver) => {
            state.ntfs_fs = Some(driver);
            true
        }
        None => false,
    }
}

fn probe_mount_fat(state: &mut VfsState, dev: u32, lba: u32, sectors: u64) -> bool {
    match crate::fs::fat::try_mount_root_typed(dev, lba, sectors) {
        Some(driver) => {
            state.fat_fs = Some(driver);
            true
        }
        None => false,
    }
}

/// Mount a filesystem at the given path.
///
/// For root mounts (`path == "/"`), iterates [`ROOT_FS_PROBES`] until
/// one succeeds and stores the resulting driver in `state.root_fs`.
/// All future root-relative VFS calls dispatch through that
/// trait object.
///
/// For sub-mounts (Iso 9660 etc.) the existing per-FS instance fields
/// are used.
pub fn mount(path: &str, fs_type: FsType, device_id: u32) {
    crate::debug_println!(
        "  [VFS] mount: path='{}' fs_type={:?} device_id={}",
        path,
        fs_type,
        device_id
    );
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().expect("VFS not initialized");

    let actual_type =
        if fs_type == FsType::Fat || fs_type == FsType::ExFat || fs_type == FsType::CoreFs {
            // Generic root-FS probe loop.  Whichever probe successfully
            // constructs a driver wins — the input fs_type parameter is
            // effectively a hint and we honour what's actually on disk.
            let lba = root_partition_lba();
            let sectors = root_blockdev_id()
                .and_then(crate::drivers::storage::blockdev::get_device)
                .map(|d| d.size_sectors)
                .unwrap_or(0);
            if sectors > 0 {
                crate::fs::blockcache::set_write_range(device_id as u8, lba as u64, sectors);
            }

            let mut chosen: FsType = fs_type;
            let mut mounted = false;
            for probe in ROOT_FS_PROBES {
                if (probe.try_mount)(state, device_id, lba, sectors) {
                    state.root_fs_type = Some(probe.fs_type);
                    chosen = probe.fs_type;
                    mounted = true;
                    crate::serial_println!(
                        "  Mounted {} at '{}' (LBA {}, device {})",
                        probe.name,
                        path,
                        lba,
                        device_id
                    );
                    break;
                }
            }
            if !mounted {
                crate::serial_println!(
                    "  No FS driver matched the root partition at LBA {} (device {})",
                    lba,
                    device_id
                );
            }
            chosen
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
    let mut vfs = vfs_lock();
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
        /* read_only = */ false,
    )
    .map_err(|e| crate::fs::corefs::corefs_to_fs_error(&e))?;
    let driver = crate::fs::corefs::CoreFsDriver::mount_writable(adapter)?;
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    if path == "/" {
        // Root mount — populated by the generic `probe_mount_corefs` path
        // during boot, or explicitly via root-level dispatch.
        if state.corefs_driver.is_some() {
            return Err(FsError::AlreadyExists);
        }
        state.corefs_driver = Some(Arc::new(driver));
    } else {
        // Sub-mount — each partition lives in its own driver slot so that
        // statfs / read / write dispatch per mount path.
        if state.mounted_corefs.iter().any(|(p, _)| p == path)
            || state.mount_points.iter().any(|mp| mp.path == path)
        {
            return Err(FsError::AlreadyExists);
        }
        state
            .mounted_corefs
            .push((String::from(path), Arc::new(driver)));
    }
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

/// Mount a FUSE-Session als VFS-Dateisystem.
///
/// Die eigentliche Dateisystem-Logik lebt im Userspace-Daemon, der über die
/// [`crate::fs::fuse::FuseSession`] Requests empfängt. `session_id` identifiziert
/// die bereits via [`crate::fs::fuse::register_session`] registrierte Session.
///
/// `mount_path` ist der VFS-Pfad, unter dem der Mount erscheinen soll
/// (z. B. `/mnt/fuse`). Die Funktion legt eine eigene [`crate::fs::fuse::inode_map::InodeMap`]
/// pro Mount an; Root-Inode (VFS-`u32 = 1`) mappt auf FUSE-`u64 = 1`.
pub fn mount_fuse(mount_path: &str, session_id: u32) -> Result<(), FsError> {
    // Session muss existieren, sonst kein sinnvoller Mount.
    if crate::fs::fuse::session(session_id).is_none() {
        return Err(FsError::NotFound);
    }
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    // Duplikatprüfung.
    for mp in &state.mount_points {
        if mp.path == mount_path {
            return Err(FsError::AlreadyExists);
        }
    }
    crate::fs::fuse::inode_map::ensure_mount(session_id);
    state.mount_points.push(MountPoint {
        path: String::from(mount_path),
        fs_type: FsType::Fuse,
        device_id: session_id,
    });
    crate::serial_verbose_println!("  Mounted FUSE session {} at '{}'", session_id, mount_path);
    Ok(())
}

/// Flush the currently-mounted CoreFS driver to disk (if any).
///
/// Intended as a shutdown / sync hook — persists any pending mutations
/// collected by the in-memory [`crate::fs::corefs::CoreFsDriver`] via
/// `save_state_native`. On read-only mounts this is a no-op.
///
/// Returns `Ok(false)` when no CoreFS volume is mounted.
pub fn sync_corefs() -> Result<bool, FsError> {
    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;
    let mut any = false;
    if let Some(driver) = state.corefs_driver.as_ref() {
        driver.flush()?;
        any = true;
    }
    for (_, driver) in &state.mounted_corefs {
        driver.flush()?;
        any = true;
    }
    Ok(any)
}

/// Mount the device filesystem at /dev, bridging built-in virtual devices
/// with HAL-registered hardware devices.
pub fn mount_devfs() {
    let mut vfs = vfs_lock();
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
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().expect("VFS not initialized");
    if state.iso9660_fs.is_some() && state.overlay_fs.is_none() {
        state.overlay_fs = Some(OverlayFs::new());
        crate::serial_println!("[VFS] OverlayFS enabled (RAM overlay over ISO 9660)");
    }
}

/// Check if the overlay filesystem is active.
pub fn has_overlay() -> bool {
    let vfs = vfs_lock();
    if let Some(ref state) = *vfs {
        state.overlay_fs.is_some()
    } else {
        false
    }
}

/// Open a file by path with the given flags. Returns a file descriptor on success.
pub fn open(path: &str, flags: FileFlags) -> Result<FileDescriptor, FsError> {
    if let Some(plan) = prepare_detached_open(path, flags)? {
        let result = execute_detached_open(plan)?;
        return insert_detached_open(result);
    }

    let mut vfs = vfs_lock();
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
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
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
                if state.ntfs_fs.is_some() {
                    if flags.write || flags.create || flags.truncate || flags.append {
                        return Err(FsError::PermissionDenied);
                    }
                    let (inode, file_type, size) = {
                        let ntfs = state.ntfs_fs.as_ref().unwrap().lock_inner();
                        ntfs.lookup(relative_path)?
                    };
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
                let exfat = state
                    .mounted_exfat
                    .iter_mut()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, fs)| fs)
                    .ok_or(FsError::IoError)?;
                let mut exfat = exfat.lock_inner();
                let lookup_result = if flags.write || flags.create || flags.truncate {
                    exfat.lookup(relative_path)
                } else {
                    resolve_exfat_path(&exfat, relative_path, true)
                        .map(|r| (r.inode, r.file_type, r.size))
                };
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
                                exfat
                                    .lookup(parent_path)
                                    .map(|(i, _, _)| crate::fs::exfat::decode_inode(i).0)
                                    .unwrap_or(0)
                            } else {
                                0
                            };
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
                drop(exfat);
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
            FsType::CoreFs => {
                let driver = state
                    .corefs_for_mount(mount_path)
                    .ok_or(FsError::NotFound)?;
                let q = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                let lookup_result = Filesystem::lookup(driver, q);
                let (inode, file_type, size) = match lookup_result {
                    Ok(r) => {
                        if flags.truncate && flags.write {
                            driver.truncate_file(r.0, 0)?;
                            (r.0, r.1, 0)
                        } else {
                            r
                        }
                    }
                    Err(FsError::NotFound) if flags.create => {
                        // Split into parent + name and create via driver.
                        let rel = q.trim_end_matches('/');
                        let (parent, name) = match rel.rfind('/') {
                            Some(0) => ("/", &rel[1..]),
                            Some(pos) => (&rel[..pos], &rel[pos + 1..]),
                            None => ("/", rel),
                        };
                        let (parent_inode, parent_type, _) = Filesystem::lookup(driver, parent)?;
                        if parent_type != FileType::Directory {
                            return Err(FsError::NotADirectory);
                        }
                        let new_inode =
                            Filesystem::create(driver, parent_inode, name, FileType::Regular)?;
                        (new_inode, FileType::Regular, 0)
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
                    fs_id: 8, // CoreFS
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
            FsType::Fuse => {
                let mount_path_owned = String::from(mount_path);
                let rel_owned = String::from(relative_path);
                let path_owned = String::from(path);
                return fuse_open_entry(state, &mount_path_owned, &rel_owned, flags, &path_owned);
            }
            FsType::Smb => {
                let mount_path_owned = String::from(mount_path);
                let relative_path_owned = String::from(relative_path);
                let smb = state
                    .smbfs
                    .iter_mut()
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
    if state.exfat_fs.is_some() {
        let (inode, file_type, size, parent_cluster) = {
            let exfat_drv = state.exfat_fs.as_ref().unwrap();
            let mut exfat_guard = exfat_drv.lock_inner();
            let exfat = &mut *exfat_guard;
            // Resolve symlinks in the path before opening
            let lookup_result = resolve_exfat_path(exfat, path, true);
            match lookup_result {
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
            }
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
    if state.fat_fs.is_some() {
        let (inode, file_type, size, parent_cluster) = {
            let fat_drv = state.fat_fs.as_ref().unwrap();
            let mut fat_guard = fat_drv.lock_inner();
            let fat = &mut *fat_guard;
            let lookup_result = fat.lookup(path);
            match lookup_result {
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
            }
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
    if state.ntfs_fs.is_some() {
        if flags.write || flags.create || flags.truncate || flags.append {
            return Err(FsError::PermissionDenied);
        }
        let (inode, file_type, size) = {
            let ntfs = state.ntfs_fs.as_ref().unwrap().lock_inner();
            ntfs.lookup(path)?
        };
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

    // --- CoreFS root path (Phase 6 generic dispatch) ---
    if state.corefs_driver.is_some() {
        let driver = state.corefs_driver.as_ref().unwrap().as_ref();
        let q = if path.is_empty() { "/" } else { path };
        let (inode, file_type, size) = match Filesystem::lookup(driver, q) {
            Ok(r) => {
                if flags.truncate && flags.write {
                    driver.truncate_file(r.0, 0)?;
                    (r.0, r.1, 0)
                } else {
                    r
                }
            }
            Err(FsError::NotFound) if flags.create => {
                let rel = q.trim_end_matches('/');
                let (parent, name) = match rel.rfind('/') {
                    Some(0) => ("/", &rel[1..]),
                    Some(pos) => (&rel[..pos], &rel[pos + 1..]),
                    None => ("/", rel),
                };
                let (parent_inode, parent_type, _) = Filesystem::lookup(driver, parent)?;
                if parent_type != FileType::Directory {
                    return Err(FsError::NotADirectory);
                }
                let new_inode = Filesystem::create(driver, parent_inode, name, FileType::Regular)?;
                (new_inode, FileType::Regular, 0)
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
            fs_id: 8, // CoreFS
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
    let mut exfat_commit: Option<DetachedExFatCommit> = None;
    let mut corefs_to_flush: Option<Arc<crate::fs::corefs::CoreFsDriver>> = None;
    let mut fuse_release: Option<(Arc<crate::fs::fuse::FuseSession>, fuse_proto::Request)> = None;
    {
        let mut vfs = vfs_lock();
        let state = vfs.as_mut().ok_or(FsError::IoError)?;

        let (refcount, fs_id, was_writable) = {
            let file = state
                .open_files
                .get(slot_id as usize)
                .and_then(|e| e.as_ref())
                .ok_or(FsError::BadFd)?;
            (file.refcount, file.fs_id, file.flags.write)
        };

        if refcount > 1 {
            let file = state
                .open_files
                .get_mut(slot_id as usize)
                .and_then(|e| e.as_mut())
                .ok_or(FsError::BadFd)?;
            file.refcount -= 1;
        } else {
            if was_writable && (fs_id == 3 || fs_id == 6) {
                exfat_commit = snapshot_open_exfat_commit(state, slot_id as usize, true)?;
                do_writeback = true;
            }
            if was_writable && fs_id == 8 {
                if let Some(path) = state
                    .open_files
                    .get(slot_id as usize)
                    .and_then(|e| e.as_ref())
                    .map(|f| f.path.clone())
                {
                    if let Some(driver) = state.corefs_handle_for_path(&path) {
                        corefs_to_flush = Some(driver);
                    }
                }
            }
            if fs_id == 9 {
                // Send Release to the FUSE daemon. Failures are logged but
                // don't block slot-free — the kernel side must always
                // release the slot to avoid FD leaks.
                if let Some(Some(file)) = state.open_files.get(slot_id as usize) {
                    if let Ok((sid, session)) = fuse_session_of(file) {
                        if let Some(ino_u64) = crate::fs::fuse::inode_map::to_u64(sid, file.inode) {
                            let fh = fuse_fh_of(file);
                            fuse_release =
                                Some((session, fuse_proto::Request::Release { ino: ino_u64, fh }));
                        }
                    }
                }
            }
            state.free_slot(slot_id);
        }
    } // VFS lock released

    if let Some(commit) = exfat_commit {
        let disk_id = finish_detached_exfat_commit(&commit)?;
        queue_disk_flush(&mut disks_to_flush, disk_id);
    }
    if let Some((session, req)) = fuse_release {
        let _ = crate::fs::fuse::fuse_call(&session, &req);
    }
    // Flush write-back cache outside VFS lock (may block on disk I/O)
    if do_writeback {
        flush_blockcache_for_disks(&disks_to_flush);
    }
    if let Some(driver) = corefs_to_flush {
        let _ = driver.flush();
    }
    Ok(())
}

/// Increment the reference count on a global open file slot (for fork/dup).
pub fn incref(slot_id: u32) {
    let mut vfs = vfs_lock();
    if let Some(state) = vfs.as_mut() {
        if let Some(Some(file)) = state.open_files.get_mut(slot_id as usize) {
            file.refcount += 1;
        }
    }
}

/// Preferred userspace copy chunk for writes into this global file slot.
///
/// exFAT benefits from larger chunks because the write path can coalesce full
/// cluster runs into fewer storage commands. CoreFS deliberately stays on the
/// conservative syscall chunk size selected by `sys_write`.
pub fn preferred_write_chunk(slot_id: FileDescriptor) -> usize {
    let mut vfs = vfs_lock();
    let Some(state) = vfs.as_mut() else {
        return 16 * 1024;
    };
    match state
        .open_files
        .get(slot_id as usize)
        .and_then(|e| e.as_ref())
        .map(|f| f.fs_id)
    {
        Some(3) | Some(6) => 128 * 1024,
        _ => 16 * 1024,
    }
}

/// Decrement the reference count on a global open file slot (for close/exit).
/// Frees the slot if refcount drops to 0. On last close of a writable exFAT
/// file, flushes deferred metadata to disk.
pub fn decref(slot_id: u32) {
    let mut do_writeback = false;
    let mut disks_to_flush: Vec<u8> = Vec::new();
    let mut exfat_commit: Option<DetachedExFatCommit> = None;
    let mut corefs_to_flush: Option<Arc<crate::fs::corefs::CoreFsDriver>> = None;
    let mut fuse_release: Option<(Arc<crate::fs::fuse::FuseSession>, fuse_proto::Request)> = None;
    let mut vfs = vfs_lock();
    if let Some(state) = vfs.as_mut() {
        let snapshot = state
            .open_files
            .get(slot_id as usize)
            .and_then(|e| e.as_ref())
            .map(|file| (file.refcount, file.fs_id, file.flags.write));
        if let Some((refcount, fs_id, was_writable)) = snapshot {
            if refcount > 1 {
                if let Some(Some(file)) = state.open_files.get_mut(slot_id as usize) {
                    file.refcount -= 1;
                }
            } else {
                if was_writable && (fs_id == 3 || fs_id == 6) {
                    exfat_commit = snapshot_open_exfat_commit(state, slot_id as usize, true)
                        .ok()
                        .flatten();
                    do_writeback = true;
                }
                if was_writable && fs_id == 8 {
                    if let Some(path) = state
                        .open_files
                        .get(slot_id as usize)
                        .and_then(|e| e.as_ref())
                        .map(|f| f.path.clone())
                    {
                        if let Some(driver) = state.corefs_handle_for_path(&path) {
                            corefs_to_flush = Some(driver);
                        }
                    }
                }
                if fs_id == 9 {
                    if let Some(Some(file)) = state.open_files.get(slot_id as usize) {
                        if let Ok((sid, session)) = fuse_session_of(file) {
                            if let Some(ino_u64) =
                                crate::fs::fuse::inode_map::to_u64(sid, file.inode)
                            {
                                let fh = fuse_fh_of(file);
                                fuse_release = Some((
                                    session,
                                    fuse_proto::Request::Release { ino: ino_u64, fh },
                                ));
                            }
                        }
                    }
                }
                state.free_slot(slot_id);
            }
        }
    }
    drop(vfs);
    if let Some(commit) = exfat_commit {
        if let Ok(disk_id) = finish_detached_exfat_commit(&commit) {
            queue_disk_flush(&mut disks_to_flush, disk_id);
        }
    }
    if let Some((session, req)) = fuse_release {
        let _ = crate::fs::fuse::fuse_call(&session, &req);
    }
    if do_writeback {
        flush_blockcache_for_disks(&disks_to_flush);
    }
    if let Some(driver) = corefs_to_flush {
        let _ = driver.flush();
    }
}

fn prepare_detached_read(slot_id: FileDescriptor, len: usize) -> Result<DetachedReadPrep, FsError> {
    if len == 0 {
        return Ok(DetachedReadPrep::Eof);
    }

    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    let (fs_id, path, position, size, inode) = {
        let file = state
            .open_files
            .get(slot_id as usize)
            .and_then(|e| e.as_ref())
            .ok_or(FsError::BadFd)?;
        (
            file.fs_id,
            file.path.clone(),
            file.position,
            file.size,
            file.inode,
        )
    };

    let backend = match fs_id {
        3 => {
            let driver = state
                .exfat_fs
                .as_ref()
                .map(Arc::clone)
                .ok_or(FsError::IoError)?;
            DetachedReadBackend::ExFat(driver)
        }
        6 => {
            let driver = state
                .mounted_exfat_handle_for_path(&path)
                .ok_or(FsError::IoError)?;
            DetachedReadBackend::ExFat(driver)
        }
        8 => {
            let driver = state
                .corefs_handle_for_path(&path)
                .ok_or(FsError::IoError)?;
            DetachedReadBackend::CoreFs(driver)
        }
        _ => return Ok(DetachedReadPrep::Unsupported),
    };

    if position >= size {
        return Ok(DetachedReadPrep::Eof);
    }

    let remaining = (size - position) as usize;
    let reserved = len.min(remaining);
    let file = state
        .open_files
        .get_mut(slot_id as usize)
        .and_then(|e| e.as_mut())
        .ok_or(FsError::BadFd)?;
    if file.fs_id != fs_id || file.inode != inode || file.path != path || file.position != position
    {
        return Err(FsError::BadFd);
    }
    file.position = position.saturating_add(reserved as u32);

    Ok(DetachedReadPrep::Ready(DetachedRead {
        backend,
        inode,
        position,
        size,
        reserved,
        path,
    }))
}

fn prepare_detached_read_at(
    slot_id: FileDescriptor,
    offset: u32,
    len: usize,
) -> Result<DetachedReadPrep, FsError> {
    if len == 0 {
        return Ok(DetachedReadPrep::Eof);
    }

    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;
    let (fs_id, path, size, inode) = {
        let file = state
            .open_files
            .get(slot_id as usize)
            .and_then(|e| e.as_ref())
            .ok_or(FsError::BadFd)?;
        (file.fs_id, file.path.clone(), file.size, file.inode)
    };

    let backend = match fs_id {
        3 => {
            let driver = state
                .exfat_fs
                .as_ref()
                .map(Arc::clone)
                .ok_or(FsError::IoError)?;
            DetachedReadBackend::ExFat(driver)
        }
        6 => {
            let driver = state
                .mounted_exfat_handle_for_path(&path)
                .ok_or(FsError::IoError)?;
            DetachedReadBackend::ExFat(driver)
        }
        8 => {
            let driver = state
                .corefs_handle_for_path(&path)
                .ok_or(FsError::IoError)?;
            DetachedReadBackend::CoreFs(driver)
        }
        _ => return Ok(DetachedReadPrep::Unsupported),
    };

    if offset >= size {
        return Ok(DetachedReadPrep::Eof);
    }

    Ok(DetachedReadPrep::Ready(DetachedRead {
        backend,
        inode,
        position: offset,
        size,
        reserved: len.min((size - offset) as usize),
        path,
    }))
}

fn finish_detached_read(
    slot_id: FileDescriptor,
    plan: &DetachedRead,
    result: Result<usize, FsError>,
) -> Result<usize, FsError> {
    let bytes_read = match result {
        Ok(n) => n,
        Err(err) => {
            rollback_detached_read(slot_id, plan, plan.reserved);
            return Err(err);
        }
    };

    let unread = plan.reserved.saturating_sub(bytes_read);
    if unread > 0 && detached_read_file_changed(slot_id, plan) {
        rollback_detached_read(slot_id, plan, unread);
        return Err(FsError::IoError);
    }
    if unread == 0 {
        return Ok(bytes_read);
    }
    rollback_detached_read(slot_id, plan, unread);
    Ok(bytes_read)
}

fn execute_detached_read(plan: &DetachedRead, buf: &mut [u8]) -> Result<usize, FsError> {
    let mut total = 0usize;
    while total < plan.reserved {
        let offset = plan.position.saturating_add(total as u32);
        let chunk = &mut buf[total..plan.reserved];
        let n = match &plan.backend {
            DetachedReadBackend::CoreFs(driver) => {
                Filesystem::read(driver.as_ref(), plan.inode, offset, chunk)
            }
            DetachedReadBackend::ExFat(driver) => {
                Filesystem::read(driver.as_ref(), plan.inode, offset, chunk)
            }
        }?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n.min(chunk.len()));
    }
    Ok(total)
}

fn rollback_detached_read(slot_id: FileDescriptor, plan: &DetachedRead, amount: usize) {
    if amount == 0 {
        return;
    }

    let mut vfs = vfs_lock();
    let Some(state) = vfs.as_mut() else {
        return;
    };
    let Some(Some(file)) = state.open_files.get_mut(slot_id as usize) else {
        return;
    };
    if file.inode != plan.inode || file.path != plan.path {
        return;
    }
    file.position = file.position.saturating_sub(amount as u32);
}

fn detached_read_file_changed(slot_id: FileDescriptor, plan: &DetachedRead) -> bool {
    let vfs = vfs_lock();
    let Some(state) = vfs.as_ref() else {
        return true;
    };
    let Some(Some(file)) = state.open_files.get(slot_id as usize) else {
        return true;
    };
    file.inode != plan.inode || file.path != plan.path || file.size != plan.size
}

fn prepare_detached_write(
    slot_id: FileDescriptor,
    len: usize,
) -> Result<DetachedWritePrep, FsError> {
    if len == 0 {
        return Ok(DetachedWritePrep::Empty);
    }

    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;
    let (
        fs_id,
        path,
        inode,
        size,
        position,
        parent_cluster,
        seek_cache_offset,
        seek_cache_cluster,
        sync_write,
    ) = {
        let file = state
            .open_files
            .get(slot_id as usize)
            .and_then(|e| e.as_ref())
            .ok_or(FsError::BadFd)?;
        if !file.flags.write {
            return Err(FsError::PermissionDenied);
        }
        (
            file.fs_id,
            file.path.clone(),
            file.inode,
            file.size,
            file.position,
            file.parent_cluster,
            file.seek_cache_offset,
            file.seek_cache_cluster,
            file.flags.sync,
        )
    };

    let backend = match fs_id {
        3 => {
            let driver = state
                .exfat_fs
                .as_ref()
                .map(Arc::clone)
                .ok_or(FsError::IoError)?;
            DetachedWriteBackend::ExFat(driver)
        }
        6 => {
            let driver = state
                .mounted_exfat_handle_for_path(&path)
                .ok_or(FsError::IoError)?;
            DetachedWriteBackend::ExFat(driver)
        }
        8 => {
            let driver = state
                .corefs_handle_for_path(&path)
                .ok_or(FsError::IoError)?;
            DetachedWriteBackend::CoreFs(driver)
        }
        _ => return Ok(DetachedWritePrep::Unsupported),
    };

    let file = state
        .open_files
        .get_mut(slot_id as usize)
        .and_then(|e| e.as_mut())
        .ok_or(FsError::BadFd)?;
    if file.fs_id != fs_id || file.inode != inode || file.path != path || file.position != position
    {
        return Err(FsError::BadFd);
    }
    file.position = position.saturating_add(len as u32);

    let seek_hint = if seek_cache_cluster >= 2 && seek_cache_offset <= position {
        Some((seek_cache_offset, seek_cache_cluster))
    } else {
        None
    };

    Ok(DetachedWritePrep::Ready(DetachedWrite {
        backend,
        fs_id,
        inode,
        old_inode: inode,
        old_size: size,
        position,
        reserved: len,
        path,
        parent_cluster,
        seek_hint,
        sync_write,
    }))
}

fn execute_detached_write(
    plan: &DetachedWrite,
    buf: &[u8],
) -> Result<DetachedWriteResult, FsError> {
    match &plan.backend {
        DetachedWriteBackend::CoreFs(driver) => {
            let bytes_written = Filesystem::write(driver.as_ref(), plan.inode, plan.position, buf)?;
            Ok(DetachedWriteResult::CoreFs { bytes_written })
        }
        DetachedWriteBackend::ExFat(driver) => {
            let mut exfat = driver.lock_inner();
            let (new_cluster, new_size, hint_offset, hint_cluster) = exfat.write_file_with_hint(
                plan.old_inode,
                plan.position,
                buf,
                plan.old_size,
                plan.seek_hint,
            )?;
            Ok(DetachedWriteResult::ExFat(DetachedExFatWriteResult {
                new_cluster,
                new_size,
                hint_offset,
                hint_cluster,
            }))
        }
    }
}

fn finish_detached_write(
    slot_id: FileDescriptor,
    plan: &DetachedWrite,
    result: Result<DetachedWriteResult, FsError>,
) -> Result<usize, FsError> {
    let result = match result {
        Ok(result) => result,
        Err(err) => {
            rollback_detached_write(slot_id, plan, plan.reserved);
            return Err(err);
        }
    };

    match result {
        DetachedWriteResult::CoreFs { bytes_written } => {
            let unread = plan.reserved.saturating_sub(bytes_written);
            if unread > 0 {
                rollback_detached_write(slot_id, plan, unread);
            }
            {
                let mut vfs = vfs_lock();
                let state = vfs.as_mut().ok_or(FsError::IoError)?;
                let file = state
                    .open_files
                    .get_mut(slot_id as usize)
                    .and_then(|e| e.as_mut())
                    .ok_or(FsError::BadFd)?;
                if file.fs_id != plan.fs_id || file.inode != plan.inode || file.path != plan.path {
                    return Err(FsError::BadFd);
                }
                let written_end = plan.position.saturating_add(bytes_written as u32);
                file.size = core::cmp::max(file.size, core::cmp::max(plan.old_size, written_end));
            }
            if plan.sync_write {
                if let DetachedWriteBackend::CoreFs(driver) = &plan.backend {
                    driver.flush()?;
                }
            }
            Ok(bytes_written)
        }
        DetachedWriteResult::ExFat(exfat_result) => {
            finish_detached_exfat_write(slot_id, plan, exfat_result)
        }
    }
}

fn finish_detached_exfat_write(
    slot_id: FileDescriptor,
    plan: &DetachedWrite,
    result: DetachedExFatWriteResult,
) -> Result<usize, FsError> {
    let mut disks_to_flush = Vec::new();
    {
        let mut vfs = vfs_lock();
        let state = vfs.as_mut().ok_or(FsError::IoError)?;
        let file = state
            .open_files
            .get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        if file.fs_id != plan.fs_id || file.path != plan.path {
            return Err(FsError::BadFd);
        }
        file.inode = result.new_cluster;
        file.size = result.new_size;
        file.seek_cache_offset = result.hint_offset;
        file.seek_cache_cluster = result.hint_cluster;
        if result.new_cluster != plan.old_inode || result.new_size != plan.old_size {
            file.entry_dirty = true;
        }
        if plan.sync_write {
            if let Some(disk_id) = commit_open_exfat_entry(state, slot_id as usize, true)? {
                queue_disk_flush(&mut disks_to_flush, disk_id);
            }
        }
    }

    if plan.sync_write {
        flush_blockcache_for_disks(&disks_to_flush);
        flush_hardware_for_disks(&disks_to_flush);
    } else {
        maybe_flush_exfat_metadata_periodic();
    }
    Ok(plan.reserved)
}

fn rollback_detached_write(slot_id: FileDescriptor, plan: &DetachedWrite, amount: usize) {
    if amount == 0 {
        return;
    }

    let mut vfs = vfs_lock();
    let Some(state) = vfs.as_mut() else {
        return;
    };
    let Some(Some(file)) = state.open_files.get_mut(slot_id as usize) else {
        return;
    };
    if file.fs_id != plan.fs_id || file.path != plan.path {
        return;
    }
    file.position = file.position.saturating_sub(amount as u32);
}

fn maybe_flush_exfat_metadata_periodic() {
    let wc = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    if wc % FLUSH_INTERVAL != FLUSH_INTERVAL - 1 {
        return;
    }

    let mut vfs = vfs_lock();
    let Some(state) = vfs.as_mut() else {
        return;
    };
    if let Some(exfat_drv) = state.exfat_fs.as_ref() {
        let mut exfat_guard = exfat_drv.lock_inner();
        let exfat = &mut *exfat_guard;
        if exfat.metadata_dirty {
            let _ = exfat.flush_metadata();
        }
    }
    for (_path, exfat) in &mut state.mounted_exfat {
        let mut exfat = exfat.lock_inner();
        if exfat.metadata_dirty {
            let _ = exfat.flush_metadata();
        }
    }
}

/// Read bytes from an open file into `buf`. `slot_id` is the global open_files index.
/// Returns the number of bytes read (0 at EOF).
pub fn read(slot_id: FileDescriptor, buf: &mut [u8]) -> Result<usize, FsError> {
    match prepare_detached_read(slot_id, buf.len())? {
        DetachedReadPrep::Eof => return Ok(0),
        DetachedReadPrep::Ready(plan) => {
            let result = execute_detached_read(&plan, buf);
            return finish_detached_read(slot_id, &plan, result);
        }
        DetachedReadPrep::Unsupported => {}
    }

    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Direct index lookup
    let file = state
        .open_files
        .get_mut(slot_id as usize)
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
        let exfat = state
            .mounted_exfat
            .iter()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, fs)| fs)
            .ok_or(FsError::IoError)?;
        let exfat = exfat.lock_inner();
        let bytes_read = exfat.read_file(file_inode, file_position, &mut buf[..to_read])?;
        let file = state
            .open_files
            .get_mut(slot_id as usize)
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
        let bytes_read = overlay.read_file(
            iso,
            file_inode,
            file_position,
            &mut buf[..to_read],
            file_size,
        )?;
        let file = state
            .open_files
            .get_mut(slot_id as usize)
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
        let ntfs_drv = state.ntfs_fs.as_ref().ok_or(FsError::IoError)?;
        let ntfs_guard = ntfs_drv.lock_inner();
        let ntfs = &*ntfs_guard;
        let bytes_read = ntfs.read_file(file.inode, file.position, &mut buf[..to_read])?;
        file.position += bytes_read as u32;
        return Ok(bytes_read);
    }

    // --- CoreFS file ---
    if file.fs_id == 8 {
        if file.position >= file.size {
            return Ok(0);
        }
        let remaining = (file.size - file.position) as usize;
        let to_read = buf.len().min(remaining);
        let file_inode = file.inode;
        let file_position = file.position;
        let file_path = file.path.clone();
        let driver = state.corefs_for_path(&file_path).ok_or(FsError::IoError)?;
        let bytes_read = Filesystem::read(driver, file_inode, file_position, &mut buf[..to_read])?;
        let file = state
            .open_files
            .get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.position += bytes_read as u32;
        return Ok(bytes_read);
    }

    // --- FUSE file ---
    if file.fs_id == 9 {
        if file.position >= file.size {
            return Ok(0);
        }
        let remaining = (file.size - file.position) as usize;
        let to_read = buf.len().min(remaining);
        let (sid, session) = fuse_session_of(file)?;
        let inode_u64 =
            crate::fs::fuse::inode_map::to_u64(sid, file.inode).ok_or(FsError::IoError)?;
        let fh = fuse_fh_of(file);
        let req = fuse_proto::Request::Read {
            ino: inode_u64,
            fh,
            offset: file.position as u64,
            size: to_read as u32,
        };
        let reply = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
        let data = match reply {
            fuse_proto::Reply::Read { data } => data,
            _ => return Err(FsError::IoError),
        };
        let n = core::cmp::min(data.len(), buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        let file = state
            .open_files
            .get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.position += n as u32;
        return Ok(n);
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
        let smb = state
            .smbfs
            .iter_mut()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, s)| s)
            .ok_or(FsError::IoError)?;
        let bytes_read = smb.read_file(file_inode, file_position, &mut buf[..to_read])?;
        // Re-borrow file after mutable smb use
        let file = state
            .open_files
            .get_mut(slot_id as usize)
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
        let exfat_drv = state.exfat_fs.as_ref().ok_or(FsError::IoError)?;
        let exfat_guard = exfat_drv.lock_inner();
        let exfat = &*exfat_guard;
        exfat.read_file(file.inode, file.position, &mut buf[..to_read])?
    } else if let Some(fat_drv) = state.fat_fs.as_ref() {
        let fat_guard = fat_drv.lock_inner();
        let fat = &*fat_guard;
        fat.read_file(file.inode, file.position, &mut buf[..to_read])?
    } else {
        return Err(FsError::IoError);
    };

    file.position += bytes_read as u32;
    Ok(bytes_read)
}

/// Read bytes from an open file at a fixed offset without changing its seek position.
pub fn read_at(slot_id: FileDescriptor, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
    match prepare_detached_read_at(slot_id, offset, buf.len())? {
        DetachedReadPrep::Eof => Ok(0),
        DetachedReadPrep::Ready(plan) => execute_detached_read(&plan, buf),
        DetachedReadPrep::Unsupported => Err(FsError::NotSupported),
    }
}

/// Write bytes from `buf` to an open file. `slot_id` is the global open_files index.
/// Returns the number of bytes written.
pub fn write(slot_id: FileDescriptor, buf: &[u8]) -> Result<usize, FsError> {
    match prepare_detached_write(slot_id, buf.len())? {
        DetachedWritePrep::Empty => return Ok(0),
        DetachedWritePrep::Ready(plan) => {
            let result = execute_detached_write(&plan, buf);
            return finish_detached_write(slot_id, &plan, result);
        }
        DetachedWritePrep::Unsupported => {}
    }

    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Direct index lookup
    let file = state
        .open_files
        .get_mut(slot_id as usize)
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
        let (new_inode, new_size) =
            overlay.write_file(iso, old_inode, position, buf, old_size, &file_path)?;
        let file = state
            .open_files
            .get_mut(slot_id as usize)
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
        let exfat = state
            .mounted_exfat
            .iter_mut()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, fs)| fs)
            .ok_or(FsError::IoError)?;
        let mut exfat = exfat.lock_inner();
        let (new_cluster, new_size, hint_offset, hint_cluster) =
            exfat.write_file_with_hint(old_inode, position, buf, old_size, hint)?;
        let file = state
            .open_files
            .get_mut(slot_id as usize)
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
        drop(exfat);
        if sync_write {
            let mut disks_to_flush = Vec::new();
            if let Some(disk_id) = commit_open_exfat_entry(state, slot_id as usize, true)? {
                queue_disk_flush(&mut disks_to_flush, disk_id);
            }
            drop(vfs);
            flush_blockcache_for_disks(&disks_to_flush);
            flush_hardware_for_disks(&disks_to_flush);
            return Ok(buf.len());
        }
        return Ok(buf.len());
    }

    // --- CoreFS file ---
    if file.fs_id == 8 {
        let file_inode = file.inode;
        let file_position = file.position;
        let sync_write = file.flags.sync;
        let file_path = file.path.clone();
        let driver = state.corefs_for_path(&file_path).ok_or(FsError::IoError)?;
        let bytes_written = Filesystem::write(driver, file_inode, file_position, buf)?;
        // Re-query the driver for the updated size — the write may have
        // extended the file beyond its previous end.
        let new_size = {
            let mount_rel = find_submount(&file_path, &state.mount_points)
                .map(|(_, rel, _)| String::from(rel))
                .unwrap_or_else(|| String::from("/"));
            let driver = state.corefs_for_path(&file_path).ok_or(FsError::IoError)?;
            let prev_size = state
                .open_files
                .get(slot_id as usize)
                .and_then(|e| e.as_ref())
                .map(|f| f.size)
                .unwrap_or(0);
            Filesystem::lookup(driver, &mount_rel)
                .map(|(_, _, s)| s)
                .unwrap_or(prev_size)
        };
        let file = state
            .open_files
            .get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.position += bytes_written as u32;
        file.size = core::cmp::max(new_size, file.position);
        if sync_write {
            let driver = state.corefs_for_path(&file_path).ok_or(FsError::IoError)?;
            driver.flush()?;
        }
        return Ok(bytes_written);
    }

    // --- FUSE file ---
    if file.fs_id == 9 {
        let (sid, session) = fuse_session_of(file)?;
        let inode_u64 =
            crate::fs::fuse::inode_map::to_u64(sid, file.inode).ok_or(FsError::IoError)?;
        let fh = fuse_fh_of(file);
        let req = fuse_proto::Request::Write {
            ino: inode_u64,
            fh,
            offset: file.position as u64,
            data: buf.to_vec(),
        };
        let reply = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
        let written = match reply {
            fuse_proto::Reply::Write { written } => written as usize,
            _ => return Err(FsError::IoError),
        };
        let file = state
            .open_files
            .get_mut(slot_id as usize)
            .and_then(|e| e.as_mut())
            .ok_or(FsError::BadFd)?;
        file.position += written as u32;
        if file.position > file.size {
            file.size = file.position;
        }
        return Ok(written);
    }

    // --- SMB file (network) ---
    if file.fs_id == 5 {
        let file_inode = file.inode;
        let file_position = file.position;
        let file_path = file.path.clone();
        let smb = state
            .smbfs
            .iter_mut()
            .find(|(p, _)| file_path.starts_with(p.as_str()))
            .map(|(_, s)| s)
            .ok_or(FsError::IoError)?;
        let bytes_written = smb.write_file(file_inode, file_position, buf)?;
        // Re-borrow file after mutable smb use
        let file = state
            .open_files
            .get_mut(slot_id as usize)
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
        let (new_cluster, new_size, hint_offset, hint_cluster) = {
            let exfat_drv = state.exfat_fs.as_ref().ok_or(FsError::IoError)?;
            let mut exfat = exfat_drv.lock_inner();
            exfat.write_file_with_hint(old_inode, position, buf, old_size, hint)?
        };
        let file = state
            .open_files
            .get_mut(slot_id as usize)
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
            flush_hardware_for_disks(&disks_to_flush);
            return Ok(buf.len());
        }
    } else {
        let fat_drv = state.fat_fs.as_ref().ok_or(FsError::IoError)?;
        let mut fat_guard = fat_drv.lock_inner();
        let fat = &mut *fat_guard;
        let (new_cluster, new_size) = fat.write_file(old_inode, position, buf, old_size)?;
        if new_cluster != old_inode || new_size != old_size {
            fat.update_entry(parent_cluster, filename, new_size, new_cluster)?;
        }
        let file = state
            .open_files
            .get_mut(slot_id as usize)
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
        if let Some(exfat_drv) = state.exfat_fs.as_ref() {
            let mut exfat_guard = exfat_drv.lock_inner();
            let exfat = &mut *exfat_guard;
            if exfat.metadata_dirty {
                let _ = exfat.flush_metadata();
            }
        }
        for (_path, exfat) in &mut state.mounted_exfat {
            let mut exfat = exfat.lock_inner();
            if exfat.metadata_dirty {
                let _ = exfat.flush_metadata();
            }
        }
    }
    Ok(buf.len())
}

/// Read directory entries at a given path.
pub fn read_dir(path: &str) -> Result<Vec<DirEntry>, FsError> {
    if let Some(plan) = prepare_detached_read_dir(path)? {
        return execute_detached_read_dir(plan);
    }
    let mut vfs = vfs_lock();
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
                        uid: 0,
                        gid: 0,
                        mode: 0xFFF,
                    });
                }
            }
        }
        return Ok(entries);
    }

    // --- Mount point path (e.g. /mnt/cdrom0/..., /mnt/share/...) ---
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
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
                if let Some(ntfs_drv) = state.ntfs_fs.as_ref() {
                    let ntfs_guard = ntfs_drv.lock_inner();
                    let ntfs = &*ntfs_guard;
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
                let exfat = state
                    .mounted_exfat
                    .iter()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, fs)| fs)
                    .ok_or(FsError::IoError)?;
                let exfat = exfat.lock_inner();
                let (inode, file_type, _size) = exfat.lookup(relative_path)?;
                if file_type != FileType::Directory {
                    return Err(FsError::NotADirectory);
                }
                let (cluster, _) = crate::fs::exfat::decode_inode(inode);
                return exfat.read_dir(cluster);
            }
            FsType::Smb => {
                let mount_path_owned = String::from(mount_path);
                let smb = state
                    .smbfs
                    .iter_mut()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, s)| s)
                    .ok_or(FsError::IoError)?;
                let (inode, file_type, _size) = smb.lookup(relative_path)?;
                if file_type != FileType::Directory {
                    return Err(FsError::NotADirectory);
                }
                return smb.read_dir(inode);
            }
            FsType::CoreFs => {
                let driver = state
                    .corefs_for_mount(mount_path)
                    .ok_or(FsError::NotFound)?;
                let q = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                let (inode, file_type, _size) = Filesystem::lookup(driver, q)?;
                if file_type != FileType::Directory {
                    return Err(FsError::NotADirectory);
                }
                return Filesystem::readdir(driver, inode);
            }
            FsType::Fuse => {
                let session_id = fuse_session_id_for(state, mount_path).ok_or(FsError::IoError)?;
                let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
                let (_attr, _vfs_u32, ino_u64) =
                    fuse_resolve_path(&session, session_id, relative_path)?;
                let req = fuse_proto::Request::Readdir {
                    ino: ino_u64,
                    offset: 0,
                };
                let reply = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
                let entries = match reply {
                    fuse_proto::Reply::Readdir(v) => v,
                    _ => return Err(FsError::IoError),
                };
                let mut out = Vec::with_capacity(entries.len());
                for e in entries {
                    out.push(DirEntry {
                        name: e.name,
                        file_type: crate::fs::fuse::attr_kind_to_file_type(e.kind),
                        size: 0,
                        is_symlink: e.kind == 3,
                        uid: 0,
                        gid: 0,
                        mode: 0o755,
                    });
                }
                return Ok(out);
            }
            _ => {
                return Err(FsError::NotFound);
            }
        }
    }

    // --- exFAT path (primary, with symlink resolution) ---
    if let Some(exfat_drv) = state.exfat_fs.as_ref() {
        let exfat_guard = exfat_drv.lock_inner();
        let exfat = &*exfat_guard;
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
    if let Some(ntfs_drv) = state.ntfs_fs.as_ref() {
        let ntfs_guard = ntfs_drv.lock_inner();
        let ntfs = &*ntfs_guard;
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
    if let Some(fat_drv) = state.fat_fs.as_ref() {
        let fat_guard = fat_drv.lock_inner();
        let fat = &*fat_guard;
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

    // --- CoreFS root path (Phase 6 generic dispatch) ---
    if let Some(driver) = state.corefs_driver.as_ref() {
        let driver = driver.as_ref();
        let q = if path.is_empty() { "/" } else { path };
        let (inode, file_type, _size) = Filesystem::lookup(driver, q)?;
        if file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let mut entries = Filesystem::readdir(driver, inode)?;
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
    add_virtual_root_entries_snapshot(
        state.devfs.is_some(),
        state
            .mount_points
            .iter()
            .any(|mp| mp.path.starts_with("/mnt/")),
        entries,
    );
}

fn add_virtual_root_entries_snapshot(add_dev: bool, add_mnt: bool, entries: &mut Vec<DirEntry>) {
    if add_dev {
        entries.push(DirEntry {
            name: String::from("dev"),
            file_type: FileType::Directory,
            size: 0,
            is_symlink: false,
            uid: 0,
            gid: 0,
            mode: 0xFFF,
        });
    }
    if add_mnt {
        entries.push(DirEntry {
            name: String::from("mnt"),
            file_type: FileType::Directory,
            size: 0,
            is_symlink: false,
            uid: 0,
            gid: 0,
            mode: 0xFFF,
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
        CoreFs {
            driver: Arc<crate::fs::corefs::CoreFsDriver>,
            inode: u32,
            size: u32,
        },
    }

    // Device files are streaming — can't read to vec
    if is_dev_path(path) {
        return Err(FsError::PermissionDenied);
    }

    // Try mount point path (e.g. /mnt/cdrom0/..., /mnt/share/...)
    {
        let mut vfs = vfs_lock();
        let state = vfs.as_mut().ok_or(FsError::IoError)?;
        if let Some((mount_path, relative_path, mnt_fs_type)) =
            find_submount(path, &state.mount_points)
        {
            match mnt_fs_type {
                FsType::Iso9660 => {
                    if let Some(ref iso) = state.iso9660_fs {
                        return iso.read_file_to_vec(relative_path);
                    }
                    return Err(FsError::NotFound);
                }
                FsType::ExFat => {
                    let mount_path_owned = String::from(mount_path);
                    let exfat = state
                        .mounted_exfat
                        .iter()
                        .find(|(p, _)| *p == mount_path_owned)
                        .map(|(_, fs)| fs)
                        .ok_or(FsError::IoError)?;
                    let exfat = exfat.lock_inner();
                    let r = resolve_exfat_path(&exfat, relative_path, true)?;
                    let inode = r.inode;
                    let file_type = r.file_type;
                    let size = r.size;
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
                    let smb = state
                        .smbfs
                        .iter_mut()
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
        let vfs = vfs_lock();
        let state = vfs.as_ref().ok_or(FsError::IoError)?;
        if let Some(exfat_drv) = state.exfat_fs.as_ref() {
            let exfat_guard = exfat_drv.lock_inner();
            let exfat = &*exfat_guard;
            let r = resolve_exfat_path(exfat, path, true)?;
            if r.file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            ReadPlan::ExFat(exfat.get_file_read_plan(r.inode, r.size))
        } else if let Some(ntfs_drv) = state.ntfs_fs.as_ref() {
            let ntfs_guard = ntfs_drv.lock_inner();
            let ntfs = &*ntfs_guard;
            let (mft_rec, file_type, size) = ntfs.lookup(path)?;
            if file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            ReadPlan::Ntfs(ntfs.get_file_read_plan(mft_rec, size))
        } else if let Some(fat_drv) = state.fat_fs.as_ref() {
            let fat_guard = fat_drv.lock_inner();
            let fat = &*fat_guard;
            let (cluster, file_type, size) = fat.lookup(path)?;
            if file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            ReadPlan::Fat(fat.get_file_read_plan(cluster, size))
        } else if let Some(driver) = state.corefs_driver.as_ref() {
            let driver = Arc::clone(driver);
            let q = if path.is_empty() { "/" } else { path };
            let (inode, file_type, size) = Filesystem::lookup(driver.as_ref(), q)?;
            if file_type == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            ReadPlan::CoreFs {
                driver,
                inode,
                size,
            }
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
        ReadPlan::CoreFs {
            driver,
            inode,
            size,
        } => {
            let mut buf = alloc::vec![0u8; size as usize];
            let n = Filesystem::read(driver.as_ref(), inode, 0, &mut buf)?;
            buf.truncate(n);
            Ok(buf)
        }
    };
    result
}

/// Delete a file, directory, or symlink at the given path.
/// Symlinks are deleted without following (only the link is removed).
pub fn delete(path: &str) -> Result<(), FsError> {
    if is_dev_path(path) {
        return Err(FsError::PermissionDenied);
    }
    if let Some(plan) = prepare_detached_delete(path)? {
        return execute_detached_delete(plan);
    }
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // --- Mount point path (SMB / CoreFS delete) ---
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        if mnt_fs_type == FsType::CoreFs {
            let driver = state
                .corefs_for_mount(mount_path)
                .ok_or(FsError::NotFound)?;
            let rel = if relative_path.is_empty() {
                "/"
            } else {
                relative_path
            };
            let rel = rel.trim_end_matches('/');
            let (parent, name) = match rel.rfind('/') {
                Some(0) => ("/", &rel[1..]),
                Some(pos) => (&rel[..pos], &rel[pos + 1..]),
                None => ("/", rel),
            };
            if name.is_empty() {
                return Err(FsError::InvalidPath);
            }
            let (parent_inode, parent_type, _) = Filesystem::lookup(driver, parent)?;
            if parent_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            return Filesystem::delete(driver, parent_inode, name);
        }
        if mnt_fs_type == FsType::Fuse {
            let session_id = fuse_session_id_for(state, mount_path).ok_or(FsError::IoError)?;
            let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
            let (parent_rel, name) = fuse_split_parent_name(relative_path)?;
            if name.is_empty() {
                return Err(FsError::InvalidPath);
            }
            let (_p_attr, _p_u32, parent_u64) =
                fuse_resolve_path(&session, session_id, parent_rel)?;
            // Resolve the child entry to pick Unlink vs. Rmdir based on type.
            let child_attr = {
                let req = fuse_proto::Request::Lookup {
                    parent: parent_u64,
                    name: String::from(name),
                };
                match crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)? {
                    fuse_proto::Reply::Lookup(a) => a,
                    _ => return Err(FsError::IoError),
                }
            };
            let req = if child_attr.kind == 2 {
                fuse_proto::Request::Rmdir {
                    parent: parent_u64,
                    name: String::from(name),
                }
            } else {
                fuse_proto::Request::Unlink {
                    parent: parent_u64,
                    name: String::from(name),
                }
            };
            let _ = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
            return Ok(());
        }
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
            let smb = state
                .smbfs
                .iter_mut()
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

    // --- Generic root-FS dispatch via Filesystem trait (Phase 6 Step 6). ---
    let driver = state.root_fs().ok_or(FsError::IoError)?;
    let (parent_path, filename) = split_parent_name(path)?;
    let (parent_inode, _, _) = driver.lookup(parent_path)?;
    driver.delete(parent_inode, filename)
}

/// Rename (move) a file or directory from old_path to new_path.
pub fn rename(old_path: &str, new_path: &str) -> Result<(), FsError> {
    if is_dev_path(old_path) || is_dev_path(new_path) {
        return Err(FsError::PermissionDenied);
    }
    if let Some(plan) = prepare_detached_rename(old_path, new_path)? {
        return execute_detached_rename(plan);
    }
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // CoreFS rename/move — currently supported only when source and target
    // live on the same CoreFS mount.
    let corefs_old = find_submount(old_path, &state.mount_points)
        .filter(|(_, _, t)| *t == FsType::CoreFs)
        .map(|(mp, rel, _)| (String::from(mp), String::from(rel)));
    let corefs_new = find_submount(new_path, &state.mount_points)
        .filter(|(_, _, t)| *t == FsType::CoreFs)
        .map(|(mp, rel, _)| (String::from(mp), String::from(rel)));
    match (corefs_old, corefs_new) {
        (Some((mp_old, rel_old)), Some((mp_new, rel_new))) if mp_old == mp_new => {
            let driver = state.corefs_for_mount(&mp_old).ok_or(FsError::NotFound)?;
            fn split_rel(rel: &str) -> Result<(String, String), FsError> {
                let rel_owned: String = if rel.is_empty() {
                    String::from("/")
                } else {
                    String::from(rel)
                };
                let trimmed = rel_owned.trim_end_matches('/');
                let (p, n): (String, String) = match trimmed.rfind('/') {
                    Some(0) => (String::from("/"), String::from(&trimmed[1..])),
                    Some(pos) => (
                        String::from(&trimmed[..pos]),
                        String::from(&trimmed[pos + 1..]),
                    ),
                    None => (String::from("/"), String::from(trimmed)),
                };
                if n.is_empty() {
                    return Err(FsError::InvalidPath);
                }
                Ok((p, n))
            }
            let (op, on) = split_rel(&rel_old)?;
            let (np, nn) = split_rel(&rel_new)?;
            let (old_parent_ino, _, _) = Filesystem::lookup(driver, &op)?;
            let (new_parent_ino, _, _) = Filesystem::lookup(driver, &np)?;
            return driver.rename_entry(old_parent_ino, &on, new_parent_ino, &nn);
        }
        (Some(_), Some(_)) => return Err(FsError::PermissionDenied), // cross-mount
        (Some(_), None) | (None, Some(_)) => return Err(FsError::PermissionDenied),
        _ => {}
    }

    // --- FUSE rename (same-mount only) -----------------------------------
    let fuse_old = find_submount(old_path, &state.mount_points)
        .filter(|(_, _, t)| *t == FsType::Fuse)
        .map(|(mp, rel, _)| (String::from(mp), String::from(rel)));
    let fuse_new = find_submount(new_path, &state.mount_points)
        .filter(|(_, _, t)| *t == FsType::Fuse)
        .map(|(mp, rel, _)| (String::from(mp), String::from(rel)));
    match (fuse_old, fuse_new) {
        (Some((mp_old, rel_old)), Some((mp_new, rel_new))) if mp_old == mp_new => {
            let session_id = fuse_session_id_for(state, &mp_old).ok_or(FsError::IoError)?;
            let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
            let (op_rel, on_owned) = {
                let (p, n) = fuse_split_parent_name(&rel_old)?;
                (String::from(p), String::from(n))
            };
            let (np_rel, nn_owned) = {
                let (p, n) = fuse_split_parent_name(&rel_new)?;
                (String::from(p), String::from(n))
            };
            if on_owned.is_empty() || nn_owned.is_empty() {
                return Err(FsError::InvalidPath);
            }
            let (_oa, _ou, old_parent_u64) = fuse_resolve_path(&session, session_id, &op_rel)?;
            let (_na, _nu, new_parent_u64) = fuse_resolve_path(&session, session_id, &np_rel)?;
            let req = fuse_proto::Request::Rename {
                parent: old_parent_u64,
                old_name: on_owned,
                new_parent: new_parent_u64,
                new_name: nn_owned,
            };
            let _ = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
            return Ok(());
        }
        (Some(_), Some(_)) => return Err(FsError::PermissionDenied), // cross-mount
        (Some(_), None) | (None, Some(_)) => return Err(FsError::PermissionDenied),
        _ => {}
    }

    // --- OverlayFS rename ---
    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_mut().ok_or(FsError::IoError)?;
        return overlay.rename(iso, old_path, new_path);
    }

    // --- Generic root-FS dispatch via Filesystem trait (Phase 6 Step 6). ---
    state
        .root_fs()
        .ok_or(FsError::IoError)?
        .rename(old_path, new_path)
}

/// Create a directory at the given path.
pub fn mkdir(path: &str) -> Result<(), FsError> {
    if is_dev_path(path) {
        return Err(FsError::PermissionDenied);
    }
    if let Some(plan) = prepare_detached_mkdir(path)? {
        return execute_detached_mkdir(plan);
    }
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // --- Mount point path (e.g. /mnt/target/...) ---
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        if mnt_fs_type == FsType::CoreFs {
            let driver = state
                .corefs_for_mount(mount_path)
                .ok_or(FsError::NotFound)?;
            let rel = if relative_path.is_empty() {
                "/"
            } else {
                relative_path
            };
            let rel = rel.trim_end_matches('/');
            let (parent, name) = match rel.rfind('/') {
                Some(0) => ("/", &rel[1..]),
                Some(pos) => (&rel[..pos], &rel[pos + 1..]),
                None => ("/", rel),
            };
            if name.is_empty() {
                return Err(FsError::InvalidPath);
            }
            let (parent_inode, parent_type, _) = Filesystem::lookup(driver, parent)?;
            if parent_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            Filesystem::create(driver, parent_inode, name, FileType::Directory)?;
            return Ok(());
        }
        if mnt_fs_type == FsType::Fuse {
            let session_id = fuse_session_id_for(state, mount_path).ok_or(FsError::IoError)?;
            let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
            let (parent_rel, name) = fuse_split_parent_name(relative_path)?;
            if name.is_empty() {
                return Err(FsError::InvalidPath);
            }
            let (_p_attr, _p_u32, parent_u64) =
                fuse_resolve_path(&session, session_id, parent_rel)?;
            let req = fuse_proto::Request::Mkdir {
                parent: parent_u64,
                name: String::from(name),
                mode: 0o755,
            };
            let _ = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
            return Ok(());
        }
        if mnt_fs_type == FsType::ExFat {
            let mount_path_owned = String::from(mount_path);
            let (parent_rel, dirname) = split_parent_name(relative_path)?;
            let exfat = state
                .mounted_exfat
                .iter_mut()
                .find(|(p, _)| *p == mount_path_owned)
                .map(|(_, fs)| fs)
                .ok_or(FsError::IoError)?;
            let mut exfat = exfat.lock_inner();
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

    // --- Generic root-FS dispatch via Filesystem trait (Phase 6 Step 6).
    //
    // Previously this had per-FS branches for CoreFS / exFAT / FAT; the
    // exFAT branch followed intermediate symlinks in the parent via
    // `resolve_exfat_path`, the others did not.  Trait lookup is a plain
    // path walk for all four root FSes — mkdir through a symlinked
    // parent on exFAT is no longer auto-resolved (unusual in practice,
    // callers can resolve the path first if they need it).
    let driver = state.root_fs().ok_or(FsError::IoError)?;
    let (parent_path, dirname) = split_parent_name(path)?;
    let (parent_inode, parent_type, _) = driver.lookup(parent_path)?;
    if parent_type != FileType::Directory {
        return Err(FsError::NotADirectory);
    }
    driver.create(parent_inode, dirname, FileType::Directory)?;
    Ok(())
}

/// Seek within an open file. `slot_id` is the global open_files index.
/// Returns new position.
pub fn lseek(slot_id: FileDescriptor, offset: i32, whence: u32) -> Result<u32, FsError> {
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    let file = state
        .open_files
        .get_mut(slot_id as usize)
        .and_then(|e| e.as_mut())
        .ok_or(FsError::BadFd)?;

    // Device files don't support seeking
    if file.fs_id == 1 {
        return Ok(0);
    }

    let new_pos = match whence {
        0 => {
            // SEEK_SET
            if offset < 0 {
                return Err(FsError::InvalidPath);
            }
            offset as u32
        }
        1 => {
            // SEEK_CUR
            if offset < 0 {
                file.position
                    .checked_sub((-offset) as u32)
                    .ok_or(FsError::InvalidPath)?
            } else {
                file.position + offset as u32
            }
        }
        2 => {
            // SEEK_END
            if offset < 0 {
                file.size
                    .checked_sub((-offset) as u32)
                    .ok_or(FsError::InvalidPath)?
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
    if let Some(plan) = prepare_detached_stat(path, follow_last) {
        return execute_detached_stat(&plan);
    }

    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    let default_stat = |ft, sz, sym| StatResult {
        file_type: ft,
        size: sz,
        is_symlink: sym,
        uid: 0,
        gid: 0,
        mode: 0xFFF,
        mtime: 0,
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
    if path == "/" {
        return Ok(default_stat(FileType::Directory, 0, false));
    }
    if path == "/mnt" || path == "/mnt/" {
        return Ok(default_stat(FileType::Directory, 0, false));
    }
    if path == "/dev" || path == "/dev/" {
        return Ok(default_stat(FileType::Directory, 0, false));
    }

    // --- Mount point path ---
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        match mnt_fs_type {
            FsType::Iso9660 => {
                if let Some(ref iso) = state.iso9660_fs {
                    let (_inode, file_type, size) = iso.lookup(relative_path)?;
                    return Ok(default_stat(file_type, size, false));
                }
                return Err(FsError::NotFound);
            }
            FsType::Ntfs => {
                if let Some(ntfs_drv) = state.ntfs_fs.as_ref() {
                    let ntfs_guard = ntfs_drv.lock_inner();
                    let ntfs = &*ntfs_guard;
                    let (_inode, file_type, size) = ntfs.lookup(relative_path)?;
                    return Ok(default_stat(file_type, size, false));
                }
                return Err(FsError::NotFound);
            }
            FsType::ExFat => {
                let mount_path_owned = String::from(mount_path);
                let exfat = state
                    .mounted_exfat
                    .iter()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, fs)| fs)
                    .ok_or(FsError::IoError)?;
                let exfat = exfat.lock_inner();
                let r = resolve_exfat_path(&exfat, relative_path, follow_last)?;
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
            FsType::Smb => {
                let mount_path_owned = String::from(mount_path);
                let smb = state
                    .smbfs
                    .iter_mut()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, s)| s)
                    .ok_or(FsError::IoError)?;
                let (_inode, file_type, size) = smb.lookup(relative_path)?;
                return Ok(default_stat(file_type, size, false));
            }
            FsType::CoreFs => {
                let driver = state
                    .corefs_for_mount(mount_path)
                    .ok_or(FsError::NotFound)?;
                // Rewrite relative_path so that the CoreFsDriver's internal
                // path layout ("/foo/bar") matches — CoreFS stores everything
                // under "/", so the relative-to-mount path already starts
                // with '/'. If the caller asked for the mount root itself,
                // map to "/".
                let q = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                return Filesystem::stat(driver, q);
            }
            FsType::Fuse => {
                let session_id = state
                    .mount_points
                    .iter()
                    .find(|m| m.path == mount_path)
                    .map(|m| m.device_id)
                    .ok_or(FsError::IoError)?;
                let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
                let (attr, _, _) = fuse_resolve_path(&session, session_id, relative_path)?;
                return Ok(StatResult {
                    file_type: crate::fs::fuse::attr_kind_to_file_type(attr.kind),
                    size: attr.size as u32,
                    is_symlink: attr.kind == 3,
                    uid: attr.uid as u16,
                    gid: attr.gid as u16,
                    mode: (attr.mode & 0xFFF) as u16,
                    mtime: attr.mtime_secs as u32,
                });
            }
            _ => return Err(FsError::NotFound),
        }
    }

    // --- exFAT path (with symlink resolution) ---
    if let Some(exfat_drv) = state.exfat_fs.as_ref() {
        let exfat_guard = exfat_drv.lock_inner();
        let exfat = &*exfat_guard;
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
    if let Some(ntfs_drv) = state.ntfs_fs.as_ref() {
        let ntfs_guard = ntfs_drv.lock_inner();
        let ntfs = &*ntfs_guard;
        let (file_type, size, _created, modified, _accessed) = ntfs.stat_path(path)?;
        return Ok(StatResult {
            file_type,
            size,
            is_symlink: false,
            uid: 0,
            gid: 0,
            mode: 0o555,
            mtime: modified,
        });
    }
    if let Some(fat_drv) = state.fat_fs.as_ref() {
        let fat_guard = fat_drv.lock_inner();
        let fat = &*fat_guard;
        let (_inode, file_type, size, mtime) = fat.stat_path(path)?;
        return Ok(StatResult {
            file_type,
            size,
            is_symlink: false,
            uid: 0,
            gid: 0,
            mode: 0xFFF,
            mtime,
        });
    }

    // --- CoreFS root path (Phase 6 generic dispatch) ---
    if let Some(driver) = state.corefs_driver.as_ref() {
        let driver = driver.as_ref();
        let q = if path.is_empty() { "/" } else { path };
        return Filesystem::stat(driver, q);
    }

    // --- OverlayFS root (CD-ROM boot with writable RAM overlay) ---
    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_ref().ok_or(FsError::IoError)?;
        let (file_type, size) = overlay.stat(iso, path)?;
        return Ok(StatResult {
            file_type,
            size,
            is_symlink: false,
            uid: 0,
            gid: 0,
            mode: 0xFFF,
            mtime: 0,
        });
    }

    // --- ISO 9660 root fallback (CD-ROM boot without overlay, read-only) ---
    if let Some(ref iso) = state.iso9660_fs {
        let (_inode, file_type, size) = iso.lookup(path)?;
        return Ok(StatResult {
            file_type,
            size,
            is_symlink: false,
            uid: 0,
            gid: 0,
            mode: 0o555,
            mtime: 0,
        });
    }

    Err(FsError::NotFound)
}

/// Get file info by slot_id (global open_files index).
/// Returns (file_type, size, position, mtime).
pub fn fstat(slot_id: FileDescriptor) -> Result<(FileType, u32, u32, u32), FsError> {
    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    let file = state
        .open_files
        .get(slot_id as usize)
        .and_then(|e| e.as_ref())
        .ok_or(FsError::BadFd)?;

    let path = file.path.clone();
    let ft = file.file_type;
    let sz = file.size;
    let pos = file.position;

    // Look up mtime from the filesystem
    let mtime = if let Some(exfat_drv) = state.exfat_fs.as_ref() {
        let exfat_guard = exfat_drv.lock_inner();
        let exfat = &*exfat_guard;
        resolve_exfat_path(exfat, &path, true)
            .map(|r| r.mtime)
            .unwrap_or(0)
    } else if let Some(ntfs_drv) = state.ntfs_fs.as_ref() {
        let ntfs_guard = ntfs_drv.lock_inner();
        let ntfs = &*ntfs_guard;
        ntfs.stat_path(&path).map(|(_, _, _, m, _)| m).unwrap_or(0)
    } else if let Some(fat_drv) = state.fat_fs.as_ref() {
        let fat_guard = fat_drv.lock_inner();
        let fat = &*fat_guard;
        fat.stat_path(&path).map(|(_, _, _, m)| m).unwrap_or(0)
    } else {
        0
    };

    Ok((ft, sz, pos, mtime))
}

/// Get the path associated with an open file descriptor.
pub fn get_fd_path(slot_id: FileDescriptor) -> Result<alloc::string::String, FsError> {
    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;
    let file = state
        .open_files
        .get(slot_id as usize)
        .and_then(|e| e.as_ref())
        .ok_or(FsError::BadFd)?;
    Ok(file.path.clone())
}

/// Truncate a file to zero length.
pub fn truncate(path: &str) -> Result<(), FsError> {
    if is_dev_path(path) {
        return Err(FsError::PermissionDenied);
    }
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // CoreFS truncate-to-zero via driver.
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        if mnt_fs_type == FsType::CoreFs {
            let driver = state
                .corefs_for_mount(mount_path)
                .ok_or(FsError::NotFound)?;
            let rel = if relative_path.is_empty() {
                "/"
            } else {
                relative_path
            };
            let (inode, ft, _sz) = Filesystem::lookup(driver, rel)?;
            if ft == FileType::Directory {
                return Err(FsError::IsADirectory);
            }
            return driver.truncate_file(inode, 0);
        }
        if mnt_fs_type == FsType::Fuse {
            let session_id = fuse_session_id_for(state, mount_path).ok_or(FsError::IoError)?;
            let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
            let (attr, _u32, ino_u64) = fuse_resolve_path(&session, session_id, relative_path)?;
            if attr.kind == 2 {
                return Err(FsError::IsADirectory);
            }
            let req = fuse_proto::Request::Setattr {
                ino: ino_u64,
                attr: fuse_proto::PartialAttr {
                    size: Some(0),
                    ..Default::default()
                },
            };
            let _ = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
            return Ok(());
        }
    }

    // --- OverlayFS truncate ---
    if state.overlay_fs.is_some() && state.iso9660_fs.is_some() {
        let iso = state.iso9660_fs.as_ref().ok_or(FsError::IoError)?;
        let overlay = state.overlay_fs.as_mut().ok_or(FsError::IoError)?;
        return overlay.truncate(iso, path);
    }

    // --- Generic root-FS dispatch (Phase 6 Step 6). ---
    state
        .root_fs()
        .ok_or(FsError::IoError)?
        .truncate_by_path(path)
}

/// Resolve a mount device spec to a BlockDevice.
///
/// Accepts either a decimal device ID (e.g. `"3"` from SYS_DISK_LIST) or a
/// device path (e.g. `"/dev/sda1"`, `"/dev/hd0p1"`).
fn resolve_mount_device(
    device: &str,
) -> Result<crate::drivers::storage::blockdev::BlockDevice, FsError> {
    use crate::drivers::storage::blockdev;
    if device.starts_with("/dev/") {
        let (disk_id, partition) = blockdev::parse_device_path(device).ok_or_else(|| {
            crate::serial_verbose_println!("  mount_fs: invalid device path '{}'", device);
            FsError::InvalidPath
        })?;
        blockdev::find_device(disk_id, partition).ok_or_else(|| {
            crate::serial_verbose_println!("  mount_fs: device '{}' not found", device);
            FsError::NotFound
        })
    } else {
        let dev_id: u8 = device.parse::<u8>().map_err(|_| {
            crate::serial_verbose_println!(
                "  mount_fs: invalid device '{}' (expected numeric device_id or /dev/ path)",
                device
            );
            FsError::InvalidPath
        })?;
        blockdev::get_device(dev_id).ok_or_else(|| {
            crate::serial_verbose_println!("  mount_fs: device {} not found", dev_id);
            FsError::NotFound
        })
    }
}

/// Mount a filesystem at the given path from userspace (syscall handler).
///
/// `mount_path`: where to mount (e.g. "/mnt/cdrom0")
/// `device`: device ID ("3"), device path ("/dev/sda1"), or "//ip/share" (SMB)
/// `fs_type_id`: 0=FAT, 1=ISO9660, 4=NTFS, 5=SMB, 6=CoreFS, 7=exFAT
///
/// Returns Ok(()) on success.
pub fn mount_fs(mount_path: &str, device: &str, fs_type_id: u32) -> Result<(), FsError> {
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Check for duplicate mount point
    for mp in &state.mount_points {
        if mp.path == mount_path {
            return Err(FsError::AlreadyExists);
        }
    }

    match fs_type_id {
        0 | 7 => {
            // exFAT/FAT mount by device ID ("3") or device path ("/dev/sda1")
            let bdev = resolve_mount_device(device)?;
            let dev_id = bdev.id;
            let start_lba = bdev.start_lba as u32;
            crate::serial_verbose_println!(
                "  mount_fs: exFAT device={} disk={} start_lba={}",
                dev_id,
                bdev.disk_id,
                start_lba
            );
            match ExFatFs::new(bdev.disk_id as u32, start_lba) {
                Ok(exfat) => {
                    state.mounted_exfat.push((
                        String::from(mount_path),
                        Arc::new(ExFatFsDriver::new(exfat)),
                    ));
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
                        state.ntfs_fs = Some(NtfsFsDriver::new(ntfs));
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
            let mut vfs = vfs_lock();
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
        6 => {
            // CoreFS mount by device ID ("3") or device path ("/dev/sda1")
            let bdev = resolve_mount_device(device)?;
            let dev_id = bdev.id;
            // Partition-only: whole-disk devices cannot be CoreFS-mounted
            if bdev.partition.is_none() {
                crate::serial_verbose_println!(
                    "  mount_fs: device {} is a whole disk, not a partition",
                    dev_id
                );
                return Err(FsError::InvalidPath);
            }
            let start_lba = bdev.start_lba as u32;
            let size_sectors = bdev.size_sectors;
            // Probe for CoreFS superblock magic
            if !crate::fs::corefs::detect(bdev.disk_id, start_lba) {
                crate::serial_verbose_println!(
                    "  mount_fs: device {} (disk={} lba={}) has no CoreFS signature",
                    dev_id,
                    bdev.disk_id,
                    start_lba
                );
                return Err(FsError::InvalidPath);
            }
            // Reject duplicate mount at the same path; multiple CoreFS
            // partitions at distinct paths are allowed (mounted_corefs Vec).
            if state.mount_points.iter().any(|mp| mp.path == mount_path) {
                crate::serial_verbose_println!("  mount_fs: already mounted at '{}'", mount_path);
                return Err(FsError::AlreadyExists);
            }
            // Drop VFS lock — mount_corefs acquires it internally
            drop(vfs);
            mount_corefs(
                mount_path,
                bdev.disk_id,
                start_lba,
                size_sectors,
                dev_id as u32,
            )
        }
        _ => Err(FsError::InvalidPath),
    }
}

/// Flush metadata for a specific open file to disk (fsync semantics).
/// Ensures all deferred FAT/bitmap writes for the file's filesystem are persisted.
pub fn fsync(slot_id: FileDescriptor) -> Result<(), FsError> {
    sync_file(slot_id, true)
}

/// Flush file data and filesystem metadata without forcing the device hardware
/// cache. This matches the lighter-weight path Linux callers expect from
/// fdatasync-style workloads and keeps benchmark fsync costs explicit.
pub fn fdatasync(slot_id: FileDescriptor) -> Result<(), FsError> {
    sync_file(slot_id, false)
}

fn sync_file(slot_id: FileDescriptor, flush_hardware: bool) -> Result<(), FsError> {
    let mut disks_to_flush: Vec<u8> = Vec::new();
    let mut exfat_commit: Option<DetachedExFatCommit> = None;
    let mut corefs_to_flush: Option<Arc<crate::fs::corefs::CoreFsDriver>> = None;
    {
        let mut vfs = vfs_lock();
        let state = vfs.as_mut().ok_or(FsError::IoError)?;

        let file = state
            .open_files
            .get(slot_id as usize)
            .and_then(|e| e.as_ref())
            .ok_or(FsError::BadFd)?;

        match file.fs_id {
            3 | 6 => {
                exfat_commit = snapshot_open_exfat_commit(state, slot_id as usize, true)?;
            }
            8 => {
                corefs_to_flush = Some(
                    state
                        .corefs_handle_for_path(&file.path)
                        .ok_or(FsError::IoError)?,
                );
            }
            _ => {} // Other filesystems flush synchronously already
        }
    }

    if let Some(commit) = exfat_commit.as_ref() {
        let disk_id = finish_detached_exfat_commit(commit)?;
        queue_disk_flush(&mut disks_to_flush, disk_id);
        let mut vfs = vfs_lock();
        if let Some(state) = vfs.as_mut() {
            mark_detached_exfat_commit_clean(state, slot_id as usize, commit);
        }
    }
    if let Some(driver) = corefs_to_flush {
        driver.flush()?;
    }
    // Flush write-back block cache, then storage hardware cache
    flush_blockcache_for_disks(&disks_to_flush);
    if flush_hardware {
        flush_hardware_for_disks(&disks_to_flush);
    }
    Ok(())
}

fn sync_all_inner(flush_hardware: bool) {
    let mut disks_to_flush: Vec<u8> = Vec::new();
    let mut exfat_commits: Vec<(usize, DetachedExFatCommit)> = Vec::new();
    let mut exfat_drivers: Vec<Arc<ExFatFsDriver>> = Vec::new();
    let mut corefs_drivers: Vec<Arc<crate::fs::corefs::CoreFsDriver>> = Vec::new();
    {
        let mut vfs = vfs_lock();
        let Some(state) = vfs.as_mut() else {
            return;
        };
        for idx in 0..state.open_files.len() {
            let needs_commit = state
                .open_files
                .get(idx)
                .and_then(|e| e.as_ref())
                .map(|file| (file.fs_id == 3 || file.fs_id == 6) && file.entry_dirty)
                .unwrap_or(false);
            if needs_commit {
                if let Ok(Some(commit)) = snapshot_open_exfat_commit(state, idx, false) {
                    exfat_commits.push((idx, commit));
                }
            }
        }
        if let Some(exfat_drv) = state.exfat_fs.as_ref() {
            exfat_drivers.push(Arc::clone(exfat_drv));
        }
        for (_path, exfat) in &state.mounted_exfat {
            exfat_drivers.push(Arc::clone(exfat));
        }
        if let Some(driver) = state.corefs_driver.as_ref() {
            corefs_drivers.push(Arc::clone(driver));
        }
        for (_, driver) in &state.mounted_corefs {
            corefs_drivers.push(Arc::clone(driver));
        }
    }

    for (idx, commit) in &exfat_commits {
        if let Ok(disk_id) = finish_detached_exfat_commit(commit) {
            queue_disk_flush(&mut disks_to_flush, disk_id);
            let mut vfs = vfs_lock();
            if let Some(state) = vfs.as_mut() {
                mark_detached_exfat_commit_clean(state, *idx, commit);
            }
        }
    }
    for driver in exfat_drivers {
        let mut exfat = driver.lock_inner();
        let _ = exfat.flush_metadata();
        queue_disk_flush(&mut disks_to_flush, exfat.device_id as u8);
    }
    for driver in corefs_drivers {
        let _ = driver.flush();
    }
    // Flush write-back block cache to disk (coalesced writes)
    flush_blockcache_for_disks(&disks_to_flush);
    if flush_hardware {
        // Then flush the drive's hardware write cache to persistent media.
        flush_hardware_for_disks(&disks_to_flush);
    }
}

/// Flush all dirty filesystem metadata and storage write caches.
pub fn sync_all() {
    sync_all_inner(true);
}

/// Flush filesystem metadata and write-back data without forcing the drive's
/// hardware cache. This is used for LXE Linux sync-style syscalls where doing
/// an AHCI FLUSH CACHE EXT for every package-manager durability point stalls
/// the whole guest much harder than Linux's block layer normally would.
pub fn sync_all_data_only() {
    sync_all_inner(false);
}

pub fn umount_fs(mount_path: &str) -> Result<(), FsError> {
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // Don't allow unmounting root or /dev
    if mount_path == "/" || mount_path == "/dev" {
        return Err(FsError::PermissionDenied);
    }

    // Find and remove the mount point
    let pos = state
        .mount_points
        .iter()
        .position(|mp| mp.path == mount_path);
    if let Some(idx) = pos {
        let mp = state.mount_points.remove(idx);

        // If it was ISO 9660 and no other ISO mounts remain, drop the fs instance
        if mp.fs_type == FsType::Iso9660 {
            let has_other_iso = state
                .mount_points
                .iter()
                .any(|m| m.fs_type == FsType::Iso9660);
            if !has_other_iso {
                state.iso9660_fs = None;
            }
        }

        // If it was mounted exFAT, flush metadata and remove the fs instance
        if mp.fs_type == FsType::ExFat {
            if let Some(idx) = state
                .mounted_exfat
                .iter()
                .position(|(p, _)| p == mount_path)
            {
                // Flush any pending metadata before dropping
                let _ = state.mounted_exfat[idx].1.lock_inner().flush_metadata();
                state.mounted_exfat.remove(idx);
            }
        }

        // If it was CoreFS, flush pending writes and drop the driver.
        // Sub-mounts live in `mounted_corefs`; only the root mount ("/")
        // uses the typed `corefs_driver` slot.
        if mp.fs_type == FsType::CoreFs {
            if let Some(idx) = state
                .mounted_corefs
                .iter()
                .position(|(p, _)| p == mount_path)
            {
                let (_, driver) = state.mounted_corefs.remove(idx);
                let _ = driver.flush();
                crate::serial_verbose_println!("  Unmounted CoreFS '{}'", mount_path);
            } else if mount_path == "/" {
                if let Some(driver) = state.corefs_driver.take() {
                    let _ = driver.flush();
                    crate::serial_verbose_println!("  Unmounted CoreFS '/'");
                }
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
    if is_dev_path(link_path) {
        return Err(FsError::PermissionDenied);
    }
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::IoError)?;

    // CoreFS symlinks via driver.
    if let Some((mount_path, relative_path, mnt_fs_type)) =
        find_submount(link_path, &state.mount_points)
    {
        if mnt_fs_type == FsType::ExFat {
            let mount_path_owned = String::from(mount_path);
            let exfat = state
                .mounted_exfat
                .iter_mut()
                .find(|(p, _)| *p == mount_path_owned)
                .map(|(_, fs)| fs)
                .ok_or(FsError::IoError)?;
            let rel = if relative_path.is_empty() {
                "/"
            } else {
                relative_path
            };
            let rel = rel.trim_end_matches('/');
            let (parent, name) = match rel.rfind('/') {
                Some(0) => ("/", &rel[1..]),
                Some(pos) => (&rel[..pos], &rel[pos + 1..]),
                None => ("/", rel),
            };
            if name.is_empty() {
                return Err(FsError::InvalidPath);
            }
            let mut exfat = exfat.lock_inner();
            let pr = resolve_exfat_path(&exfat, parent, true)?;
            if pr.file_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            let (parent_cluster, _) = crate::fs::exfat::decode_inode(pr.inode);
            exfat.create_symlink(parent_cluster, name, target)?;
            return Ok(());
        }
        if mnt_fs_type == FsType::CoreFs {
            let driver = state
                .corefs_for_mount(mount_path)
                .ok_or(FsError::NotFound)?;
            let rel = if relative_path.is_empty() {
                "/"
            } else {
                relative_path
            };
            let rel = rel.trim_end_matches('/');
            let (parent, name) = match rel.rfind('/') {
                Some(0) => ("/", &rel[1..]),
                Some(pos) => (&rel[..pos], &rel[pos + 1..]),
                None => ("/", rel),
            };
            if name.is_empty() {
                return Err(FsError::InvalidPath);
            }
            let (parent_inode, parent_type, _) = Filesystem::lookup(driver, parent)?;
            if parent_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            driver.create_symlink(parent_inode, name, target)?;
            return Ok(());
        }
        if mnt_fs_type == FsType::Fuse {
            let session_id = fuse_session_id_for(state, mount_path).ok_or(FsError::IoError)?;
            let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
            let (parent_rel, name) = fuse_split_parent_name(relative_path)?;
            if name.is_empty() {
                return Err(FsError::InvalidPath);
            }
            let (_pa, _pu, parent_u64) = fuse_resolve_path(&session, session_id, parent_rel)?;
            let req = fuse_proto::Request::Symlink {
                parent: parent_u64,
                name: String::from(name),
                target: String::from(target),
            };
            let _ = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
            return Ok(());
        }
    }

    // --- Generic root-FS dispatch (Phase 6 Step 6).  FAT/NTFS inherit
    // the trait default `NotSupported` since they have no symlink
    // support; ExFat and CoreFS override with actual implementations.
    state
        .root_fs()
        .ok_or(FsError::PermissionDenied)?
        .create_symlink(link_path, target)
}

/// Read the target of a symbolic link WITHOUT following it.
/// Returns the target path string.
pub fn readlink(path: &str) -> Result<String, FsError> {
    if is_dev_path(path) {
        return Err(FsError::InvalidPath);
    }
    if let Some(plan) = prepare_detached_readlink(path) {
        return execute_detached_readlink(plan);
    }
    let vfs = vfs_lock();
    let state = vfs.as_ref().ok_or(FsError::IoError)?;

    // FUSE mounts take precedence over legacy backends.
    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        match mnt_fs_type {
            FsType::ExFat => {
                let mount_path_owned = String::from(mount_path);
                let exfat = state
                    .mounted_exfat
                    .iter()
                    .find(|(p, _)| *p == mount_path_owned)
                    .map(|(_, fs)| fs)
                    .ok_or(FsError::IoError)?;
                let exfat = exfat.lock_inner();
                let r = resolve_exfat_path(&exfat, relative_path, false)?;
                if !r.is_symlink {
                    return Err(FsError::InvalidPath);
                }
                return exfat.readlink(r.inode, r.size);
            }
            FsType::CoreFs => {
                let driver = state
                    .corefs_for_mount(mount_path)
                    .ok_or(FsError::NotFound)?;
                let rel = if relative_path.is_empty() {
                    "/"
                } else {
                    relative_path
                };
                let st = Filesystem::stat(driver, rel)?;
                if !st.is_symlink {
                    return Err(FsError::InvalidPath);
                }
                let (inode, _, _) = Filesystem::lookup(driver, rel)?;
                return Filesystem::readlink(driver, inode);
            }
            FsType::Fuse => {
                let session_id = fuse_session_id_for(state, mount_path).ok_or(FsError::IoError)?;
                let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
                let (attr, _u32, ino_u64) = fuse_resolve_path(&session, session_id, relative_path)?;
                if attr.kind != 3 {
                    return Err(FsError::InvalidPath);
                }
                let req = fuse_proto::Request::Readlink { ino: ino_u64 };
                let reply = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
                return match reply {
                    fuse_proto::Reply::Readlink(t) => Ok(t),
                    _ => Err(FsError::IoError),
                };
            }
            _ => {}
        }
    }

    if let Some(exfat_drv) = state.exfat_fs.as_ref() {
        let exfat_guard = exfat_drv.lock_inner();
        let exfat = &*exfat_guard;
        // Resolve all path components EXCEPT the final one
        let r = resolve_exfat_path(exfat, path, false)?;
        if !r.is_symlink {
            return Err(FsError::InvalidPath); // Not a symlink
        }
        return exfat.readlink(r.inode, r.size);
    }
    if let Some(driver) = state.corefs_driver.as_ref() {
        let driver = driver.as_ref();
        let st = Filesystem::stat(driver, path)?;
        if !st.is_symlink {
            return Err(FsError::InvalidPath);
        }
        let (inode, _, _) = Filesystem::lookup(driver, path)?;
        return Filesystem::readlink(driver, inode);
    }
    Err(FsError::PermissionDenied)
}

/// Get (uid, gid, mode) for a path. Returns defaults for non-exFAT filesystems.
pub fn get_permissions(path: &str) -> Result<(u16, u16, u16), FsError> {
    // Virtual paths always have root/full-access
    if path == "/dev" || path.starts_with("/dev/") || path == "/mnt" || path.starts_with("/mnt/") {
        return Ok((0, 0, 0xFFF));
    }

    // --- Generic root-FS dispatch via Filesystem::stat (Phase 6 Step 6).
    //
    // Previously: exFAT used its inherent `get_permissions(path)`, CoreFS
    // used `Filesystem::stat(path)`, FAT/NTFS fell through to defaults.
    // All three now derive from `stat(path)`, which carries uid/gid/mode
    // when the driver populates them (ExFat: dirent, CoreFS: inode
    // metadata; FAT/NTFS return the trait's `0o777` default).
    if let Some(plan) = prepare_detached_stat(path, true) {
        if let Ok(st) = execute_detached_stat(&plan) {
            return Ok((st.uid, st.gid, st.mode));
        }
    } else {
        let q = if path.is_empty() { "/" } else { path };
        let vfs = vfs_lock();
        let state = vfs.as_ref().ok_or(FsError::NotFound)?;
        if let Some(driver) = state.root_fs() {
            if let Ok(st) = driver.stat(q) {
                return Ok((st.uid, st.gid, st.mode));
            }
        }
    }

    // FAT16 / other: no permission support
    Ok((0, 0, 0xFFF))
}

/// Set the mode bits for a path.
pub fn set_mode(path: &str, mode: u16) -> Result<(), FsError> {
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::NotFound)?;

    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        if mnt_fs_type == FsType::CoreFs {
            let driver = state
                .corefs_for_mount(mount_path)
                .ok_or(FsError::NotFound)?;
            let rel = if relative_path.is_empty() {
                "/"
            } else {
                relative_path
            };
            let (inode, _, _) = Filesystem::lookup(driver, rel)?;
            return driver.set_mode(inode, mode as u32);
        }
        if mnt_fs_type == FsType::Fuse {
            let session_id = fuse_session_id_for(state, mount_path).ok_or(FsError::IoError)?;
            let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
            let (_attr, _u32, ino_u64) = fuse_resolve_path(&session, session_id, relative_path)?;
            let req = fuse_proto::Request::Setattr {
                ino: ino_u64,
                attr: fuse_proto::PartialAttr {
                    mode: Some(mode as u32),
                    ..Default::default()
                },
            };
            let _ = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
            return Ok(());
        }
    }

    // --- Generic root-FS dispatch (Phase 6 Step 6). ---
    state
        .root_fs()
        .ok_or(FsError::PermissionDenied)?
        .set_mode_by_path(path, mode)
}

/// Set the owner (uid, gid) for a path.
pub fn set_owner(path: &str, uid: u16, gid: u16) -> Result<(), FsError> {
    let mut vfs = vfs_lock();
    let state = vfs.as_mut().ok_or(FsError::NotFound)?;

    if let Some((mount_path, relative_path, mnt_fs_type)) = find_submount(path, &state.mount_points)
    {
        if mnt_fs_type == FsType::CoreFs {
            let driver = state
                .corefs_for_mount(mount_path)
                .ok_or(FsError::NotFound)?;
            let rel = if relative_path.is_empty() {
                "/"
            } else {
                relative_path
            };
            let (inode, _, _) = Filesystem::lookup(driver, rel)?;
            return driver.set_owner(inode, uid as u32, gid as u32);
        }
        if mnt_fs_type == FsType::Fuse {
            let session_id = fuse_session_id_for(state, mount_path).ok_or(FsError::IoError)?;
            let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;
            let (_attr, _u32, ino_u64) = fuse_resolve_path(&session, session_id, relative_path)?;
            let req = fuse_proto::Request::Setattr {
                ino: ino_u64,
                attr: fuse_proto::PartialAttr {
                    uid: Some(uid as u32),
                    gid: Some(gid as u32),
                    ..Default::default()
                },
            };
            let _ = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
            return Ok(());
        }
    }

    // --- Generic root-FS dispatch (Phase 6 Step 6). ---
    state
        .root_fs()
        .ok_or(FsError::PermissionDenied)?
        .set_owner_by_path(path, uid, gid)
}

/// Returns `true` if the root filesystem is ISO 9660 (live-CD / read-only boot).
/// Used by the permission system to skip persisted-permission checks.
pub fn root_is_iso9660() -> bool {
    let vfs = vfs_lock();
    if let Some(ref state) = *vfs {
        state
            .mount_points
            .iter()
            .any(|mp| mp.path == "/" && mp.fs_type == FsType::Iso9660)
    } else {
        false
    }
}

/// Get filesystem statistics for a mount point path.
/// Returns `None` if the path is not a valid mount point or no stats available.
pub fn statfs(path: &str) -> Option<StatFs> {
    let vfs = vfs_lock();
    let state = vfs.as_ref()?;

    // Try all mount points matching the path (there can be multiple, e.g.
    // a failed disk mount + a successful ISO mount both at "/").
    for mp in state.mount_points.iter().filter(|mp| mp.path == path) {
        let result = match mp.fs_type {
            FsType::ExFat => {
                if path == "/" || path.is_empty() {
                    if let Some(drv) = state.exfat_fs.as_ref() {
                        let fs = drv.lock_inner();
                        let (total, free) = fs.fs_stats();
                        Some(StatFs {
                            total_bytes: total,
                            used_bytes: total - free,
                            free_bytes: free,
                        })
                    } else {
                        None
                    }
                } else {
                    state
                        .mounted_exfat
                        .iter()
                        .find(|(mnt_path, _)| mnt_path == path)
                        .map(|(_, fs)| {
                            let fs = fs.lock_inner();
                            let (total, free) = fs.fs_stats();
                            StatFs {
                                total_bytes: total,
                                used_bytes: total - free,
                                free_bytes: free,
                            }
                        })
                }
            }
            FsType::Iso9660 => state.iso9660_fs.as_ref().map(|iso| {
                let total = iso.total_blocks as u64 * 2048;
                StatFs {
                    total_bytes: total,
                    used_bytes: total,
                    free_bytes: 0,
                }
            }),
            FsType::Ntfs => state.ntfs_fs.as_ref().map(|drv| {
                let ntfs = drv.lock_inner();
                let total = ntfs.total_sectors as u64 * 512;
                StatFs {
                    total_bytes: total,
                    used_bytes: total,
                    free_bytes: 0,
                }
            }),
            FsType::Fat => state.fat_fs.as_ref().map(|drv| {
                let fat = drv.lock_inner();
                let cluster_bytes = fat.sectors_per_cluster as u64 * fat.bytes_per_sector as u64;
                let total = fat.total_clusters as u64 * cluster_bytes;
                StatFs {
                    total_bytes: total,
                    used_bytes: total,
                    free_bytes: 0,
                }
            }),
            FsType::DevFs | FsType::Smb | FsType::Overlay => None,
            FsType::CoreFs => state.corefs_for_mount(path).and_then(|driver| {
                driver.statfs().ok().map(|(total, used, free)| StatFs {
                    total_bytes: total,
                    used_bytes: used,
                    free_bytes: free,
                })
            }),
            FsType::Fuse => {
                // Round-trip Statfs to the userspace daemon. Session lives
                // behind an Arc — we can release the VFS lock during the
                // (potentially blocking) call but for now keep it simple.
                fuse_session_id_for(state, mp.path.as_str())
                    .and_then(crate::fs::fuse::session)
                    .and_then(|session| {
                        let req = fuse_proto::Request::Statfs;
                        crate::fs::fuse::fuse_call(&session, &req).ok()
                    })
                    .and_then(|reply| match reply {
                        fuse_proto::Reply::Statfs(s) => Some(s),
                        _ => None,
                    })
                    .map(|s| {
                        let total = s.blocks.saturating_mul(s.bsize as u64);
                        let free = s.bfree.saturating_mul(s.bsize as u64);
                        let used = total.saturating_sub(free);
                        StatFs {
                            total_bytes: total,
                            used_bytes: used,
                            free_bytes: free,
                        }
                    })
            }
        };
        if result.is_some() {
            return result;
        }
    }
    None
}

/// List all current mount points. Returns Vec of (mount_path, fs_type_name, device_id).
///
/// `device_id` is the blockdev ID as reported to userspace. For mounts other
/// than `/` it comes straight from the MountPoint. The root mount is set up
/// at boot with a placeholder device_id, so we substitute the resolved root
/// blockdev ID (recorded via `set_root_blockdev_id()`) when available.
pub fn list_mounts() -> Vec<(String, &'static str, u32)> {
    let vfs = vfs_lock();
    let root_id = root_blockdev_id().map(|v| v as u32);
    if let Some(ref state) = *vfs {
        state
            .mount_points
            .iter()
            .map(|mp| {
                let fs_name = match mp.fs_type {
                    FsType::ExFat => "exfat",
                    FsType::Fat => "fat16",
                    FsType::Iso9660 => "iso9660",
                    FsType::Ntfs => "ntfs",
                    FsType::DevFs => "devfs",
                    FsType::Smb => "smb",
                    FsType::Overlay => "overlay",
                    FsType::CoreFs => "corefs",
                    FsType::Fuse => "fuse",
                };
                let dev = if mp.path == "/" {
                    root_id.unwrap_or(mp.device_id)
                } else {
                    mp.device_id
                };
                (mp.path.clone(), fs_name, dev)
            })
            .collect()
    } else {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// FUSE dispatch helpers
// ---------------------------------------------------------------------------

use corefs_fuse_proto as fuse_proto;

/// Findet die Session-ID (device_id) eines FUSE-Mounts am gegebenen mount_path.
fn fuse_session_id_for(state: &VfsState, mount_path: &str) -> Option<u32> {
    state
        .mount_points
        .iter()
        .find(|m| m.path == mount_path && m.fs_type == FsType::Fuse)
        .map(|m| m.device_id)
}

/// Fehler-Mapping vom FuseCallError-Helper in [`FsError`].
fn fuse_err(e: crate::fs::fuse::FuseCallError) -> FsError {
    crate::fs::fuse::fuse_call_error_to_fs_error(e)
}

/// Löst einen Pfad relativ zum Mount-Root auf — Komponente für Komponente via
/// `Lookup`-Requests — und liefert das `Attr` samt VFS-u32-Inode und FUSE-u64.
/// Ein leerer/`"/"`-Pfad liefert den Root (Inode=1, via `Getattr`).
fn fuse_resolve_path(
    session: &crate::fs::fuse::FuseSession,
    session_id: u32,
    rel_path: &str,
) -> Result<(fuse_proto::Attr, u32, u64), FsError> {
    let trimmed = rel_path.trim_start_matches('/').trim_end_matches('/');
    // Root case
    if trimmed.is_empty() {
        let req = fuse_proto::Request::Getattr { ino: 1 };
        let reply = crate::fs::fuse::fuse_call(session, &req).map_err(fuse_err)?;
        let attr = match reply {
            fuse_proto::Reply::Getattr(a) => a,
            _ => return Err(FsError::IoError),
        };
        return Ok((attr, 1u32, 1u64));
    }
    // Walk components.
    let mut parent_u64: u64 = 1;
    let mut last_attr: Option<fuse_proto::Attr> = None;
    let mut last_u32: u32 = 1;
    for comp in trimmed.split('/') {
        if comp.is_empty() {
            continue;
        }
        let req = fuse_proto::Request::Lookup {
            parent: parent_u64,
            name: String::from(comp),
        };
        let reply = crate::fs::fuse::fuse_call(session, &req).map_err(fuse_err)?;
        let attr = match reply {
            fuse_proto::Reply::Lookup(a) => a,
            _ => return Err(FsError::IoError),
        };
        last_u32 = crate::fs::fuse::inode_map::intern(session_id, attr.ino);
        parent_u64 = attr.ino;
        last_attr = Some(attr);
    }
    let attr = last_attr.ok_or(FsError::NotFound)?;
    Ok((attr, last_u32, attr.ino))
}

/// Mappt einen kernel-VFS-Pfad in den Mount-relativen Pfad plus Eltern-Inode-u64.
fn fuse_split_parent_name<'a>(rel: &'a str) -> Result<(&'a str, &'a str), FsError> {
    let trimmed = rel.trim_end_matches('/');
    let trimmed = if trimmed.is_empty() { "/" } else { trimmed };
    match trimmed.rfind('/') {
        Some(0) => Ok(("/", &trimmed[1..])),
        Some(pos) => Ok((&trimmed[..pos], &trimmed[pos + 1..])),
        None => Ok(("/", trimmed)),
    }
}

/// Öffnet einen FUSE-Eintrag: Attr auflösen, `Request::Open` an Daemon,
/// neuen OpenFile-Slot anlegen. Die `fh` (FileHandle vom Daemon) wird in
/// `parent_cluster` geparkt (cast u64→u32; Daemons geben typ. kleine ids).
fn fuse_open_entry(
    state: &mut VfsState,
    mount_path: &str,
    rel_path: &str,
    flags: FileFlags,
    full_path: &str,
) -> Result<FileDescriptor, FsError> {
    let session_id = fuse_session_id_for(state, mount_path).ok_or(FsError::IoError)?;
    let session = crate::fs::fuse::session(session_id).ok_or(FsError::NotFound)?;

    // Try to resolve; on NotFound + create, issue Create.
    let resolve = fuse_resolve_path(&session, session_id, rel_path);
    let (attr, fh) = match resolve {
        Ok((attr, _u32, ino_u64)) => {
            // Open-Call
            let req = fuse_proto::Request::Open {
                ino: ino_u64,
                flags: 0,
            };
            let reply = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
            match reply {
                fuse_proto::Reply::Open { fh, .. } => (attr, fh),
                _ => return Err(FsError::IoError),
            }
        }
        Err(FsError::NotFound) if flags.create => {
            let (parent_rel, name) = fuse_split_parent_name(rel_path)?;
            let (_p_attr, _p_u32, parent_u64) =
                fuse_resolve_path(&session, session_id, parent_rel)?;
            let req = fuse_proto::Request::Create {
                parent: parent_u64,
                name: String::from(name),
                mode: 0o644,
                flags: 0,
            };
            let reply = crate::fs::fuse::fuse_call(&session, &req).map_err(fuse_err)?;
            match reply {
                fuse_proto::Reply::Create { attr, fh, .. } => {
                    let _ = crate::fs::fuse::inode_map::intern(session_id, attr.ino);
                    (attr, fh)
                }
                _ => return Err(FsError::IoError),
            }
        }
        Err(e) => return Err(e),
    };

    let inode_u32 = crate::fs::fuse::inode_map::intern(session_id, attr.ino);
    let slot_id = state.alloc_slot().ok_or(FsError::TooManyOpenFiles)?;
    let position = if flags.append { attr.size as u32 } else { 0 };
    let file = OpenFile {
        fd: slot_id,
        path: String::from(full_path),
        file_type: crate::fs::fuse::attr_kind_to_file_type(attr.kind),
        flags,
        position,
        size: attr.size as u32,
        fs_id: 9, // FUSE
        inode: inode_u32,
        parent_cluster: (fh & 0xFFFF_FFFF) as u32, // repurpose: low-32 of fh
        refcount: 1,
        seek_cache_offset: ((fh >> 32) & 0xFFFF_FFFF) as u32, // repurpose: high-32 of fh
        seek_cache_cluster: session_id,
        entry_dirty: false,
    };
    state.open_files[slot_id as usize] = Some(file);
    Ok(slot_id)
}

/// Rekonstruiert die FUSE-`fh` (u64) aus einem OpenFile-Slot.
fn fuse_fh_of(file: &OpenFile) -> u64 {
    ((file.seek_cache_offset as u64) << 32) | (file.parent_cluster as u64)
}

/// Liefert `(session_id, session)` für eine OpenFile mit `fs_id == 9`.
fn fuse_session_of(file: &OpenFile) -> Result<(u32, Arc<crate::fs::fuse::FuseSession>), FsError> {
    let sid = file.seek_cache_cluster;
    let s = crate::fs::fuse::session(sid).ok_or(FsError::NotFound)?;
    Ok((sid, s))
}
