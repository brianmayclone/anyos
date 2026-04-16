//! Block device abstraction layer.
//!
//! Each block device represents either a whole disk or a single partition.
//! Partition block devices translate relative LBAs to absolute disk LBAs
//! and enforce bounds checking.
//!
//! Device IDs are **stable**: once assigned, an ID never changes or gets
//! reused for a different device during the lifetime of the system.
//! Removing a device leaves a `None` slot that can be reclaimed by a
//! future registration.

use alloc::vec::Vec;
use crate::serial_verbose_println;
use crate::sync::spinlock::Spinlock;
use crate::fs::partition::PartitionType;

/// Block device representing a whole disk or a partition on a disk.
#[derive(Debug, Clone)]
pub struct BlockDevice {
    /// Global device ID (stable, never reassigned).
    pub id: u8,
    /// Disk index (0 = primary disk, 1 = secondary, ...).
    pub disk_id: u8,
    /// Partition index on this disk, or None for the whole disk.
    pub partition: Option<u8>,
    /// Partition type (from MBR type byte or GPT type GUID).
    pub part_type: PartitionType,
    /// Absolute start LBA on the physical disk.
    pub start_lba: u64,
    /// Total number of 512-byte sectors in this device/partition.
    pub size_sectors: u64,
    /// Disk model/label from ATA IDENTIFY or USB descriptor (trimmed, NUL-padded).
    pub label: [u8; 40],
}

impl BlockDevice {
    /// Read sectors at a partition-relative LBA into `buf`.
    ///
    /// Returns `true` on success. Fails if the read would exceed bounds.
    /// Maximum number of retries on transient I/O errors before giving up.
    const MAX_IO_RETRIES: u32 = 3;

    pub fn read_sectors(&self, relative_lba: u32, count: u32, buf: &mut [u8]) -> bool {
        if self.size_sectors > 0 && (relative_lba as u64 + count as u64) > self.size_sectors {
            serial_verbose_println!(
                "[blockdev] read out of bounds: dev={} rel_lba={} count={} size={}",
                self.id, relative_lba, count, self.size_sectors
            );
            return false;
        }
        let abs_lba = self.start_lba as u32 + relative_lba;
        // Check for per-device I/O override (USB storage, etc.)
        {
            let overrides = super::IO_OVERRIDES.lock();
            if let Some(handler) = overrides.iter().find(|h| h.disk_id == self.disk_id) {
                let f = handler.read_fn;
                let did = self.disk_id;
                drop(overrides);
                return f(did, abs_lba, count, buf);
            }
        }
        for attempt in 0..Self::MAX_IO_RETRIES {
            if super::read_sectors(abs_lba, count, buf) {
                return true;
            }
            serial_verbose_println!(
                "[blockdev] read failed: dev={} lba={} count={} attempt={}/{}",
                self.id, abs_lba, count, attempt + 1, Self::MAX_IO_RETRIES
            );
        }
        crate::serial_println!(
            "[blockdev] ERROR: read permanently failed: dev={} lba={} count={}",
            self.id, abs_lba, count
        );
        false
    }

    /// Write sectors at a partition-relative LBA from `buf`.
    ///
    /// Returns `true` on success. Fails if the write would exceed bounds.
    pub fn write_sectors(&self, relative_lba: u32, count: u32, buf: &[u8]) -> bool {
        if self.size_sectors > 0 && (relative_lba as u64 + count as u64) > self.size_sectors {
            serial_verbose_println!(
                "[blockdev] write out of bounds: dev={} rel_lba={} count={} size={}",
                self.id, relative_lba, count, self.size_sectors
            );
            return false;
        }
        let abs_lba = self.start_lba as u32 + relative_lba;
        // Check for per-device I/O override (USB storage, etc.)
        {
            let overrides = super::IO_OVERRIDES.lock();
            if let Some(handler) = overrides.iter().find(|h| h.disk_id == self.disk_id) {
                let f = handler.write_fn;
                let did = self.disk_id;
                drop(overrides);
                return f(did, abs_lba, count, buf);
            }
        }
        for attempt in 0..Self::MAX_IO_RETRIES {
            if super::write_sectors(abs_lba, count, buf) {
                return true;
            }
            serial_verbose_println!(
                "[blockdev] write failed: dev={} lba={} count={} attempt={}/{}",
                self.id, abs_lba, count, attempt + 1, Self::MAX_IO_RETRIES
            );
        }
        crate::serial_println!(
            "[blockdev] ERROR: write permanently failed: dev={} lba={} count={}",
            self.id, abs_lba, count
        );
        false
    }

