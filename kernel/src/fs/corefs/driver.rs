//! CoreFS-Treiber, der den VFS-[`Filesystem`]-Trait über `corefs_core` erfüllt.
//!
//! # Status: Read + Append-Only Write-Subset
//!
//! Der Treiber hydriert beim Mount einen [`PersistedState`] aus dem Volume
//! via [`corefs_core::storage::ondisk::native::load_state_native`] und hält
//! ihn unter einem [`crate::sync::mutex::Mutex`]. Lesende Operationen
//! (`read`, `lookup`, `readdir`) laufen direkt gegen den in-memory
//! `PersistedState`; schreibende Operationen mutieren ihn, und [`flush`]
//! schreibt alle gesammelten Änderungen atomar via
//! [`corefs_core::storage::ondisk::native::save_state_native`] zurück.
//!
//! ## Schreibpfad — Scope
//!
//! - `create(Regular | Directory)` legt einen neuen Inode an (leerer
//!   `BlockRecord` für Files). `create(Device)` → `PermissionDenied`.
//! - `write` hängt Bytes an den `BlockRecord` des Files an oder erzeugt
//!   einen neuen, wenn noch keiner existiert. Overwrites mitten in der
//!   Datei werden unterstützt (byte-precise).
//! - `delete(name)` verschiebt den betroffenen Inode nach `deleted_inodes`
//!   und entfernt seinen `BlockRecord`.
//!
//! Nicht unterstützt (würden den Service-Layer + BlockStore-Owner brauchen):
//! - Snapshots / Versioning / Sync-Status-Manipulation
//! - Quota / Security (ACL)-Änderungen
//! - Rename, chmod, chown
//!
//! ## Inode-Mapping
//!
//! Auf VFS-Ebene exponieren wir den CoreFS-Slot (0-basiert, Truncation auf
//! u32). Intern arbeiten wir aber auf `InodeId` (domain-level Identität).
//! Ein Hilfs-HashMap (`slot_to_id`) wird lazily beim ersten schreibenden
//! Zugriff aufgebaut.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use corefs_core::domain::inode::{Inode, InodeId, InodeKind};
use corefs_core::domain::metadata::FileMetadata;
use corefs_core::domain::volume::VolumeDescriptor;
use corefs_core::error::CoreFsError;
use corefs_core::platform::Timestamp;
use corefs_core::services::journal::JournalRuntimeState;
use corefs_core::storage::block_device::BlockDevice;
use corefs_core::storage::block_store::{AllocatorPolicy, BlockRecord};
use corefs_core::storage::ondisk::native::{load_state_native, save_state_native};
use corefs_core::storage::persisted_state::PersistedState;
use corefs_core::config::CoreFsConfig;

use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::{Filesystem, FsError};
use crate::sync::mutex::Mutex;

use super::block_device::BlockDeviceAdapter;

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Bildet einen [`CoreFsError`] auf den AnyOS-VFS-[`FsError`] ab.
pub fn corefs_to_fs_error(e: &CoreFsError) -> FsError {
    match e {
        CoreFsError::NotFound(_) => FsError::NotFound,
        CoreFsError::InvalidInput(_) => FsError::InvalidPath,
        CoreFsError::PolicyViolation(_) => FsError::PermissionDenied,
        CoreFsError::AlreadyExists(_) => FsError::AlreadyExists,
        _ => FsError::IoError,
    }
}

/// Mappt CoreFS-`InodeKind` → AnyOS-`FileType`.
fn kind_to_file_type(k: InodeKind) -> FileType {
    match k {
        InodeKind::File => FileType::Regular,
        InodeKind::Directory => FileType::Directory,
        InodeKind::Symlink => FileType::Regular,
    }
}

