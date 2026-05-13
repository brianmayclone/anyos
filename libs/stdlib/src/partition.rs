//! Shared partition type codes and display names for userland tools.

/// Userland partition type code and display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartitionTypeInfo {
    pub code: u8,
    pub name: &'static str,
}

pub const PART_EMPTY: u8 = 0x00;
pub const PART_FAT12: u8 = 0x01;
pub const PART_FAT16_SMALL: u8 = 0x04;
pub const PART_FAT16: u8 = 0x06;
pub const PART_NTFS_EXFAT: u8 = 0x07;
pub const PART_FAT32: u8 = 0x0B;
pub const PART_FAT32_LBA: u8 = 0x0C;
pub const PART_FAT16_LBA: u8 = 0x0E;
pub const PART_LINUX_SWAP: u8 = 0x82;
pub const PART_LINUX: u8 = 0x83;
pub const PART_GPT_BASIC_DATA: u8 = 0xBD;
pub const PART_GPT_LINUX: u8 = 0xBE;
pub const PART_COREFS: u8 = 0xCF;
pub const PART_GPT_PROTECTIVE: u8 = 0xEE;
pub const PART_EFI_SYSTEM: u8 = 0xEF;

/// Types worth showing in CLI pick-lists.
pub const KNOWN_PARTITION_TYPES: &[PartitionTypeInfo] = &[
    PartitionTypeInfo {
        code: PART_FAT12,
        name: "FAT12",
    },
    PartitionTypeInfo {
        code: PART_FAT16,
        name: "FAT16",
    },
    PartitionTypeInfo {
        code: PART_FAT16_LBA,
        name: "FAT16 (LBA)",
    },
    PartitionTypeInfo {
        code: PART_FAT32,
        name: "FAT32",
    },
    PartitionTypeInfo {
        code: PART_FAT32_LBA,
        name: "FAT32 (LBA)",
    },
    PartitionTypeInfo {
        code: PART_NTFS_EXFAT,
        name: "NTFS/exFAT",
    },
    PartitionTypeInfo {
        code: PART_LINUX_SWAP,
        name: "Linux swap",
    },
    PartitionTypeInfo {
        code: PART_LINUX,
        name: "Linux",
    },
    PartitionTypeInfo {
        code: PART_GPT_BASIC_DATA,
        name: "GPT Basic Data",
    },
    PartitionTypeInfo {
        code: PART_GPT_LINUX,
        name: "GPT Linux",
    },
    PartitionTypeInfo {
        code: PART_COREFS,
        name: "CoreFS",
    },
    PartitionTypeInfo {
        code: PART_GPT_PROTECTIVE,
        name: "GPT protective",
    },
    PartitionTypeInfo {
        code: PART_EFI_SYSTEM,
        name: "EFI System",
    },
];

/// Return the display name for a partition type code.
///
/// The kernel maps GPT GUIDs to synthetic userland codes before exposing them
/// through `sys::disk_partitions`; these names intentionally cover both MBR
/// bytes and those stable GPT codes.
pub fn partition_type_name(code: u8) -> &'static str {
    match code {
        PART_EMPTY => "Empty",
        PART_FAT12 => "FAT12",
        PART_FAT16_SMALL | PART_FAT16 => "FAT16",
        PART_FAT16_LBA => "FAT16 (LBA)",
        PART_FAT32 => "FAT32",
        PART_FAT32_LBA => "FAT32 (LBA)",
        PART_NTFS_EXFAT => "NTFS/exFAT",
        PART_LINUX_SWAP => "Linux swap",
        PART_LINUX => "Linux",
        PART_GPT_BASIC_DATA => "GPT Basic Data",
        PART_GPT_LINUX => "GPT Linux",
        PART_COREFS => "CoreFS",
        PART_GPT_PROTECTIVE => "GPT protective",
        PART_EFI_SYSTEM => "EFI System",
        _ => "Unknown",
    }
}

/// Return known metadata for an exact partition type code.
pub fn partition_type_info(code: u8) -> Option<PartitionTypeInfo> {
    for info in KNOWN_PARTITION_TYPES {
        if info.code == code {
            return Some(*info);
        }
    }
    None
}
