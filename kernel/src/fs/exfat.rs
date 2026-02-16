//! exFAT filesystem driver for anyOS.
//! Supports reading and writing files/directories on an exFAT partition.
//! Designed to coexist with the FAT16 driver — VFS auto-detects which to use.

use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::FsError;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

// exFAT FAT entry constants
const EXFAT_EOC: u32 = 0xFFFFFFFF;
const EXFAT_BAD: u32 = 0xFFFFFFF7;
const EXFAT_FREE: u32 = 0x00000000;

// Directory entry type codes (bit 7 = InUse)
const ENTRY_FILE: u8 = 0x85;
const ENTRY_STREAM: u8 = 0xC0;
const ENTRY_FILENAME: u8 = 0xC1;

// Stream extension flags
const FLAG_CONTIGUOUS: u8 = 0x02;

// File attributes
const ATTR_DIRECTORY: u16 = 0x0010;
const ATTR_ARCHIVE: u16 = 0x0020;

/// Bit flag stored in the VFS `inode` field to indicate a contiguous file.
/// Since exFAT volumes realistically have far fewer than 2^31 clusters,
/// we use bit 31 to encode the NoFatChain flag alongside the first cluster.
pub const CONTIGUOUS_BIT: u32 = 0x8000_0000;

/// Encode (cluster, contiguous) into a u32 inode for VFS storage.
pub fn encode_inode(cluster: u32, contiguous: bool) -> u32 {
    if contiguous { cluster | CONTIGUOUS_BIT } else { cluster }
}

/// Decode a u32 inode back to (cluster, contiguous).
pub fn decode_inode(inode: u32) -> (u32, bool) {
    (inode & !CONTIGUOUS_BIT, inode & CONTIGUOUS_BIT != 0)
}

/// Information about a found exFAT directory entry set.
struct FoundEntry {
    first_cluster: u32,
    data_length: u64,
    attributes: u16,
    contiguous: bool,
    /// Byte offset of the File entry (0x85) within the cluster buffer
    file_entry_offset: usize,
    secondary_count: u8,
}

/// In-memory representation of a mounted exFAT filesystem.
pub struct ExFatFs {
    pub device_id: u32,
    pub partition_start_lba: u32,
    bytes_per_sector_shift: u8,
    sectors_per_cluster_shift: u8,
    fat_offset: u32,
    fat_length: u32,
    cluster_heap_offset: u32,
    cluster_count: u32,
    root_cluster: u32,
    /// Cached FAT table (4 bytes per cluster entry).
    fat_cache: Vec<u8>,
    /// Cached allocation bitmap.
    bitmap: Vec<u8>,
    /// Cluster where the bitmap starts.
    bitmap_cluster: u32,
    /// Whether bitmap is stored contiguously.
    bitmap_contiguous: bool,
}

/// Pre-computed read plan for exFAT files (matches FAT16 FileReadPlan pattern).
/// Built under VFS lock (no disk I/O), executed after lock is dropped.
pub struct ExFatReadPlan {
    /// Contiguous (absolute_lba, sector_count) runs covering the file.
    pub runs: Vec<(u32, u32)>,
    /// Actual file size in bytes.
    pub file_size: u64,
}

impl ExFatReadPlan {
    /// Execute the read plan — reads sectors from storage.
    /// **Must be called WITHOUT the VFS lock held.**
    pub fn execute(&self) -> Result<Vec<u8>, FsError> {
        if self.file_size == 0 {
            return Ok(Vec::new());
        }
        let total_sector_bytes: usize =
            self.runs.iter().map(|(_, sc)| *sc as usize * 512).sum();
        let mut buf = vec![0u8; total_sector_bytes];
        let mut offset = 0usize;

        for &(abs_lba, sector_count) in &self.runs {
            let bytes = sector_count as usize * 512;
            if !crate::drivers::storage::read_sectors(
                abs_lba, sector_count, &mut buf[offset..offset + bytes],
            ) {
                return Err(FsError::IoError);
            }
            offset += bytes;
        }

        buf.truncate(self.file_size as usize);
        Ok(buf)
    }
}

impl ExFatFs {
    // =================================================================
    // Construction
    // =================================================================

