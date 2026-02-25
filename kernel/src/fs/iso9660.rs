//! ISO 9660 filesystem driver (read-only).
//!
//! Supports reading files and directories from CD-ROM/DVD-ROM media.
//! ISO 9660 uses 2048-byte logical blocks. The Primary Volume Descriptor
//! is at LBA 16 and contains the root directory record.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::FsError;

/// ISO 9660 logical block size.
const ISO_BLOCK_SIZE: usize = 2048;

/// Primary Volume Descriptor type.
const VD_TYPE_PRIMARY: u8 = 1;
/// Volume Descriptor Set Terminator.
const VD_TYPE_TERMINATOR: u8 = 255;
/// Standard identifier for ISO 9660 Volume Descriptors.
const ISO_STANDARD_ID: &[u8; 5] = b"CD001";

/// Parsed ISO 9660 filesystem state.
pub struct Iso9660Fs {
    /// Root directory extent (LBA).
    root_lba: u32,
    /// Root directory extent size in bytes.
    root_size: u32,
    /// Volume identifier string.
    pub volume_id: String,
    /// Total number of logical blocks.
    pub total_blocks: u32,
}

/// A parsed directory record.
struct DirRecord {
    lba: u32,
    data_length: u32,
    flags: u8,
    name: String,
}

impl Iso9660Fs {
    /// Try to mount an ISO 9660 filesystem by reading the Primary Volume Descriptor.
    pub fn new() -> Result<Self, FsError> {
        let mut buf = vec![0u8; ISO_BLOCK_SIZE];

        for lba in 16u32..32 {
            if !read_cd_block(lba, &mut buf) {
                return Err(FsError::IoError);
            }

            let vd_type = buf[0];
            if &buf[1..6] != ISO_STANDARD_ID {
                return Err(FsError::IoError);
            }

            if vd_type == VD_TYPE_TERMINATOR {
                break;
            }

            if vd_type == VD_TYPE_PRIMARY {
                return Self::parse_pvd(&buf);
            }
        }

        Err(FsError::NotFound)
    }

    fn parse_pvd(pvd: &[u8]) -> Result<Self, FsError> {
        let total_blocks = u32::from_le_bytes([pvd[80], pvd[81], pvd[82], pvd[83]]);

        let block_size = u16::from_le_bytes([pvd[128], pvd[129]]);
        if block_size != ISO_BLOCK_SIZE as u16 {
            crate::serial_println!("  ISO9660: unexpected block size {}", block_size);
            return Err(FsError::IoError);
        }

        // Root directory record at offset 156 (34 bytes)
        let rr = &pvd[156..190];
        let root_lba = u32::from_le_bytes([rr[2], rr[3], rr[4], rr[5]]);
        let root_size = u32::from_le_bytes([rr[10], rr[11], rr[12], rr[13]]);

        // Volume identifier (32 bytes at offset 40, space-padded ASCII)
        let vol_id = core::str::from_utf8(&pvd[40..72])
            .unwrap_or("")
            .trim();

        crate::serial_println!(
            "[OK] ISO 9660: '{}', {} blocks, root at LBA {}",
            vol_id, total_blocks, root_lba
        );

        Ok(Iso9660Fs {
            root_lba,
            root_size,
            volume_id: String::from(vol_id),
            total_blocks,
        })
    }

    /// Read all directory records from a directory extent.
    fn read_directory(&self, extent_lba: u32, extent_size: u32) -> Result<Vec<DirRecord>, FsError> {
        let blocks_needed = (extent_size as usize + ISO_BLOCK_SIZE - 1) / ISO_BLOCK_SIZE;
        let mut data = vec![0u8; blocks_needed * ISO_BLOCK_SIZE];

        for i in 0..blocks_needed {
            if !read_cd_block(extent_lba + i as u32, &mut data[i * ISO_BLOCK_SIZE..]) {
                return Err(FsError::IoError);
            }
        }

        let mut records = Vec::new();
        let mut offset = 0usize;

        while offset < extent_size as usize {
            if offset >= data.len() {
                break;
            }
            let record_len = data[offset] as usize;
            if record_len == 0 {
                // Skip to next block boundary
                let next_block = ((offset / ISO_BLOCK_SIZE) + 1) * ISO_BLOCK_SIZE;
                if next_block >= extent_size as usize {
                    break;
                }
                offset = next_block;
                continue;
            }

            if offset + record_len > data.len() || record_len < 33 {
                break;
            }

            let record = &data[offset..offset + record_len];
            let lba = u32::from_le_bytes([record[2], record[3], record[4], record[5]]);
            let data_length = u32::from_le_bytes([record[10], record[11], record[12], record[13]]);
            let flags = record[25];
            let name_len = record[32] as usize;

            if name_len > 0 && 33 + name_len <= record_len {
                let name_bytes = &record[33..33 + name_len];

                // Skip "." (0x00) and ".." (0x01) entries
                if name_len == 1 && (name_bytes[0] == 0 || name_bytes[0] == 1) {
                    offset += record_len;
                    continue;
                }

                let name = iso_name_to_string(name_bytes);

                records.push(DirRecord {
                    lba,
                    data_length,
                    flags,
                    name,
                });
            }

            offset += record_len;
        }

        Ok(records)
    }

