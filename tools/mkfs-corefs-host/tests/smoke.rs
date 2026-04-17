// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! End-to-end smoke tests for `mkfs-corefs-host`.
//!
//! Each test formats a fresh temporary file as CoreFS and then
//! re-opens the same file through `corefs-core` to verify the
//! superblock written by [`format::format_volume`] is structurally
//! valid — the same code path that the anyOS kernel will exercise
//! when it later mounts the volume.

use std::path::PathBuf;

use corefs_core::storage::block_device::BlockDevice;
use corefs_core::storage::ondisk::volume::inspect;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use tempfile::NamedTempFile;

// Pull in the tool's private modules — Cargo integration tests can
// reference the binary crate via its package name.
#[path = "../src/backend.rs"]
mod backend;
#[path = "../src/format.rs"]
mod format;

use backend::FileBackend;
use format::{format_volume, FormatRequest};

/// Format a 4 MiB single-partition image and verify inspect() can
/// read it back with a valid superblock.
#[test]
fn format_and_inspect_single_partition_image() {
    let (img_path, _keep) = fresh_image(4 * 1024 * 1024);
    let req = FormatRequest {
        output: &img_path,
        offset_bytes: 0,
        size_bytes: 4 * 1024 * 1024,
        label: "smoke",
        inode_count: None,
        journal_blocks: None,
    };
    let outcome = format_volume(&req).expect("format must succeed");
    assert_eq!(outcome.capacity_bytes, 4 * 1024 * 1024);
    assert_eq!(outcome.label, "smoke");
    assert!(outcome.total_blocks > 0);

    // Re-open via the same BlockDevice adapter and inspect.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&img_path)
        .expect("reopen");
    let backend = FileBackend::new(file, 512);
    let device = AnyOsBlockDevice::new(
        backend,
        0,
        /* partition_lba_offset = */ 0,
        outcome.capacity_bytes,
        512,
    )
    .expect("adapter");
    let info = inspect(&device).expect("inspect should succeed");
    assert!(info.primary_ok, "primary superblock must be readable");
    assert_eq!(info.total_blocks, outcome.total_blocks);
    assert_eq!(&info.label, "smoke");
}

/// Format a CoreFS partition inside a larger image at a non-zero
/// offset — mimics the Dual-Partition layout the image-builder will
/// produce (FAT-boot partition first, CoreFS partition second).
#[test]
fn format_partition_at_offset() {
    let total_size = 8 * 1024 * 1024;
    let fat_boot_size = 1 * 1024 * 1024; // fake FAT boot partition
    let corefs_size = total_size - fat_boot_size;

    let (img_path, _keep) = fresh_image(total_size);
    let req = FormatRequest {
        output: &img_path,
        offset_bytes: fat_boot_size,
        size_bytes: corefs_size,
        label: "system",
        inode_count: None,
        journal_blocks: None,
    };
    let outcome = format_volume(&req).expect("format must succeed");
    assert_eq!(outcome.capacity_bytes, corefs_size);

    // Ensure the first megabyte (where a FAT boot partition would
    // live) was NOT overwritten by the CoreFS format.  We seeded it
    // with a recognizable pattern below in `fresh_image`.
    let mut head = vec![0u8; 4096];
    let file = std::fs::OpenOptions::new()
        .read(true)
        .open(&img_path)
        .expect("reopen");
    use std::io::Read;
    use std::io::Seek;
    let mut file = file;
    file.rewind().unwrap();
    file.read_exact(&mut head).unwrap();
    assert!(
        head.iter().any(|&b| b == 0xAB),
        "boot region must retain its pre-format bytes (offset-respecting write)"
    );

    // Re-inspect the CoreFS partition by supplying the same offset.
    let backend = FileBackend::new(file, 512);
    let device = AnyOsBlockDevice::new(
        backend,
        0,
        /* partition_lba_offset = */ fat_boot_size / 512,
        corefs_size,
        512,
    )
    .expect("adapter");
    let info = inspect(&device).expect("inspect at offset");
    assert!(info.primary_ok);
    assert_eq!(&info.label, "system");
}

/// Helper: create a named tempfile of `size` bytes pre-seeded with
/// `0xAB` so we can later verify that format_volume respects the
/// offset (the pre-seeded boot region must survive).  Returns the
/// absolute path and a `PathBuf` wrapper so the temp file is kept
/// alive until the test exits (`NamedTempFile` would auto-delete).
fn fresh_image(size: u64) -> (PathBuf, PathBuf) {
    let tmp = NamedTempFile::new().expect("tempfile");
    let path = tmp.path().to_path_buf();
    // Keep() prevents auto-delete; we return a second PathBuf so the
    // test can decide when to remove it.
    let (_file, persisted) = tmp.keep().expect("persist tempfile");
    // Size and seed.
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .truncate(false)
        .open(&persisted)
        .expect("open for seed");
    f.set_len(size).expect("set_len");
    use std::io::Seek;
    use std::io::Write;
    f.rewind().unwrap();
    let seed = vec![0xABu8; 64 * 1024];
    f.write_all(&seed).unwrap();
    (path, persisted)
}
