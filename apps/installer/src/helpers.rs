//! Small utility functions: disk detection, size formatting, text helpers, OS detection.

use anyos_std::{format, String, Vec};
use anyos_std::sys;
use crate::state::DiskEntry;

/// Scan the kernel block-device list and return entries for both whole disks
/// and their partitions.  Uses the 64-byte entry format to retrieve labels.
pub fn detect_disks() -> Vec<DiskEntry> {
    // 64 bytes per entry, up to 32 devices
    let mut buf = [0u8; 64 * 32];
    let count = sys::disk_list(&mut buf);
    let mut entries = Vec::new();

    for i in 0..count as usize {
        let off = i * 64;
        let device_id = buf[off];
        let disk_id = buf[off + 1];
        let partition = buf[off + 2];
        let size_sectors = u64::from_le_bytes([
            buf[off + 12], buf[off + 13], buf[off + 14], buf[off + 15],
            buf[off + 16], buf[off + 17], buf[off + 18], buf[off + 19],
        ]);

        // Read label (bytes 20..60, NUL-padded)
        let label_raw = &buf[off + 20..off + 60];
        let label_end = label_raw.iter().position(|&b| b == 0).unwrap_or(40);
        let label = core::str::from_utf8(&label_raw[..label_end])
            .unwrap_or("")
            .trim();
        let label = String::from(label);

        if partition == 0xFF {
            // Whole disk — count its partitions
            let part_count = (0..count as usize)
                .filter(|&j| {
                    let jo = j * 64;
                    buf[jo + 1] == disk_id && buf[jo + 2] != 0xFF
                })
                .count() as u32;
            entries.push(DiskEntry {
                device_id,
                disk_id,
                partition_index: None,
                size_sectors,
                partition_count: part_count,
                label,
                part_type_id: 0,
            });
        } else {
            // Partition entry
            entries.push(DiskEntry {
                device_id,
                disk_id,
                partition_index: Some(partition),
                size_sectors,
                partition_count: 0,
                label,
                part_type_id: 0, // could be extended later
            });
        }
    }

    entries.sort_by_key(|d| (d.disk_id, d.partition_index.unwrap_or(0xFF)));
    entries
}

/// Human-readable size string from a sector count (512 B/sector).
pub fn format_size(sectors: u64) -> String {
    let bytes = sectors * 512;
    if bytes >= 1024 * 1024 * 1024 {
        let gb = bytes / (1024 * 1024 * 1024);
        let frac = (bytes % (1024 * 1024 * 1024)) * 10 / (1024 * 1024 * 1024);
        format!("{}.{} GB", gb, frac)
    } else if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else {
        format!("{} KB", bytes / 1024)
    }
}

/// Check if the system was booted via UEFI (boot_mode == 1).
pub fn is_uefi_boot() -> bool {
    // sysinfo cmd=2 (CPU info) returns boot_mode at offset 72
    let mut buf = [0u8; 128];
    sys::sysinfo(2, &mut buf);
    let boot_mode = u32::from_le_bytes([buf[72], buf[73], buf[74], buf[75]]);
    boot_mode == 1
}

/// Try to detect an existing operating system on a disk by reading sector 0.
/// Returns a description like "Windows", "Linux", "anyOS" or None.
pub fn detect_existing_os(device_id: u8) -> Option<String> {
    let mut mbr = [0u8; 512];
    sys::disk_read(device_id as u32, 0, 1, &mut mbr);

    // Not a valid MBR?
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return None;
    }

    // Check partition table entries for known OS types
    let mut has_ntfs = false;
    let mut has_linux = false;
    let mut has_fat = false;
    let mut has_efi = false;

    for i in 0..4 {
        let off = 446 + i * 16;
        let ptype = mbr[off + 4];
        match ptype {
            0x07 => has_ntfs = true,      // NTFS / exFAT
            0x83 => has_linux = true,      // Linux native
            0x82 => has_linux = true,      // Linux swap
            0x0B | 0x0C | 0x06 | 0x0E => has_fat = true, // FAT32/FAT16
            0xEE => has_efi = true,        // GPT protective MBR
            _ => {}
        }
    }

    // Check bootloader signature in first bytes
    // anyOS stage1 starts with EB 76 90 (same as exFAT but with "ANYO" at offset 100)
    let has_anyo_sig = mbr[3..7] == *b"ANYO"
        || (mbr.len() > 103 && mbr[100..104] == *b"ANYO");

    // Check for NTFS signature in VBR of first partition
    let ntfs_oem = &mbr[3..11];
    let is_ntfs_vbr = ntfs_oem == b"NTFS    ";

    if has_anyo_sig {
        return Some(String::from("anyOS"));
    }
    if has_efi {
        return Some(String::from("GPT disk (may contain another OS)"));
    }
    if has_ntfs || is_ntfs_vbr {
        return Some(String::from("Windows (NTFS)"));
    }
    if has_linux {
        return Some(String::from("Linux"));
    }
    if has_fat {
        // Could be anyOS or generic FAT — check first partition VBR
        let part_start = u32::from_le_bytes([mbr[454], mbr[455], mbr[456], mbr[457]]);
        if part_start > 0 {
            let mut vbr = [0u8; 512];
            sys::disk_read(device_id as u32, part_start as u64, 1, &mut vbr);
            if vbr[3..11] == *b"EXFAT   " {
                // Check exFAT volume serial for anyOS signature
                let serial = u32::from_le_bytes([vbr[100], vbr[101], vbr[102], vbr[103]]);
                if serial == 0x414E594F { // "ANYO"
                    return Some(String::from("anyOS"));
                }
            }
        }
        return None; // Generic FAT, probably empty
    }

    None
}

// ── Case-fix helpers for target filesystem ─────────────────────────────────

const CASE_MAP: &[(&str, &str)] = &[
    ("system", "System"), ("applications", "Applications"),
    ("users", "Users"), ("libraries", "Libraries"),
    ("info.conf", "Info.conf"), ("icon.ico", "Icon.ico"),
];

pub fn fix_case(name: &str) -> String {
    for &(lower, proper) in CASE_MAP {
        if name == lower { return String::from(proper); }
    }
    if name.ends_with(".app") { return capitalize_words(name); }
    if name.ends_with(".dlib") { return capitalize_first(name); }
    String::from(name)
}

fn capitalize_first(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    let mut first = true;
    for ch in s.chars() {
        if first && ch.is_ascii_lowercase() { r.push((ch as u8 - 32) as char); }
        else { r.push(ch); }
        first = false;
    }
    r
}

fn capitalize_words(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    let mut cap = true;
    for ch in s.chars() {
        if ch == ' ' || ch == '-' || ch == '_' { r.push(ch); cap = true; }
        else if cap && ch.is_ascii_lowercase() { r.push((ch as u8 - 32) as char); cap = false; }
        else { r.push(ch); cap = false; }
    }
    r
}
