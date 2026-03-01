//! File read/write operations on FAT filesystems.

use crate::fs::vfs::FsError;
use alloc::vec;
use alloc::vec::Vec;
use super::FatFs;

/// Pre-computed plan for reading a file's data sectors.
///
/// Built from the in-memory FAT cache (no disk I/O) while the VFS lock is
/// held.  Once the lock is dropped (re-enabling interrupts), [`execute`]
/// performs the actual sector reads so that timer, mouse, and keyboard
/// interrupts continue to fire normally.
pub struct FileReadPlan {
    /// Contiguous (absolute_lba, sector_count) runs covering the file.
    pub runs: Vec<(u32, u32)>,
    /// Actual file size in bytes (sectors may extend past this).
    pub file_size: u32,
}

impl FileReadPlan {
    /// Execute the read plan -- reads sectors directly from the storage backend.
    ///
    /// **Must be called WITHOUT the VFS lock held** so that interrupts remain
    /// enabled during disk I/O.
    pub fn execute(&self) -> Result<Vec<u8>, FsError> {
        if self.file_size == 0 {
            return Ok(Vec::new());
        }
        // Total sector bytes may exceed file_size (last cluster is partial).
        let total_sector_bytes: usize =
            self.runs.iter().map(|(_, sc)| *sc as usize * 512).sum();
        let mut buf = vec![0u8; total_sector_bytes];
        let mut offset = 0usize;

        for &(abs_lba, sector_count) in &self.runs {
            let bytes = sector_count as usize * 512;
            if !FatFs::storage_read_sectors(abs_lba, sector_count,
                    &mut buf[offset..offset + bytes]) {
                return Err(FsError::IoError);
            }
            offset += bytes;
        }

        buf.truncate(self.file_size as usize);
        Ok(buf)
    }
}

impl FatFs {
    /// Read up to `buf.len()` bytes from a file starting at the given cluster and byte offset.
    ///
    /// Optimized: batches contiguous clusters into single multi-sector reads
    /// and uses the in-memory FAT cache for O(1) cluster chain lookups.
    pub fn read_file(&self, start_cluster: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        if start_cluster < 2 || buf.is_empty() {
            return Ok(0);
        }
        let spc = self.sectors_per_cluster;
        let cluster_size = spc * 512;
        let mut cluster = start_cluster;

        // Skip clusters before our offset (O(1) per lookup with FAT cache)
        let mut bytes_skipped = 0u32;
        while bytes_skipped + cluster_size <= offset {
            bytes_skipped += cluster_size;
            match self.next_cluster(cluster) {
                Some(next) => cluster = next,
                None => return Ok(0),
            }
        }

        let mut bytes_read = 0usize;

        loop {
            // How many bytes we still need, plus any intra-cluster offset
            let start_in_run = if bytes_skipped < offset {
                (offset - bytes_skipped) as usize
            } else {
                0
            };
            let bytes_needed = buf.len() - bytes_read + start_in_run;
            let max_clusters = ((bytes_needed as u32 + cluster_size - 1) / cluster_size).max(1);

            // Scan ahead for contiguous clusters, capped to what we need
            let run_start_lba = self.cluster_to_lba(cluster);
            let mut run_clusters: u32 = 1;
            let mut last_cluster = cluster;

            while run_clusters < max_clusters {
                match self.next_cluster(last_cluster) {
                    Some(next) if next == last_cluster + 1 => {
                        run_clusters += 1;
                        last_cluster = next;
                    }
                    _ => break,
                }
            }

            // Read only the clusters we need
            let run_bytes = (run_clusters * cluster_size) as usize;
            let available = run_bytes - start_in_run;
            let to_copy = available.min(buf.len() - bytes_read);

            // Read directly into output buffer when possible (aligned, full run)
            if start_in_run == 0 && to_copy == run_bytes {
                let total_sectors = run_clusters * spc;
                self.read_sectors(run_start_lba, total_sectors,
                    &mut buf[bytes_read..bytes_read + run_bytes])?;
            } else {
                // Partial run -- read into temp buffer
                let mut tmp = vec![0u8; run_bytes];
                let total_sectors = run_clusters * spc;
                self.read_sectors(run_start_lba, total_sectors, &mut tmp)?;
                buf[bytes_read..bytes_read + to_copy]
                    .copy_from_slice(&tmp[start_in_run..start_in_run + to_copy]);
            }

            bytes_read += to_copy;
            bytes_skipped += run_clusters * cluster_size;

            if bytes_read >= buf.len() {
                break;
            }

            // Advance to the next (non-contiguous) cluster
            match self.next_cluster(last_cluster) {
                Some(next) => cluster = next,
                None => break,
            }
        }

        Ok(bytes_read)
    }

