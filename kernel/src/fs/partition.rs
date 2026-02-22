//! MBR and GPT partition table parsing.
//!
//! Reads sector 0 of a disk to detect the partition scheme (MBR or GPT),
//! then parses the partition entries and returns a structured table.

use alloc::vec::Vec;
use crate::serial_println;

/// Partition table scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionScheme {
    Mbr,
    Gpt,
    None,
}

/// Partition type (from MBR type byte or GPT type GUID).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionType {
    Empty,
    Fat12,
    Fat16,
    Fat16Lba,
    Fat32,
    Fat32Lba,
    NtfsExfat,   // MBR type 0x07 — disambiguated by VBR OEM check
    LinuxSwap,
    LinuxNative,
    GptEsp,
    GptBasicData,
    GptLinuxFs,
    Unknown(u8),
}

/// A single partition entry.
#[derive(Debug, Clone)]
pub struct Partition {
    pub index: u8,
    pub part_type: PartitionType,
    pub start_lba: u64,
    pub size_sectors: u64,
    pub bootable: bool,
    pub scheme: PartitionScheme,
}

/// Parsed partition table for a disk.
#[derive(Debug, Clone)]
pub struct DiskPartitionTable {
    pub scheme: PartitionScheme,
    pub partitions: Vec<Partition>,
}

// Well-known GPT type GUIDs (mixed-endian as stored on disk).
const GUID_EFI_SYSTEM: [u8; 16] = [
    0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11,
    0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
];
const GUID_BASIC_DATA: [u8; 16] = [
    0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44,
    0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7,
];
const GUID_LINUX_FS: [u8; 16] = [
    0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47,
    0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4,
];

fn le16(buf: &[u8], off: usize) -> u16 {
    (buf[off] as u16) | ((buf[off + 1] as u16) << 8)
}

fn le32(buf: &[u8], off: usize) -> u32 {
    (buf[off] as u32)
        | ((buf[off + 1] as u32) << 8)
        | ((buf[off + 2] as u32) << 16)
        | ((buf[off + 3] as u32) << 24)
}

fn le64(buf: &[u8], off: usize) -> u64 {
    (le32(buf, off) as u64) | ((le32(buf, off + 4) as u64) << 32)
}

/// Map an MBR partition type byte to our enum.
fn mbr_type(t: u8) -> PartitionType {
    match t {
        0x00 => PartitionType::Empty,
        0x01 => PartitionType::Fat12,
        0x04 | 0x06 => PartitionType::Fat16,
        0x0E => PartitionType::Fat16Lba,
        0x0B => PartitionType::Fat32,
        0x0C => PartitionType::Fat32Lba,
        0x07 => PartitionType::NtfsExfat,
        0x82 => PartitionType::LinuxSwap,
        0x83 => PartitionType::LinuxNative,
        other => PartitionType::Unknown(other),
    }
}

/// Map a GPT type GUID to our enum.
fn gpt_type(guid: &[u8; 16]) -> PartitionType {
    if *guid == GUID_EFI_SYSTEM {
        PartitionType::GptEsp
    } else if *guid == GUID_BASIC_DATA {
        PartitionType::GptBasicData
    } else if *guid == GUID_LINUX_FS {
        PartitionType::GptLinuxFs
    } else {
        // Check for all-zero (empty entry)
        if guid.iter().all(|&b| b == 0) {
            PartitionType::Empty
        } else {
            PartitionType::Unknown(0xFF)
        }
    }
}

/// Scan a disk's partition table by reading its sectors.
///
/// `read_sector_fn` reads a single 512-byte sector at the given LBA into `buf`.
/// Returns `true` on success.
pub fn scan_disk<F>(read_sector_fn: F) -> DiskPartitionTable
where
    F: Fn(u64, &mut [u8]) -> bool,
{
    let mut sector = [0u8; 512];

    // Read MBR (LBA 0)
    if !read_sector_fn(0, &mut sector) {
        serial_println!("[partition] failed to read LBA 0");
        return DiskPartitionTable { scheme: PartitionScheme::None, partitions: Vec::new() };
    }

    // Check MBR signature
    if sector[510] != 0x55 || sector[511] != 0xAA {
        serial_println!("[partition] no MBR signature (0x55AA) found");
        return DiskPartitionTable { scheme: PartitionScheme::None, partitions: Vec::new() };
    }

    // Check for protective MBR (GPT indicator)
    let mut has_gpt_indicator = false;
    for i in 0..4u8 {
        let off = 446 + (i as usize) * 16;
        let ptype = sector[off + 4];
        if ptype == 0xEE {
            has_gpt_indicator = true;
            break;
        }
    }

    if has_gpt_indicator {
        // Try GPT parsing
        if let Some(table) = parse_gpt(&read_sector_fn) {
            return table;
        }
        serial_println!("[partition] GPT indicator found but GPT parsing failed, falling back to MBR");
    }

    // Parse MBR partition entries
    parse_mbr(&sector)
}

