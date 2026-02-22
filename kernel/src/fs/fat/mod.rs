//! FAT12/16/32 filesystem driver with VFAT long filename (LFN) support.
//!
//! Submodules:
//! - [`bpb`]: Boot Parameter Block structures and constants
//! - [`datetime`]: DOS datetime conversion
//! - [`lfn`]: VFAT long filename support
//! - [`table`]: FAT table operations (cluster allocation, chain management)
//! - [`file`]: File read/write operations
//! - [`dir`]: Directory operations (listing, lookup, create, delete)

pub mod bpb;
pub mod datetime;
pub mod lfn;
mod table;
mod file;
mod dir;

pub use bpb::*;
pub use datetime::{dos_datetime_to_unix, unix_to_dos_datetime};
pub use file::FileReadPlan;

use crate::fs::vfs::FsError;
use alloc::vec;
use alloc::vec::Vec;

/// In-memory representation of a mounted FAT filesystem with cached BPB parameters.
pub struct FatFs {
    pub device_id: u32,
    pub fat_type: FatType,
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    pub first_data_sector: u32,
    pub root_dir_sectors: u32,
    pub first_fat_sector: u32,
    pub first_root_dir_sector: u32,
    pub total_clusters: u32,
    pub partition_start_lba: u32,
    /// FAT size in sectors (works for FAT12/16/32).
    pub fat_size: u32,
    pub num_fats: u32,
    /// FAT32: root directory start cluster (0 for FAT12/16).
    pub root_cluster: u32,
    /// FAT32: FSInfo sector number (0 for FAT12/16).
    pub fsinfo_sector: u32,
    /// Cached FAT table in memory for fast cluster chain lookups.
    /// For FAT12/16: entire FAT. For FAT32: entire FAT (up to ~4 MB).
    pub(crate) fat_cache: Vec<u8>,
}

/// FAT variant detected from the cluster count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatType {
    /// FAT12 (fewer than 4085 clusters).
    Fat12,
    /// FAT16 (4085 to 65524 clusters).
    Fat16,
    /// FAT32 (65525 or more clusters).
    Fat32,
}

