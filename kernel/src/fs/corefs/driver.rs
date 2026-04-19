//! CoreFS-Treiber, der den VFS-[`Filesystem`]-Trait über `corefs_core` erfüllt.
//!
//! # Status: Kernel-nativer Read/Write-Treiber
//!
//! Der Treiber hydriert beim Mount einen [`PersistedState`] aus dem Volume
//! via [`corefs_core::storage::ondisk::native::load_state_native`] und hält
//! ihn unter einem [`crate::sync::mutex::Mutex`]. Lesende Operationen
//! (`read`, `lookup`, `readdir`) laufen direkt gegen den in-memory
//! `PersistedState`; schreibende Operationen mutieren ihn, und [`flush`]
//! schreibt geänderte Metadaten und Inode-Payloads inkrementell via
//! [`corefs_core::storage::ondisk::native::save_state_native_incremental`]
//! zurück.
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
//!
//! ## Inode-Mapping
//!
//! Auf VFS-Ebene exponieren wir den CoreFS-Slot (0-basiert, Truncation auf
//! u32). Intern arbeiten wir aber auf `InodeId` (domain-level Identität).
//! Mount-seitige BTree-Indizes halten Pfad-, VFS-Inode-, CoreFS-Inode- und
//! Parent/Child-Lookups schnell, ohne Hot-Path-Scans über alle Inodes.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use corefs_core::config::CoreFsConfig;
use corefs_core::domain::inode::{Inode, InodeId, InodeKind};
use corefs_core::domain::metadata::FileMetadata;
use corefs_core::domain::volume::VolumeDescriptor;
use corefs_core::error::CoreFsError;
use corefs_core::platform::Timestamp;
use corefs_core::services::journal::JournalRuntimeState;
use corefs_core::storage::block_device::BlockDevice;
use corefs_core::storage::block_store::{AllocatorPolicy, BlockStore};
use corefs_core::storage::ondisk::layout::BLOCK_SIZE;
#[cfg(test)]
use corefs_core::storage::ondisk::native::save_state_native;
use corefs_core::storage::ondisk::native::{load_state_native, save_state_native_incremental};
use corefs_core::storage::persisted_state::PersistedState;

use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::{Filesystem, FsError, StatFs, StatResult};
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
/// persistieren inkrementell und atomar auf das Device.
pub struct CoreFsDriver {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct InodeIndexes {
    path_to_id: BTreeMap<String, InodeId>,
    vfs_to_id: BTreeMap<u32, InodeId>,
    id_to_pos: BTreeMap<u64, usize>,
    children_by_parent: BTreeMap<u64, Vec<InodeId>>,
}

impl InodeIndexes {
    fn rebuild(state: &PersistedState) -> Self {
        let mut indexes = Self::default();
        for (pos, inode) in state.active_inodes.iter().enumerate() {
            indexes.path_to_id.insert(inode.path.clone(), inode.id);
            indexes
                .vfs_to_id
                .insert(CoreFsDriver::inode_u32_from_id(inode.id), inode.id);
            indexes.id_to_pos.insert(inode.id.0, pos);
        }
        for inode in &state.active_inodes {
            if inode.path == "/" {
                continue;
            }
            if let Some(parent_id) = indexes.path_to_id.get(parent_of(&inode.path)) {
                indexes
                    .children_by_parent
                    .entry(parent_id.0)
                    .or_default()
                    .push(inode.id);
            }
        }
        indexes
    }
}

struct Inner {
    device: BlockDeviceAdapter,
    state: PersistedState,
    /// Device-backed block store — actual file bytes are written to / read
    /// from `device` on every I/O call.  No in-memory byte buffer.
    blocks: BlockStore,
    /// Laufender Zähler für neu zu vergebende `InodeId`s.
    next_id: u64,
    read_only: bool,
    /// Volume-Kapazität in Blöcken, einmalig beim Mount aus der
    /// Device-Geometrie berechnet. Ermöglicht `statfs()` ohne Disk-I/O.
    total_blocks: u64,
    /// Mount-seitige Inode-Indizes für O(log n)-Lookup und O(children)-Readdir.
    indexes: InodeIndexes,
    /// Metadaten oder BlockRecord-Sicht haben sich seit dem letzten Flush geändert.
    dirty: bool,
}

impl Inner {
    fn new(
        device: BlockDeviceAdapter,
        state: PersistedState,
        blocks: BlockStore,
        next_id: u64,
        read_only: bool,
        total_blocks: u64,
        dirty: bool,
    ) -> Self {
        let indexes = InodeIndexes::rebuild(&state);
        Self {
            device,
            state,
            blocks,
            next_id,
            read_only,
            total_blocks,
            indexes,
            dirty,
        }
    }

    fn rebuild_indexes(&mut self) {
        self.indexes = InodeIndexes::rebuild(&self.state);
    }

    #[cfg(test)]
    fn refresh_indexes_for_tests(&mut self) {
        // Unit tests seed `active_inodes` directly through `driver.inner`.
        // Production code mutates through driver methods and updates indexes
        // incrementally.
        self.rebuild_indexes();
    }

    #[cfg(not(test))]
    fn refresh_indexes_for_tests(&mut self) {}