/// Konstruiert einen frischen, leeren `PersistedState` — passend zum
/// Ergebnis von `format_device` + `save_state_native(&empty_state)`.
///
/// Pendant zu den `empty_state()`-Helfern aus den `corefs-core`-Tests.
pub fn empty_persisted_state() -> PersistedState {
    let config = CoreFsConfig::default();
    let volume = VolumeDescriptor::from_config_at(&config, Timestamp::EPOCH);
    PersistedState {
        config,
        volume,
        clean_unmount: true,
        pending_wal: None,
        active_inodes: Vec::new(),
        deleted_inodes: Vec::new(),
        allocator_policy: AllocatorPolicy::default(),
        free_extents: Vec::new(),
        hot_path_records: Vec::new(),
        block_records: Vec::new(),
        journal_entries: Vec::new(),
        journal_runtime: JournalRuntimeState::default(),
        versions: Vec::new(),
        sync_statuses: Vec::new(),
        snapshots: Vec::new(),
        next_snapshot_id: 0,
    }
}

// ---------------------------------------------------------------------------
// CoreFsDriver
// ---------------------------------------------------------------------------

/// Treiber für ein gemountetes CoreFS-Volume.
///
/// Hält den hydratisierten [`PersistedState`] im Speicher. Schreibende
/// `Filesystem`-Aufrufe mutieren den State; Aufrufe an [`CoreFsDriver::flush`]
/// persistieren via `save_state_native` atomar auf das Device.
pub struct CoreFsDriver {
    inner: Mutex<Inner>,
}

struct Inner {
    device: BlockDeviceAdapter,
    state: PersistedState,
    /// Laufender Zähler für neu zu vergebende `InodeId`s. Startet bei einem
    /// Wert oberhalb aller im State bekannten Ids, damit keine Kollision
    /// entsteht.
    next_id: u64,
    read_only: bool,
}

impl CoreFsDriver {
    /// Öffnet ein vorhandenes CoreFS-Volume read-only.
    ///
    /// Schreibende `Filesystem`-Operationen geben [`FsError::PermissionDenied`]
    /// zurück; `flush` ist ein No-op.
    pub fn mount_read_only(device: BlockDeviceAdapter) -> Result<Self, FsError> {
        let state = load_state_native(&device).map_err(|e| corefs_to_fs_error(&e))?;
        let next_id = compute_next_id(&state);
        Ok(Self {
            inner: Mutex::new(Inner {
                device,
                state,
                next_id,
                read_only: true,
            }),
        })
    }

    /// Öffnet ein vorhandenes CoreFS-Volume mit Schreibzugriff.
    ///
    /// Erfordert, dass das Volume bereits im `LAYOUT_MODE_NATIVE` steht
    /// (d.h. mindestens einmal mit `save_state_native` beschrieben wurde).
    /// Ein frisch formatiertes Volume muss vorher initialisiert werden —
    /// Tests nutzen dazu [`empty_persisted_state`] + `save_state_native`.
    pub fn mount_writable(device: BlockDeviceAdapter) -> Result<Self, FsError> {
        let state = load_state_native(&device).map_err(|e| corefs_to_fs_error(&e))?;
        let next_id = compute_next_id(&state);
        Ok(Self {
            inner: Mutex::new(Inner {
                device,
                state,
                next_id,
                read_only: false,
            }),
        })
    }

    /// Commitet alle bisher gesammelten Mutationen atomar auf das Device.
    ///
    /// Nicht Teil des [`Filesystem`]-Traits — Aufrufer (Unmount-Hook,
    /// Shutdown-Sync) rufen diese Methode gezielt auf.
    pub fn flush(&self) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        if inner.read_only {
            return Ok(());
        }
        let Inner { device, state, .. } = &mut *inner;
        save_state_native(device, state).map_err(|e| corefs_to_fs_error(&e))?;
        Ok(())
    }

    /// Erwartbarer Inode-Mapping-Helper (u32-Truncate des u64-InodeId-Wertes).
    fn inode_u32_from_id(id: InodeId) -> u32 {
        id.0 as u32
    }
}