    /// Read an entire file into a new `Vec<u8>`.
    pub fn read_file_all(&self, start_cluster: u32, file_size: u32) -> Result<Vec<u8>, FsError> {
        if file_size == 0 || start_cluster < 2 {
            return Ok(Vec::new());
        }
        let mut buf = vec![0u8; file_size as usize];
        let bytes_read = self.read_file(start_cluster, 0, &mut buf)?;
        buf.truncate(bytes_read);
        Ok(buf)
    }

    /// Build a read plan for the given file by walking the in-memory FAT cache.
    ///
    /// This collects contiguous (absolute_lba, sector_count) runs without
    /// touching the disk, so it is safe to call while holding the VFS spinlock.
    pub fn get_file_read_plan(&self, start_cluster: u32, file_size: u32) -> FileReadPlan {
        let spc = self.sectors_per_cluster;
        let mut runs = Vec::new();

        if file_size == 0 || start_cluster < 2 {
            return FileReadPlan { runs, file_size };
        }

        let mut cluster = start_cluster;
        loop {
            let run_start_lba = self.partition_start_lba + self.cluster_to_lba(cluster);
            let mut run_clusters: u32 = 1;
            let mut last_cluster = cluster;

            // Extend the run with physically contiguous clusters
            while let Some(next) = self.next_cluster(last_cluster) {
                if next == last_cluster + 1 {
                    run_clusters += 1;
                    last_cluster = next;
                } else {
                    break;
                }
            }

            runs.push((run_start_lba, run_clusters * spc));

            // Advance to the next (non-contiguous) cluster, or stop
            match self.next_cluster(last_cluster) {
                Some(next) => cluster = next,
                None => break,
            }
        }

        FileReadPlan { runs, file_size }
    }

    /// Write data to a file at the given offset, allocating clusters as needed.
    /// Returns `(first_cluster, new_size)`.
    pub fn write_file(&mut self, start_cluster: u32, offset: u32, data: &[u8], old_size: u32) -> Result<(u32, u32), FsError> {
        if data.is_empty() {
            return Ok((start_cluster, old_size));
        }
        let cluster_size = self.sectors_per_cluster * 512;
        let first_cluster = if start_cluster < 2 {
            self.alloc_cluster()?
        } else {
            start_cluster
        };

        let mut cluster = first_cluster;
        let mut cluster_offset = 0u32;
        while cluster_offset + cluster_size <= offset {
            cluster_offset += cluster_size;
            let next = self.read_fat_entry(cluster)?;
            if self.is_eoc(next) || next == 0 {
                let new = self.alloc_cluster()?;
                self.write_fat_entry(cluster, new)?;
                let zeros = vec![0u8; cluster_size as usize];
                self.write_cluster(new, &zeros)?;
                cluster = new;
            } else {
                cluster = next;
            }
        }

        let mut data_written = 0usize;
        let mut cur_cluster = cluster;
        loop {
            let start_in_cluster = if cluster_offset < offset {
                (offset - cluster_offset) as usize
            } else {
                0
            };
            let space_in_cluster = cluster_size as usize - start_in_cluster;
            let to_write = space_in_cluster.min(data.len() - data_written);
            let mut cluster_buf = vec![0u8; cluster_size as usize];
            self.read_cluster(cur_cluster, &mut cluster_buf)?;
            cluster_buf[start_in_cluster..start_in_cluster + to_write]
                .copy_from_slice(&data[data_written..data_written + to_write]);
            self.write_cluster(cur_cluster, &cluster_buf)?;
            data_written += to_write;
            cluster_offset += cluster_size;
            if data_written >= data.len() {
                break;
            }
            let next = self.read_fat_entry(cur_cluster)?;
            if self.is_eoc(next) || next == 0 {
                let new = self.alloc_cluster()?;
                self.write_fat_entry(cur_cluster, new)?;
                let zeros = vec![0u8; cluster_size as usize];
                self.write_cluster(new, &zeros)?;
                cur_cluster = new;
            } else {
                cur_cluster = next;
            }
        }
        let new_size = (offset + data.len() as u32).max(old_size);
        Ok((first_cluster, new_size))
    }
}