    fn inode_id_for_vfs(&self, inode: u32) -> Option<InodeId> {
        self.indexes.vfs_to_id.get(&inode).copied()
    }

    fn inode_by_id(&self, id: InodeId) -> Option<&Inode> {
        self.indexes
            .id_to_pos
            .get(&id.0)
            .and_then(|pos| self.state.active_inodes.get(*pos))
    }

    fn inode_mut_by_id(&mut self, id: InodeId) -> Option<&mut Inode> {
        let pos = *self.indexes.id_to_pos.get(&id.0)?;
        self.state.active_inodes.get_mut(pos)
    }

    fn inode_id_by_path(&self, path: &str) -> Option<InodeId> {
        self.indexes.path_to_id.get(path).copied()
    }

    fn inode_by_path(&self, path: &str) -> Option<&Inode> {
        self.inode_id_by_path(path)
            .and_then(|id| self.inode_by_id(id))
    }

    fn register_last_inode(&mut self, parent_id: Option<InodeId>) {
        let Some(pos) = self.state.active_inodes.len().checked_sub(1) else {
            return;
        };
        let inode = &self.state.active_inodes[pos];
        self.indexes.path_to_id.insert(inode.path.clone(), inode.id);
        self.indexes
            .vfs_to_id
            .insert(CoreFsDriver::inode_u32_from_id(inode.id), inode.id);
        self.indexes.id_to_pos.insert(inode.id.0, pos);
        if let Some(parent_id) = parent_id {
            self.indexes
                .children_by_parent
                .entry(parent_id.0)
                .or_default()
                .push(inode.id);
        }
    }
}

impl CoreFsDriver {
    /// Öffnet ein vorhandenes CoreFS-Volume read-only.
    ///
    /// Schreibende `Filesystem`-Operationen geben [`FsError::PermissionDenied`]
    /// zurück; `flush` ist ein No-op.
    pub fn mount_read_only(device: BlockDeviceAdapter) -> Result<Self, FsError> {
        let mut state = load_state_native(&device).map_err(|e| corefs_to_fs_error(&e))?;
        // Frisch formatierte Volumes haben keinen Root-Inode. In-Memory
        // synthesieren, damit lookup("/") / readdir funktionieren.
        ensure_root_directory(&mut state);
        let next_id = compute_next_id(&state);
        let first_data_block = compute_first_data_block(&state);
        let blocks = BlockStore::from_records_with_allocator_and_start(
            state.block_records.clone(),
            BLOCK_SIZE as usize,
            state.allocator_policy.clone(),
            state.free_extents.clone(),
            first_data_block,
        );
        let total_blocks = device.capacity() / BLOCK_SIZE;
        Ok(Self {
            inner: Mutex::new(Inner::new(
                device,
                state,
                blocks,
                next_id,
                true,
                total_blocks,
                false,
            )),
        })
    }

    /// Öffnet ein vorhandenes CoreFS-Volume mit Schreibzugriff.
    ///
    /// Erfordert, dass das Volume bereits im `LAYOUT_MODE_NATIVE` steht
    /// (d.h. mindestens einmal mit `save_state_native` beschrieben wurde).
    /// Ein frisch formatiertes Volume muss vorher initialisiert werden —
    /// Tests nutzen dazu [`empty_persisted_state`] + `save_state_native`.
    pub fn mount_writable(device: BlockDeviceAdapter) -> Result<Self, FsError> {
        let mut state = load_state_native(&device).map_err(|e| corefs_to_fs_error(&e))?;
        let mut needs_persist = false;

        // Unclean-Mount-Recovery: Falls der vorherige Unmount nicht clean
        // war und eine pending WAL vorliegt, spielen wir die strukturellen
        // Operationen (Create/Delete/Rename/Truncate-size) direkt hier zurück.
        // Block-Level-Ops (PatchExtent, Truncate-data) bleiben der späteren
        // App-Schicht vorbehalten und werden via `skipped_data_ops` gemeldet.
        if !state.clean_unmount && state.pending_wal.is_some() {
            let now = {
                use corefs_core::platform::Clock;
                super::KernelClock.now()
            };
            match state.replay_pending_wal(now) {
                Ok(report) => {
                    crate::serial_println!(
                        "[corefs] unclean mount recovery: structural={} skipped_data={} txn={:?}",
                        report.applied_structural,
                        report.skipped_data_ops,
                        report.transaction_id
                    );
                    needs_persist = true;
                }
                Err(e) => {
                    crate::serial_println!("[corefs] unclean mount recovery failed: {:?}", e);
                }
            }
            state.clean_unmount = true;
        }

        // Frisch formatierte Volumes haben keinen Root-Inode — beim ersten
        // Mount anlegen und direkt persistieren, damit auch Read-Only-Mounts
        // derselben Disk danach funktionieren.
        let root_added = ensure_root_directory(&mut state);
        needs_persist |= root_added;

        let next_id = compute_next_id(&state);
        let first_data_block = compute_first_data_block(&state);
        let blocks = BlockStore::from_records_with_allocator_and_start(
            state.block_records.clone(),
            BLOCK_SIZE as usize,
            state.allocator_policy.clone(),
            state.free_extents.clone(),
            first_data_block,
        );
        let total_blocks = device.capacity() / BLOCK_SIZE;
        let this = Self {
            inner: Mutex::new(Inner::new(
                device,
                state,
                blocks,
                next_id,
                false,
                total_blocks,
                needs_persist,
            )),
        };
        if needs_persist {
            this.flush()?;
        }
        Ok(this)
    }

