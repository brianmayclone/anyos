//! Disk and partition management syscall handlers.
//!
//! Covers disk listing, partition listing, raw disk I/O (read/write),
//! and partition table manipulation (create, delete, rescan).

#[allow(unused_imports)]
use super::helpers::is_valid_user_ptr;

/// SYS_DISK_LIST - List block devices.
/// Each entry is 32 bytes:
///   [0]     id (u8)
///   [1]     disk_id (u8)
///   [2]     partition index (0xFF = whole disk, else 0-based)
///   [3]     reserved
///   [4..12] start_lba (LE u64)
///   [12..20] size_sectors (LE u64)
///   [20..32] reserved (zeroed)
/// Returns total device count.
#[cfg(target_arch = "x86_64")]
pub fn sys_disk_list(buf_ptr: u32, buf_size: u32) -> u32 {
    use crate::drivers::storage::blockdev;
    let devices = blockdev::list_devices();
    let count = devices.len();
    if buf_ptr != 0 && buf_size > 0 && is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        let entry_size = 32usize;
        let max_entries = buf_size as usize / entry_size;
        for (i, dev) in devices.iter().enumerate().take(max_entries.min(count)) {
            let off = i * entry_size;
            for b in &mut buf[off..off + entry_size] { *b = 0; }
            buf[off] = dev.id;
            buf[off + 1] = dev.disk_id;
            buf[off + 2] = dev.partition.unwrap_or(0xFF);
            buf[off + 4..off + 12].copy_from_slice(&dev.start_lba.to_le_bytes());
            buf[off + 12..off + 20].copy_from_slice(&dev.size_sectors.to_le_bytes());
        }
    }
    count as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_disk_list(_buf_ptr: u32, _buf_size: u32) -> u32 {
    0
}

/// SYS_DISK_PARTITIONS - List partitions for a disk.
/// Each entry is 32 bytes:
///   [0]     index (u8)
///   [1]     type_id (u8, see PartitionType mapping)
///   [2]     bootable (u8, 0/1)
///   [3]     scheme (u8: 0=MBR, 1=GPT, 2=None)
///   [4..12] start_lba (LE u64)
///   [12..20] size_sectors (LE u64)
///   [20..32] reserved (zeroed)
/// Returns partition count.
#[cfg(target_arch = "x86_64")]
pub fn sys_disk_partitions(disk_id: u32, buf_ptr: u32, buf_size: u32) -> u32 {
    use crate::fs::partition;

    let table = partition::scan_disk(|lba, buf| {
        let abs_lba = lba as u32;
        crate::drivers::storage::read_sectors(abs_lba, 1, buf)
    });

    let count = table.partitions.len();
    if buf_ptr != 0 && buf_size > 0 && is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        let entry_size = 32usize;
        let max_entries = buf_size as usize / entry_size;
        for (i, part) in table.partitions.iter().enumerate().take(max_entries.min(count)) {
            let off = i * entry_size;
            for b in &mut buf[off..off + entry_size] { *b = 0; }
            buf[off] = part.index;
            buf[off + 1] = partition_type_to_id(&part.part_type);
            buf[off + 2] = if part.bootable { 1 } else { 0 };
            buf[off + 3] = match part.scheme {
                partition::PartitionScheme::Mbr => 0,
                partition::PartitionScheme::Gpt => 1,
                partition::PartitionScheme::None => 2,
            };
            buf[off + 4..off + 12].copy_from_slice(&part.start_lba.to_le_bytes());
            buf[off + 12..off + 20].copy_from_slice(&part.size_sectors.to_le_bytes());
        }
    }
    count as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_disk_partitions(_disk_id: u32, _buf_ptr: u32, _buf_size: u32) -> u32 {
    0
}