fn parse_mbr(mbr: &[u8; 512]) -> DiskPartitionTable {
    let mut partitions = Vec::new();

    for i in 0..4u8 {
        let off = 446 + (i as usize) * 16;
        let status = mbr[off];
        let ptype = mbr[off + 4];
        let start = le32(mbr, off + 8);
        let size = le32(mbr, off + 12);

        if ptype == 0x00 || size == 0 {
            continue;
        }

        let part = Partition {
            index: i,
            part_type: mbr_type(ptype),
            start_lba: start as u64,
            size_sectors: size as u64,
            bootable: status == 0x80,
            scheme: PartitionScheme::Mbr,
        };
        serial_println!(
            "[partition] MBR[{}]: type=0x{:02X} start={} size={} {}",
            i, ptype, start, size,
            if status == 0x80 { "(bootable)" } else { "" }
        );
        partitions.push(part);
    }

    DiskPartitionTable {
        scheme: if partitions.is_empty() { PartitionScheme::None } else { PartitionScheme::Mbr },
        partitions,
    }
}

fn parse_gpt<F>(read_sector_fn: &F) -> Option<DiskPartitionTable>
where
    F: Fn(u64, &mut [u8]) -> bool,
{
    let mut sector = [0u8; 512];

    // Read GPT header at LBA 1
    if !read_sector_fn(1, &mut sector) {
        serial_println!("[partition] failed to read GPT header at LBA 1");
        return None;
    }

    // Verify "EFI PART" signature
    if &sector[0..8] != b"EFI PART" {
        serial_println!("[partition] no EFI PART signature at LBA 1");
        return None;
    }

    let entries_lba = le64(&sector, 72);
    let entry_count = le32(&sector, 80);
    let entry_size = le32(&sector, 84);

    serial_println!(
        "[partition] GPT: entries_lba={} count={} entry_size={}",
        entries_lba, entry_count, entry_size
    );

    // Read partition entries
    // Each entry is entry_size bytes (typically 128), packed into 512-byte sectors
    let entries_per_sector = 512 / entry_size;
    let mut partitions = Vec::new();

    let max_entries = entry_count.min(128); // cap at 128
    let sectors_needed = (max_entries + entries_per_sector - 1) / entries_per_sector;

    for s in 0..sectors_needed {
        if !read_sector_fn(entries_lba + s as u64, &mut sector) {
            serial_println!("[partition] failed to read GPT entry sector {}", entries_lba + s as u64);
            break;
        }

        for e in 0..entries_per_sector {
            let idx = s * entries_per_sector + e;
            if idx >= max_entries {
                break;
            }

            let off = (e * entry_size) as usize;
            let mut type_guid = [0u8; 16];
            type_guid.copy_from_slice(&sector[off..off + 16]);

            let ptype = gpt_type(&type_guid);
            if ptype == PartitionType::Empty {
                continue;
            }

            let first_lba = le64(&sector, off + 32);
            let last_lba = le64(&sector, off + 40);

            let part = Partition {
                index: idx as u8,
                part_type: ptype,
                start_lba: first_lba,
                size_sectors: last_lba - first_lba + 1,
                bootable: false,
                scheme: PartitionScheme::Gpt,
            };
            serial_println!(
                "[partition] GPT[{}]: type={:?} start={} end={} size={}",
                idx, ptype, first_lba, last_lba, last_lba - first_lba + 1
            );
            partitions.push(part);
        }
    }

    Some(DiskPartitionTable {
        scheme: PartitionScheme::Gpt,
        partitions,
    })
}

/// Try to detect the filesystem type by reading the VBR (Volume Boot Record)
/// of a partition. Useful for MBR type 0x07 which is shared by NTFS and exFAT.
pub fn detect_fs_from_vbr<F>(read_sector_fn: F, part_start_lba: u64) -> PartitionType
where
    F: Fn(u64, &mut [u8]) -> bool,
{
    let mut vbr = [0u8; 512];
    if !read_sector_fn(part_start_lba, &mut vbr) {
        return PartitionType::Unknown(0);
    }

    // Check OEM name at offset 3 (8 bytes)
    let oem = &vbr[3..11];
    if oem == b"EXFAT   " {
        return PartitionType::NtfsExfat; // Could add ExFat variant if needed
    }
    if oem == b"NTFS    " {
        return PartitionType::NtfsExfat;
    }
    // Check for FAT32 BPB: fat_size_16 == 0 and root_entry_count == 0
    let root_entry_count = le16(&vbr, 17);
    let fat_size_16 = le16(&vbr, 22);
    if root_entry_count == 0 && fat_size_16 == 0 {
        // Likely FAT32 — check for FAT32 extended BPB signature
        let fs_type = &vbr[82..90];
        if fs_type.starts_with(b"FAT32") {
            return PartitionType::Fat32;
        }
    }
    // Check for FAT16
    let fs_type = &vbr[54..62];
    if fs_type.starts_with(b"FAT16") || fs_type.starts_with(b"FAT12") || fs_type.starts_with(b"FAT") {
        return PartitionType::Fat16;
    }

    PartitionType::Unknown(0)
}