    /// Mount an exFAT filesystem by reading the VBR from the storage device.
    pub fn new(device_id: u32, partition_start_lba: u32) -> Result<Self, FsError> {
        let mut buf = [0u8; 512];
        if !crate::drivers::storage::read_sectors(partition_start_lba, 1, &mut buf) {
            crate::serial_println!("  exFAT: Failed to read VBR at LBA {}", partition_start_lba);
            return Err(FsError::IoError);
        }

        // Validate OEM name
        if &buf[3..11] != b"EXFAT   " {
            return Err(FsError::IoError);
        }

        // MustBeZero region (bytes 11..64) — quick sanity check
        if buf[11..64].iter().any(|&b| b != 0) {
            crate::serial_println!("  exFAT: MustBeZero region is non-zero");
            return Err(FsError::IoError);
        }

        // Parse VBR fields
        let _volume_length = u64::from_le_bytes(buf[72..80].try_into().unwrap());
        let fat_offset = u32::from_le_bytes(buf[80..84].try_into().unwrap());
        let fat_length = u32::from_le_bytes(buf[84..88].try_into().unwrap());
        let cluster_heap_offset = u32::from_le_bytes(buf[88..92].try_into().unwrap());
        let cluster_count = u32::from_le_bytes(buf[92..96].try_into().unwrap());
        let root_cluster = u32::from_le_bytes(buf[96..100].try_into().unwrap());
        let bytes_per_sector_shift = buf[108];
        let sectors_per_cluster_shift = buf[109];
        let _number_of_fats = buf[110];

        if bytes_per_sector_shift != 9 {
            crate::serial_println!(
                "  exFAT: Unsupported sector size shift: {} (only 9 supported)",
                bytes_per_sector_shift
            );
            return Err(FsError::IoError);
        }

        let cluster_size = 512u32 << sectors_per_cluster_shift;
        crate::serial_println!(
            "[OK] exFAT filesystem: {} clusters, {} bytes/cluster, root=cluster {}",
            cluster_count, cluster_size, root_cluster
        );
        crate::serial_println!(
            "  exFAT: FAT at sector +{}, data at sector +{}, {} FATs",
            fat_offset, cluster_heap_offset, _number_of_fats
        );

        // Cache the entire FAT table in memory
        let fat_cache_bytes = (fat_length as usize) * 512;
        let mut fat_cache = vec![0u8; fat_cache_bytes];
        let abs_fat_lba = partition_start_lba + fat_offset;
        if !crate::drivers::storage::read_sectors(abs_fat_lba, fat_length, &mut fat_cache) {
            crate::serial_println!("  exFAT: Failed to cache FAT table");
            return Err(FsError::IoError);
        }
        crate::serial_println!("  exFAT: cached {} KB FAT table", fat_cache_bytes / 1024);

        let mut fs = ExFatFs {
            device_id,
            partition_start_lba,
            bytes_per_sector_shift,
            sectors_per_cluster_shift,
            fat_offset,
            fat_length,
            cluster_heap_offset,
            cluster_count,
            root_cluster,
            fat_cache,
            bitmap: Vec::new(),
            bitmap_cluster: 0,
            bitmap_contiguous: true,
        };

        // Scan root directory for the allocation bitmap entry
        fs.load_bitmap()?;
        Ok(fs)
    }

    // =================================================================
    // Geometry helpers
    // =================================================================

    #[inline]
    fn sectors_per_cluster(&self) -> u32 {
        1u32 << self.sectors_per_cluster_shift
    }

    #[inline]
    fn cluster_size(&self) -> u32 {
        512u32 << self.sectors_per_cluster_shift
    }

    /// Convert a cluster number (>=2) to an absolute LBA.
    #[inline]
    fn cluster_to_lba(&self, cluster: u32) -> u32 {
        self.partition_start_lba
            + self.cluster_heap_offset
            + (cluster - 2) * self.sectors_per_cluster()
    }

    // =================================================================
    // Low-level I/O
    // =================================================================

    fn read_sectors(&self, abs_lba: u32, count: u32, buf: &mut [u8]) -> Result<(), FsError> {
        if !crate::drivers::storage::read_sectors(abs_lba, count, buf) {
            Err(FsError::IoError)
        } else {
            Ok(())
        }
    }

    fn write_sectors(&self, abs_lba: u32, count: u32, buf: &[u8]) -> Result<(), FsError> {
        if !crate::drivers::storage::write_sectors(abs_lba, count, buf) {
            Err(FsError::IoError)
        } else {
            Ok(())
        }
    }

    fn read_cluster(&self, cluster: u32, buf: &mut [u8]) -> Result<(), FsError> {
        let lba = self.cluster_to_lba(cluster);
        self.read_sectors(lba, self.sectors_per_cluster(), buf)
    }

    fn write_cluster(&self, cluster: u32, buf: &[u8]) -> Result<(), FsError> {
        let lba = self.cluster_to_lba(cluster);
        let cs = self.cluster_size() as usize;
        if buf.len() >= cs {
            self.write_sectors(lba, self.sectors_per_cluster(), &buf[..cs])
        } else {
            let mut tmp = vec![0u8; cs];
            tmp[..buf.len()].copy_from_slice(buf);
            self.write_sectors(lba, self.sectors_per_cluster(), &tmp)
        }
    }

    // =================================================================
    // FAT chain helpers
    // =================================================================

    /// Read next cluster from the in-memory FAT cache. Returns `None` at end-of-chain.
    fn next_cluster(&self, cluster: u32) -> Option<u32> {
        let off = (cluster as usize) * 4;
        if off + 3 >= self.fat_cache.len() {
            return None;
        }
        let val = u32::from_le_bytes([
            self.fat_cache[off],
            self.fat_cache[off + 1],
            self.fat_cache[off + 2],
            self.fat_cache[off + 3],
        ]);
        if val == EXFAT_FREE || val >= 0xFFFFFFF8 {
            None
        } else {
            Some(val)
        }
    }

