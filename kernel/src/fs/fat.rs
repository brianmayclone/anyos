use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::FsError;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// FAT filesystem structures

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

// FAT directory entry attributes
pub const ATTR_READ_ONLY: u8 = 0x01;
pub const ATTR_HIDDEN: u8 = 0x02;
pub const ATTR_SYSTEM: u8 = 0x04;
pub const ATTR_VOLUME_ID: u8 = 0x08;
pub const ATTR_DIRECTORY: u8 = 0x10;
pub const ATTR_ARCHIVE: u8 = 0x20;
pub const ATTR_LONG_NAME: u8 = 0x0F;

const FAT16_EOC: u16 = 0xFFF8;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatType {
    Fat12,
    Fat16,
    Fat32,
}

impl FatFs {
    /// Create a new FAT filesystem by reading the BPB from the ATA device.
    pub fn new(device_id: u32, partition_start_lba: u32) -> Result<Self, FsError> {
        let mut buf = [0u8; 512];
        if !crate::drivers::ata::read_sectors(partition_start_lba, 1, &mut buf) {
            crate::serial_println!("  FAT: Failed to read boot sector at LBA {}", partition_start_lba);
            return Err(FsError::IoError);
        }

        // Parse BPB fields from raw bytes (avoid packed struct alignment issues)
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
        })
    }

    /// Read sectors from the partition (relative LBA).
    fn read_sectors(&self, relative_lba: u32, count: u32, buf: &mut [u8]) -> Result<(), FsError> {
        let abs_lba = self.partition_start_lba + relative_lba;
        let mut offset = 0usize;
        let mut remaining = count;
        let mut lba = abs_lba;

        while remaining > 0 {
            let batch = remaining.min(255) as u8;
            if !crate::drivers::ata::read_sectors(lba, batch, &mut buf[offset..]) {
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

    /// Read file data starting from a cluster with byte offset into buf.
    pub fn read_file(&self, start_cluster: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        if start_cluster < 2 {
            return Ok(0);
        }

        let cluster_size = self.sectors_per_cluster * 512;
        let mut cluster = start_cluster;
        let mut bytes_skipped = 0u32;
        let mut bytes_read = 0usize;

        // Skip clusters for offset
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

    /// Read all entries from a directory (cluster=0 means root directory).
    pub fn read_dir(&self, cluster: u32) -> Result<Vec<DirEntry>, FsError> {
        let mut entries = Vec::new();
        let raw = self.read_dir_raw(cluster)?;
        self.parse_dir_entries(&raw, &mut entries);
        Ok(entries)
    }

    fn parse_dir_entries(&self, buf: &[u8], entries: &mut Vec<DirEntry>) {
        for i in (0..buf.len()).step_by(32) {
            if i + 32 > buf.len() {
                break;
            }

            let first_byte = buf[i];
            if first_byte == 0x00 {
                break;
            }
            if first_byte == 0xE5 {
                continue;
            }

            let attr = buf[i + 11];
            if attr == ATTR_LONG_NAME || attr & ATTR_VOLUME_ID != 0 {
                continue;
            }

            let name = self.parse_83_name(&buf[i..i + 11]);
            if name == "." || name == ".." {
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
        }
    }

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

    /// Look up a file/directory by path. Returns (start_cluster, file_type, file_size).
    pub fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Ok((0, FileType::Directory, 0));
        }

        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut current_cluster: u32 = 0;

        for (i, component) in components.iter().enumerate() {
            let is_last = i == components.len() - 1;
            let dir_data = self.read_dir_raw(current_cluster)?;

            let mut found = false;
            for offset in (0..dir_data.len()).step_by(32) {
                if offset + 32 > dir_data.len() {
                    break;
                }

                let first_byte = dir_data[offset];
                if first_byte == 0x00 {
                    break;
                }
                if first_byte == 0xE5 {
                    continue;
                }

                let attr = dir_data[offset + 11];
                if attr == ATTR_LONG_NAME || attr & ATTR_VOLUME_ID != 0 {
                    continue;
                }

                if self.name_matches(&dir_data[offset..offset + 11], component) {
                    let cluster_lo = u16::from_le_bytes([dir_data[offset + 26], dir_data[offset + 27]]);
                    let cluster_hi = u16::from_le_bytes([dir_data[offset + 20], dir_data[offset + 21]]);
                    let cluster = (cluster_hi as u32) << 16 | cluster_lo as u32;
                    let size = u32::from_le_bytes([
                        dir_data[offset + 28], dir_data[offset + 29],
                        dir_data[offset + 30], dir_data[offset + 31],
                    ]);
                    let is_dir = attr & ATTR_DIRECTORY != 0;

                    if is_last {
                        let ft = if is_dir { FileType::Directory } else { FileType::Regular };
                        return Ok((cluster, ft, size));
                    } else if !is_dir {
                        return Err(FsError::NotADirectory);
                    } else {
                        current_cluster = cluster;
                        found = true;
                        break;
                    }
                }
            }

            if !found {
                return Err(FsError::NotFound);
            }
        }

        Err(FsError::NotFound)
    }

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

    /// Read an entire file into a Vec<u8>.
    pub fn read_file_all(&self, start_cluster: u32, file_size: u32) -> Result<Vec<u8>, FsError> {
        if file_size == 0 || start_cluster < 2 {
            return Ok(Vec::new());
        }
        let mut buf = vec![0u8; file_size as usize];
        let bytes_read = self.read_file(start_cluster, 0, &mut buf)?;
        buf.truncate(bytes_read);
        Ok(buf)
    }
}
