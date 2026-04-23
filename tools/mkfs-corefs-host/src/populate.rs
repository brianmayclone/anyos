// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! Recursively copy a host directory tree into a freshly-formatted CoreFS
//! volume.
//!
//! The implementation mirrors the kernel-side `CoreFsDriver::create` +
//! `fn write` path from `kernel/src/fs/corefs/driver.rs` — build one
//! [`Inode`] per file/directory, append it to `PersistedState::active_inodes`,
//! and for regular files stream the bytes onto the device via
//! [`BlockStore::write_at`].  At the end we sync
//! `state.block_records`/`state.free_extents` from the `BlockStore` and
//! persist the state with [`save_state_native`] — same finaliser the
//! kernel driver uses on its `flush()` call.
//!
//! We intentionally do **not** go through [`OdfDeviceSession`] here: the
//! session's private device field makes it impossible to hand out the
//! mutable `&mut dyn BlockDevice` that `BlockStore::write_at` requires,
//! so the caller already consumes the session via `into_device()` and
//! passes us the raw device back.

use std::fs;
use std::io;
use std::path::Path;

use corefs_core::domain::inode::{Inode, InodeId, InodeKind};
use corefs_core::domain::metadata::FileMetadata;
use corefs_core::error::CoreFsError;
use corefs_core::platform::Timestamp;
use corefs_core::storage::block_device::BlockDevice;
use corefs_core::storage::block_store::{AllocatorPolicy, BlockStore};
use corefs_core::storage::ondisk::layout::BLOCK_SIZE;
use corefs_core::storage::ondisk::native::{load_state_native, save_state_native};
use corefs_core::storage::ondisk::volume::read_sb_with_fallbacks;
use corefs_core::storage::persisted_state::PersistedState;

/// Summary of a populate operation (shown in CLI output / returned from
/// library-style callers).
#[derive(Debug, Clone, Default)]
pub struct PopulateReport {
    pub directories: u32,
    pub files: u32,
    pub bytes: u64,
    /// Host-side files we couldn't read (permission error, broken symlink
    /// etc.).  Reported but non-fatal — the populate continues.
    pub skipped: u32,
}

/// Errors specific to the populate phase.
#[derive(Debug)]
pub enum PopulateError {
    Io {
        path: String,
        source: io::Error,
    },
    CoreFs(CoreFsError),
}

impl std::fmt::Display for PopulateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PopulateError::Io { path, source } => write!(f, "I/O error on {path}: {source}"),
            PopulateError::CoreFs(e) => write!(f, "corefs error: {e}"),
        }
    }
}

impl std::error::Error for PopulateError {}

impl From<CoreFsError> for PopulateError {
    fn from(e: CoreFsError) -> Self {
        PopulateError::CoreFs(e)
    }
}

/// Copy the contents of `host_dir` into the CoreFS volume backed by
/// `device`.
///
/// The device must already contain a freshly-formatted volume (i.e.
/// [`OdfDeviceSession::format_new_at`] succeeded and the empty state
/// was persisted).  `load_state_native` pulls that empty state back in,
/// then we mutate it and re-save.
///
/// The root directory `/` is synthesized in-memory if missing — it's
/// not part of a fresh volume's state on disk.
///
/// `timestamp` is applied to every inode we create.  Using a single
/// value keeps the resulting volume deterministic (two builds of the
/// same sysroot produce byte-identical CoreFS partitions).
pub fn populate_volume(
    device: &mut dyn BlockDevice,
    host_dir: &Path,
    timestamp: Timestamp,
) -> Result<PopulateReport, PopulateError> {
    if !host_dir.is_dir() {
        return Err(PopulateError::Io {
            path: host_dir.display().to_string(),
            source: io::Error::new(io::ErrorKind::NotFound, "populate source is not a directory"),
        });
    }

    let mut state = load_state_native(device)?;

    // Root inode — same logic as CoreFsDriver::ensure_root_directory
    // (kernel/src/fs/corefs/driver.rs:485).
    let has_root = state
        .active_inodes
        .iter()
        .any(|i| i.path == "/" && matches!(i.kind, InodeKind::Directory));
    if !has_root {
        let mut md = FileMetadata::default();
        let (uid, gid, mode) = default_perms_for("/");
        md.uid = uid;
        md.gid = gid;
        md.mode = mode;
        state.active_inodes.push(Inode::new_at(
            InodeId(1),
            InodeKind::Directory,
            "/".to_string(),
            md,
            timestamp,
        ));
    }

    let first_data_block = compute_first_data_block(device, &state)?;
    let mut blocks = BlockStore::from_records_with_allocator_and_start(
        state.block_records.clone(),
        BLOCK_SIZE as usize,
        state.allocator_policy.clone(),
        state.free_extents.clone(),
        first_data_block,
    );
    let mut next_id = compute_next_id(&state);

    let mut report = PopulateReport::default();
    walk(
        device,
        &mut state,
        &mut blocks,
        &mut next_id,
        &mut report,
        host_dir,
        "/",
        timestamp,
    )?;

    // Final sync — same finaliser as CoreFsDriver::flush
    // (kernel/src/fs/corefs/driver.rs:271).
    state.block_records = blocks.records();
    state.free_extents = blocks.free_extents();
    save_state_native(device, &state)?;

    Ok(report)
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Traverse one host directory entry, materialising matching CoreFS
/// inodes under `virt_parent`.
fn walk(
    device: &mut dyn BlockDevice,
    state: &mut PersistedState,
    blocks: &mut BlockStore,
    next_id: &mut u64,
    report: &mut PopulateReport,
    host_dir: &Path,
    virt_parent: &str,
    timestamp: Timestamp,
) -> Result<(), PopulateError> {
    let mut entries: Vec<_> = match fs::read_dir(host_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .collect(),
        Err(e) => {
            eprintln!(
                "populate: cannot read {}: {}",
                host_dir.display(), e
            );
            report.skipped += 1;
            return Ok(());
        }
    };
    // Deterministic order — same contract as the C mkimage
    // populate_from_sysroot (buildsystem/mkimage/src/exfat.c:914).
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let name = entry.file_name();
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                report.skipped += 1;
                continue;
            }
        };
        // Skip hidden dot-files (matches the sysroot populate rules in
        // both the C mkimage and the Python tool).
        if name_str.starts_with('.') {
            continue;
        }

        let host_path = entry.path();
        let virt_path = join_virt(virt_parent, name_str);

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                eprintln!(
                    "populate: stat {}: {}", host_path.display(), e
                );
                report.skipped += 1;
                continue;
            }
        };

        if file_type.is_symlink() {
            // CoreFS models symlinks as a distinct InodeKind plus the
            // target stored as the inode's byte content, same as the
            // kernel driver's create_symlink path.  For initial
            // populate we skip them — the dual-partition sysroot tree
            // doesn't have any, and support can be added later.
            report.skipped += 1;
            continue;
        }

        if file_type.is_dir() {
            let id = InodeId(*next_id);
            *next_id += 1;
            let mut md = FileMetadata::default();
            let (uid, gid, mode) = default_perms_for(&virt_path);
            md.uid = uid;
            md.gid = gid;
            md.mode = mode;
            state.active_inodes.push(Inode::new_at(
                id, InodeKind::Directory, virt_path.clone(), md, timestamp,
            ));
            report.directories += 1;
            walk(device, state, blocks, next_id, report, &host_path, &virt_path, timestamp)?;
            continue;
        }

        if file_type.is_file() {
            let data = match fs::read(&host_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("populate: read {}: {}", host_path.display(), e);
                    report.skipped += 1;
                    continue;
                }
            };

            let id = InodeId(*next_id);
            *next_id += 1;

            let mut md = FileMetadata::default();
            let (uid, gid, mode) = default_perms_for(&virt_path);
            md.uid = uid;
            md.gid = gid;
            md.mode = mode;
            let mut inode = Inode::new_at(
                id, InodeKind::File, virt_path.clone(), md, timestamp,
            );
            inode.size = data.len();
            state.active_inodes.push(inode);

            if !data.is_empty() {
                blocks.write_at(device, id, 0, &data)?;
            }

            report.files += 1;
            report.bytes = report.bytes.saturating_add(data.len() as u64);
            continue;
        }

        // Pipes, sockets, device nodes — skip.
        report.skipped += 1;
    }
    Ok(())
}

