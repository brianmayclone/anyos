//! FAT16 filesystem driver with VFAT long filename (LFN) support.
//! Reads and writes files/directories on an ATA-backed FAT16 partition.

use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::FsError;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// FAT16 BIOS Parameter Block (BPB) layout at the start of the partition.
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct FatBootSector {
    pub jump: [u8; 3],
    pub oem_name: [u8; 8],
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub num_fats: u8,
    pub root_entry_count: u16,
    pub total_sectors_16: u16,
    pub media_type: u8,
    pub fat_size_16: u16,
    pub sectors_per_track: u16,
    pub num_heads: u16,
    pub hidden_sectors: u32,
    pub total_sectors_32: u32,
}

/// A 32-byte FAT directory entry (8.3 short name format).
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct FatDirEntry {
    pub name: [u8; 8],
    pub ext: [u8; 3],
    pub attr: u8,
    pub reserved: u8,
    pub create_time_tenth: u8,
    pub create_time: u16,
    pub create_date: u16,
    pub last_access_date: u16,
    pub first_cluster_hi: u16,
    pub write_time: u16,
    pub write_date: u16,
    pub first_cluster_lo: u16,
    pub file_size: u32,
}

/// FAT directory entry attribute: read-only file.
pub const ATTR_READ_ONLY: u8 = 0x01;
/// FAT directory entry attribute: hidden file.
pub const ATTR_HIDDEN: u8 = 0x02;
/// FAT directory entry attribute: system file.
pub const ATTR_SYSTEM: u8 = 0x04;
/// FAT directory entry attribute: volume label.
pub const ATTR_VOLUME_ID: u8 = 0x08;
/// FAT directory entry attribute: subdirectory.
pub const ATTR_DIRECTORY: u8 = 0x10;
/// FAT directory entry attribute: archive (modified since backup).
pub const ATTR_ARCHIVE: u8 = 0x20;
/// FAT directory entry attribute mask for VFAT long filename entries.
pub const ATTR_LONG_NAME: u8 = 0x0F;

const FAT16_EOC: u16 = 0xFFF8;

// =====================================================================
// VFAT Long Filename (LFN) helpers
// =====================================================================

/// Compute the VFAT checksum of an 8.3 name (11 bytes).
fn lfn_checksum(name83: &[u8]) -> u8 {
    let mut sum: u8 = 0;
    for i in 0..11 {
        sum = ((sum & 1) << 7).wrapping_add(sum >> 1).wrapping_add(name83[i]);
    }
    sum
}

/// Extract 13 UTF-16LE characters from a 32-byte LFN directory entry.
fn lfn_extract_chars(entry: &[u8]) -> [u16; 13] {
    let mut chars = [0u16; 13];
    // Chars 1-5: bytes 1..10
    for j in 0..5 {
        chars[j] = u16::from_le_bytes([entry[1 + j * 2], entry[2 + j * 2]]);
    }
    // Chars 6-11: bytes 14..25
    for j in 0..6 {
        chars[5 + j] = u16::from_le_bytes([entry[14 + j * 2], entry[15 + j * 2]]);
    }
    // Chars 12-13: bytes 28..31
    for j in 0..2 {
        chars[11 + j] = u16::from_le_bytes([entry[28 + j * 2], entry[29 + j * 2]]);
    }
    chars
}

/// Convert a UTF-16LE LFN buffer to a String (ASCII only).
fn lfn_to_string(buf: &[u16], max_len: usize) -> String {
    let mut s = String::new();
    for i in 0..max_len {
        let c = buf[i];
        if c == 0x0000 || c == 0xFFFF {
            break;
        }
        if c < 128 {
            s.push(c as u8 as char);
        } else {
            s.push('?');
        }
    }
    s
}

/// Check if an accumulated LFN buffer matches a name (case-insensitive).
fn lfn_name_matches(lfn_buf: &[u16], lfn_max: usize, name: &str) -> bool {
    let mut lfn_len = 0;
    for i in 0..lfn_max {
        if lfn_buf[i] == 0x0000 || lfn_buf[i] == 0xFFFF {
            break;
        }
        lfn_len += 1;
    }
    if lfn_len != name.len() {
        return false;
    }
    for (i, b) in name.bytes().enumerate() {
        if i >= lfn_max {
            return false;
        }
        let lfn_char = lfn_buf[i];
        if lfn_char >= 128 {
            return false;
        }
        if (lfn_char as u8).to_ascii_lowercase() != b.to_ascii_lowercase() {
            return false;
        }
    }
    true
}

/// Check if a filename requires LFN entries (doesn't fit 8.3 format).
fn needs_lfn(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 {
        return true;
    }
    if name.starts_with('.') && name != "." && name != ".." {
        return true;
    }
    let dot_count = name.bytes().filter(|&b| b == b'.').count();
    if dot_count > 1 {
        return true;
    }
    let (base, ext) = if let Some(dot_pos) = name.find('.') {
        (&name[..dot_pos], &name[dot_pos + 1..])
    } else {
        (name, "")
    };
    if base.len() > 8 || ext.len() > 3 {
        return true;
    }
    for b in name.bytes() {
        match b {
            b' ' | b'+' | b',' | b';' | b'=' | b'[' | b']' => return true,
            _ => {}
        }
    }
    false
}