    /// Read a single 512-byte sector at a partition-relative LBA.
    pub fn read_sector(&self, relative_lba: u32, buf: &mut [u8; 512]) -> bool {
        self.read_sectors(relative_lba, 1, buf)
    }

    /// Human-readable device name (e.g. "hd0", "hd0p1").
    pub fn name(&self) -> alloc::string::String {
        if let Some(p) = self.partition {
            alloc::format!("hd{}p{}", self.disk_id, p + 1)
        } else {
            alloc::format!("hd{}", self.disk_id)
        }
    }
}

/// Maximum number of block devices we track.
const MAX_DEVICES: usize = 32;

/// Global block device registry.  Slots may be `None` after a device
/// has been removed — this keeps all other IDs stable.
static DEVICES: Spinlock<Vec<Option<BlockDevice>>> = Spinlock::new(Vec::new());

/// Register a new block device. Returns its assigned device ID.
///
/// Reuses the first free (None) slot if one exists, otherwise appends.
pub fn register_device(dev: BlockDevice) -> u8 {
    let mut devs = DEVICES.lock();
    // Look for a free slot
    for (i, slot) in devs.iter_mut().enumerate() {
        if slot.is_none() {
            let id = i as u8;
            let mut dev = dev;
            dev.id = id;
            serial_verbose_println!(
                "[blockdev] registered: id={} disk={} part={:?} start={} size={}",
                id, dev.disk_id, dev.partition, dev.start_lba, dev.size_sectors
            );
            *slot = Some(dev);
            return id;
        }
    }
    // No free slot — append
    let id = devs.len() as u8;
    let mut dev = dev;
    dev.id = id;
    serial_verbose_println!(
        "[blockdev] registered: id={} disk={} part={:?} start={} size={}",
        id, dev.disk_id, dev.partition, dev.start_lba, dev.size_sectors
    );
    devs.push(Some(dev));
    id
}

/// Get a block device by its ID. Returns a clone.
pub fn get_device(id: u8) -> Option<BlockDevice> {
    let devs = DEVICES.lock();
    devs.get(id as usize).and_then(|slot| slot.clone())
}

/// Find a block device for a specific disk and optional partition.
pub fn find_device(disk_id: u8, partition: Option<u8>) -> Option<BlockDevice> {
    let devs = DEVICES.lock();
    devs.iter()
        .filter_map(|slot| slot.as_ref())
        .find(|d| d.disk_id == disk_id && d.partition == partition)
        .cloned()
}

/// List all registered block devices (skips empty slots).
pub fn list_devices() -> Vec<BlockDevice> {
    DEVICES.lock().iter().filter_map(|slot| slot.clone()).collect()
}

/// Count of registered (non-empty) devices.
pub fn device_count() -> usize {
    DEVICES.lock().iter().filter(|s| s.is_some()).count()
}

/// Remove all partition devices for a given disk (keeps the whole-disk
/// device).  The freed slots become `None` — IDs are **not** reassigned.
pub fn remove_partition_devices(disk_id: u8) {
    let mut devs = DEVICES.lock();
    for slot in devs.iter_mut() {
        if let Some(dev) = slot {
            if dev.disk_id == disk_id && dev.partition.is_some() {
                serial_verbose_println!(
                    "[blockdev] removed: id={} disk={} part={:?}",
                    dev.id, dev.disk_id, dev.partition
                );
                *slot = None;
            }
        }
    }
}