    /// Write an entry to the in-memory FAT cache and flush that sector to disk.
    fn write_fat_entry(&mut self, cluster: u32, value: u32) -> Result<(), FsError> {
        let off = (cluster as usize) * 4;
        if off + 3 >= self.fat_cache.len() {
            return Err(FsError::IoError);
        }
        let bytes = value.to_le_bytes();
        self.fat_cache[off..off + 4].copy_from_slice(&bytes);

        // Write-through: flush the containing 512-byte sector
        let sector_idx = off / 512;
        let sector_start = sector_idx * 512;
        let mut sector_buf = [0u8; 512];
        sector_buf.copy_from_slice(&self.fat_cache[sector_start..sector_start + 512]);
        let abs_lba = self.partition_start_lba + self.fat_offset + sector_idx as u32;
        self.write_sectors(abs_lba, 1, &sector_buf)
    }

    // =================================================================
    // Allocation bitmap
    // =================================================================

    /// Scan the root directory for the Allocation Bitmap entry and cache it.
    fn load_bitmap(&mut self) -> Result<(), FsError> {
        let cs = self.cluster_size() as usize;
        let mut cluster = self.root_cluster;

        loop {
            let mut cbuf = vec![0u8; cs];
            self.read_cluster(cluster, &mut cbuf)?;

            let mut i = 0;
            while i + 32 <= cs {
                let etype = cbuf[i];
                if etype == 0x00 {
                    break;
                }
                // Allocation Bitmap (in-use = 0x81)
                if etype == 0x81 {
                    let bm_cluster = u32::from_le_bytes(
                        cbuf[i + 20..i + 24].try_into().unwrap(),
                    );
                    let bm_size = u64::from_le_bytes(
                        cbuf[i + 24..i + 32].try_into().unwrap(),
                    );
                    self.bitmap_cluster = bm_cluster;
                    self.bitmap_contiguous = true;

                    // Read bitmap data (always contiguous per spec)
                    let num_clusters =
                        ((bm_size as u32 + self.cluster_size() - 1) / self.cluster_size()).max(1);
                    let total_sectors = num_clusters * self.sectors_per_cluster();
                    let total_bytes = total_sectors as usize * 512;
                    let mut raw = vec![0u8; total_bytes];
                    let lba = self.cluster_to_lba(bm_cluster);
                    self.read_sectors(lba, total_sectors, &mut raw)?;
                    raw.truncate(bm_size as usize);
                    self.bitmap = raw;

                    crate::serial_println!(
                        "  exFAT: allocation bitmap at cluster {}, {} bytes",
                        bm_cluster, bm_size
                    );
                    return Ok(());
                }
                i += 32;
            }

            match self.next_cluster(cluster) {
                Some(next) => cluster = next,
                None => break,
            }
        }

        crate::serial_println!("  exFAT: allocation bitmap not found!");
        Err(FsError::IoError)
    }

    /// Flush a single modified byte of the bitmap back to disk.
    fn flush_bitmap_byte(&self, byte_idx: usize) -> Result<(), FsError> {
        let cs = self.cluster_size() as usize;
        let cluster_idx = byte_idx / cs;
        let offset_in_cluster = byte_idx % cs;
        let target_cluster = self.bitmap_cluster + cluster_idx as u32;

        let sector_in_cluster = offset_in_cluster / 512;
        let lba = self.cluster_to_lba(target_cluster) + sector_in_cluster as u32;

        let mut sector_buf = [0u8; 512];
        self.read_sectors(lba, 1, &mut sector_buf)?;
        sector_buf[offset_in_cluster % 512] = self.bitmap[byte_idx];
        self.write_sectors(lba, 1, &sector_buf)
    }

    /// Allocate a single cluster. Marks bitmap + writes EOC to FAT.
    fn alloc_cluster(&mut self) -> Result<u32, FsError> {
        for i in 0..self.cluster_count {
            let byte_idx = i as usize / 8;
            let bit_idx = i as usize % 8;
            if byte_idx >= self.bitmap.len() {
                break;
            }
            if self.bitmap[byte_idx] & (1 << bit_idx) == 0 {
                self.bitmap[byte_idx] |= 1 << bit_idx;
                self.flush_bitmap_byte(byte_idx)?;
                let cluster = i + 2;
                self.write_fat_entry(cluster, EXFAT_EOC)?;
                return Ok(cluster);
            }
        }
        Err(FsError::NoSpace)
    }

    /// Free a cluster chain (FAT-chained or contiguous).
    fn free_chain(
        &mut self,
        start: u32,
        contiguous: bool,
        data_length: u64,
    ) -> Result<(), FsError> {
        if start < 2 {
            return Ok(());
        }
        if contiguous {
            let cs = self.cluster_size() as u64;
            let n = ((data_length + cs - 1) / cs) as u32;
            for j in 0..n {
                let idx = (start - 2 + j) as usize;
                let byte = idx / 8;
                let bit = idx % 8;
                if byte < self.bitmap.len() {
                    self.bitmap[byte] &= !(1 << bit);
                    self.flush_bitmap_byte(byte)?;
                }
            }
        } else {
            let mut c = start;
            loop {
                let next = self.next_cluster(c);
                let idx = (c - 2) as usize;
                let byte = idx / 8;
                let bit = idx % 8;
                if byte < self.bitmap.len() {
                    self.bitmap[byte] &= !(1 << bit);
                    self.flush_bitmap_byte(byte)?;
                }
                self.write_fat_entry(c, EXFAT_FREE)?;
                match next {
                    Some(n) => c = n,
                    None => break,
                }
            }
        }
        Ok(())
    }

