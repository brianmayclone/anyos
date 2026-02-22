//! FAT directory operations: reading, searching, creating, deleting, and updating entries.

use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::FsError;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use super::{FatFs, FatType};
use super::bpb::{ATTR_ARCHIVE, ATTR_DIRECTORY, ATTR_LONG_NAME, ATTR_VOLUME_ID};
use super::datetime::{current_dos_datetime, dos_datetime_to_unix};
use super::lfn::{lfn_checksum, lfn_extract_chars, lfn_name_matches, lfn_to_string,
                  needs_lfn, make_lfn_entries};

/// Information about a found directory entry.
pub(super) struct FoundEntry {
    /// Byte offset of the 8.3 entry in the directory data buffer.
    pub offset: usize,
    /// Byte offset of the first LFN entry (if any).
    pub lfn_start: Option<usize>,
    /// Starting cluster.
    pub cluster: u32,
    /// File size.
    pub size: u32,
    /// Is a directory.
    pub is_dir: bool,
    /// DOS write time (FAT timestamp).
    pub write_time: u16,
    /// DOS write date (FAT timestamp).
    pub write_date: u16,
}

impl FatFs {
    // =================================================================
    // Raw directory reading
    // =================================================================

    pub(crate) fn read_dir_raw(&self, cluster: u32) -> Result<Vec<u8>, FsError> {
        if cluster == 0 && self.fat_type != FatType::Fat32 {
            // FAT12/16: fixed root directory area
            let root_size = self.root_dir_sectors * 512;
            let mut buf = vec![0u8; root_size as usize];
            self.read_sectors(self.first_root_dir_sector, self.root_dir_sectors, &mut buf)?;
            Ok(buf)
        } else {
            // Cluster chain (FAT32 root or any subdirectory)
            let start = if cluster == 0 { self.root_cluster } else { cluster };
            let cluster_size = self.sectors_per_cluster * 512;
            let mut result = Vec::new();
            let mut cur = start;
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

    // =================================================================
    // 8.3 name handling
    // =================================================================

    pub(crate) fn parse_83_name(&self, raw: &[u8]) -> String {
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

    pub(crate) fn name_matches(&self, raw_name: &[u8], filename: &str) -> bool {
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

    // =================================================================
    // LFN-aware entry finding
    // =================================================================

    /// Find a directory entry by name in a raw directory buffer.
    /// Supports both 8.3 names and VFAT long filenames.
    pub(crate) fn find_entry_in_buf(&self, buf: &[u8], name: &str) -> Option<FoundEntry> {
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

            // Regular 8.3 entry -- check for match
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
                let write_time = u16::from_le_bytes([buf[i + 22], buf[i + 23]]);
                let write_date = u16::from_le_bytes([buf[i + 24], buf[i + 25]]);
                return Some(FoundEntry {
                    offset: i,
                    lfn_start: current_lfn_start,
                    cluster,
                    size,
                    is_dir: attr & ATTR_DIRECTORY != 0,
                    write_time,
                    write_date,
                });
            }

            i += 32;
        }
        None
    }

    // =================================================================
    // LFN-aware directory listing
    // =================================================================

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

            // Regular 8.3 entry -- use LFN if valid, otherwise 8.3
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
                is_symlink: false,
                uid: 0, gid: 0, mode: 0xFFF,
            });

