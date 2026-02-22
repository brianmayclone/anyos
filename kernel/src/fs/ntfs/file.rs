//! NTFS file read operations.
//!
//! Reads file data by resolving data runs from the unnamed $DATA attribute
//! of an MFT record.

use alloc::vec;
use alloc::vec::Vec;
use crate::fs::vfs::FsError;
use super::runlist::DataRun;

/// Pre-computed plan for reading an NTFS file's data.
///
/// Built from MFT record + data runs (no disk I/O beyond the initial record read).
/// Can be executed after releasing the VFS lock.
pub struct NtfsReadPlan {
    /// Contiguous (absolute_lba, sector_count) runs covering the file.
    pub runs: Vec<(u64, u32)>,
    /// Actual file size in bytes.
    pub file_size: u64,
    /// Sectors per cluster.
    pub sectors_per_cluster: u8,
    /// Partition start LBA (added to all cluster-based LBAs).
    pub partition_lba: u32,
}

impl NtfsReadPlan {
    /// Execute the read plan — reads sectors from the storage backend.
    ///
    /// **Must be called WITHOUT the VFS lock held.**
    pub fn execute(&self) -> Result<Vec<u8>, FsError> {
        if self.file_size == 0 {
            return Ok(Vec::new());
        }

        let total_bytes: usize = self.runs.iter()
            .map(|(_, sc)| *sc as usize * 512)
            .sum();
        let mut buf = vec![0u8; total_bytes];
        let mut offset = 0usize;

        for &(abs_lba, sector_count) in &self.runs {
            let bytes = sector_count as usize * 512;
            if !crate::drivers::storage::read_sectors(
                abs_lba as u32,
                sector_count,
                &mut buf[offset..offset + bytes],
            ) {
                return Err(FsError::IoError);
            }
            offset += bytes;
        }

        buf.truncate(self.file_size as usize);
        Ok(buf)
    }
}

/// Read file data from data runs into a buffer at a given byte offset.
///
/// `partition_lba`: absolute LBA of partition start.
/// `sectors_per_cluster`: from BPB.
/// `runs`: decoded data runs for this file's $DATA attribute.
/// `file_size`: real file size from the non-resident header.
/// `offset`: byte offset to start reading from.
/// `buf`: output buffer.
pub(super) fn read_from_runs(
    partition_lba: u32,
    sectors_per_cluster: u8,
    runs: &[DataRun],
    file_size: u64,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, FsError> {
    if buf.is_empty() || offset >= file_size {
        return Ok(0);
    }

    let spc = sectors_per_cluster as u64;
    let cluster_bytes = spc * 512;
    let max_read = ((file_size - offset) as usize).min(buf.len());
    let mut bytes_read = 0usize;
    let mut run_byte_offset: u64 = 0;

    for run in runs {
        let run_bytes = run.length * cluster_bytes;

        // Skip runs before our offset
        if run_byte_offset + run_bytes <= offset {
            run_byte_offset += run_bytes;
            continue;
        }

        // Calculate start position within this run
        let start_in_run = if offset > run_byte_offset {
            (offset - run_byte_offset) as usize
        } else {
            0
        };

        let available = run_bytes as usize - start_in_run;
        let to_copy = available.min(max_read - bytes_read);

        match run.lcn {
            Some(lcn) => {
                // Calculate absolute LBA for this run's data
                let abs_lba = partition_lba as u64 + lcn * spc;

                // Read full clusters covering the needed range
                let first_sector = start_in_run / 512;
                let last_sector = (start_in_run + to_copy + 511) / 512;
                let sector_count = last_sector - first_sector;

                let read_lba = abs_lba + first_sector as u64;
                let read_bytes = sector_count * 512;
                let mut tmp = vec![0u8; read_bytes];

                if !crate::drivers::storage::read_sectors(
                    read_lba as u32,
                    sector_count as u32,
                    &mut tmp,
                ) {
                    return Err(FsError::IoError);
                }

                let byte_in_sector = start_in_run % 512;
                buf[bytes_read..bytes_read + to_copy]
                    .copy_from_slice(&tmp[byte_in_sector..byte_in_sector + to_copy]);
            }
            None => {
                // Sparse run — fill with zeros
                for b in &mut buf[bytes_read..bytes_read + to_copy] {
                    *b = 0;
                }
            }
        }

        bytes_read += to_copy;
        run_byte_offset += run_bytes;

        if bytes_read >= max_read {
            break;
        }
    }

    Ok(bytes_read)
}

/// Build a read plan from data runs (no disk I/O).
pub(super) fn build_read_plan(
    partition_lba: u32,
    sectors_per_cluster: u8,
    runs: &[DataRun],
    file_size: u64,
) -> NtfsReadPlan {
    let spc = sectors_per_cluster as u32;
    let mut plan_runs = Vec::new();

    if file_size == 0 {
        return NtfsReadPlan {
            runs: plan_runs,
            file_size,
            sectors_per_cluster,
            partition_lba,
        };
    }

    for run in runs {
        match run.lcn {
            Some(lcn) => {
                let abs_lba = partition_lba as u64 + lcn * spc as u64;
                let sector_count = run.length as u32 * spc;
                plan_runs.push((abs_lba, sector_count));
            }
            None => {
                // Sparse runs can't be represented as LBA reads; skip
                // (the execute() method would need special handling for these)
            }
        }
    }

    NtfsReadPlan {
        runs: plan_runs,
        file_size,
        sectors_per_cluster,
        partition_lba,
    }
}