    // =================================================================
    // Directory entry parsing
    // =================================================================

    /// Compute the exFAT entry-set checksum (skipping bytes 2-3 of the first entry).
    fn entry_set_checksum(data: &[u8], entry_count: usize) -> u16 {
        let total = entry_count * 32;
        let mut cs: u16 = 0;
        for i in 0..total.min(data.len()) {
            if i == 2 || i == 3 {
                continue;
            }
            cs = ((cs << 15) | (cs >> 1)).wrapping_add(data[i] as u16);
        }
        cs
    }

    /// Compute the exFAT name hash over a UTF-16 name (upper-cased).
    fn name_hash(name: &[u16]) -> u16 {
        let mut h: u16 = 0;
        for &ch in name {
            let uc = Self::upcase(ch);
            h = ((h << 15) | (h >> 1)).wrapping_add((uc & 0xFF) as u16);
            h = ((h << 15) | (h >> 1)).wrapping_add((uc >> 8) as u16);
        }
        h
    }

    /// Simple ASCII upper-case (sufficient for our OS — no full Unicode upcase table).
    #[inline]
    fn upcase(ch: u16) -> u16 {
        if ch >= 0x61 && ch <= 0x7A { ch - 0x20 } else { ch }
    }

    /// Read all raw directory data from a cluster chain.
    fn read_dir_raw(&self, cluster: u32) -> Result<Vec<u8>, FsError> {
        let cs = self.cluster_size() as usize;
        let mut result = Vec::new();
        let mut cur = cluster;
        loop {
            let mut cbuf = vec![0u8; cs];
            self.read_cluster(cur, &mut cbuf)?;
            result.extend_from_slice(&cbuf);
            match self.next_cluster(cur) {
                Some(next) => cur = next,
                None => break,
            }
        }
        Ok(result)
    }

    /// Collect the UTF-16 name from an entry set starting at `base_offset` in `buf`.
    fn collect_name(buf: &[u8], base_offset: usize, secondary_count: u8, name_length: usize) -> Vec<u16> {
        let total = 1 + secondary_count as usize;
        let mut name = Vec::with_capacity(name_length);
        // FileName entries start at index 2 (after File + Stream)
        let mut fn_idx = 2;
        while fn_idx < total && name.len() < name_length {
            let off = base_offset + fn_idx * 32;
            if off + 32 > buf.len() || buf[off] != ENTRY_FILENAME {
                break;
            }
            for j in 0..15 {
                if name.len() >= name_length {
                    break;
                }
                let ch = u16::from_le_bytes([buf[off + 2 + j * 2], buf[off + 3 + j * 2]]);
                name.push(ch);
            }
            fn_idx += 1;
        }
        name
    }

    /// Case-insensitive comparison of a UTF-16 name against an ASCII name.
    fn names_equal(utf16: &[u16], ascii: &str) -> bool {
        let bytes = ascii.as_bytes();
        if utf16.len() != bytes.len() {
            return false;
        }
        for (i, &ch) in utf16.iter().enumerate() {
            if Self::upcase(ch) != Self::upcase(bytes[i] as u16) {
                return false;
            }
        }
        true
    }

    /// Find a named entry in a raw directory buffer.
    fn find_entry_in_buf(&self, buf: &[u8], name: &str) -> Option<FoundEntry> {
        let mut i = 0;
        while i + 32 <= buf.len() {
            let etype = buf[i];
            if etype == 0x00 {
                break;
            }
            if etype != ENTRY_FILE {
                i += 32;
                continue;
            }

            let secondary_count = buf[i + 1];
            let attributes = u16::from_le_bytes([buf[i + 4], buf[i + 5]]);
            let total = 1 + secondary_count as usize;
            if i + total * 32 > buf.len() {
                break;
            }

            // Stream Extension at i+32
            let s = i + 32;
            if buf[s] != ENTRY_STREAM {
                i += 32;
                continue;
            }

            let general_flags = buf[s + 1];
            let contiguous = general_flags & FLAG_CONTIGUOUS != 0;
            let name_length = buf[s + 3] as usize;
            let first_cluster = u32::from_le_bytes(buf[s + 20..s + 24].try_into().unwrap());
            let data_length = u64::from_le_bytes(buf[s + 24..s + 32].try_into().unwrap());

            let collected = Self::collect_name(buf, i, secondary_count, name_length);
            if Self::names_equal(&collected, name) {
                return Some(FoundEntry {
                    first_cluster,
                    data_length,
                    attributes,
                    contiguous,
                    file_entry_offset: i,
                    secondary_count,
                });
            }

            i += total * 32;
        }
        None
    }