            i += 32;
        }
    }

    // =================================================================
    // Public directory operations
    // =================================================================

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

    /// Look up a file/directory and return (start_cluster, file_type, file_size, mtime_unix).
    pub fn stat_path(&self, path: &str) -> Result<(u32, FileType, u32, u32), FsError> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Ok((0, FileType::Directory, 0, 0));
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
                        let mtime = dos_datetime_to_unix(found.write_date, found.write_time);
                        return Ok((found.cluster, ft, found.size, mtime));
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

    // =================================================================
    // LFN entry generation (for create)
    // =================================================================

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

    // =================================================================
    // Directory entry creation (LFN-aware)
    // =================================================================

    pub(crate) fn fill_dir_entry(&self, entry: &mut [u8], name83: &[u8; 11], attr: u8, first_cluster: u32, size: u32) {
        let (date, time) = current_dos_datetime();
        entry[0..11].copy_from_slice(name83);
        entry[11] = attr;
        entry[12] = 0;                   // reserved
        entry[13] = 0;                   // create_time_tenth
        entry[14..16].copy_from_slice(&time.to_le_bytes()); // create_time
        entry[16..18].copy_from_slice(&date.to_le_bytes()); // create_date
        entry[18..20].copy_from_slice(&date.to_le_bytes()); // last_access_date
        entry[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
        entry[22..24].copy_from_slice(&time.to_le_bytes()); // write_time
        entry[24..26].copy_from_slice(&date.to_le_bytes()); // write_date
        entry[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
        entry[28..32].copy_from_slice(&size.to_le_bytes());
    }

    /// Create a new directory entry (with LFN entries if needed).
    pub fn create_entry(&mut self, parent_cluster: u32, name: &str, attr: u8, first_cluster: u32, size: u32) -> Result<(), FsError> {
        let use_lfn = needs_lfn(name);
        let name83 = if use_lfn {
            Self::generate_short_name(name)
        } else {
            Self::make_83_name(name)
        };
        let lfn_entries = if use_lfn {
            make_lfn_entries(name, &name83)
        } else {
            Vec::new()
        };
        let total_slots = lfn_entries.len() + 1;

        if parent_cluster == 0 && self.fat_type != FatType::Fat32 {
            // FAT12/16: fixed root directory area
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
            // Cluster chain (FAT32 root or any subdirectory)
            let start = if parent_cluster == 0 { self.root_cluster } else { parent_cluster };
            let cluster_size = (self.sectors_per_cluster * 512) as usize;
            let mut cur = start;
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
                        self.write_fat_entry(cur, new)?;
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

    // =================================================================
    // Directory entry deletion (LFN-aware)
    // =================================================================

    /// Delete the 8.3 and any associated LFN entries for a file by name.
    /// Returns `(start_cluster, file_size)` of the deleted entry.
    pub fn delete_entry(&self, parent_cluster: u32, name: &str) -> Result<(u32, u32), FsError> {
        if parent_cluster == 0 && self.fat_type != FatType::Fat32 {
            // FAT12/16: fixed root directory area
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
            // Cluster chain (FAT32 root or any subdirectory)
            let start = if parent_cluster == 0 { self.root_cluster } else { parent_cluster };
            let cluster_size = (self.sectors_per_cluster * 512) as usize;
            let mut cur = start;
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

    // =================================================================
    // Directory entry update (LFN-aware lookup)
    // =================================================================

    /// Update the size, starting cluster, and modification time of an existing directory entry.
    pub fn update_entry(&self, parent_cluster: u32, name: &str, new_size: u32, new_cluster: u32) -> Result<(), FsError> {
        let (date, time) = current_dos_datetime();
        if parent_cluster == 0 && self.fat_type != FatType::Fat32 {
            // FAT12/16: fixed root directory area
            let root_size = (self.root_dir_sectors * 512) as usize;
            let mut buf = vec![0u8; root_size];
            self.read_sectors(self.first_root_dir_sector, self.root_dir_sectors, &mut buf)?;

            if let Some(found) = self.find_entry_in_buf(&buf, name) {
                let i = found.offset;
                buf[i + 22..i + 24].copy_from_slice(&time.to_le_bytes()); // write_time
                buf[i + 24..i + 26].copy_from_slice(&date.to_le_bytes()); // write_date
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
            // Cluster chain (FAT32 root or any subdirectory)
            let start = if parent_cluster == 0 { self.root_cluster } else { parent_cluster };
            let cluster_size = (self.sectors_per_cluster * 512) as usize;
            let mut cur = start;
            loop {
                let mut cbuf = vec![0u8; cluster_size];
                self.read_cluster(cur, &mut cbuf)?;

                if let Some(found) = self.find_entry_in_buf(&cbuf, name) {
                    let i = found.offset;
                    cbuf[i + 22..i + 24].copy_from_slice(&time.to_le_bytes()); // write_time
                    cbuf[i + 24..i + 26].copy_from_slice(&date.to_le_bytes()); // write_date
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

    // =================================================================
    // High-level file operations
    // =================================================================

    /// Create a new empty file in the given parent directory.
    pub fn create_file(&mut self, parent_cluster: u32, name: &str) -> Result<(), FsError> {
        let dir_data = self.read_dir_raw(parent_cluster)?;
        if self.find_entry_in_buf(&dir_data, name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        self.create_entry(parent_cluster, name, ATTR_ARCHIVE, 0, 0)
    }

    /// Create a new subdirectory with `.` and `..` entries. Returns the new cluster.
    pub fn create_dir(&mut self, parent_cluster: u32, name: &str) -> Result<u32, FsError> {
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
    pub fn delete_file(&mut self, parent_cluster: u32, name: &str) -> Result<(), FsError> {
        let (start_cluster, _size) = self.delete_entry(parent_cluster, name)?;
        if start_cluster >= 2 {
            self.free_chain(start_cluster)?;
        }
        Ok(())
    }

    /// Rename (move) a file: remove old dir entry (keeping clusters), create new entry.
    pub fn rename_entry(&mut self, old_parent: u32, old_name: &str, new_parent: u32, new_name: &str) -> Result<(), FsError> {
        // Look up old entry to get cluster and size
        let dir_data = self.read_dir_raw(old_parent)?;
        let found = self.find_entry_in_buf(&dir_data, old_name)
            .ok_or(FsError::NotFound)?;
        let cluster = found.cluster;
        let size = found.size;
        let is_dir = found.is_dir;
        // Delete old directory entry (does NOT free cluster chain)
        self.delete_entry(old_parent, old_name)?;
        // Create new entry pointing to the same cluster chain
        let attr: u8 = if is_dir { 0x10 } else { 0x20 };
        self.create_entry(new_parent, new_name, attr, cluster, size)?;
        Ok(())
    }

    /// Truncate a file to zero length: free its cluster chain and update the directory entry.
    pub fn truncate_file(&mut self, parent_cluster: u32, name: &str) -> Result<(), FsError> {
        let dir_data = self.read_dir_raw(parent_cluster)?;
        let found = self.find_entry_in_buf(&dir_data, name)
            .ok_or(FsError::NotFound)?;

        if found.cluster >= 2 {
            self.free_chain(found.cluster)?;
        }
        self.update_entry(parent_cluster, name, 0, 0)
    }
}
