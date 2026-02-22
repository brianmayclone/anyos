//! NTFS directory index ($I30) parsing.
//!
//! NTFS directories use B+ trees stored in $INDEX_ROOT (resident) and
//! $INDEX_ALLOCATION (non-resident) attributes to index file entries.

use alloc::vec::Vec;
use crate::fs::vfs::FsError;
use super::mft::{self, FileName};

/// A directory entry extracted from an NTFS index.
#[derive(Debug)]
pub(super) struct IndexEntry {
    /// MFT file reference for this entry's file/directory.
    pub file_ref: u64,
    /// Parsed $FILE_NAME attribute from the index entry.
    pub file_name: FileName,
}

/// Parse index entries from $INDEX_ROOT attribute data.
///
/// $INDEX_ROOT layout:
/// - 0x00: attribute type (u32, 0x30 for $FILE_NAME)
/// - 0x04: collation rule (u32)
/// - 0x08: index allocation entry size (u32)
/// - 0x0C: clusters per index record (u8)
/// - 0x10: index header (node_entries_offset, total_size, allocated_size, flags)
///
/// Returns (entries, has_sub_nodes) where has_sub_nodes indicates $INDEX_ALLOCATION exists.
pub(super) fn parse_index_root(data: &[u8]) -> (Vec<IndexEntry>, bool) {
    if data.len() < 32 {
        return (Vec::new(), false);
    }

    // Index header starts at offset 0x10
    let entries_offset = u32::from_le_bytes([data[0x10], data[0x11], data[0x12], data[0x13]]) as usize;
    let total_size = u32::from_le_bytes([data[0x14], data[0x15], data[0x16], data[0x17]]) as usize;
    let flags = u32::from_le_bytes([data[0x1C], data[0x1D], data[0x1E], data[0x1F]]);
    let has_sub_nodes = flags & 0x01 != 0;

    let base = 0x10 + entries_offset;
    let end = (0x10 + total_size).min(data.len());

    let entries = parse_index_entries(&data[..end], base);
    (entries, has_sub_nodes)
}

/// Parse index entries from an INDX record (index allocation block).
///
/// INDX layout:
/// - 0x00: "INDX" signature
/// - 0x04-0x05: fixup offset
/// - 0x06-0x07: fixup count
/// - 0x18: index header offset (entries_offset, total_size, ...)
pub(super) fn parse_indx_record(raw: &[u8], record_size: u32) -> Result<Vec<IndexEntry>, FsError> {
    let size = record_size as usize;
    if raw.len() < size || size < 0x28 {
        return Err(FsError::IoError);
    }

    // Verify "INDX" signature
    if &raw[0..4] != b"INDX" {
        return Err(FsError::IoError);
    }

    let fixup_offset = u16::from_le_bytes([raw[0x04], raw[0x05]]) as usize;
    let fixup_count = u16::from_le_bytes([raw[0x06], raw[0x07]]) as usize;

    // Copy and apply fixup
    let mut data = alloc::vec![0u8; size];
    data[..size].copy_from_slice(&raw[..size]);
    apply_indx_fixup(&mut data, fixup_offset, fixup_count)?;

    // Index header at 0x18
    let entries_offset = u32::from_le_bytes([
        data[0x18], data[0x19], data[0x1A], data[0x1B],
    ]) as usize;
    let total_size = u32::from_le_bytes([
        data[0x1C], data[0x1D], data[0x1E], data[0x1F],
    ]) as usize;

    let base = 0x18 + entries_offset;
    let end = (0x18 + total_size).min(data.len());

    Ok(parse_index_entries(&data[..end], base))
}

/// Parse a chain of index entries starting at `base` offset in `data`.
fn parse_index_entries(data: &[u8], mut offset: usize) -> Vec<IndexEntry> {
    let mut entries = Vec::new();

    loop {
        if offset + 16 > data.len() {
            break;
        }

        let file_ref = u64::from_le_bytes([
            data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
            data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
        ]);

        let entry_length = u16::from_le_bytes([data[offset + 8], data[offset + 9]]) as usize;
        let stream_length = u16::from_le_bytes([data[offset + 10], data[offset + 11]]) as usize;
        let entry_flags = u32::from_le_bytes([
            data[offset + 12], data[offset + 13], data[offset + 14], data[offset + 15],
        ]);

        if entry_length < 16 || entry_length > data.len() - offset {
            break;
        }

        // Flag 0x02 = last entry in node (no more entries after this)
        let is_last = entry_flags & 0x02 != 0;

        // Only process if there's actual stream data (file name)
        if stream_length > 0 && offset + 16 + stream_length <= data.len() {
            let stream_data = &data[offset + 16..offset + 16 + stream_length];
            let record_num = mft::file_ref_to_record(file_ref);

            if let Some(fname) = FileName::parse(stream_data) {
                // Skip DOS-only names (namespace 2) to avoid duplicates
                if !fname.is_dos_name() && !is_meta_file(&fname.name) {
                    entries.push(IndexEntry {
                        file_ref: record_num,
                        file_name: fname,
                    });
                }
            }
        }

        if is_last {
            break;
        }

        offset += entry_length;
    }

    entries
}

/// Check if a filename is an NTFS metadata file (starts with '$').
fn is_meta_file(name: &str) -> bool {
    name.starts_with('$')
}

/// Apply fixup array to an INDX record (same algorithm as FILE records).
fn apply_indx_fixup(data: &mut [u8], fixup_offset: usize, fixup_count: usize) -> Result<(), FsError> {
    if fixup_count < 2 || fixup_offset + fixup_count * 2 > data.len() {
        return Err(FsError::IoError);
    }

    let sig = u16::from_le_bytes([data[fixup_offset], data[fixup_offset + 1]]);

    for i in 1..fixup_count {
        let sector_end = i * 512;
        if sector_end > data.len() {
            break;
        }

        let found = u16::from_le_bytes([data[sector_end - 2], data[sector_end - 1]]);
        if found != sig {
            return Err(FsError::IoError);
        }

        let fixup_pos = fixup_offset + i * 2;
        data[sector_end - 2] = data[fixup_pos];
        data[sector_end - 1] = data[fixup_pos + 1];
    }

    Ok(())
}