    /// Parse directory entries for listing (readdir).
    fn parse_dir_entries(&self, buf: &[u8], entries: &mut Vec<DirEntry>) {
        let mut i = 0;
        while i + 32 <= buf.len() {
            let etype = buf[i];
            if etype == 0x00 {
                break;
            }
            if etype != ENTRY_FILE {
                i += 32;
                continue;
            }

            let secondary_count = buf[i + 1];
            let attributes = u16::from_le_bytes([buf[i + 4], buf[i + 5]]);
            let total = 1 + secondary_count as usize;
            if i + total * 32 > buf.len() {
                break;
            }

            let s = i + 32;
            if s + 32 > buf.len() || buf[s] != ENTRY_STREAM {
                i += 32;
                continue;
            }

            let name_length = buf[s + 3] as usize;
            let data_length = u64::from_le_bytes(buf[s + 24..s + 32].try_into().unwrap());

            let collected = Self::collect_name(buf, i, secondary_count, name_length);
            let name_str = Self::utf16_to_string(&collected);

            let file_type = if attributes & ATTR_DIRECTORY != 0 {
                FileType::Directory
            } else {
                FileType::Regular
            };

            entries.push(DirEntry {
                name: name_str,
                file_type,
                size: data_length as u32,
            });

            i += total * 32;
        }
    }

    /// Convert UTF-16LE code units to an ASCII `String`.
    fn utf16_to_string(chars: &[u16]) -> String {
        let mut s = String::new();
        for &ch in chars {
            if ch == 0 {
                break;
            }
            s.push(if ch < 128 { ch as u8 as char } else { '?' });
        }
        s
    }

    // =================================================================
    // Public API — lookup
    // =================================================================

