//! MFT (Master File Table) record parsing.
//!
//! Each MFT record is typically 1024 bytes, begins with "FILE" signature,
//! and contains a fixup array that must be applied before reading attributes.

use alloc::vec;
use alloc::vec::Vec;
use crate::fs::vfs::FsError;
use super::attr::{self, NtfsAttr};
use super::runlist::{self, DataRun};

/// Well-known MFT record numbers.
pub(super) mod records {
    pub const MFT: u64 = 0;
    pub const MFT_MIRROR: u64 = 1;
    pub const LOG_FILE: u64 = 2;
    pub const VOLUME: u64 = 3;
    pub const ROOT_DIR: u64 = 5;
    pub const BITMAP: u64 = 6;
    pub const UPCASE: u64 = 10;
}

/// Parsed MFT record header.
#[derive(Debug)]
pub(super) struct MftRecord {
    /// Raw record bytes (after fixup).
    pub data: Vec<u8>,
    /// Offset of first attribute.
    pub attr_offset: u16,
    /// Flags: 0x01 = in use, 0x02 = directory.
    pub flags: u16,
    /// Base record reference (0 if this is a base record).
    pub base_record: u64,
}

impl MftRecord {
    /// Parse an MFT record from raw bytes. Applies fixup array.
    pub fn parse(raw: &[u8], record_size: u32) -> Result<Self, FsError> {
        let size = record_size as usize;
        if raw.len() < size || size < 48 {
            return Err(FsError::IoError);
        }

        // Verify "FILE" signature
        if &raw[0..4] != b"FILE" {
            return Err(FsError::IoError);
        }

        let fixup_offset = u16::from_le_bytes([raw[0x04], raw[0x05]]) as usize;
        let fixup_count = u16::from_le_bytes([raw[0x06], raw[0x07]]) as usize;
        let attr_offset = u16::from_le_bytes([raw[0x14], raw[0x15]]);
        let flags = u16::from_le_bytes([raw[0x16], raw[0x17]]);
        let base_record = u48_to_u64(&raw[0x20..0x26]);

        // Copy record data so we can apply fixups
        let mut data = vec![0u8; size];
        data[..size].copy_from_slice(&raw[..size]);

        // Apply fixup array
        apply_fixup(&mut data, fixup_offset, fixup_count)?;

        Ok(MftRecord {
            data,
            attr_offset,
            flags,
            base_record,
        })
    }

    /// Check if this record is in use.
    pub fn in_use(&self) -> bool {
        self.flags & 0x01 != 0
    }

    /// Check if this record is a directory.
    pub fn is_directory(&self) -> bool {
        self.flags & 0x02 != 0
    }

    /// Parse all attributes in this record.
    pub fn attributes(&self) -> Vec<NtfsAttr> {
        attr::parse_attributes(&self.data, self.attr_offset)
    }

    /// Find the first attribute of a given type with the given name (or unnamed).
    pub fn find_attr(&self, attr_type: u32, named: Option<&str>) -> Option<NtfsAttr> {
        self.attributes().into_iter().find(|a| {
            if a.header.attr_type != attr_type {
                return false;
            }
            match named {
                None => a.is_unnamed(),
                Some(name) => a.name.as_deref() == Some(name),
            }
        })
    }

    /// Get resident data for an attribute.
    pub fn get_resident_data(&self, attr: &NtfsAttr) -> Option<&[u8]> {
        attr::get_resident_data(&self.data, attr)
    }

    /// Decode data runs for a non-resident attribute.
    pub fn get_data_runs(&self, attr: &NtfsAttr) -> Vec<DataRun> {
        let nr = match &attr.non_resident {
            Some(nr) => nr,
            None => return Vec::new(),
        };
        let offset = attr.record_offset + nr.data_runs_offset as usize;
        if offset >= self.data.len() {
            return Vec::new();
        }
        runlist::decode_data_runs(&self.data[offset..])
    }
}

/// Apply the NTFS fixup array to a record buffer.
///
/// The fixup array stores the original last 2 bytes of each 512-byte sector,
/// which were replaced with a signature value for integrity checking.
fn apply_fixup(data: &mut [u8], fixup_offset: usize, fixup_count: usize) -> Result<(), FsError> {
    if fixup_count < 2 || fixup_offset + fixup_count * 2 > data.len() {
        return Err(FsError::IoError);
    }

    // First entry is the signature value
    let sig = u16::from_le_bytes([data[fixup_offset], data[fixup_offset + 1]]);

    // Entries 1..fixup_count-1 are the original values for each sector
    for i in 1..fixup_count {
        let sector_end = i * 512;
        if sector_end > data.len() {
            break;
        }

        // Verify the signature at end of each sector
        let found = u16::from_le_bytes([data[sector_end - 2], data[sector_end - 1]]);
        if found != sig {
            return Err(FsError::IoError);
        }

        // Replace with original bytes from fixup array
        let fixup_pos = fixup_offset + i * 2;
        data[sector_end - 2] = data[fixup_pos];
        data[sector_end - 1] = data[fixup_pos + 1];
    }

    Ok(())
}

/// Extract a 48-bit (6-byte) little-endian MFT file reference number.
/// The low 48 bits are the record number, upper 16 bits are sequence number.
fn u48_to_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5], 0, 0,
    ])
}

/// Extract MFT record number from a file reference (low 48 bits).
pub(super) fn file_ref_to_record(file_ref: u64) -> u64 {
    file_ref & 0x0000_FFFF_FFFF_FFFF
}

/// Parse $FILE_NAME attribute data to extract the filename and parent reference.
#[derive(Debug)]
pub(super) struct FileName {
    pub parent_ref: u64,
    pub name: alloc::string::String,
    pub namespace: u8,
    pub flags: u32,
    pub real_size: u64,
}

impl FileName {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 66 {
            return None;
        }

        let parent_ref = u64::from_le_bytes([
            data[0], data[1], data[2], data[3],
            data[4], data[5], data[6], data[7],
        ]);

        let real_size = u64::from_le_bytes([
            data[0x30], data[0x31], data[0x32], data[0x33],
            data[0x34], data[0x35], data[0x36], data[0x37],
        ]);

        let flags = u32::from_le_bytes([
            data[0x38], data[0x39], data[0x3A], data[0x3B],
        ]);

        let name_length = data[0x40] as usize;
        let namespace = data[0x41];

        if data.len() < 0x42 + name_length * 2 {
            return None;
        }

        let mut name = alloc::string::String::new();
        for i in 0..name_length {
            let c = u16::from_le_bytes([
                data[0x42 + i * 2],
                data[0x42 + i * 2 + 1],
            ]);
            if c < 128 {
                name.push(c as u8 as char);
            } else {
                name.push('?');
            }
        }

        Some(FileName {
            parent_ref,
            name,
            namespace,
            flags,
            real_size,
        })
    }

    /// Check if this is a DOS 8.3 name (namespace 2) — we prefer Win32 names.
    pub fn is_dos_name(&self) -> bool {
        self.namespace == 2
    }
}