    /// Liefert (total_bytes, used_bytes, free_bytes) für `df` / VFS-`statfs`.
    ///
    /// Antwortet ohne Disk-I/O aus dem in-memory `BlockStore`:
    ///   - `total` kommt aus der beim Mount gecachten Volume-Kapazität.
    ///   - `used` ist die Summe aller Extents über die bekannten `BlockRecord`s
    ///     (tatsächlich an Files gebundene Blöcke).
    ///   - `free` = `total - used`.
    ///
    /// Wir zählen hier bewusst NICHT `fragmentation_report().total_free_blocks`,
    /// weil dieser Report nur explizit freigegebene Extents führt — bei frisch
    /// formatierten Volumes ohne Deallokationen sind alle freien Blöcke
    /// implizit oberhalb des Allokator-Wasserstands und fehlen dort.
    ///
    /// Vorher rief diese Methode `corefs_core::volume::inspect()` auf, das
    /// drei Superblöcke (Primary, Tertiary, Secondary am Volume-Ende) frisch
    /// las — der Secondary-Read konnte am Partitions-Ende einen AHCI-Timeout
    /// provozieren und `df` um 10 s verlangsamen.
    pub fn statfs(&self) -> Result<(u64, u64, u64), FsError> {
        let inner = self.inner.lock();
        let used_blocks: u64 = inner
            .blocks
            .records()
            .iter()
            .map(|r| r.total_blocks())
            .sum();
        let total = inner.total_blocks.saturating_mul(BLOCK_SIZE);
        let used = used_blocks.saturating_mul(BLOCK_SIZE);
        let free = total.saturating_sub(used);
        Ok((total, used, free))
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
        if !inner.dirty {
            return Ok(());
        }

        // Sync metadata from BlockStore into state, then persist.
        // File bytes are already on the device (written by write_at / write_device).
        let Inner {
            device,
            state,
            blocks,
            dirty,
            ..
        } = &mut *inner;
        state.block_records = blocks.records();
        state.free_extents = blocks.free_extents();
        save_state_native_incremental(device, state).map_err(|e| corefs_to_fs_error(&e))?;
        *dirty = false;
        Ok(())
    }

    /// Erwartbarer Inode-Mapping-Helper (u32-Truncate des u64-InodeId-Wertes).
    fn inode_u32_from_id(id: InodeId) -> u32 {
        id.0 as u32
    }

    // -----------------------------------------------------------------
    // Extended driver APIs (truncate / rename / chmod / chown / symlink)
    // -----------------------------------------------------------------

    /// Passt die Grösse einer regulären Datei an. Wird die Datei verkleinert,
    /// werden überzählige Bytes aus dem zugehörigen `BlockRecord` entfernt;
    /// wird sie erweitert, wird mit Null-Bytes aufgefüllt (sparse-äquivalente
    /// Repräsentation — CoreFS hält Dateiinhalte in-memory).
    pub fn truncate_file(&self, inode: u32, new_size: u32) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }
        let id = inner.inode_id_for_vfs(inode).ok_or(FsError::NotFound)?;
        if let Some(i) = inner.inode_by_id(id) {
            if !matches!(i.kind, InodeKind::File) {
                return Err(FsError::IsADirectory);
            }
        }

        let target = new_size as u64;

        // Resize on device: grow with zero-fill or shrink.
        let Inner {
            blocks,
            device,
            state,
            ..
        } = &mut *inner;
        blocks
            .truncate(device, id, target)
            .map_err(|_| FsError::IoError)?;

