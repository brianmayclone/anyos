// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! [`FileBackend`] — [`DiskBackend`] implementation over a regular file.
//!
//! Lets `libcorefs-tools::AnyOsBlockDevice` drive host-side disk images:
//! `.img` files, loopback devices, or a raw `/dev/sdX`.  The backend
//! translates the `(lba, count)` sector-addressing contract into
//! `pread(2)` / `pwrite(2)`-style positional I/O against the underlying
//! [`std::fs::File`].
//!
//! # Partition offset
//!
//! The wrapping [`AnyOsBlockDevice`] already knows about the partition
//! LBA offset, so this backend works in **absolute** file offsets.
//! When a caller wants to format partition `N` inside a disk image, it
//! constructs an `AnyOsBlockDevice::new(FileBackend, device_id=0,
//! partition_lba_offset, capacity, sector_size)` and the adapter takes
//! care of adding the offset before delegating to us.
//!
//! # Thread-safety
//!
//! [`DiskBackend`] requires `Send`.  We wrap the file in a [`Mutex`] so
//! both the `read` (`&self`) and `write` (`&mut self`) paths can
//! share the same handle without pulling in platform-specific
//! `pread`/`pwrite` helpers.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

use libcorefs_tools::block_device::DiskBackend;

/// Sector-addressable backend backed by a single [`File`] handle.
///
/// Construct via [`FileBackend::new`] or [`FileBackend::open`].
pub struct FileBackend {
    /// Logical sector size (bytes per LBA).  Supplied at construction
    /// and used to translate `(lba, count)` → byte offsets.
    sector_size: u32,
    /// Underlying file handle, guarded by a mutex so that the backend
    /// can be shared across threads (required by the `Send` bound on
    /// [`DiskBackend`]).
    file: Mutex<File>,
}

impl FileBackend {
    /// Wrap an already-opened file.
    ///
    /// `sector_size` must match the sector size passed to
    /// [`AnyOsBlockDevice::new`] and is usually `512`.
    pub fn new(file: File, sector_size: u32) -> Self {
        Self {
            sector_size,
            file: Mutex::new(file),
        }
    }

    /// Open `path` read-write, creating it if missing, and optionally
    /// truncate/extend it to `ensure_len` bytes before returning the
    /// backend.
    ///
    /// `ensure_len == 0` skips the resize step — useful when the caller
    /// wants to format an existing file whose size has already been
    /// fixed (e.g. a partition inside a larger disk image).
    pub fn open(path: &std::path::Path, ensure_len: u64, sector_size: u32) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;
        if ensure_len > 0 {
            file.set_len(ensure_len)?;
        }
        Ok(Self::new(file, sector_size))
    }

    /// Expose the current file length (for CLI diagnostics and the
    /// `--size` auto-detect path).  Marked `#[allow(dead_code)]`
    /// because the binary's current CLI goes through
    /// `Path::metadata` directly; the helper is kept available for
    /// downstream tools that hold a constructed backend.
    #[allow(dead_code)]
    pub fn file_len(&self) -> std::io::Result<u64> {
        let guard = self.file.lock().expect("FileBackend mutex poisoned");
        guard.metadata().map(|m| m.len())
    }
}

impl std::fmt::Debug for FileBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileBackend")
            .field("sector_size", &self.sector_size)
            .finish_non_exhaustive()
    }
}

impl DiskBackend for FileBackend {
    fn read(&self, _device_id: u32, lba: u64, count: u32, buf: &mut [u8]) -> u32 {
        let byte_offset = lba.saturating_mul(u64::from(self.sector_size));
        let length = (count as usize).saturating_mul(self.sector_size as usize);
        if buf.len() < length {
            return u32::MAX;
        }
        let mut guard = match self.file.lock() {
            Ok(g) => g,
            Err(_) => return u32::MAX,
        };
        if guard.seek(SeekFrom::Start(byte_offset)).is_err() {
            return u32::MAX;
        }
        match guard.read_exact(&mut buf[..length]) {
            Ok(()) => 0,
            Err(_) => u32::MAX,
        }
    }

    fn write(&mut self, _device_id: u32, lba: u64, count: u32, buf: &[u8]) -> u32 {
        let byte_offset = lba.saturating_mul(u64::from(self.sector_size));
        let length = (count as usize).saturating_mul(self.sector_size as usize);
        if buf.len() < length {
            return u32::MAX;
        }
        let mut guard = match self.file.lock() {
            Ok(g) => g,
            Err(_) => return u32::MAX,
        };
        if guard.seek(SeekFrom::Start(byte_offset)).is_err() {
            return u32::MAX;
        }
        match guard.write_all(&buf[..length]) {
            Ok(()) => 0,
            Err(_) => u32::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_backend(bytes: &[u8]) -> FileBackend {
        let mut tmp = NamedTempFile::new().expect("tempfile");
        tmp.write_all(bytes).expect("seed write");
        let (file, _path) = tmp.keep().expect("persist tempfile");
        FileBackend::new(file, 512)
    }

    #[test]
    fn read_returns_zero_on_success() {
        let mut data = vec![0u8; 4096];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i & 0xff) as u8;
        }
        let backend = make_backend(&data);
        let mut out = vec![0u8; 512];
        assert_eq!(backend.read(0, 2, 1, &mut out), 0);
        assert_eq!(out, data[1024..1536]);
    }

    #[test]
    fn read_rejects_short_buffer() {
        let backend = make_backend(&vec![0u8; 2048]);
        let mut out = vec![0u8; 100]; // smaller than 512
        assert_eq!(backend.read(0, 0, 1, &mut out), u32::MAX);
    }

    #[test]
    fn write_then_read_round_trips() {
        let backend = make_backend(&vec![0u8; 8192]);
        let mut backend = backend;
        let payload = vec![0xA5u8; 1024];
        assert_eq!(backend.write(0, 4, 2, &payload), 0);
        let mut got = vec![0u8; 1024];
        assert_eq!(backend.read(0, 4, 2, &mut got), 0);
        assert_eq!(got, payload);
    }

    #[test]
    fn open_creates_and_sizes_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("disk.img");
        let backend = FileBackend::open(&path, 4 * 1024 * 1024, 512).expect("open");
        assert_eq!(backend.file_len().unwrap(), 4 * 1024 * 1024);
    }
}
