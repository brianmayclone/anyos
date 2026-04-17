// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! High-level format operation: wires a [`FileBackend`] into an
//! [`AnyOsBlockDevice`] and invokes `OdfDeviceSession::format_new_at`
//! to lay down a fresh CoreFS superblock, bitmaps, inode table and
//! journal on the backing file.
//!
//! Callers (CLI, Image-Builder, Installer) talk to this module via
//! [`format_volume`], which takes a single [`FormatRequest`] and
//! returns a [`FormatOutcome`] describing what was written.

use std::path::Path;

use corefs_core::config::CoreFsConfig;
use corefs_core::error::CoreFsResult;
use corefs_core::platform::Timestamp;
use corefs_core::storage::block_device::SECTOR_SIZE_512;
use corefs_core::storage::ondisk::layout::BLOCK_SIZE;
use corefs_core::storage::ondisk::session::{OdfDeviceSession, OdfSessionOptions};
use corefs_core::storage::ondisk::volume::FormatOptions;
use libcorefs_tools::block_device::AnyOsBlockDevice;

use crate::backend::FileBackend;

/// Parameters controlling a single CoreFS-format operation.
///
/// All sizes are in bytes and must be multiples of the sector size
/// (512).  The caller is responsible for ensuring the file/device at
/// `output` is at least `offset + size` bytes long — [`format_volume`]
/// will grow a newly-created file but never truncate an existing one.
#[derive(Debug, Clone)]
pub struct FormatRequest<'a> {
    /// Path to the disk image or device node to write into.
    pub output: &'a Path,
    /// Byte offset inside `output` where the CoreFS partition starts.
    /// Use `0` when `output` is a whole-disk image that should hold a
    /// single bare-metal filesystem without partition table.
    pub offset_bytes: u64,
    /// Size of the CoreFS partition in bytes.  Must be aligned to the
    /// sector size.
    pub size_bytes: u64,
    /// Human-readable volume label (max 16 bytes, truncated otherwise).
    pub label: &'a str,
    /// Optional override for the number of inode slots to reserve.
    /// `None` picks [`FormatOptions::default`]'s value.
    pub inode_count: Option<u64>,
    /// Optional override for the number of journal blocks.
    /// `None` picks [`FormatOptions::default`]'s value.
    pub journal_blocks: Option<u64>,
    /// Optional host directory whose contents will be copied into the
    /// freshly-formatted volume after save.  `None` leaves the volume
    /// empty (just root).
    pub populate_from: Option<&'a Path>,
}

/// Summary of a successful format operation.
#[derive(Debug, Clone)]
pub struct FormatOutcome {
    pub capacity_bytes: u64,
    pub total_blocks: u64,
    pub inode_count: u64,
    pub journal_blocks: u64,
    pub label: String,
    pub generation: u64,
    /// Populated only when [`FormatRequest::populate_from`] was set.
    pub populate: Option<crate::populate::PopulateReport>,
}

