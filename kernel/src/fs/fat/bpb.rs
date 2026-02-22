//! FAT Boot Parameter Block (BPB) structures and constants.

/// FAT BIOS Parameter Block (BPB) layout at the start of the partition.
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

/// FAT16 end-of-chain marker threshold.
pub const FAT16_EOC: u16 = 0xFFF8;
/// FAT32 end-of-chain marker threshold.
pub const FAT32_EOC: u32 = 0x0FFFFFF8;
/// FAT32 entry mask (upper 4 bits are reserved).
pub const FAT32_MASK: u32 = 0x0FFFFFFF;