impl FatFs {
    /// Create a new FAT filesystem by reading the BPB from the ATA device.
    /// Supports FAT12, FAT16, and FAT32 (including extended BPB fields).
    pub fn new(device_id: u32, partition_start_lba: u32) -> Result<Self, FsError> {
        let mut buf = [0u8; 512];
        if !crate::drivers::storage::read_sectors(partition_start_lba, 1, &mut buf) {
            crate::serial_println!("  FAT: Failed to read boot sector at LBA {}", partition_start_lba);
            return Err(FsError::IoError);
        }

        let bytes_per_sector = u16::from_le_bytes([buf[11], buf[12]]) as u32;
        let sectors_per_cluster = buf[13] as u32;
        let reserved_sectors = u16::from_le_bytes([buf[14], buf[15]]) as u32;
        let num_fats = buf[16] as u32;
        let root_entry_count = u16::from_le_bytes([buf[17], buf[18]]) as u32;
        let total_sectors_16 = u16::from_le_bytes([buf[19], buf[20]]) as u32;
        let fat_size_16 = u16::from_le_bytes([buf[22], buf[23]]) as u32;
        let total_sectors_32 = u32::from_le_bytes([buf[32], buf[33], buf[34], buf[35]]);

        // FAT32 extended BPB fields (only valid when fat_size_16 == 0)
        let fat_size_32 = u32::from_le_bytes([buf[36], buf[37], buf[38], buf[39]]);
        let fat_size = if fat_size_16 != 0 { fat_size_16 } else { fat_size_32 };

        let total_sectors = if total_sectors_16 != 0 { total_sectors_16 } else { total_sectors_32 };

        if bytes_per_sector != 512 {
            crate::serial_println!("  FAT: Unsupported sector size: {}", bytes_per_sector);
            return Err(FsError::IoError);
        }
        if sectors_per_cluster == 0 || fat_size == 0 {
            crate::serial_println!("  FAT: Invalid BPB (spc={}, fat_size={})", sectors_per_cluster, fat_size);
            return Err(FsError::IoError);
        }

        let root_dir_sectors = (root_entry_count * 32 + bytes_per_sector - 1) / bytes_per_sector;
        let first_fat_sector = reserved_sectors;
        let first_root_dir_sector = reserved_sectors + num_fats * fat_size;
        let first_data_sector = first_root_dir_sector + root_dir_sectors;
        let data_sectors = total_sectors - first_data_sector;
        let total_clusters = data_sectors / sectors_per_cluster;

        let fat_type = if total_clusters < 4085 {
            FatType::Fat12
        } else if total_clusters < 65525 {
            FatType::Fat16
        } else {
            FatType::Fat32
        };

        // FAT32-specific fields
        let root_cluster = if fat_type == FatType::Fat32 {
            u32::from_le_bytes([buf[44], buf[45], buf[46], buf[47]])
        } else {
            0
        };
        let fsinfo_sector = if fat_type == FatType::Fat32 {
            u16::from_le_bytes([buf[48], buf[49]]) as u32
        } else {
            0
        };

        let oem = core::str::from_utf8(&buf[3..11]).unwrap_or("?");

        crate::serial_println!(
            "[OK] FAT{} filesystem: {} clusters, {} sec/cluster, OEM='{}'",
            match fat_type { FatType::Fat12 => "12", FatType::Fat16 => "16", FatType::Fat32 => "32" },
            total_clusters, sectors_per_cluster, oem.trim(),
        );
        if fat_type == FatType::Fat32 {
            crate::serial_println!(
                "  FAT32: root_cluster={}, fsinfo_sector={}, fat_size={} sectors",
                root_cluster, fsinfo_sector, fat_size,
            );
        }
        crate::serial_println!(
            "  FAT: first_fat={}, root_dir={}, data={}, total_sectors={}",
            first_fat_sector, first_root_dir_sector, first_data_sector, total_sectors
        );

        // Cache the entire FAT table in memory for fast cluster chain lookups.
        // FAT16: ~64 KB typical. FAT32: up to ~4 MB for a 4 GB disk (4KB clusters).
        let fat_cache_size = (fat_size * 512) as usize;
        if fat_cache_size > 8 * 1024 * 1024 {
            crate::serial_println!("  FAT: FAT table too large to cache ({} KB)", fat_cache_size / 1024);
            return Err(FsError::IoError);
        }
        let mut fat_cache = vec![0u8; fat_cache_size];
        let abs_fat_lba = partition_start_lba + first_fat_sector;
        if !crate::drivers::storage::read_sectors(abs_fat_lba, fat_size, &mut fat_cache) {
            crate::serial_println!("  FAT: Failed to cache FAT table");
            return Err(FsError::IoError);
        }
        crate::serial_println!("  FAT: cached {} KB FAT table in memory", fat_cache_size / 1024);

        Ok(FatFs {
            device_id,
            fat_type,
            bytes_per_sector,
            sectors_per_cluster,
            first_data_sector,
            root_dir_sectors,
            first_fat_sector,
            first_root_dir_sector,
            total_clusters,
            partition_start_lba,
            fat_size,
            num_fats,
            root_cluster,
            fsinfo_sector,
            fat_cache,
        })
    }

    // =================================================================
    // Low-level I/O (shared by all submodules)
    // =================================================================

    pub(crate) fn read_sectors(&self, relative_lba: u32, count: u32, buf: &mut [u8]) -> Result<(), FsError> {
        let abs_lba = self.partition_start_lba + relative_lba;
        if !crate::drivers::storage::read_sectors(abs_lba, count, buf) {
            return Err(FsError::IoError);
        }
        Ok(())
    }

    pub(crate) fn write_sectors(&self, relative_lba: u32, count: u32, buf: &[u8]) -> Result<(), FsError> {
        let abs_lba = self.partition_start_lba + relative_lba;
        if !crate::drivers::storage::write_sectors(abs_lba, count, buf) {
            return Err(FsError::IoError);
        }
        Ok(())
    }

    pub(crate) fn cluster_to_lba(&self, cluster: u32) -> u32 {
        self.first_data_sector + (cluster - 2) * self.sectors_per_cluster
    }

    pub(crate) fn read_cluster(&self, cluster: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        let lba = self.cluster_to_lba(cluster);
        let size = self.sectors_per_cluster * 512;
        self.read_sectors(lba, self.sectors_per_cluster, &mut buf[..size as usize])?;
        Ok(size as usize)
    }

    pub(crate) fn write_cluster(&self, cluster: u32, data: &[u8]) -> Result<(), FsError> {
        let lba = self.cluster_to_lba(cluster);
        let cluster_size = (self.sectors_per_cluster * 512) as usize;
        if data.len() >= cluster_size {
            self.write_sectors(lba, self.sectors_per_cluster, &data[..cluster_size])
        } else {
            let mut buf = vec![0u8; cluster_size];
            buf[..data.len()].copy_from_slice(data);
            self.write_sectors(lba, self.sectors_per_cluster, &buf)
        }
    }
}