/// Format a CoreFS volume according to `req`.
///
/// Steps:
/// 1. Ensure the output file exists and is at least `offset + size`
///    bytes long.  A freshly-created file is sized via `set_len`; an
///    existing file is accepted as-is (we never shrink).
/// 2. Wrap the file in a [`FileBackend`] with 512-byte sectors.
/// 3. Construct an [`AnyOsBlockDevice`] that maps the partition slot
///    starting at `offset / 512` (LBA) for `size / 512` sectors.
/// 4. Call `OdfDeviceSession::format_new_at` — this writes superblock,
///    bitmaps, inode table, journal, and the native-layout persisted
///    state.
/// 5. Drop the session so all buffers are flushed to disk.
pub fn format_volume(req: &FormatRequest<'_>) -> Result<FormatOutcome, FormatError> {
    let sector_size = SECTOR_SIZE_512;
    if req.size_bytes == 0 || req.size_bytes % u64::from(sector_size) != 0 {
        return Err(FormatError::InvalidSize {
            size: req.size_bytes,
            sector_size,
        });
    }
    if req.offset_bytes % u64::from(sector_size) != 0 {
        return Err(FormatError::InvalidOffset {
            offset: req.offset_bytes,
            sector_size,
        });
    }

    let total_required = req
        .offset_bytes
        .checked_add(req.size_bytes)
        .ok_or(FormatError::SizeOverflow)?;

    // Grow (or create) the file so its length covers the partition.
    // We never truncate — if the file is larger, we leave the tail
    // alone (useful when writing into a pre-partitioned disk image).
    let file_path = req.output;
    let current_len = file_path
        .metadata()
        .map(|m| m.len())
        .unwrap_or(0);
    let ensure_len = current_len.max(total_required);
    let backend = FileBackend::open(file_path, ensure_len, sector_size).map_err(|e| {
        FormatError::Io {
            path: file_path.display().to_string(),
            source: e,
        }
    })?;

    let partition_lba_offset = req.offset_bytes / u64::from(sector_size);
    let device = AnyOsBlockDevice::new(
        backend,
        /* device_id = */ 0,
        partition_lba_offset,
        req.size_bytes,
        sector_size,
    )
    .map_err(FormatError::CoreFs)?;

    let defaults = FormatOptions::default();
    let inode_count = req.inode_count.unwrap_or(defaults.inode_count);
    let journal_blocks = req.journal_blocks.unwrap_or(defaults.journal_blocks);

    let label = req.label.to_owned();
    let opts = OdfSessionOptions {
        capacity_bytes: req.size_bytes,
        label: label.clone(),
        uuid: seeded_uuid(req),
        inode_count,
        journal_blocks,
        config: CoreFsConfig::default(),
    };

    let session: OdfDeviceSession =
        OdfDeviceSession::format_new_at(Box::new(device), &opts, Timestamp::EPOCH)
            .map_err(FormatError::CoreFs)?;

    // CoreFS addresses data in `BLOCK_SIZE` (4096-byte) blocks — this is
    // the unit persisted in `Superblock::total_blocks` and also what
    // `corefs_core::volume::inspect` reports.  Use the same unit here
    // so downstream tooling (image builder, inspect, fsck) sees
    // consistent numbers.
    let total_blocks = req.size_bytes / BLOCK_SIZE;

    // Optional populate.  We reclaim the device from the session first
    // because BlockStore::write_at needs a `&mut dyn BlockDevice` and
    // the session's device field is private.
    let populate_report = if let Some(src) = req.populate_from {
        let mut device = session.into_device();
        let report = crate::populate::populate_volume(
            device.as_mut(),
            src,
            Timestamp::EPOCH,
        )
        .map_err(|e| FormatError::Populate(e.to_string()))?;
        // Device drops here, flushing pending writes to the file.
        Some(report)
    } else {
        drop(session); // flush on drop
        None
    };

    Ok(FormatOutcome {
        capacity_bytes: req.size_bytes,
        total_blocks,
        inode_count,
        journal_blocks,
        label,
        // `format_new_at` writes superblock twice (format + save_state_native),
        // so generation starts at 2.  Matching the anyOS-side mkfs.corefs.
        generation: 2,
        populate: populate_report,
    })
}

/// Deterministic 16-byte "UUID" derived from the request fields so
/// repeated formats of the same partition keep the same volume
/// identifier.  Not a real UUIDv4 — a stable hash over `(offset, size,
/// label)` bytes.  Sufficient for intra-image identification; Phase 4
/// (Installer) may overwrite with a real hardware-entropy UUID.
fn seeded_uuid(req: &FormatRequest<'_>) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&req.size_bytes.to_le_bytes());
    out[8..16].copy_from_slice(&req.offset_bytes.to_le_bytes());
    for (i, b) in req.label.bytes().enumerate().take(16) {
        out[i] ^= b;
    }
    out
}

/// Errors surfaced by [`format_volume`].
#[derive(Debug)]
pub enum FormatError {
    /// `size_bytes` is zero or not a multiple of the sector size.
    InvalidSize { size: u64, sector_size: u32 },
    /// `offset_bytes` is not a multiple of the sector size.
    InvalidOffset { offset: u64, sector_size: u32 },
    /// `offset + size` would overflow a `u64`.
    SizeOverflow,
    /// Host-side I/O error (file create / seek / write).
    Io {
        path: String,
        source: std::io::Error,
    },
    /// CoreFS format or adapter error bubbled up from `corefs-core`.
    CoreFs(corefs_core::error::CoreFsError),
    /// Error during the optional post-format populate step.
    Populate(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::InvalidSize { size, sector_size } => write!(
                f,
                "invalid --size {size}: must be a non-zero multiple of {sector_size}"
            ),
            FormatError::InvalidOffset {
                offset,
                sector_size,
            } => write!(
                f,
                "invalid --offset {offset}: must be a multiple of {sector_size}"
            ),
            FormatError::SizeOverflow => {
                write!(f, "offset + size exceeds u64::MAX")
            }
            FormatError::Io { path, source } => write!(f, "I/O error on {path}: {source}"),
            FormatError::CoreFs(e) => write!(f, "corefs format failed: {e}"),
            FormatError::Populate(msg) => write!(f, "populate failed: {msg}"),
        }
    }
}

impl std::error::Error for FormatError {}

// Allow `?`-propagation of CoreFsError into FormatError.
impl From<corefs_core::error::CoreFsError> for FormatError {
    fn from(e: corefs_core::error::CoreFsError) -> Self {
        FormatError::CoreFs(e)
    }
}

// Expose the base `CoreFsResult` so callers stay independent of our
// `FormatError` wrapping if they prefer.
#[allow(dead_code)]
pub fn format_volume_raw(req: &FormatRequest<'_>) -> CoreFsResult<FormatOutcome> {
    format_volume(req).map_err(|e| match e {
        FormatError::CoreFs(inner) => inner,
        other => corefs_core::error::CoreFsError::InvalidInput(other.to_string()),
    })
}