    /// Look up a file/directory by path.
    /// Returns `(encoded_inode, file_type, size)` where encoded_inode has
    /// the contiguous bit set if the file uses NoFatChain.
    pub fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Ok((self.root_cluster, FileType::Directory, 0));
        }

        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut current_cluster = self.root_cluster;

        for (idx, component) in components.iter().enumerate() {
            let is_last = idx == components.len() - 1;
            let dir_data = self.read_dir_raw(current_cluster)?;

            match self.find_entry_in_buf(&dir_data, component) {
                Some(found) => {
                    let is_dir = found.attributes & ATTR_DIRECTORY != 0;
                    if is_last {
                        let ft = if is_dir {
                            FileType::Directory
                        } else {
                            FileType::Regular
                        };
                        let inode = encode_inode(found.first_cluster, found.contiguous);
                        return Ok((inode, ft, found.data_length as u32));
                    } else if !is_dir {
                        return Err(FsError::NotADirectory);
                    } else {
                        current_cluster = found.first_cluster;
                    }
                }
                None => return Err(FsError::NotFound),
            }
        }

        Err(FsError::NotFound)
    }

    // =================================================================
    // Public API — directory listing
    // =================================================================

    /// List all entries in a directory given its first cluster.
    pub fn read_dir(&self, cluster: u32) -> Result<Vec<DirEntry>, FsError> {
        let mut entries = Vec::new();
        let raw = self.read_dir_raw(cluster)?;
        self.parse_dir_entries(&raw, &mut entries);
        Ok(entries)
    }

    // =================================================================
    // Public API — file read
    // =================================================================

    /// Read bytes from a file starting at `offset` into `buf`.
    /// `inode` is the encoded inode (cluster + contiguous bit).
    pub fn read_file(&self, inode: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        let (start_cluster, contiguous) = decode_inode(inode);
        if start_cluster < 2 || buf.is_empty() {
            return Ok(0);
        }

        let cs = self.cluster_size();
        let spc = self.sectors_per_cluster();

        if contiguous {
            return self.read_file_contiguous(start_cluster, offset, buf, cs, spc);
        }

        // Non-contiguous: follow FAT chain, batch contiguous runs
        let mut cluster = start_cluster;
        let mut bytes_skipped = 0u32;

        // Skip clusters before offset
        while bytes_skipped + cs <= offset {
            bytes_skipped += cs;
            match self.next_cluster(cluster) {
                Some(next) => cluster = next,
                None => return Ok(0),
            }
        }

        let mut bytes_read = 0usize;

        loop {
            let start_in_run = if bytes_skipped < offset {
                (offset - bytes_skipped) as usize
            } else {
                0
            };
            let bytes_needed = buf.len() - bytes_read + start_in_run;
            let max_clusters = ((bytes_needed as u32 + cs - 1) / cs).max(1);

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

            let run_bytes = (run_clusters * cs) as usize;
            let available = run_bytes - start_in_run;
            let to_copy = available.min(buf.len() - bytes_read);

            if start_in_run == 0 && to_copy == run_bytes {
                self.read_sectors(
                    run_start_lba,
                    run_clusters * spc,
                    &mut buf[bytes_read..bytes_read + run_bytes],
                )?;
            } else {
                let mut tmp = vec![0u8; run_bytes];
                self.read_sectors(run_start_lba, run_clusters * spc, &mut tmp)?;
                buf[bytes_read..bytes_read + to_copy]
                    .copy_from_slice(&tmp[start_in_run..start_in_run + to_copy]);
            }

            bytes_read += to_copy;
            bytes_skipped += run_clusters * cs;

            if bytes_read >= buf.len() {
                break;
            }

            match self.next_cluster(last_cluster) {
                Some(next) => cluster = next,
                None => break,
            }
        }

        Ok(bytes_read)
    }

    /// Optimized contiguous-file read (NoFatChain — clusters are sequential).
    fn read_file_contiguous(
        &self,
        start_cluster: u32,
        offset: u32,
        buf: &mut [u8],
        cs: u32,
        spc: u32,
    ) -> Result<usize, FsError> {
        let first_cluster = start_cluster + offset / cs;
        let byte_in_first = (offset % cs) as usize;

        if byte_in_first == 0 {
            // Aligned — single batch read
            let needed_clusters =
                ((buf.len() as u32 + cs - 1) / cs).max(1);
            let total_sectors = needed_clusters * spc;
            let total_bytes = total_sectors as usize * 512;

            if total_bytes == buf.len() {
                let lba = self.cluster_to_lba(first_cluster);
                self.read_sectors(lba, total_sectors, buf)?;
            } else {
                let mut tmp = vec![0u8; total_bytes];
                let lba = self.cluster_to_lba(first_cluster);
                self.read_sectors(lba, total_sectors, &mut tmp)?;
                let copy_len = buf.len().min(total_bytes);
                buf[..copy_len].copy_from_slice(&tmp[..copy_len]);
            }
            return Ok(buf.len());
        }

        // Unaligned start — read first partial cluster, then batch the rest
        let mut bytes_read = 0usize;
        let first_avail = cs as usize - byte_in_first;
        let first_copy = first_avail.min(buf.len());

        let mut cbuf = vec![0u8; cs as usize];
        self.read_cluster(first_cluster, &mut cbuf)?;
        buf[..first_copy].copy_from_slice(&cbuf[byte_in_first..byte_in_first + first_copy]);
        bytes_read += first_copy;

        if bytes_read < buf.len() {
            let remaining = buf.len() - bytes_read;
            let needed_clusters = ((remaining as u32 + cs - 1) / cs).max(1);
            let total_sectors = needed_clusters * spc;
            let total_bytes = total_sectors as usize * 512;
            let next_cluster = first_cluster + 1;
            let lba = self.cluster_to_lba(next_cluster);

            if total_bytes <= remaining {
                self.read_sectors(lba, total_sectors, &mut buf[bytes_read..bytes_read + total_bytes])?;
                bytes_read += total_bytes;
            } else {
                let mut tmp = vec![0u8; total_bytes];
                self.read_sectors(lba, total_sectors, &mut tmp)?;
                let copy_len = remaining.min(total_bytes);
                buf[bytes_read..bytes_read + copy_len].copy_from_slice(&tmp[..copy_len]);
                bytes_read += copy_len;
            }
        }

        Ok(bytes_read)
    }

    /// Build a read plan (for lock-free I/O in `read_file_to_vec`).
    pub fn get_file_read_plan(&self, inode: u32, file_size: u32) -> ExFatReadPlan {
        let (start_cluster, contiguous) = decode_inode(inode);
        let spc = self.sectors_per_cluster();
        let mut runs = Vec::new();
        let file_size_u64 = file_size as u64;

        if file_size == 0 || start_cluster < 2 {
            return ExFatReadPlan { runs, file_size: file_size_u64 };
        }

        if contiguous {
            let cs = self.cluster_size() as u64;
            let n = ((file_size_u64 + cs - 1) / cs) as u32;
            let lba = self.cluster_to_lba(start_cluster);
            runs.push((lba, n * spc));
            return ExFatReadPlan { runs, file_size: file_size_u64 };
        }

        // Follow FAT chain, coalesce contiguous runs
        let mut cluster = start_cluster;
        loop {
            let run_start_lba = self.cluster_to_lba(cluster);
            let mut run_clusters: u32 = 1;
            let mut last = cluster;
            while let Some(next) = self.next_cluster(last) {
                if next == last + 1 {
                    run_clusters += 1;
                    last = next;
                } else {
                    break;
                }
            }
            runs.push((run_start_lba, run_clusters * spc));
            match self.next_cluster(last) {
                Some(next) => cluster = next,
                None => break,
            }
        }

        ExFatReadPlan { runs, file_size: file_size_u64 }
    }

    // =================================================================
    // Public API — file write
    // =================================================================

    /// Write data to a file at the given offset, allocating clusters as needed.
    /// Returns `(new_inode, new_size)`. The returned inode does NOT have the
    /// contiguous bit set (writes may fragment the file).
    pub fn write_file(
        &mut self,
        inode: u32,
        offset: u32,
        data: &[u8],
        old_size: u32,
    ) -> Result<(u32, u32), FsError> {
        let (start_cluster, _) = decode_inode(inode);
        if data.is_empty() {
            return Ok((start_cluster, old_size));
        }

        let cs = self.cluster_size();
        let first = if start_cluster < 2 {
            self.alloc_cluster()?
        } else {
            start_cluster
        };

        let mut cluster = first;
        let mut cluster_offset = 0u32;

        // Skip to the cluster containing `offset`
        while cluster_offset + cs <= offset {
            cluster_offset += cs;
            match self.next_cluster(cluster) {
                Some(next) => cluster = next,
                None => {
                    let new = self.alloc_cluster()?;
                    self.write_fat_entry(cluster, new)?;
                    let zeros = vec![0u8; cs as usize];
                    self.write_cluster(new, &zeros)?;
                    cluster = new;
                }
            }
        }

        let mut written = 0usize;
        let mut cur = cluster;

        loop {
            let start_in = if cluster_offset < offset {
                (offset - cluster_offset) as usize
            } else {
                0
            };
            let space = cs as usize - start_in;
            let to_write = space.min(data.len() - written);

            let mut cbuf = vec![0u8; cs as usize];
            self.read_cluster(cur, &mut cbuf)?;
            cbuf[start_in..start_in + to_write]
                .copy_from_slice(&data[written..written + to_write]);
            self.write_cluster(cur, &cbuf)?;

            written += to_write;
            cluster_offset += cs;

            if written >= data.len() {
                break;
            }

            match self.next_cluster(cur) {
                Some(next) => cur = next,
                None => {
                    let new = self.alloc_cluster()?;
                    self.write_fat_entry(cur, new)?;
                    let zeros = vec![0u8; cs as usize];
                    self.write_cluster(new, &zeros)?;
                    cur = new;
                }
            }
        }

        let new_size = (offset + data.len() as u32).max(old_size);
        Ok((first, new_size))
    }

    // =================================================================
    // Public API — directory entry creation / deletion
    // =================================================================

    /// Build a complete entry set (File + Stream + FileName entries) as raw bytes.
    fn build_entry_set(
        name: &str,
        attributes: u16,
        first_cluster: u32,
        data_length: u64,
        contiguous: bool,
    ) -> Vec<u8> {
        let utf16: Vec<u16> = name.bytes().map(|b| b as u16).collect();
        let name_len = utf16.len();
        let fn_entries = (name_len + 14) / 15;
        let secondary = 1 + fn_entries; // Stream + FileName(s)
        let total = 1 + secondary;
        let mut set = vec![0u8; total * 32];

        // -- File Directory Entry (0x85) --
        set[0] = ENTRY_FILE;
        set[1] = secondary as u8;
        // [2..3] = SetChecksum (filled last)
        set[4..6].copy_from_slice(&attributes.to_le_bytes());

        // -- Stream Extension (0xC0) --
        let s = 32;
        set[s] = ENTRY_STREAM;
        let mut flags: u8 = 0x01; // AllocationPossible
        if contiguous {
            flags |= FLAG_CONTIGUOUS;
        }
        set[s + 1] = flags;
        set[s + 3] = name_len as u8;
        let nh = Self::name_hash(&utf16);
        set[s + 4..s + 6].copy_from_slice(&nh.to_le_bytes());
        set[s + 8..s + 16].copy_from_slice(&data_length.to_le_bytes()); // ValidDataLength
        set[s + 20..s + 24].copy_from_slice(&first_cluster.to_le_bytes());
        set[s + 24..s + 32].copy_from_slice(&data_length.to_le_bytes()); // DataLength

        // -- FileName entries (0xC1) --
        for fi in 0..fn_entries {
            let f = (2 + fi) * 32;
            set[f] = ENTRY_FILENAME;
            for j in 0..15 {
                let ci = fi * 15 + j;
                let ch = if ci < utf16.len() { utf16[ci] } else { 0x0000 };
                set[f + 2 + j * 2..f + 4 + j * 2].copy_from_slice(&ch.to_le_bytes());
            }
        }

        // -- Checksum --
        let checksum = Self::entry_set_checksum(&set, total);
        set[2..4].copy_from_slice(&checksum.to_le_bytes());

        set
    }

    /// Find `count` consecutive free 32-byte entry slots in a directory buffer.
    fn find_free_entries(buf: &[u8], count: usize) -> Option<usize> {
        let max = buf.len() / 32;
        let mut run_start = 0;
        let mut run_len = 0;

        for idx in 0..max {
            let off = idx * 32;
            let etype = buf[off];

            if etype == 0x00 {
                // End-of-directory — all remaining slots are free
                if run_len == 0 {
                    run_start = idx;
                }
                let available = max - run_start;
                return if available >= count { Some(run_start * 32) } else { None };
            }

            if etype & 0x80 == 0 {
                // Deleted entry (InUse bit cleared)
                if run_len == 0 {
                    run_start = idx;
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

    /// Create a new file or directory entry in a parent directory.
    pub fn create_entry(
        &mut self,
        parent_cluster: u32,
        name: &str,
        is_dir: bool,
        first_cluster: u32,
        data_length: u64,
    ) -> Result<(), FsError> {
        let attr = if is_dir { ATTR_DIRECTORY } else { ATTR_ARCHIVE };
        let entry_set = Self::build_entry_set(name, attr, first_cluster, data_length, false);
        let num = entry_set.len() / 32;
        let cs = self.cluster_size() as usize;
        let mut cur = parent_cluster;

        loop {
            let mut cbuf = vec![0u8; cs];
            self.read_cluster(cur, &mut cbuf)?;

            if let Some(off) = Self::find_free_entries(&cbuf, num) {
                cbuf[off..off + entry_set.len()].copy_from_slice(&entry_set);
                self.write_cluster(cur, &cbuf)?;
                return Ok(());
            }

            match self.next_cluster(cur) {
                Some(next) => cur = next,
                None => {
                    let new = self.alloc_cluster()?;
                    self.write_fat_entry(cur, new)?;
                    let mut new_buf = vec![0u8; cs];
                    new_buf[..entry_set.len()].copy_from_slice(&entry_set);
                    self.write_cluster(new, &new_buf)?;
                    return Ok(());
                }
            }
        }
    }

    /// Create a new empty file.
    pub fn create_file(&mut self, parent_cluster: u32, name: &str) -> Result<(), FsError> {
        let raw = self.read_dir_raw(parent_cluster)?;
        if self.find_entry_in_buf(&raw, name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        self.create_entry(parent_cluster, name, false, 0, 0)
    }

    /// Create a new subdirectory. Returns the new cluster.
    pub fn create_dir(&mut self, parent_cluster: u32, name: &str) -> Result<u32, FsError> {
        let raw = self.read_dir_raw(parent_cluster)?;
        if self.find_entry_in_buf(&raw, name).is_some() {
            return Err(FsError::AlreadyExists);
        }
        let cluster = self.alloc_cluster()?;
        let cs = self.cluster_size() as usize;
        let zeros = vec![0u8; cs];
        self.write_cluster(cluster, &zeros)?;
        self.create_entry(parent_cluster, name, true, cluster, 0)?;
        Ok(cluster)
    }

    /// Delete a file or directory and free its cluster chain.
    pub fn delete_file(&mut self, parent_cluster: u32, name: &str) -> Result<(), FsError> {
        let cs = self.cluster_size() as usize;
        let mut cur = parent_cluster;

        loop {
            let mut cbuf = vec![0u8; cs];
            self.read_cluster(cur, &mut cbuf)?;

            if let Some(found) = self.find_entry_in_buf(&cbuf, name) {
                let total = 1 + found.secondary_count as usize;
                let off = found.file_entry_offset;
                // Mark all entries as deleted (clear InUse bit 7)
                for e in 0..total {
                    let eoff = off + e * 32;
                    if eoff < cbuf.len() {
                        cbuf[eoff] &= 0x7F;
                    }
                }
                self.write_cluster(cur, &cbuf)?;
                if found.first_cluster >= 2 {
                    self.free_chain(found.first_cluster, found.contiguous, found.data_length)?;
                }
                return Ok(());
            }

            match self.next_cluster(cur) {
                Some(next) => cur = next,
                None => return Err(FsError::NotFound),
            }
        }
    }

    /// Update a file's size and first cluster in its directory entry.
    pub fn update_entry(
        &mut self,
        parent_cluster: u32,
        name: &str,
        new_size: u32,
        new_cluster: u32,
    ) -> Result<(), FsError> {
        let cs = self.cluster_size() as usize;
        let mut cur = parent_cluster;

        loop {
            let mut cbuf = vec![0u8; cs];
            self.read_cluster(cur, &mut cbuf)?;

            if let Some(found) = self.find_entry_in_buf(&cbuf, name) {
                let off = found.file_entry_offset;
                let s = off + 32; // Stream Extension offset
                let sz = new_size as u64;

                cbuf[s + 8..s + 16].copy_from_slice(&sz.to_le_bytes()); // ValidDataLength
                cbuf[s + 20..s + 24].copy_from_slice(&new_cluster.to_le_bytes());
                cbuf[s + 24..s + 32].copy_from_slice(&sz.to_le_bytes()); // DataLength

                // Clear contiguous flag (writes may fragment)
                cbuf[s + 1] = (cbuf[s + 1] & !FLAG_CONTIGUOUS) | 0x01;

                // Recompute checksum
                let total = 1 + found.secondary_count as usize;
                let checksum = Self::entry_set_checksum(&cbuf[off..], total);
                cbuf[off + 2..off + 4].copy_from_slice(&checksum.to_le_bytes());

                self.write_cluster(cur, &cbuf)?;
                return Ok(());
            }

            match self.next_cluster(cur) {
                Some(next) => cur = next,
                None => return Err(FsError::NotFound),
            }
        }
    }

    /// Truncate a file to zero length.
    pub fn truncate_file(&mut self, parent_cluster: u32, name: &str) -> Result<(), FsError> {
        let raw = self.read_dir_raw(parent_cluster)?;
        let found = self.find_entry_in_buf(&raw, name).ok_or(FsError::NotFound)?;
        if found.first_cluster >= 2 {
            self.free_chain(found.first_cluster, found.contiguous, found.data_length)?;
        }
        self.update_entry(parent_cluster, name, 0, 0)
    }
}
