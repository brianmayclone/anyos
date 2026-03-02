//! Minimal read-only NTFS filesystem driver.
//!
//! Submodules:
//! - [`boot`]: Boot sector (VBR) parsing
//! - [`attr`]: Attribute type definitions and header parsing
//! - [`runlist`]: Data run decoding for non-resident attributes
//! - [`mft`]: MFT record parsing with fixup arrays
//! - [`index`]: Directory index ($I30) B+ tree parsing
//! - [`file`]: File read operations and NtfsReadPlan

mod boot;
mod attr;

// ── Storage I/O helpers (cfg-gated for ARM64 compilation) ──

/// Arch-abstracted storage read. Returns `false` on ARM64 (no storage driver yet).
#[cfg(target_arch = "x86_64")]
pub(crate) fn storage_read_sectors(abs_lba: u32, count: u32, buf: &mut [u8]) -> bool {
    storage_read_sectors(abs_lba, count, buf)
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn storage_read_sectors(abs_lba: u32, count: u32, buf: &mut [u8]) -> bool {
    crate::drivers::arm::storage::read_sectors(abs_lba, count, buf)
}
mod runlist;
mod mft;
mod index;
mod file;

pub use file::NtfsReadPlan;

use alloc::vec;
use alloc::vec::Vec;
use crate::fs::vfs::FsError;
use crate::fs::file::{DirEntry, FileType};
use self::boot::NtfsBpb;
use self::mft::MftRecord;
use self::attr::types as at;
use self::runlist::DataRun;

/// In-memory representation of a mounted NTFS filesystem (read-only).
pub struct NtfsFs {
    pub partition_lba: u32,
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    cluster_size: u32,
    mft_cluster: u64,
    mft_record_size: u32,
    index_record_size: u32,
    total_sectors: u64,
    /// Data runs for the $MFT file itself (decoded from MFT record 0).
    mft_runs: Vec<DataRun>,
}

impl NtfsFs {
    /// Mount an NTFS filesystem by reading the boot sector and MFT record 0.
    pub fn new(_device_id: u32, partition_lba: u32) -> Result<Self, FsError> {
        // Read boot sector
        let mut buf = [0u8; 512];
        if !storage_read_sectors(partition_lba, 1, &mut buf) {
            return Err(FsError::IoError);
        }

        let bpb = NtfsBpb::parse(&buf).ok_or_else(|| {
            crate::serial_println!("  NTFS: invalid boot sector at LBA {}", partition_lba);
            FsError::IoError
        })?;

        crate::serial_println!(
            "[OK] NTFS filesystem: {}B/sector, {} sec/cluster, MFT at cluster {}, record_size={}",
            bpb.bytes_per_sector, bpb.sectors_per_cluster,
            bpb.mft_cluster, bpb.mft_record_size,
        );

        let mut ntfs = NtfsFs {
            partition_lba,
            bytes_per_sector: bpb.bytes_per_sector,
            sectors_per_cluster: bpb.sectors_per_cluster,
            cluster_size: bpb.cluster_size,
            mft_cluster: bpb.mft_cluster,
            mft_record_size: bpb.mft_record_size,
            index_record_size: bpb.index_record_size,
            total_sectors: bpb.total_sectors,
            mft_runs: Vec::new(),
        };

        // Read MFT record 0 ($MFT itself) to get the MFT's own data runs
        let mft_lba = partition_lba as u64
            + bpb.mft_cluster * bpb.sectors_per_cluster as u64;
        let record_sectors = (bpb.mft_record_size + 511) / 512;
        let mut mft_buf = vec![0u8; bpb.mft_record_size as usize];

        if !storage_read_sectors(
            mft_lba as u32,
            record_sectors,
            &mut mft_buf,
        ) {
            crate::serial_println!("  NTFS: failed to read MFT record 0");
            return Err(FsError::IoError);
        }

        let mft_record = MftRecord::parse(&mft_buf, bpb.mft_record_size)?;

        // Find the unnamed $DATA attribute of $MFT
        let data_attr = mft_record.find_attr(at::DATA, None)
            .ok_or_else(|| {
                crate::serial_println!("  NTFS: MFT record 0 has no $DATA attribute");
                FsError::IoError
            })?;

        ntfs.mft_runs = mft_record.get_data_runs(&data_attr);
        if ntfs.mft_runs.is_empty() {
            crate::serial_println!("  NTFS: MFT has no data runs");
            return Err(FsError::IoError);
        }

        let total_mft_clusters: u64 = ntfs.mft_runs.iter().map(|r| r.length).sum();
        crate::serial_println!(
            "  NTFS: MFT spans {} clusters ({} records approx), index_record_size={}",
            total_mft_clusters,
            total_mft_clusters * bpb.cluster_size as u64 / bpb.mft_record_size as u64,
            bpb.index_record_size,
        );

        Ok(ntfs)
    }

    // =========================================================
    // MFT record I/O
    // =========================================================

    /// Read and parse an MFT record by its record number.
    fn read_mft_record(&self, record_num: u64) -> Result<MftRecord, FsError> {
        let byte_offset = record_num * self.mft_record_size as u64;
        let mut buf = vec![0u8; self.mft_record_size as usize];

        // Read MFT record data using the MFT's own data runs
        let bytes_read = file::read_from_runs(
            self.partition_lba,
            self.sectors_per_cluster,
            &self.mft_runs,
            u64::MAX, // MFT has no meaningful "file size" limit for reads
            byte_offset,
            &mut buf,
        )?;

        if bytes_read < self.mft_record_size as usize {
            return Err(FsError::IoError);
        }

        MftRecord::parse(&buf, self.mft_record_size)
    }

    // =========================================================
    // Directory operations
    // =========================================================

    /// List all entries in a directory given its MFT record number.
    pub fn read_dir(&self, dir_record: u64) -> Result<Vec<DirEntry>, FsError> {
        let internal = self.read_dir_internal(dir_record)?;
        Ok(internal.iter().map(|ie| self.index_entry_to_dir_entry(ie)).collect())
    }

    /// Internal: list directory entries with full MFT references.
    fn read_dir_internal(&self, dir_record: u64) -> Result<Vec<index::IndexEntry>, FsError> {
        let record = self.read_mft_record(dir_record)?;
        if !record.is_directory() {
            return Err(FsError::NotADirectory);
        }

        let mut entries = Vec::new();

        // 1. Parse $INDEX_ROOT (always resident)
        if let Some(idx_root_attr) = record.find_attr(at::INDEX_ROOT, Some("$I30")) {
            if let Some(data) = record.get_resident_data(&idx_root_attr) {
                let (root_entries, _has_sub) = index::parse_index_root(data);
                entries.extend(root_entries);
            }
        }

        // 2. Parse $INDEX_ALLOCATION (non-resident, optional)
        if let Some(idx_alloc_attr) = record.find_attr(at::INDEX_ALLOCATION, Some("$I30")) {
            let runs = record.get_data_runs(&idx_alloc_attr);
            if !runs.is_empty() {
                self.read_index_allocation_internal(&runs, &mut entries)?;
            }
        }

        Ok(entries)
    }

    /// Read all INDX records from $INDEX_ALLOCATION data runs.
    fn read_index_allocation_internal(
        &self,
        runs: &[DataRun],
        entries: &mut Vec<index::IndexEntry>,
    ) -> Result<(), FsError> {
        let spc = self.sectors_per_cluster as u64;
        let cluster_bytes = self.cluster_size as usize;

        for run in runs {
            let lcn = match run.lcn {
                Some(lcn) => lcn,
                None => continue, // sparse
            };

            let run_bytes = run.length as usize * cluster_bytes;
            let abs_lba = self.partition_lba as u64 + lcn * spc;
            let sector_count = (run_bytes + 511) / 512;

            let mut buf = vec![0u8; run_bytes];
            if !storage_read_sectors(
                abs_lba as u32,
                sector_count as u32,
                &mut buf,
            ) {
                continue; // skip unreadable runs
            }

            // Each INDX record is index_record_size bytes
            let rec_size = self.index_record_size as usize;
            let mut offset = 0;
            while offset + rec_size <= buf.len() {
                if &buf[offset..offset + 4] == b"INDX" {
                    if let Ok(idx_entries) = index::parse_indx_record(
                        &buf[offset..offset + rec_size],
                        self.index_record_size,
                    ) {
                        entries.extend(idx_entries);
                    }
                }
                offset += rec_size;
            }
        }

        Ok(())
    }

    /// Convert an index entry to a VFS DirEntry.
    fn index_entry_to_dir_entry(&self, ie: &index::IndexEntry) -> DirEntry {
        let is_dir = ie.file_name.flags & 0x10000000 != 0; // FILE_ATTR_DIRECTORY
        DirEntry {
            name: ie.file_name.name.clone(),
            file_type: if is_dir { FileType::Directory } else { FileType::Regular },
            size: ie.file_name.real_size as u32,
            is_symlink: false,
            uid: 0,
            gid: 0,
            mode: if is_dir { 0o755 } else { 0o644 },
        }
    }

    // =========================================================
    // Path resolution
    // =========================================================

    /// Lookup a path and return (mft_record, file_type, size).
    ///
    /// Path must start with "/" and use "/" as separator.
    pub fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        if path == "/" {
            return Ok((mft::records::ROOT_DIR as u32, FileType::Directory, 0));
        }

        let path = path.trim_start_matches('/');
        let mut current_record = mft::records::ROOT_DIR;

        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;

            // Read directory and get internal entries with MFT refs
            let entries = self.read_dir_internal(current_record)?;
            let found = entries.iter().find(|e| {
                e.file_name.name.eq_ignore_ascii_case(part)
            });

            match found {
                Some(entry) => {
                    let is_dir = entry.file_name.flags & 0x10000000 != 0;
                    if is_last {
                        let ft = if is_dir { FileType::Directory } else { FileType::Regular };
                        return Ok((entry.file_ref as u32, ft, entry.file_name.real_size as u32));
                    }
                    if !is_dir {
                        return Err(FsError::NotADirectory);
                    }
                    current_record = entry.file_ref;
                }
                None => return Err(FsError::NotFound),
            }
        }

        Err(FsError::NotFound)
    }

    /// Stat a path: returns (file_type, size, created, modified, accessed).
    pub fn stat_path(&self, path: &str) -> Result<(FileType, u32, u32, u32, u32), FsError> {
        let (mft_rec, file_type, _) = self.lookup(path)?;

        // Read MFT record for accurate size and timestamps
        let record = self.read_mft_record(mft_rec as u64)?;

        // Get file size from unnamed $DATA attribute
        let size = if file_type == FileType::Regular {
            if let Some(data_attr) = record.find_attr(at::DATA, None) {
                if let Some(ref nr) = data_attr.non_resident {
                    nr.real_size as u32
                } else if let Some(ref res) = data_attr.resident {
                    res.data_length
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        };

        // Get timestamps from $STANDARD_INFORMATION
        let (created, modified, accessed) = if let Some(si_attr) = record.find_attr(at::STANDARD_INFORMATION, None) {
            if let Some(data) = record.get_resident_data(&si_attr) {
                parse_timestamps(data)
            } else {
                (0, 0, 0)
            }
        } else {
            (0, 0, 0)
        };

        Ok((file_type, size, created, modified, accessed))
    }

    // =========================================================
    // File read operations
    // =========================================================

    /// Read file data starting at the given byte offset.
    pub fn read_file(&self, mft_record: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        let record = self.read_mft_record(mft_record as u64)?;

        let data_attr = record.find_attr(at::DATA, None)
            .ok_or(FsError::NotFound)?;

        if let Some(ref _res) = data_attr.resident {
            // Resident: data is inline in the MFT record
            if let Some(data) = record.get_resident_data(&data_attr) {
                let start = (offset as usize).min(data.len());
                let end = data.len();
                let to_copy = (end - start).min(buf.len());
                buf[..to_copy].copy_from_slice(&data[start..start + to_copy]);
                return Ok(to_copy);
            }
            return Err(FsError::IoError);
        }

        if let Some(ref nr) = data_attr.non_resident {
            let runs = record.get_data_runs(&data_attr);
            return file::read_from_runs(
                self.partition_lba,
                self.sectors_per_cluster,
                &runs,
                nr.real_size,
                offset as u64,
                buf,
            );
        }

        Err(FsError::IoError)
    }

    /// Read an entire file into a Vec<u8>.
    pub fn read_file_all(&self, mft_record: u32, file_size: u32) -> Result<Vec<u8>, FsError> {
        if file_size == 0 {
            return Ok(Vec::new());
        }
        let mut buf = vec![0u8; file_size as usize];
        let read = self.read_file(mft_record, 0, &mut buf)?;
        buf.truncate(read);
        Ok(buf)
    }

    /// Build a read plan for the given file (no disk I/O beyond MFT record read).
    pub fn get_file_read_plan(&self, mft_record: u32, file_size: u32) -> NtfsReadPlan {
        let record = match self.read_mft_record(mft_record as u64) {
            Ok(r) => r,
            Err(_) => return NtfsReadPlan {
                runs: Vec::new(),
                file_size: file_size as u64,
                sectors_per_cluster: self.sectors_per_cluster,
                partition_lba: self.partition_lba,
            },
        };

        let data_attr = match record.find_attr(at::DATA, None) {
            Some(a) => a,
            None => return NtfsReadPlan {
                runs: Vec::new(),
                file_size: file_size as u64,
                sectors_per_cluster: self.sectors_per_cluster,
                partition_lba: self.partition_lba,
            },
        };

        let runs = record.get_data_runs(&data_attr);
        let real_size = data_attr.non_resident
            .as_ref()
            .map(|nr| nr.real_size)
            .unwrap_or(file_size as u64);

        file::build_read_plan(
            self.partition_lba,
            self.sectors_per_cluster,
            &runs,
            real_size,
        )
    }
}

/// Parse $STANDARD_INFORMATION timestamps (NTFS FILETIME → Unix timestamp).
///
/// NTFS FILETIME: 100-nanosecond intervals since 1601-01-01.
/// Unix timestamp: seconds since 1970-01-01.
fn parse_timestamps(data: &[u8]) -> (u32, u32, u32) {
    if data.len() < 32 {
        return (0, 0, 0);
    }

    let created = filetime_to_unix(u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]));
    let modified = filetime_to_unix(u64::from_le_bytes([
        data[8], data[9], data[10], data[11],
        data[12], data[13], data[14], data[15],
    ]));
    let accessed = filetime_to_unix(u64::from_le_bytes([
        data[24], data[25], data[26], data[27],
        data[28], data[29], data[30], data[31],
    ]));

    (created, modified, accessed)
}

/// Convert NTFS FILETIME (100ns since 1601-01-01) to Unix timestamp.
fn filetime_to_unix(ft: u64) -> u32 {
    // Difference between 1601 and 1970 in 100ns intervals
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    if ft <= EPOCH_DIFF {
        return 0;
    }
    let unix_100ns = ft - EPOCH_DIFF;
    let unix_secs = unix_100ns / 10_000_000;
    if unix_secs > u32::MAX as u64 {
        return u32::MAX;
    }
    unix_secs as u32
}