/// Scan a disk for partitions and register each as a block device.
///
/// Call this after registering the whole-disk device. Reads sector 0 of the
/// disk via `super::read_sectors()` and parses MBR/GPT.
pub fn scan_and_register_partitions(disk_id: u8) {
    use crate::fs::partition;

    let whole_disk = match find_device(disk_id, None) {
        Some(d) => d,
        None => {
            serial_verbose_println!("[blockdev] scan_and_register: disk {} not found", disk_id);
            return;
        }
    };

    let table = partition::scan_disk(|lba, buf| {
        let mut sector_buf = [0u8; 512];
        if !whole_disk.read_sectors(lba as u32, 1, &mut sector_buf) {
            return false;
        }
        buf[..512].copy_from_slice(&sector_buf);
        true
    });

    serial_verbose_println!(
        "[blockdev] disk {} partition scheme: {:?}, {} partitions found",
        disk_id, table.scheme, table.partitions.len()
    );

    // Inherit label from the whole-disk device
    let parent_label = whole_disk.label;

    for part in &table.partitions {
        if part.part_type == partition::PartitionType::Empty {
            continue;
        }
        register_device(BlockDevice {
            id: 0, // assigned by register_device
            disk_id,
            partition: Some(part.index),
            part_type: part.part_type,
            start_lba: part.start_lba,
            size_sectors: part.size_sectors,
            label: parent_label,
        });
    }
}

/// Auto-mount all partitions on a removable disk under /Volumes/.
/// Called after scan_and_register_partitions() for hot-plugged devices (USB, SD).
/// Mount point format: /Volumes/disk{disk_id}p{part+1}
pub fn auto_mount_removable(disk_id: u8) {
    let devs = list_devices();
    for dev in &devs {
        if dev.disk_id != disk_id || dev.partition.is_none() { continue; }
        let part_idx = dev.partition.unwrap();
        let mount_path = alloc::format!("/Volumes/disk{}p{}", disk_id, part_idx + 1);

        // Try mounting as exFAT/FAT (most common for removable media)
        crate::fs::vfs::set_root_partition_lba(dev.start_lba as u32);
        let result = crate::fs::vfs::mount_fs(&mount_path, "", 0); // 0 = auto-detect
        match result {
            Ok(()) => {
                crate::serial_println!("  [auto-mount] {} → mounted (disk {} part {})",
                    mount_path, disk_id, part_idx + 1);
                crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
                    crate::ipc::event_bus::EVT_VOLUME_MOUNTED,
                    disk_id as u32, part_idx as u32, 0, 0,
                ));
            }
            Err(_) => {
                crate::serial_verbose_println!("  [auto-mount] {} → failed", mount_path);
            }
        }
    }
}

/// Parse a device path like "/dev/hd0p1" into (disk_id, partition_index).
///
/// Returns `(disk_id, Some(part_idx))` for partitions or `(disk_id, None)` for whole disks.
/// partition_index is 0-based (hd0p1 → partition index 0).
pub fn parse_device_path(path: &str) -> Option<(u8, Option<u8>)> {
    let name = path.strip_prefix("/dev/").unwrap_or(path);

    if !name.starts_with("hd") {
        return None;
    }
    let rest = &name[2..];

    // Find where 'p' is (if present)
    if let Some(p_pos) = rest.find('p') {
        let disk_str = &rest[..p_pos];
        let part_str = &rest[p_pos + 1..];
        let disk_id: u8 = disk_str.parse().ok()?;
        let part_num: u8 = part_str.parse().ok()?;
        if part_num == 0 {
            return None; // partitions are 1-based in user-facing names
        }
        Some((disk_id, Some(part_num - 1)))
    } else {
        let disk_id: u8 = rest.parse().ok()?;
        Some((disk_id, None))
    }
}