        if let Some(i) = state.active_inodes.iter_mut().find(|i| i.id == id) {
            i.size = target as usize;
            i.touch_modified_at(Timestamp::EPOCH);
        }
        inner.dirty = true;
        Ok(())
    }

    /// Benennt einen Eintrag im Elternverzeichnis um bzw. verschiebt ihn in
    /// ein anderes Verzeichnis. Die Pfade aller (rekursiv) untergeordneten
    /// Inodes werden entsprechend angepasst.
    pub fn rename_entry(
        &self,
        parent: u32,
        old_name: &str,
        new_parent: u32,
        new_name: &str,
    ) -> Result<(), FsError> {
        if old_name.is_empty()
            || new_name.is_empty()
            || old_name.contains('/')
            || new_name.contains('/')
        {
            return Err(FsError::InvalidPath);
        }
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }

        // Resolve both parent paths.
        let old_parent_id = inner.inode_id_for_vfs(parent).ok_or(FsError::NotFound)?;
        let old_parent_path = inner
            .inode_by_id(old_parent_id)
            .map(|i| i.path.clone())
            .ok_or(FsError::NotFound)?;
        let new_parent_id = inner
            .inode_id_for_vfs(new_parent)
            .ok_or(FsError::NotFound)?;
        let new_parent_path = inner
            .inode_by_id(new_parent_id)
            .map(|i| i.path.clone())
            .ok_or(FsError::NotFound)?;

        let old_path = join_path(&old_parent_path, old_name);
        let new_path = join_path(&new_parent_path, new_name);
        if old_path == new_path {
            return Ok(());
        }

        // Source must exist.
        let src_kind = inner
            .inode_by_path(&old_path)
            .map(|i| i.kind)
            .ok_or(FsError::NotFound)?;

        // POSIX rename-overwrite semantics when the destination exists.
        let dst_info: Option<(InodeId, InodeKind)> =
            inner.inode_by_path(&new_path).map(|i| (i.id, i.kind));
        let target_remove_id: Option<InodeId> = match (src_kind, dst_info) {
            (_, None) => None,
            (InodeKind::File | InodeKind::Symlink, Some((_, InodeKind::Directory))) => {
                return Err(FsError::IsADirectory);
            }
            (InodeKind::Directory, Some((_, InodeKind::File | InodeKind::Symlink))) => {
                return Err(FsError::NotADirectory);
            }
            (
                InodeKind::File | InodeKind::Symlink,
                Some((id, InodeKind::File | InodeKind::Symlink)),
            ) => Some(id),
            (InodeKind::Directory, Some((id, InodeKind::Directory))) => {
                // Destination directory must be empty.
                let non_empty = inner
                    .indexes
                    .children_by_parent
                    .get(&id.0)
                    .map(|children| !children.is_empty())
                    .unwrap_or(false);
                if non_empty {
                    return Err(FsError::DirectoryNotEmpty);
                }
                Some(id)
            }
        };

        // Remove overwrite target, if any.
        if let Some(target_id) = target_remove_id {
            if let Some(idx) = inner.indexes.id_to_pos.get(&target_id.0).copied() {
                let removed = inner.state.active_inodes.remove(idx);
                inner.state.block_records.retain(|r| r.inode != removed.id);
                inner.state.deleted_inodes.push(removed);
                let Inner { blocks, device, .. } = &mut *inner;
                blocks.remove_inode(device, target_id);
                inner.rebuild_indexes();
            }
        }

        // Rewrite matching paths (the source itself + any descendants when
        // it's a directory).
        let mut old_prefix = old_path.clone();
        old_prefix.push('/');
        for i in inner.state.active_inodes.iter_mut() {
            if i.path == old_path {
                i.path = new_path.clone();
                i.touch_changed_at(Timestamp::EPOCH);
            } else if i.path.starts_with(&old_prefix) {
                let suffix = &i.path[old_prefix.len()..];
                let mut rebuilt = new_path.clone();
                rebuilt.push('/');
                rebuilt.push_str(suffix);
                i.path = rebuilt;
                i.touch_changed_at(Timestamp::EPOCH);
            }
        }
        inner.rebuild_indexes();
        inner.dirty = true;
        Ok(())
    }

    /// Setzt die POSIX-Mode-Bits (Permissions + Type-Bits) eines Inodes.
    pub fn set_mode(&self, inode: u32, mode: u32) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }
        let id = inner.inode_id_for_vfs(inode).ok_or(FsError::NotFound)?;
        let i = inner.inode_mut_by_id(id).ok_or(FsError::NotFound)?;
        i.metadata.mode = mode;
        i.touch_changed_at(Timestamp::EPOCH);
        inner.dirty = true;
        Ok(())
    }

    /// Setzt Owner (`uid`, `gid`) eines Inodes.
    pub fn set_owner(&self, inode: u32, uid: u32, gid: u32) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }
        let id = inner.inode_id_for_vfs(inode).ok_or(FsError::NotFound)?;
        let i = inner.inode_mut_by_id(id).ok_or(FsError::NotFound)?;
        i.metadata.uid = uid;
        i.metadata.gid = gid;
        i.touch_changed_at(Timestamp::EPOCH);
        inner.dirty = true;
        Ok(())
    }

    /// Legt einen symbolischen Link im angegebenen Elternverzeichnis an. Das
    /// Ziel wird als UTF-8 im zugehörigen `BlockRecord` abgelegt — so kann
    /// `readlink` das Ziel ohne zusätzliche Metadaten rekonstruieren.
    pub fn create_symlink(&self, parent: u32, name: &str, target: &str) -> Result<u32, FsError> {
        if name.is_empty() || name.contains('/') {
            return Err(FsError::InvalidPath);
        }
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }
        let parent_id = inner.inode_id_for_vfs(parent).ok_or(FsError::NotFound)?;
        let parent_path = inner
            .inode_by_id(parent_id)
            .map(|i| i.path.clone())
            .ok_or(FsError::NotFound)?;
        let new_path = join_path(&parent_path, name);
        if inner.inode_id_by_path(&new_path).is_some() {
            return Err(FsError::AlreadyExists);
        }

        let id = InodeId(inner.next_id);
        inner.next_id += 1;
        let target_bytes = target.as_bytes();
        let mut md = FileMetadata::default();
        md.uid = crate::task::scheduler::current_thread_uid() as u32;
        md.gid = crate::task::scheduler::current_thread_gid() as u32;
        md.mode = 0xF11;
        let mut inode = Inode::new_at(id, InodeKind::Symlink, new_path, md, Timestamp::EPOCH);
        inode.size = target_bytes.len();
        let Inner { blocks, device, .. } = &mut *inner;
        blocks
            .write_device(device, id, target_bytes)
            .map_err(|_| FsError::IoError)?;
        inner.state.active_inodes.push(inode);
        inner.register_last_inode(Some(parent_id));
        inner.dirty = true;
        Ok(CoreFsDriver::inode_u32_from_id(id))
    }
}