fn compute_next_id(state: &PersistedState) -> u64 {
    let mut max_id: u64 = 0;
    for i in &state.active_inodes {
        if i.id.0 > max_id {
            max_id = i.id.0;
        }
    }
    for i in &state.deleted_inodes {
        if i.id.0 > max_id {
            max_id = i.id.0;
        }
    }
    max_id + 1
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn parent_of(path: &str) -> &str {
    // Splits off the last path segment. Root (`/`) has no parent.
    if path == "/" || path.is_empty() {
        return "/";
    }
    match path.rfind('/') {
        Some(0) => "/",
        Some(idx) => &path[..idx],
        None => "/",
    }
}

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        let mut s = String::from("/");
        s.push_str(name);
        s
    } else {
        let mut s = String::from(parent);
        s.push('/');
        s.push_str(name);
        s
    }
}

// ---------------------------------------------------------------------------
// Filesystem trait implementation
// ---------------------------------------------------------------------------

impl Filesystem for CoreFsDriver {
    fn read(&self, inode: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        // Find BlockRecord by inode.id truncated to u32.
        for rec in &inner.state.block_records {
            if CoreFsDriver::inode_u32_from_id(rec.inode) == inode {
                let off = offset as usize;
                if off >= rec.bytes.len() {
                    return Ok(0);
                }
                let n = core::cmp::min(rec.bytes.len() - off, buf.len());
                buf[..n].copy_from_slice(&rec.bytes[off..off + n]);
                return Ok(n);
            }
        }
        // Inode exists but has no block record → 0 bytes (empty file).
        for i in &inner.state.active_inodes {
            if CoreFsDriver::inode_u32_from_id(i.id) == inode {
                return Ok(0);
            }
        }
        Err(FsError::NotFound)
    }

    fn write(&self, inode: u32, offset: u32, buf: &[u8]) -> Result<usize, FsError> {
        let mut inner = self.inner.lock();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }
        // Locate the active inode first — directories cannot hold bytes.
        let mut inode_id: Option<InodeId> = None;
        for i in &inner.state.active_inodes {
            if CoreFsDriver::inode_u32_from_id(i.id) == inode {
                if !matches!(i.kind, InodeKind::File) {
                    return Err(FsError::IsADirectory);
                }
                inode_id = Some(i.id);
                break;
            }
        }
        let id = inode_id.ok_or(FsError::NotFound)?;

        // Append/overwrite into BlockRecord.
        let off = offset as usize;
        let end = off + buf.len();
        let mut found = false;
        for rec in inner.state.block_records.iter_mut() {
            if rec.inode == id {
                if rec.bytes.len() < end {
                    rec.bytes.resize(end, 0);
                }
                rec.bytes[off..end].copy_from_slice(buf);
                found = true;
                break;
            }
        }
        if !found {
            let mut bytes: Vec<u8> = Vec::new();
            bytes.resize(end, 0);
            bytes[off..end].copy_from_slice(buf);
            inner.state.block_records.push(BlockRecord {
                inode: id,
                bytes,
                checksum: 0,
                device_block: 0,
                allocated_blocks: 0,
            });
        }

        // Update inode size field to reflect highest written offset.
        for i in inner.state.active_inodes.iter_mut() {
            if i.id == id {
                if end > i.size {
                    i.size = end;
                }
                i.touch_modified_at(Timestamp::EPOCH);
                break;
            }
        }

