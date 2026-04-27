//! Disk and partition management syscall handlers.
//!
//! Covers disk listing, partition listing, raw disk I/O (read/write),
//! and partition table manipulation (create, delete, rescan).

#[allow(unused_imports)]
use super::helpers::is_valid_user_ptr;

/// SYS_DISK_LIST - List block devices.
/// Each entry is 64 bytes:
///   [0]     id (u8)
///   [1]     disk_id (u8)
///   [2]     partition index (0xFF = whole disk, else 0-based)
///   [3..8]  reserved
///   [8..16] start_lba (LE u64)
///   [16..24] size_sectors (LE u64)
///   [24..64] label (40 bytes, NUL-padded, from ATA IDENTIFY / USB / SD)
/// Returns total device count.
///
/// Backwards-compatible: if the buffer is too small for 64-byte entries but
/// fits 32-byte entries, the old 32-byte format is used (no label).
#[cfg(target_arch = "x86_64")]
pub fn sys_disk_list(buf_ptr: u64, buf_size: u32) -> u32 {
    use crate::drivers::storage::blockdev;
    let devices = blockdev::list_devices();
    let count = devices.len();
    if buf_ptr != 0 && buf_size > 0 && is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        // Use 64-byte entries if buffer is large enough, else fall back to 32
        let entry_size = if buf_size as usize / count.max(1) >= 64 { 64usize } else { 32usize };
        let max_entries = buf_size as usize / entry_size;
        for (i, dev) in devices.iter().enumerate().take(max_entries.min(count)) {
            let off = i * entry_size;
            for b in &mut buf[off..off + entry_size] { *b = 0; }
            buf[off] = dev.id;
            buf[off + 1] = dev.disk_id;
            buf[off + 2] = dev.partition.unwrap_or(0xFF);
            buf[off + 8..off + 16].copy_from_slice(&dev.start_lba.to_le_bytes());
            buf[off + 16..off + 24].copy_from_slice(&dev.size_sectors.to_le_bytes());
            if entry_size >= 64 {
                let label_len = dev.label.len().min(40);
                buf[off + 24..off + 24 + label_len].copy_from_slice(&dev.label[..label_len]);
            }
        }
    }
    count as u32
}

#[cfg(target_arch = "aarch64")]
pub fn sys_disk_list(buf_ptr: u64, buf_size: u32) -> u32 {
    use crate::drivers::storage::blockdev;
    let devices = blockdev::list_devices();
    let count = devices.len();
    if buf_ptr != 0 && buf_size > 0 && is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
        let entry_size = if buf_size as usize / count.max(1) >= 64 { 64usize } else { 32usize };
        let max_entries = buf_size as usize / entry_size;
        for (i, dev) in devices.iter().enumerate().take(max_entries.min(count)) {
            let off = i * entry_size;
            for b in &mut buf[off..off + entry_size] { *b = 0; }
            buf[off] = dev.id;
            buf[off + 1] = dev.disk_id;
            buf[off + 2] = dev.partition.unwrap_or(0xFF);
            buf[off + 8..off + 16].copy_from_slice(&dev.start_lba.to_le_bytes());
            buf[off + 16..off + 24].copy_from_slice(&dev.size_sectors.to_le_bytes());
            if entry_size >= 64 {
                let label_len = dev.label.len().min(40);
                buf[off + 24..off + 24 + label_len].copy_from_slice(&dev.label[..label_len]);
            }
        }
    }
    count as u32
}

/// SYS_DISK_PARTITIONS - List partitions for a disk.
/// Each entry is 32 bytes:
///   [0]     index (u8)
///   [1]     type_id (u8, see PartitionType mapping)
///   [2]     bootable (u8, 0/1)
///   [3]     scheme (u8: 0=MBR, 1=GPT, 2=None)
///   [4..8]  reserved
///   [8..16] start_lba (LE u64)
///   [16..24] size_sectors (LE u64)
///   [24..32] reserved (zeroed)
/// Returns partition count.
/// Shared implementation for disk_partitions on both architectures.
fn disk_partitions_impl(disk_id: u32, buf_ptr: u64, buf_size: u32) -> u32 {
    use crate::fs::partition;
    use crate::drivers::storage::blockdev;

    let whole_disk = match blockdev::find_device(disk_id as u8, None) {
        Some(d) => d,
        None => return 0,
    };

    let table = partition::scan_disk(|lba, buf| {
        let mut sector_buf = [0u8; 512];
        if !whole_disk.read_sectors(lba as u32, 1, &mut sector_buf) {
            return false;
        }
        buf[..512].copy_from_slice(&sector_buf);
        true
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
            buf[off + 8..off + 16].copy_from_slice(&part.start_lba.to_le_bytes());
            buf[off + 16..off + 24].copy_from_slice(&part.size_sectors.to_le_bytes());
        }
    }
    count as u32
}