/// Stellt sicher, dass der `PersistedState` einen Root-Directory-Inode
/// (`path = "/"`, `InodeKind::Directory`, Mode 0o755) enthält. Liefert
/// `true`, wenn ein Root-Inode neu angelegt wurde.
fn ensure_root_directory(state: &mut PersistedState) -> bool {
    let has_root = state
        .active_inodes
        .iter()
        .any(|i| i.path == "/" && matches!(i.kind, InodeKind::Directory));
    if has_root {
        return false;
    }
    let mut metadata = FileMetadata::default();
    // anyOS permission encoding: owner=RMDC, group=R, other=R.
    metadata.mode = 0xF11;
    let root = Inode::new_at(
        InodeId(1),
        InodeKind::Directory,
        String::from("/"),
        metadata,
        Timestamp::EPOCH,
    );
    state.active_inodes.push(root);
    true
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

/// Berechnet den ersten sicheren physischen Block für Datei-Schreibvorgänge.
/// Ist der erste Block hinter dem Ende aller bekannten Extents,
/// oder 256 (sicher hinter dem ODF-Metadaten-Bereich) — je nachdem, was größer ist.
fn compute_first_data_block(state: &PersistedState) -> u64 {
    let highest = state
        .block_records
        .iter()
        .flat_map(|r| r.extents.iter())
        .map(|e| e.physical_block + u64::from(e.length_blocks))
        .max()
        .unwrap_or(0);
    // 256 blocks × 4096 bytes = 1 MiB — sicher hinter jedem ODF-Metadaten-Bereich.
    highest.max(256)
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

/// Split a path into (parent, name).  Duplicate of the private
/// helper in `fs::vfs::path` so we can use it from within the FS
/// driver without making that module `pub(crate)`.
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
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        // Find the domain InodeId for this VFS inode number.
        let inode_id = inner.inode_id_for_vfs(inode).ok_or(FsError::NotFound)?;
        // Stream bytes from the device via BlockStore.
        let n = inner
            .blocks
            .read_bytes(&inner.device, inode_id, offset as u64, buf)
            .map_err(|_| FsError::IoError)?;
        Ok(n)
    }

    fn write(&self, inode: u32, offset: u32, buf: &[u8]) -> Result<usize, FsError> {
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }
        // Locate the active inode first — directories cannot hold bytes.
        let id = inner.inode_id_for_vfs(inode).ok_or(FsError::NotFound)?;
        if let Some(i) = inner.inode_by_id(id) {
            if !matches!(i.kind, InodeKind::File) {
                return Err(FsError::IsADirectory);
            }
        }

        // Write bytes directly to device (offset write, RMW if partial block).
        let new_end = offset as usize + buf.len();
        {
            let Inner { blocks, device, .. } = &mut *inner;
            blocks
                .write_at(device, id, offset as u64, buf)
                .map_err(|_| FsError::IoError)?;
        }

        // Update inode size field to reflect highest written offset.
        if let Some(i) = inner.inode_mut_by_id(id) {
            if new_end > i.size {
                i.size = new_end;
            }
            i.touch_modified_at(Timestamp::EPOCH);
        }

        inner.dirty = true;
        Ok(buf.len())
    }

    fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        if path.is_empty() {
            return Err(FsError::InvalidPath);
        }
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        let i = inner.inode_by_path(path).ok_or(FsError::NotFound)?;
        Ok((
            CoreFsDriver::inode_u32_from_id(i.id),
            kind_to_file_type(i.kind),
            i.size as u32,
        ))
    }

    fn readdir(&self, inode: u32) -> Result<Vec<DirEntry>, FsError> {
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        // Find the parent path.
        let parent_id = inner.inode_id_for_vfs(inode).ok_or(FsError::NotFound)?;
        let parent = inner.inode_by_id(parent_id).ok_or(FsError::NotFound)?;
        if !matches!(parent.kind, InodeKind::Directory) {
            return Err(FsError::NotADirectory);
        }
        let mut entries: Vec<DirEntry> = Vec::new();
        if let Some(children) = inner.indexes.children_by_parent.get(&parent_id.0) {
            for child_id in children {
                let Some(i) = inner.inode_by_id(*child_id) else {
                    continue;
                };
                let rest = i.path.rsplit('/').next().unwrap_or(i.path.as_str());
                if rest.is_empty() {
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
        }
        Ok(entries)
    }

    fn create(&self, parent_inode: u32, name: &str, file_type: FileType) -> Result<u32, FsError> {
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
        inner.refresh_indexes_for_tests();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }

        // Resolve parent path.
        let parent_id = inner
            .inode_id_for_vfs(parent_inode)
            .ok_or(FsError::NotFound)?;
        let parent_path = inner
            .inode_by_id(parent_id)
            .map(|i| i.path.clone())
            .ok_or(FsError::NotFound)?;
        let new_path = join_path(&parent_path, name);

        // Reject duplicates.
        if inner.inode_id_by_path(&new_path).is_some() {
            return Err(FsError::AlreadyExists);
        }

        let id = InodeId(inner.next_id);
        inner.next_id += 1;
        let mut md = FileMetadata::default();
        md.uid = crate::task::scheduler::current_thread_uid() as u32;
        md.gid = crate::task::scheduler::current_thread_gid() as u32;
        // anyOS permission encoding (see kernel/src/fs/permissions.rs):
        // owner=RMDC, group=R, other=R — owner has full control, everyone
        // else can only read (and, for directories, traverse).
        md.mode = 0xF11;
        let inode = Inode::new_at(id, kind, new_path, md, Timestamp::EPOCH);
        inner.state.active_inodes.push(inode);
        inner.register_last_inode(Some(parent_id));
        inner.dirty = true;
        Ok(CoreFsDriver::inode_u32_from_id(id))
    }

    fn delete(&self, parent_inode: u32, name: &str) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        if inner.read_only {
            return Err(FsError::PermissionDenied);
        }
        let parent_id = inner
            .inode_id_for_vfs(parent_inode)
            .ok_or(FsError::NotFound)?;
        let parent_path = inner
            .inode_by_id(parent_id)
            .map(|i| i.path.clone())
            .ok_or(FsError::NotFound)?;
        let target = join_path(&parent_path, name);

        let target_id = inner.inode_id_by_path(&target).ok_or(FsError::NotFound)?;
        let pos = inner
            .indexes
            .id_to_pos
            .get(&target_id.0)
            .copied()
            .ok_or(FsError::NotFound)?;

        let removed = inner.state.active_inodes.remove(pos);
        let rid = removed.id;
        inner.state.deleted_inodes.push(removed);
        inner.state.block_records.retain(|rec| rec.inode != rid);
        // Release device extents for the deleted inode.
        let Inner { blocks, device, .. } = &mut *inner;
        blocks.remove_inode(device, rid);
        inner.rebuild_indexes();
        inner.dirty = true;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Extended trait methods — mostly forwarding to the inherent
    // helper fns defined earlier in this file.  The shape mismatch
    // between `pub fn xyz(parent, ...)` (parent-inode based) and the
    // trait `pub fn xyz(path, ...)` (path based) is handled here.
    // -----------------------------------------------------------------

    fn stat(&self, path: &str) -> Result<StatResult, FsError> {
        if path.is_empty() {
            return Err(FsError::InvalidPath);
        }
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        let i = inner.inode_by_path(path).ok_or(FsError::NotFound)?;
        Ok(StatResult {
            file_type: kind_to_file_type(i.kind),
            size: i.size as u32,
            is_symlink: matches!(i.kind, InodeKind::Symlink),
            uid: i.metadata.uid as u16,
            gid: i.metadata.gid as u16,
            mode: i.metadata.mode as u16,
            // CoreFS stores Timestamp fields on the inode —
            // project to Unix seconds for the VFS StatResult.
            mtime: i.modified_at.as_secs() as u32,
        })
    }

    fn statfs(&self) -> Result<StatFs, FsError> {
        let (total, used, free) = self.statfs()?;
        Ok(StatFs {
            total_bytes: total,
            used_bytes: used,
            free_bytes: free,
        })
    }

    fn rename(&self, old_path: &str, new_path: &str) -> Result<(), FsError> {
        // Inherent rename_entry takes (parent, old_name, new_parent,
        // new_name) — split each path into those components.  Root is
        // a sentinel; resolve each parent through the active-inodes
        // table to find their inode numbers.
        let (old_parent_path, old_name) = split_parent_name(old_path)?;
        let (new_parent_path, new_name) = split_parent_name(new_path)?;
        let (old_parent, new_parent) = {
            let mut inner = self.inner.lock();
            inner.refresh_indexes_for_tests();
            let old_p = inner
                .inode_by_path(old_parent_path)
                .map(|i| CoreFsDriver::inode_u32_from_id(i.id))
                .ok_or(FsError::NotFound)?;
            let new_p = inner
                .inode_by_path(new_parent_path)
                .map(|i| CoreFsDriver::inode_u32_from_id(i.id))
                .ok_or(FsError::NotFound)?;
            (old_p, new_p)
        };
        self.rename_entry(old_parent, old_name, new_parent, new_name)
    }

    fn truncate(&self, inode: u32, size: u32) -> Result<(), FsError> {
        self.truncate_file(inode, size)
    }

    fn set_mode(&self, inode: u32, mode: u16) -> Result<(), FsError> {
        // Inherent uses u32 (POSIX mode with type bits), trait uses
        // u16 (VFS mode mask).  Sign-extend the u16 into the low
        // 16 bits of a u32 — type bits above the permission bits
        // stay 0, which matches what the VFS sets today.
        self.set_mode(inode, mode as u32)
    }

    fn set_owner(&self, inode: u32, uid: u16, gid: u16) -> Result<(), FsError> {
        self.set_owner(inode, uid as u32, gid as u32)
    }

    fn create_symlink(&self, link_path: &str, target: &str) -> Result<(), FsError> {
        let (parent_path, name) = split_parent_name(link_path)?;
        let parent = {
            let mut inner = self.inner.lock();
            inner.refresh_indexes_for_tests();
            inner
                .inode_by_path(parent_path)
                .map(|i| CoreFsDriver::inode_u32_from_id(i.id))
                .ok_or(FsError::NotFound)?
        };
        // Inherent create_symlink returns the new inode; the trait
        // variant returns unit — discard the handle.
        let _ = self.create_symlink(parent, name, target)?;
        Ok(())
    }

    fn readlink(&self, inode: u32) -> Result<String, FsError> {
        // Symlinks store their target as the inode's byte content
        // (see create_symlink: `blocks.write_device(..., target_bytes)`).
        // Read it back and decode as UTF-8.
        let mut inner = self.inner.lock();
        inner.refresh_indexes_for_tests();
        let inode_id = inner.inode_id_for_vfs(inode).ok_or(FsError::NotFound)?;
        let inode_ref = inner.inode_by_id(inode_id).ok_or(FsError::NotFound)?;
        if !matches!(inode_ref.kind, InodeKind::Symlink) {
            return Err(FsError::InvalidPath);
        }
        let id = inode_ref.id;
        let mut buf = alloc::vec![0u8; inode_ref.size];
        let n = inner
            .blocks
            .read_bytes(&inner.device, id, 0, buf.as_mut_slice())
            .map_err(|_| FsError::IoError)?;
        buf.truncate(n);
        String::from_utf8(buf).map_err(|_| FsError::IoError)
    }

    fn sync(&self) -> Result<(), FsError> {
        self.flush()
    }

    fn read_only(&self) -> bool {
        self.inner.lock().read_only
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    // commit_open_file inherits the default no-op:  CoreFS persists
    // writes immediately via BlockStore::write_at, no dirent fix-up
    // needed at close().
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::corefs::block_device::MemSectorIo;
    use alloc::boxed::Box;
    use corefs_core::error::CoreFsError;
    use corefs_core::storage::ondisk::volume::{format_device, FormatOptions};

    fn build_native_adapter(part_sectors: u64) -> BlockDeviceAdapter {
        // Build an adapter backed by a single shared MemSectorIo, format it,
        // and drop an empty NATIVE PersistedState on it.
        let io = Box::new(MemSectorIo::new(part_sectors + 16));
        let mut adapter =
            BlockDeviceAdapter::with_io(0, 8 /* partition offset */, part_sectors, false, io)
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
        assert!(matches!(
            kind_to_file_type(InodeKind::File),
            FileType::Regular
        ));
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
        assert!(matches!(
            driver.lookup("/doomed").unwrap_err(),
            FsError::NotFound
        ));

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

    // ---------------------------------------------------------------
    // Extended API tests
    // ---------------------------------------------------------------

    fn seed_root(driver: &CoreFsDriver) -> u32 {
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
        drop(inner);
        driver.lookup("/").unwrap().0
    }

    #[test]
    fn truncate_shrinks_file_size_and_bytes() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let f = driver.create(root, "s.bin", FileType::Regular).unwrap();
        driver.write(f, 0, b"0123456789").unwrap();
        driver.truncate_file(f, 4).unwrap();
        let (_, _, size) = driver.lookup("/s.bin").unwrap();
        assert_eq!(size, 4);
        let mut buf = [0u8; 16];
        let n = driver.read(f, 0, &mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&buf[..4], b"0123");
    }

    #[test]
    fn truncate_extends_with_zero_fill() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let f = driver.create(root, "g.bin", FileType::Regular).unwrap();
        driver.write(f, 0, b"AB").unwrap();
        driver.truncate_file(f, 5).unwrap();
        let (_, _, size) = driver.lookup("/g.bin").unwrap();
        assert_eq!(size, 5);
        let mut buf = [0u8; 8];
        let n = driver.read(f, 0, &mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], b"AB\0\0\0");
    }

    #[test]
    fn truncate_on_directory_is_rejected() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let d = driver.create(root, "dir", FileType::Directory).unwrap();
        let err = driver.truncate_file(d, 0).unwrap_err();
        assert!(matches!(err, FsError::IsADirectory));
    }

    #[test]
    fn rename_within_same_parent_updates_path() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let f = driver.create(root, "a.txt", FileType::Regular).unwrap();
        driver.write(f, 0, b"hi").unwrap();
        driver.rename_entry(root, "a.txt", root, "b.txt").unwrap();
        assert!(matches!(
            driver.lookup("/a.txt").unwrap_err(),
            FsError::NotFound
        ));
        let (ino, _, size) = driver.lookup("/b.txt").unwrap();
        assert_eq!(ino, f);
        assert_eq!(size, 2);
    }

    #[test]
    fn rename_directory_rewrites_descendants() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let d = driver.create(root, "old", FileType::Directory).unwrap();
        driver.create(d, "child.txt", FileType::Regular).unwrap();
        driver.rename_entry(root, "old", root, "new").unwrap();
        assert!(driver.lookup("/new").is_ok());
        assert!(driver.lookup("/new/child.txt").is_ok());
        assert!(driver.lookup("/old/child.txt").is_err());
    }

    #[test]
    fn rename_file_over_file_overwrites_target() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let a = driver.create(root, "a", FileType::Regular).unwrap();
        driver.write(a, 0, b"AA").unwrap();
        let b = driver.create(root, "b", FileType::Regular).unwrap();
        driver.write(b, 0, b"BBBB").unwrap();
        // POSIX: file-over-file rename overwrites the target atomically.
        driver.rename_entry(root, "a", root, "b").unwrap();
        // "a" is gone.
        assert!(matches!(
            driver.lookup("/a").unwrap_err(),
            FsError::NotFound
        ));
        // "b" now has the source inode (a) and its content.
        let (ino, _, size) = driver.lookup("/b").unwrap();
        assert_eq!(ino, a);
        assert_eq!(size, 2);
    }

    #[test]
    fn rename_dir_over_empty_dir_succeeds() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let src = driver.create(root, "src", FileType::Directory).unwrap();
        driver.create(src, "inner", FileType::Regular).unwrap();
        let _dst = driver.create(root, "dst", FileType::Directory).unwrap();
        driver.rename_entry(root, "src", root, "dst").unwrap();
        // /src is gone; /dst/inner is the carried-over descendant.
        assert!(matches!(
            driver.lookup("/src").unwrap_err(),
            FsError::NotFound
        ));
        assert!(driver.lookup("/dst").is_ok());
        assert!(driver.lookup("/dst/inner").is_ok());
    }

    #[test]
    fn rename_dir_over_non_empty_dir_is_not_empty() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let _src = driver.create(root, "src", FileType::Directory).unwrap();
        let dst = driver.create(root, "dst", FileType::Directory).unwrap();
        driver.create(dst, "keep", FileType::Regular).unwrap();
        let err = driver.rename_entry(root, "src", root, "dst").unwrap_err();
        assert!(matches!(err, FsError::DirectoryNotEmpty));
    }

    #[test]
    fn rename_file_over_dir_is_isdir() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        driver.create(root, "f", FileType::Regular).unwrap();
        driver.create(root, "d", FileType::Directory).unwrap();
        let err = driver.rename_entry(root, "f", root, "d").unwrap_err();
        assert!(matches!(err, FsError::IsADirectory));
    }

    #[test]
    fn rename_dir_over_file_is_not_dir() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        driver.create(root, "d", FileType::Directory).unwrap();
        driver.create(root, "f", FileType::Regular).unwrap();
        let err = driver.rename_entry(root, "d", root, "f").unwrap_err();
        assert!(matches!(err, FsError::NotADirectory));
    }

    #[test]
    fn set_mode_updates_inode_mode() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let f = driver.create(root, "m", FileType::Regular).unwrap();
        driver.set_mode(f, 0o644).unwrap();
        let inner = driver.inner.lock();
        let inode = inner
            .state
            .active_inodes
            .iter()
            .find(|i| i.path == "/m")
            .unwrap();
        assert_eq!(inode.metadata.mode, 0o644);
    }

    #[test]
    fn set_mode_on_unknown_inode_fails() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let err = driver.set_mode(999_000, 0o600).unwrap_err();
        assert!(matches!(err, FsError::NotFound));
    }

    #[test]
    fn set_owner_updates_uid_gid() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let f = driver.create(root, "o", FileType::Regular).unwrap();
        driver.set_owner(f, 1000, 100).unwrap();
        let inner = driver.inner.lock();
        let inode = inner
            .state
            .active_inodes
            .iter()
            .find(|i| i.path == "/o")
            .unwrap();
        assert_eq!(inode.metadata.uid, 1000);
        assert_eq!(inode.metadata.gid, 100);
    }

    #[test]
    fn set_owner_readonly_mount_rejected() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_read_only(adapter).expect("mount_ro");
        let err = driver.set_owner(1, 1, 1).unwrap_err();
        assert!(matches!(err, FsError::PermissionDenied));
    }

    #[test]
    fn create_symlink_stores_target_in_block_record() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        let link = driver
            .create_symlink(root, "lnk", "/target/path")
            .expect("symlink");
        let mut buf = [0u8; 32];
        let n = driver.read(link, 0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"/target/path");
        let (ino, _ft, size) = driver.lookup("/lnk").unwrap();
        assert_eq!(ino, link);
        assert_eq!(size as usize, "/target/path".len());
    }

    #[test]
    fn create_symlink_duplicate_fails() {
        let adapter = build_native_adapter(4096);
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount");
        let root = seed_root(&driver);
        driver.create_symlink(root, "l", "a").unwrap();
        let err = driver.create_symlink(root, "l", "b").unwrap_err();
        assert!(matches!(err, FsError::AlreadyExists));
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
        let f = driver
            .create(root, "persist.txt", FileType::Regular)
            .unwrap();
        driver.write(f, 0, b"persistent").unwrap();
        driver.flush().expect("flush");

        // Re-hydrate state from the persisted device.
        let inner = driver.inner.lock();
        let device_ref: &BlockDeviceAdapter = &inner.device;
        let reloaded = load_state_native(device_ref).expect("reload");
        assert!(reloaded
            .active_inodes
            .iter()
            .any(|i| i.path == "/persist.txt"));
        assert!(reloaded
            .block_records
            .iter()
            .any(|r| r.logical_size == b"persistent".len() as u64));
    }
}