/// Store 13 UTF-16LE characters into a 32-byte LFN entry buffer.
fn lfn_store_chars(entry: &mut [u8], chars: &[u16; 13]) {
    for j in 0..5 {
        let bytes = chars[j].to_le_bytes();
        entry[1 + j * 2] = bytes[0];
        entry[2 + j * 2] = bytes[1];
    }
    for j in 0..6 {
        let bytes = chars[5 + j].to_le_bytes();
        entry[14 + j * 2] = bytes[0];
        entry[15 + j * 2] = bytes[1];
    }
    for j in 0..2 {
        let bytes = chars[11 + j].to_le_bytes();
        entry[28 + j * 2] = bytes[0];
        entry[29 + j * 2] = bytes[1];
    }
}

/// Information about a found directory entry.
struct FoundEntry {
    /// Byte offset of the 8.3 entry in the directory data buffer
    offset: usize,
    /// Byte offset of the first LFN entry (if any)
    lfn_start: Option<usize>,
    /// Starting cluster
    cluster: u32,
    /// File size
    size: u32,
    /// Is a directory
    is_dir: bool,
}

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
    pub fat_size_16: u32,
    pub num_fats: u32,
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
    pub fn new(device_id: u32, partition_start_lba: u32) -> Result<Self, FsError> {
        let mut buf = [0u8; 512];
        if !crate::drivers::storage::ata::read_sectors(partition_start_lba, 1, &mut buf) {
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

        let total_sectors = if total_sectors_16 != 0 { total_sectors_16 } else { total_sectors_32 };

        if bytes_per_sector != 512 {
            crate::serial_println!("  FAT: Unsupported sector size: {}", bytes_per_sector);
            return Err(FsError::IoError);
        }
        if sectors_per_cluster == 0 || fat_size_16 == 0 {
            crate::serial_println!("  FAT: Invalid BPB (spc={}, fat_size={})", sectors_per_cluster, fat_size_16);
            return Err(FsError::IoError);
        }

        let root_dir_sectors = (root_entry_count * 32 + bytes_per_sector - 1) / bytes_per_sector;
        let first_fat_sector = reserved_sectors;
        let first_root_dir_sector = reserved_sectors + num_fats * fat_size_16;
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

        let oem = core::str::from_utf8(&buf[3..11]).unwrap_or("?");

        crate::serial_println!(
            "[OK] FAT{} filesystem: {} clusters, {} sec/cluster, OEM='{}'",
            match fat_type { FatType::Fat12 => "12", FatType::Fat16 => "16", FatType::Fat32 => "32" },
            total_clusters, sectors_per_cluster, oem.trim(),
        );
        crate::serial_println!(
            "  FAT: first_fat={}, root_dir={}, data={}, total_sectors={}",
            first_fat_sector, first_root_dir_sector, first_data_sector, total_sectors
        );

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
            fat_size_16,
            num_fats,
        })
    }

    // =====================================================================
    // Low-level I/O
    // =====================================================================

    fn read_sectors(&self, relative_lba: u32, count: u32, buf: &mut [u8]) -> Result<(), FsError> {
        let abs_lba = self.partition_start_lba + relative_lba;
        let mut offset = 0usize;
        let mut remaining = count;
        let mut lba = abs_lba;
        while remaining > 0 {
            let batch = remaining.min(255) as u8;
            if !crate::drivers::storage::ata::read_sectors(lba, batch, &mut buf[offset..]) {
                return Err(FsError::IoError);
            }
            offset += batch as usize * 512;
            lba += batch as u32;
            remaining -= batch as u32;
        }
        Ok(())
    }

    fn write_sectors(&self, relative_lba: u32, count: u32, buf: &[u8]) -> Result<(), FsError> {
        let abs_lba = self.partition_start_lba + relative_lba;
        let mut offset = 0usize;
        let mut remaining = count;
        let mut lba = abs_lba;
        while remaining > 0 {
            let batch = remaining.min(255) as u8;
            if !crate::drivers::storage::ata::write_sectors(lba, batch, &buf[offset..]) {
                return Err(FsError::IoError);
            }
            offset += batch as usize * 512;
            lba += batch as u32;
            remaining -= batch as u32;
        }
        Ok(())
    }

    fn cluster_to_lba(&self, cluster: u32) -> u32 {
        self.first_data_sector + (cluster - 2) * self.sectors_per_cluster
    }

    fn next_cluster(&self, cluster: u32) -> Option<u32> {
        let fat_offset = cluster * 2;
        let fat_sector = self.first_fat_sector + fat_offset / 512;
        let offset_in_sector = (fat_offset % 512) as usize;
        let mut sector_buf = [0u8; 512];
        if self.read_sectors(fat_sector, 1, &mut sector_buf).is_err() {
            return None;
        }
        let value = u16::from_le_bytes([
            sector_buf[offset_in_sector],
            sector_buf[offset_in_sector + 1],
        ]);
        if value >= FAT16_EOC || value == 0 {
            None
        } else {
            Some(value as u32)
        }
    }

    fn read_cluster(&self, cluster: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        let lba = self.cluster_to_lba(cluster);
        let size = self.sectors_per_cluster * 512;
        self.read_sectors(lba, self.sectors_per_cluster, &mut buf[..size as usize])?;
        Ok(size as usize)
    }

    fn write_cluster(&self, cluster: u32, data: &[u8]) -> Result<(), FsError> {
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

    // =====================================================================
    // FAT table operations
    // =====================================================================

    fn write_fat_entry(&self, cluster: u32, value: u16) -> Result<(), FsError> {
        let fat_offset = cluster * 2;
        let fat_sector_rel = fat_offset / 512;
        let offset_in_sector = (fat_offset % 512) as usize;
        let mut sector_buf = [0u8; 512];

        let fat1_sector = self.first_fat_sector + fat_sector_rel;
        self.read_sectors(fat1_sector, 1, &mut sector_buf)?;
        sector_buf[offset_in_sector] = value as u8;
        sector_buf[offset_in_sector + 1] = (value >> 8) as u8;
        self.write_sectors(fat1_sector, 1, &sector_buf)?;

        if self.num_fats > 1 {
            let fat2_sector = self.first_fat_sector + self.fat_size_16 + fat_sector_rel;
            self.read_sectors(fat2_sector, 1, &mut sector_buf)?;
            sector_buf[offset_in_sector] = value as u8;
            sector_buf[offset_in_sector + 1] = (value >> 8) as u8;
            self.write_sectors(fat2_sector, 1, &sector_buf)?;
        }
        Ok(())
    }

    fn read_fat_entry(&self, cluster: u32) -> Result<u16, FsError> {
        let fat_offset = cluster * 2;
        let fat_sector = self.first_fat_sector + fat_offset / 512;
        let offset_in_sector = (fat_offset % 512) as usize;
        let mut sector_buf = [0u8; 512];
        self.read_sectors(fat_sector, 1, &mut sector_buf)?;
        Ok(u16::from_le_bytes([
            sector_buf[offset_in_sector],
            sector_buf[offset_in_sector + 1],
        ]))
    }

    fn alloc_cluster(&self) -> Result<u32, FsError> {
        for cluster in 2..self.total_clusters + 2 {
            let entry = self.read_fat_entry(cluster)?;
            if entry == 0x0000 {
                self.write_fat_entry(cluster, 0xFFFF)?;
                return Ok(cluster);
            }
        }
        Err(FsError::NoSpace)
    }

    /// Free an entire cluster chain starting at `start_cluster`.
    pub fn free_chain(&self, start_cluster: u32) -> Result<(), FsError> {
        if start_cluster < 2 {
            return Ok(());
        }
        let mut cluster = start_cluster;
        loop {
            let next = self.read_fat_entry(cluster)?;
            self.write_fat_entry(cluster, 0x0000)?;
            if next >= FAT16_EOC || next == 0 {
                break;
            }
            cluster = next as u32;
        }
        Ok(())
    }

    // =====================================================================
    // File read/write
    // =====================================================================

    /// Read up to `buf.len()` bytes from a file starting at the given cluster and byte offset.
    pub fn read_file(&self, start_cluster: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        if start_cluster < 2 {
            return Ok(0);
        }
        let cluster_size = self.sectors_per_cluster * 512;
        let mut cluster = start_cluster;
        let mut bytes_skipped = 0u32;
        let mut bytes_read = 0usize;

        while bytes_skipped + cluster_size <= offset {
            bytes_skipped += cluster_size;
            match self.next_cluster(cluster) {
                Some(next) => cluster = next,
                None => return Ok(0),
            }
        }

        let mut cluster_buf = vec![0u8; cluster_size as usize];
        loop {
            self.read_cluster(cluster, &mut cluster_buf)?;
            let start_in_cluster = if bytes_skipped < offset {
                (offset - bytes_skipped) as usize
            } else {
                0
            };
            bytes_skipped += cluster_size;
            let available = cluster_size as usize - start_in_cluster;
            let to_copy = available.min(buf.len() - bytes_read);
            buf[bytes_read..bytes_read + to_copy]
                .copy_from_slice(&cluster_buf[start_in_cluster..start_in_cluster + to_copy]);
            bytes_read += to_copy;
            if bytes_read >= buf.len() {
                break;
            }
            match self.next_cluster(cluster) {
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

    /// Write data to a file at the given offset, allocating clusters as needed.
    /// Returns `(first_cluster, new_size)`.
    pub fn write_file(&self, start_cluster: u32, offset: u32, data: &[u8], old_size: u32) -> Result<(u32, u32), FsError> {
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
            if next >= FAT16_EOC || next == 0 {
                let new = self.alloc_cluster()?;
                self.write_fat_entry(cluster, new as u16)?;
                let zeros = vec![0u8; cluster_size as usize];
                self.write_cluster(new, &zeros)?;
                cluster = new;
            } else {
                cluster = next as u32;
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
            if next >= FAT16_EOC || next == 0 {
                let new = self.alloc_cluster()?;
                self.write_fat_entry(cur_cluster, new as u16)?;
                let zeros = vec![0u8; cluster_size as usize];
                self.write_cluster(new, &zeros)?;
                cur_cluster = new;
            } else {
                cur_cluster = next as u32;
            }
        }
        let new_size = (offset + data.len() as u32).max(old_size);
        Ok((first_cluster, new_size))
    }

    // =====================================================================
    // Directory reading — raw data
    // =====================================================================

    fn read_dir_raw(&self, cluster: u32) -> Result<Vec<u8>, FsError> {
        if cluster == 0 {
            let root_size = self.root_dir_sectors * 512;
            let mut buf = vec![0u8; root_size as usize];
            self.read_sectors(self.first_root_dir_sector, self.root_dir_sectors, &mut buf)?;
            Ok(buf)
        } else {
            let cluster_size = self.sectors_per_cluster * 512;
            let mut result = Vec::new();
            let mut cur = cluster;
            loop {
                let mut cbuf = vec![0u8; cluster_size as usize];
                self.read_cluster(cur, &mut cbuf)?;
                result.extend_from_slice(&cbuf);
                match self.next_cluster(cur) {
                    Some(next) => cur = next,
                    None => break,
                }
            }
            Ok(result)
        }
    }

    // =====================================================================
    // 8.3 name handling
    // =====================================================================

    fn parse_83_name(&self, raw: &[u8]) -> String {
        let base_end = raw[0..8].iter().rposition(|&b| b != b' ').map_or(0, |p| p + 1);
        let base = core::str::from_utf8(&raw[..base_end]).unwrap_or("");
        let ext_end = raw[8..11].iter().rposition(|&b| b != b' ').map_or(0, |p| p + 1);
        let ext = core::str::from_utf8(&raw[8..8 + ext_end]).unwrap_or("");
        let mut name = String::new();
        for c in base.chars() {
            name.push(c.to_ascii_lowercase());
        }
        if !ext.is_empty() {
            name.push('.');
            for c in ext.chars() {
                name.push(c.to_ascii_lowercase());
            }
        }
        name
    }

    fn name_matches(&self, raw_name: &[u8], filename: &str) -> bool {
        let filename_upper: Vec<u8> = filename.bytes().map(|b| b.to_ascii_uppercase()).collect();
        let (base, ext) = if let Some(dot_pos) = filename_upper.iter().position(|&b| b == b'.') {
            (&filename_upper[..dot_pos], &filename_upper[dot_pos + 1..])
        } else {
            (&filename_upper[..], &[][..])
        };
        for i in 0..8 {
            let raw_byte = raw_name[i].to_ascii_uppercase();
            let expected = if i < base.len() { base[i] } else { b' ' };
            if raw_byte != expected {
                return false;
            }
        }
        for i in 0..3 {
            let raw_byte = raw_name[8 + i].to_ascii_uppercase();
            let expected = if i < ext.len() { ext[i] } else { b' ' };
            if raw_byte != expected {
                return false;
            }
        }
        true
    }

    fn make_83_name(filename: &str) -> [u8; 11] {
        let mut result = [b' '; 11];
        let upper: Vec<u8> = filename.bytes().map(|b| b.to_ascii_uppercase()).collect();
        let (base, ext) = if let Some(dot_pos) = upper.iter().position(|&b| b == b'.') {
            (&upper[..dot_pos], &upper[dot_pos + 1..])
        } else {
            (&upper[..], &[][..])
        };
        for (i, &b) in base.iter().enumerate().take(8) {
            result[i] = b;
        }
        for (i, &b) in ext.iter().enumerate().take(3) {
            result[8 + i] = b;
        }
        result
    }

    // =====================================================================
    // LFN-aware entry finding
    // =====================================================================

    /// Find a directory entry by name in a raw directory buffer.
    /// Supports both 8.3 names and VFAT long filenames.
    fn find_entry_in_buf(&self, buf: &[u8], name: &str) -> Option<FoundEntry> {
        let mut lfn_chars = [0u16; 260];
        let mut lfn_len: usize = 0;
        let mut lfn_chksum: u8 = 0;
        let mut lfn_valid = false;
        let mut lfn_start_offset: Option<usize> = None;

        let mut i = 0;
        while i + 32 <= buf.len() {
            let first_byte = buf[i];
            if first_byte == 0x00 {
                break;
            }
            if first_byte == 0xE5 {
                lfn_valid = false;
                lfn_start_offset = None;
                i += 32;
                continue;
            }

            let attr = buf[i + 11];

            if attr == ATTR_LONG_NAME {
                let seq = buf[i] & 0x3F;
                let is_last = buf[i] & 0x40 != 0;
                let chksum = buf[i + 13];

                if is_last {
                    lfn_chars = [0xFFFF; 260];
                    lfn_chksum = chksum;
                    lfn_valid = true;
                    lfn_len = seq as usize * 13;
                    lfn_start_offset = Some(i);
                } else if !lfn_valid || chksum != lfn_chksum {
                    lfn_valid = false;
                    lfn_start_offset = None;
                }

                if lfn_valid && seq > 0 {
                    let chars = lfn_extract_chars(&buf[i..i + 32]);
                    let start = (seq as usize - 1) * 13;
                    for j in 0..13 {
                        if start + j < 260 {
                            lfn_chars[start + j] = chars[j];
                        }
                    }
                }
                i += 32;
                continue;
            }

            if attr & ATTR_VOLUME_ID != 0 {
                lfn_valid = false;
                lfn_start_offset = None;
                i += 32;
                continue;
            }

            // Regular 8.3 entry — check for match
            let mut matches = false;

            if lfn_valid {
                if lfn_checksum(&buf[i..i + 11]) == lfn_chksum {
                    matches = lfn_name_matches(&lfn_chars, lfn_len, name);
                }
            }

            if !matches {
                matches = self.name_matches(&buf[i..i + 11], name);
            }

            let current_lfn_start = lfn_start_offset;
            lfn_valid = false;
            lfn_start_offset = None;

            if matches {
                let cluster_lo = u16::from_le_bytes([buf[i + 26], buf[i + 27]]) as u32;
                let cluster_hi = u16::from_le_bytes([buf[i + 20], buf[i + 21]]) as u32;
                let cluster = (cluster_hi << 16) | cluster_lo;
                let size = u32::from_le_bytes([buf[i + 28], buf[i + 29], buf[i + 30], buf[i + 31]]);
                return Some(FoundEntry {
                    offset: i,
                    lfn_start: current_lfn_start,
                    cluster,
                    size,
                    is_dir: attr & ATTR_DIRECTORY != 0,
                });
            }

            i += 32;
        }
        None
    }

    // =====================================================================
    // LFN-aware directory parsing (for read_dir)
    // =====================================================================

    fn parse_dir_entries(&self, buf: &[u8], entries: &mut Vec<DirEntry>) {
        let mut lfn_chars = [0u16; 260];
        let mut lfn_len: usize = 0;
        let mut lfn_chksum: u8 = 0;
        let mut lfn_valid = false;

        let mut i = 0;
        while i + 32 <= buf.len() {
            let first_byte = buf[i];
            if first_byte == 0x00 {
                break;
            }
            if first_byte == 0xE5 {
                lfn_valid = false;
                i += 32;
                continue;
            }

            let attr = buf[i + 11];

            if attr == ATTR_LONG_NAME {
                let seq = buf[i] & 0x3F;
                let is_last = buf[i] & 0x40 != 0;
                let chksum = buf[i + 13];

                if is_last {
                    lfn_chars = [0xFFFF; 260];
                    lfn_chksum = chksum;
                    lfn_valid = true;
                    lfn_len = seq as usize * 13;
                } else if !lfn_valid || chksum != lfn_chksum {
                    lfn_valid = false;
                }

                if lfn_valid && seq > 0 {
                    let chars = lfn_extract_chars(&buf[i..i + 32]);
                    let start = (seq as usize - 1) * 13;
                    for j in 0..13 {
                        if start + j < 260 {
                            lfn_chars[start + j] = chars[j];
                        }
                    }
                }
                i += 32;
                continue;
            }

            if attr & ATTR_VOLUME_ID != 0 {
                lfn_valid = false;
                i += 32;
                continue;
            }

            // Regular 8.3 entry — use LFN if valid, otherwise 8.3
            let name = if lfn_valid && lfn_checksum(&buf[i..i + 11]) == lfn_chksum {
                lfn_to_string(&lfn_chars, lfn_len)
            } else {
                self.parse_83_name(&buf[i..i + 11])
            };

            lfn_valid = false;

            if name == "." || name == ".." {
                i += 32;
                continue;
            }

            let file_type = if attr & ATTR_DIRECTORY != 0 {
                FileType::Directory
            } else {
                FileType::Regular
            };
            let file_size = u32::from_le_bytes([buf[i + 28], buf[i + 29], buf[i + 30], buf[i + 31]]);
            entries.push(DirEntry {
                name,
                file_type,
                size: file_size,
            });

            i += 32;
        }
    }

    // =====================================================================
    // Directory operations
    // =====================================================================

    /// List all directory entries (files and subdirectories) in the given directory cluster.
    pub fn read_dir(&self, cluster: u32) -> Result<Vec<DirEntry>, FsError> {
        let mut entries = Vec::new();
        let raw = self.read_dir_raw(cluster)?;
        self.parse_dir_entries(&raw, &mut entries);
        Ok(entries)
    }

    /// Look up a file/directory by path. Returns (start_cluster, file_type, file_size).
    pub fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Ok((0, FileType::Directory, 0));
        }

        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut current_cluster: u32 = 0;

        for (idx, component) in components.iter().enumerate() {
            let is_last = idx == components.len() - 1;
            let dir_data = self.read_dir_raw(current_cluster)?;

            match self.find_entry_in_buf(&dir_data, component) {
                Some(found) => {
                    if is_last {
                        let ft = if found.is_dir { FileType::Directory } else { FileType::Regular };
                        return Ok((found.cluster, ft, found.size));
                    } else if !found.is_dir {
                        return Err(FsError::NotADirectory);
                    } else {
                        current_cluster = found.cluster;
                    }
                }
                None => return Err(FsError::NotFound),
            }
        }

        Err(FsError::NotFound)
    }

    // =====================================================================
    // LFN entry generation (for create)
    // =====================================================================

    /// Generate an 8.3 short name for a long filename (e.g., "LONGFI~1.TXT").
    fn generate_short_name(name: &str) -> [u8; 11] {
        let mut result = [b' '; 11];

        let (base_str, ext_str) = if let Some(dot_pos) = name.rfind('.') {
            (&name[..dot_pos], &name[dot_pos + 1..])
        } else {
            (name, "")
        };

        // Filter to valid 8.3 chars and uppercase
        let base_clean: Vec<u8> = base_str
            .bytes()
            .filter(|&b| b != b' ' && b != b'.' && b != b'+' && b != b',' && b != b';' && b != b'=' && b != b'[' && b != b']')
            .map(|b| b.to_ascii_uppercase())
            .collect();

        let ext_clean: Vec<u8> = ext_str
            .bytes()
            .filter(|&b| b != b' ' && b != b'.')
            .map(|b| b.to_ascii_uppercase())
            .take(3)
            .collect();

        // Base: first 6 chars + ~1
        let base_take = base_clean.len().min(6);
        for i in 0..base_take {
            result[i] = base_clean[i];
        }
        result[base_take] = b'~';
        result[base_take + 1] = b'1';

        for (i, &b) in ext_clean.iter().enumerate() {
            result[8 + i] = b;
        }

        result
    }

    /// Build LFN directory entries for a name. Returns entries in disk order
    /// (last LFN entry first, sequence 1 last, ready to write before the 8.3 entry).
    fn make_lfn_entries(name: &str, name83: &[u8; 11]) -> Vec<[u8; 32]> {
        let chksum = lfn_checksum(name83);
        let utf16: Vec<u16> = name.bytes().map(|b| b as u16).collect();
        let num_entries = (utf16.len() + 12) / 13;

        let mut entries = Vec::with_capacity(num_entries);

        for seq in 1..=num_entries {
            let mut entry = [0u8; 32];
            let is_last = seq == num_entries;

            entry[0] = seq as u8 | if is_last { 0x40 } else { 0 };
            entry[11] = ATTR_LONG_NAME;
            entry[12] = 0;
            entry[13] = chksum;
            entry[26] = 0;
            entry[27] = 0;

            let start = (seq - 1) * 13;
            let mut chars = [0xFFFFu16; 13];
            for j in 0..13 {
                let idx = start + j;
                if idx < utf16.len() {
                    chars[j] = utf16[idx];
                } else if idx == utf16.len() {
                    chars[j] = 0x0000;
                }
            }
            lfn_store_chars(&mut entry, &chars);
            entries.push(entry);
        }

        entries.reverse();
        entries
    }

    /// Find N consecutive free entry slots in a directory buffer.
    fn find_consecutive_free(buf: &[u8], count: usize) -> Option<usize> {
        let max_entries = buf.len() / 32;
        let mut run_start = 0;
        let mut run_len = 0;

        for entry_idx in 0..max_entries {
            let i = entry_idx * 32;
            let first = buf[i];

            if first == 0x00 {
                if run_len == 0 {
                    run_start = entry_idx;
                }
                let available = max_entries - run_start;
                if available >= count {
                    return Some(run_start * 32);
                }
                return None;
            }

            if first == 0xE5 {
                if run_len == 0 {
                    run_start = entry_idx;
                }
                run_len += 1;
                if run_len >= count {
                    return Some(run_start * 32);
                }
            } else {
                run_len = 0;
            }
        }
        None
    }

    // =====================================================================
    // Directory entry creation (LFN-aware)
    // =====================================================================

    fn fill_dir_entry(&self, entry: &mut [u8], name83: &[u8; 11], attr: u8, first_cluster: u32, size: u32) {
        entry[0..11].copy_from_slice(name83);
        entry[11] = attr;
        entry[12] = 0;
        entry[13] = 0;
        entry[14..16].copy_from_slice(&0u16.to_le_bytes());
        entry[16..18].copy_from_slice(&0u16.to_le_bytes());
        entry[18..20].copy_from_slice(&0u16.to_le_bytes());
        entry[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
        entry[22..24].copy_from_slice(&0u16.to_le_bytes());
        entry[24..26].copy_from_slice(&0u16.to_le_bytes());
        entry[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
        entry[28..32].copy_from_slice(&size.to_le_bytes());
    }

    /// Create a new directory entry (with LFN entries if needed).
    pub fn create_entry(&self, parent_cluster: u32, name: &str, attr: u8, first_cluster: u32, size: u32) -> Result<(), FsError> {
        let use_lfn = needs_lfn(name);
        let name83 = if use_lfn {
            Self::generate_short_name(name)
        } else {
            Self::make_83_name(name)
        };
        let lfn_entries = if use_lfn {
            Self::make_lfn_entries(name, &name83)
        } else {
            Vec::new()
        };
        let total_slots = lfn_entries.len() + 1;

        if parent_cluster == 0 {
            let root_size = (self.root_dir_sectors * 512) as usize;
            let mut buf = vec![0u8; root_size];
            self.read_sectors(self.first_root_dir_sector, self.root_dir_sectors, &mut buf)?;

            let start_offset = Self::find_consecutive_free(&buf, total_slots)
                .ok_or(FsError::NoSpace)?;

            for (idx, lfn_entry) in lfn_entries.iter().enumerate() {
                let off = start_offset + idx * 32;
                buf[off..off + 32].copy_from_slice(lfn_entry);
            }
            let entry_off = start_offset + lfn_entries.len() * 32;
            self.fill_dir_entry(&mut buf[entry_off..entry_off + 32], &name83, attr, first_cluster, size);

            let first_sector_idx = start_offset / 512;
            let last_sector_idx = (entry_off + 31) / 512;
            for sec in first_sector_idx..=last_sector_idx {
                let sec_start = sec * 512;
                self.write_sectors(
                    self.first_root_dir_sector + sec as u32,
                    1,
                    &buf[sec_start..sec_start + 512],
                )?;
            }
            Ok(())
        } else {
            let cluster_size = (self.sectors_per_cluster * 512) as usize;
            let mut cur = parent_cluster;
            loop {
                let mut cbuf = vec![0u8; cluster_size];
                self.read_cluster(cur, &mut cbuf)?;

                if let Some(start_offset) = Self::find_consecutive_free(&cbuf, total_slots) {
                    for (idx, lfn_entry) in lfn_entries.iter().enumerate() {
                        let off = start_offset + idx * 32;
                        cbuf[off..off + 32].copy_from_slice(lfn_entry);
                    }
                    let entry_off = start_offset + lfn_entries.len() * 32;
                    self.fill_dir_entry(&mut cbuf[entry_off..entry_off + 32], &name83, attr, first_cluster, size);
                    self.write_cluster(cur, &cbuf)?;
                    return Ok(());
                }

                match self.next_cluster(cur) {
                    Some(next) => cur = next,
                    None => {
                        let new = self.alloc_cluster()?;
                        self.write_fat_entry(cur, new as u16)?;
                        let mut new_buf = vec![0u8; cluster_size];
                        for (idx, lfn_entry) in lfn_entries.iter().enumerate() {
                            let off = idx * 32;
                            new_buf[off..off + 32].copy_from_slice(lfn_entry);
                        }
                        let entry_off = lfn_entries.len() * 32;
                        self.fill_dir_entry(&mut new_buf[entry_off..entry_off + 32], &name83, attr, first_cluster, size);
                        self.write_cluster(new, &new_buf)?;
                        return Ok(());
                    }
                }
            }
        }
    }

    // =====================================================================
    // Directory entry deletion (LFN-aware)
    // =====================================================================

    /// Delete the 8.3 and any associated LFN entries for a file by name.
    /// Returns `(start_cluster, file_size)` of the deleted entry.
    pub fn delete_entry(&self, parent_cluster: u32, name: &str) -> Result<(u32, u32), FsError> {
        if parent_cluster == 0 {
            let root_size = (self.root_dir_sectors * 512) as usize;
            let mut buf = vec![0u8; root_size];
            self.read_sectors(self.first_root_dir_sector, self.root_dir_sectors, &mut buf)?;

            if let Some(found) = self.find_entry_in_buf(&buf, name) {
                let cluster = found.cluster;
                let size = found.size;
                buf[found.offset] = 0xE5;
                if let Some(lfn_start) = found.lfn_start {
                    let mut j = lfn_start;
                    while j < found.offset {
                        if buf[j + 11] == ATTR_LONG_NAME {
                            buf[j] = 0xE5;
                        }
                        j += 32;
                    }
                }
                let first_sec = if let Some(s) = found.lfn_start { s / 512 } else { found.offset / 512 };
                let last_sec = found.offset / 512;
                for sec in first_sec..=last_sec {
                    let sec_start = sec * 512;
                    self.write_sectors(
                        self.first_root_dir_sector + sec as u32,
                        1,
                        &buf[sec_start..sec_start + 512],
                    )?;
                }
                return Ok((cluster, size));
            }
            Err(FsError::NotFound)
        } else {
            let cluster_size = (self.sectors_per_cluster * 512) as usize;
            let mut cur = parent_cluster;
            loop {
                let mut cbuf = vec![0u8; cluster_size];
                self.read_cluster(cur, &mut cbuf)?;

                if let Some(found) = self.find_entry_in_buf(&cbuf, name) {
                    let cluster = found.cluster;
                    let size = found.size;
                    cbuf[found.offset] = 0xE5;
                    if let Some(lfn_start) = found.lfn_start {
                        let mut j = lfn_start;
                        while j < found.offset {
                            if cbuf[j + 11] == ATTR_LONG_NAME {
                                cbuf[j] = 0xE5;
                            }
                            j += 32;
                        }
                    }
                    self.write_cluster(cur, &cbuf)?;
                    return Ok((cluster, size));
                }

                match self.next_cluster(cur) {
                    Some(next) => cur = next,
                    None => return Err(FsError::NotFound),
                }
            }
        }
    }

    // =====================================================================
    // Directory entry update (LFN-aware lookup)
    // =====================================================================

    /// Update the size and starting cluster of an existing directory entry.
    pub fn update_entry(&self, parent_cluster: u32, name: &str, new_size: u32, new_cluster: u32) -> Result<(), FsError> {
        if parent_cluster == 0 {
            let root_size = (self.root_dir_sectors * 512) as usize;
            let mut buf = vec![0u8; root_size];
            self.read_sectors(self.first_root_dir_sector, self.root_dir_sectors, &mut buf)?;

            if let Some(found) = self.find_entry_in_buf(&buf, name) {
                let i = found.offset;
                buf[i + 26..i + 28].copy_from_slice(&(new_cluster as u16).to_le_bytes());
                buf[i + 20..i + 22].copy_from_slice(&((new_cluster >> 16) as u16).to_le_bytes());
                buf[i + 28..i + 32].copy_from_slice(&new_size.to_le_bytes());
                let sector_idx = i / 512;
                let sector_start = sector_idx * 512;
                self.write_sectors(
                    self.first_root_dir_sector + sector_idx as u32,
                    1,
                    &buf[sector_start..sector_start + 512],
                )?;
                return Ok(());
            }
            Err(FsError::NotFound)
        } else {
            let cluster_size = (self.sectors_per_cluster * 512) as usize;
            let mut cur = parent_cluster;
            loop {
                let mut cbuf = vec![0u8; cluster_size];
                self.read_cluster(cur, &mut cbuf)?;

                if let Some(found) = self.find_entry_in_buf(&cbuf, name) {
                    let i = found.offset;
                    cbuf[i + 26..i + 28].copy_from_slice(&(new_cluster as u16).to_le_bytes());
                    cbuf[i + 20..i + 22].copy_from_slice(&((new_cluster >> 16) as u16).to_le_bytes());
                    cbuf[i + 28..i + 32].copy_from_slice(&new_size.to_le_bytes());
                    self.write_cluster(cur, &cbuf)?;
                    return Ok(());
                }

                match self.next_cluster(cur) {
                    Some(next) => cur = next,
                    None => return Err(FsError::NotFound),
                }
            }
        }
    }

    // =====================================================================
    // High-level file operations
    // =====================================================================

    /// Create a new empty file in the given parent directory.
    pub fn create_file(&self, parent_cluster: u32, name: &str) -> Result<(), FsError> {
        let dir_data = self.read_dir_raw(parent_cluster)?;
        if self.find_entry_in_buf(&dir_data, name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        self.create_entry(parent_cluster, name, ATTR_ARCHIVE, 0, 0)
    }

    /// Create a new subdirectory with `.` and `..` entries. Returns the new cluster.
    pub fn create_dir(&self, parent_cluster: u32, name: &str) -> Result<u32, FsError> {
        let dir_data = self.read_dir_raw(parent_cluster)?;
        if self.find_entry_in_buf(&dir_data, name).is_some() {
            return Err(FsError::AlreadyExists);
        }

        let cluster = self.alloc_cluster()?;
        let cluster_size = (self.sectors_per_cluster * 512) as usize;
        let mut buf = vec![0u8; cluster_size];

        let dot_name: [u8; 11] = *b".          ";
        self.fill_dir_entry(&mut buf[0..32], &dot_name, ATTR_DIRECTORY, cluster, 0);
        let dotdot_name: [u8; 11] = *b"..         ";
        self.fill_dir_entry(&mut buf[32..64], &dotdot_name, ATTR_DIRECTORY, parent_cluster, 0);
        self.write_cluster(cluster, &buf)?;

        self.create_entry(parent_cluster, name, ATTR_DIRECTORY, cluster, 0)?;
        Ok(cluster)
    }

    /// Delete a file: remove the directory entry and free its cluster chain.
    pub fn delete_file(&self, parent_cluster: u32, name: &str) -> Result<(), FsError> {
        let (start_cluster, _size) = self.delete_entry(parent_cluster, name)?;
        if start_cluster >= 2 {
            self.free_chain(start_cluster)?;
        }
        Ok(())
    }

    /// Truncate a file to zero length: free its cluster chain and update the directory entry.
    pub fn truncate_file(&self, parent_cluster: u32, name: &str) -> Result<(), FsError> {
        let dir_data = self.read_dir_raw(parent_cluster)?;
        let found = self.find_entry_in_buf(&dir_data, name)
            .ok_or(FsError::NotFound)?;

        if found.cluster >= 2 {
            self.free_chain(found.cluster)?;
        }
        self.update_entry(parent_cluster, name, 0, 0)
    }
}