        Ok(buf.len())
    }

    fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        if path.is_empty() {
            return Err(FsError::InvalidPath);
        }
        let inner = self.inner.lock();
        for i in &inner.state.active_inodes {
            if i.path == path {
                return Ok((
                    CoreFsDriver::inode_u32_from_id(i.id),
                    kind_to_file_type(i.kind),
                    i.size as u32,
                ));
            }
        }
        Err(FsError::NotFound)
    }

    fn readdir(&self, inode: u32) -> Result<Vec<DirEntry>, FsError> {
        let inner = self.inner.lock();
        // Find the parent path.
        let parent_path = inner
            .state
            .active_inodes
            .iter()
            .find(|i| CoreFsDriver::inode_u32_from_id(i.id) == inode)
            .map(|i| i.path.clone())
            .ok_or(FsError::NotFound)?;
        let prefix = if parent_path == "/" {
            "/".to_string()
        } else {
            let mut p = parent_path.clone();
            p.push('/');
            p
        };
        let mut entries: Vec<DirEntry> = Vec::new();
        for i in &inner.state.active_inodes {
            if i.path == parent_path {
                continue;
            }
            if !i.path.starts_with(&prefix) {
                continue;
            }
            let rest = &i.path[prefix.len()..];
            if rest.is_empty() || rest.contains('/') {
                continue;
            }
            entries.push(DirEntry {
                name: rest.to_string(),
                file_type: kind_to_file_type(i.kind),
                size: i.size as u32,
                is_symlink: matches!(i.kind, InodeKind::Symlink),
                uid: i.metadata.uid as u16,
                gid: i.metadata.gid as u16,
                mode: i.metadata.mode as u16,
            });
        }
        Ok(entries)
    }

    fn create(
        &self,
        parent_inode: u32,
        name: &str,
        file_type: FileType,
    ) -> Result<u32, FsError> {
        if name.is_empty() || name.contains('/') {
            return Err(FsError::InvalidPath);
        }
        let kind = match file_type {
            FileType::Regular => InodeKind::File,
            FileType::Directory => InodeKind::Directory,
            // Device nodes are not modelled by CoreFS inodes.
            FileType::Device => return Err(FsError::PermissionDenied),
        };

        let mut inner = self.inner.lock();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }

        // Resolve parent path.
        let parent_path = inner
            .state
            .active_inodes
            .iter()
            .find(|i| CoreFsDriver::inode_u32_from_id(i.id) == parent_inode)
            .map(|i| i.path.clone())
            .ok_or(FsError::NotFound)?;
        let new_path = join_path(&parent_path, name);

        // Reject duplicates.
        if inner.state.active_inodes.iter().any(|i| i.path == new_path) {
            return Err(FsError::AlreadyExists);
        }

        let id = InodeId(inner.next_id);
        inner.next_id += 1;
        let inode = Inode::new_at(id, kind, new_path, FileMetadata::default(), Timestamp::EPOCH);
        inner.state.active_inodes.push(inode);
        Ok(CoreFsDriver::inode_u32_from_id(id))
    }

    fn delete(&self, parent_inode: u32, name: &str) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }
        let parent_path = inner
            .state
            .active_inodes
            .iter()
            .find(|i| CoreFsDriver::inode_u32_from_id(i.id) == parent_inode)
            .map(|i| i.path.clone())
            .ok_or(FsError::NotFound)?;
        let target = join_path(&parent_path, name);

        let pos = inner
            .state
            .active_inodes
            .iter()
            .position(|i| i.path == target)
            .ok_or(FsError::NotFound)?;

        let removed = inner.state.active_inodes.remove(pos);
        let rid = removed.id;
        inner.state.deleted_inodes.push(removed);
        inner.state.block_records.retain(|rec| rec.inode != rid);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use corefs_core::storage::ondisk::volume::{FormatOptions, format_device};
    use corefs_core::error::CoreFsError;
    use crate::fs::corefs::block_device::MemSectorIo;

    fn build_native_adapter(part_sectors: u64) -> BlockDeviceAdapter {
        // Build an adapter backed by a single shared MemSectorIo, format it,
        // and drop an empty NATIVE PersistedState on it.
        let io = Box::new(MemSectorIo::new(part_sectors + 16));
        let mut adapter = BlockDeviceAdapter::with_io(
            0,
            8, /* partition offset */
            part_sectors,
            false,
            io,
        )
        .unwrap();

        let opts = FormatOptions {
            label: alloc::string::String::from("corefs"),
            uuid: *b"ANYOSCOREFSTEST1",
            inode_count: 256,
            journal_blocks: 8,
        };
        format_device(&mut adapter, &opts).expect("format_device");
        save_state_native(&mut adapter, &empty_persisted_state()).expect("save empty state");
        adapter
    }

    #[test]
    fn corefs_not_found_maps_to_fs_not_found() {
        let e = CoreFsError::NotFound(format!("x"));
        assert!(matches!(corefs_to_fs_error(&e), FsError::NotFound));
    }

    #[test]
    fn corefs_policy_maps_to_permission_denied() {
        let e = CoreFsError::PolicyViolation(format!("ro"));
        assert!(matches!(corefs_to_fs_error(&e), FsError::PermissionDenied));
    }

    #[test]
    fn corefs_invalid_input_maps_to_invalid_path() {
        let e = CoreFsError::InvalidInput(format!("bad"));
        assert!(matches!(corefs_to_fs_error(&e), FsError::InvalidPath));
    }

    #[test]
    fn corefs_already_exists_maps() {
        let e = CoreFsError::AlreadyExists(format!("dup"));
        assert!(matches!(corefs_to_fs_error(&e), FsError::AlreadyExists));
    }

    #[test]
    fn kind_file_maps_regular() {
        assert!(matches!(kind_to_file_type(InodeKind::File), FileType::Regular));
    }

    #[test]
    fn kind_directory_maps_directory() {
        assert!(matches!(
            kind_to_file_type(InodeKind::Directory),
            FileType::Directory
        ));
    }

    #[test]
    fn kind_symlink_falls_back_to_regular() {
        assert!(matches!(
            kind_to_file_type(InodeKind::Symlink),
            FileType::Regular
        ));
    }

    #[test]
    fn parent_of_root_is_root() {
        assert_eq!(parent_of("/"), "/");
    }

    #[test]
    fn parent_of_top_level_is_root() {
        assert_eq!(parent_of("/foo"), "/");
    }

    #[test]
    fn parent_of_nested_strips_last_segment() {
        assert_eq!(parent_of("/a/b/c"), "/a/b");
    }

    #[test]
    fn join_path_root_adds_single_slash() {
        assert_eq!(join_path("/", "foo"), "/foo");
    }

    #[test]
    fn join_path_nested() {
        assert_eq!(join_path("/a/b", "c"), "/a/b/c");
    }

    #[test]
    fn empty_persisted_state_has_no_inodes() {
        let s = empty_persisted_state();
        assert!(s.active_inodes.is_empty());
        assert!(s.deleted_inodes.is_empty());
        assert!(s.block_records.is_empty());
    }

    #[test]
    fn compute_next_id_with_empty_state_is_one() {
        let s = empty_persisted_state();
        assert_eq!(compute_next_id(&s), 1);
    }

    #[test]
    fn writable_mount_on_formatted_volume_succeeds() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount_writable");
        // Empty volume — no files, no root inode populated yet.
        let inner = driver.inner.lock();
        assert!(inner.state.active_inodes.is_empty());
    }

    #[test]
    fn create_regular_file_and_read_back() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        // Seed root directory manually so we have a parent for create().
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                alloc::string::String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root_inode = driver.lookup("/").expect("root").0;
        let file_inode = driver
            .create(root_inode, "hello.txt", FileType::Regular)
            .expect("create");
        let payload = b"hello corefs!";
        assert_eq!(driver.write(file_inode, 0, payload).unwrap(), payload.len());

        let mut buf = [0u8; 64];
        let n = driver.read(file_inode, 0, &mut buf).unwrap();
        assert_eq!(n, payload.len());
        assert_eq!(&buf[..n], payload);
    }

    #[test]
    fn create_duplicate_fails() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                alloc::string::String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root = driver.lookup("/").unwrap().0;
        driver.create(root, "x", FileType::Regular).unwrap();
        let err = driver.create(root, "x", FileType::Regular).unwrap_err();
        assert!(matches!(err, FsError::AlreadyExists));
    }

    #[test]
    fn create_device_returns_permission_denied() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                alloc::string::String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root = driver.lookup("/").unwrap().0;
        let err = driver.create(root, "tty", FileType::Device).unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied));
    }

    #[test]
    fn delete_moves_inode_and_drops_content() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                alloc::string::String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root = driver.lookup("/").unwrap().0;
        let f = driver.create(root, "doomed", FileType::Regular).unwrap();
        driver.write(f, 0, b"bye").unwrap();

        driver.delete(root, "doomed").unwrap();
        assert!(matches!(driver.lookup("/doomed").unwrap_err(), FsError::NotFound));

        let inner = driver.inner.lock();
        assert_eq!(inner.state.deleted_inodes.len(), 1);
        assert!(inner.state.block_records.is_empty());
    }

    #[test]
    fn readdir_lists_only_direct_children() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                alloc::string::String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root = driver.lookup("/").unwrap().0;
        let d = driver.create(root, "sub", FileType::Directory).unwrap();
        driver.create(root, "top.txt", FileType::Regular).unwrap();
        driver.create(d, "nested.txt", FileType::Regular).unwrap();

        let entries = driver.readdir(root).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
        assert!(names.contains(&"sub".to_string()));
        assert!(names.contains(&"top.txt".to_string()));
        assert!(!names.contains(&"nested.txt".to_string()));
    }

    #[test]
    fn read_only_mount_blocks_mutations() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_read_only(adapter).expect("mount_ro");
        let err = driver.create(1, "x", FileType::Regular).unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied));
        let err = driver.write(1, 0, b"nope").unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied));
        // flush() is a no-op on read-only mounts.
        driver.flush().unwrap();
    }

    #[test]
    fn write_extends_size_on_append() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                alloc::string::String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root = driver.lookup("/").unwrap().0;
        let f = driver.create(root, "grow.log", FileType::Regular).unwrap();

        // Three successive appends: 0..4, 4..8, 8..12.
        driver.write(f, 0, b"AAAA").unwrap();
        driver.write(f, 4, b"BBBB").unwrap();
        driver.write(f, 8, b"CCCC").unwrap();

        let (_ino, _ft, size) = driver.lookup("/grow.log").unwrap();
        assert_eq!(size, 12);

        let mut buf = [0u8; 16];
        let n = driver.read(f, 0, &mut buf).unwrap();
        assert_eq!(n, 12);
        assert_eq!(&buf[..12], b"AAAABBBBCCCC");
    }

    #[test]
    fn overlapping_write_overwrites_bytes() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                alloc::string::String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root = driver.lookup("/").unwrap().0;
        let f = driver.create(root, "ov.bin", FileType::Regular).unwrap();
        driver.write(f, 0, b"0123456789").unwrap();
        // Overlapping overwrite in the middle.
        driver.write(f, 3, b"XYZ").unwrap();

        let mut buf = [0u8; 16];
        let n = driver.read(f, 0, &mut buf).unwrap();
        assert_eq!(n, 10);
        assert_eq!(&buf[..10], b"012XYZ6789");
    }

    #[test]
    fn write_to_directory_is_rejected() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                alloc::string::String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root = driver.lookup("/").unwrap().0;
        let d = driver.create(root, "adir", FileType::Directory).unwrap();
        let err = driver.write(d, 0, b"nope").unwrap_err();
        assert!(matches!(err, FsError::IsADirectory));
    }

    #[test]
    fn read_from_unknown_inode_is_not_found() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let mut buf = [0u8; 4];
        let err = driver.read(42_000_000, 0, &mut buf).unwrap_err();
        assert!(matches!(err, FsError::NotFound));
    }

    #[test]
    fn write_then_flush_then_remount_roundtrips_data() {
        let adapter = build_native_adapter(4096);
        // Extract the underlying MemSectorIo so we can build a second adapter
        // over the same backing memory. MemSectorIo doesn't expose that
        // directly, so we drive this through the same adapter — flush then
        // remount_writable to the same adapter.
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                alloc::string::String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root = driver.lookup("/").unwrap().0;
        let f = driver.create(root, "persist.txt", FileType::Regular).unwrap();
        driver.write(f, 0, b"persistent").unwrap();
        driver.flush().expect("flush");

        // Re-hydrate state from the persisted device.
        let inner = driver.inner.lock();
        let device_ref: &BlockDeviceAdapter = &inner.device;
        let reloaded = load_state_native(device_ref).expect("reload");
        assert!(reloaded.active_inodes.iter().any(|i| i.path == "/persist.txt"));
        assert!(reloaded
            .block_records
            .iter()
            .any(|r| r.bytes == b"persistent"));
    }
}
