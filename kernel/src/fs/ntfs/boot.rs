//! NTFS boot sector (VBR) parsing.
//!
//! The NTFS boot sector contains the BPB (BIOS Parameter Block) with
//! geometry information and NTFS-specific fields like MFT location.

/// Parsed NTFS boot sector parameters.
#[derive(Debug)]
pub(super) struct NtfsBpb {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub cluster_size: u32,
    pub total_sectors: u64,
    pub mft_cluster: u64,
    pub mft_mirror_cluster: u64,
    pub mft_record_size: u32,
    pub index_record_size: u32,
}

impl NtfsBpb {
    /// Parse an NTFS boot sector from a 512-byte buffer.
    ///
    /// Returns `None` if the OEM signature is not "NTFS    ".
    pub fn parse(buf: &[u8; 512]) -> Option<Self> {
        // OEM ID at offset 3: must be "NTFS    " (8 bytes)
        if &buf[3..11] != b"NTFS    " {
            return None;
        }

        let bytes_per_sector = u16::from_le_bytes([buf[0x0B], buf[0x0C]]);
        let sectors_per_cluster = buf[0x0D];

        if bytes_per_sector == 0 || sectors_per_cluster == 0 {
            return None;
        }

        let cluster_size = bytes_per_sector as u32 * sectors_per_cluster as u32;
        let total_sectors = u64::from_le_bytes([
            buf[0x28], buf[0x29], buf[0x2A], buf[0x2B],
            buf[0x2C], buf[0x2D], buf[0x2E], buf[0x2F],
        ]);

        let mft_cluster = u64::from_le_bytes([
            buf[0x30], buf[0x31], buf[0x32], buf[0x33],
            buf[0x34], buf[0x35], buf[0x36], buf[0x37],
        ]);

        let mft_mirror_cluster = u64::from_le_bytes([
            buf[0x38], buf[0x39], buf[0x3A], buf[0x3B],
            buf[0x3C], buf[0x3D], buf[0x3E], buf[0x3F],
        ]);

        // MFT record size: signed byte at 0x40.
        // Positive → clusters per record, negative → 2^|val| bytes.
        let mft_record_size = decode_record_size(buf[0x40], cluster_size);

        // Index record size: signed byte at 0x44 (same encoding).
        let index_record_size = decode_record_size(buf[0x44], cluster_size);

        Some(NtfsBpb {
            bytes_per_sector,
            sectors_per_cluster,
            cluster_size,
            total_sectors,
            mft_cluster,
            mft_mirror_cluster,
            mft_record_size,
            index_record_size,
        })
    }
}

/// Decode NTFS record size field (used for MFT and index records).
///
/// If the value is positive, it's clusters per record.
/// If negative, the size is 2^|val| bytes.
fn decode_record_size(raw: u8, cluster_size: u32) -> u32 {
    let signed = raw as i8;
    if signed > 0 {
        signed as u32 * cluster_size
    } else {
        1u32 << (-(signed as i32) as u32)
    }
}