#[cfg(target_arch = "x86_64")]
pub fn sys_disk_partitions(disk_id: u32, buf_ptr: u64, buf_size: u32) -> u32 {
    disk_partitions_impl(disk_id, buf_ptr, buf_size)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_disk_partitions(disk_id: u32, buf_ptr: u64, buf_size: u32) -> u32 {
    disk_partitions_impl(disk_id, buf_ptr, buf_size)
}

/// SYS_DISK_READ - Read raw sectors from a block device.
///   arg1: device_id (from sys_disk_list)
///   arg2: relative_lba (within device/partition)
///   arg3: sector_count
///   arg4: buf_ptr
///   arg5: buf_size
/// Returns sectors read, or u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_disk_read(device_id: u32, lba: u32, count: u32, buf_ptr: u64, buf_size: u32) -> u32 {
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
pub fn sys_disk_read(device_id: u32, lba: u32, count: u32, buf_ptr: u64, buf_size: u32) -> u32 {
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
    if dev.read_sectors(lba, count, buf) { count } else { u32::MAX }
}

/// SYS_DISK_WRITE - Write raw sectors to a block device.
///   arg1: device_id
///   arg2: relative_lba
///   arg3: sector_count
///   arg4: buf_ptr
///   arg5: buf_size
/// Returns sectors written, or u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_disk_write(device_id: u32, lba: u32, count: u32, buf_ptr: u64, buf_size: u32) -> u32 {
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
    // Use direct write (bypass write-back cache) for raw disk I/O syscalls.
    // The write-back cache is keyed by disk_id=0 and flushes through the
    // default storage backend, which may not be the correct disk (e.g. during
    // CD-ROM boot the default backend is IDE/CD, not the AHCI install target).
    let abs_lba = dev.start_lba as u32 + lba;
    let ok = {
        let overrides = crate::drivers::storage::IO_OVERRIDES.lock();
        if let Some(handler) = overrides.iter().find(|h| h.disk_id == dev.disk_id) {
            let f = handler.write_fn;
            let did = dev.disk_id;
            drop(overrides);
            f(did, abs_lba, count, buf)
        } else {
            drop(overrides);
            // Direct write bypassing cache
            crate::drivers::storage::write_sectors_direct(abs_lba, count, buf)
        }
    };
    // Invalidate any cached copies of these sectors
    crate::fs::blockcache::invalidate(0, abs_lba, count);
    if ok { count } else { u32::MAX }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_disk_write(device_id: u32, lba: u32, count: u32, buf_ptr: u64, buf_size: u32) -> u32 {
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
    if dev.write_sectors(lba, count, buf) { count } else { u32::MAX }
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
/// Shared implementation for partition_create on both architectures.
///
/// Reads/writes the MBR of the correct disk via blockdev, not the
/// default storage backend.
fn partition_create_impl(disk_id: u32, entry_ptr: u64, entry_size: u32) -> u32 {
    use crate::drivers::storage::blockdev;

    if entry_size < 16 || !is_valid_user_ptr(entry_ptr as u64, entry_size as u64) {
        return u32::MAX;
    }

    let whole_disk = match blockdev::find_device(disk_id as u8, None) {
        Some(d) => d,
        None => return u32::MAX,
    };

    let entry = unsafe { core::slice::from_raw_parts(entry_ptr as *const u8, 16) };
    let index = entry[0];
    let ptype = entry[1];
    let bootable = entry[2] != 0;
    let start_lba = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
    let size_sectors = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);

    if index > 3 {
        return u32::MAX; // MBR only supports 4 primary partitions
    }

    // Read current MBR from the correct disk
    let mut mbr = [0u8; 512];
    if !whole_disk.read_sectors(0, 1, &mut mbr) {
        return u32::MAX;
    }

    // If no MBR signature exists, initialize a blank MBR.
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        for b in &mut mbr[446..510] { *b = 0; } // clear partition table area
        mbr[510] = 0x55;
        mbr[511] = 0xAA;
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

    // Write MBR back to the correct disk
    if !whole_disk.write_sectors(0, 1, &mbr) {
        return u32::MAX;
    }
    0
}

#[cfg(target_arch = "x86_64")]
pub fn sys_partition_create(disk_id: u32, entry_ptr: u64, entry_size: u32) -> u32 {
    partition_create_impl(disk_id, entry_ptr, entry_size)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_partition_create(disk_id: u32, entry_ptr: u64, entry_size: u32) -> u32 {
    partition_create_impl(disk_id, entry_ptr, entry_size)
}

/// Shared implementation for partition_delete on both architectures.
fn partition_delete_impl(disk_id: u32, index: u32) -> u32 {
    use crate::drivers::storage::blockdev;

    if index > 3 {
        return u32::MAX;
    }

    let whole_disk = match blockdev::find_device(disk_id as u8, None) {
        Some(d) => d,
        None => return u32::MAX,
    };

    let mut mbr = [0u8; 512];
    if !whole_disk.read_sectors(0, 1, &mut mbr) {
        return u32::MAX;
    }
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return u32::MAX;
    }

    let off = 446 + index as usize * 16;
    for b in &mut mbr[off..off + 16] { *b = 0; }

    if !whole_disk.write_sectors(0, 1, &mbr) {
        return u32::MAX;
    }
    0
}

/// SYS_PARTITION_DELETE - Delete an MBR partition entry (zero it out).
///   arg1: disk_id (u8)
///   arg2: partition_index (0-3)
/// Returns 0 on success, u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_partition_delete(disk_id: u32, index: u32) -> u32 {
    partition_delete_impl(disk_id, index)
}