/// SYS_DISK_READ - Read raw sectors from a block device.
///   arg1: device_id (from sys_disk_list)
///   arg2: relative_lba (within device/partition)
///   arg3: sector_count
///   arg4: buf_ptr
///   arg5: buf_size
/// Returns sectors read, or u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_disk_read(device_id: u32, lba: u32, count: u32, buf_ptr: u32, buf_size: u32) -> u32 {
    use crate::drivers::storage::blockdev;

    let needed = count as u64 * 512;
    if needed > buf_size as u64 || buf_ptr == 0 {
        return u32::MAX;
    }
    if !is_valid_user_ptr(buf_ptr as u64, needed) {
        return u32::MAX;
    }

    let dev = match blockdev::get_device(device_id as u8) {
        Some(d) => d,
        None => return u32::MAX,
    };

    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, needed as usize) };
    if dev.read_sectors(lba, count, buf) {
        count
    } else {
        u32::MAX
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_disk_read(_device_id: u32, _lba: u32, _count: u32, _buf_ptr: u32, _buf_size: u32) -> u32 {
    u32::MAX
}

/// SYS_DISK_WRITE - Write raw sectors to a block device.
///   arg1: device_id
///   arg2: relative_lba
///   arg3: sector_count
///   arg4: buf_ptr
///   arg5: buf_size
/// Returns sectors written, or u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_disk_write(device_id: u32, lba: u32, count: u32, buf_ptr: u32, buf_size: u32) -> u32 {
    use crate::drivers::storage::blockdev;

    let needed = count as u64 * 512;
    if needed > buf_size as u64 || buf_ptr == 0 {
        return u32::MAX;
    }
    if !is_valid_user_ptr(buf_ptr as u64, needed) {
        return u32::MAX;
    }

    let dev = match blockdev::get_device(device_id as u8) {
        Some(d) => d,
        None => return u32::MAX,
    };

    let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, needed as usize) };
    if dev.write_sectors(lba, count, buf) {
        count
    } else {
        u32::MAX
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_disk_write(_device_id: u32, _lba: u32, _count: u32, _buf_ptr: u32, _buf_size: u32) -> u32 {
    u32::MAX
}

/// SYS_PARTITION_CREATE - Create/update an MBR partition entry.
///   arg1: disk_id (u8)
///   arg2: entry_ptr — pointer to 16-byte struct:
///         [0]     partition index (0-3 for MBR)
///         [1]     type byte (MBR type, e.g. 0x0B=FAT32, 0x07=NTFS)
///         [2]     bootable (0/1)
///         [3]     reserved
///         [4..8]  start_lba (LE u32)
///         [8..12] size_sectors (LE u32)
///         [12..16] reserved
///   arg3: entry_size (must be >= 16)
/// Returns 0 on success, u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_partition_create(disk_id: u32, entry_ptr: u32, entry_size: u32) -> u32 {
    if entry_size < 16 || !is_valid_user_ptr(entry_ptr as u64, entry_size as u64) {
        return u32::MAX;
    }
    let entry = unsafe { core::slice::from_raw_parts(entry_ptr as *const u8, 16) };
    let index = entry[0];
    let ptype = entry[1];
    let bootable = entry[2] != 0;
    let start_lba = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
    let size_sectors = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);

    if index > 3 {
        return u32::MAX; // MBR only supports 4 primary partitions
    }

    // Read current MBR
    let mut mbr = [0u8; 512];
    if !crate::drivers::storage::read_sectors(0, 1, &mut mbr) {
        return u32::MAX;
    }

    // Verify MBR signature
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return u32::MAX;
    }

    // Write partition entry at offset 446 + index*16
    let off = 446 + index as usize * 16;
    mbr[off] = if bootable { 0x80 } else { 0x00 };
    mbr[off + 1] = 0xFE; // CHS start (LBA-only)
    mbr[off + 2] = 0xFF;
    mbr[off + 3] = 0xFF;
    mbr[off + 4] = ptype;
    mbr[off + 5] = 0xFE; // CHS end (LBA-only)
    mbr[off + 6] = 0xFF;
    mbr[off + 7] = 0xFF;
    mbr[off + 8..off + 12].copy_from_slice(&start_lba.to_le_bytes());
    mbr[off + 12..off + 16].copy_from_slice(&size_sectors.to_le_bytes());

    // Write MBR back
    if !crate::drivers::storage::write_sectors(0, 1, &mbr) {
        return u32::MAX;
    }
    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_partition_create(_disk_id: u32, _entry_ptr: u32, _entry_size: u32) -> u32 {
    u32::MAX
}

/// SYS_PARTITION_DELETE - Delete an MBR partition entry (zero it out).
///   arg1: disk_id (u8)
///   arg2: partition_index (0-3)
/// Returns 0 on success, u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_partition_delete(disk_id: u32, index: u32) -> u32 {
    if index > 3 {
        return u32::MAX;
    }

    let mut mbr = [0u8; 512];
    if !crate::drivers::storage::read_sectors(0, 1, &mut mbr) {
        return u32::MAX;
    }
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return u32::MAX;
    }

    let off = 446 + index as usize * 16;
    for b in &mut mbr[off..off + 16] { *b = 0; }

    if !crate::drivers::storage::write_sectors(0, 1, &mbr) {
        return u32::MAX;
    }
    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_partition_delete(_disk_id: u32, _index: u32) -> u32 {
    u32::MAX
}

/// SYS_PARTITION_RESCAN - Re-scan partition table and re-register block devices.
///   arg1: disk_id (u8)
/// Returns partition count found, or u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_partition_rescan(disk_id: u32) -> u32 {
    use crate::drivers::storage::blockdev;

    // Remove existing partition devices for this disk
    blockdev::remove_partition_devices(disk_id as u8);

    // Re-scan and register
    blockdev::scan_and_register_partitions(disk_id as u8);

    // Return count of partitions found
    let devices = blockdev::list_devices();
    devices.iter().filter(|d| d.disk_id == disk_id as u8 && d.partition.is_some()).count() as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_partition_rescan(_disk_id: u32) -> u32 {
    u32::MAX
}

/// Maps a `PartitionType` enum variant to its corresponding MBR type byte.
#[cfg(target_arch = "x86_64")]
fn partition_type_to_id(pt: &crate::fs::partition::PartitionType) -> u8 {
    use crate::fs::partition::PartitionType;
    match pt {
        PartitionType::Empty => 0x00,
        PartitionType::Fat12 => 0x01,
        PartitionType::Fat16 => 0x06,
        PartitionType::Fat16Lba => 0x0E,
        PartitionType::Fat32 => 0x0B,
        PartitionType::Fat32Lba => 0x0C,
        PartitionType::NtfsExfat => 0x07,
        PartitionType::LinuxSwap => 0x82,
        PartitionType::LinuxNative => 0x83,
        PartitionType::GptEsp => 0xEF,
        PartitionType::GptBasicData => 0xBD,
        PartitionType::GptLinuxFs => 0xBE,
        PartitionType::Unknown(v) => *v,
    }
}