    /// Resolve a full path to (LBA, size, is_directory).
    fn resolve_path(&self, path: &str) -> Result<(u32, u32, bool), FsError> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Ok((self.root_lba, self.root_size, true));
        }

        let mut cur_lba = self.root_lba;
        let mut cur_size = self.root_size;
        let mut is_dir = true;

        for component in path.split('/') {
            if component.is_empty() {
                continue;
            }
            if !is_dir {
                return Err(FsError::NotADirectory);
            }

            let records = self.read_directory(cur_lba, cur_size)?;
            let name_upper = component.to_uppercase();

            let mut found = false;
            for record in records {
                if record.name.to_uppercase() == name_upper {
                    cur_lba = record.lba;
                    cur_size = record.data_length;
                    is_dir = record.flags & 0x02 != 0;
                    found = true;
                    break;
                }
            }

            if !found {
                return Err(FsError::NotFound);
            }
        }

        Ok((cur_lba, cur_size, is_dir))
    }

    /// Look up a path and return (inode=LBA, file_type, size).
    pub fn lookup(&self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        let (lba, size, is_dir) = self.resolve_path(path)?;
        let file_type = if is_dir { FileType::Directory } else { FileType::Regular };
        Ok((lba, file_type, size))
    }

    /// Read bytes from a file at a given offset. inode = extent LBA.
    pub fn read_file(&self, extent_lba: u32, offset: u32, buf: &mut [u8], file_size: u32) -> Result<usize, FsError> {
        if offset >= file_size {
            return Ok(0);
        }

        let remaining = (file_size - offset) as usize;
        let to_read = buf.len().min(remaining);
        if to_read == 0 {
            return Ok(0);
        }

        let block_start = offset as usize / ISO_BLOCK_SIZE;
        let offset_in_block = offset as usize % ISO_BLOCK_SIZE;

        let mut block_buf = vec![0u8; ISO_BLOCK_SIZE];
        let mut bytes_read = 0usize;
        let mut cur_block = block_start;
        let mut cur_offset = offset_in_block;

        while bytes_read < to_read {
            if !read_cd_block(extent_lba + cur_block as u32, &mut block_buf) {
                if bytes_read > 0 {
                    return Ok(bytes_read);
                }
                return Err(FsError::IoError);
            }

            let available = ISO_BLOCK_SIZE - cur_offset;
            let to_copy = available.min(to_read - bytes_read);
            buf[bytes_read..bytes_read + to_copy]
                .copy_from_slice(&block_buf[cur_offset..cur_offset + to_copy]);
            bytes_read += to_copy;
            cur_block += 1;
            cur_offset = 0;
        }

        Ok(bytes_read)
    }

    /// Read all directory entries at a given path.
    pub fn read_dir(&self, extent_lba: u32, extent_size: u32) -> Result<Vec<DirEntry>, FsError> {
        let records = self.read_directory(extent_lba, extent_size)?;
        Ok(records
            .into_iter()
            .map(|r| DirEntry {
                name: r.name,
                file_type: if r.flags & 0x02 != 0 {
                    FileType::Directory
                } else {
                    FileType::Regular
                },
                size: r.data_length,
                is_symlink: false,
                uid: 0, gid: 0, mode: 0xFFF,
            })
            .collect())
    }

    /// Read an entire file into a Vec<u8>.
    pub fn read_file_to_vec(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let (lba, file_type, size) = self.lookup(path)?;
        if file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        if size == 0 {
            return Ok(Vec::new());
        }

        let mut data = vec![0u8; size as usize];
        let read = self.read_file(lba, 0, &mut data, size)?;
        data.truncate(read);
        Ok(data)
    }
}

/// Convert ISO 9660 filename to a clean string.
/// Strips version number (";1") and trailing dots, converts to lowercase.
fn iso_name_to_string(bytes: &[u8]) -> String {
    let s = core::str::from_utf8(bytes).unwrap_or("");
    // Strip version number (;N)
    let name = if let Some(pos) = s.find(';') {
        &s[..pos]
    } else {
        s
    };
    // Strip trailing dot
    let name = name.trim_end_matches('.');
    // Convert to lowercase
    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        result.push(if c.is_ascii_uppercase() {
            (c as u8 + 32) as char
        } else {
            c
        });
    }
    result
}

/// Read a single 2048-byte CD block from USB CDROM or ATAPI drive.
fn read_cd_block(lba: u32, buf: &mut [u8]) -> bool {
    // Try USB CDROM first (via storage I/O override)
    if let Some(disk_id) = crate::drivers::usb::storage::first_cdrom_disk_id() {
        return crate::drivers::storage::read_via_override(disk_id, lba, 1, buf);
    }
    // Fall back to IDE ATAPI CD-ROM
    crate::drivers::storage::atapi::read_sectors(lba, 1, buf)
}