/// Pick a sensible (uid, gid, mode) tuple for a freshly-populated inode
/// based on its virtual path.
///
/// anyOS permission encoding (see `kernel/src/fs/permissions.rs`):
/// 4 bits per class — Read=1, Modify=2, Delete=4, Create=8.
///
/// - `/Users/Shared`, `/users/shared`, `/tmp`, `/var/tmp`: `0xFFF` — world-
///   writable scratch areas (everyone can read, modify, delete, create).
/// - Everything else (incl. `/System/**`): `0xF11` — owner has full access,
///   group and other can only READ (and, for directories, traverse). This
///   keeps System binaries executable by non-root users after login's
///   identity switch while preventing them from modifying system files.
fn default_perms_for(virt_path: &str) -> (u32, u32, u32) {
    // Normalise for prefix comparison.
    let p = virt_path;
    let world_rw = p == "/Users/Shared"
        || p.starts_with("/Users/Shared/")
        || p == "/users/shared"
        || p.starts_with("/users/shared/")
        || p == "/tmp"
        || p.starts_with("/tmp/")
        || p == "/var/tmp"
        || p.starts_with("/var/tmp/");
    let mode = if world_rw { 0xFFF } else { 0xF11 };
    (0, 0, mode)
}

/// `"/" + name` or `parent + "/" + name`, keeping paths canonical.
fn join_virt(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// Mirror of `compute_next_id` in
/// kernel/src/fs/corefs/driver.rs:506.
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

/// Mirror of `compute_first_data_block` in
/// kernel/src/fs/corefs/driver.rs:524.
/// Pick the first block number the data allocator may hand out.
///
/// Must be at or beyond `geom.data_start` — otherwise the allocator
/// walks into the reserved regions (superblocks, bitmaps, inode table,
/// journal) and `BlockStore::write_at` silently corrupts them.  An
/// earlier version used a hardcoded `.max(256)` fallback which landed
/// inside the journal on small volumes (447 MiB → journal_start ≈ 134,
/// journal spans ~134..390) and invalidated the journal header CRC.
fn compute_first_data_block(
    device: &dyn BlockDevice,
    state: &PersistedState,
) -> Result<u64, CoreFsError> {
    let sb = read_sb_with_fallbacks(device)?;
    let data_start = sb.geometry().data_start;
    let highest = state
        .block_records
        .iter()
        .flat_map(|r| r.extents.iter())
        .map(|e| e.physical_block + u64::from(e.length_blocks))
        .max()
        .unwrap_or(0);
    Ok(highest.max(data_start))
}

// Suppress "unused" warning when AllocatorPolicy is only referenced
// via state.allocator_policy.clone() — the import is load-bearing.
#[allow(dead_code)]
fn _unused_marker(_: AllocatorPolicy) {}
