//! FAT table operations: reading/writing FAT entries, cluster allocation, chain management.

use crate::fs::vfs::FsError;
use super::{FatFs, FatType};
use super::bpb::FAT32_MASK;

impl FatFs {
    /// Follow the cluster chain: returns the next cluster, or None if end-of-chain.
    pub(crate) fn next_cluster(&self, cluster: u32) -> Option<u32> {
        match self.read_fat_entry(cluster) {
            Ok(val) if val == 0 || self.is_eoc(val) => None,
            Ok(val) => Some(val),
            Err(_) => None,
        }
    }

    /// Check if a FAT entry value indicates end-of-chain.
    pub(crate) fn is_eoc(&self, value: u32) -> bool {
        match self.fat_type {
            FatType::Fat12 => value >= 0x0FF8,
            FatType::Fat16 => value >= 0xFFF8,
            FatType::Fat32 => value >= 0x0FFFFFF8,
        }
    }

    /// Get the end-of-chain marker for the current FAT type.
    pub(crate) fn eoc_mark(&self) -> u32 {
        match self.fat_type {
            FatType::Fat12 => 0x0FFF,
            FatType::Fat16 => 0xFFFF,
            FatType::Fat32 => 0x0FFFFFFF,
        }
    }

    /// Write a FAT entry value for a cluster. Handles FAT16 (2 bytes) and FAT32 (4 bytes).
    /// For FAT32, preserves the upper 4 bits of the existing entry.
    pub(crate) fn write_fat_entry(&mut self, cluster: u32, value: u32) -> Result<(), FsError> {
        let (fat_offset, entry_size) = match self.fat_type {
            FatType::Fat12 => {
                // FAT12 not writable for now
                return Err(FsError::IoError);
            }
            FatType::Fat16 => ((cluster * 2) as usize, 2usize),
            FatType::Fat32 => ((cluster * 4) as usize, 4usize),
        };

        if fat_offset + entry_size > self.fat_cache.len() {
            return Err(FsError::IoError);
        }

        // Update in-memory cache
        match self.fat_type {
            FatType::Fat16 => {
                let v16 = value as u16;
                self.fat_cache[fat_offset] = v16 as u8;
                self.fat_cache[fat_offset + 1] = (v16 >> 8) as u8;
            }
            FatType::Fat32 => {
                // Preserve upper 4 bits of existing entry
                let old = u32::from_le_bytes([
                    self.fat_cache[fat_offset],
                    self.fat_cache[fat_offset + 1],
                    self.fat_cache[fat_offset + 2],
                    self.fat_cache[fat_offset + 3],
                ]);
                let new_val = (old & 0xF0000000) | (value & FAT32_MASK);
                let bytes = new_val.to_le_bytes();
                self.fat_cache[fat_offset..fat_offset + 4].copy_from_slice(&bytes);
            }
            _ => {}
        }

        // Write through to disk (both FAT copies)
        let fat_sector_rel = fat_offset as u32 / 512;
        let sector_start = (fat_sector_rel * 512) as usize;
        let mut sector_buf = [0u8; 512];
        sector_buf.copy_from_slice(&self.fat_cache[sector_start..sector_start + 512]);

        let fat1_sector = self.first_fat_sector + fat_sector_rel;
        self.write_sectors(fat1_sector, 1, &sector_buf)?;

        if self.num_fats > 1 {
            let fat2_sector = self.first_fat_sector + self.fat_size + fat_sector_rel;
            self.write_sectors(fat2_sector, 1, &sector_buf)?;
        }
        Ok(())
    }

    /// Read a FAT entry for a cluster. Returns the entry value (masked for FAT32).
    pub(crate) fn read_fat_entry(&self, cluster: u32) -> Result<u32, FsError> {
        match self.fat_type {
            FatType::Fat12 => {
                // FAT12: 1.5 bytes per entry
                let fat_offset = (cluster as usize * 3) / 2;
                if fat_offset + 1 >= self.fat_cache.len() {
                    return Err(FsError::IoError);
                }
                let raw = u16::from_le_bytes([
                    self.fat_cache[fat_offset],
                    self.fat_cache[fat_offset + 1],
                ]);
                let val = if cluster & 1 != 0 { raw >> 4 } else { raw & 0x0FFF };
                Ok(val as u32)
            }
            FatType::Fat16 => {
                let fat_offset = (cluster * 2) as usize;
                if fat_offset + 1 >= self.fat_cache.len() {
                    return Err(FsError::IoError);
                }
                Ok(u16::from_le_bytes([
                    self.fat_cache[fat_offset],
                    self.fat_cache[fat_offset + 1],
                ]) as u32)
            }
            FatType::Fat32 => {
                let fat_offset = (cluster * 4) as usize;
                if fat_offset + 3 >= self.fat_cache.len() {
                    return Err(FsError::IoError);
                }
                let raw = u32::from_le_bytes([
                    self.fat_cache[fat_offset],
                    self.fat_cache[fat_offset + 1],
                    self.fat_cache[fat_offset + 2],
                    self.fat_cache[fat_offset + 3],
                ]);
                Ok(raw & FAT32_MASK)
            }
        }
    }

    /// Allocate a free cluster, mark it as end-of-chain.
    pub(crate) fn alloc_cluster(&mut self) -> Result<u32, FsError> {
        for cluster in 2..self.total_clusters + 2 {
            let entry = self.read_fat_entry(cluster)?;
            if entry == 0 {
                self.write_fat_entry(cluster, self.eoc_mark())?;
                return Ok(cluster);
            }
        }
        Err(FsError::NoSpace)
    }

    /// Free an entire cluster chain starting at `start_cluster`.
    pub fn free_chain(&mut self, start_cluster: u32) -> Result<(), FsError> {
        if start_cluster < 2 {
            return Ok(());
        }
        let mut cluster = start_cluster;
        loop {
            let next = self.read_fat_entry(cluster)?;
            self.write_fat_entry(cluster, 0)?;
            if self.is_eoc(next) || next == 0 {
                break;
            }
            cluster = next;
        }
        Ok(())
    }
}