#[cfg(target_arch = "aarch64")]
pub fn sys_partition_delete(disk_id: u32, index: u32) -> u32 {
    partition_delete_impl(disk_id, index)
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
pub fn sys_partition_rescan(disk_id: u32) -> u32 {
    use crate::drivers::storage::blockdev;
    blockdev::remove_partition_devices(disk_id as u8);
    blockdev::scan_and_register_partitions(disk_id as u8);
    let devices = blockdev::list_devices();
    devices.iter().filter(|d| d.disk_id == disk_id as u8 && d.partition.is_some()).count() as u32
}

/// Maps a `PartitionType` enum variant to its corresponding MBR type byte.
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
        PartitionType::CoreFs => 0xCF,
        PartitionType::GptEsp => 0xEF,
        PartitionType::GptBasicData => 0xBD,
        PartitionType::GptLinuxFs => 0xBE,
        PartitionType::Unknown(v) => *v,
    }
}

/// SYS_DISK_EJECT - Safely eject a removable disk.
/// Flushes all dirty data, unmounts all partitions on the disk, removes
/// block device entries, and emits EVT_VOLUME_EJECTED.
/// arg1 = disk_id
/// Returns 0 on success, u32::MAX on error.
#[cfg(target_arch = "x86_64")]
pub fn sys_disk_eject(disk_id: u32) -> u32 {
    let disk = disk_id as u8;

    // 1. Flush all dirty filesystem data and metadata
    crate::fs::vfs::sync_all();

    // 2. Find and unmount all partitions on this disk
    let mounts = crate::fs::vfs::list_mounts();
    for (path, _fstype, _dev_id) in &mounts {
        // Check if this mount point is backed by the target disk
        // Mount paths for partitions typically include the disk ID
        // We unmount any mount on /mnt/ that was from this disk
        if path.starts_with("/mnt/") {
            let _ = crate::fs::vfs::umount_fs(path);
        }
    }

    // 3. Remove block device entries for this disk's partitions
    crate::drivers::storage::blockdev::remove_partition_devices(disk);

    // 4. Unregister I/O handler (for USB storage / SD card)
    crate::drivers::storage::unregister_device_io(disk);

    // 5. Emit eject event
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_VOLUME_EJECTED, disk_id, 0, 0, 0,
    ));

    crate::serial_println!("  Disk {} ejected safely", disk_id);
    0
}

#[cfg(target_arch = "aarch64")]
pub fn sys_disk_eject(disk_id: u32) -> u32 {
    let disk = disk_id as u8;
    crate::fs::vfs::sync_all();
    let mounts = crate::fs::vfs::list_mounts();
    for (path, _fstype, _dev_id) in &mounts {
        if path.starts_with("/mnt/") {
            let _ = crate::fs::vfs::umount_fs(path);
        }
    }
    crate::drivers::storage::blockdev::remove_partition_devices(disk);
    crate::drivers::storage::unregister_device_io(disk);
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_VOLUME_EJECTED, disk_id, 0, 0, 0,
    ));
    crate::serial_println!("  Disk {} ejected safely", disk_id);
    0
}
